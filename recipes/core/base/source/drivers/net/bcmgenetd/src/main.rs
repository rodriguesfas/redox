mod breadcrumb;
mod genet;
mod led;

use std::thread;
use std::time::Duration;

use common;
use daemon::Daemon;
use fdt::Fdt;
use genet::Genet;

fn main() {
    daemon::Daemon::new(daemon);
}

fn daemon(daemon: Daemon) -> ! {
    common::init();
    common::setup_logging(
        "net",
        "platform",
        "bcmgenetd",
        common::output_level(),
        common::file_level(),
    );

    // LED (GPIO 42, active low). Map it early; tolerate failure.
    let led_base = unsafe {
        common::physmap(0xfe200000, 0x1000, common::Prot::RW, common::MemoryType::DeviceMemory)
    };
    let led = match led_base {
        Ok(p) => {
            let l = led::Led::new(p as usize);
            l.init();
            Some(l)
        }
        Err(_) => None,
    };
    if let Some(l) = &led {
        l.blink(3, 150, 150); // hello
    }

    // Read the DTB served by the kernel.
    let dtb_data = match std::fs::read("kernel.dtb:") {
        Ok(d) => d,
        Err(_) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(0, "dtb read failed", Some("cannot read kernel.dtb:"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    let fdt = match Fdt::new(&dtb_data) {
        Ok(f) => f,
        Err(_) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(0, "fdt parse failed", Some("Fdt::new failed"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    // Find the GENET node.
    let genet_node = match fdt.find_compatible(&["brcm,bcm2711-genet-v5"]) {
        Some(n) => n,
        None => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(1, "genet node not found", Some("no compatible brcm,bcm2711-genet-v5"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    // Physmap GENET MMIO (child 0x7d580000 -> CPU 0xfd580000 via /scb).
    let reg = match genet_node.reg().and_then(|mut r| r.next()) {
        Some(r) => r,
        None => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(2, "genet reg missing", Some("no reg property"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };
    let child_addr = reg.starting_address as usize;
    let size = reg.size.unwrap_or(0x10000) as usize;
    let cpu_addr = genet::translate(&fdt, child_addr);

    let base = match unsafe {
        common::physmap(cpu_addr, size, common::Prot::RW, common::MemoryType::DeviceMemory)
    } {
        Ok(p) => p as usize,
        Err(_) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(2, "physmap failed", Some("common::physmap GENET failed"));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    // MAC from DT (firmware sets local-mac-address).
    let mut mac = [0u8; 6];
    let mut have_mac = false;
    if let Some(prop) = genet_node.property("local-mac-address") {
        let v = prop.value;
        if v.len() >= 6 {
            mac.copy_from_slice(&v[..6]);
            have_mac = true;
        }
    }
    if !have_mac {
        mac = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    }

    // PHY address + max-speed from DT.
    let (phyaddr, max_speed) = genet::find_phy(&fdt, &genet_node);

    // rxid from phy-mode.
    let rxid = genet_node
        .property("phy-mode")
        .and_then(|p| p.as_str())
        .map(|s| s == "rgmii-rxid")
        .unwrap_or(false);

    if let Some(l) = &led {
        l.stage(1);
    }
    breadcrumb::write(1, "genet node found, mmio mapped", None);

    // Init GENET.
    let mut genet = match Genet::new(base, mac, phyaddr, max_speed, rxid) {
        Ok(g) => g,
        Err(e) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(2, "genet new failed", Some(e));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    };

    if let Some(l) = &led {
        l.stage(2);
    }

    match genet.init(led.as_ref().unwrap()) {
        Ok(()) => {}
        Err(e) => {
            if let Some(l) = &led {
                l.error();
            }
            breadcrumb::write(3, "genet init failed", Some(e));
            loop {
                thread::sleep(Duration::from_secs(10));
            }
        }
    }

    if let Some(l) = &led {
        l.stage(3);
    }
    breadcrumb::write(3, "genet init ok, starting beacons", None);

    daemon.ready();

    // Beacon loop.
    let mut counter: u64 = 0;
    loop {
        counter += 1;
        let msg = format!(
            "AIOS-BOOT n={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} speed={}",
            counter, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], genet.max_speed()
        );
        let _ = genet.tx_beacon(counter, &msg);
        genet.poll_rx();
        if counter % 10 == 0 {
            breadcrumb::write(4, "beacon loop running", None);
        }
        if let Some(l) = &led {
            l.success();
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn translate(fdt: &Fdt, child_addr: usize) -> usize {
    genet::translate(fdt, child_addr)
}

fn find_phy(fdt: &Fdt, genet: &fdt::node::FdtNode) -> (u32, u32) {
    genet::find_phy(fdt, genet)
}