//! Synopsys DesignWare I2C host driver (Master mode) — MMIO register map.
//!
//! Alvo: Acer Nitro (i2c_designware.0 em PCI 00:15.0).

#![allow(dead_code)]

// Regs (word offsets = byte offset / 4)
pub const REG_CON: usize = 0x00 / 4;
pub const REG_TAR: usize = 0x04 / 4;
pub const REG_DATA_CMD: usize = 0x10 / 4;
pub const REG_SS_SCL_HCNT: usize = 0x14 / 4;
pub const REG_SS_SCL_LCNT: usize = 0x18 / 4;
pub const REG_FS_SCL_HCNT: usize = 0x1C / 4;
pub const REG_FS_SCL_LCNT: usize = 0x20 / 4;
pub const REG_INTSTAT: usize = 0x2C / 4;
pub const REG_INTMASK: usize = 0x30 / 4;
pub const REG_RXFLR: usize = 0x38 / 4;
pub const REG_TXFLR: usize = 0x3C / 4;
pub const REG_CLR_INTR: usize = 0x40 / 4;
pub const REG_CLR_RX_FULL: usize = 0x60 / 4;
pub const REG_CLR_TX_ABRT: usize = 0x54 / 4;
pub const REG_ENABLE: usize = 0x6C / 4;
pub const REG_STATUS: usize = 0x70 / 4;
pub const REG_COMP_TYPE: usize = 0xFC / 4;

// IC_CON bits
pub const CON_MASTER: u32 = 1 << 0;
pub const CON_SPEED_SS: u32 = 1 << 1; // Standard 100k
pub const CON_SPEED_FS: u32 = 2 << 1; // Fast 400k
pub const CON_ADDR_7BIT: u32 = 1 << 4;
pub const CON_SLAVE_DISABLE: u32 = 1 << 6;
pub const CON_RESTART_EN: u32 = 1 << 5;

// IC_STATUS bits
pub const STATUS_ACTIVE: u32 = 1 << 0;
pub const STATUS_TFNF: u32 = 1 << 1;
pub const STATUS_TFE: u32 = 1 << 2;
pub const STATUS_RFNE: u32 = 1 << 3;
pub const STATUS_RFF: u32 = 1 << 4;
pub const STATUS_MST_ACTIVITY: u32 = 1 << 5;

// IC_DATA_CMD bits
pub const DATA_CMD_READ: u32 = 1 << 8;
pub const DATA_CMD_STOP: u32 = 1 << 9;
pub const DATA_CMD_RESTART: u32 = 1 << 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reg_offsets_correct() {
        assert_eq!(REG_CON, 0x00);
        assert_eq!(REG_TAR, 0x01);
        assert_eq!(REG_DATA_CMD, 0x04);
        assert_eq!(REG_ENABLE, 0x1B);
        assert_eq!(REG_STATUS, 0x1C);
    }

    #[test]
    fn con_composer_values() {
        let con = CON_MASTER | CON_SPEED_FS | CON_ADDR_7BIT | CON_RESTART_EN | CON_SLAVE_DISABLE;
        assert_eq!(con, 0x75);
    }

    #[test]
    fn status_bits() {
        assert_eq!(STATUS_TFNF, 2);
        assert_eq!(STATUS_RFNE, 8);
        assert_eq!(STATUS_TFE, 4);
    }
}
