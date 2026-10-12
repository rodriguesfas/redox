//! EFUSE (Electrical Fuse) reading for Realtek rtw88

use crate::regs::*;

#[derive(Clone, Debug)]
pub struct Efuse {
    mac_addr: [u8; 6],
    crystal_cap: u8,
    thermal_meter: u8,
    iqk_lck: u8,
    txpwr_2g: Vec<u8>,
    txpwr_5g: Vec<u8>,
    txpwr_6g: Vec<u8>,
}

impl Efuse {
    pub fn read_efuse(regs: *mut u32) -> Self {
        log::info!("rtw88 EFUSE: Reading EFUSE data");

        // In a real implementation, this would read from the hardware EFUSE
        // For now, we simulate reading with default values

        // Read MAC address from EFUSE
        let mac_low = unsafe { core::ptr::read_volatile(regs.add(EFUSE_MAC_ADDR)) };
        let mac_high = unsafe { core::ptr::read_volatile(regs.add(EFUSE_MAC_ADDR + 4)) };

        let mac_addr = if mac_low == 0 && mac_high == 0 {
            // Generate a locally administered MAC based on Realtek OUI
            [0x00, 0xE0, 0x4C, 0x12, 0x34, 0x56]
        } else {
            [
                (mac_low & 0xFF) as u8,
                ((mac_low >> 8) & 0xFF) as u8,
                ((mac_low >> 16) & 0xFF) as u8,
                ((mac_low >> 24) & 0xFF) as u8,
                (mac_high & 0xFF) as u8,
                ((mac_high >> 8) & 0xFF) as u8,
            ]
        };

        // Read other EFUSE values (simulated)
        let crystal_cap = CRYSTAL_CAP_DEFAULT;
        let thermal_meter = THERMAL_METER_DEFAULT;
        let iqk_lck = 0x00;

        // Simulated TX power tables
        let txpwr_2g = vec![0x10; 14];  // 14 channels for 2.4GHz
        let txpwr_5g = vec![0x12; 25];  // 25 channels for 5GHz
        let txpwr_6g = vec![0x14; 20];  // 20 channels for 6GHz

        Self {
            mac_addr,
            crystal_cap,
            thermal_meter,
            iqk_lck,
            txpwr_2g,
            txpwr_5g,
            txpwr_6g,
        }
    }

    pub fn mac_addr(&self) -> [u8; 6] {
        self.mac_addr
    }

    pub fn crystal_cap(&self) -> u8 {
        self.crystal_cap
    }

    pub fn thermal_meter(&self) -> u8 {
        self.thermal_meter
    }

    pub fn iqk_lck(&self) -> u8 {
        self.iqk_lck
    }

    pub fn txpwr_2g(&self, channel: u8) -> Option<u8> {
        if channel >= 1 && channel <= 14 {
            Some(self.txpwr_2g[(channel - 1) as usize])
        } else {
            None
        }
    }

    pub fn txpwr_5g(&self, channel: u8) -> Option<u8> {
        // Map 5GHz channels to index
        let idx = match channel {
            36..=48 => channel - 36,
            52..=64 => channel - 52 + 13,
            100..=144 => channel - 100 + 25,
            149..=165 => channel - 149 + 69,
            _ => return None,
        };
        self.txpwr_5g.get(idx as usize).copied()
    }

    pub fn txpwr_6g(&self, channel: u8) -> Option<u8> {
        if channel >= 1 && channel <= 233 {
            Some(self.txpwr_6g[(channel - 1) as usize])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_efuse_mac_addr() {
        let efuse = Efuse {
            mac_addr: [0x00, 0xE0, 0x4C, 0x12, 0x34, 0x56],
            crystal_cap: 0x20,
            thermal_meter: 0x1F,
            iqk_lck: 0x00,
            txpwr_2g: vec![],
            txpwr_5g: vec![],
            txpwr_6g: vec![],
        };
        assert_eq!(efuse.mac_addr(), [0x00, 0xE0, 0x4C, 0x12, 0x34, 0x56]);
    }

    #[test]
    fn test_txpwr_lookup() {
        let mut efuse = Efuse {
            mac_addr: [0; 6],
            crystal_cap: 0x20,
            thermal_meter: 0x1F,
            iqk_lck: 0x00,
            txpwr_2g: vec![0x10; 14],
            txpwr_5g: vec![0x12; 25],
            txpwr_6g: vec![0x14; 20],
        };

        assert_eq!(efuse.txpwr_2g(1), Some(0x10));
        assert_eq!(efuse.txpwr_2g(14), Some(0x10));
        assert_eq!(efuse.txpwr_2g(15), None);
    }
}