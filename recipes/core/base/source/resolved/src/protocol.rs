//! The request protocol spoken over `/scheme/resolved`.
//!
//! Control is "write a request, read the response": a client opens one of the
//! control files of the scheme (`sync`, `set-dns`), writes a single request line
//! and reads the single response line back from the same file descriptor. The
//! grammar is text so that a daemon that has to say "the resolver changed" can
//! do it with plain file I/O — `dhcpd` writes to the scheme long before anything
//! would be linked against this crate.
//!
//! ```text
//! sync
//! set-dns <address>
//! ```
//!
//! A response is `ok <message>` or `err <message>`. For the two requests here
//! the message says what happened to the file, in the words of [`Outcome`]:
//! `wrote /etc/resolv.conf` or `unchanged /etc/resolv.conf`. The path is in the
//! answer because a client that named its own file has to be able to tell
//! whether the daemon is talking about that file.
//!
//! Reading needs no request: the scheme serves the live report as the file
//! `status` (see [`crate::render::render_status_document`]).

use std::fmt;
use std::path::{Path, PathBuf};

use networkd::Ipv4Addr;

/// Returned when a request or a response is not understood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolError(pub String);

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolError {}

/// A control request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Write what the network stack holds to the file, if it says otherwise.
    Sync,
    /// Decide the resolver: in the stack, and then in the file.
    SetDns(Ipv4Addr),
}

impl Request {
    /// The verb of the request, which is also the file name in the scheme.
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::SetDns(_) => "set-dns",
        }
    }

    /// Parses one request line.
    pub fn parse(line: &str) -> Result<Self, ProtocolError> {
        let line = line.trim();
        if line.is_empty() {
            return Err(ProtocolError("empty request".to_owned()));
        }
        if line.starts_with('#') {
            return Err(ProtocolError("comment, not a request".to_owned()));
        }

        let (verb, rest) = match line.split_once(char::is_whitespace) {
            Some((verb, rest)) => (verb, rest.trim()),
            None => (line, ""),
        };

        match verb {
            "sync" | "update" | "reload" => {
                if !rest.is_empty() {
                    return Err(ProtocolError(format!(
                        "sync takes no arguments, got {rest:?}"
                    )));
                }
                Ok(Self::Sync)
            }
            "set-dns" | "set-resolver" => {
                let args: Vec<&str> = rest.split_whitespace().collect();
                if args.len() != 1 {
                    return Err(ProtocolError(format!(
                        "set-dns takes one address, got {}",
                        args.len()
                    )));
                }
                match args[0].parse() {
                    Ok(addr) => Ok(Self::SetDns(addr)),
                    Err(err) => Err(ProtocolError(format!("{}", err))),
                }
            }
            _ => Err(ProtocolError(format!("unknown request {verb:?}"))),
        }
    }

    /// Renders the request the way a client writes it.
    pub fn encode(&self) -> String {
        match self {
            Self::Sync => "sync\n".to_owned(),
            Self::SetDns(addr) => format!("set-dns {addr}\n"),
        }
    }
}

/// What the daemon did with the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The file was written.
    Wrote(PathBuf),
    /// The file already said what the stack says.
    Unchanged(PathBuf),
}

impl Outcome {
    /// The outcome of a write of `path` that did or did not change it.
    pub fn new(path: impl Into<PathBuf>, changed: bool) -> Self {
        let path = path.into();
        if changed {
            Self::Wrote(path)
        } else {
            Self::Unchanged(path)
        }
    }

    /// The file this outcome is about.
    pub fn path(&self) -> &Path {
        match self {
            Self::Wrote(path) | Self::Unchanged(path) => path,
        }
    }

    /// Whether the file was written.
    pub fn changed(&self) -> bool {
        matches!(self, Self::Wrote(_))
    }

    /// Whether this outcome is about `path`.
    pub fn describes(&self, path: &Path) -> bool {
        self.path() == path
    }

    /// The message the client reads, which is how the outcome travels.
    pub fn encode(&self) -> String {
        let verb = if self.changed() { "wrote" } else { "unchanged" };
        format!("{verb} {}", self.path().display())
    }

    /// Reads back the message of a response.
    pub fn parse(message: &str) -> Result<Self, ProtocolError> {
        let message = message.trim();
        let (verb, path) = match message.split_once(char::is_whitespace) {
            Some((verb, path)) => (verb, path.trim()),
            None => (message, ""),
        };

        if path.is_empty() {
            return Err(ProtocolError(format!("no file in the answer {message:?}")));
        }

        match verb {
            "wrote" => Ok(Self::Wrote(PathBuf::from(path))),
            "unchanged" => Ok(Self::Unchanged(PathBuf::from(path))),
            _ => Err(ProtocolError(format!("malformed answer {message:?}"))),
        }
    }
}

/// Renders the response line for a handled request.
pub fn format_response(response: &Result<String, ProtocolError>) -> String {
    match response {
        Ok(message) if message.is_empty() => "ok\n".to_owned(),
        Ok(message) => format!("ok {message}\n"),
        Err(error) => format!("err {error}\n"),
    }
}

/// Reads the response line of a request.
pub fn parse_response(text: &str) -> Result<String, ProtocolError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ProtocolError(
            "no response from the resolver daemon".to_owned(),
        ));
    }

    if let Some(message) = text.strip_prefix("ok") {
        return Ok(message.trim_start().to_owned());
    }
    if let Some(message) = text.strip_prefix("err") {
        return Err(ProtocolError(message.trim_start().to_owned()));
    }

    Err(ProtocolError(format!("malformed response {text:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requests_are_the_files_that_take_them() {
        assert_eq!(Request::parse("sync").unwrap(), Request::Sync);
        assert_eq!(Request::parse("  update \n").unwrap(), Request::Sync);
        assert_eq!(Request::parse("reload").unwrap(), Request::Sync);
        assert_eq!(Request::Sync.verb(), "sync");
        assert_eq!(Request::Sync.encode(), "sync\n");

        let set = Request::parse("set-dns 1.1.1.1\n").unwrap();
        assert_eq!(set, Request::SetDns("1.1.1.1".parse().unwrap()));
        assert_eq!(set.verb(), "set-dns");
        assert_eq!(set.encode(), "set-dns 1.1.1.1\n");

        // A request survives a round trip through the wire, both ways.
        assert_eq!(
            Request::parse(&Request::Sync.encode()).unwrap(),
            Request::Sync
        );
        assert_eq!(Request::parse(&set.encode()).unwrap(), set);
    }

    #[test]
    fn malformed_requests_are_rejected() {
        for line in [
            "",
            "   ",
            "# a comment",
            "flush-caches",
            "sync now",
            "set-dns",
            "set-dns 9.9.9.9 1.1.1.1",
            "set-dns banana",
        ] {
            let error = Request::parse(line).unwrap_err();
            assert!(!error.0.is_empty(), "{line:?} was accepted");
        }

        for (line, expected) in [
            ("flush-caches", "unknown request"),
            ("set-dns", "one address"),
            ("set-dns banana", "not an IPv4 address"),
        ] {
            let error = Request::parse(line).unwrap_err();
            assert!(error.0.contains(expected), "{line:?}: {error}");
        }
    }

    #[test]
    fn an_outcome_says_which_file_and_whether_it_moved() {
        let wrote = Outcome::new("/etc/resolv.conf", true);
        assert_eq!(wrote.encode(), "wrote /etc/resolv.conf");
        assert_eq!(wrote.path(), Path::new("/etc/resolv.conf"));
        assert!(wrote.changed());
        assert!(wrote.describes(Path::new("/etc/resolv.conf")));
        assert!(!wrote.describes(Path::new("/tmp/resolv.conf")));

        let unchanged = Outcome::new(PathBuf::from("/etc/resolv.conf"), false);
        assert_eq!(unchanged.encode(), "unchanged /etc/resolv.conf");
        assert!(!unchanged.changed());

        // What the daemon answers is what the client reads.
        for outcome in [wrote, unchanged] {
            assert_eq!(Outcome::parse(&outcome.encode()).unwrap(), outcome);
            let response = format_response(&Ok(outcome.encode()));
            let message = parse_response(&response).unwrap();
            assert_eq!(Outcome::parse(&message).unwrap(), outcome);
        }
    }

    #[test]
    fn a_malformed_outcome_is_not_a_write() {
        for message in ["", "  ", "wrote", "yes /etc/resolv.conf", "banana"] {
            assert!(Outcome::parse(message).is_err(), "{message:?} was accepted");
        }
    }

    #[test]
    fn responses_are_a_single_verdict() {
        assert_eq!(format_response(&Ok("wrote /x".to_owned())), "ok wrote /x\n");
        assert_eq!(format_response(&Ok(String::new())), "ok\n");
        assert_eq!(
            format_response(&Err(ProtocolError("no file".to_owned()))),
            "err no file\n"
        );

        assert_eq!(
            parse_response("ok wrote /x\n").unwrap(),
            "wrote /x\n".trim()
        );
        assert_eq!(parse_response("ok\n").unwrap(), "");
        assert_eq!(parse_response("err no file\n").unwrap_err().0, "no file");
        assert!(parse_response("").is_err());
        assert!(parse_response("whatever\n").is_err());
    }
}
