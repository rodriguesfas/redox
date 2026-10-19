use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::time::Duration;
use std::{env, process, time};

use dhcp::Dhcp;
use networkd::{Client, Decision, Mode, is_interface_name};

mod dhcp;
mod lease;

use crate::lease::{Lease, Stage, Timers};

/// The interface the DHCP client configures when nobody names one.
const DEFAULT_IFACE: &str = "eth0";

/// How long to wait for an answer to a DHCP message.
const RESPONSE_TIMEOUT_SECS: u64 = 10;

/// How long to wait before the first retry when nothing answered.
const RETRY_SECS: u64 = 30;

/// The longest pause between retries, so a server that is gone for a while is
/// still asked again before the lease runs out.
const MAX_RETRY_SECS: u64 = 300;

/// Option 53: the server refused the address (NAK).
const MESSAGE_TYPE_NAK: u8 = 6;

/// Whether the DHCP client runs for an interface.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    /// The interface is left to DHCP: do the exchange.
    Start,
    /// The configuration addresses the interface itself: a lease written over
    /// it would be a change nobody asked for.
    Skip { source: Option<PathBuf> },
    /// The daemon refused to decide, and a refusal is not swallowed.
    Refused(String),
}

/// What the network daemon decided about an interface.
///
/// The decision belongs to the daemon, which is where the configuration is
/// read. The answer has to be about the interface that was asked about, and a
/// refusal has to reach whoever asked: a client that decided for a daemon that
/// could not answer would write a lease nobody asked for.
fn verdict(iface: &str, asked: Option<io::Result<Decision>>) -> Verdict {
    let Some(answer) = asked else {
        return Verdict::Start;
    };

    let decision = match answer {
        Ok(decision) => decision,
        Err(err) => return Verdict::Refused(format!("cannot ask the network daemon: {err}")),
    };

    if !decision.about(iface) {
        return Verdict::Refused(format!(
            "the network daemon answered about {}",
            decision.iface()
        ));
    }

    match decision.mode() {
        Mode::Dhcp => Verdict::Start,
        Mode::Static => Verdict::Skip {
            source: decision.source().map(PathBuf::from),
        },
    }
}

/// Asks the network daemon what the configuration decides for an interface.
fn decide(iface: &str) -> Verdict {
    verdict(iface, Client::from_env().dhcp_if_mounted(iface))
}

/// How the DHCP client was asked to run.
///
/// The third value is `--once`: do the exchange and leave, instead of keeping
/// the lease alive.
fn parse_args(args: impl Iterator<Item = String>) -> Result<(String, bool, bool), String> {
    let mut iface: Option<String> = None;
    let mut verbose = false;
    let mut once = false;

    for arg in args {
        if arg == "-v" || arg == "--verbose" {
            verbose = true;
            continue;
        }
        if arg == "--once" {
            once = true;
            continue;
        }
        if arg.starts_with('-') {
            return Err(format!("unknown option {arg:?}"));
        }
        if iface.is_some() {
            return Err(format!("unexpected argument {arg:?}"));
        }
        if !is_interface_name(&arg) {
            return Err(format!("not an interface name: {arg:?}"));
        }
        iface = Some(arg);
    }

    Ok((
        iface.unwrap_or_else(|| DEFAULT_IFACE.to_owned()),
        verbose,
        once,
    ))
}

macro_rules! try_fmt {
    ($e:expr, $m:expr) => {
        match $e {
            Ok(ok) => ok,
            Err(err) => return Err(format!("{}: {}", $m, err)),
        }
    };
}

fn get_cfg_value(path: &str) -> Result<String, String> {
    let path = format!("/scheme/netcfg/{path}");
    let mut file = File::open(&path).map_err(|_| format!("Can't open {path}"))?;
    let mut result = String::new();
    file.read_to_string(&mut result)
        .map_err(|_| format!("Can't read {path}"))?;
    Ok(result)
}

fn get_iface_cfg_value(iface: &str, cfg: &str) -> Result<String, String> {
    let path = format!("ifaces/{iface}/{cfg}");
    get_cfg_value(&path)
}

fn set_cfg_value(path: &str, value: &str) -> Result<(), String> {
    let path = format!("/scheme/netcfg/{path}");
    let mut file = OpenOptions::new()
        .read(false)
        .write(true)
        .create(false)
        .open(&path)
        .map_err(|_| format!("Can't open {path}"))?;
    file.write(value.as_bytes())
        .map(|_| ())
        .map_err(|_| format!("Can't write {value} to {path}"))?;
    file.sync_data()
        .map_err(|_| format!("Can't commit {value} to {path}"))
}

fn set_iface_cfg_value(iface: &str, cfg: &str, value: &str) -> Result<(), String> {
    let path = format!("ifaces/{iface}/{cfg}");
    set_cfg_value(&path, value)
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
struct MacAddr {
    bytes: [u8; 6],
}

impl MacAddr {
    fn from_str(string: &str) -> Self {
        MacAddr::try_parse_with_delimeter(string, ':')
            .or_else(|| MacAddr::try_parse_with_delimeter(string, '-'))
            .unwrap_or_default()
    }

    fn try_parse_with_delimeter(string: &str, delimeter: char) -> Option<MacAddr> {
        let mut addr = MacAddr::default();
        let mut segments = 0;

        for part in string.split(delimeter) {
            if segments >= addr.bytes.len() {
                return None;
            }
            addr.bytes[segments] = match u8::from_str_radix(part, 16) {
                Ok(b) => b,
                _ => return None,
            };
            segments += 1;
        }

        if segments == addr.bytes.len() {
            Some(addr)
        } else {
            None
        }
    }

    fn to_string(&self) -> String {
        format!(
            "{:>02X}-{:>02X}-{:>02X}-{:>02X}-{:>02X}-{:>02X}",
            self.bytes[0],
            self.bytes[1],
            self.bytes[2],
            self.bytes[3],
            self.bytes[4],
            self.bytes[5]
        )
    }
}

/// Does the DISCOVER/OFFER/REQUEST/ACK exchange and applies the lease.
///
/// Returns the lease to keep alive, or `None` when the server did not say how
/// long the address is valid: without option 51 there is nothing to renew.
fn dhcp(iface: &str, verbose: bool) -> Result<Option<Lease>, String> {
    let current_mac = MacAddr::from_str(get_iface_cfg_value(iface, "mac")?.trim());

    let current_ip = get_iface_cfg_value(iface, "addr/list")?
        .lines()
        .next()
        .map(|l| l.to_owned())
        .unwrap_or("0.0.0.0".to_string());

    if verbose {
        println!(
            "DHCP: MAC: {} Current IP: {}",
            current_mac.to_string(),
            current_ip.trim()
        );
    }

    let tid = try_fmt!(
        time::SystemTime::now().duration_since(time::UNIX_EPOCH),
        "failed to get time"
    )
    .subsec_nanos();

    let socket = try_fmt!(UdpSocket::bind(("0.0.0.0", 68)), "failed to bind udp");
    try_fmt!(
        socket.connect(SocketAddr::from(([255, 255, 255, 255], 67))),
        "failed to connect udp"
    );
    try_fmt!(
        socket.set_read_timeout(Some(Duration::from_secs(RESPONSE_TIMEOUT_SECS))),
        "failed to set read timeout"
    );
    try_fmt!(
        socket.set_write_timeout(Some(Duration::from_secs(RESPONSE_TIMEOUT_SECS))),
        "failed to set write timeout"
    );

    {
        let mut discover = Dhcp {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            tid,
            secs: 0,
            flags: 0x8000u16.to_be(),
            ciaddr: [0, 0, 0, 0],
            yiaddr: [0, 0, 0, 0],
            siaddr: [0, 0, 0, 0],
            giaddr: [0, 0, 0, 0],
            chaddr: [
                current_mac.bytes[0],
                current_mac.bytes[1],
                current_mac.bytes[2],
                current_mac.bytes[3],
                current_mac.bytes[4],
                current_mac.bytes[5],
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
            ],
            sname: [0; 64],
            file: [0; 128],
            magic: 0x63825363u32.to_be(),
            options: [0; 308],
        };

        for (s, d) in [
            // DHCP Message Type (Discover)
            53, 1, 1, // End
            255,
        ]
        .iter()
        .zip(discover.options.iter_mut())
        {
            *d = *s;
        }

        let discover_data = unsafe {
            std::slice::from_raw_parts((&discover as *const Dhcp) as *const u8, size_of::<Dhcp>())
        };

        let _sent = try_fmt!(socket.send(discover_data), "failed to send discover");

        if verbose {
            println!("DHCP: Sent Discover");
        }
    }

    let mut offer_data = [0; 65536];
    try_fmt!(socket.recv(&mut offer_data), "failed to receive offer");
    let offer = unsafe { &*(offer_data.as_ptr() as *const Dhcp) };
    if verbose {
        println!(
            "DHCP: Offer IP: {:?}, Server IP: {:?}",
            offer.yiaddr, offer.siaddr
        );
    }

    let offer_options = dhcp::options::parse(&offer.options);
    if verbose {
        println!("DHCP: Subnet Mask: {:?}", offer_options.subnet);
        println!("DHCP: Router: {:?}", offer_options.router);
        println!("DHCP: Domain Name Server: {:?}", offer_options.dns);
        println!("DHCP: Lease Time: {:?}", offer_options.lease);
        println!("DHCP: Message Type: {:?}", offer_options.message_type);
        println!("DHCP: Server ID: {:?}", offer_options.server_id);
    }

    {
        let mask_len = if let Some(subnet) = offer_options.subnet {
            let mut subnet: u32 = (subnet[0] as u32) << 24
                | (subnet[1] as u32) << 16
                | (subnet[2] as u32) << 8
                | subnet[3] as u32;
            subnet = !subnet;
            subnet.leading_zeros()
        } else {
            println!("DHCP: No subnet mask");
            0
        };

        let new_ips = format!(
            "{}.{}.{}.{}/{}\n",
            offer.yiaddr[0], offer.yiaddr[1], offer.yiaddr[2], offer.yiaddr[3], mask_len
        );
        try_fmt!(
            set_iface_cfg_value(iface, "addr/set", &new_ips),
            "failed to set ip"
        );

        if verbose {
            let new_ip = try_fmt!(get_iface_cfg_value(iface, "addr/list"), "failed to get ip");
            println!("DHCP: New IP: {}", new_ip.trim());
        }

        if let Some(router) = offer_options.router {
            let default_route = format!(
                "default via {}.{}.{}.{}",
                router[0], router[1], router[2], router[3]
            );

            try_fmt!(
                set_cfg_value("route/add", &default_route),
                "failed to set default route"
            );

            if verbose {
                let new_router = try_fmt!(get_cfg_value("route/list"), "failed to get ip router");
                println!("DHCP: New Router: {}", new_router.trim());
            }
        } else {
            println!("DHCP: No router");
        }

        if let Some(mut dns) = offer_options.dns {
            if dns[0] == 127 {
                let quad9 = [9, 9, 9, 9];
                if verbose {
                    println!(
                        "DHCP: Received sarcastic DNS suggestion {}.{}.{}.{}, using {}.{}.{}.{} instead",
                        dns[0], dns[1], dns[2], dns[3], quad9[0], quad9[1], quad9[2], quad9[3]
                    );
                }
                dns = quad9;
            }

            let nameserver = format!("{}.{}.{}.{}", dns[0], dns[1], dns[2], dns[3]);

            try_fmt!(
                set_cfg_value("resolv/nameserver", &nameserver),
                "failed to set name server"
            );

            if verbose {
                let new_dns = try_fmt!(get_cfg_value("resolv/nameserver"), "failed to get dns");
                println!("DHCP: New DNS: {}", new_dns.trim());
            }
        } else {
            println!("DHCP: No DNS resolver");
        }
    }

    {
        let mut request = Dhcp {
            op: 1,
            htype: 1,
            hlen: 6,
            hops: 0,
            tid,
            secs: 0,
            flags: 0,
            ciaddr: [0; 4],
            yiaddr: [0; 4],
            siaddr: [0; 4],
            giaddr: [0; 4],
            chaddr: [
                current_mac.bytes[0],
                current_mac.bytes[1],
                current_mac.bytes[2],
                current_mac.bytes[3],
                current_mac.bytes[4],
                current_mac.bytes[5],
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
                0x00,
            ],
            sname: [0; 64],
            file: [0; 128],
            magic: 0x63825363u32.to_be(),
            options: [0; 308],
        };

        // If the server_id_option was None, use "0.0.0.0"
        let server_id = offer_options.server_id.unwrap_or([0, 0, 0, 0]);

        for (s, d) in [
            // DHCP Message Type (Request)
            53,
            1,
            3,
            // Requested IP Address
            50,
            4,
            offer.yiaddr[0],
            offer.yiaddr[1],
            offer.yiaddr[2],
            offer.yiaddr[3],
            // Server Identifier - use Option 54 from the Offer
            54,
            4,
            server_id[0],
            server_id[1],
            server_id[2],
            server_id[3],
            // End
            255,
        ]
        .iter()
        .zip(request.options.iter_mut())
        {
            *d = *s;
        }

        let request_data = unsafe {
            std::slice::from_raw_parts((&request as *const Dhcp) as *const u8, size_of::<Dhcp>())
        };

        let _sent = try_fmt!(socket.send(request_data), "failed to send request");

        if verbose {
            println!("DHCP: Sent Request");
        }
    }

    let mut ack_data = [0; 65536];
    try_fmt!(socket.recv(&mut ack_data), "failed to receive ack");
    let ack = unsafe { &*(ack_data.as_ptr() as *const Dhcp) };
    let ack_options = dhcp::options::parse(&ack.options);

    if verbose {
        println!(
            "DHCP: Ack IP: {:?}, Server IP: {:?}",
            ack.yiaddr, ack.siaddr
        );
    }

    // The lease is the one the ACK grants: the OFFER is only a proposal. The
    // OFFER stays as the fallback for a server that says its times only once.
    if ack_options.message_type == Some(MESSAGE_TYPE_NAK) {
        return Err("the server refused the address (NAK)".to_owned());
    }

    let timers = Timers::from_options(
        ack_options.lease.or(offer_options.lease),
        ack_options.t1.or(offer_options.t1),
        ack_options.t2.or(offer_options.t2),
    );

    Ok(timers.map(|timers| Lease {
        server: ack_options.server_id.or(offer_options.server_id),
        timers,
        addr: offer.yiaddr,
    }))
}

/// The request that renews a lease.
///
/// The address is already ours, so it goes in `ciaddr`, and options 50
/// (requested address) and 54 (server) stay out: this is not a request for a
/// new address, it is the same address asking to stay (RFC 2131 §4.3.2). When
/// rebinding the request becomes a broadcast, because whoever granted it did
/// not answer.
fn renewal_packet(mac: &MacAddr, tid: u32, addr: [u8; 4], broadcast: bool) -> Dhcp {
    let mut chaddr = [0; 16];
    chaddr[..6].copy_from_slice(&mac.bytes);

    let mut request = Dhcp {
        op: 1,
        htype: 1,
        hlen: 6,
        hops: 0,
        tid,
        secs: 0,
        flags: if broadcast { 0x8000u16.to_be() } else { 0 },
        ciaddr: addr,
        yiaddr: [0; 4],
        siaddr: [0; 4],
        giaddr: [0; 4],
        chaddr,
        sname: [0; 64],
        file: [0; 128],
        magic: 0x63825363u32.to_be(),
        options: [0; 308],
    };

    // DHCP Message Type (Request), End.
    for (source, destination) in [53, 1, 3, 255].iter().zip(request.options.iter_mut()) {
        *destination = *source;
    }

    request
}

/// Asks the server to extend the lease (or anyone, when rebinding).
fn renew(iface: &str, verbose: bool, current: &Lease, broadcast: bool) -> Result<Lease, String> {
    let mac = MacAddr::from_str(get_iface_cfg_value(iface, "mac")?.trim());

    let tid = try_fmt!(
        time::SystemTime::now().duration_since(time::UNIX_EPOCH),
        "failed to get time"
    )
    .subsec_nanos();

    let socket = try_fmt!(UdpSocket::bind(("0.0.0.0", 68)), "failed to bind udp");
    let destination = match (broadcast, current.server) {
        (false, Some(server)) => SocketAddr::from((server, 67)),
        // Not knowing who granted the lease leaves no unicast to aim at.
        _ => SocketAddr::from(([255, 255, 255, 255], 67)),
    };
    try_fmt!(socket.connect(destination), "failed to connect udp");
    try_fmt!(
        socket.set_read_timeout(Some(Duration::from_secs(RESPONSE_TIMEOUT_SECS))),
        "failed to set read timeout"
    );
    try_fmt!(
        socket.set_write_timeout(Some(Duration::from_secs(RESPONSE_TIMEOUT_SECS))),
        "failed to set write timeout"
    );

    let request = renewal_packet(&mac, tid, current.addr, broadcast);
    let request_data = unsafe {
        std::slice::from_raw_parts((&request as *const Dhcp) as *const u8, size_of::<Dhcp>())
    };
    try_fmt!(socket.send(request_data), "failed to send renewal");

    if verbose {
        println!(
            "DHCP: Sent {} Request",
            if broadcast { "Rebind" } else { "Renew" }
        );
    }

    let mut ack_data = [0; 65536];
    try_fmt!(socket.recv(&mut ack_data), "failed to receive renewal ack");
    let ack = unsafe { &*(ack_data.as_ptr() as *const Dhcp) };
    let ack_options = dhcp::options::parse(&ack.options);

    if verbose {
        println!(
            "DHCP: Renewal Ack IP: {:?}, Server IP: {:?}",
            ack.yiaddr, ack.siaddr
        );
    }

    if ack_options.message_type == Some(MESSAGE_TYPE_NAK) {
        return Err("the server refused the renewal (NAK)".to_owned());
    }

    // A different address cannot be half applied: the DISCOVER path is the one
    // that configures the interface, so the renewal keeps the old address and
    // the lease runs its course.
    if ack.yiaddr != [0; 4] && ack.yiaddr != current.addr {
        return Err(format!(
            "the server granted a different address ({:?}) on renewal",
            ack.yiaddr
        ));
    }

    Ok(Lease {
        server: ack_options.server_id.or(current.server),
        timers: current
            .timers
            .renewed(ack_options.lease, ack_options.t1, ack_options.t2),
        addr: current.addr,
    })
}

/// Keeps the lease alive: renews at T1, rebinds at T2 and, when it expires,
/// does the DISCOVER again.
fn watch(iface: &str, verbose: bool, first: Lease) {
    let mut current = first;
    let mut started = time::Instant::now();
    let mut backoff = Duration::from_secs(RETRY_SECS);

    loop {
        match current.timers.at(started.elapsed()) {
            Stage::Waiting => std::thread::sleep(current.timers.wait(started.elapsed())),
            Stage::Renew | Stage::Rebind => {
                let rebinding = current.timers.at(started.elapsed()) == Stage::Rebind;
                match renew(iface, verbose, &current, rebinding) {
                    Ok(renewed) => {
                        if verbose {
                            println!(
                                "DHCP: Lease {} until {:?}",
                                if rebinding { "rebound" } else { "renewed" },
                                renewed.timers.lease
                            );
                        }
                        current = renewed;
                        started = time::Instant::now();
                        backoff = Duration::from_secs(RETRY_SECS);
                    }
                    Err(err) => {
                        let stage = if rebinding { "rebind" } else { "renew" };
                        eprintln!("dhcpd: {iface}: {stage} failed: {err}");
                        // Whoever granted it may answer again in a moment, so
                        // retry with a growing pause until the stage moves on.
                        let wait = current.timers.wait(started.elapsed());
                        std::thread::sleep(wait.min(backoff));
                        backoff = (backoff * 2).min(Duration::from_secs(MAX_RETRY_SECS));
                    }
                }
            }
            Stage::Expired => {
                eprintln!("dhcpd: {iface}: lease expired, starting over");
                match dhcp(iface, verbose) {
                    Ok(Some(lease)) => {
                        current = lease;
                        started = time::Instant::now();
                        backoff = Duration::from_secs(RETRY_SECS);
                    }
                    // No lease time: nothing to keep alive.
                    Ok(None) => return,
                    Err(err) => {
                        eprintln!("dhcpd: {iface}: {err}");
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(Duration::from_secs(MAX_RETRY_SECS));
                    }
                }
            }
        }
    }
}

fn main() {
    let (iface, verbose, once) = match parse_args(env::args().skip(1)) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("dhcpd: {err}");
            eprintln!("usage: dhcpd [INTERFACE] [-v] [--once]");
            process::exit(2);
        }
    };

    // Who decides whether DHCP runs here is the network daemon, which is where
    // the configuration is read. An interface the configuration addresses
    // itself gets no lease, and a daemon that could not be asked is not a
    // daemon that said yes.
    match decide(&iface) {
        Verdict::Start => {}
        Verdict::Skip { source } => {
            match source {
                Some(source) => eprintln!(
                    "dhcpd: {iface} is configured statically in {}, not starting",
                    source.display()
                ),
                None => eprintln!("dhcpd: {iface} is configured statically, not starting"),
            }
            return;
        }
        Verdict::Refused(err) => {
            eprintln!("dhcpd: {iface}: {err}");
            process::exit(1);
        }
    }

    match dhcp(&iface, verbose) {
        // A server that did not say until when the address is valid leaves
        // nothing to renew: the exchange is all there is (the old behaviour,
        // and what `--once` asks for).
        Ok(Some(lease)) if !once => watch(&iface, verbose, lease),
        Ok(_) => {}
        Err(err) => {
            eprintln!("dhcpd: {err}");
            process::exit(1);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn decision(iface: &str, mode: Mode, source: Option<&str>) -> Decision {
        Decision {
            iface: iface.to_owned(),
            source: source.map(PathBuf::from),
            mode,
        }
    }

    fn parse(args: &[&str]) -> Result<(String, bool, bool), String> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn the_interface_comes_from_the_arguments_or_is_eth0() {
        assert_eq!(parse(&[]).unwrap(), ("eth0".to_owned(), false, false));
        assert_eq!(
            parse(&["wlan0"]).unwrap(),
            ("wlan0".to_owned(), false, false)
        );
        assert_eq!(parse(&["-v"]).unwrap(), ("eth0".to_owned(), true, false));
        assert_eq!(
            parse(&["--verbose", "eth1"]).unwrap(),
            ("eth1".to_owned(), true, false)
        );
        assert_eq!(
            parse(&["eth1", "-v"]).unwrap(),
            ("eth1".to_owned(), true, false)
        );
    }

    #[test]
    fn once_is_asked_for_on_the_command_line() {
        assert_eq!(
            parse(&["--once"]).unwrap(),
            ("eth0".to_owned(), false, true)
        );
        assert_eq!(
            parse(&["wlan0", "-v", "--once"]).unwrap(),
            ("wlan0".to_owned(), true, true)
        );
    }

    #[test]
    fn a_renewal_carries_the_address_ourselves_and_asks_for_no_new_one() {
        let mac = MacAddr {
            bytes: [0x01, 0x23, 0x45, 0x67, 0x89, 0xab],
        };
        let request = renewal_packet(&mac, 7, [10, 0, 0, 42], false);

        // `Dhcp` is packed: reading a field through a reference is unaligned.
        let (tid, flags) = (request.tid, request.flags);
        assert_eq!(request.op, 1);
        assert_eq!(request.hlen, 6);
        assert_eq!(tid, 7);
        assert_eq!(request.ciaddr, [10, 0, 0, 42]);
        assert_eq!(request.yiaddr, [0; 4]);
        assert_eq!(flags, 0, "renewing is a unicast, not a broadcast");
        assert_eq!(&request.chaddr[..6], &mac.bytes);
        assert_eq!(&request.chaddr[6..], &[0; 10]);
        // Message Type (Request), End: no requested address, no server id.
        assert_eq!(&request.options[..4], &[53, 1, 3, 255]);
        assert_eq!(&request.options[4..], &[0; 304][..]);

        // Rebinding goes out as a broadcast.
        let request = renewal_packet(&mac, 7, [10, 0, 0, 42], true);
        let flags = request.flags;
        assert_eq!(flags, 0x8000u16.to_be());
    }

    #[test]
    fn an_argument_that_is_not_an_interface_is_refused() {
        for args in [
            vec!["--nope"],
            vec!["eth0", "wlan0"],
            vec![".."],
            vec!["."],
            vec!["a/b"],
            vec![""],
        ] {
            assert!(parse(&args).is_err(), "{args:?} was accepted");
        }
    }

    #[test]
    fn the_network_daemon_decides_whether_dhcp_runs() {
        // A daemon that leaves the interface to DHCP, or no daemon at all: the
        // client does what it did before there was one.
        assert_eq!(verdict("eth0", None), Verdict::Start);
        assert_eq!(
            verdict("eth0", Some(Ok(decision("eth0", Mode::Dhcp, None)))),
            Verdict::Start
        );
        assert_eq!(
            verdict(
                "eth0",
                Some(Ok(decision(
                    "eth0",
                    Mode::Dhcp,
                    Some("/etc/networkd/10-eth0.network")
                )))
            ),
            Verdict::Start
        );

        // An interface the configuration addresses itself: a lease written
        // over it is a change nobody asked for, and the file that decided is
        // named.
        assert_eq!(
            verdict(
                "eth0",
                Some(Ok(decision(
                    "eth0",
                    Mode::Static,
                    Some("/etc/networkd/10-eth0.network")
                )))
            ),
            Verdict::Skip {
                source: Some(PathBuf::from("/etc/networkd/10-eth0.network"))
            }
        );

        // A refusal is a verdict, and it is not swallowed: the client does not
        // decide for a daemon that could not answer.
        let refused = verdict("eth0", Some(Err(io::Error::other("no response"))));
        assert!(
            matches!(refused, Verdict::Refused(ref err) if err.contains("no response")),
            "{refused:?}"
        );

        // An answer about another interface is about another interface.
        let refused = verdict("eth0", Some(Ok(decision("wlan0", Mode::Dhcp, None))));
        assert!(
            matches!(refused, Verdict::Refused(ref err) if err.contains("wlan0")),
            "{refused:?}"
        );
    }

    #[test]
    fn from_str_test() {
        let mac = MacAddr {
            bytes: [0x01, 0x23, 0x45, 0x67, 0x89, 0xab],
        };
        let empty_mac = MacAddr::default();

        assert_eq!(mac, MacAddr::from_str("01:23:45:67:89:ab"));
        assert_eq!(mac, MacAddr::from_str("1:23:45:67:89:ab"));
        assert_eq!(mac, MacAddr::from_str("01:23:45:67:89:AB"));
        assert_eq!(mac, MacAddr::from_str("01-23-45-67-89-ab"));
        assert_eq!(empty_mac, MacAddr::from_str(""));
        assert_eq!(empty_mac, MacAddr::from_str("01:23:45:67:89"));
        assert_eq!(empty_mac, MacAddr::from_str("01:23:45:67:89:ab:cd"));
        assert_eq!(empty_mac, MacAddr::from_str("x1:23:45:67:89:ab"));
        assert_eq!(empty_mac, MacAddr::from_str("01:23-45-67-89-ab"));
        assert_eq!(empty_mac, MacAddr::from_str("01-23-45-67-89-ag"));
        assert_eq!(empty_mac, MacAddr::from_str("01.23.45.67.89.ab"));
        assert_eq!(empty_mac, MacAddr::from_str("01234-23-45-67-89-ab"));
        assert_eq!(empty_mac, MacAddr::from_str("01--23-45-67-89-ab"));
        assert_eq!(empty_mac, MacAddr::from_str("12"));
        assert_eq!(empty_mac, MacAddr::from_str("0:0:0:0:0:0"));

        assert_eq!(mac, MacAddr::from_str(&mac.to_string()));
        assert_eq!(empty_mac, MacAddr::from_str(&empty_mac.to_string()));
    }
}
