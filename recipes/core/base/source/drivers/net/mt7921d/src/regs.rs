//! Register definitions for MT7921

// WPDMA Global Configuration
pub const MT7921_WPDMA_GLO_CFG: usize = 0x0000;
pub const MT7921_WPDMA_GLO_CFG_TX_DMA_EN: u32 = 0x1;
pub const MT7921_WPDMA_GLO_CFG_RX_DMA_EN: u32 = 0x2;

// Interrupt registers
pub const MT7921_INT_ENABLE: usize = 0x0200;
pub const MT7921_INT_STATUS: usize = 0x0204;
pub const MT7921_INT_ACK: usize = 0x0208;
pub const MT7921_INT_MASK: usize = 0x020C;

// MAC registers
pub const MT7921_MAC_TX_CTRL: usize = 0x1100;
pub const MT7921_MAC_RX_CTRL: usize = 0x1200;
pub const MT7921_MAC_TX_BASE_PTR: usize = 0x1000;
pub const MT7921_MAC_RX_BASE_PTR: usize = 0x2000;

// Coexistence
pub const MT7921_COEX_CTRL: usize = 0x3000;

// PHY registers
pub const MT7921_PHY_CTRL: usize = 0x4000;
pub const MT7921_PHY_RF_CTRL: usize = 0x4100;
pub const MT7921_PHY_CHANNEL: usize = 0x4200;
pub const MT7921_PHY_BAND: usize = 0x4204;
pub const MT7921_PHY_BANDWIDTH: usize = 0x4208;

// MCU registers
pub const MT7921_MCU_CTRL: usize = 0x5000;
pub const MT7921_MCU_DOORBELL: usize = 0x5004;
pub const MT7921_MCU_INT_ENABLE: usize = 0x5100;
pub const MT7921_MCU_INT_STATUS: usize = 0x5104;
pub const MT7921_MCU_CMD: usize = 0x5200;
pub const MT7921_MCU_CMD_ID: usize = 0x5204;
pub const MT7921_MCU_EVENT: usize = 0x5300;
pub const MT7921_MCU_EVENT_ID: usize = 0x5304;

// MCU Commands
pub const MT7921_MCU_CMD_FW_START: u32 = 0x01;
pub const MT7921_MCU_CMD_FW_STOP: u32 = 0x02;
pub const MT7921_MCU_CMD_SCAN: u32 = 0x10;
pub const MT7921_MCU_CMD_CONNECT: u32 = 0x11;
pub const MT7921_MCU_CMD_DISCONNECT: u32 = 0x12;
pub const MT7921_MCU_CMD_SET_CHANNEL: u32 = 0x20;
pub const MT7921_MCU_CMD_SET_BANDWIDTH: u32 = 0x21;
pub const MT7921_MCU_CMD_SET_TX_POWER: u32 = 0x22;
pub const MT7921_MCU_CMD_GET_STATS: u32 = 0x30;

// Interrupt bits
pub const MT7921_INT_TX_DONE: u32 = 0x1;
pub const MT7921_INT_RX_DONE: u32 = 0x2;
pub const MT7921_INT_MCU_EVENT: u32 = 0x4;
pub const MT7921_INT_TBTT: u32 = 0x8;
pub const MT7921_INT_ALL: u32 = 0xFFFFFFFF;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_addresses() {
        // Test that register addresses are correctly defined
        assert_eq!(MT7921_WPDMA_GLO_CFG, 0x0000);
        assert_eq!(MT7921_INT_ENABLE, 0x0200);
        assert_eq!(MT7921_INT_STATUS, 0x0204);
        assert_eq!(MT7921_INT_ACK, 0x0208);
        assert_eq!(MT7921_MAC_TX_CTRL, 0x1100);
        assert_eq!(MT7921_MAC_RX_CTRL, 0x1200);
        assert_eq!(MT7921_MAC_TX_BASE_PTR, 0x1000);
        assert_eq!(MT7921_MAC_RX_BASE_PTR, 0x2000);
        assert_eq!(MT7921_COEX_CTRL, 0x3000);
    }

    #[test]
    fn test_phy_registers() {
        assert_eq!(MT7921_PHY_CTRL, 0x4000);
        assert_eq!(MT7921_PHY_RF_CTRL, 0x4100);
        assert_eq!(MT7921_PHY_CHANNEL, 0x4200);
        assert_eq!(MT7921_PHY_BAND, 0x4204);
        assert_eq!(MT7921_PHY_BANDWIDTH, 0x4208);
    }

    #[test]
    fn test_mcu_registers() {
        assert_eq!(MT7921_MCU_CTRL, 0x5000);
        assert_eq!(MT7921_MCU_DOORBELL, 0x5004);
        assert_eq!(MT7921_MCU_INT_ENABLE, 0x5100);
        assert_eq!(MT7921_MCU_INT_STATUS, 0x5104);
        assert_eq!(MT7921_MCU_CMD, 0x5200);
        assert_eq!(MT7921_MCU_CMD_ID, 0x5204);
        assert_eq!(MT7921_MCU_EVENT, 0x5300);
        assert_eq!(MT7921_MCU_EVENT_ID, 0x5304);
    }

    #[test]
    fn test_mcu_commands() {
        assert_eq!(MT7921_MCU_CMD_FW_START, 0x01);
        assert_eq!(MT7921_MCU_CMD_FW_STOP, 0x02);
        assert_eq!(MT7921_MCU_CMD_SCAN, 0x10);
        assert_eq!(MT7921_MCU_CMD_CONNECT, 0x11);
        assert_eq!(MT7921_MCU_CMD_DISCONNECT, 0x12);
        assert_eq!(MT7921_MCU_CMD_SET_CHANNEL, 0x20);
        assert_eq!(MT7921_MCU_CMD_SET_BANDWIDTH, 0x21);
        assert_eq!(MT7921_MCU_CMD_SET_TX_POWER, 0x22);
        assert_eq!(MT7921_MCU_CMD_GET_STATS, 0x30);
    }

    #[test]
    fn test_interrupt_bits() {
        assert_eq!(MT7921_INT_TX_DONE, 0x1);
        assert_eq!(MT7921_INT_RX_DONE, 0x2);
        assert_eq!(MT7921_INT_MCU_EVENT, 0x4);
        assert_eq!(MT7921_INT_TBTT, 0x8);
        assert_eq!(MT7921_INT_ALL, 0xFFFFFFFF);
    }

    #[test]
    fn test_wpdma_config() {
        assert_eq!(MT7921_WPDMA_GLO_CFG_TX_DMA_EN, 0x1);
        assert_eq!(MT7921_WPDMA_GLO_CFG_RX_DMA_EN, 0x2);
    }
}
// ==== Firmware upload (staging window) ====
/// Base word offset of the SRAM staging window for the firmware blob.
pub const MT7921_FW_SRAM_WINDOW: usize = 0x4000;
pub const MT7921_FW_CTRL: usize = 0x4010;
pub const MT7921_FW_STATUS: usize = 0x4011;
pub const MT7921_FW_CTRL_START: u32 = 0x0000_0001;
pub const MT7921_FW_STATUS_BUSY: u32 = 0x0000_0001;

/// Crate-internal aliases used by the upload helper.
pub mod regsfw {
    pub use super::MT7921_FW_SRAM_WINDOW as REG_FW_SRAM_WINDOW;
    pub use super::MT7921_FW_CTRL as REG_FW_CTRL;
    pub use super::MT7921_FW_STATUS as REG_FW_STATUS;
    pub use super::MT7921_FW_CTRL_START as FW_CTRL_START;
    pub use super::MT7921_FW_STATUS_BUSY as FW_STATUS_BUSY;
}

// ==== RX/TX descriptor rings (device MMIO contract) ====
pub const RING_LEN: u32 = 32;
pub const RING_DESC_WORDS: usize = 4;
pub const REG_TXQ_RING: usize = 0x7000;
pub const REG_RXQ_RING: usize = 0x7080;
pub const REG_TXQ_HEAD: usize = 0x7100;
pub const REG_TXQ_TAIL: usize = 0x7101;
pub const REG_RXQ_HEAD: usize = 0x7102;
pub const REG_RXQ_TAIL: usize = 0x7103;
pub const TX_DATA_WINDOW: usize = 0x8000;
pub const RX_DATA_WINDOW: usize = 0xC000;
pub const FRAME_SLOT_BYTES: usize = 2048;
pub const FRAME_SLOT_WORDS: usize = FRAME_SLOT_BYTES / 4;

// ==== Scan control ====
/// Escrever 1 inicia uma rodada de scan; resultados chegam como beacons na RX ring.
pub const REG_SCAN_CTRL: usize = 0x7104;
