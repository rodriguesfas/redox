//! `ai-monitor`: best-effort system sampling (CPU, RAM, network) reading
//! `/proc` on Linux-style systems. Every value degrades to `None` where the
//! source is missing (e.g. Redox), keeping the daemon portable.

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Sample {
    pub cpu_percent: Option<f64>,
    pub mem_used_mb: Option<f64>,
    pub mem_total_mb: Option<f64>,
    pub net_rx_kbps: Option<f64>,
    pub net_tx_kbps: Option<f64>,
}

pub struct Sampler {
    last_cpu: Option<CpuTicks>,
    last_net: Option<(u64, u64)>,
    last_net_ts: Option<f64>,
}

struct CpuTicks {
    total: u64,
    idle: u64,
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler {
            last_cpu: None,
            last_net: None,
            last_net_ts: None,
        }
    }

    fn now() -> f64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    }

    pub fn sample(&mut self) -> Sample {
        let mut out = Sample::default();

        if let Some((used, total)) = read_mem_kb() {
            out.mem_total_mb = Some(total as f64 / 1024.0);
            out.mem_used_mb = Some(used as f64 / 1024.0);
        }

        if let Some((total, idle)) = read_cpu_ticks() {
            if let Some(last) = self.last_cpu.take() {
                let d_total = total.saturating_sub(last.total);
                let d_idle = idle.saturating_sub(last.idle);
                if d_total > 0 {
                    let busy = (d_total - d_idle) as f64 / d_total as f64 * 100.0;
                    out.cpu_percent = Some(busy.clamp(0.0, 100.0));
                }
            }
            self.last_cpu = Some(CpuTicks { total, idle });
        }

        if let Some((rx, tx)) = read_net_bytes() {
            let ts = Self::now();
            if let Some((last_rx, last_tx)) = self.last_net {
                let dts = ts - self.last_net_ts.unwrap_or(ts);
                if dts > 0.0 {
                    out.net_rx_kbps = Some((rx.saturating_sub(last_rx)) as f64 / (dts * 1000.0));
                    out.net_tx_kbps = Some((tx.saturating_sub(last_tx)) as f64 / (dts * 1000.0));
                }
            }
            self.last_net = Some((rx, tx));
            self.last_net_ts = Some(ts);
        }

        out
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler::new()
    }
}

fn read_proc(rel: &str) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{rel}")).ok()
}

/// Returns `(used_kb, total_kb)` from /proc/meminfo.
fn read_mem_kb() -> Option<(u64, u64)> {
    let text = read_proc("meminfo")?;
    let mut total = 0u64;
    let mut available = 0u64;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = kb_of(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available = kb_of(rest);
        }
    }
    if total == 0 {
        return None;
    }
    let available = if available == 0 { total } else { available };
    Some((total.saturating_sub(available), total))
}

fn kb_of(rest: &str) -> u64 {
    rest.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Returns `(total_ticks, idle_ticks)` from the `cpu` line of /proc/stat.
fn read_cpu_ticks() -> Option<(u64, u64)> {
    let text = read_proc("stat")?;
    let line = text.lines().next()?;
    if !line.starts_with("cpu ") {
        return None;
    }
    let vals: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    if vals.len() < 4 {
        return None;
    }
    // fields: user nice system idle iowait irq softirq steal ...
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = vals.iter().sum();
    Some((total, idle))
}

/// Returns `(rx_bytes, tx_bytes)` summed across all interfaces.
fn read_net_bytes() -> Option<(u64, u64)> {
    let text = read_proc("net/dev")?;
    let mut rx = 0u64;
    let mut tx = 0u64;
    for line in text.lines().skip(2) {
        let mut parts = line.split(':');
        let Some(right) = parts.nth(1) else { continue };
        let mut fields = right.split_whitespace();
        let Some(r) = fields.next().and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        let Some(t) = fields.nth(8).and_then(|s| s.parse::<u64>().ok()) else {
            continue;
        };
        rx += r;
        tx += t;
    }
    if rx == 0 && tx == 0 {
        return None;
    }
    Some((rx, tx))
}

/// Count of printable CPU cores (best-effort, for reference).
pub fn parallelism() -> Option<usize> {
    std::thread::available_parallelism().ok().map(|n| n.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_defaults_without_proc() {
        // On any machine /proc should exist; this just asserts no panic.
        let mut s = Sampler::new();
        let sample = s.sample();
        let _ = sample;
    }

    #[test]
    fn kb_of_parses_first_number() {
        assert_eq!(kb_of("    16384 kB\n"), 16384);
        assert_eq!(kb_of("nope"), 0);
    }
}
