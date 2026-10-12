//! Register definitions for Intel iwlwifi

// CSR (Control Status Register) offsets
pub const CSR_HW_IF_CONFIG_REG: usize = 0x0000;
pub const CSR_INT: usize = 0x0008;
pub const CSR_INT_MASK: usize = 0x000C;
pub const CSR_INT_ACK: usize = 0x0010;
pub const CSR_RESET: usize = 0x0020;
pub const CSR_GP_CNTRL: usize = 0x0024;
pub const CSR_HW_REV: usize = 0x0028;
pub const CSR_EEPROM_REG: usize = 0x0030;
pub const CSR_EEPROM_GP: usize = 0x0034;
pub const CSR_OTP_GP: usize = 0x0038;
pub const CSR_GIO_REG: usize = 0x003C;
pub const CSR_GP_UCODE: usize = 0x0040;
pub const CSR_GP_DRIVER: usize = 0x0044;
pub const CSR_UCODE_DRV_GP1: usize = 0x0048;
pub const CSR_UCODE_DRV_GP2: usize = 0x004C;
pub const CSR_LED_REG: usize = 0x004C;
pub const CSR_DRAM_INT_TBL_REG: usize = 0x0050;
pub const CSR_GIO_CHICKEN_BITS: usize = 0x0054;
pub const CSR_ANA_PLL_CFG: usize = 0x0058;
pub const CSR_HW_REV_WA_REG: usize = 0x005C;
pub const CSR_DBG_HPET_MEM_REG: usize = 0x0060;
pub const CSR_DBG_LINK_PWR_MGMT_REG: usize = 0x0064;

// CSR_RESET bits
pub const CSR_RESET_BIT: u32 = 0x1;

// CSR_INT bits
pub const CSR_INT_TX_DONE: u32 = 0x00000001;
pub const CSR_INT_RX_DONE: u32 = 0x00000002;
pub const CSR_INT_MVM: u32 = 0x00000004;
pub const CSR_INT_TBTT: u32 = 0x00000008;
pub const CSR_INT_RF_KILL: u32 = 0x00000010;
pub const CSR_INT_PERIODIC: u32 = 0x00000020;
pub const CSR_INT_HW_ERR: u32 = 0x00000040;
pub const CSR_INT_SW_ERR: u32 = 0x00000080;

// FH (Frame Handler) registers
pub const FH_RSCSR_CHNL0_STTS_WPTR_REG: usize = 0x0100;
pub const FH_RSCSR_CHNL0_RBDCB_BASE_REG: usize = 0x0108;
pub const FH_RSCSR_CHNL0_WPTR: usize = 0x0110;
pub const FH_MEM_RCSR_CHNL0_CONFIG_REG: usize = 0x0118;
pub const FH_MEM_RCSR_CHNL0_RBDCB_WPTR_REG: usize = 0x011C;
pub const FH_MEM_RCSR_CHNL0_FLUSH_RB_REQ: usize = 0x0120;

// HBUS_TARG registers
pub const HBUS_TARG_MBX_C: usize = 0x0200;
pub const HBUS_TARG_MBX_D: usize = 0x0204;
pub const HBUS_TARG_MBX_E: usize = 0x0208;
pub const HBUS_TARG_MBX_F: usize = 0x020C;

// TX/RX rings
pub const FH_RSCSR_CHNL0_TX_RING_BASE: usize = 0x0300;
pub const FH_RSCSR_CHNL0_RX_RING_BASE: usize = 0x0400;

// Device IDs for supported devices
pub const DEVICE_AX210: u16 = 0x2723;
pub const DEVICE_AX210_VPRO: u16 = 0x2725;
pub const DEVICE_AX200: u16 = 0x7AF0;
pub const DEVICE_AX200_VPRO: u16 = 0x7AF1;
pub const DEVICE_AX201: u16 = 0x7DF0;
pub const DEVICE_AX201_VPRO: u16 = 0x7DF1;
pub const DEVICE_AX211: u16 = 0x2720;
pub const DEVICE_AX211_VPRO: u16 = 0x2721;
pub const DEVICE_BE200: u16 = 0x54F0;
pub const DEVICE_BE200_VPRO: u16 = 0x54F1;

// Firmware API versions
pub const FW_API_VERSION_1: u32 = 1;
pub const FW_API_VERSION_2: u32 = 2;
pub const FW_API_VERSION_3: u32 = 3;
pub const FW_API_VERSION_4: u32 = 4;
pub const FW_API_VERSION_5: u32 = 5;
pub const FW_API_VERSION_6: u32 = 6;
pub const FW_API_VERSION_7: u32 = 7;
pub const FW_API_VERSION_8: u32 = 8;
pub const FW_API_VERSION_9: u32 = 9;
pub const FW_API_VERSION_10: u32 = 10;

// TLV (Type-Length-Value) types for firmware
pub const FW_TLV_TYPE_UCODE: u32 = 0x01;
pub const FW_TLV_TYPE_DEBUG: u32 = 0x02;
pub const FW_TLV_TYPE_CALIB: u32 = 0x03;
pub const FW_TLV_TYPE_RADIO: u32 = 0x04;
pub const FW_TLV_TYPE_REGULATORY: u32 = 0x05;
pub const FW_TLV_TYPE_NVM: u32 = 0x06;
pub const FW_TLV_TYPE_SEC_RT: u32 = 0x07;
pub const FW_TLV_TYPE_SEC_MANUF: u32 = 0x08;
pub const FW_TLV_TYPE_SEC_SBOOT: u32 = 0x09;

// PHY configuration
pub const PHY_SKU_N_2x2: u32 = 0x1;
pub const PHY_SKU_AC_2x2: u32 = 0x2;
pub const PHY_SKU_AX_2x2: u32 = 0x4;
pub const PHY_SKU_BE_2x2: u32 = 0x8;

// MAC configuration
pub const MAC_MTU_MAX: usize = 2304;
pub const MAC_HEADER_SIZE: usize = 24;
pub const MAC_FCS_SIZE: usize = 4;

// MAC register offsets
pub const MAC_TX_CTRL: usize = 0x1100;
pub const MAC_TX_CTRL_ENABLE: u32 = 0x1;
pub const MAC_RX_CTRL: usize = 0x1200;
pub const MAC_RX_CTRL_ENABLE: u32 = 0x1;
pub const MAC_ADDRESS_LOW: usize = 0x1300;
pub const MAC_ADDRESS_HIGH: usize = 0x1304;
pub const MAC_COEX_CTRL: usize = 0x3000;
pub const MAC_COEX_CTRL_ENABLE: u32 = 0x1;
pub const MAC_COEX_TABLE: usize = 0x3004;

// MAC power management registers
pub const MAC_PM_CTRL: usize = 0x3500;
pub const MAC_PM_CTRL_DISABLE: u32 = 0x0;
pub const MAC_PM_CTRL_ENABLE: u32 = 0x1;
pub const MAC_PM_CTRL_UAPSD_ENABLE: u32 = 0x2;

// TX/RX descriptor flags
pub const TX_DESC_DONE: u32 = 0x80000000;
pub const TX_DESC_VALID: u32 = 0x1;
pub const RX_DESC_DONE: u32 = 0x80000000;
pub const RX_DESC_VALID: u32 = 0x1;

// PHY register offsets
pub const PHY_CTRL: usize = 0x4000;
pub const PHY_CTRL_ENABLE: u32 = 0x1;
pub const PHY_RF_CTRL: usize = 0x4100;
pub const PHY_RF_CTRL_ENABLE: u32 = 0x1;
pub const PHY_CHANNEL: usize = 0x4200;
pub const PHY_BAND: usize = 0x4204;
pub const PHY_BANDWIDTH: usize = 0x4208;
pub const PHY_TX_POWER: usize = 0x4210;
pub const PHY_RSSI: usize = 0x4300;
pub const PHY_NOISE_FLOOR: usize = 0x4304;

// MVM register offsets
pub const MVM_CTRL: usize = 0x5000;
pub const MVM_CTRL_ENABLE: u32 = 0x1;
pub const MVM_DOORBELL: usize = 0x5004;
pub const MVM_INT_ENABLE: usize = 0x5100;
pub const MVM_INT_STATUS: usize = 0x5104;
pub const MVM_STATUS: usize = 0x5108;
pub const MVM_STATUS_FW_READY: u32 = 0x1;
pub const MVM_CMD: usize = 0x5200;
pub const MVM_CMD_ID: usize = 0x5204;
pub const MVM_CMD_DATA: usize = 0x5208;
pub const MVM_EVENT: usize = 0x5300;
pub const MVM_EVENT_ID: usize = 0x5304;
pub const MVM_EVENT_DATA: usize = 0x5308;

// MVM Commands
pub const MVM_CMD_FW_START: u32 = 0x01;
pub const MVM_CMD_FW_STOP: u32 = 0x02;
pub const MVM_CMD_SCAN: u32 = 0x10;
pub const MVM_CMD_CONNECT: u32 = 0x11;
pub const MVM_CMD_DISCONNECT: u32 = 0x12;
pub const MVM_CMD_ROAM: u32 = 0x13;
pub const MVM_CMD_SET_CHANNEL: u32 = 0x20;
pub const MVM_CMD_SET_BANDWIDTH: u32 = 0x21;
pub const MVM_CMD_SET_TX_POWER: u32 = 0x22;
pub const MVM_CMD_ADD_BA: u32 = 0x30;
pub const MVM_CMD_DEL_BA: u32 = 0x31;
pub const MVM_CMD_GET_STATS: u32 = 0x40;

// MVM Events (async notifications)
pub const MVM_EVENT_RX: u32 = 0x100;
pub const MVM_EVENT_TX: u32 = 0x101;
pub const MVM_EVENT_BA: u32 = 0x102;
pub const MVM_EVENT_SCAN: u32 = 0x103;
pub const MVM_EVENT_BEACON: u32 = 0x104;
pub const MVM_EVENT_TDLS: u32 = 0x105;

// TX register offsets
pub const TX_QUEUE_BASE: usize = 0x1000;
pub const TX_QUEUE_STRIDE: usize = 0x1000;
pub const TX_DMA_CTRL: usize = 0x0100;
pub const TX_DMA_CTRL_ENABLE: u32 = 0x1;
pub const TX_DOORBELL: usize = 0x0180;

// RX register offsets
pub const RX_QUEUE_MAIN_BASE: usize = 0x2000;
pub const RX_QUEUE_MONITOR_BASE: usize = 0x2800;
pub const RX_QUEUE_MCAST_BASE: usize = 0x2C00;
pub const RX_DMA_CTRL: usize = 0x0200;
pub const RX_DMA_CTRL_ENABLE: u32 = 0x1;
pub const RX_MONITOR_CTRL: usize = 0x0204;
pub const RX_MONITOR_CTRL_ENABLE: u32 = 0x1;
pub const RX_RSSI_THRESHOLD: usize = 0x0208;

// Debug registers
pub const DBG_TX_FIFO: usize = 0x6000;
pub const DBG_RX_FIFO: usize = 0x6100;
pub const DBG_MVM_STATUS: usize = 0x6200;
pub const DBG_FW_ERROR: usize = 0x6300;
pub const DBG_TEMPERATURE: usize = 0x6400;
pub const DBG_VOLTAGE: usize = 0x6404;
// ==== Firmware upload (staging window) ====
/// Base word offset of the SRAM staging window for the firmware blob.
pub const REG_FW_SRAM_WINDOW: usize = 0x5000;
pub const REG_FW_CTRL: usize = 0x5010;
pub const REG_FW_STATUS: usize = 0x5011;
pub const FW_CTRL_START: u32 = 0x0000_0001;
pub const FW_STATUS_BUSY: u32 = 0x0000_0001;

// ==== RX/TX descriptor rings (device MMIO contract) ====
pub const RING_LEN: u32 = 32;
pub const RING_DESC_WORDS: usize = 4;
pub const REG_TXQ_RING: usize = 0x2800;
pub const REG_RXQ_RING: usize = 0x2880;
pub const REG_TXQ_HEAD: usize = 0x2A00;
pub const REG_TXQ_TAIL: usize = 0x2A01;
pub const REG_RXQ_HEAD: usize = 0x2A02;
pub const REG_RXQ_TAIL: usize = 0x2A03;
pub const TX_DATA_WINDOW: usize = 0x4000;
pub const RX_DATA_WINDOW: usize = 0x8000;
pub const FRAME_SLOT_BYTES: usize = 2048;
pub const FRAME_SLOT_WORDS: usize = FRAME_SLOT_BYTES / 4;
