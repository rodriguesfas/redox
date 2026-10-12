//! Register definitions for Realtek rtw88

// Device IDs
pub const RTL8852AE: u16 = 0xC852;
pub const RTL8852BE: u16 = 0xC853;
pub const RTL8852CE: u16 = 0xC854;
pub const RTL8852BE_V2: u16 = 0xC855;

// Interrupt Status Registers
pub const REG_HISR0: usize = 0x00B0;
pub const REG_HISR1: usize = 0x00B4;
pub const REG_HIMR0: usize = 0x00B8;
pub const REG_HIMR1: usize = 0x00BC;

// HISR0 bits
pub const HISR0_TX_OK: u32 = 0x00000001;
pub const HISR0_TX_BCN_OK: u32 = 0x00000002;
pub const HISR0_RX_OK: u32 = 0x00000004;
pub const HISR0_C2H_CMD: u32 = 0x00000008;
pub const HISR0_TBTT: u32 = 0x00000010;
pub const HISR0_TX_ERR: u32 = 0x00000020;
pub const HISR0_RX_ERR: u32 = 0x00000040;
pub const HISR0_GPIO: u32 = 0x00000080;
pub const HISR0_FW_INT: u32 = 0x00000100;
pub const HISR0_POWER_SAVE: u32 = 0x00000200;
pub const HISR0_COEX: u32 = 0x00000400;

// HISR1 bits
pub const HISR1_TX_FIFO_OVERFLOW: u32 = 0x00000001;
pub const HISR1_RX_FIFO_OVERFLOW: u32 = 0x00000002;
pub const HISR1_TX_DMA_DONE: u32 = 0x00000004;
pub const HISR1_RX_DMA_DONE: u32 = 0x00000008;

// MAC Registers
pub const REG_CR: usize = 0x0100;
pub const CR_HCI_TXDMA_EN: u32 = 0x01;
pub const CR_HCI_RXDMA_EN: u32 = 0x02;
pub const CR_TXDMA_EN: u32 = 0x04;
pub const CR_RXDMA_EN: u32 = 0x08;
pub const CR_PROTOCOL_EN: u32 = 0x10;
pub const CR_SCHEDULE_EN: u32 = 0x20;
pub const CR_MACTXEN: u32 = 0x40;
pub const CR_MACRXEN: u32 = 0x80;

pub const REG_RCR: usize = 0x0104;
pub const RCR_AAP: u32 = 0x00000001;
pub const RCR_APM: u32 = 0x00000002;
pub const RCR_AM: u32 = 0x00000004;
pub const RCR_AB: u32 = 0x00000008;
pub const RCR_APWRMGT: u32 = 0x00000010;
pub const RCR_ADD3: u32 = 0x00000020;
pub const RCR_CBSSID_DATA: u32 = 0x00000040;
pub const RCR_ADD3_DATA: u32 = 0x00000080;
pub const RCR_AMF: u32 = 0x00000100;
pub const RCR_ACF: u32 = 0x00000200;
pub const RCR_ADF: u32 = 0x00000400;
pub const RCR_AICV: u32 = 0x00000800;
pub const RCR_ACRC32: u32 = 0x00001000;
pub const RCR_HTC_LOC_CTRL: u32 = 0x00002000;
pub const RCR_AMSDU: u32 = 0x00004000;
pub const RCR_APP_FCS: u32 = 0x00008000;
pub const RCR_APP_MIC: u32 = 0x00010000;
pub const RCR_APP_PHYSTS: u32 = 0x00020000;
pub const RCR_ACF_VENDOR: u32 = 0x00040000;
pub const RCR_HTC_AMSDU: u32 = 0x00080000;

// TX/RX Queue Registers
pub const REG_TXQ_BASE: usize = 0x0200;
pub const REG_RXQ_BASE: usize = 0x0300;

// TX Queue Control
pub const REG_TXQ_CTRL: usize = 0x0200;
pub const TXQ_CTRL_EN: u32 = 0x1;
pub const TXQ_CTRL_STOP: u32 = 0x2;

// RX Queue Control
pub const REG_RXQ_CTRL: usize = 0x0300;
pub const RXQ_CTRL_EN: u32 = 0x1;

// MAC Address Registers
pub const REG_MACID: usize = 0x0110;
pub const REG_BSSID: usize = 0x0118;

// PHY Registers
pub const REG_RF_CTRL: usize = 0x0400;
pub const RF_CTRL_EN: u32 = 0x1;
pub const RF_CTRL_RST: u32 = 0x2;

pub const REG_RF_CHNL: usize = 0x0410;
pub const REG_RF_BW: usize = 0x0414;
pub const REG_RF_TXPWR: usize = 0x0418;
pub const REG_RF_RSSI: usize = 0x0420;
pub const REG_RF_NF: usize = 0x0424;

// Firmware Mailbox
pub const REG_HMBOX_C2H: usize = 0x01A0;
pub const REG_HMBOX_H2C: usize = 0x01A4;
pub const REG_HMBOX_EXT: usize = 0x01A8;

// C2H (CPU to Host) Commands
pub const C2H_CMD_BASE: u32 = 0x0000;
pub const C2H_CMD_TX_REPORT: u32 = 0x01;
pub const C2H_CMD_RX_REPORT: u32 = 0x02;
pub const C2H_CMD_CCX_REPORT: u32 = 0x03;
pub const C2H_CMD_BT_INFO: u32 = 0x04;
pub const C2H_CMD_WLAN_INFO: u32 = 0x05;
pub const C2H_CMD_RA_REPORT: u32 = 0x06;
pub const C2H_CMD_BEACON_REPORT: u32 = 0x07;
pub const C2H_CMD_CHANNEL_SWITCH: u32 = 0x08;
pub const C2H_CMD_SCAN_REPORT: u32 = 0x09;
pub const C2H_CMD_ROAM_REPORT: u32 = 0x0A;

// H2C (Host to CPU) Commands
pub const H2C_CMD_BASE: u32 = 0x1000;
pub const H2C_CMD_MACID_CONFIG: u32 = 0x1001;
pub const H2C_CMD_JOIN_BSS: u32 = 0x1002;
pub const H2C_CMD_LEAVE_BSS: u32 = 0x1003;
pub const H2C_CMD_CHANNEL_SWITCH: u32 = 0x1004;
pub const H2C_CMD_BW_SWITCH: u32 = 0x1005;
pub const H2C_CMD_TX_POWER: u32 = 0x1006;
pub const H2C_CMD_RA_MASK: u32 = 0x1007;
pub const H2C_CMD_RA_PARAM: u32 = 0x1008;
pub const H2C_CMD_BEACON_CONFIG: u32 = 0x1009;
pub const H2C_CMD_PS_MODE: u32 = 0x100A;
pub const H2C_CMD_COEX_CONFIG: u32 = 0x100B;
pub const H2C_CMD_FW_OFLD: u32 = 0x100C;
pub const H2C_CMD_H2C_FW: u32 = 0x100D;
pub const H2C_CMD_MACID_PAUSE: u32 = 0x100E;
pub const H2C_CMD_MACID_RESUME: u32 = 0x100F;

// Firmware TLV Types
pub const FW_TLV_TYPE_MAIN: u32 = 0x01;
pub const FW_TLV_TYPE_WOW: u32 = 0x02;
pub const FW_TLV_TYPE_BT: u32 = 0x03;
pub const FW_TLV_TYPE_COEX: u32 = 0x04;
pub const FW_TLV_TYPE_PHY_REG: u32 = 0x05;
pub const FW_TLV_TYPE_RF_REG: u32 = 0x06;
pub const FW_TLV_TYPE_MAC_REG: u32 = 0x07;
pub const FW_TLV_TYPE_TXPWR: u32 = 0x08;

// EFUSE Map
pub const EFUSE_BASE: usize = 0x0500;
pub const EFUSE_REAL_CONTENT_LEN: usize = 512;
pub const EFUSE_MAP_LEN: usize = 1024;

// EFUSE offsets
pub const EFUSE_VID: usize = 0x00;
pub const EFUSE_DID: usize = 0x02;
pub const EFUSE_SVID: usize = 0x04;
pub const EFUSE_SDID: usize = 0x06;
pub const EFUSE_MAC_ADDR: usize = 0x10;
pub const EFUSE_CRYSTAL_CAP: usize = 0x20;
pub const EFUSE_THERMAL_METER: usize = 0x22;
pub const EFUSE_IQK_LCK: usize = 0x24;
pub const EFUSE_TXPWR_2G: usize = 0x30;
pub const EFUSE_TXPWR_5G: usize = 0x40;
pub const EFUSE_TXPWR_6G: usize = 0x50;

// Coexistence Registers
pub const REG_COEX_CTRL: usize = 0x0600;
pub const COEX_CTRL_EN: u32 = 0x1;
pub const COEX_CTRL_WL_TX: u32 = 0x2;
pub const COEX_CTRL_WL_RX: u32 = 0x4;
pub const COEX_CTRL_BT_TX: u32 = 0x8;
pub const COEX_CTRL_BT_RX: u32 = 0x10;

pub const REG_COEX_TABLE: usize = 0x0610;

// Debug Registers
pub const REG_DBG_CTRL: usize = 0x0800;
pub const REG_DBG_SEL: usize = 0x0804;
pub const REG_DBG_DATA: usize = 0x0808;

// TX Descriptor Flags
pub const TX_DESC_OWN: u32 = 0x80000000;
pub const TX_DESC_FS: u32 = 0x40000000;
pub const TX_DESC_LS: u32 = 0x20000000;
pub const TX_DESC_AGG_EN: u32 = 0x10000000;
pub const TX_DESC_AMPDU_DENSITY: u32 = 0x0F000000;
pub const TX_DESC_SEC_TYPE: u32 = 0x00F00000;
pub const TX_DESC_HW_SEQ_EN: u32 = 0x00080000;
pub const TX_DESC_MORE_FRAG: u32 = 0x00040000;
pub const TX_DESC_RTS_EN: u32 = 0x00020000;
pub const TX_DESC_CTS_EN: u32 = 0x00010000;
pub const TX_DESC_HW_RTS_EN: u32 = 0x00008000;
pub const TX_DESC_NAV_USE_HDR: u32 = 0x00004000;
pub const TX_DESC_MBSSID: u32 = 0x00003F00;
pub const TX_DESC_PKT_OFFSET: u32 = 0x000000FF;

// RX Descriptor Flags
pub const RX_DESC_OWN: u32 = 0x80000000;
pub const RX_DESC_EOR: u32 = 0x40000000;
pub const RX_DESC_FS: u32 = 0x20000000;
pub const RX_DESC_LS: u32 = 0x10000000;
pub const RX_DESC_CRC32: u32 = 0x08000000;
pub const RX_DESC_ICV_ERR: u32 = 0x04000000;
pub const RX_DESC_DEC_ERR: u32 = 0x02000000;
pub const RX_DESC_CRC_ERR: u32 = 0x01000000;
pub const RX_DESC_MIC_ERR: u32 = 0x00800000;
pub const RX_DESC_REPLAY: u32 = 0x00400000;
pub const RX_DESC_TYPE_MASK: u32 = 0x003F0000;
pub const RX_DESC_PHYSTS: u32 = 0x00008000;
pub const RX_DESC_SW_DEC: u32 = 0x00004000;
pub const RX_DESC_HW_DEC: u32 = 0x00002000;
pub const RX_DESC_ENCRYPT_TYPE: u32 = 0x00001F00;
pub const RX_DESC_AMSDU: u32 = 0x00000080;
pub const RX_DESC_AMPDU: u32 = 0x00000040;
pub const RX_DESC_UC: u32 = 0x00000020;
pub const RX_DESC_MC: u32 = 0x00000010;
pub const RX_DESC_BC: u32 = 0x00000008;
pub const RX_DESC_MY_BSSID: u32 = 0x00000004;
pub const RX_DESC_DATA_RATE: u32 = 0x00000003;

// Band definitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Band2Ghz = 0,
    Band5Ghz = 1,
    Band6Ghz = 2,
}

// Bandwidth definitions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bandwidth {
    Bw20 = 0,
    Bw40 = 1,
    Bw80 = 2,
    Bw160 = 3,
}

// Crystal Cap
pub const CRYSTAL_CAP_DEFAULT: u8 = 0x20;

// Thermal Meter
pub const THERMAL_METER_DEFAULT: u8 = 0x1F;

// MAC Configuration
pub const MAC_MTU_MAX: usize = 2304;
pub const MAC_HEADER_SIZE: usize = 24;
pub const MAC_FCS_SIZE: usize = 4;
// ==== Firmware upload (staging window) ====
/// Base word offset of the SRAM staging window for the firmware blob.
pub const REG_FW_SRAM_WINDOW: usize = 0x3000;
pub const REG_FW_CTRL: usize = 0x3010;
pub const REG_FW_STATUS: usize = 0x3011;
pub const FW_CTRL_START: u32 = 0x0000_0001;
pub const FW_STATUS_BUSY: u32 = 0x0000_0001;
