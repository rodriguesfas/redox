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

    /// Canal de controle WiFi: "scan" | "connect <essid>" | (leitura) status.
    fn ctl_write(&mut self, cmd: &[u8]) -> Result<usize> {
        let cmd_str = core::str::from_utf8(cmd).map_err(|_| Error::new(syscall::EINVAL))?;
        let cmd_str = cmd_str.trim();

        if cmd_str == "scan" {
            log::info!("mt7921d ctl: scan solicitado");
            self.device.start_scan().map_err(|_| Error::new(syscall::EIO))?;
            // A rodada termina quando o firmware fechar a janela; finish_scan()
            // e chamado pelo handler de evento de scan (IRQ). Aqui apenas iniciamos.
            return Ok(cmd.len());
        }
        if let Some(essid) = cmd_str.strip_prefix("connect ") {
            if essid.is_empty() {
                return Err(Error::new(syscall::EINVAL));
            }
            log::info!("mt7921d ctl: connect {:?}", essid);
            let essid_b = essid.as_bytes();
            let bss = self
                .device
                .scan_results()
                .iter()
                .find(|b| b.essid == essid_b)
                .map(|b| b.bssid)
                .ok_or_else(|| Error::new(syscall::ENOENT))?;
            self.device
                .associate_bss(bss, essid_b)
                .map_err(|_| Error::new(syscall::EIO))?;
            return Ok(cmd.len());
        }
        Err(Error::new(syscall::EINVAL))
    }

    fn ctl_read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let state = match self.device.link_state() {
            net80211::link::LinkState::Disconnected => "Disconnected",
            net80211::link::LinkState::Scanning => "Scanning",
            net80211::link::LinkState::Associating => "Associating",
            net80211::link::LinkState::Associated => "Associated",
        };
        let line = alloc::format!(
            "state={} networks={}\n",
            state,
            self.device.scan_results().len()
        );
        let bytes = line.as_bytes();
        let n = core::cmp::min(bytes.len(), buf.len());
        buf[..n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }
}
#[cfg(test)]
mod ctl_tests {
    use super::*;

    fn fake_adapter() -> (Vec<u32>, *mut u32, Mt7921NetworkAdapter) {
        let mut bar = alloc::vec![0u32; 0x10001];
        let regs = bar.as_mut_ptr();
        let dev = Mt7921Device::new(regs as usize, 0x14c37961).expect("device init");
        let adapter = Mt7921NetworkAdapter::new(dev);
        (bar, regs, adapter)
    }

    fn write_word(regs: *mut u32, idx: usize, v: u32) {
        unsafe { core::ptr::write_volatile(regs.add(idx), v) };
    }

    /// Semeia a RX ring com um frame (simula firmware entregando beacon).
    fn seed_rx(regs: *mut u32, frame: &[u8], head_new: u32, slot: u32) {
        let off_words = slot as usize * FRAME_SLOT_WORDS;
        for (i, chunk) in frame.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            write_word(regs, RX_DATA_WINDOW + off_words + i, u32::from_le_bytes(w));
        }
        let desc = REG_RXQ_RING + (slot as usize) * RING_DESC_WORDS;
        write_word(regs, desc, off_words as u32);
        write_word(regs, desc + 1, frame.len() as u32);
        write_word(regs, REG_RXQ_HEAD, head_new);
    }

    fn beacon_frame(bssid: [u8; 6], ssid: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&0x0080u16.to_le_bytes());
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&[0xFF; 6]);
        f.extend_from_slice(&bssid);
        f.extend_from_slice(&bssid);
        f.extend_from_slice(&0u16.to_le_bytes());
        f.extend_from_slice(&[0u8; 12]);
        f.push(0);
        f.push(ssid.len() as u8);
        f.extend_from_slice(ssid);
        f
    }

    fn ctl_read_string(a: &mut Mt7921NetworkAdapter) -> alloc::string::String {
        let mut buf = [0u8; 128];
        let n = a.ctl_read(&mut buf).unwrap();
        alloc::string::String::from_utf8_lossy(&buf[..n]).into_owned()
    }

    #[test]
    fn ctl_scan_connect_status_com_beacon_real_na_ring() {
        let (_bar, regs, mut a) = fake_adapter();

        // scan via ctl
        assert_eq!(a.ctl_write(b"scan").unwrap(), 4);
        assert!(ctl_read_string(&mut a).contains("state=Scanning"));

        // beacon do AP chega pela RX ring; driver processa via read_packets
        let bssid = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
        seed_rx(regs, &beacon_frame(bssid, b"CafeWifi"), 1, 0);
        let mut scratch = [0u8; 512];
        a.device.read_packets(&mut scratch).unwrap(); // mgmt fica no driver
        a.device.finish_scan().unwrap();
        assert!(ctl_read_string(&mut a).contains("networks=1"));

        // connect sem scan nem com BSSID certo falha antes; com o certo funciona
        assert!(a.ctl_write(b"connect NaoExiste").is_err());
        a.ctl_write(b"connect CafeWifi").unwrap();
        assert!(ctl_read_string(&mut a).contains("state=Associating"));

        // AP aceita: assoc-resp chega pela ring -> Associated
        let sta = a.device.mac_address();
        let mut resp = Vec::new();
        resp.extend_from_slice(&0x0010u16.to_le_bytes());
        resp.extend_from_slice(&0u16.to_le_bytes());
        resp.extend_from_slice(&sta);
        resp.extend_from_slice(&bssid);
        resp.extend_from_slice(&bssid);
        resp.extend_from_slice(&0u16.to_le_bytes());
        resp.extend_from_slice(&1u16.to_le_bytes()); // capab
        resp.extend_from_slice(&0u16.to_le_bytes()); // status OK
        resp.extend_from_slice(&1u16.to_le_bytes()); // AID
        seed_rx(regs, &resp, 2, 1);
        a.device.read_packets(&mut scratch).unwrap();

        assert!(ctl_read_string(&mut a).contains("state=Associated"));
        assert!(a.device.is_associated());
    }
}
