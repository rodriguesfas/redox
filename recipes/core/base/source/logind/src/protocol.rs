//! The request protocol spoken over `/scheme/logind`.
//!
//! Control is "write a request, read the response": a client opens one of the
//! control files of the scheme (`login`, `logout`, `activate`, `terminate`),
//! writes a single request line and reads the single response line back from
//! the same file descriptor. The grammar is text so that the login path can
//! report a session with plain file I/O: `login` runs long before anything
//! would be linked against this crate.
//!
//! ```text
//! login <user> <uid> <tty> <pid> [seat]
//! logout <session|tty>
//! activate <session|tty>
//! terminate <session|tty>
//! ```
//!
//! A response is `ok [message]` or `err <message>`.
//!
//! Reads do not need a request: the scheme serves the live sessions as records
//! (see [`crate::session::Sessions::encode`]) and the seats as tab separated
//! lines ([`encode_seats`]).

use std::fmt;
use std::str::FromStr;

use crate::session::Seat;

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
    /// A login is starting and owns a new session.
    Login {
        /// Login name.
        user: String,
        /// Numeric user id, known by whoever authenticated the login.
        uid: u32,
        /// Terminal the session runs on.
        tty: String,
        /// Pid of the session leader.
        leader: i32,
        /// Seat to place the session on, when it is not the default one.
        seat: Option<String>,
    },
    /// A session is over.
    Logout {
        /// Session identifier or terminal name.
        target: String,
    },
    /// A session takes over the console of its seat.
    Activate {
        /// Session identifier or terminal name.
        target: String,
    },
    /// A session has to be stopped.
    Terminate {
        /// Session identifier or terminal name.
        target: String,
    },
}

impl Request {
    /// The verb of the request, which is also the file name in the scheme.
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Login { .. } => "login",
            Self::Logout { .. } => "logout",
            Self::Activate { .. } => "activate",
            Self::Terminate { .. } => "terminate",
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
        let args: Vec<&str> = rest.split_whitespace().collect();

        match verb {
            "login" => {
                if args.len() != 4 && args.len() != 5 {
                    return Err(ProtocolError(format!(
                        "login takes <user> <uid> <tty> <pid> [seat], got {} arguments",
                        args.len()
                    )));
                }
                Ok(Self::Login {
                    user: args[0].to_owned(),
                    uid: parse_field(args[1], "uid")?,
                    tty: args[2].to_owned(),
                    leader: parse_field(args[3], "pid")?,
                    seat: args.get(4).map(|seat| (*seat).to_owned()),
                })
            }
            "logout" | "activate" | "terminate" => {
                if args.len() != 1 {
                    return Err(ProtocolError(format!(
                        "{verb} takes one session or terminal, got {} arguments",
                        args.len()
                    )));
                }
                let target = args[0].to_owned();
                Ok(match verb {
                    "logout" => Self::Logout { target },
                    "activate" => Self::Activate { target },
                    _ => Self::Terminate { target },
                })
            }
            _ => Err(ProtocolError(format!("unknown request {verb:?}"))),
        }
    }

    /// Renders the request the way a client writes it.
    pub fn encode(&self) -> String {
        match self {
            Self::Login {
                user,
                uid,
                tty,
                leader,
                seat,
            } => match seat {
                Some(seat) => format!("login {user} {uid} {tty} {leader} {seat}\n"),
                None => format!("login {user} {uid} {tty} {leader}\n"),
            },
            Self::Logout { target } => format!("logout {target}\n"),
            Self::Activate { target } => format!("activate {target}\n"),
            Self::Terminate { target } => format!("terminate {target}\n"),
        }
    }
}

fn parse_field<T: FromStr>(value: &str, name: &str) -> Result<T, ProtocolError>
where
    T::Err: fmt::Display,
{
    value
        .parse()
        .map_err(|err| ProtocolError(format!("invalid {name} {value:?}: {err}")))
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
            "no response from the login manager".to_owned(),
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

/// Renders the seats as tab separated lines:
/// `seat`, `active session` (empty when the seat has none), `sessions`.
pub fn encode_seats(seats: &[Seat]) -> String {
    let mut out = String::new();
    for seat in seats {
        out.push_str(&format!(
            "{}\t{}\t{}\n",
            seat.id,
            seat.active.clone().unwrap_or_default(),
            seat.sessions.join(" ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_carries_the_session_leader() {
        let request = Request::parse("login user 1000 tty1 42").unwrap();
        assert_eq!(
            request,
            Request::Login {
                user: "user".to_owned(),
                uid: 1000,
                tty: "tty1".to_owned(),
                leader: 42,
                seat: None,
            }
        );
        assert_eq!(request.verb(), "login");
        assert_eq!(request.encode(), "login user 1000 tty1 42\n");

        // A seat is optional, and extra blanks are ignored.
        let request = Request::parse("  login   root 0 tty2 7 seat1  \n").unwrap();
        assert_eq!(
            request,
            Request::Login {
                user: "root".to_owned(),
                uid: 0,
                tty: "tty2".to_owned(),
                leader: 7,
                seat: Some("seat1".to_owned()),
            }
        );
        assert_eq!(request.encode(), "login root 0 tty2 7 seat1\n");
    }

    #[test]
    fn targets_address_a_session_or_a_terminal() {
        assert_eq!(
            Request::parse("logout 1").unwrap(),
            Request::Logout {
                target: "1".to_owned()
            }
        );
        assert_eq!(
            Request::parse("logout tty1").unwrap().verb(),
            "logout",
            "a terminal is as good an address as an identifier"
        );
        assert_eq!(
            Request::parse("activate 2\n").unwrap(),
            Request::Activate {
                target: "2".to_owned()
            }
        );
        assert_eq!(
            Request::parse("terminate tty3").unwrap(),
            Request::Terminate {
                target: "tty3".to_owned()
            }
        );
        assert_eq!(
            Request::parse("activate 2").unwrap().encode(),
            "activate 2\n"
        );
    }

    #[test]
    fn malformed_requests_are_rejected() {
        for line in [
            "",
            "   ",
            "# a comment",
            "reboot",
            "login",
            "login user 1000 tty1",
            "login user notanumber tty1 42",
            "login user 1000 tty1 notanumber",
            "login user 1000 tty1 42 seat extra",
            "logout",
            "logout 1 2",
        ] {
            let error = Request::parse(line).unwrap_err();
            assert!(!error.0.is_empty(), "{line:?} was accepted");
        }

        let error = Request::parse("login user notanumber tty1 42").unwrap_err();
        assert!(error.0.contains("uid"), "{error}");
    }

    #[test]
    fn responses_are_a_single_verdict() {
        assert_eq!(format_response(&Ok("3".to_owned())), "ok 3\n");
        assert_eq!(format_response(&Ok(String::new())), "ok\n");
        assert_eq!(
            format_response(&Err(ProtocolError("no such session".to_owned()))),
            "err no such session\n"
        );

        assert_eq!(parse_response("ok 3\n").unwrap(), "3");
        assert_eq!(parse_response("ok\n").unwrap(), "");
        assert_eq!(parse_response("  ok  \n").unwrap(), "");
        assert_eq!(
            parse_response("err no such session\n").unwrap_err().0,
            "no such session"
        );
        assert!(parse_response("").is_err());
        assert!(parse_response("whatever\n").is_err());

        // The response survives a round trip, both ways.
        let response = format_response(&Ok("tty1".to_owned()));
        assert_eq!(parse_response(&response).unwrap(), "tty1");
    }

    #[test]
    fn seats_are_records_too() {
        let seats = vec![
            Seat {
                id: "seat0".to_owned(),
                active: Some("1".to_owned()),
                sessions: vec!["1".to_owned(), "2".to_owned()],
            },
            Seat {
                id: "seat1".to_owned(),
                active: None,
                sessions: Vec::new(),
            },
        ];
        let encoded = encode_seats(&seats);
        assert_eq!(encoded, "seat0\t1\t1 2\nseat1\t\t\n");
        assert_eq!(encode_seats(&[]), "");
    }
}
