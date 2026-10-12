//! ath9k register offsets (simplified map, in 32-bit words).

/// MAC register offsets
pub const REG_TX_CTRL: usize = 0x0100;
pub const REG_RX_CTRL: usize = 0x0104;
pub const REG_MAC_ADDRESS: usize = 0x0500;

/// Interrupt registers
pub const REG_INT_STATUS: usize = 0x0300;
pub const REG_INT_CLEAR: usize = 0x0304;

/// Interrupt bits
pub const INT_TX_DONE: u32 = 0x0000_0001;
pub const INT_RX_DONE: u32 = 0x0000_0002;
pub const INT_TBTT: u32 = 0x0000_0004;

/// PHY registers
pub const REG_PHY_CTRL: usize = 0x0400;
pub const REG_PHY_BANDWIDTH: usize = 0x0418;

/// Power management
pub const REG_PM_CTRL: usize = 0x0700;

/// Reset / firmware upload (staging SRAM window + control)
pub const REG_RESET: usize = 0x0004;
pub const REG_RESET_BIT: u32 = 0x0000_0001;
/// Base offset (in u32 words) of the SRAM staging window for firmware blobs.
pub const REG_FW_SRAM_WINDOW: usize = 0x2000;
pub const REG_FW_CTRL: usize = 0x2010;
pub const REG_FW_STATUS: usize = 0x2011;
/// FW_CTRL: write 1 to start execution of the uploaded image.
pub const FW_CTRL_START: u32 = 0x0000_0001;
/// FW_STATUS: read 0 = engine ready/idle (accepts upload or running).
pub const FW_STATUS_BUSY: u32 = 0x0000_0001;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reg_offsets_distinct() {
        let offs = [
            REG_TX_CTRL,
            REG_RX_CTRL,
            REG_INT_STATUS,
            REG_INT_CLEAR,
            REG_PHY_CTRL,
            REG_PHY_BANDWIDTH,
            REG_PM_CTRL,
            REG_MAC_ADDRESS,
        ];
        for i in 0..offs.len() {
            for j in (i + 1)..offs.len() {
                assert_ne!(offs[i], offs[j]);
            }
        }
    }

    #[test]
    fn irq_bits_distinct() {
        assert_eq!(INT_TX_DONE & INT_RX_DONE, 0);
        assert_eq!(INT_TX_DONE & INT_TBTT, 0);
        assert_eq!(INT_RX_DONE & INT_TBTT, 0);
    }

    #[test]
    fn fw_regs_layout() {
        assert_eq!(REG_FW_CTRL, 0x2010);
        assert_eq!(REG_FW_STATUS, 0x2011);
        assert!(REG_FW_SRAM_WINDOW < REG_FW_CTRL);
    }
}
