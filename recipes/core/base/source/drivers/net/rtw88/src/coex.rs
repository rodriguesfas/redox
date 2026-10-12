//! WiFi/Bluetooth coexistence for Realtek rtw88

use crate::regs::*;

pub struct Coex {
    regs: *mut u32,
    wifi_tx_priority: bool,
    bt_rx_priority: bool,
}

impl Coex {
    pub fn new(regs: *mut u32, _fw: &crate::fw::Firmware) -> Result<Self, &'static str> {
        log::info!("rtw88 Coex: Initializing WiFi/BT coexistence");

        // Initialize coexistence registers
        unsafe {
            core::ptr::write_volatile(regs.add(REG_COEX_CTRL), COEX_CTRL_EN);
        }

        Ok(Self {
            regs,
            wifi_tx_priority: true,
            bt_rx_priority: true,
        })
    }

    pub fn configure_mac(&self, mac_regs: *mut u32) -> Result<(), &'static str> {
        log::info!("rtw88 Coex: Configuring MAC for coexistence");

        // Configure MAC for coexistence
        unsafe {
            // Set coexistence table
            core::ptr::write_volatile(mac_regs.add(REG_COEX_TABLE), 0x00000003);
        }

        Ok(())
    }

    pub fn set_wifi_tx_priority(&mut self, priority: bool) {
        self.wifi_tx_priority = priority;
        unsafe {
            if priority {
                core::ptr::write_volatile(self.regs.add(REG_COEX_CTRL), COEX_CTRL_EN | COEX_CTRL_WL_TX);
            } else {
                core::ptr::write_volatile(self.regs.add(REG_COEX_CTRL), COEX_CTRL_EN & !COEX_CTRL_WL_TX);
            }
        }
    }

    pub fn set_bt_rx_priority(&mut self, priority: bool) {
        self.bt_rx_priority = priority;
        unsafe {
            if priority {
                core::ptr::write_volatile(self.regs.add(REG_COEX_CTRL), COEX_CTRL_EN | COEX_CTRL_BT_RX);
            } else {
                core::ptr::write_volatile(self.regs.add(REG_COEX_CTRL), COEX_CTRL_EN & !COEX_CTRL_BT_RX);
            }
        }
    }

    pub fn handle_bt_info(&mut self, bt_info: &[u8]) {
        // Parse BT info from firmware
        if bt_info.len() < 1 {
            return;
        }

        let bt_status = bt_info[0];
        log::trace!("rtw88 Coex: BT status {:#x}", bt_status);

        // Adjust coexistence based on BT activity
        if bt_status & 0x01 != 0 {
            // BT is transmitting, give WiFi RX priority
            self.set_wifi_tx_priority(false);
        } else {
            self.set_wifi_tx_priority(true);
        }

        if bt_status & 0x02 != 0 {
            // BT is receiving, give BT RX priority
            self.set_bt_rx_priority(true);
        } else {
            self.set_bt_rx_priority(false);
        }
    }

    pub fn get_status(&self) -> CoexStatus {
        let ctrl = unsafe { core::ptr::read_volatile(self.regs.add(REG_COEX_CTRL)) };
        CoexStatus {
            enabled: ctrl & COEX_CTRL_EN != 0,
            wifi_tx_priority: ctrl & COEX_CTRL_WL_TX != 0,
            wifi_rx_priority: ctrl & COEX_CTRL_WL_RX != 0,
            bt_tx_priority: ctrl & COEX_CTRL_BT_TX != 0,
            bt_rx_priority: ctrl & COEX_CTRL_BT_RX != 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CoexStatus {
    pub enabled: bool,
    pub wifi_tx_priority: bool,
    pub wifi_rx_priority: bool,
    pub bt_tx_priority: bool,
    pub bt_rx_priority: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coex_status() {
        let status = CoexStatus {
            enabled: true,
            wifi_tx_priority: true,
            wifi_rx_priority: false,
            bt_tx_priority: false,
            bt_rx_priority: true,
        };
        assert!(status.enabled);
        assert!(status.wifi_tx_priority);
        assert!(status.bt_rx_priority);
    }
}