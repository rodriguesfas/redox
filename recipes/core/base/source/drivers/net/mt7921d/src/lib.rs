//! MediaTek MT7921 WiFi 6E driver library
//!
//! Based on Linux kernel driver: drivers/net/wireless/mediatek/mt7921/
//! PCIe device: 14c3:7961 (MT7921E)
//! Features: WiFi 6E (2.4/5/6 GHz), 2x2 MIMO, 160 MHz, Bluetooth 5.2 coexistence

#![feature(allocator_api)]

extern crate alloc;

pub mod fw;
pub mod mac;
pub mod phy;
pub mod mcu;
pub mod debug;
pub mod eeprom;
pub mod regs;

use alloc::vec::Vec;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};
use std::os::fd::AsRawFd;

use common::MemoryType;
use pcid_interface::irq_helpers::pci_allocate_interrupt_vector;
use pcid_interface::PciFunctionHandle;
use redox_scheme::scheme::register_sync_scheme;
use redox_scheme::scheme::SchemeSync;
use redox_scheme::Socket;
use syscall::error::Error as SchemeError;
use syscall::schemev2::NewFdFlags;
use scheme_utils::ReadinessBased;

use fw::Firmware;
use mac::Mac;
use phy::Phy;
use mcu::Mcu;
use alloc::sync::Arc;

/// MediaTek MT7921 WiFi 6E device
pub struct Mt7921Device {
    regs: *mut u32,
    fw: Arc<Firmware>,
    mac: Mac,
    phy: Phy,
    mcu: Mcu,
    irq_count: AtomicU64,
    mcu_irq_fd: usize,
}

impl Mt7921Device {
    /// Initialize the device
    pub fn new(bar0: usize, vend_prod: u32) -> Result<Self, &'static str> {
        log::info!("MT7921: Initializing device at BAR0={:#x}, vend_prod={:#x}", bar0, vend_prod);

        // Verify device ID
        if vend_prod != 0x14c37961 {
            return Err("Invalid device ID for MT7921");
        }

        let regs = bar0 as *mut u32;

        // Reset device
        unsafe { core::ptr::write_volatile(regs.add(0x0000), 0x1) }; // MT7921_WPDMA_GLO_CFG reset
        core::hint::spin_loop();

        // Initialize firmware
        let fw = Firmware::load("/lib/firmware/mediatek/mt7921_firmware.bin")?;
        log::info!("MT7921: Firmware loaded, version: {}", fw.version());

        // Initialize MCU
        let mcu = Mcu::new(regs, fw.clone())?;
        log::info!("MT7921: MCU initialized");

        // Initialize MAC
        let mut mac = Mac::new(regs, &fw)?;
        crate::mac::upload_firmware_fn(regs, fw.data())?;
        log::info!("MT7921: MAC initialized, firmware running");

        // Initialize PHY
        let phy = Phy::new(regs, &fw)?;
        log::info!("MT7921: PHY initialized");

        // Configure coexistence
        mac.configure_coex()?;
        log::info!("MT7921: Coexistence configured");

        // Enable interrupts
        unsafe { core::ptr::write_volatile(regs.add(0x0200), 0xFFFFFFFF) }; // MT7921_INT_ENABLE

        Ok(Self {
            regs,
            fw,
            mac,
            phy,
            mcu,
            irq_count: AtomicU64::new(0),
            mcu_irq_fd: 0,
        })
    }

    /// Handle interrupt
    pub fn handle_irq(&mut self) {
        let status = unsafe { core::ptr::read_volatile(self.regs.add(0x0204)) }; // MT7921_INT_STATUS
        if status == 0 {
            return;
        }

        self.irq_count.fetch_add(1, Ordering::Relaxed);

        // Acknowledge interrupts
        unsafe { core::ptr::write_volatile(self.regs.add(0x0208), status) }; // MT7921_INT_ACK

        // Handle different interrupt types
        if status & 0x1 != 0 {
            self.mac.handle_tx_done();
        }
        if status & 0x2 != 0 {
            self.mac.handle_rx_done();
        }
        if status & 0x4 != 0 {
            self.mcu.handle_firmware_event();
        }
        if status & 0x8 != 0 {
            self.handle_tbtt();
        }
    }

    /// Handle MCU event
    pub fn handle_mcu_event(&mut self) {
        self.mcu.process_messages();
    }

    /// Handle scheme request
    pub fn handle_scheme_request(&mut self) {
        // Handle net scheme requests (connect, scan, etc.)
    }

    /// Handle TBTT
    pub fn handle_tbtt(&mut self) {
        self.mac.handle_beacon();
    }

    /// Get MCU IRQ file descriptor
    pub fn mcu_irq_fd(&self) -> usize {
        self.mcu_irq_fd
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
    /// Controle: inicia scan (beacons chegam pela RX ring; Link em net80211).
    pub fn start_scan(&mut self) -> Result<(), &'static str> {
        self.mac.start_scan()
    }

    /// Controle: encerra scan; retorna quantidade de BSS descobertos.
    pub fn finish_scan(&mut self) -> Result<usize, &'static str> {
        self.mac.finish_scan()
    }

    /// BSS descobertos na ultima rodada de scan.
    pub fn scan_results(&self) -> &[net80211::link::Bss] {
        self.mac.link().scan_results()
    }

    /// Estado da camada de link (Disconnected/Scanning/Associating/Associated).
    pub fn link_state(&self) -> net80211::link::LinkState {
        self.mac.link().state()
    }

    /// Inicia associacao com um BSS do scan.
    pub fn associate_bss(&mut self, bssid: [u8; 6], essid: &[u8]) -> Result<(), &'static str> {
        self.mac.associate(bssid, essid)
    }

    pub fn is_associated(&self) -> bool {
        self.mac.is_associated()
    }
}

impl SchemeSync for Mt7921Device {
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
                let essid = self.mac.current_essid();
                unsafe { core::ptr::copy_nonoverlapping(essid.as_ptr(), arg as *mut u8, essid.len()) };
                Ok(essid.len())
            }
            _ => Err(SchemeError::new(syscall::EINVAL)),
        }
    }
}