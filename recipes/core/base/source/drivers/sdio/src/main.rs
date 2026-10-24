use sdio::Sdio;
use common::{physmap, Prot, MemoryType};
use fdt::Fdt;

fn main() {
    daemon::Daemon::new(daemon);
}

fn daemon(_: daemon::Daemon) -> ! {
    common::init();
    
    let dtb_data = std::fs::read("kernel.dtb:").unwrap();
    let fdt = Fdt::new(&dtb_data).unwrap();
    
    // Find SDIO node for BCM43455
    let sdio_node = fdt.find_compatible(&["brcm,bcm43455-sdio", "brcm,bcm43455"]).unwrap();
    
    let reg = sdio_node.reg().and_then(|mut r| r.next()).unwrap();
    let child_addr = reg.starting_address as usize;
    let size = reg.size.unwrap_or(sdio::SDIO_SIZE) as usize;
    
    // Map the SDIO controller
    let base = unsafe { physmap(child_addr, size, Prot::RW, MemoryType::DeviceMemory) }.unwrap() as usize;
    
    let mut sdio = Sdio::new(base).unwrap();
    sdio.init().unwrap();
    
    println!("SDIO initialized for BCM43455");
    
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
}