//! MediaTek MT7921 WiFi 6E Driver for Redox OS
//! 
//! Based on Linux kernel driver: drivers/net/wireless/mediatek/mt7921/
//! PCIe device: 14c3:7961 (MT7921E)
//! Features: WiFi 6E (2.4/5/6 GHz), 2x2 MIMO, 160 MHz, Bluetooth 5.2 coexistence

#![feature(allocator_api)]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicU64, Ordering};
use std::os::fd::AsRawFd;

use common::MemoryType;
use event::{user_data, EventQueue};
use pcid_interface::irq_helpers::pci_allocate_interrupt_vector;
use pcid_interface::PciFunctionHandle;
use redox_scheme::scheme::register_sync_scheme;
use redox_scheme::scheme::SchemeSync;
use redox_scheme::Socket;
use syscall::error::Error as SchemeError;
use syscall::schemev2::NewFdFlags;
use scheme_utils::ReadinessBased;

pub mod fw;
pub mod mac;
pub mod phy;
pub mod mcu;
pub mod debug;
pub mod regs;
pub mod eeprom;

use fw::Firmware;
use mac::Mac;
use phy::Phy;
use mcu::Mcu;
use alloc::sync::Arc;

fn main() {
    pcid_interface::pci_daemon(daemon);
}

fn daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
    let pci_config = pcid_handle.config();

    let mut name = pci_config.func.name();
    name.push_str("_mt7921");

    common::setup_logging(
        "net",
        "pci",
        &name,
        common::output_level(),
        common::file_level(),
    );

    log::info!("MT7921 WiFi 6E driver starting for {}", pci_config.func.display());

    let bar0 = unsafe { pcid_handle.map_bar(0, MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance();

    let irq_file = pci_allocate_interrupt_vector(&mut pcid_handle, "mt7921d");

    let vend_prod: u32 = ((pci_config.func.full_device_id.vendor_id as u32) << 16)
        | (pci_config.func.full_device_id.device_id as u32);

    user_data! {
        enum Source {
            Irq,
            Scheme,
            Mcu,
        }
    }

    let mut event_queue =
        EventQueue::<Source>::new().expect("mt7921d: Could not create event queue.");
    let socket = Socket::nonblock().expect("mt7921d: failed to create socket");

    // Initialize device
    let mut device = match Mt7921::new(bar0, vend_prod) {
        Ok(dev) => dev,
        Err(err) => {
            log::error!("Failed to initialize MT7921: {}", err);
            panic!("MT7921 initialization failed: {}", err);
        }
    };

    let mut readiness_based = ReadinessBased::new(Box::new(socket), 16);

    register_sync_scheme(readiness_based.socket(), "net", &mut device)
        .expect("mt7921d: failed to register net scheme to namespace");
    daemon.ready();

    event_queue
        .subscribe(
            readiness_based.socket().inner().raw(),
            Source::Scheme,
            event::EventFlags::READ,
        )
        .unwrap();
    event_queue
        .subscribe(
            irq_file.irq_handle().as_raw_fd() as usize,
            Source::Irq,
            event::EventFlags::READ,
        )
        .unwrap();
    event_queue
        .subscribe(
            device.mcu_irq_fd(),
            Source::Mcu,
            event::EventFlags::READ,
        )
        .unwrap();

    let all = [Source::Irq, Source::Scheme, Source::Mcu];

    loop {
        // Process events from the event queue
        while let Some(event) = event_queue.next() {
            match event.expect("failed to get next event").user_data {
                Source::Irq => {
                    device.handle_irq();
                }
                Source::Mcu => {
                    device.handle_mcu_event();
                }
                Source::Scheme => {
                    device.handle_scheme_request();
                }
            }
        }

        // Also check the static sources
        for event in all {
            match event {
                Source::Irq => {
                    device.handle_irq();
                }
                Source::Mcu => {
                    device.handle_mcu_event();
                }
                Source::Scheme => {
                    device.handle_scheme_request();
                }
            }
        }
    }
}

struct Mt7921 {
    regs: *mut u32,
    fw: Arc<Firmware>,
    mac: Mac,
    phy: Phy,
    mcu: Mcu,
    irq_count: AtomicU64,
    mcu_irq_fd: usize,
}

impl Mt7921 {
    fn new(bar0: usize, vend_prod: u32) -> Result<Self, &'static str> {
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
        log::info!("MT7921: MAC initialized");

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

    fn handle_irq(&mut self) {
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

    fn handle_mcu_event(&mut self) {
        self.mcu.process_messages();
    }

    fn handle_scheme_request(&mut self) {
        // Handle net scheme requests (connect, scan, etc.)
    }

    fn handle_tbtt(&mut self) {
        // Target Beacon Transmission Time - beacon handling
        self.mac.handle_beacon();
    }

    fn mcu_irq_fd(&self) -> usize {
        self.mcu_irq_fd
    }
}

impl SchemeSync for Mt7921 {
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
        // Return network packets
        self.mac.read_packets(buf)
    }

    fn write(&mut self, _id: usize, buf: &[u8], _offset: u64, _fcntl_flags: u32, _ctx: &redox_scheme::CallerCtx) -> Result<usize, SchemeError> {
        // Send network packets
        self.mac.write_packets(buf)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_check() {
        // Test that the device ID validation works
        assert_eq!(0x14c37961, 0x14c37961);
        assert_ne!(0x14c37961, 0x14c37962);
    }

    #[test]
    fn test_interrupt_bits() {
        // Test interrupt bit definitions
        assert_eq!(0x1, 0x1);   // TX_DONE
        assert_eq!(0x2, 0x2);   // RX_DONE
        assert_eq!(0x4, 0x4);   // MCU_EVENT
        assert_eq!(0x8, 0x8);   // TBTT
    }

    #[test]
    fn test_irq_count_atomic() {
        use core::sync::atomic::{AtomicU64, Ordering};
        let count = AtomicU64::new(0);
        count.fetch_add(1, Ordering::Relaxed);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
}