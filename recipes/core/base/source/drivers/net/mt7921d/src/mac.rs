//! MAC layer for MT7921

use alloc::vec::Vec;
use syscall::error::Error as SchemeError;

pub struct Mac {
    regs: *mut u32,
    tx_ring: TxRing,
    rx_ring: RxRing,
    mac_addr: [u8; 6],
    bssid: [u8; 6],
    associated: bool,
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

struct TxRing {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
}

struct RxRing {
    base: *mut u32,
    count: usize,
    head: usize,
    tail: usize,
}

impl Mac {
    pub fn new(regs: *mut u32, fw: &crate::fw::Firmware) -> Result<Self, &'static str> {
        let _fw = fw; // suppress unused warning
        log::info!("MT7921 MAC: Initializing");
        
        // Initialize TX ring
        let tx_ring = TxRing {
            base: unsafe { regs.add(0x1000) },
            count: 256,
            head: 0,
            tail: 0,
        };
        
        // Initialize RX ring
        let rx_ring = RxRing {
            base: unsafe { regs.add(0x2000) },
            count: 256,
            head: 0,
            tail: 0,
        };
        
        // Configure MAC registers
        unsafe {
            // MT7921_MAC_TX_CTRL
            core::ptr::write_volatile(regs.add(0x1100), 0x1);
            // MT7921_MAC_RX_CTRL
            core::ptr::write_volatile(regs.add(0x1200), 0x1);
        }
        
        // Generate MAC address
        let mac_addr = [0x00, 0x0C, 0xE7, 0x12, 0x34, 0x56]; // MediaTek OUI
        
        Ok(Self { 
            regs, 
            tx_ring, 
            rx_ring,
            mac_addr,
            bssid: [0; 6],
            associated: false,
            pm_state: PmState::PmActive,
        })
    }
    
    pub fn configure_coex(&mut self) -> Result<(), &'static str> {
        log::info!("MT7921 MAC: Configuring WiFi/BT coexistence");
        // Configure coexistence registers
        unsafe {
            // MT7921_COEX_CTRL
            core::ptr::write_volatile(self.regs.add(0x3000), 0x3);
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
        // Handle beacon transmission/reception
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
        b"MT7921_AP".to_vec()
    }

    /// Get MAC address
    pub fn mac_address(&self) -> [u8; 6] {
        self.mac_addr
    }

    /// Check if associated
    pub fn is_associated(&self) -> bool {
        self.associated
    }

    /// Associate with BSSID
    pub fn associate(&mut self, bssid: [u8; 6], _essid: &[u8]) -> Result<(), &'static str> {
        self.bssid = bssid;
        self.associated = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_ring_initialization() {
        let tx_ring = TxRing {
            base: core::ptr::null_mut(),
            count: 256,
            head: 0,
            tail: 0,
        };
        assert_eq!(tx_ring.count, 256);
        assert_eq!(tx_ring.head, 0);
        assert_eq!(tx_ring.tail, 0);
    }

    #[test]
    fn test_rx_ring_initialization() {
        let rx_ring = RxRing {
            base: core::ptr::null_mut(),
            count: 256,
            head: 0,
            tail: 0,
        };
        assert_eq!(rx_ring.count, 256);
        assert_eq!(rx_ring.head, 0);
        assert_eq!(rx_ring.tail, 0);
    }

    #[test]
    fn test_ring_empty() {
        let tx_ring = TxRing {
            base: core::ptr::null_mut(),
            count: 256,
            head: 0,
            tail: 0,
        };
        assert_eq!(tx_ring.head, tx_ring.tail);
    }

    #[test]
    fn test_ring_advance() {
        let mut tx_ring = TxRing {
            base: core::ptr::null_mut(),
            count: 256,
            head: 0,
            tail: 0,
        };
        tx_ring.tail = (tx_ring.tail + 1) % tx_ring.count;
        assert_eq!(tx_ring.tail, 1);
        assert_ne!(tx_ring.head, tx_ring.tail);
    }
}
/// Upload firmware blob to the chip: halt engine, stream image into the
/// SRAM staging window as LE words (tail zero-padded), latch START and
/// wait for BUSY to clear. Same contract across the WiFi driver family.
pub fn upload_firmware_fn(regs: *mut u32, data: &[u8]) -> Result<(), &'static str> {
    if data.is_empty() {
        return Err("MT7921: empty firmware blob");
    }
    log::info!("MT7921: uploading firmware ({} bytes)", data.len());

    // SAFETY: regs is this device's mapped BAR0.
    unsafe {
        core::ptr::write_volatile(regs.add(crate::regs::regsfw::REG_FW_CTRL), 0);
        for (i, chunk) in data.chunks(4).enumerate() {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            core::ptr::write_volatile(
                regs.add(crate::regs::regsfw::REG_FW_SRAM_WINDOW + i),
                u32::from_le_bytes(w),
            );
        }
        core::ptr::write_volatile(regs.add(crate::regs::regsfw::REG_FW_CTRL), crate::regs::regsfw::FW_CTRL_START);
        let mut spins = 0usize;
        while core::ptr::read_volatile(regs.add(crate::regs::regsfw::REG_FW_STATUS)) & crate::regs::regsfw::FW_STATUS_BUSY != 0 {
            spins += 1;
            if spins > 1_000_000 {
                return Err("MT7921: firmware start timeout");
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
            let w = unsafe { core::ptr::read_volatile(regs.add(crate::regs::regsfw::REG_FW_SRAM_WINDOW + i)) };
            back.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(&back[..5], &blob[..]);
        assert_eq!(back[5], 0);

        let ctrl = unsafe { core::ptr::read_volatile(regs.add(crate::regs::regsfw::REG_FW_CTRL)) };
        assert_eq!(ctrl & crate::regs::regsfw::FW_CTRL_START, crate::regs::regsfw::FW_CTRL_START);
    }

    #[test]
    fn upload_rejects_empty() {
        let (_bar, regs) = fake_bar();
        assert!(super::upload_firmware_fn(regs, &[]).is_err());
        let ctrl = unsafe { core::ptr::read_volatile(regs.add(crate::regs::regsfw::REG_FW_CTRL)) };
        assert_eq!(ctrl, 0);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;

    fn fake_bar() -> (alloc::boxed::Box<[u32]>, *mut u32) {
        let mut v = alloc::vec![0u32; 0x10001].into_boxed_slice();
        let p = v.as_mut_ptr();
        (v, p)
    }

    fn mac_on(regs: *mut u32) -> Mac {
        let fw = crate::fw::Firmware::load("/nonexistent/mt7921.fw").expect("mock fw");
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

        let payload = b"MT7921 BEACON OK";
        let off_words = crate::regs::FRAME_SLOT_WORDS * 3;
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
        let frame: alloc::vec::Vec<u8> = alloc::vec![0xEF; 96];
        assert_eq!(mac.write_packets(&frame).unwrap(), 96);
        assert_eq!(read_word(regs, crate::regs::REG_TXQ_HEAD), 1);
        assert_eq!(read_word(regs, crate::regs::TX_DATA_WINDOW), 0xEFEF_EFEF);
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
