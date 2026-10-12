//! Debug and statistics for Intel iwlwifi

use core::sync::atomic::{AtomicU64, Ordering};
use crate::regs::*;

pub struct DebugStats {
    irq_count: AtomicU64,
    tx_packets: AtomicU64,
    rx_packets: AtomicU64,
    tx_bytes: AtomicU64,
    rx_bytes: AtomicU64,
    tx_retries: AtomicU64,
    tx_failed: AtomicU64,
    rx_crc_errors: AtomicU64,
    fw_restarts: AtomicU64,
    cmd_timeouts: AtomicU64,
    scan_count: AtomicU64,
    assoc_count: AtomicU64,
    disassoc_count: AtomicU64,
}

impl DebugStats {
    pub fn new() -> Self {
        Self {
            irq_count: AtomicU64::new(0),
            tx_packets: AtomicU64::new(0),
            rx_packets: AtomicU64::new(0),
            tx_bytes: AtomicU64::new(0),
            rx_bytes: AtomicU64::new(0),
            tx_retries: AtomicU64::new(0),
            tx_failed: AtomicU64::new(0),
            rx_crc_errors: AtomicU64::new(0),
            fw_restarts: AtomicU64::new(0),
            cmd_timeouts: AtomicU64::new(0),
            scan_count: AtomicU64::new(0),
            assoc_count: AtomicU64::new(0),
            disassoc_count: AtomicU64::new(0),
        }
    }

    pub fn increment_irq(&self) {
        self.irq_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_tx_packets(&self, bytes: u64) {
        self.tx_packets.fetch_add(1, Ordering::Relaxed);
        self.tx_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn increment_rx_packets(&self, bytes: u64) {
        self.rx_packets.fetch_add(1, Ordering::Relaxed);
        self.rx_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn increment_tx_retries(&self) {
        self.tx_retries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_tx_failed(&self) {
        self.tx_failed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_rx_crc_errors(&self) {
        self.rx_crc_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_fw_restarts(&self) {
        self.fw_restarts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_cmd_timeouts(&self) {
        self.cmd_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_scan_count(&self) {
        self.scan_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_assoc_count(&self) {
        self.assoc_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_disassoc_count(&self) {
        self.disassoc_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> StatsSnapshot {
        StatsSnapshot {
            irq_count: self.irq_count.load(Ordering::Relaxed),
            tx_packets: self.tx_packets.load(Ordering::Relaxed),
            rx_packets: self.rx_packets.load(Ordering::Relaxed),
            tx_bytes: self.tx_bytes.load(Ordering::Relaxed),
            rx_bytes: self.rx_bytes.load(Ordering::Relaxed),
            tx_retries: self.tx_retries.load(Ordering::Relaxed),
            tx_failed: self.tx_failed.load(Ordering::Relaxed),
            rx_crc_errors: self.rx_crc_errors.load(Ordering::Relaxed),
            fw_restarts: self.fw_restarts.load(Ordering::Relaxed),
            cmd_timeouts: self.cmd_timeouts.load(Ordering::Relaxed),
            scan_count: self.scan_count.load(Ordering::Relaxed),
            assoc_count: self.assoc_count.load(Ordering::Relaxed),
            disassoc_count: self.disassoc_count.load(Ordering::Relaxed),
        }
    }

    pub fn print_stats(&self) {
        let s = self.get_stats();
        log::info!("iwlwifi Stats:");
        log::info!("  IRQs: {}", s.irq_count);
        log::info!("  TX: {} packets ({} bytes), retries: {}, failed: {}", s.tx_packets, s.tx_bytes, s.tx_retries, s.tx_failed);
        log::info!("  RX: {} packets ({} bytes), CRC errors: {}", s.rx_packets, s.rx_bytes, s.rx_crc_errors);
        log::info!("  FW restarts: {}, CMD timeouts: {}", s.fw_restarts, s.cmd_timeouts);
        log::info!("  Scans: {}, Assoc: {}, Disassoc: {}", s.scan_count, s.assoc_count, s.disassoc_count);
    }
}

#[derive(Debug, Clone)]
pub struct StatsSnapshot {
    pub irq_count: u64,
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub tx_retries: u64,
    pub tx_failed: u64,
    pub rx_crc_errors: u64,
    pub fw_restarts: u64,
    pub cmd_timeouts: u64,
    pub scan_count: u64,
    pub assoc_count: u64,
    pub disassoc_count: u64,
}

// Debug register access
pub fn read_debug_reg(regs: *mut u32, offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile(regs.add(offset)) }
}

pub fn write_debug_reg(regs: *mut u32, offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile(regs.add(offset), value); }
}

// Debug registers
pub const DBG_TX_FIFO: usize = 0x6000;
pub const DBG_RX_FIFO: usize = 0x6100;
pub const DBG_MVM_STATUS: usize = 0x6200;
pub const DBG_FW_ERROR: usize = 0x6300;
pub const DBG_TEMPERATURE: usize = 0x6400;
pub const DBG_VOLTAGE: usize = 0x6404;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_stats_creation() {
        let stats = DebugStats::new();
        let snapshot = stats.get_stats();
        assert_eq!(snapshot.irq_count, 0);
        assert_eq!(snapshot.tx_packets, 0);
        assert_eq!(snapshot.rx_packets, 0);
    }

    #[test]
    fn test_debug_stats_increment() {
        let stats = DebugStats::new();
        stats.increment_irq();
        stats.increment_tx_packets(1500);
        stats.increment_rx_packets(1000);

        let snapshot = stats.get_stats();
        assert_eq!(snapshot.irq_count, 1);
        assert_eq!(snapshot.tx_packets, 1);
        assert_eq!(snapshot.tx_bytes, 1500);
        assert_eq!(snapshot.rx_packets, 1);
        assert_eq!(snapshot.rx_bytes, 1000);
    }

    #[test]
    fn test_debug_stats_multiple_increments() {
        let stats = DebugStats::new();
        for _ in 0..100 {
            stats.increment_irq();
            stats.increment_tx_packets(1000);
        }

        let snapshot = stats.get_stats();
        assert_eq!(snapshot.irq_count, 100);
        assert_eq!(snapshot.tx_packets, 100);
        assert_eq!(snapshot.tx_bytes, 100_000);
    }
}