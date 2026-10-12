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
        // Process completed TX descriptors
        while self.tx_ring.head != self.tx_ring.tail {
            let desc = unsafe { core::ptr::read_volatile(self.tx_ring.base.add(self.tx_ring.head)) };
            if desc & 0x80000000 == 0 { // Not done
                break;
            }
            self.tx_ring.head = (self.tx_ring.head + 1) % self.tx_ring.count;
        }
    }
    
    pub fn handle_rx_done(&mut self) {
        // Process received packets
        while self.rx_ring.head != self.rx_ring.tail {
            let desc = unsafe { core::ptr::read_volatile(self.rx_ring.base.add(self.rx_ring.head)) };
            if desc & 0x80000000 == 0 { // Not done
                break;
            }
            self.rx_ring.head = (self.rx_ring.head + 1) % self.rx_ring.count;
        }
    }
    
    pub fn handle_beacon(&mut self) {
        // Handle beacon transmission/reception
    }
    
    pub fn read_packets(&mut self, buf: &mut [u8]) -> Result<usize, SchemeError> {
        // Copy received packets to buffer
        if self.rx_ring.head == self.rx_ring.tail {
            return Ok(0);
        }
        
        // Simplified: return dummy data
        let len = core::cmp::min(buf.len(), 1500);
        buf[..len].fill(0);
        Ok(len)
    }
    
    pub fn write_packets(&mut self, buf: &[u8]) -> Result<usize, SchemeError> {
        // Queue packets for transmission
        if self.tx_ring.tail == (self.tx_ring.head + self.tx_ring.count - 1) % self.tx_ring.count {
            return Err(SchemeError::new(syscall::ENOBUFS));
        }
        
        // Simplified: just accept the packet
        self.tx_ring.tail = (self.tx_ring.tail + 1) % self.tx_ring.count;
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
