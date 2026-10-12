//! Intel iwlwifi driver library for Redox OS
//!
//! Based on Linux kernel driver: drivers/net/wireless/intel/iwlwifi/
//! Supports: AX200, AX201, AX210, AX211, BE200 (WiFi 6/6E/7)

#![feature(allocator_api)]

extern crate alloc;

pub mod fw;
pub mod pcie;
pub mod mac;
pub mod phy;
pub mod mvm;
pub mod tx;
pub mod rx;
pub mod debug;
pub mod regs;

use alloc::sync::Arc;
use alloc::boxed::Box;
use common::MemoryType;
use event::{user_data, EventQueue};
use pcid_interface::irq_helpers::pci_allocate_interrupt_vector;
use pcid_interface::PciFunctionHandle;
use redox_scheme::scheme::register_sync_scheme;
use redox_scheme::scheme::SchemeSync;
use redox_scheme::Socket;
use std::os::fd::AsRawFd;
use syscall::error::Error as SchemeError;
use syscall::schemev2::NewFdFlags;
use scheme_utils::ReadinessBased;

use fw::Firmware;
use mac::Mac;
use phy::Phy;
use mvm::Mvm;
use crate::regs::*;

/// Intel WiFi device driver
pub struct IwlwifiDevice {
    regs: *mut u32,
    fw: Arc<Firmware>,
    mac: Mac,
    phy: Phy,
    mvm: Mvm,
    irq_count: core::sync::atomic::AtomicU64,
    mvm_irq_fd: usize,
}

impl IwlwifiDevice {
    /// Initialize the device
    pub fn new(bar0: usize, vend_prod: u32) -> Result<Self, &'static str> {
        log::info!("iwlwifi: Initializing device at BAR0={:#x}, vend_prod={:#x}", bar0, vend_prod);

        // Verify device ID is supported
        if !Self::is_supported_device(vend_prod) {
            return Err("Unsupported Intel WiFi device");
        }

        let regs = bar0 as *mut u32;

        // Reset device
        unsafe { core::ptr::write_volatile(regs.add(CSR_RESET), CSR_RESET_BIT) };
        core::hint::spin_loop();

        // Initialize firmware
        let fw = Firmware::load("/lib/firmware/intel/iwlwifi_firmware.bin")?;
        log::info!("iwlwifi: Firmware loaded, version: {}", fw.version());

        // Initialize MAC
        let mut mac = Mac::new(regs, &fw)?;
        log::info!("iwlwifi: MAC initialized");

        // Initialize PHY
        let phy = Phy::new(regs, &fw)?;
        log::info!("iwlwifi: PHY initialized");

        // Initialize MVM (MAC VM)
        let mvm = Mvm::new(regs, &fw, &mut mac)?;
        log::info!("iwlwifi: MVM initialized");

        // Enable interrupts
        unsafe { core::ptr::write_volatile(regs.add(CSR_INT_MASK), 0xFFFFFFFF) };

        Ok(Self {
            regs,
            fw,
            mac,
            phy,
            mvm,
            irq_count: core::sync::atomic::AtomicU64::new(0),
            mvm_irq_fd: 0,
        })
    }

    /// Check if device is supported
    pub fn is_supported_device(vend_prod: u32) -> bool {
        match vend_prod {
            0x80862723 | 0x80862725 => true, // AX210
            0x80867AF0 | 0x80867AF1 => true, // AX200
            0x80867DF0 | 0x80867DF1 => true, // AX201
            0x80862720 | 0x80862721 => true, // AX211
            0x808654F0 | 0x808654F1 => true, // BE200
            _ => false,
        }
    }

    /// Handle interrupt
    pub fn handle_irq(&mut self) {
        let status = unsafe { core::ptr::read_volatile(self.regs.add(CSR_INT)) };
        if status == 0 {
            return;
        }

        self.irq_count.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        // Acknowledge interrupts
        unsafe { core::ptr::write_volatile(self.regs.add(CSR_INT_ACK), status) };

        // Handle different interrupt types
        if status & CSR_INT_TX_DONE != 0 {
            self.mac.handle_tx_done();
        }
        if status & CSR_INT_RX_DONE != 0 {
            self.mac.handle_rx_done();
        }
        if status & CSR_INT_MVM != 0 {
            self.mvm.handle_firmware_event();
        }
        if status & CSR_INT_TBTT != 0 {
            self.handle_tbtt();
        }
    }

    /// Handle MVM event
    pub fn handle_mvm_event(&mut self) {
        self.mvm.process_messages();
    }

    /// Handle scheme request
    pub fn handle_scheme_request(&mut self) {
        // Handle net scheme requests (connect, scan, etc.)
    }

    /// Handle TBTT (Target Beacon Transmission Time)
    pub fn handle_tbtt(&mut self) {
        self.mac.handle_beacon();
    }

    /// Get MVM IRQ file descriptor
    pub fn mvm_irq_fd(&self) -> usize {
        self.mvm_irq_fd
    }

    /// Get MAC address
    pub fn mac_address(&self) -> [u8; 6] {
        self.mac.mac_address()
    }

    /// Read packets
    pub fn read_packets(&mut self, buf: &mut [u8]) -> Result<usize, SchemeError> {
        self.mac.read_packets(buf)
    }

    /// Write packets
    pub fn write_packets(&mut self, buf: &[u8]) -> Result<usize, SchemeError> {
        self.mac.write_packets(buf)
    }

    /// Check if associated
    pub fn is_associated(&self) -> bool {
        self.mac.is_associated()
    }

    /// Get current ESSID
    pub fn current_essid(&self) -> Vec<u8> {
        self.mac.current_essid()
    }

    /// Associate with AP
    pub fn associate(&mut self, bssid: [u8; 6], essid: &[u8]) -> Result<(), &'static str> {
        self.mac.associate(bssid, essid)
    }

    /// Disassociate
    pub fn disassociate(&mut self) -> Result<(), &'static str> {
        self.mac.disassociate()
    }
}

impl SchemeSync for IwlwifiDevice {
    fn openat(
        &mut self,
        _fd: usize,
        _path: &str,
        _flags: usize,
        _fcntl_flags: u32,
        _ctx: &redox_scheme::CallerCtx,
    ) -> Result<redox_scheme::OpenResult, SchemeError> {
        Ok(redox_scheme::OpenResult::ThisScheme { number: 0, flags: NewFdFlags::empty() })
    }

    fn read(&mut self, _id: usize, buf: &mut [u8], _offset: u64, _fcntl_flags: u32, _ctx: &redox_scheme::CallerCtx) -> Result<usize, SchemeError> {
        self.read_packets(buf)
    }

    fn write(&mut self, _id: usize, buf: &[u8], _offset: u64, _fcntl_flags: u32, _ctx: &redox_scheme::CallerCtx) -> Result<usize, SchemeError> {
        self.write_packets(buf)
    }

    fn fcntl(&mut self, _id: usize, cmd: usize, arg: usize, _ctx: &redox_scheme::CallerCtx) -> Result<usize, SchemeError> {
        match cmd {
            0x8900 => { // SIOCGIWNAME
                let name = b"IEEE 802.11\0";
                unsafe { core::ptr::copy_nonoverlapping(name.as_ptr(), arg as *mut u8, name.len()) };
                Ok(name.len())
            }
            0x8910 => { // SIOCGIWESSID
                let essid = self.current_essid();
                unsafe { core::ptr::copy_nonoverlapping(essid.as_ptr(), arg as *mut u8, essid.len()) };
                Ok(essid.len())
            }
            0x8927 => { // SIOCGIWFREQ (channel in hundreds of kHz)
                let channel = self.phy.current_channel() as u32;
                let freq = match channel {
                    1 => 2412,
                    2 => 2417,
                    3 => 2422,
                    4 => 2427,
                    5 => 2432,
                    6 => 2437,
                    7 => 2442,
                    8 => 2447,
                    9 => 2452,
                    10 => 2457,
                    11 => 2462,
                    12 => 2467,
                    13 => 2472,
                    14 => 2484,
                    _ => 2412,
                };
                unsafe { core::ptr::write(arg as *mut u32, freq) };
                Ok(0)
            }
            _ => Err(SchemeError::new(syscall::EINVAL)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_devices() {
        assert!(IwlwifiDevice::is_supported_device(0x80862723)); // AX210
        assert!(IwlwifiDevice::is_supported_device(0x80867AF0)); // AX200
        assert!(IwlwifiDevice::is_supported_device(0x80867DF0)); // AX201
        assert!(IwlwifiDevice::is_supported_device(0x80862720)); // BE200
        assert!(!IwlwifiDevice::is_supported_device(0x80861234)); // Unsupported
    }
}