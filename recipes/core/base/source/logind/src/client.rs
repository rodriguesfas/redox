//! The `loginctl` side of the daemon protocol.
//!
//! Reads come from the daemon when it answers and from the record file
//! otherwise, so `loginctl` keeps working during early boot and from a rescue
//! shell. Writes always need the daemon: a session cannot be stopped by editing
//! a file.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::passwd::lookup_uid;
use crate::protocol::{parse_response, Request};
use crate::session::Sessions;
use crate::utmp::UtmpFile;

/// Where the login manager is mounted.
pub const DEFAULT_SCHEME: &str = "/scheme/logind";

/// Environment variable overriding the mount point of the login manager.
pub const ENV_SCHEME: &str = "LOGIND_SCHEME";

/// Where a session list came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The live view of the running daemon.
    Daemon,
    /// The `utmp` record file, which is the only view when no daemon runs.
    File,
}

/// A connection to the login manager.
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

    /// The sessions the daemon is tracking.
    pub fn sessions(&self) -> io::Result<Sessions> {
        Sessions::parse(&self.read("sessions")?).map_err(|err| io::Error::other(err.0))
    }

    /// Sends a request and returns its message.
    ///
    /// Control files answer on the same descriptor they are written to, so the
    /// response of a request is read back right after writing it. The exception
    /// is a request that is not understood: the scheme answers with an error,
    /// which is what the caller sees.
    pub fn request(&self, request: &Request) -> io::Result<String> {
        let mut file = File::options()
            .read(true)
            .write(true)
            .open(self.scheme.join(request.verb()))?;
        exchange(&mut file, &request.encode())
    }

    /// Stops a session.
    pub fn terminate(&self, target: &str) -> io::Result<String> {
        self.request(&Request::Terminate {
            target: target.to_owned(),
        })
    }

    /// Gives a session the console of its seat.
    pub fn activate(&self, target: &str) -> io::Result<String> {
        self.request(&Request::Activate {
            target: target.to_owned(),
        })
    }

    /// The sessions of the system: the daemon's live view when it answers, the
    /// record file otherwise.
    pub fn sessions_or_file(&self, utmp: &UtmpFile) -> io::Result<(Sessions, Origin)> {
        match self.sessions() {
            Ok(sessions) => Ok((sessions, Origin::Daemon)),
            Err(_) => Ok((file_sessions(utmp)?, Origin::File)),
        }
    }
}

/// The sessions recorded in the `utmp` file.
///
/// The file stores login names only, so the names are resolved again against
/// the account database to make the listings readable.
pub fn file_sessions(utmp: &UtmpFile) -> io::Result<Sessions> {
    let mut sessions = Sessions::from_utmp(&utmp.read_all()?);
    sessions.assign_uids(lookup_uid);
    Ok(sessions)
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
    use crate::utmp::UtmpRecord;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("logind-client-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_mount_point_comes_from_the_environment() {
        assert_eq!(Client::new("/tmp/x").scheme(), Path::new("/tmp/x"));

        // `from_env` reads the environment, so the test has to set it. It runs
        // in its own process-wide variable, which the other tests do not touch.
        std::env::set_var(ENV_SCHEME, "/tmp/scheme");
        assert_eq!(Client::from_env().scheme(), Path::new("/tmp/scheme"));
        std::env::set_var(ENV_SCHEME, "  ");
        assert_eq!(Client::from_env().scheme(), Path::new(DEFAULT_SCHEME));
        std::env::remove_var(ENV_SCHEME);
        assert_eq!(Client::from_env().scheme(), Path::new(DEFAULT_SCHEME));
    }

    #[test]
    fn a_missing_daemon_falls_back_to_the_record_file() {
        let dir = scratch("fallback");
        let utmp = UtmpFile::new(dir.join("utmp"));
        utmp.put(&UtmpRecord::user_process(42, "tty1", "user", 1_100))
            .unwrap();

        // No scheme: the file is the only view there is.
        let client = Client::new(dir.join("scheme"));
        assert!(client.sessions().is_err());
        let (sessions, origin) = client.sessions_or_file(&utmp).unwrap();
        assert_eq!(origin, Origin::File);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions.list()[0].user, "user");

        // A scheme that answers wins over the file.
        let scheme = scratch("daemon");
        std::fs::write(
            scheme.join("sessions"),
            "7\t1000\troot\tseat0\ttty9\ttty\tactive\t5\t1\t\n",
        )
        .unwrap();
        let client = Client::new(&scheme);
        let (sessions, origin) = client.sessions_or_file(&utmp).unwrap();
        assert_eq!(origin, Origin::Daemon);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions.list()[0].id, "7");

        // A daemon that answers nonsense is not trusted either.
        std::fs::write(scheme.join("sessions"), "not a record\n").unwrap();
        let (sessions, origin) = client.sessions_or_file(&utmp).unwrap();
        assert_eq!(origin, Origin::File);
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn requests_are_written_and_the_response_is_read_back() {
        // A control file answers on the descriptor it was written to, which a
        // socket pair models on the host: what the daemon would read, and what
        // it would answer with.
        let (mut client_end, mut daemon_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let request = read_line(&mut daemon_end);
            assert_eq!(request, "terminate 3\n");
            daemon_end.write_all(b"ok 3\n").unwrap();
            daemon_end.flush().unwrap();
            // Returning closes the daemon side, which is what ends the
            // response as far as the client is concerned.
        });

        let response = exchange(&mut client_end, "terminate 3\n").unwrap();
        assert_eq!(response, "3");
    }

    #[test]
    fn a_refused_request_is_an_error() {
        let (mut client_end, mut daemon_end) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let request = read_line(&mut daemon_end);
            assert_eq!(request, "activate 9\n");
            let _ = daemon_end.write_all(b"err no such session\n");
            let _ = daemon_end.flush();
        });

        let error = exchange(&mut client_end, "activate 9\n").unwrap_err();
        assert_eq!(error.to_string(), "no such session");

        // A path that is not there at all, as when no daemon runs.
        let dir = scratch("no-daemon");
        let client = Client::new(dir.join("scheme"));
        assert!(client.terminate("1").is_err());
        assert!(client.activate("tty1").is_err());
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
