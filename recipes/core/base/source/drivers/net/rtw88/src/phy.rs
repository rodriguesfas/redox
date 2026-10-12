//! PHY layer for Realtek rtw88

use alloc::vec::Vec;
use alloc::sync::Arc;
use crate::fw::Firmware;
use crate::regs::*;

pub struct Phy {
    regs: *mut u32,
    fw: Arc<Firmware>,
    current_channel: u8,
    current_band: Band,
    current_bandwidth: Bandwidth,
    calibration_data: Option<Vec<u8>>,
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

impl Phy {
    pub fn new(regs: *mut u32, fw: &Arc<Firmware>) -> Result<Self, &'static str> {
        log::info!("rtw88 PHY: Initializing");

        // Initialize PHY registers
        unsafe {
            // PHY_CTRL
            core::ptr::write_volatile(regs.add(REG_RF_CTRL), RF_CTRL_EN);
        }

        // Load PHY calibration data from firmware
        let calibration_data = fw.get_coex().map(|d| d.to_vec());
        if let Some(ref cal) = calibration_data {
            log::info!("rtw88 PHY: Loaded calibration data ({} bytes)", cal.len());
        }

        Ok(Self {
            regs,
            fw: fw.clone(),
            current_channel: 1,
            current_band: Band::Band2Ghz,
            current_bandwidth: Bandwidth::Bw20,
            calibration_data,
        })
    }

    pub fn set_channel(&mut self, channel: u8, band: Band) -> Result<(), &'static str> {
        log::info!("rtw88 PHY: Setting channel {} band {:?}", channel, band);

        // Validate channel for band
        if !Self::is_valid_channel(channel, band) {
            return Err("Invalid channel for band");
        }

        self.current_channel = channel;
        self.current_band = band;

        // Configure RF registers for channel
        let freq = Self::channel_to_frequency(channel, band);

        unsafe {
            // RF_CHNL
            core::ptr::write_volatile(self.regs.add(REG_RF_CHNL), freq);
            // RF_BAND
            core::ptr::write_volatile(self.regs.add(REG_RF_CHNL + 4), band as u32);
        }

        // Apply calibration for this channel/band
        let cal_data = self.calibration_data.clone();
        if let Some(cal) = cal_data {
            self.apply_calibration(channel, band, &cal)?;
        }

        Ok(())
    }

    pub fn set_bandwidth(&mut self, bw: Bandwidth) -> Result<(), &'static str> {
        log::info!("rtw88 PHY: Setting bandwidth {:?}", bw);

        // Validate bandwidth for current band
        if !Self::is_valid_bandwidth(bw, self.current_band) {
            return Err("Invalid bandwidth for current band");
        }

        self.current_bandwidth = bw;

        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_RF_BW), bw as u32);
        }

        Ok(())
    }

    pub fn set_tx_power(&mut self, power_dbm: i8) -> Result<(), &'static str> {
        log::info!("rtw88 PHY: Setting TX power to {} dBm", power_dbm);

        // Clamp to valid range
        let clamped = power_dbm.clamp(0, 20);

        unsafe {
            core::ptr::write_volatile(self.regs.add(REG_RF_TXPWR), clamped as u32);
        }

        Ok(())
    }

    pub fn get_rssi(&self) -> i8 {
        let rssi_reg = unsafe { core::ptr::read_volatile(self.regs.add(REG_RF_RSSI)) };
        rssi_reg as i8
    }

    pub fn get_noise_floor(&self) -> i8 {
        let nf_reg = unsafe { core::ptr::read_volatile(self.regs.add(REG_RF_NF)) };
        nf_reg as i8
    }

    pub fn current_channel(&self) -> u8 {
        self.current_channel
    }

    pub fn current_band(&self) -> Band {
        self.current_band
    }

    pub fn current_bandwidth(&self) -> Bandwidth {
        self.current_bandwidth
    }

    fn is_valid_channel(channel: u8, band: Band) -> bool {
        match band {
            Band::Band2Ghz => (1..=14).contains(&channel),
            Band::Band5Ghz => matches!(channel, 36..=48 | 52..=64 | 100..=144 | 149..=165),
            Band::Band6Ghz => (1..=233).contains(&channel),
        }
    }

    fn is_valid_bandwidth(bw: Bandwidth, band: Band) -> bool {
        match band {
            Band::Band2Ghz => matches!(bw, Bandwidth::Bw20 | Bandwidth::Bw40),
            Band::Band5Ghz => matches!(bw, Bandwidth::Bw20 | Bandwidth::Bw40 | Bandwidth::Bw80 | Bandwidth::Bw160),
            Band::Band6Ghz => matches!(bw, Bandwidth::Bw20 | Bandwidth::Bw40 | Bandwidth::Bw80 | Bandwidth::Bw160),
        }
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

    fn apply_calibration(&mut self, _channel: u8, _band: Band, _cal_data: &[u8]) -> Result<(), &'static str> {
        // Apply per-channel/band calibration data
        log::trace!("rtw88 PHY: Applying calibration");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_validation_2ghz() {
        assert!(Phy::is_valid_channel(1, Band::Band2Ghz));
        assert!(Phy::is_valid_channel(6, Band::Band2Ghz));
        assert!(Phy::is_valid_channel(11, Band::Band2Ghz));
        assert!(Phy::is_valid_channel(14, Band::Band2Ghz));
        assert!(!Phy::is_valid_channel(0, Band::Band2Ghz));
        assert!(!Phy::is_valid_channel(15, Band::Band2Ghz));
    }

    #[test]
    fn test_channel_validation_5ghz() {
        assert!(Phy::is_valid_channel(36, Band::Band5Ghz));
        assert!(Phy::is_valid_channel(40, Band::Band5Ghz));
        assert!(Phy::is_valid_channel(48, Band::Band5Ghz));
        assert!(Phy::is_valid_channel(52, Band::Band5Ghz));
        assert!(Phy::is_valid_channel(64, Band::Band5Ghz));
        assert!(Phy::is_valid_channel(100, Band::Band5Ghz));
        assert!(Phy::is_valid_channel(165, Band::Band5Ghz));
        assert!(!Phy::is_valid_channel(35, Band::Band5Ghz));
        assert!(!Phy::is_valid_channel(49, Band::Band5Ghz));
    }

    #[test]
    fn test_channel_validation_6ghz() {
        assert!(Phy::is_valid_channel(1, Band::Band6Ghz));
        assert!(Phy::is_valid_channel(233, Band::Band6Ghz));
        assert!(!Phy::is_valid_channel(0, Band::Band6Ghz));
        assert!(!Phy::is_valid_channel(234, Band::Band6Ghz));
    }

    #[test]
    fn test_bandwidth_validation() {
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw20, Band::Band2Ghz));
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw40, Band::Band2Ghz));
        assert!(!Phy::is_valid_bandwidth(Bandwidth::Bw80, Band::Band2Ghz));
        assert!(!Phy::is_valid_bandwidth(Bandwidth::Bw160, Band::Band2Ghz));

        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw20, Band::Band5Ghz));
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw40, Band::Band5Ghz));
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw80, Band::Band5Ghz));
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw160, Band::Band5Ghz));

        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw20, Band::Band6Ghz));
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw40, Band::Band6Ghz));
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw80, Band::Band6Ghz));
        assert!(Phy::is_valid_bandwidth(Bandwidth::Bw160, Band::Band6Ghz));
    }

    #[test]
    fn test_channel_to_frequency() {
        // 2.4 GHz
        assert_eq!(Phy::channel_to_frequency(1, Band::Band2Ghz), 2412);
        assert_eq!(Phy::channel_to_frequency(6, Band::Band2Ghz), 2437);
        assert_eq!(Phy::channel_to_frequency(11, Band::Band2Ghz), 2462);
        assert_eq!(Phy::channel_to_frequency(14, Band::Band2Ghz), 2484);

        // 5 GHz
        assert_eq!(Phy::channel_to_frequency(36, Band::Band5Ghz), 5180);
        assert_eq!(Phy::channel_to_frequency(40, Band::Band5Ghz), 5200);
        assert_eq!(Phy::channel_to_frequency(48, Band::Band5Ghz), 5240);
        assert_eq!(Phy::channel_to_frequency(149, Band::Band5Ghz), 5745);

        // 6 GHz
        assert_eq!(Phy::channel_to_frequency(1, Band::Band6Ghz), 5955);
        assert_eq!(Phy::channel_to_frequency(93, Band::Band6Ghz), 6415);
    }
}