//! Session and seat management for AIOS/Redox: the login manager.
//!
//! `logind` answers "who is logged in, on which terminal, and who owns the
//! console". Most of it is pure logic that is tested on the host:
//!
//! - [`utmp`]: the standard 384-byte login record and the file holding them
//! - [`session`]: sessions, seats and the transitions between them
//! - [`passwd`]: resolving a login name to its numeric user id
//! - [`protocol`]: the request grammar of the daemon
//! - [`registry`]: the live state, and what a request means for the system
//! - [`client`]: what `loginctl` uses to talk to the daemon
//! - [`cli`] and [`render`]: the `loginctl` command line and its output
//!
//! The daemon itself is [`scheme`]: it serves the state at `/scheme/logind` and
//! keeps `/var/run/utmp` up to date, so tools that know `utmp` — and `loginctl`
//! itself, when the daemon is not up yet — see the same sessions.

#![deny(clippy::precedence)]

pub mod cli;
pub mod client;
pub mod passwd;
pub mod protocol;
pub mod registry;
pub mod render;
pub mod session;
pub mod utmp;

#[cfg(target_os = "redox")]
pub mod scheme;

pub use cli::{parse_args, parse_env_args, Cli, CliError, Command, ENV_UTMP, USAGE};
pub use client::{Client, Origin, DEFAULT_SCHEME, ENV_SCHEME};
pub use passwd::{lookup_uid, parse_uid, PASSWD_PATH};
pub use protocol::{encode_seats, format_response, parse_response, ProtocolError, Request};
pub use registry::{Outcome, Registry};
pub use render::{
    render_seat_status, render_seats, render_session_status, render_sessions, render_show_seat,
    render_show_session, unknown_target,
};
pub use session::{
    LoginRequest, RecordError, Seat, Session, SessionState, SessionType, Sessions, DEFAULT_SEAT,
    RECORD_FIELDS,
};
pub use utmp::{RecordType, UtmpError, UtmpFile, UtmpRecord, DEFAULT_PATH, RECORD_SIZE};
