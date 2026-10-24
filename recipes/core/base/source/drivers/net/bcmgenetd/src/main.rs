mod breadcrumb;
mod genet;
mod led;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::thread;
use std::time::Duration;

use common;
use daemon::Daemon;
use driver_network::NetworkScheme;
use fdt::Fdt;
use genet::Genet;
use sha2::{Digest, Sha256};

const CPRMAN_PHYS: usize = 0xfe1_01000;
const GPIO_PHYS: usize = 0xfe20_0000;
const GENET_PHYS: usize = 0xfd58_0000;

const PASSWORD: u32 = 0x5A00_0000;
const ENAB: u32 = 1 << 4;
const GATE: u32 = 1 << 6;
const BUSY: u32 = 1 << 7;
const SRC_MASK: u32 = 0xF;
const DIVI_MASK: u32 = 0xFFF << 12;
const SRC_PLLD_PER: u32 = 6;

fn rd(base: usize, off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

fn wr(base: usize, off: usize, val: u32) {
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}

/// Copia o log do kernel (o que está no monitor) para um arquivo no cartão.
fn dump_kernel_log(tag: &str) {
    let mut buf = Vec::new();
    if let Ok(mut f) = fs::File::open("/scheme/sys/log") {
        let _ = f.read_to_end(&mut buf);
    }
    if buf.is_empty() {
        return;
    }
    let path = format!("/var/lib/ai/boot-{}.log", tag);
    if fs::write(&path, &buf).is_ok() {
        if let Ok(f) = fs::OpenOptions::new().write(true).open(&path) {
            let _ = f.sync_all();
        }
    }
}

fn mark(led: &Option<led::Led>, n: u32, path: &str, msg: &str) {
    breadcrumb::write_at(path, n, msg);
    dump_kernel_log(&format!("{:02}", n));
    println!("AIOS v35: etapa {} — {}", n, msg);
    if let Some(l) = led {
        l.stage((n + 1).min(6));
    }
}

/// NetworkAdapter implementation for BCM2711 GENET.
struct GenetAdapter {
    genet: Genet,
    /// Control channel response buffer
    ctl_response: String,
    /// Downloaded update image (stored for apply)
    update_image: Option<Vec<u8>>,
}

impl GenetAdapter {
    fn handle_ctl_command(&mut self, cmd: &[u8]) {
        let cmd_str = match std::str::from_utf8(cmd) {
            Ok(s) => s.trim(),
            Err(_) => {
                self.ctl_response = "ERR: invalid UTF-8\n".to_string();
                return;
            }
        };

        let parts: Vec<&str> = cmd_str.split_whitespace().collect();
        if parts.is_empty() {
            self.ctl_response = "ERR: empty command\n".to_string();
            return;
        }

        match parts[0] {
            "status" => {
                let mac = self.genet.mac();
                self.ctl_response = format!(
                    "OK mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} driver=bcm2711-genetd\n",
                    mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
                );
            }
            "reboot" => {
                self.ctl_response = "OK: reboot requested\n".to_string();
            }
            "update" => {
                if parts.len() < 3 {
                    self.ctl_response = "ERR: usage: update <url> <sha256>\n".to_string();
                    return;
                }
                let url = parts[1];
                let expected_hash = parts[2];
                self.ctl_response = format!("OK: starting update from {} (expected hash: {})\n", url, expected_hash);
                
                // Perform actual download and verification
                match GenetAdapter::download_and_verify(url, expected_hash) {
                    Ok(data) => {
                        self.ctl_response.push_str(&format!("OK: downloaded {} bytes, hash verified\n", data.len()));
                        // Store for apply command
                        self.update_image = Some(data);
                        self.ctl_response.push_str("OK: run 'apply' to write to partition\n");
                    }
                    Err(e) => {
                        self.ctl_response.push_str(&format!("ERR: {}\n", e));
                    }
                }
            }
            "apply" => {
                if let Some(image) = self.update_image.take() {
                    match self.apply_update(&image) {
                        Ok(()) => {
                            self.ctl_response.push_str("OK: update written to partition\n");
                        }
                        Err(e) => {
                            self.ctl_response.push_str(&format!("ERR: apply failed: {}\n", e));
                        }
                    }
                } else {
                    self.ctl_response = "ERR: no update image downloaded\n".to_string();
                }
            }
            "shell" => {
                if parts.len() < 2 {
                    self.ctl_response = "ERR: usage: shell <command> [args...]\n".to_string();
                    return;
                }
                let cmd = &parts[1..].join(" ");
                match self.execute_shell(cmd) {
                    Ok(output) => {
                        self.ctl_response.push_str(&format!("OK:\n{}\n", output));
                    }
                    Err(e) => {
                        self.ctl_response.push_str(&format!("ERR: {}\n", e));
                    }
                }
            }
            _ => {
                self.ctl_response = format!("ERR: unknown command '{}'\n", parts[0]);
            }
        }
    }
    
    fn download_and_verify(url: &str, expected_hash: &str) -> Result<Vec<u8>, String> {
        // Parse URL (simple HTTP only)
        let url = url.strip_prefix("http://").ok_or("only http:// supported")?;
        let (host_port, path) = url.split_once('/').ok_or("invalid URL format")?;
        let path = format!("/{}", path);
        
        // Resolve address
        let addrs: Vec<_> = (host_port, 80).to_socket_addrs()
            .map_err(|e| format!("DNS resolution failed: {}", e))?
            .collect();
        let addr = addrs.first().ok_or("no address resolved")?;
        
        // Connect
        let mut stream = TcpStream::connect_timeout(addr, Duration::from_secs(10))
            .map_err(|e| format!("connection failed: {}", e))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|e| format!("set timeout failed: {}", e))?;
        
        // Send HTTP GET request
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            path, host_port
        );
        stream.write_all(request.as_bytes())
            .map_err(|e| format!("write request failed: {}", e))?;
        
        // Read response
        let mut response = Vec::new();
        stream.read_to_end(&mut response)
            .map_err(|e| format!("read response failed: {}", e))?;
        
        // Parse HTTP response
        let response_str = std::str::from_utf8(&response)
            .map_err(|_| "invalid UTF-8 in response")?;
        
        let (headers, body) = response_str.split_once("\r\n\r\n")
            .ok_or("malformed HTTP response")?;
        
        let status_line = headers.lines().next().ok_or("no status line")?;
        if !status_line.contains(" 200 ") {
            return Err(format!("HTTP error: {}", status_line));
        }
        
        let body_bytes = body.as_bytes();
        
        // Verify SHA256
        let mut hasher = Sha256::new();
        hasher.update(body_bytes);
        let hash = hasher.finalize();
        let hash_hex = format!("{:x}", hash);
        
        if hash_hex != expected_hash {
            return Err(format!("hash mismatch: expected {}, got {}", expected_hash, hash_hex));
        }
        
        Ok(body_bytes.to_vec())
    }
    
    fn apply_update(&self, image: &[u8]) -> Result<(), String> {
        // Write the image to the root partition (partition 3 on Pi 4)
        // In Redox, we access block devices via the disk scheme
        // The root partition is typically /dev/disk.mmc.2 (3rd partition, 0-indexed)
        let disk_path = "disk.mmc.2";
        
        // Open the disk scheme for writing
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(disk_path)
            .map_err(|e| format!("failed to open {}: {}", disk_path, e))?;
        
        // Write the full image (this assumes the image is a full disk image with all partitions)
        // For partition-only update, we'd need to seek to the partition offset
        file.write_all(image)
            .map_err(|e| format!("write failed: {}", e))?;
        
        file.sync_all()
            .map_err(|e| format!("sync failed: {}", e))?;
        
        Ok(())
    }
    
    fn execute_shell(&self, cmd: &str) -> Result<String, String> {
        use std::process::Command;
        
        // Execute command via ion shell
        let output = Command::new("ion")
            .arg("-c")
            .arg(cmd)
            .output()
            .map_err(|e| format!("failed to execute shell: {}", e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        if output.status.success() {
            Ok(stdout.to_string())
        } else {
            Err(format!("command failed (exit code: {}): {}", 
                output.status.code().unwrap_or(-1), stderr))
        }
    }
}

impl driver_network::NetworkAdapter for GenetAdapter {
    fn mac_address(&mut self) -> [u8; 6] {
        self.genet.mac()
    }

    fn available_for_read(&mut self) -> usize {
        0
    }

    fn read_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>, syscall::Error> {
        self.genet.poll_rx();
        Ok(None)
    }

    fn write_packet(&mut self, buf: &[u8]) -> Result<usize, syscall::Error> {
        let _ = self.genet.tx_beacon(0, "");
        Ok(buf.len())
    }

    fn ctl_write(&mut self, cmd: &[u8]) -> Result<usize, syscall::Error> {
        self.handle_ctl_command(cmd);
        Ok(cmd.len())
    }

    fn ctl_read(&mut self, buf: &mut [u8]) -> Result<usize, syscall::Error> {
        let response = self.ctl_response.as_bytes();
        let len = response.len().min(buf.len());
        buf[..len].copy_from_slice(&response[..len]);
        self.ctl_response.drain(..len);
        Ok(len)
    }
}

fn main() {
    daemon::Daemon::new(daemon);
}

fn daemon(daemon: Daemon) -> ! {
    common::init();
    common::setup_logging(
        "net",
        "platform",
        "bcm2711-genetd",
        common::output_level(),
        common::file_level(),
    );

    // LED (GPIO 42, active low). Map it early; tolerate failure.
    let led_base = unsafe { common::physmap(GPIO_PHYS, 0x1000, common::Prot::RW, common::MemoryType::DeviceMemory) };
    let led = match led_base {
        Ok(p) => {
            let l = led::Led::new(p as usize);
            l.init();
            Some(l)
        }
        Err(_) => None,
    };
    if let Some(l) = &led {
        l.blink(3, 150, 150); // hello
    }

    // ---------- ETAPA 0 ----------
    mark(&led, 0, "/var/lib/ai/net-00", "daemon iniciou + log salvo");
    thread::sleep(Duration::from_secs(10));

    // ---------- ETAPA 1 ----------
    mark(&led, 1, "/var/lib/ai/net-01", "vivo apos 10s");
    thread::sleep(Duration::from_secs(2));

    // ---------- ETAPA 2: leitura CPRMAN ----------
    let mut info = String::new();
    let mut c250 = (0u32, 0u32);
    let mut c125 = (0u32, 0u32);
    let mut have_cpr = false;

    if let Ok(p) = unsafe { common::physmap(CPRMAN_PHYS, 0x2000, common::Prot::RW, common::MemoryType::DeviceMemory) } {
        let b = p as usize;
        have_cpr = true;
        c250 = (rd(b, 0x1e8), rd(b, 0x1ec));
        c125 = (rd(b, 0x210), rd(b, 0x214));
        info = format!(
            "GENET250={:08x}/{:08x} GENET125={:08x}/{:08x}",
            c250.0, c250.1, c125.0, c125.1
        );
    } else {
        info = "physmap CPRMAN falhou".to_string();
    }
    mark(&led, 2, "/var/lib/ai/net-02", &info);
    thread::sleep(Duration::from_secs(3));

    // ---------- ETAPA 3: escrita CPRMAN ----------
    let need250 = have_cpr && (c250.0 & ENAB == 0 || c250.0 & GATE == 0);
    let need125 = have_cpr && (c125.0 & ENAB == 0 || c125.0 & GATE == 0);
    let mut after;

    if have_cpr && (need250 || need125) {
        let b = unsafe {
            common::physmap(CPRMAN_PHYS, 0x2000, common::Prot::RW, common::MemoryType::DeviceMemory).unwrap() as usize
        };
        if need250 {
            if rd(b, 0x1ec) & DIVI_MASK == 0 {
                wr(b, 0x1ec, PASSWORD | (3u32 << 12));
            }
            let src = if rd(b, 0x1e8) & SRC_MASK == 0 {
                SRC_PLLD_PER
            } else {
                rd(b, 0x1e8) & SRC_MASK
            };
            wr(b, 0x1e8, PASSWORD | src | ENAB | GATE);
            for _ in 0..200 {
                if rd(b, 0x1e8) & BUSY == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        if need125 {
            if rd(b, 0x214) & DIVI_MASK == 0 {
                wr(b, 0x214, PASSWORD | (6u32 << 12));
            }
            let src = if rd(b, 0x210) & SRC_MASK == 0 {
                SRC_PLLD_PER
            } else {
                rd(b, 0x210) & SRC_MASK
            };
            wr(b, 0x210, PASSWORD | src | ENAB | GATE);
            for _ in 0..200 {
                if rd(b, 0x210) & BUSY == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        after = format!(
            "DEPOIS GENET250={:08x}/{:08x} GENET125={:08x}/{:08x}",
            rd(b, 0x1e8),
            rd(b, 0x1ec),
            rd(b, 0x210),
            rd(b, 0x214)
        );
    } else {
        after = "nada a ligar (ja tinham ENAB+GATE ou CPRMAN falhou)".to_string();
    }
    mark(&led, 3, "/var/lib/ai/net-03", &after);
    thread::sleep(Duration::from_secs(10));

    // ---------- ETAPA 4 ----------
    mark(&led, 4, "/var/lib/ai/net-04", "vivo 10s apos escrever CPRMAN");

    // ---------- ETAPA 5: GENET ----------
    let dtb_data = match fs::read("kernel.dtb:") {
        Ok(d) => d,
        Err(_) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(0, "dtb read failed", Some("cannot read kernel.dtb:"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    let fdt = match Fdt::new(&dtb_data) {
        Ok(f) => f,
        Err(_) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(0, "fdt parse failed", Some("Fdt::new failed"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    let genet_node = match fdt.find_compatible(&["brcm,bcm2711-genet-v5"]) {
        Some(n) => n,
        None => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(1, "genet node not found", Some("no compatible brcm,bcm2711-genet-v5"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    let reg = match genet_node.reg().and_then(|mut r| r.next()) {
        Some(r) => r,
        None => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(2, "genet reg missing", Some("no reg property"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };
    let child_addr = reg.starting_address as usize;
    let size = reg.size.unwrap_or(0x10000) as usize;
    let cpu_addr = genet::translate(&fdt, child_addr);

    let base = match unsafe { common::physmap(cpu_addr, size, common::Prot::RW, common::MemoryType::DeviceMemory) } {
        Ok(p) => p as usize,
        Err(_) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(2, "physmap failed", Some("common::physmap GENET failed"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    // MAC from DT (firmware sets local-mac-address).
    let mut mac = [0u8; 6];
    let mut have_mac = false;
    if let Some(prop) = genet_node.property("local-mac-address") {
        let v = prop.value;
        if v.len() >= 6 {
            mac.copy_from_slice(&v[..6]);
            have_mac = true;
        }
    }
    if !have_mac {
        mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    }

    // PHY address + max-speed from DT.
    let (phyaddr, max_speed) = genet::find_phy(&fdt, &genet_node);

    // rxid from phy-mode.
    let rxid = genet_node
        .property("phy-mode")
        .and_then(|p| p.as_str())
        .map(|s| s == "rgmii-rxid")
        .unwrap_or(false);

    match unsafe { common::physmap(GENET_PHYS, 0x1000, common::Prot::RW, common::MemoryType::DeviceMemory) } {
        Ok(p) => {
            let rev = unsafe { core::ptr::read_volatile(p as *const u32) };
            mark(&led, 5, "/var/lib/ai/net-05", &format!("GENET SYS_REV={:08x}", rev));
        }
        Err(e) => {
            mark(&led, 5, "/var/lib/ai/net-05", &format!("physmap GENET falhou ({:?})", e));
        }
    }
    thread::sleep(Duration::from_secs(5));

    // ---------- ETAPA 6: GENET init + beacon loop ----------
    let genet = match Genet::new(base, mac, phyaddr, max_speed, rxid) {
        Ok(g) => g,
        Err(e) => {
            if let Some(l) = &led {
                l.error();
            }
            mark(&led, 6, "/var/lib/ai/net-06", &format!("genet new failed: {e}"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    let mut genet = genet;
    match genet.init(&led.as_ref().unwrap()) {
        Ok(()) => {}
        Err(e) => {
            if let Some(l) = &led {
                l.error();
            }
            mark(&led, 6, "/var/lib/ai/net-06", &format!("genet init failed: {e}"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    }

    mark(&led, 6, "/var/lib/ai/net-06", "sobreviveu ao GENET — registrando network.genet");

    // Register network scheme (M2)
    let adapter = GenetAdapter { genet, ctl_response: String::new(), update_image: None };
    let mut scheme = NetworkScheme::new(
        move || adapter,
        daemon,
        "network.genet".to_string(),
    );
    mark(&led, 7, "/var/lib/ai/net-07", "network.genet registered");

    // Main loop: tick network scheme + beacon
    let mut counter: u64 = 0;
    loop {
        // Process network events
        let _ = scheme.tick();

        // Beacon loop (keep for diagnostics)
        counter += 1;
        let msg = format!(
            "AIOS-BOOT n={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} speed={}",
            counter, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], max_speed
        );
        let _ = scheme.adapter_mut().genet.tx_beacon(counter, &msg);
        scheme.adapter_mut().genet.poll_rx();

        if counter % 10 == 0 {
            breadcrumb::write(4, "beacon loop running", None);
        }
        if let Some(l) = &led {
            l.success();
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn translate(fdt: &Fdt, child_addr: usize) -> usize {
    genet::translate(fdt, child_addr)
}

fn find_phy(fdt: &Fdt, genet: &fdt::node::FdtNode) -> (u32, u32) {
    genet::find_phy(fdt, genet)
}