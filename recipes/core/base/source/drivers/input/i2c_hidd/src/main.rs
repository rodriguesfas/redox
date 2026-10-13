//! i2c-hidd — daemon I2C HID over DesignWare I2C (PCI 00:15.0, 8086:51e8).
//!
//! Alvo: touchpad PixArt PIXA3848 (093A:3848) do Acer Nitro (#31).

extern crate alloc;

use alloc::vec::Vec;
use common::MemoryType;
use i2c_dwd::Dwd;
use i2c_hid::{I2cBus, I2cHidDevice, parse_contact_block};
#[cfg(not(test))]
use inputd::ProducerHandle;
use pcid_interface::irq_helpers::pci_allocate_interrupt_vector;
use pcid_interface::PciFunctionHandle;

#[cfg(not(test))]
fn main() {
    pcid_interface::pci_daemon(daemon);
}

#[cfg(not(test))]
struct DwdBus(pub Dwd);

#[cfg(not(test))]
impl I2cBus for DwdBus {
    fn tx(&mut self, addr: u8, data: &[u8]) -> Result<(), &'static str> {
        self.0.master_tx(addr, data)
    }
    fn rx(&mut self, addr: u8, out: &mut [u8]) -> Result<(), &'static str> {
        self.0.master_rx(addr, out)
    }
    fn tx_rx(&mut self, addr: u8, tx: &[u8], out: &mut [u8]) -> Result<(), &'static str> {
        self.0.master_tx(addr, tx)?;
        self.0.master_rx(addr, out)
    }
}

#[cfg(not(test))]
fn daemon(d: daemon::Daemon, mut pcid_handle: PciFunctionHandle) -> ! {
    let _ = d;
    let pci_config = pcid_handle.config();

    let mut name = pci_config.func.name();
    name.push_str("_i2c-hidd");

    common::setup_logging(
        "input",
        "pci",
        &name,
        common::output_level(),
        common::file_level(),
    );
    log::info!("i2c-hidd starting for {}", pci_config.func.display());

    let bar0 = unsafe { pcid_handle.map_bar(0, MemoryType::Uncacheable) }
        .ptr
        .as_ptr()
        .expose_provenance() as *mut u32;

    let _irq_file = pci_allocate_interrupt_vector(&mut pcid_handle, "i2c-hidd");

    let mut bus = match Dwd::new(bar0) {
        Ok(b) => DwdBus(b),
        Err(e) => {
            log::error!("i2c-hidd: dwd init: {e}");
            std::process::exit(1);
        }
    };

    // I2C addresses comuns de touchpad i2c-hid (PixArt/Elan). Poderiamos vir
    // da tabela DSDT/ACPI fora do pcid; aqui sondamos os conhecidos.
    let candidates = [0x15u8, 0x10, 0x20, 0x2C];
    let mut found: Option<(u8, i2c_hid::HidDescriptor)> = None;
    for addr in candidates {
        let mut probe = I2cHidDevice::new(bus, addr);
        match probe.fetch_descriptor() {
            Ok(desc) => {
                log::info!(
                    "i2c-hid found at 0x{addr:02x}: vid={:#x} pid={:#x}",
                    desc.vendor_id, desc.product_id
                );
                found = Some((addr, desc));
                bus = probe.bus;
                break;
            }
            Err(_) => {
                bus = probe.bus;
            }
        }
    }

    let Some((addr, desc)) = found else {
        log::error!("i2c-hidd: nenhum i2c-hid respondeu");
        std::process::exit(2);
    };
    let _ = desc;

    log::info!("i2c-hidd: device at {addr:#x}; power on + idle off");
    let mut dev = I2cHidDevice::new(bus, addr);
    dev.power_on().expect("power on");
    dev.set_idle(0).expect("set idle");

    let mut input = ProducerHandle::new().expect("i2c-hidd: inputd connection");

    log::info!("i2c-hidd ready (polling 10ms)");
    let mut buf = [0u8; 32];
    loop {
        if let Ok(()) = dev.read_input(&mut buf) {
            if let Some(tp) = parse_contact_block(&buf) {
                if tp.contact {
                    let _ = input.write_event(orbclient::MouseEvent {
                        x: tp.x as i32,
                        y: tp.y as i32,
                    }
                    .to_event());
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

// Testes MP: =hook de tela vazio; o codec fica na crate i2c-hid.
#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_sanity() {
        assert!(true);
    }
}
