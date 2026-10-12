//! Realtek rtw88 WiFi 6/6E driver for Redox OS
//!
//! Based on Linux kernel driver: drivers/net/wireless/realtek/rtw88/
//! Supports: RTL8852AE (WiFi 6), RTL8852BE (WiFi 6E), RTL8852CE (WiFi 6)

#![feature(allocator_api)]

extern crate alloc;

pub mod fw;
pub mod mac;
pub mod phy;
pub mod tx;
pub mod rx;
pub mod debug;
pub mod regs;
pub mod coex;
pub mod efuse;

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
use coex::Coex;
use crate::regs::*;

fn main() {
    pcid_interface::pci_daemon(daemon);
}

fn daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
    let pci_config = pcid_handle.config();

    let mut name = pci_config.func.name();
    name.push_str("_rtw88");

    common::setup_logging(
        "net",
        "pci",
        &name,
        common::output_level(),
        common::file_level(),
    );

    log::info!("rtw88 driver starting for {}", pci_config.func.display());

    let bar0 = unsafe { pcid_handle.map_bar(0, MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance();

    let irq_file = pci_allocate_interrupt_vector(&mut pcid_handle, "rtw88");

    let vend_prod: u32 = ((pci_config.func.full_device_id.vendor_id as u32) << 16)
        | (pci_config.func.full_device_id.device_id as u32);

    user_data! {
        enum Source {
            Irq,
            Scheme,
            C2h,
        }
    }

    let mut event_queue =
        EventQueue::<Source>::new().expect("rtw88: Could not create event queue.");
    let socket = Socket::nonblock().expect("rtw88: failed to create socket");

    // Initialize device
    let mut device = match Rtw88::new(bar0, vend_prod) {
        Ok(dev) => dev,
        Err(err) => {
            log::error!("Failed to initialize rtw88: {}", err);
            panic!("rtw88 initialization failed: {}", err);
        }
    };

    let mut readiness_based = ReadinessBased::new(Box::new(socket), 16);

    register_sync_scheme(readiness_based.socket(), "net", &mut device)
        .expect("rtw88: failed to register net scheme to namespace");
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
            device.c2h_irq_fd(),
            Source::C2h,
            event::EventFlags::READ,
        )
        .unwrap();

    let all = [Source::Irq, Source::Scheme, Source::C2h];

    loop {
        // Process events from the event queue
        while let Some(event) = event_queue.next() {
            match event.expect("failed to get next event").user_data {
                Source::Irq => {
                    device.handle_irq();
                }
                Source::C2h => {
                    device.handle_c2h();
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
                Source::C2h => {
                    device.handle_c2h();
                }
                Source::Scheme => {
                    device.handle_scheme_request();
                }
            }
        }
    }
}

struct Rtw88 {
    regs: *mut u32,
    fw: Arc<Firmware>,
    mac: Mac,
    phy: Phy,
    coex: Coex,
    irq_count: core::sync::atomic::AtomicU64,
    c2h_irq_fd: usize,
}

impl Rtw88 {
    fn new(bar0: usize, vend_prod: u32) -> Result<Self, &'static str> {
        log::info!("rtw88: Initializing device at BAR0={:#x}, vend_prod={:#x}", bar0, vend_prod);

        // Verify device ID is supported
        if !Self::is_supported_device(vend_prod) {
            return Err("Unsupported Realtek WiFi device");
        }

        let regs = bar0 as *mut u32;

        // Read EFUSE to get device info
        let efuse = crate::efuse::Efuse::read_efuse(regs);
        log::info!("rtw88: EFUSE read complete");

        // Initialize firmware
        let fw = Firmware::load("/lib/firmware/realtek/rtw88_firmware.bin")?;
        log::info!("rtw88: Firmware loaded, version: {}", fw.version());

        // Initialize MAC
        let mut mac = Mac::new(regs, &fw, &efuse)?;
        mac.upload_firmware()?;
        log::info!("rtw88: MAC initialized, firmware running");

        // Initialize PHY
        let phy = Phy::new(regs, &fw)?;
        log::info!("rtw88: PHY initialized");

        // Initialize Coexistence
        let coex = Coex::new(regs, &fw)?;
        log::info!("rtw88: Coexistence initialized");

        // Enable interrupts
        unsafe { core::ptr::write_volatile(regs.add(REG_HISR0), 0xFFFFFFFF) };

        Ok(Self {
            regs,
            fw,
            mac,
            phy,
            coex,
            irq_count: core::sync::atomic::AtomicU64::new(0),
            c2h_irq_fd: 0,
        })
    }

    fn is_supported_device(vend_prod: u32) -> bool {
        matches!(vend_prod,
            0x10ECC852 |  // RTL8852AE
            0x10ECC853 |  // RTL8852BE
            0x10ECC854 |  // RTL8852CE
            0x10ECC855    // RTL8852BE v2
        )
    }

    fn handle_irq(&mut self) {
        let hisr0 = unsafe { core::ptr::read_volatile(self.regs.add(REG_HISR0)) };
        let hisr1 = unsafe { core::ptr::read_volatile(self.regs.add(REG_HISR1)) };

        if hisr0 == 0 && hisr1 == 0 {
            return;
        }

        self.irq_count.fetch_add(1, core::sync::atomic::Ordering::Relaxed);

        // Acknowledge interrupts
        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_HISR0), hisr0);
            core::ptr::write_volatile(self.regs.add(REG_HISR1), hisr1);
        }

        // Handle different interrupt types
        if hisr0 & HISR0_TX_OK != 0 {
            self.mac.handle_tx_done();
        }
        if hisr0 & HISR0_RX_OK != 0 {
            self.mac.handle_rx_done();
        }
        if hisr0 & HISR0_C2H_CMD != 0 {
            self.handle_c2h();
        }
        if hisr0 & HISR0_TBTT != 0 {
            self.handle_tbtt();
        }
    }

    fn handle_c2h(&mut self) {
        // Handle C2H (CPU to Host) messages from firmware
        self.mac.process_c2h_messages();
    }

    fn handle_scheme_request(&mut self) {
        // Handle net scheme requests (connect, scan, etc.)
    }

    fn handle_tbtt(&mut self) {
        self.mac.handle_beacon();
    }

    fn c2h_irq_fd(&self) -> usize {
        self.c2h_irq_fd
    }
}

impl SchemeSync for Rtw88 {
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
        self.mac.read_packets(buf)
    }

    fn write(&mut self, _id: usize, buf: &[u8], _offset: u64, _fcntl_flags: u32, _ctx: &redox_scheme::CallerCtx) -> Result<usize, SchemeError> {
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
    fn test_supported_devices() {
        assert!(Rtw88::is_supported_device(0x10ECC852)); // RTL8852AE
        assert!(Rtw88::is_supported_device(0x10ECC853)); // RTL8852BE
        assert!(Rtw88::is_supported_device(0x10ECC854)); // RTL8852CE
        assert!(Rtw88::is_supported_device(0x10ECC855)); // RTL8852BE v2
        assert!(!Rtw88::is_supported_device(0x10EC1234)); // Unsupported
    }

    #[test]
    fn test_device_id_constants() {
        assert_eq!(RTL8852AE, 0xC852);
        assert_eq!(RTL8852BE, 0xC853);
        assert_eq!(RTL8852CE, 0xC854);
    }
}