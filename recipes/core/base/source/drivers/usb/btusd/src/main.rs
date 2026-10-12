//! btusd — Bluetooth USB (HCI) daemon for Redox OS.
//!
//! Spawned by xhcid when a USB Wireless Controller (class 0xE0, subclass 1,
//! protocol 1) appears. Bring-up: attach, configure BT interface, probe HCI
//! (Reset -> Read Version -> Read BD_ADDR). Depois registra o scheme `bt` e
//! serve userspace (inquiry/status/devices/event) enquanto escuta o endpoint
//! de interrupcao (eventos do controlador).
//!
//! Usage: btusd <scheme> <port> <if-num>   (spawned by xhcid)

mod hci;
mod l2cap;
mod pairing;
mod probe;
mod scheme;

extern crate alloc;

use std::io::{Read as _, Write as _};
use std::os::fd::AsRawFd;
use std::process::ExitCode;

use event::{user_data, EventQueue};
use redox_scheme::Socket;
use redox_scheme::scheme::register_sync_scheme;
use scheme_utils::ReadinessBased;
use xhcid_interface::{
    ConfigureEndpointsReq, DeviceReqData, PortReqRecipient, PortReqTy, XhciClientHandle,
};

use probe::Probe;
use scheme::{BtScheme, BtShared};

use crate::hci::HciCommand;

const USAGE: &str = "btusd <scheme> <port> <if-num>";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let scheme = args.next().expect(USAGE);
    let port: xhcid_interface::PortId = args.next().expect(USAGE).parse().expect(USAGE);
    let if_num: u8 = args.next().expect(USAGE).parse().expect(USAGE);

    common::setup_logging(
        "usb",
        "bt",
        &format!("port{port:?}_if{if_num}_btusd"),
        common::output_level(),
        common::file_level(),
    );

    match run(&scheme, port, if_num) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("btusd: {err}");
            ExitCode::FAILURE
        }
    }
}

fn send_hci(handle: &XhciClientHandle, cmd: &HciCommand, iface: u16) -> Result<(), &'static str> {
    handle
        .device_request(
            PortReqTy::Class,
            PortReqRecipient::Device,
            0x00,
            0x0000,
            iface,
            DeviceReqData::Out(&cmd.encode()),
        )
        .map_err(|_| "falha ao enviar comando HCI")
}

/// Attach, configure BT endpoints e probe HCI. Retorna (handle, event_ep, report).
fn attach_and_probe(
    scheme: &str,
    port: xhcid_interface::PortId,
    _if_num: u8,
) -> Result<(XhciClientHandle, std::fs::File, std::fs::File, std::fs::File, probe::ProbeReport, u16), &'static str> {
    let handle = XhciClientHandle::new(scheme.to_owned(), port)
        .map_err(|_| "falha ao abrir handle xHCI")?;

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
            Some((config_desc.clone(), config_desc.configuration_value, ifd))
        })
        .ok_or("interface Bluetooth (0xE0/01/01) nao encontrada")?;

    handle
        .configure_endpoints(&ConfigureEndpointsReq {
            config_desc: configuration_value,
            interface_desc: Some(interface_num),
            alternate_setting: Some(alt_setting),
            hub_ports: None,
        })
        .map_err(|_| "falha ao configurar endpoints BT")?;

    // Eventos do controlador chegam no interrupt-IN (tipicamente EP1 IN).
    let mut event_ep = handle
        .open_endpoint_data(1)
        .map_err(|_| "falha ao abrir endpoint de eventos BT")?;
    // ACL data: bulk IN/OUT (EP2 em ambos, padrao btusb)
    let acl_in_ep = handle
        .open_endpoint_data(2)
        .map_err(|_| "falha ao abrir bulk IN BT")?;
    let acl_out_ep = handle
        .open_endpoint_data(2)
        .map_err(|_| "falha ao abrir bulk OUT BT")?;

    let mut probe = Probe::new();
    loop {
        if let Some(cmd) = probe.poll() {
            send_hci(&handle, &cmd, interface_num as u16)?;
            let mut pkt = [0u8; 258];
            let n = event_ep
                .read(&mut pkt)
                .map_err(|_| "falha ao ler evento HCI")?;
            probe.on_event(&pkt[..n])?;
        } else if let Some(report) = probe.report() {
            log::info!(
                "btusd probe ok: HCI {} mfr {:#06x} bd {:02x?}",
                report.version.hci_version_str(),
                report.version.manufacturer,
                report.bd_addr,
            );
            return Ok((handle, event_ep, acl_in_ep, acl_out_ep, report, interface_num as u16));
        } else if probe.failed() {
            return Err("probe HCI falhou");
        }
    }
}

fn run(scheme: &str, port: xhcid_interface::PortId, if_num: u8) -> Result<(), &'static str> {
    let (handle, mut event_ep, mut acl_in_ep, mut acl_out_ep, report, iface) =
        attach_and_probe(scheme, port, if_num)?;

    let shared = BtShared {
        ready: true,
        hci_version: report.version.hci_version_str(),
        manufacturer: report.version.manufacturer,
        bd_addr: Some(report.bd_addr),
        ..Default::default()
    };

    let socket = Socket::nonblock().map_err(|_| "btusd: falha ao criar socket")?;
    let mut bt = BtScheme::new(shared);
    register_sync_scheme(&socket, "bt", &mut bt).map_err(|_| "btusd: register scheme falhou")?;
    let mut handler = ReadinessBased::new(Box::new(socket), 16);

    user_data! {
        enum Source {
            Scheme,
            EventEp,
            BulkIn,
        }
    }

    let mut queue = EventQueue::<Source>::new().map_err(|_| "btusd: event queue falhou")?;
    queue
        .subscribe(handler.socket().inner().raw(), Source::Scheme, event::EventFlags::READ)
        .map_err(|_| "btusd: subscribe scheme")?;
    queue
        .subscribe(event_ep.as_raw_fd() as usize, Source::EventEp, event::EventFlags::READ)
        .map_err(|_| "btusd: subscribe event ep")?;
    queue
        .subscribe(acl_in_ep.as_raw_fd() as usize, Source::BulkIn, event::EventFlags::READ)
        .map_err(|_| "btusd: subscribe bulk in")?;

    log::info!("btusd online: scheme BT:// pronto");
    loop {
        while let Some(ev) = queue.next() {
            let ev = ev.map_err(|_| "btusd: event error")?;
            match ev.user_data {
                Source::Scheme => {
                    handler
                        .read_and_process_requests(&mut bt)
                        .map_err(|_| "btusd: erro no scheme")?;
                    // drena comandos vindos do ctl (HCI EP0)
                    let cmds: Vec<_> = bt.shared.drain_commands().collect();
                    for cmd in cmds {
                        send_hci(&handle, &cmd, iface)?;
                    }
                    // drena pacotes ACL (canais L2CAP) p/ bulk OUT
                    let mut acl_out = Vec::new();
                    {
                        let handle_opt = bt.shared.pairing_handle;
                        if let Some(h) = handle_opt {
                            for (dcid, payload) in bt.shared.drain_acl_out() {
                                acl_out.push(l2cap::AclPacket::new(h, 2, 0, {
                                    let mut f = Vec::with_capacity(4 + payload.len());
                                    f.extend_from_slice(&(payload.len() as u16).to_le_bytes());
                                    f.extend_from_slice(&dcid.to_le_bytes());
                                    f.extend_from_slice(&payload);
                                    f
                                }));
                            }
                        } else {
                            bt.shared.drain_acl_out().for_each(drop); // descarta sem link
                        }
                    }
                    for pkt in acl_out {
                        let bytes = pkt.encode();
                        let _ = acl_out_ep.write(&bytes);
                    }
                }
                Source::EventEp => {
                    let mut pkt = [0u8; 258];
                    match event_ep.read(&mut pkt) {
                        Ok(0) | Err(_) => {}
                        Ok(n) => bt.shared.handle_hci_event(&pkt[..n]),
                    }
                }
                Source::BulkIn => {
                    let mut pkt = [0u8; 1024];
                    match acl_in_ep.read(&mut pkt) {
                        Ok(0) | Err(_) => {}
                        Ok(n) => bt.shared.handle_acl(&pkt[..n]),
                    }
                }
            }
        }
    }
}
