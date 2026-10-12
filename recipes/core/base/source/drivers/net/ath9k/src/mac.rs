//! MAC layer management for ath9k.

use alloc::vec::Vec;

use syscall::{Error, Result, EWOULDBLOCK};

use crate::regs::*;

/// Fixed station MAC until EEPROM reading is implemented.
const DEFAULT_MAC: [u8; 6] = [0x00, 0x03, 0x7F, 0x12, 0x34, 0x56]; // Atheros OUI 00:03:7F

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

/// Power-save attributes surfaced to the OS.
pub struct PsAttr {
    pub pm_state: PmState,
    pub uapsd_config: u8,
    pub ps_dtim: u8,
    pub ps_poll_interval: u16,
}

/// MAC layer state.
pub struct Mac {
    regs: *mut u32,
    mac_addr: [u8; 6],
    bssid: [u8; 6],
    current_essid: Vec<u8>,
    associated: bool,
    pm_state: PmState,
    rx_backlog: usize,
}

impl Mac {
    pub fn new(regs: *mut u32, _fw: &crate::fw::Firmware) -> Result<Self, &'static str> {
        log::info!("ath9k MAC: initializing");

        // SAFETY: regs is the mapped BAR0 for this device.
        unsafe {
            core::ptr::write_volatile(regs.add(REG_TX_CTRL), 0x1);
            core::ptr::write_volatile(regs.add(REG_RX_CTRL), 0x1);
            core::ptr::write_volatile(regs.add(REG_PM_CTRL), 0x0);
        }

        Ok(Self {
            regs,
            mac_addr: DEFAULT_MAC,
            bssid: [0; 6],
            current_essid: Vec::new(),
            associated: false,
            pm_state: PmState::PmActive,
            rx_backlog: 0,
        })
    }

    pub fn mac_address(&self) -> [u8; 6] {
        self.mac_addr
    }

    pub fn bssid(&self) -> [u8; 6] {
        self.bssid
    }

    pub fn current_essid(&self) -> &[u8] {
        &self.current_essid
    }

    pub fn is_associated(&self) -> bool {
        self.associated
    }

    pub fn pm_state(&self) -> PmState {
        self.pm_state
    }

    pub fn rx_pending(&self) -> bool {
        self.rx_backlog > 0
    }

    pub fn associate(&mut self, bssid: [u8; 6], essid: &[u8]) {
        log::info!("ath9k MAC: associating with {:02x?}", bssid);
        self.bssid = bssid;
        self.current_essid = essid.to_vec();
        self.associated = true;
    }

    pub fn disassociate(&mut self) {
        log::info!("ath9k MAC: disassociating");
        self.bssid = [0; 6];
        self.current_essid.clear();
        self.associated = false;
    }

    pub fn handle_tx_done(&mut self) {}
    pub fn handle_rx_done(&mut self) {}
    pub fn handle_beacon(&mut self) {}

    /// Non-blocking packet read; Ok(None) when the RX ring is empty.
    pub fn read_packet(&mut self, _buf: &mut [u8]) -> Result<Option<usize>> {
        if self.rx_backlog == 0 {
            return Ok(None);
        }
        self.rx_backlog -= 1;
        Ok(Some(0))
    }

    pub fn write_packet(&mut self, buf: &[u8]) -> Result<usize> {
        if !self.associated {
            return Err(Error::new(EWOULDBLOCK));
        }
        Ok(buf.len())
    }

    pub fn set_power_save(&mut self, mode: PowerSaveMode) {
        log::info!("ath9k MAC: power save {:?}", mode);
        self.pm_state = match mode {
            PowerSaveMode::Active => PmState::PmActive,
            PowerSaveMode::PowerSave => PmState::PmPowerSave,
            PowerSaveMode::Uapac => PmState::PmAuto,
        };
        // SAFETY: regs is the mapped BAR0 for this device.
        unsafe { core::ptr::write_volatile(self.regs.add(REG_PM_CTRL), mode as u32) };
    }

    /// Upload the firmware image to the chip's SRAM staging window and start
    /// the on-chip engine.
    ///
    /// Sequence: halt engine -> stream the blob as LE 32-bit words (tail
    /// zero-padded) -> latch START -> poll STATUS until busy clears.
    pub fn upload_firmware(&mut self, fw: &crate::fw::Firmware) -> Result<(), &'static str> {
        if fw.is_empty() {
            return Err("ath9k: empty firmware blob");
        }
        log::info!(
            "ath9k MAC: uploading firmware ({} bytes, version {})",
            fw.len(),
            fw.version()
        );

        // SAFETY for all MMIO below: `regs` is this device's mapped BAR0.
        unsafe {
            // 1. Halt engine and pulse chip reset.
            core::ptr::write_volatile(self.regs.add(REG_FW_CTRL), 0);
            core::ptr::write_volatile(self.regs.add(REG_RESET), REG_RESET_BIT);

            // 2. Stream blob into the SRAM staging window.
            for (i, chunk) in fw.data().chunks(4).enumerate() {
                let mut word = [0u8; 4];
                word[..chunk.len()].copy_from_slice(chunk);
                core::ptr::write_volatile(
                    self.regs.add(REG_FW_SRAM_WINDOW + i),
                    u32::from_le_bytes(word),
                );
            }

            // 3. Start the engine.
            core::ptr::write_volatile(self.regs.add(REG_FW_CTRL), FW_CTRL_START);

            // 4. Wait for busy to clear (bounded).
            let mut spins: usize = 0;
            while core::ptr::read_volatile(self.regs.add(REG_FW_STATUS)) & FW_STATUS_BUSY != 0 {
                spins += 1;
                if spins > 1_000_000 {
                    return Err("ath9k: firmware start timeout");
                }
                core::hint::spin_loop();
            }
        }

        log::info!("ath9k MAC: firmware engine running");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mac_is_atheros_oui() {
        assert_eq!(&DEFAULT_MAC[..3], &[0x00, 0x03, 0x7F]);
        // Unicast, globally administered.
        assert_eq!(DEFAULT_MAC[0] & 0x01, 0);
        assert_eq!(DEFAULT_MAC[0] & 0x02, 0);
    }

    #[test]
    fn power_save_mode_map() {
        assert_eq!(PowerSaveMode::Active as u32, 0);
        assert_eq!(PowerSaveMode::PowerSave as u32, 1);
        assert_eq!(PowerSaveMode::Uapac as u32, 2);
    }

    /// Fake BAR0 in heap memory so upload paths can be tested on host.
    fn fake_bar() -> (alloc::boxed::Box<[u32]>, *mut u32) {
        let mut v = alloc::vec![0u32; 0x4000].into_boxed_slice();
        let ptr = v.as_mut_ptr();
        (v, ptr)
    }

    fn mac_on(regs: *mut u32) -> Mac {
        Mac {
            regs,
            mac_addr: DEFAULT_MAC,
            bssid: [0; 6],
            current_essid: Vec::new(),
            associated: false,
            pm_state: PmState::PmActive,
            rx_backlog: 0,
        }
    }

    #[test]
    fn upload_writes_blob_and_starts_engine() {
        let (_bar, regs) = fake_bar();
        let blob = alloc::vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]; // 7B -> 2 words
        let fw = crate::fw::Firmware::from_bytes(blob.clone());

        let mut mac = mac_on(regs);
        mac.upload_firmware(&fw).expect("upload ok");

        // Blob must be present at the staging window, LE words, tail padded.
        let mut back = alloc::vec::Vec::new();
        for i in 0..2 {
            let w = unsafe { core::ptr::read_volatile(regs.add(REG_FW_SRAM_WINDOW + i)) };
            back.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(&back[..blob.len()], &blob[..]);
        assert_eq!(back[7], 0);

        // START bit must be latched.
        let ctrl = unsafe { core::ptr::read_volatile(regs.add(REG_FW_CTRL)) };
        assert_eq!(ctrl & FW_CTRL_START, FW_CTRL_START);

        // Reset was pulsed (register written with RESET bit at some point;
        // fake BAR keeps the last write).
        let reset = unsafe { core::ptr::read_volatile(regs.add(REG_RESET)) };
        assert_eq!(reset & REG_RESET_BIT, REG_RESET_BIT);
    }

    #[test]
    fn upload_rejects_empty_blob() {
        let (_bar, regs) = fake_bar();
        let fw = crate::fw::Firmware::new(); // empty
        let mut mac = mac_on(regs);
        assert!(mac.upload_firmware(&fw).is_err());

        // Nothing must have been started.
        let ctrl = unsafe { core::ptr::read_volatile(regs.add(REG_FW_CTRL)) };
        assert_eq!(ctrl, 0);
    }

    #[test]
    fn upload_word_count_rounds_up() {
        let (_bar, regs) = fake_bar();
        let blob = alloc::vec![0xAB; 5]; // 5B -> 2 words
        let fw = crate::fw::Firmware::from_bytes(blob);
        let mut mac = mac_on(regs);
        mac.upload_firmware(&fw).unwrap();

        let w0 = unsafe { core::ptr::read_volatile(regs.add(REG_FW_SRAM_WINDOW)) };
        let w1 = unsafe { core::ptr::read_volatile(regs.add(REG_FW_SRAM_WINDOW + 1)) };
        assert_eq!(w0, 0xABAB_ABAB);
        assert_eq!(w1, 0x0000_00AB);
    }
}
