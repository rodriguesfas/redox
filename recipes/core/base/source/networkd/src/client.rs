//! The `networkctl` side of the daemon protocol.
//!
//! Writes go through the daemon when it is there: one writer means the stack is
//! written by whoever owns it, and a request that arrives while the network is
//! changing cannot race with a write. Reads never need the daemon — the
//! configuration and the stack are readable by anyone — so they are answered
//! without it, which is also what makes `networkctl` work from a rescue shell.
//!
//! When the daemon is not there, a write is done directly, which is what the
//! tool did before there was a daemon.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::cli::Cli;
use crate::protocol::{parse_response, Outcome, Request};

/// Where the network daemon is mounted.
pub const DEFAULT_SCHEME: &str = "/scheme/networkd";

/// Environment variable overriding the mount point of the network daemon.
pub const ENV_SCHEME: &str = "NETWORKD_SCHEME";

/// A connection to the network daemon.
#[derive(Clone, Debug)]
pub struct Client {
    scheme: PathBuf,
}

impl Client {
    /// A client for a mount point.
    pub fn new(scheme: impl Into<PathBuf>) -> Self {
        Self {
            scheme: scheme.into(),
        }
    }

    /// A client for the mount point the environment selects.
    pub fn from_env() -> Self {
        match std::env::var(ENV_SCHEME) {
            Ok(scheme) if !scheme.trim().is_empty() => Self::new(scheme),
            _ => Self::new(DEFAULT_SCHEME),
        }
    }

    /// The mount point this client talks to.
    pub fn scheme(&self) -> &Path {
        &self.scheme
    }

    /// Reads one of the files the scheme serves.
    pub fn read(&self, name: &str) -> io::Result<String> {
        let mut text = String::new();
        File::open(self.scheme.join(name))?.read_to_string(&mut text)?;
        Ok(text)
    }

    /// Sends a request and returns its message.
    ///
    /// Control files answer on the same descriptor they are written to, so the
    /// response of a request is read back right after writing it. A request the
    /// daemon refuses comes back as an error, which is what the caller sees; a
    /// daemon that is not there cannot be opened at all.
    pub fn request(&self, request: &Request) -> io::Result<String> {
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(self.scheme.join(request.verb()))?;
        exchange(&mut file, &request.encode())
    }

    /// Sends a request if a daemon answers, and says so when none does.
    ///
    /// `None` is "there is nothing to ask": the mount point is not there, which
    /// is what a system without the daemon looks like. An answer is always a
    /// verdict — including a refusal, which a caller has to report instead of
    /// quietly doing the work itself.
    pub fn request_if_mounted(&self, request: &Request) -> Option<io::Result<Outcome>> {
        let mut file = match File::options()
            .read(true)
            .write(true)
            .open(self.scheme.join(request.verb()))
        {
            Ok(file) => file,
            Err(_) => return None,
        };

        Some(
            exchange(&mut file, &request.encode()).and_then(|message| {
                Outcome::parse(&message).map_err(|err| io::Error::other(err.0))
            }),
        )
    }

    /// Asks the daemon to make the stack look like the configuration.
    pub fn apply(&self) -> io::Result<Outcome> {
        answer(&self.request(&Request::Apply)?)
    }

    /// The live report the daemon serves, which is what it sees right now.
    pub fn status_report(&self) -> io::Result<String> {
        self.read("status")
    }
}

/// Whether a command leaves the write to the daemon.
///
/// A dry run never writes, so it never asks; and a command that named a
/// configuration directory or a mount point of its own is about that one, which
/// is not what the daemon manages. What is left is the write of the system,
/// which is the daemon's to make when it is running.
pub fn writes_through_the_daemon(cli: &Cli) -> bool {
    !cli.dry_run && !cli.config_dir_given && !cli.netcfg_given
}

/// Reads the outcome out of a response message.
fn answer(message: &str) -> io::Result<Outcome> {
    Outcome::parse(message).map_err(|err| io::Error::other(err.0))
}

/// Writes a request and reads the response back from the same file.
///
/// The response is read until the end of the file, which for a control file
/// means until the scheme has nothing left to hand over.
pub fn exchange(file: &mut (impl Read + Write), request: &str) -> io::Result<String> {
    file.write_all(request.as_bytes())?;
    file.flush()?;

    let mut response = String::new();
    file.read_to_string(&mut response)?;
    parse_response(&response).map_err(|err| io::Error::other(err.0))
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("networkd-client-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_mount_point_comes_from_the_environment() {
        assert_eq!(Client::new("/tmp/x").scheme(), Path::new("/tmp/x"));

        std::env::set_var(ENV_SCHEME, "/tmp/scheme");
        assert_eq!(Client::from_env().scheme(), Path::new("/tmp/scheme"));
        std::env::set_var(ENV_SCHEME, "  ");
        assert_eq!(Client::from_env().scheme(), Path::new(DEFAULT_SCHEME));
        std::env::remove_var(ENV_SCHEME);
        assert_eq!(Client::from_env().scheme(), Path::new(DEFAULT_SCHEME));
    }

    #[test]
    fn a_request_the_daemon_answers_comes_back_as_an_outcome() {
        // A control file answers on the descriptor it was written to, which a
        // socket pair models on the host: what the daemon would read, and what
        // it would answer with.
        let (mut client_end, mut daemon_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            assert_eq!(read_line(&mut daemon_end), "apply\n");
            daemon_end
                .write_all(b"ok applied\t3\t/scheme/netcfg\t/etc/networkd\n")
                .unwrap();
            daemon_end.flush().unwrap();
        });

        let message = exchange(&mut client_end, "apply\n").unwrap();
        let outcome = answer(&message).unwrap();
        assert_eq!(
            outcome,
            Outcome::Applied {
                writes: 3,
                netcfg: PathBuf::from("/scheme/netcfg"),
                config: PathBuf::from("/etc/networkd"),
            }
        );
        assert!(outcome.changed());
    }

    #[test]
    fn a_request_the_daemon_refuses_is_an_error() {
        let (mut client_end, mut daemon_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            assert_eq!(read_line(&mut daemon_end), "apply\n");
            let _ = daemon_end.write_all(b"err cannot read the interfaces in /scheme/netcfg\n");
            let _ = daemon_end.flush();
        });

        let error = exchange(&mut client_end, "apply\n").unwrap_err();
        assert!(
            error.to_string().contains("cannot read the interfaces"),
            "{error}"
        );
    }

    #[test]
    fn a_daemon_that_is_not_there_cannot_be_written_to() {
        let dir = scratch("no-daemon");
        let client = Client::new(dir.join("scheme"));

        assert!(client.apply().is_err());
        assert!(client.status_report().is_err());
        assert!(client.request_if_mounted(&Request::Apply).is_none());
    }

    #[test]
    fn the_report_the_daemon_serves_is_read_as_it_is() {
        let dir = scratch("status");
        std::fs::write(dir.join("status"), "eth0: dhcp, from /etc/networkd\n").unwrap();

        let report = Client::new(&dir).status_report().unwrap();
        assert_eq!(report, "eth0: dhcp, from /etc/networkd\n");
    }

    #[test]
    fn the_write_of_the_system_is_the_daemons_and_the_others_are_not() {
        let parse = |args: &[&str]| crate::cli::parse_args(args.iter().map(|arg| arg.to_string()));

        // The configuration of the system, applied for real: the daemon's write.
        assert!(writes_through_the_daemon(&parse(&["apply"]).unwrap()));

        // What is only printed is not written by anyone.
        assert!(!writes_through_the_daemon(
            &parse(&["--dry-run", "apply"]).unwrap()
        ));

        // A directory or a mount point that was named is not the system's.
        assert!(!writes_through_the_daemon(
            &parse(&["--config-dir", "/tmp/conf", "apply"]).unwrap()
        ));
        assert!(!writes_through_the_daemon(
            &parse(&["--netcfg", "/tmp/netcfg", "apply"]).unwrap()
        ));
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
}
