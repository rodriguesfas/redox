//! The request protocol spoken over `/scheme/networkd`.
//!
//! Control is "write a request, read the response": a client opens the control
//! file of the scheme (`apply`, `dhcp`), writes a single request line and reads
//! the response line back from the same file descriptor. The grammar is text so
//! that a daemon that has to say "the configuration changed" can do it with
//! plain file I/O.
//!
//! ```text
//! apply
//! dhcp eth0
//! ```
//!
//! A response is `ok <message>` or `err <message>`. For `apply` the message
//! says what happened to the stack, in the words of [`Outcome`]:
//! `applied <n> <netcfg>\t<config>` or `unchanged <netcfg>\t<config>`. For
//! `dhcp` it says what the configuration decided about one interface, in the
//! words of [`Decision`]. The paths are in the answers because a client that
//! named its own configuration or its own stack has to be able to tell whether
//! the daemon is talking about them — and they are separated by a tab because a
//! path may contain spaces and neither of them may be cut in half.
//!
//! Reading needs no request: the scheme serves the live report as the file
//! `status` (see [`crate::render::render_status_document`]).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::plan::{InterfacePlan, Mode};
use crate::state::Application;

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
    /// Read the configuration, and write what it says to the stack.
    Apply,
    /// Ask what the configuration decides about the address of one interface.
    Dhcp {
        /// The interface being asked about (`eth0`).
        iface: String,
    },
}

impl Request {
    /// The verb of the request, which is also the file name in the scheme.
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Dhcp { .. } => "dhcp",
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

        match line {
            "apply" | "sync" | "reload" | "update" => Ok(Self::Apply),
            _ => match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["dhcp"] => Err(ProtocolError("dhcp needs an interface".to_owned())),
                ["dhcp", iface] if is_interface_name(iface) => Ok(Self::Dhcp {
                    iface: (*iface).to_owned(),
                }),
                ["dhcp", iface] => {
                    Err(ProtocolError(format!("{iface:?} is not an interface name")))
                }
                ["dhcp", ..] => Err(ProtocolError(format!("dhcp takes one interface: {line:?}"))),
                _ => Err(ProtocolError(format!("unknown request {line:?}"))),
            },
        }
    }

    /// Renders the request the way a client writes it.
    pub fn encode(&self) -> String {
        match self {
            Self::Apply => "apply\n".to_owned(),
            Self::Dhcp { iface } => format!("dhcp {iface}\n"),
        }
    }
}

/// Whether a word can be the name of an interface.
///
/// The name is used as a path element of the stack (`ifaces/<name>/mac`), so a
/// name that could walk out of it is not a name: `..` and anything with a
/// separator in it describes a file, not an interface.
pub fn is_interface_name(word: &str) -> bool {
    !word.is_empty() && word != "." && word != ".." && !word.contains(['/', '\0'])
}

/// What the configuration decides about the address of one interface.
///
/// This is the question the DHCP client asks at boot: an interface the
/// configuration leaves to DHCP gets its address from a server, and a client
/// that ran for an interface the configuration addresses itself would write
/// over the address the configuration gave it. The file that decided is in the
/// answer — when one did — because "DHCP=yes in this file" and "no file
/// matched" are different situations for whoever is reading the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    /// The interface this is about.
    pub iface: String,
    /// The file of the configuration that decided, when one did.
    pub source: Option<PathBuf>,
    /// How the address is decided.
    pub mode: Mode,
}

impl Decision {
    /// What the configuration decided about one interface.
    pub fn of(plan: &InterfacePlan) -> Self {
        Self {
            iface: plan.name.clone(),
            source: plan.source.clone(),
            mode: plan.mode,
        }
    }

    /// The interface this is about.
    pub fn iface(&self) -> &str {
        &self.iface
    }

    /// The file that decided, when one did.
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// How the address is decided.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether this is the answer to a question about this interface.
    pub fn about(&self, iface: &str) -> bool {
        self.iface == iface
    }

    /// The message the client reads, which is how the decision travels.
    ///
    /// The file comes before the mode so that a decision no file made keeps
    /// its fields: an empty file field in the middle survives the line being
    /// trimmed, and a field at the end would not.
    pub fn encode(&self) -> String {
        format!(
            "dhcp\t{}\t{}\t{}",
            self.iface,
            self.source
                .as_deref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            self.mode.as_str(),
        )
    }

    /// Reads back the message of a response.
    pub fn parse(message: &str) -> Result<Self, ProtocolError> {
        let mut fields = message.trim().split('\t');
        let malformed = || ProtocolError(format!("malformed answer {:?}", message.trim()));

        if fields.next().ok_or_else(malformed)? != "dhcp" {
            return Err(malformed());
        }

        let iface = fields.next().ok_or_else(malformed)?;
        let source = fields.next().ok_or_else(malformed)?;
        let mode = match fields.next().ok_or_else(malformed)? {
            "dhcp" => Mode::Dhcp,
            "static" => Mode::Static,
            _ => return Err(malformed()),
        };

        if fields.next().is_some() || iface.is_empty() || !is_interface_name(iface) {
            return Err(malformed());
        }

        Ok(Self {
            iface: iface.to_owned(),
            source: if source.is_empty() {
                None
            } else {
                Some(PathBuf::from(source))
            },
            mode,
        })
    }
}

/// What applying the configuration did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The stack was written to.
    Applied {
        /// How many writes landed.
        writes: usize,
        /// The stack that was written.
        netcfg: PathBuf,
        /// The configuration that was read.
        config: PathBuf,
    },
    /// There was nothing to write.
    Unchanged {
        /// The stack that was read.
        netcfg: PathBuf,
        /// The configuration that was read.
        config: PathBuf,
    },
}

impl Outcome {
    /// The outcome of what a manager applied.
    pub fn of(application: &Application) -> Self {
        if application.changed {
            Self::Applied {
                writes: application.writes.len(),
                netcfg: PathBuf::from(&application.netcfg),
                config: application.config.clone(),
            }
        } else {
            Self::Unchanged {
                netcfg: PathBuf::from(&application.netcfg),
                config: application.config.clone(),
            }
        }
    }

    /// The stack this outcome is about.
    pub fn netcfg(&self) -> &Path {
        match self {
            Self::Applied { netcfg, .. } | Self::Unchanged { netcfg, .. } => netcfg,
        }
    }

    /// The configuration this outcome is about.
    pub fn config(&self) -> &Path {
        match self {
            Self::Applied { config, .. } | Self::Unchanged { config, .. } => config,
        }
    }

    /// Whether the stack was written to.
    pub fn changed(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }

    /// Whether this is about the same stack and the same configuration.
    pub fn describes(&self, netcfg: &Path, config: &Path) -> bool {
        self.netcfg() == netcfg && self.config() == config
    }

    /// The message the client reads, which is how the outcome travels.
    pub fn encode(&self) -> String {
        match self {
            Self::Applied {
                writes,
                netcfg,
                config,
            } => format!(
                "applied\t{writes}\t{}\t{}",
                netcfg.display(),
                config.display()
            ),
            Self::Unchanged { netcfg, config } => {
                format!("unchanged\t{}\t{}", netcfg.display(), config.display())
            }
        }
    }

    /// Reads back the message of a response.
    pub fn parse(message: &str) -> Result<Self, ProtocolError> {
        let mut fields = message.trim().split('\t');
        let malformed = || ProtocolError(format!("malformed answer {:?}", message.trim()));

        let verb = fields.next().ok_or_else(malformed)?;

        match verb {
            "applied" => {
                let writes = fields
                    .next()
                    .ok_or_else(malformed)?
                    .parse::<usize>()
                    .map_err(|_| malformed())?;
                let netcfg = fields.next().ok_or_else(malformed)?;
                let config = fields.next().ok_or_else(malformed)?;
                if fields.next().is_some() || netcfg.is_empty() || config.is_empty() {
                    return Err(malformed());
                }

                Ok(Self::Applied {
                    writes,
                    netcfg: PathBuf::from(netcfg),
                    config: PathBuf::from(config),
                })
            }
            "unchanged" => {
                let netcfg = fields.next().ok_or_else(malformed)?;
                let config = fields.next().ok_or_else(malformed)?;
                if fields.next().is_some() || netcfg.is_empty() || config.is_empty() {
                    return Err(malformed());
                }

                Ok(Self::Unchanged {
                    netcfg: PathBuf::from(netcfg),
                    config: PathBuf::from(config),
                })
            }
            _ => Err(malformed()),
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
            "no response from the network daemon".to_owned(),
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
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn the_request_is_the_file_that_takes_it() {
        assert_eq!(Request::parse("apply").unwrap(), Request::Apply);
        assert_eq!(Request::parse("  reload \n").unwrap(), Request::Apply);
        assert_eq!(Request::Apply.verb(), "apply");
        assert_eq!(Request::Apply.encode(), "apply\n");
        assert_eq!(
            Request::parse(&Request::Apply.encode()).unwrap(),
            Request::Apply
        );
    }

    #[test]
    fn malformed_requests_are_rejected() {
        for line in [
            "",
            "   ",
            "# apply",
            "flush",
            "apply now",
            "address 10.0.0.1",
        ] {
            let error = Request::parse(line).unwrap_err();
            assert!(!error.0.is_empty(), "{line:?} was accepted");
        }
        assert!(Request::parse("flush").unwrap_err().0.contains("unknown"));
    }

    #[test]
    fn an_outcome_says_what_was_written_where() {
        let applied = Outcome::Applied {
            writes: 3,
            netcfg: PathBuf::from("/scheme/netcfg"),
            config: PathBuf::from("/etc/networkd"),
        };
        assert_eq!(
            applied.encode(),
            "applied\t3\t/scheme/netcfg\t/etc/networkd"
        );
        assert!(applied.changed());
        assert!(applied.describes(Path::new("/scheme/netcfg"), Path::new("/etc/networkd")));
        assert!(!applied.describes(Path::new("/tmp/netcfg"), Path::new("/etc/networkd")));

        let unchanged = Outcome::Unchanged {
            netcfg: PathBuf::from("/scheme/netcfg"),
            config: PathBuf::from("/etc/networkd"),
        };
        assert_eq!(
            unchanged.encode(),
            "unchanged\t/scheme/netcfg\t/etc/networkd"
        );
        assert!(!unchanged.changed());
    }

    #[test]
    fn an_outcome_survives_a_path_with_a_space_in_it() {
        let outcome = Outcome::Unchanged {
            netcfg: PathBuf::from("/tmp/net cfg"),
            config: PathBuf::from("/etc/network files"),
        };

        assert_eq!(Outcome::parse(&outcome.encode()).unwrap(), outcome);

        let response = format_response(&Ok(outcome.encode()));
        let message = parse_response(&response).unwrap();
        assert_eq!(Outcome::parse(&message).unwrap(), outcome);
    }

    #[test]
    fn what_the_manager_did_is_what_travels() {
        use crate::state::Application;

        let application = Application {
            netcfg: "/scheme/netcfg".to_owned(),
            config: PathBuf::from("/etc/networkd"),
            writes: vec!["write /scheme/netcfg/route/add default via 10.0.0.1".to_owned()],
            changed: true,
        };
        assert_eq!(
            Outcome::of(&application).encode(),
            "applied\t1\t/scheme/netcfg\t/etc/networkd"
        );

        let application = Application {
            writes: Vec::new(),
            changed: false,
            ..application
        };
        assert_eq!(
            Outcome::of(&application).encode(),
            "unchanged\t/scheme/netcfg\t/etc/networkd"
        );
    }

    #[test]
    fn a_malformed_outcome_is_not_an_answer() {
        for message in [
            "",
            "applied",
            "applied\t3",
            "applied\t3\t/scheme/netcfg",
            "applied\tbanana\t/scheme/netcfg\t/etc/networkd",
            "applied\t3\t\t/etc/networkd",
            "applied\t3\t/scheme/netcfg\t/etc/networkd\textra",
            "unchanged",
            "unchanged\t/scheme/netcfg",
            "yes\t/scheme/netcfg\t/etc/networkd",
        ] {
            assert!(Outcome::parse(message).is_err(), "{message:?} was accepted");
        }
    }

    #[test]
    fn responses_are_a_single_verdict() {
        assert_eq!(
            format_response(&Ok("unchanged\t/scheme/netcfg\t/etc/networkd".to_owned())),
            "ok unchanged\t/scheme/netcfg\t/etc/networkd\n"
        );
        assert_eq!(format_response(&Ok(String::new())), "ok\n");
        assert_eq!(
            format_response(&Err(ProtocolError("no stack".to_owned()))),
            "err no stack\n"
        );

        assert_eq!(parse_response("ok\n").unwrap(), "");
        assert_eq!(parse_response("err no stack\n").unwrap_err().0, "no stack");
        assert!(parse_response("").is_err());
        assert!(parse_response("whatever\n").is_err());
    }

    #[test]
    fn the_dhcp_question_names_the_interface_it_is_about() {
        let request = Request::Dhcp {
            iface: "eth0".to_owned(),
        };
        assert_eq!(request.verb(), "dhcp");
        assert_eq!(request.encode(), "dhcp eth0\n");
        assert_eq!(Request::parse(&request.encode()).unwrap(), request);
        assert_eq!(
            Request::parse("  dhcp wlan0 \n").unwrap(),
            Request::Dhcp {
                iface: "wlan0".to_owned()
            }
        );
    }

    #[test]
    fn a_dhcp_question_about_a_file_is_not_a_question_about_an_interface() {
        // The name is used as a path element of the stack, so a name that
        // could walk out of it is not a name.
        assert!(Request::parse("dhcp").unwrap_err().0.contains("interface"));
        for line in [
            "dhcp eth0 wlan0",
            "dhcp ..",
            "dhcp .",
            "dhcp ../eth0",
            "dhcp a/b",
            "dhcp\t",
        ] {
            assert!(Request::parse(line).is_err(), "{line:?} was accepted");
        }
    }

    #[test]
    fn a_decision_says_what_the_configuration_decided() {
        let decision = Decision {
            iface: "eth0".to_owned(),
            source: Some(PathBuf::from("/etc/networkd/10-eth0.network")),
            mode: Mode::Static,
        };
        assert_eq!(
            decision.encode(),
            "dhcp\teth0\t/etc/networkd/10-eth0.network\tstatic"
        );
        assert_eq!(Decision::parse(&decision.encode()).unwrap(), decision);
        assert!(decision.about("eth0"));
        assert!(!decision.about("wlan0"));
        assert_eq!(decision.mode(), Mode::Static);
        assert_eq!(
            decision.source(),
            Some(Path::new("/etc/networkd/10-eth0.network"))
        );

        // A decision no file made has no source, and the field stays where it
        // is: "a file said DHCP" and "no file matched" are different answers,
        // and a field at the end of the line would be trimmed away.
        let decision = Decision {
            iface: "wlan0".to_owned(),
            source: None,
            mode: Mode::Dhcp,
        };
        assert_eq!(decision.encode(), "dhcp\twlan0\t\tdhcp");
        assert_eq!(Decision::parse(&decision.encode()).unwrap(), decision);
        assert_eq!(decision.source(), None);

        // The decision travels inside a response, which is how a client reads
        // it, and a path with a space in it survives the trip.
        let decision = Decision {
            iface: "eth0".to_owned(),
            source: Some(PathBuf::from("/etc/network files/10-eth0.network")),
            mode: Mode::Static,
        };
        let response = format_response(&Ok(decision.encode()));
        let message = parse_response(&response).unwrap();
        assert_eq!(Decision::parse(&message).unwrap(), decision);
    }

    #[test]
    fn a_malformed_decision_is_not_an_answer() {
        for message in [
            "",
            "dhcp",
            "dhcp\teth0",
            "dhcp\teth0\t/etc/networkd/10-eth0.network",
            "dhcp\teth0\t/etc/networkd/10-eth0.network\tbanana",
            "dhcp\teth0\t/etc/networkd/10-eth0.network\tstatic\textra",
            "dhcp\t\t/etc/networkd/10-eth0.network\tstatic",
            "dhcp\t../eth0\t\tstatic",
            "applied\t3\t/scheme/netcfg\t/etc/networkd",
        ] {
            assert!(
                Decision::parse(message).is_err(),
                "{message:?} was accepted"
            );
        }
    }
}
