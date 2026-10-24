//! brcmfmac - Broadcom FullMAC WiFi driver for BCM43455 (Raspberry Pi 4)
//! Uses SDIO interface and integrates with net80211 stack

use core::ptr::{read_volatile, write_volatile};
use core::time::Duration;
use std::thread;

use common::{physmap, Prot, MemoryType};
use daemon::Daemon;
use driver_network::{NetworkAdapter, NetworkScheme};
use fdt::Fdt;
use sdio::Sdio;

const SDIO_BASE: usize = 0xfe30_0000;

// Broadcom SDIO function numbers
const SDIO_FUNC_BACKPLANE: u8 = 1;
const SDIO_FUNC_WLAN: u8 = 2;

// Backplane address window
const BACKPLANE_WINDOW: u32 = 0x18000;

// ChipCommon core registers (relative to backplane base)
const CHIPCOMMON_BASE: u32 = 0x1800_0000;
const CC_CHIPCTL: usize = 0x680;
const CC_CAPABILITIES: usize = 0x684;
const CC_CHIPSTATUS: usize = 0x688;

// SDIO registers (function 1 - backplane)
const SDIO_CCCR_IOR: u32 = 0x00;
const SDIO_CCCR_CCCR: u32 = 0x01;
const SDIO_CCCR_CAPS: u32 = 0x02;
const SDIO_CCCR_IEN: u32 = 0x04;
const SDIO_CCCR_IENx: u32 = 0x05;
const SDIO_CCCR_INTx: u32 = 0x06;
const SDIO_CCCR_ABORT: u32 = 0x06;
const SDIO_CCCR_BUSCTRL: u32 = 0x07;
const SDIO_CCCR_CARDCTRL: u32 = 0x08;
const SDIO_CCCR_SPEED: u32 = 0x0D;
const SDIO_CCCR_UHS: u32 = 0x0E;
const SDIO_CCCR_DRIVE: u32 = 0x0F;
const SDIO_CCCR_INTEXT: u32 = 0x10;
const SDIO_CCCR_SEP_INT: u32 = 0x14;
const SDIO_CCCR_BLKSIZE: u32 = 0x10;

// F1 (Backplane) registers
const SDIO_F1_BASE: u32 = 0x10000;
const SDIO_F1_BLKSIZE: u32 = SDIO_F1_BASE + SDIO_CCCR_BLKSIZE;

// F2 (WLAN) registers
const SDIO_F2_BASE: u32 = 0x20000;
const SDIO_F2_BLKSIZE: u32 = SDIO_F2_BASE + SDIO_CCCR_BLKSIZE;

// BCM43455 chip ID
const BCM43455_CHIP_ID: u32 = 0x43455;

fn rd8(base: usize, off: u32) -> u8 {
    unsafe { read_volatile((base + off as usize) as *const u8) }
}

fn wr8(base: usize, off: u32, val: u8) {
    unsafe { write_volatile((base + off as usize) as *mut u8, val) }
}

fn rd32(base: usize, off: u32) -> u32 {
    unsafe { read_volatile((base + off as usize) as *const u32) }
}

fn wr32(base: usize, off: u32, val: u32) {
    unsafe { write_volatile((base + off as usize) as *mut u32, val) }
}

struct Brcmfmac {
    sdio: Sdio,
    mac_addr: [u8; 6],
    associated: bool,
    bssid: [u8; 6],
}

impl Brcmfmac {
    fn new(sdio: Sdio) -> Result<Self, &'static str> {
        let mut dev = Self {
            sdio,
            mac_addr: [0; 6],
            associated: false,
            bssid: [0; 6],
        };
        dev.init()?;
        Ok(dev)
    }
    
    fn init(&mut self) -> Result<(), &'static str> {
        // Enable SDIO function 1 (backplane)
        self.enable_func(SDIO_FUNC_BACKPLANE)?;
        
        // Read chip ID
        let chip_id = self.read_backplane(CHIPCOMMON_BASE + CC_CHIPCTL as u32);
        println!("BCM chip ID: 0x{:08x}", chip_id);
        
        // Enable function 2 (WLAN)
        self.enable_func(SDIO_FUNC_WLAN)?;
        
        // Set block size for F2 (WLAN)
        self.set_block_size(SDIO_FUNC_WLAN, 512)?;
        
        // Download firmware (simplified - in reality would load from /lib/firmware/brcm/)
        self.download_firmware()?;
        
        // Initialize WLAN core
        self.init_wlan_core()?;
        
        // Read MAC address (from OTP or firmware)
        self.read_mac_addr()?;
        
        Ok(())
    }
    
    fn enable_func(&mut self, func: u8) -> Result<(), &'static str> {
        // Write to CCCR IOEN to enable function
        let ior = self.sdio.read_byte(SDIO_CCCR_IOR);
        self.sdio.write_byte(SDIO_CCCR_IOR, ior | (1 << func));
        
        // Wait for IO Ready
        for _ in 0..1000 {
            let ior = self.sdio.read_byte(SDIO_CCCR_IOR);
            if ior & (1 << func) != 0 {
                break;
            }
            thread::sleep(Duration::from_micros(100));
        }
        
        Ok(())
    }
    
    fn set_block_size(&mut self, func: u8, size: u16) -> Result<(), &'static str> {
        let base = match func {
            1 => SDIO_F1_BLKSIZE,
            2 => SDIO_F2_BLKSIZE,
            _ => return Err("invalid function"),
        };
        
        self.sdio.write_byte(base, (size & 0xFF) as u8);
        self.sdio.write_byte(base + 1, ((size >> 8) & 0xFF) as u8);
        Ok(())
    }
    
    fn download_firmware(&mut self) -> Result<(), &'static str> {
        // In real implementation, would read from /lib/firmware/brcm/brcmfmac43455-sdio.bin
        // For now, skip firmware download - assume firmware is pre-loaded or in ROM
        println!("Firmware download skipped (stub)");
        Ok(())
    }
    
    fn init_wlan_core(&mut self) -> Result<(), &'static str> {
        // Initialize ARM CM3 core, enable WLAN
        // This is highly chip-specific and requires firmware
        println!("WLAN core init (stub)");
        Ok(())
    }
    
    fn read_mac_addr(&mut self) -> Result<(), &'static str> {
        // In real driver, read from OTP or firmware
        // For now, use a default
        self.mac_addr = [0x00, 0x1A, 0x11, 0x22, 0x33, 0x44];
        Ok(())
    }
    
    fn read_backplane(&mut self, addr: u32) -> u32 {
        // Set backplane window
        let window = (addr & !0xFFFF) | BACKPLANE_WINDOW;
        self.sdio.write_backplane_window(window);
        let offset = (addr & 0xFFFF) as u32;
        self.sdio.read_backplane(offset)
    }
    
    fn write_backplane(&mut self, addr: u32, val: u32) {
        let window = (addr & !0xFFFF) | BACKPLANE_WINDOW;
        self.sdio.write_backplane_window(window);
        let offset = (addr & 0xFFFF) as u32;
        self.sdio.write_backplane(offset, val);
    }
}

impl NetworkAdapter for Brcmfmac {
    fn mac_address(&mut self) -> [u8; 6] {
        self.mac_addr
    }
    
    fn available_for_read(&mut self) -> usize {
        0 // Would check RX queue
    }
    
    fn read_packet(&mut self, buf: &mut [u8]) -> Result<Option<usize>, syscall::Error> {
        // Read 802.11 frame from WLAN function, convert to Ethernet
        // For now, return None (no packets)
        Ok(None)
    }
    
    fn write_packet(&mut self, buf: &[u8]) -> Result<usize, syscall::Error> {
        if !self.associated {
            return Err(syscall::Error::new(syscall::ENOTCONN));
        }
        
        // Convert Ethernet to 802.11 and send via SDIO F2
        // Stub for now
        Ok(buf.len())
    }
    
    fn ctl_write(&mut self, cmd: &[u8]) -> Result<usize, syscall::Error> {
        let cmd_str = core::str::from_utf8(cmd).unwrap_or("").trim();
        
        match cmd_str.split_whitespace().next() {
            Some("scan") => {
                // Trigger scan
                println!("Scan requested");
            }
            Some("connect") => {
                // Connect to AP
                println!("Connect requested");
            }
            Some("disconnect") => {
                self.associated = false;
                println!("Disconnect");
            }
            _ => {}
        }
        Ok(cmd.len())
    }
    
    fn ctl_read(&mut self, buf: &mut [u8]) -> Result<usize, syscall::Error> {
        let status = format!("associated={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\n",
            self.associated,
            self.mac_addr[0], self.mac_addr[1], self.mac_addr[2],
            self.mac_addr[3], self.mac_addr[4], self.mac_addr[5]
        );
        let bytes = status.as_bytes();
        let len = bytes.len().min(buf.len());
        buf[..len].copy_from_slice(&bytes[..len]);
        Ok(len)
    }
}

fn main() {
    Daemon::new(daemon);
}

fn daemon(daemon: Daemon) -> ! {
    common::init();
    common::setup_logging(
        "net", "platform", "brcmfmac",
        common::output_level(), common::file_level()
    );
    
    let dtb_data = std::fs::read("kernel.dtb:").unwrap();
    let fdt = Fdt::new(&dtb_data).unwrap();
    
    // Find SDIO node
    let sdio_node = fdt.find_compatible(&["brcm,bcm43455-sdio", "brcm,bcm43455"])
        .expect("BCM43455 SDIO node not found in DT");
    
    let reg = sdio_node.reg().and_then(|mut r| r.next()).unwrap();
    let child_addr = reg.starting_address as usize;
    let size = reg.size.unwrap_or(0x10000) as usize;
    
    let base = unsafe { physmap(child_addr, size, Prot::RW, MemoryType::DeviceMemory) }.unwrap() as usize;
    
    let mut sdio = Sdio::new(base).unwrap();
    sdio.init().unwrap();
    
    let mut brcmfmac = Brcmfmac::new(sdio).unwrap();
    
    let scheme_name = "network.wlan0".to_string();
    let mut scheme = NetworkScheme::new(
        move || brcmfmac,
        daemon,
        scheme_name,
    );
    
    println!("brcmfmac: network.wlan0 registered");
    
    loop {
        scheme.tick().unwrap();
        thread::sleep(Duration::from_millis(10));
    }
}