//! Sessions and seats: the model behind `loginctl`.
//!
//! A *seat* is the set of hardware a single user is sitting in front of: one
//! keyboard, one mouse, one screen. Sessions belong to a seat, and a seat has at
//! most one *active* session — the one receiving input. `logind` is what turns
//! that state into something the rest of the system can query, and it is what
//! decides which session gets the console when two people are logged in.
//!
//! The registry is pure logic with no I/O, so the whole state machine is tested
//! on the host; the daemon and the `utmp` file are thin layers on top of it.

use std::collections::BTreeMap;
use std::fmt;

use crate::utmp::{UtmpRecord, DEFAULT_PATH};

/// Default seat, used for local terminals.
pub const DEFAULT_SEAT: &str = "seat0";

/// How a session is connected.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Hash)]
pub enum SessionType {
    /// A local terminal.
    #[default]
    Tty,
    /// A graphical session on a seat.
    Graphical,
    /// A session with no seat, such as a remote login.
    Other,
}

impl SessionType {
    /// The name used by `loginctl`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tty => "tty",
            Self::Graphical => "x11",
            Self::Other => "unspecified",
        }
    }

    /// Parses the name used by `loginctl`.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "tty" => Self::Tty,
            "x11" | "wayland" | "mir" => Self::Graphical,
            "unspecified" | "other" => Self::Other,
            _ => return None,
        })
    }
}

impl fmt::Display for SessionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Where a session is in its life cycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Hash)]
pub enum SessionState {
    /// Logged in, but not the active session of its seat.
    #[default]
    Online,
    /// The session receiving input on its seat.
    Active,
    /// Ending: the leader is gone and the session is being cleaned up.
    Closing,
}

impl SessionState {
    /// The name used by `loginctl`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Active => "active",
            Self::Closing => "closing",
        }
    }

    /// Parses the name used by `loginctl`.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "online" => Self::Online,
            "active" => Self::Active,
            "closing" => Self::Closing,
            _ => return None,
        })
    }

    /// Whether the session is active.
    pub fn is_active(self) -> bool {
        self == Self::Active
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One login session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// Identifier, `1`, `2`, … as in `loginctl`.
    pub id: String,
    /// Numeric user id.
    pub uid: u32,
    /// Login name.
    pub user: String,
    /// Seat this session belongs to, empty when it has none.
    pub seat: String,
    /// Terminal name, without `/dev/`.
    pub tty: String,
    /// How the session is connected.
    pub session_type: SessionType,
    /// Life cycle state.
    pub state: SessionState,
    /// Pid of the session leader.
    pub leader: i32,
    /// Seconds since the epoch when the session started.
    pub since: i64,
    /// Remote host, empty for local logins.
    pub host: String,
}

impl Session {
    /// Whether this is the active session of its seat.
    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// The `USER_PROCESS` record describing this session.
    pub fn to_utmp(&self) -> UtmpRecord {
        let mut record = UtmpRecord::user_process(self.leader, &self.tty, &self.user, self.since);
        record.host = self.host.clone();
        record
    }

    /// Rebuilds a session from a `utmp` record.
    ///
    /// The file has no notion of seats or states, so the seat is derived from
    /// the terminal and the state is filled in by [`Sessions::from_utmp`].
    pub fn from_utmp(record: &UtmpRecord, index: usize) -> Self {
        Self {
            id: (index + 1).to_string(),
            uid: 0,
            user: record.user.clone(),
            seat: seat_for_line(&record.line),
            tty: record.line.clone(),
            session_type: SessionType::Tty,
            state: SessionState::Online,
            leader: record.pid,
            since: record.time,
            host: record.host.clone(),
        }
    }

    /// The one-line description used by `loginctl list-sessions`.
    pub fn to_row(&self) -> String {
        format!(
            "{:<8} {:<10} {:<10} {:<8} {}",
            self.id, self.uid, self.user, self.seat, self.tty
        )
    }
}

/// What the caller knows about a login that is starting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginRequest {
    /// Login name.
    pub user: String,
    /// Numeric user id.
    pub uid: u32,
    /// Terminal the session is running on.
    pub tty: String,
    /// Pid of the session leader.
    pub leader: i32,
    /// Remote host, when the login is not local.
    pub host: String,
    /// How the session is connected.
    pub session_type: SessionType,
    /// Seat to place the session on; derived from the terminal when absent.
    pub seat: Option<String>,
    /// Seconds since the epoch.
    pub now: i64,
}

impl LoginRequest {
    /// A local terminal login.
    pub fn tty(
        user: impl Into<String>,
        uid: u32,
        tty: impl Into<String>,
        leader: i32,
        now: i64,
    ) -> Self {
        Self {
            user: user.into(),
            uid,
            tty: tty.into(),
            leader,
            host: String::new(),
            session_type: SessionType::Tty,
            seat: None,
            now,
        }
    }

    /// The seat this request lands on.
    pub fn seat(&self) -> String {
        self.seat
            .clone()
            .unwrap_or_else(|| seat_for_line(&self.tty))
    }
}

/// A seat: the hardware one user is sitting in front of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seat {
    /// Seat name, `seat0` for the local console.
    pub id: String,
    /// Active session, if any.
    pub active: Option<String>,
    /// Sessions on this seat, in the order they were created.
    pub sessions: Vec<String>,
}

impl Seat {
    /// How many sessions the seat has.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Whether the seat has no sessions.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

/// Every session on the system.
#[derive(Clone, Debug, Default)]
pub struct Sessions {
    sessions: Vec<Session>,
    next_id: u64,
}

impl Sessions {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            next_id: 0,
        }
    }

    /// Whether there is no session.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// How many sessions there are.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Every session, oldest first.
    pub fn list(&self) -> &[Session] {
        &self.sessions
    }

    /// Looks a session up by identifier.
    pub fn session(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }

    /// Looks a session up by terminal.
    pub fn session_for_tty(&self, tty: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.tty == tty)
    }

    /// Registers a new session and returns its identifier.
    ///
    /// A session becomes active when its seat has no active session yet: the
    /// first login on a seat owns the console, later ones wait until activated.
    pub fn login(&mut self, request: &LoginRequest) -> String {
        self.next_id += 1;
        let id = self.next_id.to_string();
        let seat = request.seat();

        let seat_has_active = self
            .sessions
            .iter()
            .any(|session| session.seat == seat && session.is_active());

        self.sessions.push(Session {
            id: id.clone(),
            uid: request.uid,
            user: request.user.clone(),
            seat,
            tty: request.tty.clone(),
            session_type: request.session_type,
            state: if seat_has_active {
                SessionState::Online
            } else {
                SessionState::Active
            },
            leader: request.leader,
            since: request.now,
            host: request.host.clone(),
        });

        id
    }

    /// Removes a session, returning it.
    ///
    /// When the removed session was active, the newest remaining session on the
    /// seat takes over, so a seat is never left without an active session while
    /// somebody is still logged in.
    pub fn logout(&mut self, id: &str) -> Option<Session> {
        let index = self.sessions.iter().position(|session| session.id == id)?;
        let removed = self.sessions.remove(index);

        if removed.is_active() {
            let seat = removed.seat.clone();
            if let Some(next) = self
                .sessions
                .iter_mut()
                .rfind(|session| session.seat == seat)
            {
                next.state = SessionState::Active;
            }
        }

        Some(removed)
    }

    /// Makes a session the active one of its seat.
    ///
    /// Returns whether the session exists. Sessions on other seats are left
    /// alone: each seat has its own active session.
    pub fn activate(&mut self, id: &str) -> bool {
        let Some(seat) = self
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.seat.clone())
        else {
            return false;
        };

        for session in self.sessions.iter_mut() {
            if session.seat == seat {
                session.state = if session.id == id {
                    SessionState::Active
                } else {
                    SessionState::Online
                };
            }
        }
        true
    }

    /// Marks a session as ending, keeping it visible until it disappears.
    pub fn set_closing(&mut self, id: &str) -> bool {
        match self.session_mut(id) {
            Some(session) => {
                session.state = SessionState::Closing;
                true
            }
            None => false,
        }
    }

    /// The active session of a seat.
    pub fn active_on(&self, seat: &str) -> Option<&Session> {
        self.sessions
            .iter()
            .find(|session| session.seat == seat && session.is_active())
    }

    /// The seats, in name order, with their active session filled in.
    pub fn seats(&self) -> Vec<Seat> {
        let mut seats: BTreeMap<String, Seat> = BTreeMap::new();
        for session in &self.sessions {
            let seat = seats.entry(session.seat.clone()).or_insert_with(|| Seat {
                id: session.seat.clone(),
                ..Seat::default()
            });
            seat.sessions.push(session.id.clone());
            if session.is_active() {
                seat.active = Some(session.id.clone());
            }
        }
        seats.into_values().collect()
    }

    /// Looks a seat up by name.
    pub fn seat(&self, id: &str) -> Option<Seat> {
        self.seats().into_iter().find(|seat| seat.id == id)
    }

    /// Rebuilds a registry from the records of a `utmp` file.
    ///
    /// Only logged-in users are considered, and the state is inferred: the first
    /// session on a seat owns it, because the file records no seat switch. The
    /// daemon keeps the exact state; this is what makes `loginctl` usable when
    /// it reads the file directly.
    pub fn from_utmp(records: &[UtmpRecord]) -> Self {
        let mut sessions = Self::new();
        for record in records.iter().filter(|record| record.is_user()) {
            let session = Session::from_utmp(record, sessions.sessions.len());
            sessions.sessions.push(session);
        }
        sessions.next_id = sessions.sessions.len() as u64;

        // The file records no seat switches, so the first session of each seat
        // is taken to own it — the same rule the live registry starts from.
        let mut seen: Vec<String> = Vec::new();
        for session in sessions.sessions.iter_mut() {
            if seen.contains(&session.seat) {
                continue;
            }
            session.state = SessionState::Active;
            seen.push(session.seat.clone());
        }

        sessions
    }

    fn session_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    /// Fills in numeric user ids.
    ///
    /// `utmp` stores login names only, so a registry rebuilt from the file has
    /// to resolve names again; the daemon already knows the ids.
    pub fn assign_uids(&mut self, lookup: impl Fn(&str) -> Option<u32>) {
        for session in self.sessions.iter_mut() {
            if let Some(uid) = lookup(&session.user) {
                session.uid = uid;
            }
        }
    }
}

/// The seat a terminal belongs to.
///
/// The local console is one seat: every terminal a user can physically reach
/// belongs to it, and a login with no terminal belongs to none.
pub fn seat_for_line(line: &str) -> String {
    if line.is_empty() {
        String::new()
    } else {
        DEFAULT_SEAT.to_owned()
    }
}

/// The default file the registry is kept in.
pub fn default_utmp_path() -> &'static str {
    DEFAULT_PATH
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(sessions: &mut Sessions, user: &str, tty: &str, now: i64) -> String {
        sessions.login(&LoginRequest::tty(user, 1000, tty, 42, now))
    }

    #[test]
    fn the_first_session_on_a_seat_is_active() {
        let mut sessions = Sessions::new();
        let first = login(&mut sessions, "user", "tty1", 100);
        let second = login(&mut sessions, "root", "tty2", 200);

        assert_eq!(first, "1");
        assert_eq!(second, "2");
        assert!(sessions.session("1").unwrap().is_active());
        assert!(!sessions.session("2").unwrap().is_active());
        assert_eq!(sessions.active_on(DEFAULT_SEAT).unwrap().id, "1");
        assert_eq!(sessions.session("1").unwrap().seat, DEFAULT_SEAT);
    }

    #[test]
    fn activating_moves_the_console_between_sessions_of_a_seat() {
        let mut sessions = Sessions::new();
        login(&mut sessions, "user", "tty1", 100);
        login(&mut sessions, "root", "tty2", 200);

        assert!(sessions.activate("2"));
        assert!(!sessions.session("1").unwrap().is_active());
        assert!(sessions.session("2").unwrap().is_active());
        assert_eq!(sessions.active_on(DEFAULT_SEAT).unwrap().id, "2");

        // Activating an unknown session changes nothing.
        assert!(!sessions.activate("99"));
        assert!(sessions.session("2").unwrap().is_active());
        assert!(!sessions.session("1").unwrap().is_active());
    }

    #[test]
    fn a_seat_without_an_active_session_is_filled_on_logout() {
        let mut sessions = Sessions::new();
        login(&mut sessions, "user", "tty1", 100);
        login(&mut sessions, "root", "tty2", 200);
        assert!(sessions.activate("2"));

        // The active session goes away: the other one takes the seat.
        let removed = sessions.logout("2").unwrap();
        assert_eq!(removed.id, "2");
        assert_eq!(sessions.len(), 1);
        assert!(sessions.session("1").unwrap().is_active());
        assert_eq!(sessions.active_on(DEFAULT_SEAT).unwrap().id, "1");

        // Logging out the last session leaves the seat empty.
        sessions.logout("1").unwrap();
        assert!(sessions.is_empty());
        assert!(sessions.seats().is_empty());
    }

    #[test]
    fn logging_out_an_unknown_session_is_not_an_error() {
        let mut sessions = Sessions::new();
        login(&mut sessions, "user", "tty1", 100);
        assert!(sessions.logout("42").is_none());
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn sealing_a_session_marks_it_closing() {
        let mut sessions = Sessions::new();
        login(&mut sessions, "user", "tty1", 100);
        assert!(sessions.set_closing("1"));
        assert_eq!(sessions.session("1").unwrap().state, SessionState::Closing);
        assert!(!sessions.set_closing("2"));
    }

    #[test]
    fn a_session_without_a_terminal_has_no_seat() {
        let mut sessions = Sessions::new();
        let request = LoginRequest {
            session_type: SessionType::Other,
            host: "example.org".to_owned(),
            ..LoginRequest::tty("user", 1000, "", 7, 100)
        };
        sessions.login(&request);

        let session = &sessions.list()[0];
        assert_eq!(session.seat, "");
        assert_eq!(session.session_type, SessionType::Other);
        assert_eq!(session.host, "example.org");
        // A seatless session is still tracked, just not on a seat.
        assert!(session.is_active());
    }

    #[test]
    fn seats_group_their_sessions_and_name_their_active_one() {
        let mut sessions = Sessions::new();
        login(&mut sessions, "user", "tty1", 100);
        login(&mut sessions, "root", "tty2", 200);
        sessions.login(&LoginRequest {
            session_type: SessionType::Graphical,
            seat: Some("seat1".to_owned()),
            ..LoginRequest::tty("guest", 1001, "display0", 30, 300)
        });

        let seats = sessions.seats();
        assert_eq!(seats.len(), 2);
        assert_eq!(seats[0].id, "seat0");
        assert_eq!(seats[0].sessions, vec!["1", "2"]);
        assert_eq!(seats[0].active.as_deref(), Some("1"));
        assert_eq!(seats[1].id, "seat1");
        assert_eq!(seats[1].sessions, vec!["3"]);
        assert_eq!(seats[1].active.as_deref(), Some("3"));
        assert_eq!(seats[1].len(), 1);

        // Each seat keeps its own active session.
        assert!(sessions.activate("2"));
        assert!(sessions.session("3").unwrap().is_active());
    }

    #[test]
    fn a_session_round_trips_through_a_utmp_record() {
        let mut sessions = Sessions::new();
        sessions.login(&LoginRequest {
            host: "example.org".to_owned(),
            ..LoginRequest::tty("user", 1000, "tty7", 987, 1_792_022_400)
        });

        let record = sessions.list()[0].to_utmp();
        assert_eq!(record.record_type, crate::utmp::RecordType::UserProcess);
        assert_eq!(record.line, "tty7");
        assert_eq!(record.user, "user");
        assert_eq!(record.pid, 987);
        assert_eq!(record.time, 1_792_022_400);
        assert_eq!(record.host, "example.org");

        let restored = Sessions::from_utmp(&[record]);
        assert_eq!(restored.len(), 1);
        let session = &restored.list()[0];
        assert_eq!(session.user, "user");
        assert_eq!(session.tty, "tty7");
        assert_eq!(session.seat, DEFAULT_SEAT);
        assert!(session.is_active());
    }

    #[test]
    fn from_utmp_ignores_non_login_records_and_shares_seats() {
        let records = vec![
            UtmpRecord::boot_time(1, 10),
            UtmpRecord::user_process(10, "tty1", "user", 100),
            UtmpRecord::dead_process("tty3", 200),
            UtmpRecord::user_process(20, "tty2", "root", 300),
        ];

        let sessions = Sessions::from_utmp(&records);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.list()[0].user, "user");
        assert_eq!(sessions.list()[1].user, "root");
        // Only one session per seat is active.
        assert!(sessions.list()[0].is_active());
        assert!(!sessions.list()[1].is_active());
        assert_eq!(sessions.active_on(DEFAULT_SEAT).unwrap().user, "user");

        // Identifiers keep counting after a rebuild.
        let mut moved = sessions.clone();
        let id = login(&mut moved, "guest", "tty9", 400);
        assert_eq!(id, "3");
    }

    #[test]
    fn session_types_and_states_round_trip_their_names() {
        for session_type in [SessionType::Tty, SessionType::Graphical, SessionType::Other] {
            assert_eq!(
                SessionType::parse(session_type.as_str()),
                Some(session_type)
            );
        }
        assert_eq!(SessionType::parse("wayland"), Some(SessionType::Graphical));
        assert_eq!(SessionType::parse("bogus"), None);

        for state in [
            SessionState::Online,
            SessionState::Active,
            SessionState::Closing,
        ] {
            assert_eq!(SessionState::parse(state.as_str()), Some(state));
        }
        assert_eq!(SessionState::parse("bogus"), None);
    }

    #[test]
    fn terminals_belong_to_the_console_seat() {
        assert_eq!(seat_for_line("tty1"), DEFAULT_SEAT);
        assert_eq!(seat_for_line("pty/3"), DEFAULT_SEAT);
        assert_eq!(seat_for_line(""), "");
        assert_eq!(default_utmp_path(), "/var/run/utmp");
    }

    #[test]
    fn rows_line_up_for_list_sessions() {
        let mut sessions = Sessions::new();
        login(&mut sessions, "user", "tty1", 100);
        let row = sessions.list()[0].to_row();
        assert_eq!(row, "1        1000       user       seat0    tty1");
    }
}
