//! btusd — Bluetooth USB (HCI) driver for Redox OS.
//!
//! Spawned by xhcid when a USB Wireless Controller (class 0xE0, subclass 1,
//! protocol 1) appears. Bring-up flow: attach to the port, configure the BT
//! interface endpoints, then run the HCI probe (Reset 5 Read Local Version 5
//! Read BD_ADDR). Events arrive on the interrupt-IN endpoint file.
//!
//! Usage: btusd <scheme> <port> <if-num>   (spawned by xhcid)
//!
//! TODO(epic): expose an HCI scheme for userspace (pairing, L2CAP etc.).

mod hci;
mod probe;

extern crate alloc;

use std::io::Read as _;
use std::process::ExitCode;

use xhcid_interface::{ConfigureEndpointsReq, DeviceReqData, PortReqRecipient, PortReqTy, XhciClientHandle};

use probe::{Probe, ProbeReport};

const USAGE: &str = "btusd <scheme> <port> <if-num>";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let scheme = args.next().expect(USAGE);
    let port: xhcid_interface::PortId = args.next().expect(USAGE).parse().expect(USAGE);
    let if_num: u8 = args.next().expect(USAGE).parse().expect(USAGE);

    common::setup_logging(
        "usb",
        "bt",
        &format!("port{port}_if{if_num}_btusd"),
        common::output_level(),
        common::file_level(),
    );

    match run(&scheme, port, if_num) {
        Ok(report) => {
            log::info!(
                "btusd ready: HCI {} | mfr {:#06x} | BD_ADDR {:02x?}",
                report.version.hci_version_str(),
                report.version.manufacturer,
                report.bd_addr,
            );
            ExitCode::SUCCESS
        }
        Err(err) => {
            log::error!("btusd bring-up failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(scheme: &str, port: xhcid_interface::PortId, _if_num: u8) -> Result<ProbeReport, &'static str> {
    let handle = XhciClientHandle::new(scheme.to_owned(), port)
        .map_err(|_| "falha ao abrir handle xHCI")?;

    // 1) Acha a interface Bluetooth (class 0xE0, subclass 1, protocol 1)
    let desc = handle
        .get_standard_descs()
        .map_err(|_| "falha ao ler descritores USB")?;
    let (_conf_desc, configuration_value, (_if_desc, interface_num, alt_setting)) = desc
        .config_descs
        .iter()
        .find_map(|config_desc| {
            let ifd = config_desc.interface_descs.iter().find_map(|if_desc| {
                if if_desc.class == 0xE0 && if_desc.sub_class == 1 && if_desc.protocol == 1 {
                    Some((if_desc.clone(), if_desc.number, if_desc.alternate_setting))
                } else {
                    None
                }
            })?;
            Some((
                config_desc.clone(),
                config_desc.configuration_value,
                ifd,
            ))
        })
        .ok_or("interface Bluetooth (0xE0/01/01) nao encontrada")?;

    // 2) Configura os endpoints da interface BT
    handle
        .configure_endpoints(&xhcid_interface::ConfigureEndpointsReq {
            config_desc: configuration_value,
            interface_desc: Some(interface_num),
            alternate_setting: Some(alt_setting),
            hub_ports: None,
        })
        .map_err(|_| "falha ao configurar endpoints BT")?;

    // 3) Endpoint de eventos: primeiro interrupt-IN (tipicamente EP1 IN)
    let (_ep_descs, event_ep_num) = ((), 1u8); // TODO: ler do IfDescriptor (bt interrupt-in)
    let mut event_ep = handle
        .open_endpoint_data(event_ep_num)
        .map_err(|_| "falha ao abrir endpoint de eventos BT")?;

    // 4) Probe HCI via EP0 (classe, device, request 0)
    let mut probe = Probe::new();
    loop {
        if let Some(cmd) = probe.poll() {
            handle
                .device_request(
                    PortReqTy::Class,
                    PortReqRecipient::Device,
                    0x00,
                    0x0000,
                    interface_num as u16, // wIndex = interface (BT class req)
                    DeviceReqData::Out(&cmd.encode()),
                )
                .map_err(|_| "falha ao enviar comando HCI")?;

            let mut pkt = [0u8; 258];
            let n = event_ep
                .read(&mut pkt)
                .map_err(|_| "falha ao ler evento HCI")?;
            probe.on_event(&pkt[..n])?;
        } else if let Some(report) = probe.report() {
            return Ok(report);
        } else if probe.failed() {
            return Err("probe HCI falhou");
        }
    }
}

// Reexport para o log no main (e testes de integracao futura).
#[allow(dead_code)]
fn hci_command_reset() -> Vec<u8> {
    hci::HciCommand::new(hci::CMD_RESET).encode()
}
