//! ath9k network scheme binary for Redox OS
//!
//! Runs as a PCI daemon exposing a `network_wifi*` scheme backed by the
//! ath9k Atheros WiFi driver library.

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use driver_network::{NetworkAdapter, NetworkScheme};
use event::{user_data, EventQueue};
use syscall::{Error, Result};

use ath9k::{Ath9kDevice, Firmware};
use common::MemoryType;
use pcid_interface::irq_helpers::pci_allocate_interrupt_vector;
use pcid_interface::PciFunctionHandle;
use std::os::fd::AsRawFd;

const ETH_MTU: usize = 1500;

fn main() {
    pcid_interface::pci_daemon(run_ath9k_daemon);
}

fn run_ath9k_daemon(daemon: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
    let pci_config = pcid_handle.config();

    let mut name = pci_config.func.name();
    name.push_str("_ath9k");

    common::setup_logging(
        "net",
        "pci",
        &name,
        common::output_level(),
        common::file_level(),
    );

    log::info!("ath9k network scheme starting for {}", pci_config.func.display());

    let vend_prod: u32 = ((pci_config.func.full_device_id.vendor_id as u32) << 16)
        | (pci_config.func.full_device_id.device_id as u32);

    if !Ath9kDevice::is_supported_device(vend_prod) {
        log::error!("ath9k: unsupported device {vend_prod:#x}");
        panic!("ath9k: unsupported device");
    }

    let bar0 = unsafe { pcid_handle.map_bar(0, MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance();

    let irq_file = pci_allocate_interrupt_vector(&mut pcid_handle, "ath9k");

    // Firmware: real blob when present, mock fallback for bring-up.
    let fw = Firmware::load("/lib/firmware/ath9k/ath9k_fw.bin")
        .expect("ath9k: firmware load failed");
    log::info!("ath9k: firmware ready (version {}, real={})", fw.version(), fw.is_real());

    let device = Ath9kDevice::new(bar0 as *mut u32, fw)
        .expect("ath9k: device init failed");

    let scheme_name = format!("network_wifi0_{}", name);
    let mut network_scheme = NetworkScheme::new(
        move || Ath9kNetAdapter::new(device),
        daemon,
        scheme_name,
    );

    user_data! {
        enum Source {
            Irq,
            Scheme,
        }
    }

    let mut event_queue = EventQueue::<Source>::new().expect("ath9k: could not create event queue");

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

    loop {
        while let Some(event) = event_queue.next() {
            match event.expect("ath9k: event error").user_data {
                Source::Irq => network_scheme.adapter_mut().handle_irq(),
                Source::Scheme => network_scheme.tick().unwrap(),
            }
        }
    }
}

/// NetworkAdapter wrapper adding 802.11 <-> Ethernet frame conversion.
struct Ath9kNetAdapter {
    device: Ath9kDevice,
    irq_count: AtomicU64,
    bssid: [u8; 6],
}

impl Ath9kNetAdapter {
    fn new(device: Ath9kDevice) -> Self {
        Self {
            device,
            irq_count: AtomicU64::new(0),
            bssid: [0; 6],
        }
    }

    fn handle_irq(&mut self) {
        self.device.handle_irq();
        self.irq_count.fetch_add(1, Ordering::Relaxed);
    }
}

impl NetworkAdapter for Ath9kNetAdapter {
    fn mac_address(&mut self) -> [u8; 6] {
        self.device.mac_address()
    }

    fn available_for_read(&mut self) -> usize {
        1 // TODO: real RX ring depth once queues exist
    }

    fn read_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        let mut tmp = [0u8; ETH_MTU + 64];
        let len = match self.device.read_packets(&mut tmp) {
            Ok(Some(len)) => len,
            Ok(None) => return Ok(None),
            Err(_) => return Ok(None),
        };

        match net80211::ieee80211::dot11_to_ethernet(&tmp[..len], self.device.mac_address(), self.bssid) {
            Some(eth) => {
                let n = core::cmp::min(eth.len(), buf.len());
                buf[..n].copy_from_slice(&eth[..n]);
                Ok(Some(n))
            }
            None => Ok(None),
        }
    }

    fn write_packet(&mut self, buf: &[u8]) -> Result<usize> {
        if !self.device.is_associated() {
            return Err(Error::new(syscall::ENOTCONN));
        }
        match net80211::ieee80211::ethernet_to_dot11(buf, self.device.mac_address(), self.bssid) {
            Some(dot11) => self
                .device
                .write_packets(&dot11)
                .map(|_| buf.len().min(ETH_MTU)),
            None => Err(Error::new(syscall::EINVAL)),
        }
    }
}

