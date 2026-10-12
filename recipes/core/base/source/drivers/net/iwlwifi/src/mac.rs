//! MAC layer for Intel iwlwifi

use alloc::vec::Vec;
use alloc::boxed::Box;
use alloc::sync::Arc;
use syscall::error::Error as SchemeError;
use crate::fw::Firmware;
use crate::regs::*;
use crate::phy::Band;

/// Intel WiFi MAC layer
pub struct Mac {
    regs: *mut u32,
    fw: Arc<Firmware>,
    tx_queue: TxQueue,
    rx_queue: RxQueue,
    mac_addr: [u8; 6],
    current_bssid: [u8; 6],
    current_essid: Vec<u8>,
    associated: bool,
    pm_state: PmState,
}

struct TxQueue {
    descriptors: Vec<TxDescriptor>,
    head: usize,
    tail: usize,
    count: usize,
}

struct RxQueue {
    descriptors: Vec<RxDescriptor>,
    head: usize,
    tail: usize,
    count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct TxDescriptor {
    addr_low: u32,
    addr_high: u32,
    length: u32,
    flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct RxDescriptor {
    addr_low: u32,
    addr_high: u32,
    length: u32,
    flags: u32,
    rssi: i8,
    rate: u8,
    channel: u8,
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

impl Mac {
    pub fn new(regs: *mut u32, fw: &Arc<Firmware>) -> Result<Self, &'static str> {
        log::info!("iwlwifi MAC: Initializing");

        // Read MAC address from NVM (simulated)
        let mac_addr = Self::read_mac_address(regs);

        let tx_queue = TxQueue {
            descriptors: Vec::with_capacity(256),
            head: 0,
            tail: 0,
            count: 256,
        };

        let rx_queue = RxQueue {
            descriptors: Vec::with_capacity(256),
            head: 0,
            tail: 0,
            count: 256,
        };

        // Configure MAC registers
        unsafe {
            // TX control
            core::ptr::write_volatile(regs.add(MAC_TX_CTRL), MAC_TX_CTRL_ENABLE);
            // RX control
            core::ptr::write_volatile(regs.add(MAC_RX_CTRL), MAC_RX_CTRL_ENABLE);
            // Set MAC address
            Self::write_mac_address(regs, mac_addr);
        }

        Ok(Self {
            regs,
            fw: fw.clone(),
            tx_queue,
            rx_queue,
            mac_addr,
            current_bssid: [0; 6],
            current_essid: Vec::new(),
            associated: false,
            pm_state: PmState::PmActive,
        })
    }

    fn read_mac_address(regs: *mut u32) -> [u8; 6] {
        let mac_low = unsafe { core::ptr::read_volatile(regs.add(MAC_ADDRESS_LOW)) };
        let mac_high = unsafe { core::ptr::read_volatile(regs.add(MAC_ADDRESS_HIGH)) };

        if mac_low == 0 && mac_high == 0 {
            [0x02, 0x00, 0x86, 0x7A, 0xF0, 0x01] // Intel OUI + locally administered
        } else {
            [
                (mac_low & 0xFF) as u8,
                ((mac_low >> 8) & 0xFF) as u8,
                ((mac_low >> 16) & 0xFF) as u8,
                ((mac_low >> 24) & 0xFF) as u8,
                (mac_high & 0xFF) as u8,
                ((mac_high >> 8) & 0xFF) as u8,
            ]
        }
    }

    fn write_mac_address(regs: *mut u32, addr: [u8; 6]) {
        let low = u32::from_le_bytes([addr[0], addr[1], addr[2], addr[3]]);
        let high = u16::from_le_bytes([addr[4], addr[5]]) as u32;

        unsafe {
            core::ptr::write_volatile(regs.add(MAC_ADDRESS_LOW), low);
            core::ptr::write_volatile(regs.add(MAC_ADDRESS_HIGH), high);
        }
    }

    pub fn mac_address(&self) -> [u8; 6] {
        self.mac_addr
    }

    pub fn configure_coex(&mut self) -> Result<(), &'static str> {
        log::info!("iwlwifi MAC: Configuring WiFi/BT coexistence");

        // Configure coexistence with Bluetooth
        unsafe {
            core::ptr::write_volatile(self.regs.add(MAC_COEX_CTRL), MAC_COEX_CTRL_ENABLE);
            core::ptr::write_volatile(self.regs.add(MAC_COEX_TABLE), 0x00000003);
        }
        Ok(())
    }

    pub fn handle_tx_done(&mut self) {
        // Hardware concluiu os TX pendentes: consome todos ate o head.
        let head = unsafe { core::ptr::read_volatile(self.regs.add(crate::regs::REG_TXQ_HEAD)) };
        unsafe { core::ptr::write_volatile(self.regs.add(crate::regs::REG_TXQ_TAIL), head) };
    }


    pub fn handle_rx_done(&mut self) {
        // Frames RX ficam na ring ate read_packets() consumir.
    }


    pub fn handle_beacon(&mut self) {
        if self.associated {
            log::trace!("iwlwifi MAC: Beacon handling for BSSID {:02x?}", self.current_bssid);
        }
    }

    pub fn associate(&mut self, bssid: [u8; 6], essid: &[u8]) -> Result<(), &'static str> {
        log::info!("iwlwifi MAC: Associating with BSSID {:02x?}, ESSID {:?}", bssid, essid);

        self.current_bssid = bssid;
        self.current_essid = essid.to_vec();
        self.associated = true;

        self.send_assoc_cmd(bssid, essid)?;
        Ok(())
    }

    pub fn disassociate(&mut self) -> Result<(), &'static str> {
        log::info!("iwlwifi MAC: Disassociating");

        self.current_bssid = [0; 6];
        self.current_essid.clear();
        self.associated = false;

        self.send_disassoc_cmd()?;
        Ok(())
    }

    fn send_assoc_cmd(&self, _bssid: [u8; 6], _essid: &[u8]) -> Result<(), &'static str> {
        log::trace!("iwlwifi MAC: Sending assoc command");
        Ok(())
    }

    fn send_disassoc_cmd(&self) -> Result<(), &'static str> {
        log::trace!("iwlwifi MAC: Sending disassoc command");
        Ok(())
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
            b"IWLWIFI".to_vec()
        }
    }

    pub fn is_associated(&self) -> bool {
        self.associated
    }

    pub fn set_channel(&mut self, _channel: u8, _band: Band) -> Result<(), &'static str> {
        Ok(())
    }

    /// Get a reference to the MAC registers
    pub fn regs(&self) -> *mut u32 {
        self.regs
    }

    /// Check if RX queue has packets available
    pub fn rx_queue_has_packets(&self) -> bool {
        self.rx_queue.head != self.rx_queue.tail
    }

    /// Set power save mode
    pub fn set_power_save(&mut self, mode: PowerSaveMode) -> Result<(), &'static str> {
        log::info!("iwlwifi MAC: Setting power save mode {:?}", mode);

        let mut pm_state = PmState::PmActive;

        match mode {
            PowerSaveMode::Active => {
                unsafe {
                    core::ptr::write_volatile(self.regs.add(MAC_PM_CTRL), MAC_PM_CTRL_DISABLE);
                }
                pm_state = PmState::PmActive;
            }
            PowerSaveMode::PowerSave => {
                unsafe {
                    core::ptr::write_volatile(self.regs.add(MAC_PM_CTRL), MAC_PM_CTRL_ENABLE);
                }
                pm_state = PmState::PmPowerSave;
            }
            PowerSaveMode::Uapac => {
                unsafe {
                    core::ptr::write_volatile(self.regs.add(MAC_PM_CTRL), MAC_PM_CTRL_UAPSD_ENABLE);
                }
                pm_state = PmState::PmAuto;
            }
        }
        self.pm_state = pm_state;
        Ok(())
    }

    /// Get current power save state
    pub fn pm_state(&self) -> PmState {
        self.pm_state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mac_address_generation() {
        let addr = [0x02, 0x00, 0x86, 0x7A, 0xF0, 0x01];
        assert_eq!(addr[0] & 0x02, 0x02); // Locally administered bit set
        assert_eq!(addr[0] & 0x01, 0x00); // Unicast bit clear
    }

    #[test]
    fn test_band_enum() {
        assert_eq!(Band::Band2Ghz as u8, 0);
        assert_eq!(Band::Band5Ghz as u8, 1);
        assert_eq!(Band::Band6Ghz as u8, 2);
    }

    #[test]
    fn test_tx_rx_flags() {
        assert_eq!(crate::regs::TX_DESC_DONE, 0x80000000);
        assert_eq!(crate::regs::RX_DESC_DONE, 0x80000000);
        assert_eq!(crate::regs::TX_DESC_VALID, 0x1);
        assert_eq!(crate::regs::RX_DESC_VALID, 0x1);
    }
}
/// Upload firmware blob to the chip: halt engine, stream image into the
/// SRAM staging window as LE words (tail zero-padded), latch START and
/// wait for BUSY to clear. Same contract across the WiFi driver family.
pub fn upload_firmware_fn(regs: *mut u32, data: &[u8]) -> Result<(), &'static str> {
    if data.is_empty() {
        return Err("iwlwifi: empty firmware blob");
    }
    log::info!("iwlwifi: uploading firmware ({} bytes)", data.len());

    // SAFETY: regs is this device's mapped BAR0.
    unsafe {
        core::ptr::write_volatile(regs.add(crate::regs::REG_FW_CTRL), 0);
        for (i, chunk) in data.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            core::ptr::write_volatile(
                regs.add(crate::regs::REG_FW_SRAM_WINDOW + i),
                u32::from_le_bytes(w),
            );
        }
        core::ptr::write_volatile(regs.add(crate::regs::REG_FW_CTRL), crate::regs::FW_CTRL_START);
        let mut spins = 0usize;
        while core::ptr::read_volatile(regs.add(crate::regs::REG_FW_STATUS)) & crate::regs::FW_STATUS_BUSY != 0 {
            spins += 1;
            if spins > 1_000_000 {
                return Err("iwlwifi: firmware start timeout");
            }
            core::hint::spin_loop();
        }
    }
    Ok(())
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
        super::upload_firmware_fn(regs, &blob).expect("upload ok");

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
        assert!(super::upload_firmware_fn(regs, &[]).is_err());
        let ctrl = unsafe { core::ptr::read_volatile(regs.add(crate::regs::REG_FW_CTRL)) };
        assert_eq!(ctrl, 0);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;

    fn fake_bar() -> (alloc::boxed::Box<[u32]>, *mut u32) {
        let mut v = alloc::vec![0u32; 0xC001].into_boxed_slice();
        let p = v.as_mut_ptr();
        (v, p)
    }

    fn mac_on(regs: *mut u32) -> Mac {
        let fw = crate::fw::Firmware::load("/nonexistent/iwlwifi.fw").expect("mock fw");
        Mac::new(regs, &fw).expect("mac init")
    }

    fn write_word(regs: *mut u32, idx: usize, v: u32) {
        unsafe { core::ptr::write_volatile(regs.add(idx), v) };
    }
    fn read_word(regs: *mut u32, idx: usize) -> u32 {
        unsafe { core::ptr::read_volatile(regs.add(idx)) }
    }

    #[test]
    fn rx_empty_retorna_zero() {
        let (_bar, regs) = fake_bar();
        let mut mac = mac_on(regs);
        let mut buf = [0u8; 128];
        assert_eq!(mac.read_packets(&mut buf).unwrap(), 0);
    }

    #[test]
    fn rx_le_frame_semeado_e_recicla() {
        let (_bar, regs) = fake_bar();
        let mut mac = mac_on(regs);

        let payload = b"IWLWIFI RX 80211";
        let off_words = crate::regs::FRAME_SLOT_WORDS;
        for (i, chunk) in payload.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            write_word(regs, crate::regs::RX_DATA_WINDOW + off_words + i, u32::from_le_bytes(w));
        }
        write_word(regs, crate::regs::REG_RXQ_RING, off_words as u32);
        write_word(regs, crate::regs::REG_RXQ_RING + 1, payload.len() as u32);
        write_word(regs, crate::regs::REG_RXQ_HEAD, 1);

        let mut buf = [0u8; 1518];
        assert_eq!(mac.read_packets(&mut buf).unwrap(), payload.len());
        assert_eq!(&buf[..payload.len()], payload);
        assert_eq!(read_word(regs, crate::regs::REG_RXQ_TAIL), 1);
        assert_eq!(mac.read_packets(&mut buf).unwrap(), 0);
    }

    #[test]
    fn tx_escreve_slot_e_bump() {
        let (_bar, regs) = fake_bar();
        let mut mac = mac_on(regs);
        mac.associated = true;
        let frame: alloc::vec::Vec<u8> = alloc::vec![0x5A; 72];
        assert_eq!(mac.write_packets(&frame).unwrap(), 72);
        assert_eq!(read_word(regs, crate::regs::REG_TXQ_HEAD), 1);
        assert_eq!(read_word(regs, crate::regs::TX_DATA_WINDOW), 0x5A5A_5A5A);
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
        assert_eq!(mac.write_packets(&frame).unwrap_err().errno, syscall::ENOBUFS);

        mac.handle_tx_done();
        assert_eq!(read_word(regs, crate::regs::REG_TXQ_TAIL), crate::regs::RING_LEN);
        assert!(mac.write_packets(&frame).is_ok());
    }
}
