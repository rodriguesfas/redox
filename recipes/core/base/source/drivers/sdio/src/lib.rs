//! SDIO driver library for BCM2711 (Raspberry Pi 4)

use core::ptr::{read_volatile, write_volatile};
use core::time::Duration;
use std::thread;

use common::{physmap, Prot, MemoryType};

const SDIO_BASE: usize = 0xfe30_0000;
pub const SDIO_SIZE: usize = 0x10000;

// SDIO register offsets (BCM2711 SDHOST / EMMC2 / SDIO)
const SD_ARGUMENT: usize = 0x0000;
const SD_CMD: usize = 0x0004;
const SD_CMD_XFER: usize = 0x0008;
const SD_CMD_XFER_MODE: usize = 0x000C;
const SD_RESP0: usize = 0x0010;
const SD_RESP1: usize = 0x0014;
const SD_RESP2: usize = 0x0018;
const SD_RESP3: usize = 0x001C;
const SD_BUF: usize = 0x0020;
const SD_BUF_DATPORT: usize = 0x0030;
const SD_BLKSIZE: usize = 0x0034;
const SD_BLKCNT: usize = 0x0038;
const SD_ARGUMENT2: usize = 0x003C;
const SD_TUNING_CTRL: usize = 0x0040;
const SD_SPI_INT_STATUS: usize = 0x0044;
const SD_SPI_INT_MASK: usize = 0x0048;
const SD_SLAVE_INT_STATUS: usize = 0x004C;
const SD_SLAVE_INT_MASK: usize = 0x0050;
const SD_HOST_CTRL1: usize = 0x0054;
const SD_PWR_CTRL: usize = 0x0058;
const SD_BLK_GAP_CTRL: usize = 0x005C;
const SD_WAKEUP_CTRL: usize = 0x0060;
const SD_CLK_CTRL: usize = 0x0064;
const SD_TIMEOUT_CTRL: usize = 0x0068;
const SD_SW_RST: usize = 0x006C;
const SD_NORMAL_INT_STATUS: usize = 0x0070;
const SD_ERROR_INT_STATUS: usize = 0x0074;
const SD_NORMAL_INT_STATUS_EN: usize = 0x0078;
const SD_ERROR_INT_STATUS_EN: usize = 0x007C;
const SD_NORMAL_INT_SIGNAL_EN: usize = 0x0080;
const SD_ERROR_INT_SIGNAL_EN: usize = 0x0084;
const SD_AUTO_CMD_ERR_STATUS: usize = 0x0088;
const SD_HOST_CTRL2: usize = 0x008C;
const SD_CAPS: usize = 0x0090;
const SD_MAX_CURRENT: usize = 0x0098;
const SD_FORCE_EVENT_AUTO_CMD_ERR: usize = 0x00A0;
const SD_FORCE_EVENT_ERROR: usize = 0x00A4;
const SD_ADMA_ERR_STATUS: usize = 0x00A8;
const SD_ADMA_SYS_ADDR: usize = 0x00AC;
const SD_PRESET_VAL: usize = 0x00B0;
const SD_BOOT_TIMEOUT_CTRL: usize = 0x00C0;
const SD_SLOT_INT_STATUS: usize = 0x00FC;
const SD_HOST_CTRL_VERS: usize = 0x0100;

fn rd(base: usize, off: usize) -> u32 {
    unsafe { read_volatile((base + off) as *const u32) }
}

fn wr(base: usize, off: usize, val: u32) {
    unsafe { write_volatile((base + off) as *mut u32, val) }
}

/// SDIO controller for BCM2711
pub struct Sdio {
    base: usize,
}

impl Sdio {
    pub fn new(base: usize) -> Result<Self, &'static str> {
        // Reset the controller
        let mut reg = rd(base, SD_SW_RST);
        reg |= 1; // SW_RESET_ALL
        wr(base, SD_SW_RST, reg);
        
        // Wait for reset to complete
        for _ in 0..1000 {
            let reg = rd(base, SD_SW_RST);
            if reg & 1 == 0 {
                break;
            }
            thread::sleep(Duration::from_micros(100));
        }
        
        Ok(Self { base })
    }
    
    pub fn init(&mut self) -> Result<(), &'static str> {
        // Enable internal clock
        let mut reg = rd(self.base, SD_CLK_CTRL);
        reg |= 1; // INT_CLK_EN
        wr(self.base, SD_CLK_CTRL, reg);
        
        // Wait for internal clock to stabilize
        for _ in 0..1000 {
            let reg = rd(self.base, SD_CLK_CTRL);
            if reg & 2 != 0 { // INT_CLK_STABLE
                break;
            }
            thread::sleep(Duration::from_micros(100));
        }
        
        // Enable SD clock
        reg = rd(self.base, SD_CLK_CTRL);
        reg |= 4; // SD_CLK_EN
        wr(self.base, SD_CLK_CTRL, reg);
        
        // Software reset for DAT and CMD lines
        reg = rd(self.base, SD_SW_RST);
        reg |= (1 << 1) | (1 << 2); // SW_RESET_DAT | SW_RESET_CMD
        wr(self.base, SD_SW_RST, reg);
        
        for _ in 0..1000 {
            let reg = rd(self.base, SD_SW_RST);
            if (reg & ((1 << 1) | (1 << 2))) == 0 {
                break;
            }
            thread::sleep(Duration::from_micros(100));
        }
        
        // Enable normal interrupts
        wr(self.base, SD_NORMAL_INT_STATUS_EN, 0xFFFF);
        wr(self.base, SD_ERROR_INT_STATUS_EN, 0xFFFF);
        wr(self.base, SD_NORMAL_INT_SIGNAL_EN, 0xFFFF);
        wr(self.base, SD_ERROR_INT_SIGNAL_EN, 0xFFFF);
        
        // Set timeout
        wr(self.base, SD_TIMEOUT_CTRL, 0xE);
        
        Ok(())
    }
    
    // Byte-level register access for SDIO CCCR/FBR registers
    pub fn read_byte(&mut self, offset: u32) -> u8 {
        // CCCR registers are at the base of the SDIO function 0
        unsafe { core::ptr::read_volatile((self.base + offset as usize) as *const u8) }
    }
    
    pub fn write_byte(&mut self, offset: u32, val: u8) {
        unsafe { core::ptr::write_volatile((self.base + offset as usize) as *mut u8, val) }
    }
    
    // Backplane window access for Broadcom chips
    pub fn write_backplane_window(&mut self, window: u32) {
        // SBS (SDIO Bus Slave) register at 0x10000 for function 1
        let sbs_addr = self.base + 0x10000 + 0x10010; // SBADDRLOW/SBADDRMID/SBADDRHIGH
        wr(0, sbs_addr, window & 0xFFFF);
        wr(0, sbs_addr + 4, (window >> 16) & 0xFFFF);
    }
    
    pub fn read_backplane(&mut self, offset: u32) -> u32 {
        // Read from backplane window (function 1 data area)
        let addr = self.base + 0x10000 + 0x10000 + (offset as usize);
        rd(0, addr)
    }
    
    pub fn write_backplane(&mut self, offset: u32, val: u32) {
        let addr = self.base + 0x10000 + 0x10000 + (offset as usize);
        wr(0, addr, val);
    }
}