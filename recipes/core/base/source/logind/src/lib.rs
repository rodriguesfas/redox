//! Session and seat management for AIOS/Redox: the login manager.
//!
//! `logind` answers "who is logged in, on which terminal, and who owns the
//! console". The parts that are pure logic — the session state machine, the
//! `utmp` record format, the passwd lookup and the `loginctl` output — live
//! here so they can be tested on the host:
//!
//! - [`utmp`]: the standard 384-byte login record and the file holding them
//! - [`session`]: sessions, seats and the transitions between them
//! - [`passwd`]: resolving a login name to its numeric user id
//! - [`cli`] and [`render`]: the `loginctl` command line and its output

#![deny(clippy::precedence)]

pub mod cli;
pub mod passwd;
pub mod render;
pub mod session;
pub mod utmp;

pub use cli::{parse_args, parse_env_args, Cli, CliError, Command, ENV_UTMP, USAGE};
pub use passwd::{lookup_uid, parse_uid, PASSWD_PATH};
pub use render::{
    render_seat_status, render_seats, render_session_status, render_sessions, render_show_seat,
    render_show_session, unknown_target,
};
pub use session::{LoginRequest, Seat, Session, SessionState, SessionType, Sessions, DEFAULT_SEAT};
pub use utmp::{RecordType, UtmpError, UtmpFile, UtmpRecord, DEFAULT_PATH, RECORD_SIZE};
