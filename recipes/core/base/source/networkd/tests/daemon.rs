//! End-to-end test of the network daemon: what it applies at boot, what it does
//! when the configuration or the interfaces change under it, what a request over
//! the wire produces, and what `networkctl` sees afterwards.
//!
//! The scheme itself only runs on Redox; everything under it is tested here —
//! the service that applies a request, the watcher that keeps the stack in step,
//! the protocol as it travels on a descriptor — and so is the real `networkctl`
//! binary reading the stack the daemon wrote.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use networkd::client::exchange;
use networkd::netcfg::Netcfg;
use networkd::plan::Mode;
use networkd::protocol::{format_response, Decision, Outcome, ProtocolError, Request};
use networkd::service::Service;
use networkd::state::{shared, Networkd};
use networkd::watch::{Watcher, DEFAULT_INTERVAL};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("networkd-daemon-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// The control files a stack with one interface serves.
fn stack_of(name: &str, mounted: bool) -> (PathBuf, PathBuf) {
    let dir = scratch(name);
    let netcfg = dir.join(if mounted { "netcfg" } else { "absent" });

    if mounted {
        for path in [
            "ifaces/eth0/mac",
            "ifaces/eth0/addr/set",
            "route/add",
            "resolv/nameserver",
        ] {
            let file = netcfg.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, "").unwrap();
        }
    }

    (dir.clone(), netcfg)
}

/// A configuration directory holding one file.
fn configuration(dir: &Path, name: &str, text: &str) -> PathBuf {
    let config = dir.join("networkd");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join(name), text).unwrap();
    config
}

/// The service a daemon would run, over a configuration and a stack.
fn service_of(config: &Path, netcfg: &Path) -> Service {
    Service::new(shared(Networkd::new(
        config.to_path_buf(),
        Netcfg::new(netcfg.to_string_lossy().into_owned()),
    )))
}

/// Sends a request the way `networkctl` does, and reads the answer back.
///
/// The socket pair is the descriptor the scheme hands to a client: the request
/// is written to it, and the side that answers closes when it is done, which is
/// what ends the response — the same way a scheme answers with nothing left.
fn ask(service: &Service, request: Request) -> io::Result<String> {
    let (mut client_end, mut daemon_end) = UnixStream::pair().unwrap();
    let service = service.clone();
    let line = request.encode();
    let expected = line.clone();

    std::thread::spawn(move || {
        assert_eq!(read_line(&mut daemon_end), expected);
        let response = answer(&service, &line);
        daemon_end.write_all(response.as_bytes()).unwrap();
        daemon_end.flush().unwrap();
    });

    exchange(&mut client_end, &request.encode())
}

/// The answer a daemon gives to a line, which is what the scheme writes back.
fn answer(service: &Service, line: &str) -> String {
    match Request::parse(line) {
        Ok(request) => format_response(&service.handle(&request)),
        Err(err) => format_response(&Err::<String, ProtocolError>(err)),
    }
}

/// Reads one request line, the way the daemon reads it.
fn read_line(stream: &mut UnixStream) -> String {
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while stream.read(&mut byte).unwrap_or(0) == 1 {
        request.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    String::from_utf8(request).unwrap()
}

/// The real tool, with the paths named: a command that names them is about them,
/// and never about what a daemon manages.
fn networkctl(config: &Path, netcfg: &Path, scheme: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_networkctl"))
        .env_remove("NETWORKD_CONFIG")
        .env_remove("NETWORKD_NETCFG")
        .env("NETWORKD_SCHEME", scheme)
        .arg("--config-dir")
        .arg(config)
        .arg("--netcfg")
        .arg(netcfg)
        .args(args)
        .output()
        .expect("networkctl should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_daemon_applies_the_configuration_at_boot_and_keeps_it_in_step() {
    let (dir, netcfg) = stack_of("boot", true);
    let config = configuration(
        &dir,
        "10-eth0.network",
        "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\nGateway=192.168.1.1\n",
    );

    let service = service_of(&config, &netcfg);
    let mut watcher = Watcher::new(service.state().clone(), DEFAULT_INTERVAL);

    // Nothing is written until there is a configuration to apply.
    assert!(service.status_document().ends_with("applied: no\n"));

    // The first look is what the boot does: the address is written, and the
    // stack is not asked twice for the same one.
    let application = watcher.look().unwrap().expect("there is work to do");
    assert!(application.changed);
    assert_eq!(
        fs::read_to_string(netcfg.join("ifaces/eth0/addr/set")).unwrap(),
        "192.168.1.50/24\n"
    );
    assert_eq!(
        fs::read_to_string(netcfg.join("route/add")).unwrap(),
        "default via 192.168.1.1\n"
    );
    assert!(watcher.look().unwrap().is_none(), "already applied");
    assert!(service.status_document().ends_with("applied: yes\n"));

    // The tool sees the stack the daemon wrote.
    let status = networkctl(&config, &netcfg, &dir.join("no-daemon"), &["status"]);
    assert!(status.status.success(), "{}", stdout(&status));
    assert!(
        stdout(&status).contains("    address 192.168.1.50/24\n"),
        "{}",
        stdout(&status)
    );

    // The configuration changes, as an editor changes it: the next look is what
    // carries it into the stack.
    fs::write(
        config.join("10-eth0.network"),
        "[Match]\nName=eth0\n\n[Network]\nAddress=10.0.0.2/8\n",
    )
    .unwrap();
    assert!(watcher.look().unwrap().is_some());
    // A control file is a value, not a file that grows: on the host it is a
    // regular file, so the value at the start is what the stack reads.
    assert!(fs::read_to_string(netcfg.join("ifaces/eth0/addr/set"))
        .unwrap()
        .starts_with("10.0.0.2/8\n"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_request_applies_the_configuration_and_answers_with_the_stack_it_wrote() {
    let (dir, netcfg) = stack_of("request", true);
    let config = configuration(
        &dir,
        "10-eth0.network",
        "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\nDNS=9.9.9.9\n",
    );

    let service = service_of(&config, &netcfg);

    // A request travels the wire the way the scheme hands it over: a line in,
    // and a line back that says what happened to which stack.
    let message = ask(&service, Request::Apply).unwrap();
    let outcome = Outcome::parse(&message).unwrap();
    assert!(outcome.changed());
    assert!(outcome.describes(&netcfg, &config));
    assert_eq!(
        fs::read_to_string(netcfg.join("ifaces/eth0/addr/set")).unwrap(),
        "192.168.1.50/24\n"
    );
    assert_eq!(
        fs::read_to_string(netcfg.join("resolv/nameserver")).unwrap(),
        "9.9.9.9\n"
    );

    // Asking again is not a write: the stack already says this.
    let message = ask(&service, Request::Apply).unwrap();
    let outcome = Outcome::parse(&message).unwrap();
    assert!(!outcome.changed());
    assert!(outcome.describes(&netcfg, &config));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_interface_left_to_dhcp_has_nothing_to_write() {
    let (dir, netcfg) = stack_of("dhcp", true);
    let config = configuration(
        &dir,
        "10-eth0.network",
        "[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n",
    );

    let service = service_of(&config, &netcfg);

    // The address, the route and the resolver arrive with the lease: applying
    // the configuration is applying nothing, and the stack is left alone.
    let outcome = Outcome::parse(&ask(&service, Request::Apply).unwrap()).unwrap();
    assert!(!outcome.changed());
    assert!(outcome.describes(&netcfg, &config));
    assert_eq!(
        fs::read_to_string(netcfg.join("ifaces/eth0/addr/set")).unwrap(),
        ""
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_request_that_is_not_a_request_is_answered_with_an_error() {
    let (dir, netcfg) = stack_of("malformed", true);
    let config = configuration(&dir, "10-eth0.network", "[Match]\nName=eth0\n");
    let service = service_of(&config, &netcfg);

    let (mut client_end, mut daemon_end) = UnixStream::pair().unwrap();
    let service = service.clone();
    std::thread::spawn(move || {
        assert_eq!(read_line(&mut daemon_end), "flush\n");
        let response = answer(&service, "flush\n");
        daemon_end.write_all(response.as_bytes()).unwrap();
        daemon_end.flush().unwrap();
    });

    let error = exchange(&mut client_end, "flush\n").unwrap_err();
    assert!(error.to_string().contains("unknown request"), "{error}");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_stack_that_is_not_mounted_is_waited_for_and_then_refused_out_loud() {
    let (dir, netcfg) = stack_of("unmounted", false);
    let config = configuration(
        &dir,
        "10-eth0.network",
        "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n",
    );

    let service = service_of(&config, &netcfg);
    let mut watcher = Watcher::new(service.state().clone(), DEFAULT_INTERVAL);

    // The network comes up after this daemon: a stack that is not mounted is a
    // reason to look again, not a failure.
    assert!(watcher.look().unwrap().is_none());

    // But a request that asks for the write right now is told why it cannot
    // happen, in the words of the client.
    let error = ask(&service, Request::Apply).unwrap_err();
    assert!(
        error.to_string().contains("cannot read the interfaces"),
        "{error}"
    );

    // And the report says it too, instead of showing an empty network.
    let report = service.status_document();
    assert!(report.starts_with("error: "), "{report}");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn nothing_is_asked_of_a_daemon_when_the_paths_were_named() {
    let (dir, netcfg) = stack_of("named", true);
    let config = configuration(
        &dir,
        "10-eth0.network",
        "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n",
    );

    // A scheme that would answer, if the tool asked: a command that named the
    // configuration and the stack is about them, and the answer of a daemon for
    // the system is not the answer for them.
    let scheme = dir.join("scheme");
    fs::create_dir_all(&scheme).unwrap();
    fs::write(
        scheme.join("apply"),
        "ok applied\t9\t/scheme/netcfg\t/etc/networkd\n",
    )
    .unwrap();

    let applied = networkctl(&config, &netcfg, &scheme, &["apply"]);
    assert!(applied.status.success(), "{}", stdout(&applied));
    assert!(
        !stdout(&applied).contains("the network daemon"),
        "the daemon was asked: {}",
        stdout(&applied)
    );
    assert!(
        stdout(&applied).contains("write "),
        "the tool did not write: {}",
        stdout(&applied)
    );
    assert_eq!(
        fs::read_to_string(netcfg.join("ifaces/eth0/addr/set")).unwrap(),
        "192.168.1.50/24\n"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_dhcp_client_asks_the_daemon_who_decides_the_address() {
    let (dir, netcfg) = stack_of("decide", true);
    let config = configuration(
        &dir,
        "10-eth0.network",
        "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.50/24\n",
    );

    let service = service_of(&config, &netcfg);
    let ask_dhcp = |iface: &str| -> io::Result<Decision> {
        let message = ask(
            &service,
            Request::Dhcp {
                iface: iface.to_owned(),
            },
        )?;
        Decision::parse(&message).map_err(|err| io::Error::other(err.0))
    };
    let file = config.join("10-eth0.network");

    // The interface the configuration addresses is not left to DHCP: a client
    // that ran anyway would write a lease over the address the configuration
    // gave it, and the answer names the file that decided.
    let decision = ask_dhcp("eth0").unwrap();
    assert!(decision.about("eth0"));
    assert_eq!(decision.mode(), Mode::Static);
    assert_eq!(decision.source(), Some(file.as_path()));

    // A file that says DHCP leaves the decision to the server, and the file is
    // still the one that decided.
    fs::write(
        config.join("10-eth0.network"),
        "[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n",
    )
    .unwrap();
    let decision = ask_dhcp("eth0").unwrap();
    assert_eq!(decision.mode(), Mode::Dhcp);
    assert_eq!(decision.source(), Some(file.as_path()));

    // An interface no file matches is left to DHCP too — the way a system with
    // no configuration comes up — and then there is no file to name.
    let decision = ask_dhcp("wlan0").unwrap();
    assert_eq!(decision.mode(), Mode::Dhcp);
    assert_eq!(decision.source(), None);

    // A configuration that cannot be read is refused, and the refusal reaches
    // the client: a DHCP client that cannot be told must not decide on its own.
    fs::write(config.join("10-eth0.network"), [0xff, 0xfe]).unwrap();
    let error = ask(
        &service,
        Request::Dhcp {
            iface: "eth0".to_owned(),
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("10-eth0.network"), "{error}");

    let _ = fs::remove_dir_all(dir);
}
