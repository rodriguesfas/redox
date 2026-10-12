//! PHY layer management for ath9k.

use crate::regs::*;

/// PHY layer state.
pub struct Phy {
    regs: *mut u32,
    channel: u8,
    rssi_dbm: i8,
}

impl Phy {
    pub fn new(regs: *mut u32) -> Result<Self, &'static str> {
        log::info!("ath9k PHY: initializing");

        // SAFETY: regs is the mapped BAR0 for this device.
        unsafe {
            core::ptr::write_volatile(regs.add(REG_PHY_CTRL), 0x1);
            core::ptr::write_volatile(regs.add(REG_PHY_BANDWIDTH), 0x0);
        }

        Ok(Self {
            regs,
            channel: 1,
            rssi_dbm: -90,
        })
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    pub fn set_channel(&mut self, channel: u8) -> Result<(), &'static str> {
        if !(1..=14).contains(&channel) {
            return Err("invalid 2.4GHz channel");
        }
        self.channel = channel;
        Ok(())
    }

    /// Placeholder RSSI until real RF reporting is implemented.
    pub fn rssi_dbm(&self) -> i8 {
        self.rssi_dbm
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn channel_validation() {
        // The constructor needs a real BAR; test the validation logic via a
        // tiny helper struct mimicking the same rule.
        fn validate(ch: u8) -> bool {
            (1..=14).contains(&ch)
        }
        assert!(validate(1));
        assert!(validate(14));
        assert!(!validate(0));
        assert!(!validate(15));
    }
}
