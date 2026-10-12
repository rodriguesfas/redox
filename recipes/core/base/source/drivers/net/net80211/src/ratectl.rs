//! Rate Control - Minstrel-HT implementation
//!
//! Minstrel-HT is a rate control algorithm for 802.11n/ac/ax.
//! It tracks per-rate statistics and selects the best rate based on
//   throughput probability.

use alloc::vec::Vec;
use core::time::Duration;

/// Number of supported rates (legacy + MCS)
pub const MAX_RATES: usize = 32;

/// Maximum number of spatial streams
pub const MAX_STREAMS: usize = 4;

/// Rate information
#[derive(Clone, Copy, Debug)]
pub struct RateInfo {
    pub idx: u8,              // Rate index
    pub phy: PhyMode,         // PHY mode (CCK, OFDM, HT, VHT, HE, EHT)
    pub mcs: u8,              // MCS index
    pub nss: u8,              // Number of spatial streams (1-8)
    pub bw: Bandwidth,        // Channel bandwidth
    pub sgi: bool,            // Short GI
    pub ldpc: bool,           // LDPC coding
    pub stbc: bool,           // STBC
    pub bitrate: u16,         // Bitrate in 100 kbps (500 = 50 Mbps)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PhyMode {
    CCK = 0,
    OFDM = 1,
    HT = 2,
    VHT = 3,
    HE = 4,
    EHT = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Bandwidth {
    BW20 = 0,
    BW40 = 1,
    BW80 = 2,
    BW160 = 3,
    BW320 = 4, // EHT 320MHz
}

/// Per-rate statistics for Minstrel
#[derive(Clone, Copy, Debug, Default)]
pub struct RateStats {
    pub attempts: u32,
    pub successes: u32,
    pub retries: u32,
    pub last_success: u64,
    pub last_attempt: u64,
    pub ewma_prob: u16,        // EWMA of success probability (0-65535)
    pub ewma_throughput: u16,  // EWMA of throughput (kbps * 10)
    pub last_update: u64,      // Timestamp of last update
    pub prob_ewma: u16,        // EWMA of success probability
    pub throughput_ewma: u16,  // EWMA of throughput
}

impl RateStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, success: bool, now: u64, throughput_kbps: u16) {
        self.attempts = self.attempts.saturating_add(1);
        if success {
            self.successes = self.successes.saturating_add(1);
        } else {
            self.retries = self.retries.saturating_add(1);
        }
        self.last_attempt = self.last_attempt.saturating_add(1);

        // EWMA update (alpha = 0.75)
        let alpha = 0.75;
        let success_rate = if success { 65535 } else { 0 };
        self.prob_ewma = (self.prob_ewma as f32 * (1.0 - 0.75) + (success_rate as f32 * 0.75)) as u16;

        if success {
            self.ewma_throughput = (self.ewma_throughput as f32 * (1.0 - 0.75) + (throughput_kbps as f32 * 0.75)) as u16;
        }

        self.last_update = now;
    }

    pub fn success_prob(&self) -> f32 {
        self.prob_ewma as f32 / 65535.0
    }

    pub fn throughput_kbps(&self) -> u16 {
        self.ewma_throughput
    }

    pub fn expected_throughput(&self) -> u16 {
        (self.prob_ewma as u32 * self.ewma_throughput as u32 / 65535) as u16
    }
}

/// Per-rate statistics table
#[derive(Default)]
pub struct RateTable {
    pub rates: [RateStats; 32],
    pub n_rates: usize,
    pub lookaround_rate: usize,
    pub lookaround_time: u64,
}

impl RateTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rate(&mut self, info: RateInfo) -> usize {
        if self.n_rates < 32 {
            let idx = self.n_rates;
            self.n_rates += 1;
            idx
        } else {
            31
        }
    }
}

/// Minstrel-HT rate control algorithm
pub struct Minstrel {
    table: RateTable,
    current_rate: usize,
    next_rate: usize,
    lookaround_rate: usize,
    lookaround_time: u64,
    last_update: u64,
    update_interval: core::time::Duration,
    sample_interval: core::time::Duration,
    lookaround_interval: core::time::Duration,
}

impl Minstrel {
    pub fn new(rates: alloc::vec::Vec<RateInfo>) -> Self {
        let mut table = RateTable::new();
        for rate in rates {
            table.add_rate(rate);
        }
        Self {
            table,
            current_rate: 0,
            next_rate: 0,
            lookaround_rate: 0,
            lookaround_time: 0,
            last_update: 0,
            update_interval: core::time::Duration::from_millis(100),
            sample_interval: core::time::Duration::from_millis(20),
            lookaround_interval: core::time::Duration::from_millis(1000),
        }
    }

    /// Select the best rate for a frame
    pub fn get_rate(&mut self, now: core::time::Duration) -> usize {
        let now_ms = now.as_millis() as u64;

        // Check if it's time to update statistics
        if now_ms.saturating_sub(self.last_update) >= self.update_interval.as_millis() as u64 {
            self.update_rates();
            self.last_update = now.as_millis() as u64;
        }

        // Lookaround logic: occasionally try other rates
        if now_ms.saturating_sub(self.lookaround_time) >= self.lookaround_interval.as_millis() as u64 {
            self.lookaround_rate = (self.lookaround_rate + 1) % self.table.n_rates;
            self.lookaround_time = now_ms;
            return self.lookaround_rate;
        }

        self.current_rate
    }

    fn update_rates(&mut self) {
        // Find best rate based on expected throughput
        let mut best_idx = 0;
        let mut best_tp = 0;

        for i in 0..self.table.n_rates {
            let tp = self.table.rates[i].expected_throughput();
            if tp > best_tp {
                best_tp = tp;
                best_idx = i;
            }
        }

        self.current_rate = best_idx;

        // Select next rate for sampling (lookaround)
        self.lookaround_rate = (self.lookaround_rate + 1) % self.table.n_rates;
    }

    /// Record transmission result
    pub fn record_tx(&mut self, rate_idx: usize, success: bool, now: core::time::Duration, throughput_kbps: u16) {
        if rate_idx < self.table.n_rates {
            self.table.rates[rate_idx].update(success, now.as_millis() as u64, throughput_kbps);
        }
    }

    /// Get current best rate
    pub fn best_rate(&self) -> Option<usize> {
        if self.table.n_rates == 0 {
            return None;
        }
        Some(self.current_rate)
    }

    /// Get current rate info
    pub fn current_rate_info(&self) -> Option<RateInfo> {
        if self.current_rate < 32 {
            Some(RateInfo {
                idx: self.current_rate as u8,
                phy: PhyMode::HT,
                mcs: self.current_rate as u8,
                nss: 1,
                bw: Bandwidth::BW20,
                sgi: false,
                ldpc: false,
                stbc: false,
                bitrate: 6500,
            })
        } else {
            None
        }
    }
}

/// Minimal rate control wrapper for Redox
pub struct RateControl {
    minstrel: Minstrel,
    rates: alloc::vec::Vec<RateInfo>,
}

impl RateControl {
    pub fn new() -> Self {
        let rates = Self::default_rates();
        Self {
            minstrel: Minstrel::new(rates),
            rates: Self::default_rates(),
        }
    }

    fn default_rates() -> alloc::vec::Vec<RateInfo> {
        // Legacy rates
        let mut rates = alloc::vec::Vec::new();
        rates.extend([
            RateInfo { idx: 0, phy: PhyMode::CCK, mcs: 0, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 1000 },
            RateInfo { idx: 1, phy: PhyMode::CCK, mcs: 1, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 2000 },
            RateInfo { idx: 2, phy: PhyMode::CCK, mcs: 2, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 5500 },
            RateInfo { idx: 3, phy: PhyMode::CCK, mcs: 3, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 11000 },
            // OFDM rates
            RateInfo { idx: 4, phy: PhyMode::OFDM, mcs: 0, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 6000 },
            RateInfo { idx: 5, phy: PhyMode::OFDM, mcs: 1, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 9000 },
            RateInfo { idx: 6, phy: PhyMode::OFDM, mcs: 2, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 12000 },
            RateInfo { idx: 7, phy: PhyMode::OFDM, mcs: 3, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 18000 },
            RateInfo { idx: 8, phy: PhyMode::OFDM, mcs: 4, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 24000 },
            RateInfo { idx: 9, phy: PhyMode::OFDM, mcs: 5, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 36000 },
            RateInfo { idx: 10, phy: PhyMode::OFDM, mcs: 6, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 48000 },
            RateInfo { idx: 11, phy: PhyMode::OFDM, mcs: 7, nss: 1, bw: Bandwidth::BW20, sgi: false, ldpc: false, stbc: false, bitrate: 54000 },
        ]);

        // HT MCS rates (MCS 0-7, 1 spatial stream)
        for mcs in 0..8 {
            rates.push(RateInfo {
                idx: (12 + mcs) as u8,
                phy: PhyMode::HT,
                mcs: mcs as u8,
                nss: 1,
                bw: Bandwidth::BW20,
                sgi: false,
                ldpc: false,
                stbc: false,
                bitrate: 6500 + (mcs as u16 * 800),
            });
        }

        // HT 2 spatial streams
        for mcs in 8..16 {
            rates.push(RateInfo {
                idx: (12 + mcs) as u8,
                phy: PhyMode::HT,
                mcs: mcs as u8,
                nss: 2,
                bw: Bandwidth::BW20,
                sgi: false,
                ldpc: false,
                stbc: false,
                bitrate: 13000 + ((mcs - 8) as u16 * 1500),
            });
        }

        // VHT rates (simplified)
        for mcs in 0..10 {
            for nss in 1..=4 {
                rates.push(RateInfo {
                    idx: (28 + mcs + (nss - 1) * 10) as u8,
                    phy: PhyMode::VHT,
                    mcs: mcs as u8,
                    nss,
                    bw: Bandwidth::BW20,
                    sgi: false,
                    ldpc: false,
                    stbc: false,
                    bitrate: 1000,
                });
            }
        }

        rates
    }

    pub fn get_rate(&mut self, now_ms: u64) -> usize {
        self.minstrel.get_rate(core::time::Duration::from_millis(now_ms))
    }

    pub fn record_tx(&mut self, rate_idx: usize, success: bool, now_ms: u64, throughput_kbps: u16) {
        self.minstrel.record_tx(rate_idx, success, core::time::Duration::from_millis(now_ms), throughput_kbps);
    }

    pub fn best_rate(&self) -> Option<usize> {
        self.minstrel.best_rate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_control() {
        let mut rc = RateControl::new();
        assert!(rc.minstrel.table.n_rates > 0);

        // Test rate selection
        let rate = rc.get_rate(1000);
        assert!(rate < rc.rates.len());

        // Test recording
        rc.record_tx(0, true, 1000, 1000);
        rc.record_tx(0, false, 1001, 0);
    }
}
