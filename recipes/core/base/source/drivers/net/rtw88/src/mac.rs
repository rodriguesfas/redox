//! MAC layer for Realtek rtw88

use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::sync::Arc;
use syscall::error::Error as SchemeError;
use crate::fw::Firmware;
use crate::regs::*;
use crate::efuse::Efuse;

pub struct Mac {
    regs: *mut u32,
    fw: Arc<Firmware>,
    efuse: Efuse,
    tx_queues: [TxQueue; 8],
    rx_queue: RxQueue,
    mac_addr: [u8; 6],
    current_bssid: [u8; 6],
    current_essid: Vec<u8>,
    associated: bool,
    c2h_queue: Vec<C2hMessage>,
    pm_state: PmState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmState {
    PmActive = 0,
    PmPowerSave = 1,
    PmAuto = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSaveMode {
    Active = 0,
    PowerSave = 1,
    Uapac = 2,
}

pub struct PsAttr {
    pub pm_state: PmState,
    pub uapsd_config: u8,
    pub ps_dtim: u8,
    pub ps_poll_interval: u16,
}

struct TxQueue {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
    queue_id: u8,
}

struct RxQueue {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
}

#[derive(Clone, Copy, Debug)]
struct TxDescriptor {
    addr_low: u32,
    addr_high: u32,
    length: u32,
    flags: u32,
}

#[derive(Clone, Copy, Debug)]
struct RxDescriptor {
    addr_low: u32,
    addr_high: u32,
    length: u32,
    flags: u32,
    rssi: i8,
    rate: u8,
    channel: u8,
}

#[derive(Debug, Clone)]
struct C2hMessage {
    cmd: u32,
    data: Vec<u8>,
}

impl Mac {
    pub fn new(regs: *mut u32, fw: &Arc<Firmware>, efuse: &Efuse) -> Result<Self, &'static str> {
        log::info!("rtw88 MAC: Initializing");

        // Read MAC address from EFUSE
        let mac_addr = efuse.mac_addr();

        let mut tx_queues: [TxQueue; 8] = core::array::from_fn(|i| TxQueue {
            base: unsafe { regs.add(REG_TXQ_BASE + i * 0x100) },
            count: 256,
            head: 0,
            tail: 0,
            queue_id: i as u8,
        });

        let rx_queue = RxQueue {
            base: unsafe { regs.add(REG_RXQ_BASE) },
            count: 512,
            head: 0,
            tail: 0,
        };

        // Configure MAC registers
        unsafe {
            // Enable TX/RX DMA
            core::ptr::write_volatile(regs.add(REG_CR), CR_HCI_TXDMA_EN | CR_HCI_RXDMA_EN | CR_TXDMA_EN | CR_RXDMA_EN | CR_PROTOCOL_EN | CR_SCHEDULE_EN);
            // Configure RX filter
            core::ptr::write_volatile(regs.add(REG_RCR), RCR_APM | RCR_AM | RCR_AB | RCR_APWRMGT | RCR_AMSDU);
            // Set MAC address
            Self::write_mac_address(regs, mac_addr);
            // Set BSSID
            core::ptr::write_volatile(regs.add(REG_BSSID), 0);
            core::ptr::write_volatile(regs.add(REG_BSSID + 4), 0);
        }

        Ok(Self {
            regs,
            fw: fw.clone(),
            efuse: efuse.clone(),
            tx_queues,
            rx_queue,
            mac_addr,
            current_bssid: [0; 6],
            current_essid: Vec::new(),
            associated: false,
            c2h_queue: Vec::new(),
            pm_state: PmState::PmActive,
        })
    }

    fn write_mac_address(regs: *mut u32, addr: [u8; 6]) {
        let low = u32::from_le_bytes([addr[0], addr[1], addr[2], addr[3]]);
        let high = u16::from_le_bytes([addr[4], addr[5]]) as u32;

        unsafe {
            core::ptr::write_volatile(regs.add(REG_MACID), low);
            core::ptr::write_volatile(regs.add(REG_MACID + 4), high);
        }
    }

    pub fn mac_address(&self) -> [u8; 6] {
        self.mac_addr
    }

    pub fn configure_coex(&mut self, coex: &crate::coex::Coex) -> Result<(), &'static str> {
        log::info!("rtw88 MAC: Configuring WiFi/BT coexistence");
        coex.configure_mac(self.regs)?;
        Ok(())
    }

    pub fn handle_tx_done(&mut self) {
        // Hardware concluiu os TX pendentes: consome todos ate o head.
        let head = unsafe { core::ptr::read_volatile(self.regs.add(crate::regs::REG_TXQ_HEAD)) };
        unsafe { core::ptr::write_volatile(self.regs.add(crate::regs::REG_TXQ_TAIL), head) };
    }


    pub fn handle_rx_done(&mut self) {
        // Frames RX ficam na ring ate read_packets() consumir; nada a avancar aqui.
    }


    pub fn handle_beacon(&mut self) {
        if self.associated {
            log::trace!("rtw88 MAC: Beacon handling for BSSID {:02x?}", self.current_bssid);
        }
    }

    pub fn associate(&mut self, bssid: [u8; 6], essid: &[u8]) -> Result<(), &'static str> {
        log::info!("rtw88 MAC: Associating with BSSID {:02x?}, ESSID {:?}", bssid, essid);

        self.current_bssid = bssid;
        self.current_essid = essid.to_vec();
        self.associated = true;

        // Send join BSS command to firmware
        self.send_h2c_cmd(H2C_CMD_JOIN_BSS, &[
            bssid[0], bssid[1], bssid[2], bssid[3], bssid[4], bssid[5],
            essid.len() as u8
        ])?;

        Ok(())
    }

    pub fn disassociate(&mut self) -> Result<(), &'static str> {
        log::info!("rtw88 MAC: Disassociating");

        self.current_bssid = [0; 6];
        self.current_essid.clear();
        self.associated = false;

        // Send leave BSS command to firmware
        self.send_h2c_cmd(H2C_CMD_LEAVE_BSS, &[])?;

        Ok(())
    }

    fn send_h2c_cmd(&self, cmd: u32, data: &[u8]) -> Result<(), &'static str> {
        // Write H2C command to firmware mailbox
        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_HMBOX_H2C), cmd);
            if !data.is_empty() {
                let data_ptr = self.regs.add(REG_HMBOX_EXT) as *mut u8;
                for (i, &byte) in data.iter().enumerate() {
                    core::ptr::write_volatile(data_ptr.add(i), byte);
                }
            }
        }
        Ok(())
    }

    pub fn process_c2h_messages(&mut self) {
        // Read C2H messages from firmware mailbox
        let c2h_cmd = unsafe { core::ptr::read_volatile(self.regs.add(REG_HMBOX_C2H)) };
        let c2h_ext = unsafe { core::ptr::read_volatile(self.regs.add(REG_HMBOX_EXT)) };

        log::trace!("rtw88 MAC: C2H cmd {:#x}, ext {:#x}", c2h_cmd, c2h_ext);

        // Parse C2H message based on command type
        match c2h_cmd & 0xFF {
            C2H_CMD_TX_REPORT => {
                log::trace!("rtw88 MAC: TX report received");
            }
            C2H_CMD_RX_REPORT => {
                log::trace!("rtw88 MAC: RX report received");
            }
            C2H_CMD_BT_INFO => {
                log::trace!("rtw88 MAC: BT info received");
            }
            C2H_CMD_WLAN_INFO => {
                log::trace!("rtw88 MAC: WLAN info received");
            }
            C2H_CMD_BEACON_REPORT => {
                log::trace!("rtw88 MAC: Beacon report received");
            }
            C2H_CMD_SCAN_REPORT => {
                log::trace!("rtw88 MAC: Scan report received");
            }
            _ => {
                log::trace!("rtw88 MAC: Unknown C2H cmd {:#x}", c2h_cmd);
            }
        }
    }

    pub fn read_packets(&mut self, buf: &mut [u8]) -> Result<usize, SchemeError> {
        // Pop de 1 frame da RX ring (descritores em MMIO). Ok(0) se vazio.
        let (head, tail) = unsafe {
            (
                core::ptr::read_volatile(self.regs.add(crate::regs::REG_RXQ_HEAD)),
                core::ptr::read_volatile(self.regs.add(crate::regs::REG_RXQ_TAIL)),
            )
        };
        if head == tail {
            return Ok(0);
        }
        let desc = crate::regs::REG_RXQ_RING + (tail % crate::regs::RING_LEN) as usize * crate::regs::RING_DESC_WORDS;
        let (off_words, len) = unsafe {
            (
                core::ptr::read_volatile(self.regs.add(desc)) as usize,
                core::ptr::read_volatile(self.regs.add(desc + 1)) as usize,
            )
        };
        let n = core::cmp::min(len, buf.len());
        for (i, out) in buf[..n].chunks_mut(4).enumerate() {
            let w = unsafe { core::ptr::read_volatile(self.regs.add(crate::regs::RX_DATA_WINDOW + off_words + i)) };
            let bytes = w.to_le_bytes();
            out.copy_from_slice(&bytes[..out.len()]);
        }
        unsafe {
            core::ptr::write_volatile(self.regs.add(desc), 0);
            core::ptr::write_volatile(self.regs.add(desc + 1), 0);
            core::ptr::write_volatile(self.regs.add(crate::regs::REG_RXQ_TAIL), tail.wrapping_add(1));
        }
        Ok(n)
    }


    pub fn write_packets(&mut self, buf: &[u8]) -> Result<usize, SchemeError> {
        if !self.associated {
            return Err(SchemeError::new(syscall::EWOULDBLOCK));
        }
        if buf.len() > crate::regs::FRAME_SLOT_BYTES {
            return Err(SchemeError::new(syscall::EINVAL));
        }
        let (head, tail) = unsafe {
            (
                core::ptr::read_volatile(self.regs.add(crate::regs::REG_TXQ_HEAD)),
                core::ptr::read_volatile(self.regs.add(crate::regs::REG_TXQ_TAIL)),
            )
        };
        if head.wrapping_sub(tail) >= crate::regs::RING_LEN {
            return Err(SchemeError::new(syscall::ENOBUFS));
        }
        let slot = (head % crate::regs::RING_LEN) as usize;
        let off_words = slot * crate::regs::FRAME_SLOT_WORDS;
        for (i, chunk) in buf.chunks(4).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            unsafe {
                core::ptr::write_volatile(
                    self.regs.add(crate::regs::TX_DATA_WINDOW + off_words + i),
                    u32::from_le_bytes(word),
                );
            }
        }
        let desc = crate::regs::REG_TXQ_RING + slot * crate::regs::RING_DESC_WORDS;
        unsafe {
            core::ptr::write_volatile(self.regs.add(desc), off_words as u32);
            core::ptr::write_volatile(self.regs.add(desc + 1), buf.len() as u32);
            core::ptr::write_volatile(self.regs.add(crate::regs::REG_TXQ_HEAD), head.wrapping_add(1));
        }
        Ok(buf.len())
    }


    pub fn current_essid(&self) -> Vec<u8> {
        if self.associated {
            self.current_essid.clone()
        } else {
            b"RTW88".to_vec()
        }
    }

    pub fn is_associated(&self) -> bool {
        self.associated
    }

    pub fn set_channel(&mut self, channel: u8, band: Band) -> Result<(), &'static str> {
        log::info!("rtw88 MAC: Setting channel {} band {:?}", channel, band);
        // Send channel switch command to firmware
        let freq = Self::channel_to_frequency(channel, band);
        self.send_h2c_cmd(H2C_CMD_CHANNEL_SWITCH, &[
            channel,
            band as u8,
            (freq & 0xFF) as u8,
            ((freq >> 8) & 0xFF) as u8,
        ])?;
        Ok(())
    }

    fn channel_to_frequency(channel: u8, band: Band) -> u32 {
        match band {
            Band::Band2Ghz => {
                if channel == 14 { 2484 } else { 2412 + (channel - 1) as u32 * 5 }
            }
            Band::Band5Ghz => 5180 + (channel - 36) as u32 * 5,
            Band::Band6Ghz => 5955 + (channel - 1) as u32 * 5,
        }
    }

    /// Upload firmware blob to the chip: halt engine, stream image into the
    /// SRAM staging window as LE words (tail zero-padded), latch START and
    /// wait for BUSY to clear. Same contract across the WiFi driver family.
    pub fn upload_firmware(&mut self) -> Result<(), &'static str> {
        let data: &[u8] = self.fw.data();
        do_upload(self.regs, data)
    }
}

/// Firmware upload over the register window, factored out so tests can drive
/// it against a fake BAR without constructing a full `Mac`.
fn do_upload(regs: *mut u32, data: &[u8]) -> Result<(), &'static str> {
    if data.is_empty() {
        return Err("rtw88: empty firmware blob");
    }
    log::info!("rtw88: uploading firmware ({} bytes)", data.len());

    // SAFETY: regs is this device's mapped BAR0.
    unsafe {
        // Halt engine.
        core::ptr::write_volatile(regs.add(REG_FW_CTRL), 0);
        // Stream blob, 32-bit LE words, tail zero-padded.
        for (i, chunk) in data.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            core::ptr::write_volatile(
                regs.add(REG_FW_SRAM_WINDOW + i),
                u32::from_le_bytes(w),
            );
        }
        // Start engine.
        core::ptr::write_volatile(regs.add(REG_FW_CTRL), FW_CTRL_START);
        // Wait for busy to clear (bounded).
        let mut spins = 0usize;
        while core::ptr::read_volatile(regs.add(REG_FW_STATUS)) & FW_STATUS_BUSY != 0 {
            spins += 1;
            if spins > 1_000_000 {
                return Err("rtw88: firmware start timeout");
            }
            core::hint::spin_loop();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mac_address_format() {
        let addr = [0x00, 0xE0, 0x4C, 0x12, 0x34, 0x56];
        assert_eq!(addr[0] & 0x01, 0x00); // Unicast
        assert_eq!(addr[0] & 0x02, 0x00); // Not locally administered
    }

    #[test]
    fn test_band_enum() {
        assert_eq!(Band::Band2Ghz as u8, 0);
        assert_eq!(Band::Band5Ghz as u8, 1);
        assert_eq!(Band::Band6Ghz as u8, 2);
    }

    #[test]
    fn test_tx_descriptor_flags() {
        assert_eq!(TX_DESC_OWN, 0x80000000);
        assert_eq!(TX_DESC_FS, 0x40000000);
        assert_eq!(TX_DESC_LS, 0x20000000);
    }

    #[test]
    fn test_rx_descriptor_flags() {
        assert_eq!(RX_DESC_OWN, 0x80000000);
        assert_eq!(RX_DESC_EOR, 0x40000000);
    }

    #[test]
    fn test_channel_to_frequency() {
        assert_eq!(Mac::channel_to_frequency(1, Band::Band2Ghz), 2412);
        assert_eq!(Mac::channel_to_frequency(36, Band::Band5Ghz), 5180);
        assert_eq!(Mac::channel_to_frequency(1, Band::Band6Ghz), 5955);
    }
}
#[cfg(test)]
mod fw_upload_tests {
    use alloc::vec;

    fn fake_bar() -> (alloc::boxed::Box<[u32]>, *mut u32) {
        let mut v = vec![0u32; 0x4000].into_boxed_slice();
        let p = v.as_mut_ptr();
        (v, p)
    }

    #[test]
    fn upload_streams_blob_and_latches_start() {
        let (_bar, regs) = fake_bar();
        let blob = vec![0x11, 0x22, 0x33, 0x44, 0x55];
        super::do_upload(regs, &blob).expect("upload ok");

        let mut back = alloc::vec::Vec::new();
        for i in 0..2usize {
            let w = unsafe { core::ptr::read_volatile(regs.add(crate::regs::REG_FW_SRAM_WINDOW + i)) };
            back.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(&back[..5], &blob[..]);
        assert_eq!(back[5], 0);

        let ctrl = unsafe { core::ptr::read_volatile(regs.add(crate::regs::REG_FW_CTRL)) };
        assert_eq!(ctrl & crate::regs::FW_CTRL_START, crate::regs::FW_CTRL_START);
    }

    #[test]
    fn upload_rejects_empty() {
        let (_bar, regs) = fake_bar();
        assert!(super::do_upload(regs, &[]).is_err());
        let ctrl = unsafe { core::ptr::read_volatile(regs.add(crate::regs::REG_FW_CTRL)) };
        assert_eq!(ctrl, 0);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;
    use alloc::sync::Arc;

    fn fake_bar() -> (alloc::boxed::Box<[u32]>, *mut u32) {
        let mut v = alloc::vec![0u32; 0xC001].into_boxed_slice();
        let p = v.as_mut_ptr();
        (v, p)
    }

    fn mac_on(regs: *mut u32) -> Mac {
        let efuse = crate::efuse::Efuse::read_efuse(regs);
        let fw = crate::fw::Firmware::load("/nonexistent/rtw88.fw").expect("mock fw");
        Mac::new(regs, &Arc::clone(&fw), &efuse).expect("mac init")
    }

    fn write_word(regs: *mut u32, idx: usize, v: u32) {
        unsafe { core::ptr::write_volatile(regs.add(idx), v) };
    }
    fn read_word(regs: *mut u32, idx: usize) -> u32 {
        unsafe { core::ptr::read_volatile(regs.add(idx)) }
    }

    #[test]
    fn rx_empty_ring_returns_zero() {
        let (_bar, regs) = fake_bar();
        let mut mac = mac_on(regs);
        let mut buf = [0u8; 128];
        assert_eq!(mac.read_packets(&mut buf).unwrap(), 0);
    }

    #[test]
    fn rx_reads_seeded_frame_and_recycles_slot() {
        let (_bar, regs) = fake_bar();
        let mut mac = mac_on(regs);

        let payload = b"RTW88 RX FRAME";
        let off_words = crate::regs::FRAME_SLOT_WORDS * 2; // slot 2
        for (i, chunk) in payload.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            write_word(regs, crate::regs::RX_DATA_WINDOW + off_words + i, u32::from_le_bytes(w));
        }
        write_word(regs, crate::regs::REG_RXQ_RING, off_words as u32);
        write_word(regs, crate::regs::REG_RXQ_RING + 1, payload.len() as u32);
        write_word(regs, crate::regs::REG_RXQ_HEAD, 1);

        let mut buf = [0u8; 1518];
        let n = mac.read_packets(&mut buf).unwrap();
        assert_eq!(n, payload.len());
        assert_eq!(&buf[..n], payload);

        assert_eq!(read_word(regs, crate::regs::REG_RXQ_TAIL), 1);
        assert_eq!(read_word(regs, crate::regs::REG_RXQ_RING), 0);
        assert_eq!(mac.read_packets(&mut buf).unwrap(), 0);
    }

    #[test]
    fn tx_writes_slot_and_bumps_head() {
        let (_bar, regs) = fake_bar();
        let mut mac = mac_on(regs);
        mac.associated = true;

        let frame: alloc::vec::Vec<u8> = alloc::vec![0xCD; 100];
        assert_eq!(mac.write_packets(&frame).unwrap(), 100);
        assert_eq!(read_word(regs, crate::regs::REG_TXQ_HEAD), 1);
        assert_eq!(read_word(regs, crate::regs::REG_TXQ_RING + 1), 100);
        assert_eq!(read_word(regs, crate::regs::TX_DATA_WINDOW), 0xCDCD_CDCD);
    }

    #[test]
    fn tx_gates_e_ring_cheio() {
        let (_bar, regs) = fake_bar();
        let mut mac = mac_on(regs);
        let frame = [1u8; 64];
        assert_eq!(mac.write_packets(&frame).unwrap_err().errno, syscall::EWOULDBLOCK);

        mac.associated = true;
        let big = alloc::vec![0u8; 3000];
        assert_eq!(mac.write_packets(&big).unwrap_err().errno, syscall::EINVAL);

        write_word(regs, crate::regs::REG_TXQ_HEAD, crate::regs::RING_LEN);
        write_word(regs, crate::regs::REG_TXQ_TAIL, 0);
        assert_eq!(mac.write_packets(&frame).unwrap_err().errno, syscall::ENOBUFS);

        // handle_tx_done consome tudo -> ring livre
        mac.handle_tx_done();
        assert_eq!(read_word(regs, crate::regs::REG_TXQ_TAIL), crate::regs::RING_LEN);
        assert!(mac.write_packets(&frame).is_ok());
    }
}
