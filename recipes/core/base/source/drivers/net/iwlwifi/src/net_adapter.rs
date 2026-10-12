//! Network adapter for iwlwifi - implements NetworkAdapter trait for driver-network integration

extern crate alloc;

use alloc::vec::Vec;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU64, Ordering};
use driver_network::{NetworkAdapter, NetworkScheme};
use syscall::{Error, Result};
use iwlwifi::{regs::*, mac::Mac, phy::Phy, mvm::Mvm, fw::Firmware};
use alloc::sync::Arc;
use std::os::fd::AsRawFd;

/// Maximum Ethernet frame size
const ETH_MTU: usize = 1500;

/// Network adapter for iwlwifi implementing NetworkAdapter trait
pub struct IwlwifiNetworkAdapter {
    mac: Mac,
    phy: Phy,
    mvm: Mvm,
    fw: Arc<Firmware>,
    irq_count: AtomicU64,
    associated: bool,
    bssid: [u8; 6],
    current_channel: u8,
    current_band: crate::phy::Band,
    mvm_irq_fd: usize,
}

impl IwlwifiNetworkAdapter {
    pub fn new(regs: *mut u32, fw: &Arc<Firmware>) -> Result<Self, &'static str> {
        let mut mac = Mac::new(regs, fw)?;
        crate::mac::upload_firmware_fn(regs, fw.data())?;
        let phy = Phy::new(regs, fw)?;
        let mvm = Mvm::new(regs, fw, &mut mac)?;

        // Enable interrupts
        unsafe { core::ptr::write_volatile(regs.add(CSR_INT_MASK), 0xFFFFFFFF) };

        Ok(Self {
            mac,
            phy,
            mvm,
            fw: fw.clone(),
            irq_count: AtomicU64::new(0),
            associated: false,
            bssid: [0; 6],
            current_channel: 1,
            current_band: crate::phy::Band::Band2Ghz,
            mvm_irq_fd: 0,
        })
    }

    pub fn handle_irq(&mut self) {
        let status = unsafe { core::ptr::read_volatile(self.mac.regs().add(CSR_INT)) };
        if status == 0 {
            return;
        }

        self.irq_count.fetch_add(1, Ordering::Relaxed);

        // Acknowledge interrupts
        unsafe { core::ptr::write_volatile(self.mac.regs().add(CSR_INT_ACK), status) };

        if status & CSR_INT_RX_DONE != 0 {
            self.mac.handle_rx_done();
        }
        if status & CSR_INT_TX_DONE != 0 {
            self.mac.handle_tx_done();
        }
        if status & CSR_INT_MVM != 0 {
            self.mvm.handle_firmware_event();
        }
        if status & CSR_INT_TBTT != 0 {
            self.mac.handle_beacon();
        }
    }

    pub fn handle_mvm_event(&mut self) {
        self.mvm.process_messages();
    }

    pub fn associate(&mut self, bssid: [u8; 6], _essid: &[u8], channel: u8, band: crate::phy::Band) -> Result<(), &'static str> {
        self.mac.associate(bssid, &[])?;
        self.mac.configure_coex()?;
        self.phy.set_channel(channel, band)?;
        self.bssid = bssid;
        self.current_channel = channel;
        self.current_band = band;
        self.associated = true;
        Ok(())
    }

    pub fn disassociate(&mut self) -> Result<(), &'static str> {
        self.mac.disassociate()?;
        self.associated = false;
        self.bssid = [0; 6];
        Ok(())
    }

    fn read_rx_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        // Try to read from MAC layer
        let mut temp_buf = [0u8; ETH_MTU];
        match self.mac.read_packets(&mut temp_buf) {
            Ok(0) => Ok(None),
            Ok(len) => {
                // Convert 802.11 to Ethernet
                if let Some(eth_frame) = net80211::ieee80211::dot11_to_ethernet(
                    &temp_buf[..len],
                    self.mac.mac_address(),
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

    fn write_tx_packet(&mut self, buf: &[u8]) -> Result<usize> {
        if !self.associated {
            return Err(Error::new(syscall::ENOTCONN));
        }

        // Convert Ethernet to 802.11
        if let Some(dot11_frame) = net80211::ieee80211::ethernet_to_dot11(buf, self.mac.mac_address(), self.bssid) {
            self.mac.write_packets(&dot11_frame)
        } else {
            Err(Error::new(syscall::EINVAL))
        }
    }

    pub fn mvm_irq_fd(&self) -> usize {
        self.mvm_irq_fd
    }
}

impl NetworkAdapter for IwlwifiNetworkAdapter {
    fn mac_address(&mut self) -> [u8; 6] {
        self.mac.mac_address()
    }

    fn available_for_read(&mut self) -> usize {
        // Check if RX queue has packets
        if self.mac.rx_queue_has_packets() {
            1
        } else {
            0
        }
    }

    fn read_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        self.read_rx_packet(buf)
    }

    fn write_packet(&mut self, buf: &[u8]) -> Result<usize> {
        self.write_tx_packet(buf)
    }
}

/// Run the iwlwifi driver with NetworkScheme
pub fn run_iwlwifi_network_scheme(daemon: daemon::Daemon, mut pcid_handle: pcid_interface::PciFunctionHandle) -> ! {
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

    let bar0 = unsafe { pcid_handle.map_bar(0, common::MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance();

    let irq_file = pcid_interface::irq_helpers::pci_allocate_interrupt_vector(&mut pcid_handle, "iwlwifi");

    let vend_prod: u32 = ((pci_config.func.full_device_id.vendor_id as u32) << 16)
        | (pci_config.func.full_device_id.device_id as u32);

    // Initialize device
    let fw = Firmware::load("/lib/firmware/intel/iwlwifi_firmware.bin").expect("Failed to load firmware");
    let mut adapter = IwlwifiNetworkAdapter::new(bar0 as *mut u32, &fw).expect("Failed to initialize iwlwifi adapter");

    // Create network scheme
    let scheme_name = format!("network_wifi0_{}", name);
    let mut network_scheme = NetworkScheme::new(
        move || adapter,
        daemon,
        scheme_name,
    );

    // daemon.ready() is called inside NetworkScheme::new

    event::user_data! {
        enum Source {
            Irq,
            Scheme,
            Mvm,
        }
    }

    let mut event_queue = event::EventQueue::<Source>::new().expect("iwlwifi: Could not create event queue.");
    let _socket = redox_scheme::Socket::nonblock().expect("iwlwifi: failed to create socket");

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

/// Main entry point for the iwlwifi network scheme binary
fn main() {
    pcid_interface::pci_daemon(run_iwlwifi_network_scheme);
}