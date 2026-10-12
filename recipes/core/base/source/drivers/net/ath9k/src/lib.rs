//! Atheros ath9k WiFi driver library for Redox OS.
//!
//! Supports: AR9271, AR9280, AR9285, AR9287, AR9380, AR9382, AR9462.
//! Loosely based on Linux `drivers/net/wireless/ath/ath9k`.

#![feature(allocator_api)]

extern crate alloc;

pub mod fw;
pub mod mac;
pub mod phy;
pub mod regs;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

pub use fw::Firmware;
pub use mac::{Mac, PmState, PowerSaveMode, PsAttr};
pub use phy::Phy;
pub use regs::*;

/// Supported Atheros PCI vendor/product IDs (vendor << 16 | device).
pub const SUPPORTED_DEVICES: &[u32] = &[
    0x168C_002B, // AR9271 (USB)
    0x168C_0029, // AR9287
    0x168C_0030, // AR9280
    0x168C_0031, // AR9285
    0x168C_0032, // AR9287 v2
    0x168C_0033, // AR9380
    0x168C_0034, // AR9382
    0x168C_0035, // AR9462
];

/// Atheros ath9k WiFi device.
pub struct Ath9kDevice {
    regs: *mut u32,
    fw: Arc<Firmware>,
    mac: Mac,
    phy: Phy,
    irq_count: AtomicUsize,
}

// The BAR is a per-device mapping owned by this driver instance.
unsafe impl Send for Ath9kDevice {}

impl Ath9kDevice {
    /// Initialize the device on an already-mapped BAR0.
    pub fn new(regs: *mut u32, fw: Arc<Firmware>) -> Result<Self, &'static str> {
        log::info!("ath9k: initializing device at regs={:#x}", regs as usize);

        let mut mac = Mac::new(regs, &fw)?;
        mac.upload_firmware(&fw)?;
        let phy = Phy::new(regs)?;

        Ok(Self {
            regs,
            fw,
            mac,
            phy,
            irq_count: AtomicUsize::new(0),
        })
    }

    /// Check if a PCI vendor/product pair is a supported ath9k device.
    pub fn is_supported_device(vend_prod: u32) -> bool {
        SUPPORTED_DEVICES.contains(&vend_prod)
    }

    /// Handle one hardware interrupt.
    pub fn handle_irq(&mut self) {
        // SAFETY: regs points to the mapped BAR0 of this device.
        let status = unsafe { core::ptr::read_volatile(self.regs.add(REG_INT_STATUS)) };
        if status == 0 {
            return;
        }

        self.irq_count.fetch_add(1, Ordering::Relaxed);

        // Acknowledge interrupts.
        unsafe { core::ptr::write_volatile(self.regs.add(REG_INT_CLEAR), status) };

        if status & INT_TX_DONE != 0 {
            self.mac.handle_tx_done();
        }
        if status & INT_RX_DONE != 0 {
            self.mac.handle_rx_done();
        }
        if status & INT_TBTT != 0 {
            self.mac.handle_beacon();
        }
    }

    pub fn is_associated(&self) -> bool {
        self.mac.is_associated()
    }

    pub fn pm_state(&self) -> PmState {
        self.mac.pm_state()
    }

    pub fn set_power_save(&mut self, mode: PowerSaveMode) {
        self.mac.set_power_save(mode);
    }

    pub fn irq_count(&self) -> usize {
        self.irq_count.load(Ordering::Relaxed)
    }

    /// Station MAC address.
    pub fn mac_address(&self) -> [u8; 6] {
        self.mac.mac_address()
    }

    /// Non-blocking read of one 802.11 frame; Ok(None) when RX is empty.
    pub fn read_packets(&mut self, buf: &mut [u8]) -> Result<Option<usize>, syscall::error::Error> {
        self.mac.read_packet(buf)
    }

    /// Write one 802.11 frame. ENOTCONN-style error when not associated.
    pub fn write_packets(&mut self, buf: &[u8]) -> Result<usize, syscall::error::Error> {
        self.mac.write_packet(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_devices_table() {
        for &id in SUPPORTED_DEVICES {
            assert!(Ath9kDevice::is_supported_device(id));
        }
        assert!(!Ath9kDevice::is_supported_device(0x168C_0000));
        assert!(!Ath9kDevice::is_supported_device(0x8086_1234)); // Intel, not Atheros
    }

    #[test]
    fn pm_state_values() {
        assert_eq!(PmState::PmActive as u8, 0);
        assert_eq!(PmState::PmPowerSave as u8, 1);
        assert_eq!(PmState::PmAuto as u8, 2);
    }

    #[test]
    fn power_save_mode_values() {
        assert_eq!(PowerSaveMode::Active as u8, 0);
        assert_eq!(PowerSaveMode::PowerSave as u8, 1);
        assert_eq!(PowerSaveMode::Uapac as u8, 2);
    }
}
