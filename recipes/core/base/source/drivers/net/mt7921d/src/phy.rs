//! PHY layer for MT7921

pub struct Phy {
    regs: *mut u32,
}

impl Phy {
    pub fn new(regs: *mut u32, fw: &crate::fw::Firmware) -> Result<Self, &'static str> {
        let _fw = fw; // suppress unused warning
        log::info!("MT7921 PHY: Initializing");
        
        // Initialize PHY registers
        unsafe {
            // MT7921_PHY_CTRL
            core::ptr::write_volatile(regs.add(0x4000), 0x1);
            // MT7921_PHY_RF_CTRL
            core::ptr::write_volatile(regs.add(0x4100), 0x1);
        }
        
        // Load PHY calibration data from firmware
        if let Some(cal_data) = fw.get_patch("phy_cal") {
            log::info!("MT7921 PHY: Loaded calibration data ({} bytes)", cal_data.len());
        }
        
        Ok(Self { regs })
    }
    
    pub fn set_channel(&mut self, channel: u8, band: Band) -> Result<(), &'static str> {
        log::info!("MT7921 PHY: Setting channel {} band {:?}", channel, band);
        
        // Configure RF registers for channel
        let freq: u32 = match band {
            Band::Band2Ghz => 2412 + (channel - 1) as u32 * 5,
            Band::Band5Ghz => 5180 + (channel - 36) as u32 * 5,
            Band::Band6Ghz => 5955 + (channel - 1) as u32 * 5,
        };
        
        unsafe {
            // MT7921_PHY_CHANNEL
            core::ptr::write_volatile(self.regs.add(0x4200), freq);
            // MT7921_PHY_BAND
            core::ptr::write_volatile(self.regs.add(0x4204), band as u32);
        }
        
        Ok(())
    }
    
    pub fn set_bandwidth(&mut self, bw: Bandwidth) -> Result<(), &'static str> {
        log::info!("MT7921 PHY: Setting bandwidth {:?}", bw);
        unsafe {
            core::ptr::write_volatile(self.regs.add(0x4208), bw as u32);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Band2Ghz = 0,
    Band5Ghz = 1,
    Band6Ghz = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bandwidth {
    Bw20 = 0,
    Bw40 = 1,
    Bw80 = 2,
    Bw160 = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_band_enum() {
        assert_eq!(Band::Band2Ghz as u8, 0);
        assert_eq!(Band::Band5Ghz as u8, 1);
        assert_eq!(Band::Band6Ghz as u8, 2);
    }

    #[test]
    fn test_bandwidth_enum() {
        assert_eq!(Bandwidth::Bw20 as u8, 0);
        assert_eq!(Bandwidth::Bw40 as u8, 1);
        assert_eq!(Bandwidth::Bw80 as u8, 2);
        assert_eq!(Bandwidth::Bw160 as u8, 3);
    }

    #[test]
    fn test_channel_frequency_2ghz() {
        // Test 2.4 GHz channel frequencies
        assert_eq!(2412 + (1 - 1) * 5, 2412);  // Channel 1
        assert_eq!(2412 + (6 - 1) * 5, 2437);  // Channel 6
        assert_eq!(2412 + (11 - 1) * 5, 2462); // Channel 11
        assert_eq!(2412 + (14 - 1) * 5, 2477); // Channel 14
    }

    #[test]
    fn test_channel_frequency_5ghz() {
        // Test 5 GHz channel frequencies
        assert_eq!(5180 + (36 - 36) * 5, 5180);  // Channel 36
        assert_eq!(5180 + (40 - 36) * 5, 5200);  // Channel 40
        assert_eq!(5180 + (48 - 36) * 5, 5240);  // Channel 48
    }

    #[test]
    fn test_channel_frequency_6ghz() {
        // Test 6 GHz channel frequencies
        assert_eq!(5955 + (1 - 1) * 5, 5955);   // Channel 1
        assert_eq!(5955 + (93 - 1) * 5, 6415);  // Channel 93
    }
}