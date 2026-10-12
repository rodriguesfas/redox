//! iwlwifi network scheme binary for Redox OS
//!
//! This binary runs as a daemon and exposes a network scheme
//! for the Intel WiFi driver, integrating with the netstack.

#![feature(allocator_api)]

extern crate alloc;

use alloc::vec::Vec;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};
use driver_network::{NetworkAdapter, NetworkScheme};
use syscall::{Error, Result};
use iwlwifi::{regs::*, mac::Mac, phy::Phy, mvm::Mvm, fw::Firmware, IwlwifiDevice};
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
    pcid_interface::pci_daemon(run_iwlwifi_daemon);
}

fn run_iwlwifi_daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
    let pci_config = pcid_handle.config();

    let mut name = pci_config.func.name();
    name.push_str("_iwlwifi");

    common::setup_logging(
        "net",
        "pci",
        &name,
        common::output_level(),
        common::file_level(),
    );

    log::info!("iwlwifi network scheme starting for {}", pci_config.func.display());

    let bar0 = unsafe { pcid_handle.map_bar(0, MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance();

    let irq_file = pci_allocate_interrupt_vector(&mut pcid_handle, "iwlwifi");

    let vend_prod: u32 = ((pci_config.func.full_device_id.vendor_id as u32) << 16)
        | (pci_config.func.full_device_id.device_id as u32);

    // Initialize device
    let mut device = match IwlwifiDevice::new(bar0 as usize, vend_prod) {
        Ok(dev) => dev,
        Err(err) => {
            log::error!("Failed to initialize iwlwifi: {}", err);
            panic!("iwlwifi initialization failed: {}", err);
        }
    };

    // Capture mvm_irq_fd before moving device into closure
    let mvm_irq_fd = device.mvm_irq_fd();

    // Create network scheme using driver-network wrapper
    let scheme_name = format!("network_wifi0_{}", name);
    let mut network_scheme = NetworkScheme::new(
        move || IwlwifiNetworkAdapter::new(device),
        daemon,
        scheme_name.clone(),
    );

    // daemon.ready() is called inside NetworkScheme::new

    event::user_data! {
        enum Source {
            Irq,
            Scheme,
            Mvm,
        }
    }

    let mut event_queue = EventQueue::<Source>::new().expect("iwlwifi: Could not create event queue.");
    let _socket = Socket::nonblock().expect("iwlwifi: failed to create socket");

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
            mvm_irq_fd,
            Source::Mvm,
            event::EventFlags::READ,
        )
        .unwrap();

    let all = [Source::Irq, Source::Scheme, Source::Mvm];

    loop {
        // Process events from the event queue
        while let Some(event) = event_queue.next() {
            match event.expect("failed to get next event").user_data {
                Source::Irq => {
                    network_scheme.adapter_mut().handle_irq();
                }
                Source::Mvm => {
                    network_scheme.adapter_mut().handle_mvm_event();
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
                Source::Mvm => {
                    network_scheme.adapter_mut().handle_mvm_event();
                }
                Source::Scheme => {
                    network_scheme.tick().unwrap();
                }
            }
        }
    }
}


/// Network adapter for iwlwifi implementing NetworkAdapter trait
struct IwlwifiNetworkAdapter {
    device: IwlwifiDevice,
    irq_count: AtomicU64,
    associated: bool,
    bssid: [u8; 6],
    current_channel: u8,
    current_band: iwlwifi::phy::Band,
}

impl IwlwifiNetworkAdapter {
    fn new(device: IwlwifiDevice) -> Self {
        Self {
            device,
            irq_count: AtomicU64::new(0),
            associated: false,
            bssid: [0; 6],
            current_channel: 1,
            current_band: iwlwifi::phy::Band::Band2Ghz,
        }
    }

    fn handle_irq(&mut self) {
        self.device.handle_irq();
        self.irq_count.fetch_add(1, Ordering::Relaxed);
    }

    fn handle_mvm_event(&mut self) {
        self.device.handle_mvm_event();
    }
}

impl NetworkAdapter for IwlwifiNetworkAdapter {
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