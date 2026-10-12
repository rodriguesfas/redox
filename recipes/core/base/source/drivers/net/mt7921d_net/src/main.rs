//! mt7921d network scheme binary for Redox OS
//!
//! This binary runs as a daemon and exposes a network scheme
//! for the MediaTek MT7921 WiFi driver, integrating with the netstack.

#![feature(allocator_api)]

extern crate alloc;

use alloc::vec::Vec;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};
use driver_network::{NetworkAdapter, NetworkScheme};
use syscall::{Error, Result};
use mt7921d::{fw::Firmware, mac::Mac, phy::Phy, mcu::Mcu, regs::*, Mt7921Device};
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
    pcid_interface::pci_daemon(run_mt7921d_daemon);
}

fn run_mt7921d_daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
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

    log::info!("mt7921d network scheme starting for {}", pci_config.func.display());

    let bar0 = unsafe { pcid_handle.map_bar(0, MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance();

    let irq_file = pci_allocate_interrupt_vector(&mut pcid_handle, "mt7921d");

    let vend_prod: u32 = ((pci_config.func.full_device_id.vendor_id as u32) << 16)
        | (pci_config.func.full_device_id.device_id as u32);

    // Initialize device
    let mut device = match Mt7921Device::new(bar0 as usize, vend_prod) {
        Ok(dev) => dev,
        Err(err) => {
            log::error!("Failed to initialize mt7921d: {}", err);
            panic!("mt7921d initialization failed: {}", err);
        }
    };

    // Capture mcu_irq_fd before moving device into closure
    let mcu_irq_fd = device.mcu_irq_fd();

    // Create network scheme using driver-network wrapper
    let scheme_name = format!("network_wifi0_{}", name);
    let mut network_scheme = NetworkScheme::new(
        move || Mt7921NetworkAdapter::new(device),
        daemon,
        scheme_name.clone(),
    );

    // daemon.ready() is called inside NetworkScheme::new

    event::user_data! {
        enum Source {
            Irq,
            Scheme,
            Mcu,
        }
    }

    let mut event_queue = EventQueue::<Source>::new().expect("mt7921d: Could not create event queue.");
    let _socket = Socket::nonblock().expect("mt7921d: failed to create socket");

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
            mcu_irq_fd,
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
                    network_scheme.adapter_mut().handle_irq();
                }
                Source::Mcu => {
                    network_scheme.adapter_mut().handle_mcu_event();
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
                Source::Mcu => {
                    network_scheme.adapter_mut().handle_mcu_event();
                }
                Source::Scheme => {
                    network_scheme.tick().unwrap();
                }
            }
        }
    }
}


/// Network adapter for mt7921d implementing NetworkAdapter trait
struct Mt7921NetworkAdapter {
    device: Mt7921Device,
    irq_count: AtomicU64,
    associated: bool,
    bssid: [u8; 6],
}

impl Mt7921NetworkAdapter {
    fn new(device: Mt7921Device) -> Self {
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

    fn handle_mcu_event(&mut self) {
        self.device.handle_mcu_event();
    }
}

impl NetworkAdapter for Mt7921NetworkAdapter {
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