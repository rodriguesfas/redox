//! rtw88 network scheme binary for Redox OS
//!
//! This binary runs as a daemon and exposes a network scheme
//! for the Realtek WiFi driver, integrating with the netstack.

#![feature(allocator_api)]

extern crate alloc;

use alloc::vec::Vec;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};
use driver_network::{NetworkAdapter, NetworkScheme};
use syscall::{Error, Result};
use rtw88::{fw::Firmware, mac::Mac, phy::Phy, coex::Coex, regs::*};
use alloc::sync::Arc;
use std::os::fd::AsRawFd;

use common::MemoryType;
use event::{user_data, EventQueue};
use pcid_interface::irq_helpers::pci_allocate_interrupt_vector;
use pcid_interface::PciFunctionHandle;
use redox_scheme::Socket;
use scheme_utils::ReadinessBased;

/// Maximum Ethernet frame size
const ETH_MTU: usize = 1500;

/// Main entry point
fn main() {
    pcid_interface::pci_daemon(run_rtw88_daemon);
}

fn run_rtw88_daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
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

    log::info!("rtw88 network scheme starting for {}", pci_config.func.display());

    let bar0 = unsafe { pcid_handle.map_bar(0, MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance();

    let irq_file = pci_allocate_interrupt_vector(&mut pcid_handle, "rtw88");

    let vend_prod: u32 = ((pci_config.func.full_device_id.vendor_id as u32) << 16)
        | (pci_config.func.full_device_id.device_id as u32);

    // Initialize device
    let mut device = match Rtw88Device::new(bar0 as usize, vend_prod) {
        Ok(dev) => dev,
        Err(err) => {
            log::error!("Failed to initialize rtw88: {}", err);
            panic!("rtw88 initialization failed: {}", err);
        }
    };

    // Capture c2h_irq_fd before moving device into closure
    let c2h_irq_fd = device.c2h_irq_fd();

    // Create network scheme using driver-network wrapper
    let scheme_name = format!("network_wifi0_{}", name);
    let mut network_scheme = NetworkScheme::new(
        move || Rtw88NetworkAdapter::new(device),
        daemon,
        scheme_name.clone(),
    );

    // daemon.ready() is called inside NetworkScheme::new

    event::user_data! {
        enum Source {
            Irq,
            Scheme,
            C2h,
        }
    }

    let mut event_queue = EventQueue::<Source>::new().expect("rtw88: Could not create event queue.");
    let _socket = Socket::nonblock().expect("rtw88: failed to create socket");

    event_queue
        .subscribe(
            network_scheme.event_handle().raw(),
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
            c2h_irq_fd,
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
                    network_scheme.adapter_mut().handle_irq();
                }
                Source::C2h => {
                    network_scheme.adapter_mut().handle_c2h();
                }
                Source::Scheme => {
                    network_scheme.tick().unwrap();
                }
            }
        }

        // Also check the static sources
        for event in all {
            match event {
                Source::Irq => {
                    network_scheme.adapter_mut().handle_irq();
                }
                Source::C2h => {
                    network_scheme.adapter_mut().handle_c2h();
                }
                Source::Scheme => {
                    network_scheme.tick().unwrap();
                }
            }
        }
    }
}

/// Realtek WiFi device driver
struct Rtw88Device {
    regs: *mut u32,
    fw: Arc<Firmware>,
    mac: Mac,
    phy: Phy,
    coex: rtw88::coex::Coex,
    irq_count: AtomicU64,
    c2h_irq_fd: usize,
}

impl Rtw88Device {
    fn new(bar0: usize, vend_prod: u32) -> Result<Self, &'static str> {
        log::info!("rtw88: Initializing device at BAR0={:#x}, vend_prod={:#x}", bar0, vend_prod);

        // Verify device ID is supported
        if !Self::is_supported_device(vend_prod) {
            return Err("Unsupported Realtek WiFi device");
        }

        let regs = bar0 as *mut u32;

        // Read EFUSE to get device info
        let efuse = rtw88::efuse::Efuse::read_efuse(regs);
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
        let coex = rtw88::coex::Coex::new(regs, &fw)?;
        log::info!("rtw88: Coexistence initialized");

        // Enable interrupts
        unsafe { core::ptr::write_volatile(regs.add(REG_HISR0), 0xFFFFFFFF) };

        Ok(Self {
            regs,
            fw,
            mac,
            phy,
            coex,
            irq_count: AtomicU64::new(0),
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

        self.irq_count.fetch_add(1, Ordering::Relaxed);

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
        self.mac.process_c2h_messages();
    }

    fn handle_tbtt(&mut self) {
        self.mac.handle_beacon();
    }

    fn c2h_irq_fd(&self) -> usize {
        self.c2h_irq_fd
    }

    fn mac_address(&self) -> [u8; 6] {
        self.mac.mac_address()
    }

    fn read_packets(&mut self, buf: &mut [u8]) -> Result<usize, syscall::error::Error> {
        self.mac.read_packets(buf)
    }

    fn write_packets(&mut self, buf: &[u8]) -> Result<usize, syscall::error::Error> {
        self.mac.write_packets(buf)
    }

    fn is_associated(&self) -> bool {
        self.mac.is_associated()
    }
}


/// Network adapter for rtw88 implementing NetworkAdapter trait
struct Rtw88NetworkAdapter {
    device: Rtw88Device,
    irq_count: AtomicU64,
    associated: bool,
    bssid: [u8; 6],
}

impl Rtw88NetworkAdapter {
    fn new(device: Rtw88Device) -> Self {
        Self {
            device,
            irq_count: AtomicU64::new(0),
            associated: false,
            bssid: [0; 6],
        }
    }

    fn handle_irq(&mut self) {
        self.device.handle_irq();
        self.irq_count.fetch_add(1, Ordering::Relaxed);
    }

    fn handle_c2h(&mut self) {
        self.device.handle_c2h();
    }
}

impl NetworkAdapter for Rtw88NetworkAdapter {
    fn mac_address(&mut self) -> [u8; 6] {
        self.device.mac_address()
    }

    fn available_for_read(&mut self) -> usize {
        // Check if RX queue has packets
        1 // Placeholder - would check actual queue depth
    }

    fn read_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        let mut temp_buf = [0u8; ETH_MTU];
        match self.device.read_packets(&mut temp_buf) {
            Ok(0) => Ok(None),
            Ok(len) => {
                // Convert 802.11 to Ethernet
                if let Some(eth_frame) = net80211::ieee80211::dot11_to_ethernet(
                    &temp_buf[..len],
                    self.device.mac_address(),
                    self.bssid
                ) {
                    let copy_len = core::cmp::min(eth_frame.len(), buf.len());
                    buf[..copy_len].copy_from_slice(&eth_frame[..copy_len]);
                    Ok(Some(copy_len))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None),
        }
    }

    fn write_packet(&mut self, buf: &[u8]) -> Result<usize> {
        if !self.device.is_associated() {
            return Err(Error::new(syscall::ENOTCONN));
        }

        // Convert Ethernet to 802.11
        if let Some(dot11_frame) = net80211::ieee80211::ethernet_to_dot11(buf, self.device.mac_address(), self.bssid) {
            self.device.write_packets(&dot11_frame)
        } else {
            Err(Error::new(syscall::EINVAL))
        }
    }
}