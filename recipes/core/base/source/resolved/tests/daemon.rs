//! End-to-end test of the resolver daemon: what it writes at boot, what it does
//! when the network changes under it, what a request over the wire produces, and
//! what `resolvectl` sees afterwards.
//!
//! The scheme itself only runs on Redox; everything under it is tested here —
//! the service that applies a request, the watcher that keeps the file in step,
//! the protocol as it travels on a descriptor — and so is the real `resolvectl`
//! binary reading the file the daemon wrote.

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use resolved::client::exchange;
use resolved::protocol::{format_response, Outcome, ProtocolError, Request};
use resolved::service::Service;
use resolved::state::{shared, Resolved, NETCFG_PATH};
use resolved::watch::{Watcher, DEFAULT_INTERVAL};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("resolved-daemon-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A stack mount point that holds `resolver`, and the file to manage.
fn system(name: &str, resolver: Option<&str>) -> (PathBuf, PathBuf, PathBuf) {
    let dir = scratch(name);
    let netcfg = dir.join("netcfg");
    let control = netcfg.join(NETCFG_PATH);
    fs::create_dir_all(control.parent().unwrap()).unwrap();
    fs::write(&control, resolver.unwrap_or("")).unwrap();

    (dir.clone(), netcfg, dir.join("resolv.conf"))
}

/// The service a daemon would run, over a stack mount point and a file.
fn service_of(netcfg: &Path, file: &Path) -> Service {
    Service::new(shared(Resolved::new(
        file.to_path_buf(),
        networkd::Netcfg::new(netcfg.to_string_lossy().into_owned()),
    )))
}

/// The line the C library reads first, which is the whole point of the file.
fn first_nameserver(file: &Path) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    text.lines()
        .find(|line| line.starts_with("nameserver "))
        .map(str::to_owned)
}

/// Sends a request the way `resolvectl` does, and reads the answer back.
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

fn resolvectl(file: &Path, netcfg: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_resolvectl"))
        .env_remove("RESOLVED_FILE")
        .env_remove("NETWORKD_NETCFG")
        .env_remove("RESOLVED_SCHEME")
        .arg("--file")
        .arg(file)
        .arg("--netcfg")
        .arg(netcfg)
        .args(args)
        .output()
        .expect("resolvectl should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn the_daemon_writes_the_file_when_a_lease_arrives_and_keeps_it_in_step() {
    let (_dir, netcfg, file) = system("boot", None);
    let service = service_of(&netcfg, &file);
    let mut watcher = Watcher::new(service.state().clone(), DEFAULT_INTERVAL);

    // Nothing to write at boot: the stack is there and holds no resolver yet.
    assert!(watcher.look().unwrap().is_none());
    assert!(!file.exists());
    assert!(service.status_document().contains("   In sync: no\n"));

    // The lease arrives and the resolver is in the stack: the daemon writes it
    // without being asked, which is what makes a name resolvable.
    fs::write(netcfg.join(NETCFG_PATH), "1.1.1.1\n").unwrap();
    let sync = watcher
        .look()
        .unwrap()
        .expect("the resolver has to be written");
    assert!(sync.changed);
    assert_eq!(
        first_nameserver(&file).as_deref(),
        Some("nameserver 1.1.1.1")
    );

    // The tool sees the file the daemon wrote.
    let status = resolvectl(&file, &netcfg, &["status"]);
    assert!(status.status.success(), "{}", stdout(&status));
    assert!(
        stdout(&status).contains("   In sync: yes\n"),
        "{}",
        stdout(&status)
    );
    assert!(
        stdout(&status).contains(" Resolvers: 1.1.1.1\n"),
        "{}",
        stdout(&status)
    );

    // The network changes again: the daemon carries it, and only then does the
    // file move.
    fs::write(netcfg.join(NETCFG_PATH), "9.9.9.9\n").unwrap();
    assert!(watcher.look().unwrap().is_some());
    assert_eq!(
        first_nameserver(&file).as_deref(),
        Some("nameserver 9.9.9.9")
    );

    // And a look with nothing to do writes nothing at all.
    let before = fs::read_to_string(&file).unwrap();
    assert!(watcher.look().unwrap().is_none());
    assert_eq!(fs::read_to_string(&file).unwrap(), before);

    let _ = fs::remove_dir_all(_dir);
}

#[test]
fn a_request_decides_both_places_and_the_file_says_it() {
    let (_dir, netcfg, file) = system("request", Some("8.8.8.8\n"));
    let service = service_of(&netcfg, &file);

    // A request travels the wire the way the scheme hands it over: a line in,
    // and a line back that says what happened to which file.
    let message = ask(&service, Request::Sync).unwrap();
    let outcome = Outcome::parse(&message).unwrap();
    assert_eq!(outcome, Outcome::Wrote(file.clone()));
    assert_eq!(
        first_nameserver(&file).as_deref(),
        Some("nameserver 8.8.8.8")
    );

    let message = ask(&service, Request::SetDns("1.1.1.1".parse().unwrap())).unwrap();
    let outcome = Outcome::parse(&message).unwrap();
    assert!(outcome.changed());
    assert!(outcome.describes(&file));
    assert_eq!(
        fs::read_to_string(netcfg.join(NETCFG_PATH)).unwrap(),
        "1.1.1.1\n"
    );
    assert_eq!(
        first_nameserver(&file).as_deref(),
        Some("nameserver 1.1.1.1")
    );

    // Asking for the same resolver again is not a write.
    let message = ask(&service, Request::SetDns("1.1.1.1".parse().unwrap())).unwrap();
    assert_eq!(
        Outcome::parse(&message).unwrap(),
        Outcome::Unchanged(file.clone())
    );

    // The daemon report and the tool agree on the state of the file.
    assert!(service.status_document().contains("   In sync: yes\n"));
    let dns = resolvectl(&file, &netcfg, &["dns"]);
    assert!(stdout(&dns).contains("1.1.1.1"), "{}", stdout(&dns));

    let _ = fs::remove_dir_all(_dir);
}

#[test]
fn a_resolver_a_query_could_not_be_sent_to_is_refused_on_the_wire() {
    let (_dir, netcfg, file) = system("refused", Some("8.8.8.8\n"));
    let service = service_of(&netcfg, &file);

    let error = ask(&service, Request::SetDns("0.0.0.0".parse().unwrap())).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("not an address a query can be sent to"),
        "{error}"
    );

    // Nothing moved: the stack is as it was, and no file was created when the
    // refusal happened before there was one to keep.
    assert_eq!(
        fs::read_to_string(netcfg.join(NETCFG_PATH)).unwrap(),
        "8.8.8.8\n"
    );
    assert!(!file.exists());

    let _ = fs::remove_dir_all(_dir);
}

#[test]
fn a_request_that_is_not_a_request_is_answered_with_an_error() {
    let (_dir, netcfg, file) = system("malformed", Some("8.8.8.8\n"));
    let service = service_of(&netcfg, &file);

    let (mut client_end, mut daemon_end) = UnixStream::pair().unwrap();
    let service = service.clone();
    std::thread::spawn(move || {
        assert_eq!(read_line(&mut daemon_end), "flush-caches\n");
        let response = answer(&service, "flush-caches\n");
        daemon_end.write_all(response.as_bytes()).unwrap();
        daemon_end.flush().unwrap();
    });

    let error = exchange(&mut client_end, "flush-caches\n").unwrap_err();
    assert!(error.to_string().contains("unknown request"), "{error}");

    let _ = fs::remove_dir_all(_dir);
}

#[test]
fn a_stack_that_holds_no_resolver_is_refused_in_the_words_of_the_client() {
    let (_dir, netcfg, file) = system("empty-stack", None);
    let service = service_of(&netcfg, &file);

    let error = ask(&service, Request::Sync).unwrap_err();
    assert!(error.to_string().contains("holds no resolver"), "{error}");
    assert!(!file.exists());

    // A stack that is not mounted at all says that instead of pretending there
    // is nothing to write.
    let unmounted = dir_of_absent_stack(&file);
    let error = ask(&unmounted, Request::Sync).unwrap_err();
    assert!(error.to_string().contains("cannot be read"), "{error}");

    let _ = fs::remove_dir_all(_dir);
}

/// A service whose stack mount point does not exist: no netstack yet.
fn dir_of_absent_stack(file: &Path) -> Service {
    Service::new(shared(Resolved::new(
        file.to_path_buf(),
        networkd::Netcfg::new(
            file.parent()
                .unwrap()
                .join("absent")
                .to_string_lossy()
                .into_owned(),
        ),
    )))
}

#[test]
fn nothing_is_asked_of_a_daemon_that_is_not_there() {
    let dir = scratch("no-daemon");
    let client = resolved::client::Client::new(dir.join("scheme"));

    // The mount point is not there: there is nothing to ask, and the caller
    // writes the file itself, as the tool did before there was a daemon.
    assert!(client.request_if_mounted(&Request::Sync).is_none());
    assert!(client
        .request_if_mounted(&Request::SetDns("9.9.9.9".parse().unwrap()))
        .is_none());

    // A daemon that answers about another file is a daemon for another system:
    // its answer is an outcome about that file, and the caller can tell.
    let outcome = Outcome::Wrote(PathBuf::from("/etc/resolv.conf"));
    assert!(!outcome.describes(&dir.join("resolv.conf")));

    let _ = fs::remove_dir_all(dir);
}
