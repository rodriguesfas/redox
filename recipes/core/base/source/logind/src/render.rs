//! Output for `loginctl`.
//!
//! The shapes follow `loginctl` closely enough to be familiar: aligned tables
//! for listings, a short block for `session-status`, and `key=value` lines for
//! `show-*`, which is what scripts parse.

use journald::time::format_timestamp;

use crate::session::{Seat, Session, Sessions};

const SESSION_COLUMNS: [&str; 5] = ["SESSION", "UID", "USER", "SEAT", "TTY"];
const SEAT_COLUMNS: [&str; 3] = ["SEAT", "ACTIVE", "SESSIONS"];

/// Renders a row of aligned columns, right-aligning the given fields.
fn row(values: &[String], widths: &[usize], right: &[usize]) -> String {
    let mut line = String::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            line.push(' ');
        }
        let width = widths[index];
        if right.contains(&index) {
            line.push_str(&format!("{value:>width$}"));
        } else {
            line.push_str(&format!("{value:<width$}"));
        }
    }
    line.trim_end().to_owned()
}

fn widths(columns: &[&str], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = columns.iter().map(|column| column.len()).collect();
    for values in rows {
        for (index, value) in values.iter().enumerate() {
            if index < widths.len() {
                widths[index] = widths[index].max(value.chars().count());
            }
        }
    }
    widths
}

/// `loginctl list-sessions`: an aligned table of sessions.
pub fn render_sessions(sessions: &[Session], no_legend: bool) -> String {
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .map(|session| {
            vec![
                session.id.clone(),
                session.uid.to_string(),
                session.user.clone(),
                session.seat.clone(),
                session.tty.clone(),
            ]
        })
        .collect();

    let widths = widths(&SESSION_COLUMNS, &rows);
    let mut out = String::new();
    if !no_legend {
        let header: Vec<String> = SESSION_COLUMNS
            .iter()
            .map(|column| (*column).to_owned())
            .collect();
        out.push_str(&row(&header, &widths, &[0, 1]));
        out.push('\n');
    }
    for values in rows {
        out.push_str(&row(&values, &widths, &[0, 1]));
        out.push('\n');
    }
    out
}

/// `loginctl list-seats`: an aligned table of seats.
pub fn render_seats(seats: &[Seat], no_legend: bool) -> String {
    let rows: Vec<Vec<String>> = seats
        .iter()
        .map(|seat| {
            vec![
                seat.id.clone(),
                seat.active.clone().unwrap_or_default(),
                seat.sessions.join(" "),
            ]
        })
        .collect();

    let widths = widths(&SEAT_COLUMNS, &rows);
    let mut out = String::new();
    if !no_legend {
        let header: Vec<String> = SEAT_COLUMNS
            .iter()
            .map(|column| (*column).to_owned())
            .collect();
        out.push_str(&row(&header, &widths, &[]));
        out.push('\n');
    }
    for values in rows {
        out.push_str(&row(&values, &widths, &[]));
        out.push('\n');
    }
    out
}

/// `loginctl session-status`: the terse human-readable block.
pub fn render_session_status(session: &Session) -> String {
    let mut out = format!("{} - {} ({})\n", session.id, session.user, session.uid);
    let fields: [(&str, String); 7] = [
        ("Since", format_timestamp(session.since)),
        ("State", session.state.to_string()),
        ("Seat", session.seat.clone()),
        ("TTY", session.tty.clone()),
        ("Type", session.session_type.to_string()),
        ("Leader", session.leader.to_string()),
        ("Host", session.host.clone()),
    ];
    for (name, value) in fields {
        if value.is_empty() {
            continue;
        }
        out.push_str(&format!("{:>14}: {value}\n", name));
    }
    out
}

/// `loginctl seat-status`: the terse human-readable block.
///
/// The seat name on its own, then the tree of sessions attached to it.
pub fn render_seat_status(seat: &Seat, sessions: &Sessions) -> String {
    let mut out = format!("{}\n", seat.id);

    if seat.sessions.is_empty() {
        return out;
    }

    out.push_str(&format!("{:>14}:\n", "Sessions"));
    let last = seat.sessions.len() - 1;
    for (index, id) in seat.sessions.iter().enumerate() {
        let line = match sessions.session(id) {
            Some(session) => format!("{} {} ({})", session.id, session.user, session.tty),
            None => id.clone(),
        };
        let branch = if index == last { "└─" } else { "├─" };
        out.push_str(&format!("{branch}{line}\n"));
    }
    out
}

/// `loginctl show-session`: `key=value` properties.
pub fn render_show_session(session: &Session) -> String {
    let mut out = String::new();
    for (key, value) in [
        ("Id", session.id.clone()),
        ("User", session.user.clone()),
        ("UID", session.uid.to_string()),
        ("Seat", session.seat.clone()),
        ("TTY", session.tty.clone()),
        ("Type", session.session_type.to_string()),
        ("State", session.state.to_string()),
        ("Leader", session.leader.to_string()),
        ("Since", format_timestamp(session.since)),
        ("Host", session.host.clone()),
        ("Active", yes_no(session.is_active()).to_owned()),
    ] {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

/// `loginctl show-seat`: `key=value` properties.
pub fn render_show_seat(seat: &Seat) -> String {
    let mut out = String::new();
    out.push_str(&format!("Id={}\n", seat.id));
    out.push_str(&format!(
        "ActiveSession={}\n",
        seat.active.clone().unwrap_or_default()
    ));
    out.push_str(&format!("Sessions={}\n", seat.sessions.join(" ")));
    out
}

/// A terse error for a name the login manager does not know.
pub fn unknown_target(command: &str, target: &str) -> String {
    format!("{command}: no such session or seat: {target}")
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{LoginRequest, SessionType};

    fn populated() -> Sessions {
        let mut sessions = Sessions::new();
        sessions.login(&LoginRequest::tty("user", 1000, "tty1", 42, 1_792_022_400));
        sessions.login(&LoginRequest::tty("root", 0, "tty2", 77, 1_792_022_500));
        sessions
    }

    #[test]
    fn session_listing_is_aligned_and_has_a_header() {
        let sessions = populated();
        let rendered = render_sessions(sessions.list(), false);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "SESSION  UID USER SEAT  TTY");
        assert!(lines[1].starts_with("      1 1000 user seat0 tty1"));
        assert!(lines[2].starts_with("      2    0 root seat0 tty2"));

        let rendered = render_sessions(sessions.list(), true);
        assert_eq!(rendered.lines().count(), 2);
        assert!(!rendered.contains("SESSION"));

        assert_eq!(render_sessions(&[], false), "SESSION UID USER SEAT TTY\n");
        assert_eq!(render_sessions(&[], true), "");
    }

    #[test]
    fn seat_listing_shows_the_active_session() {
        let sessions = populated();
        let rendered = render_seats(&sessions.seats(), false);
        assert_eq!(rendered.lines().count(), 2);
        assert_eq!(rendered.lines().next().unwrap(), "SEAT  ACTIVE SESSIONS");
        assert!(rendered.contains("seat0 1      1 2"));

        let rendered = render_seats(&sessions.seats(), true);
        assert!(!rendered.contains("ACTIVE"));
        assert_eq!(rendered.lines().count(), 1);
    }

    #[test]
    fn session_status_has_the_expected_fields() {
        let sessions = populated();
        let rendered = render_session_status(sessions.session("1").unwrap());
        assert!(rendered.starts_with("1 - user (1000)\n"));
        assert!(rendered.contains("Since: 2026-10-15 00:00:00\n"));
        assert!(rendered.contains("State: active\n"));
        assert!(rendered.contains("Seat: seat0\n"));
        assert!(rendered.contains("TTY: tty1\n"));
        assert!(rendered.contains("Type: tty\n"));
        assert!(rendered.contains("Leader: 42\n"));
        // An empty host is omitted rather than shown blank.
        assert!(!rendered.contains("Host:"));
    }

    #[test]
    fn session_status_shows_the_host_for_remote_logins() {
        let mut sessions = Sessions::new();
        sessions.login(&LoginRequest {
            host: "example.org".to_owned(),
            session_type: SessionType::Other,
            ..LoginRequest::tty("user", 1000, "", 9, 1_792_022_400)
        });

        let rendered = super::render_session_status(&sessions.list()[0]);
        assert!(rendered.contains("Host: example.org\n"));
    }

    #[test]
    fn show_session_is_key_value() {
        let sessions = populated();
        let rendered = render_show_session(sessions.session("2").unwrap());
        assert!(rendered.contains("Id=2\n"));
        assert!(rendered.contains("User=root\n"));
        assert!(rendered.contains("UID=0\n"));
        assert!(rendered.contains("Seat=seat0\n"));
        assert!(rendered.contains("TTY=tty2\n"));
        assert!(rendered.contains("State=online\n"));
        assert!(rendered.contains("Leader=77\n"));
        assert!(rendered.contains("Since=2026-10-15 00:01:40\n"));
        assert!(rendered.contains("Active=no\n"));

        // Every line is a pair, which is what a parser expects.
        for line in rendered.lines() {
            assert!(line.contains('='), "not a property: {line}");
        }
    }

    #[test]
    fn show_seat_lists_its_sessions() {
        let sessions = populated();
        let seat = sessions
            .seats()
            .into_iter()
            .find(|seat| seat.id == "seat0")
            .unwrap();
        let rendered = render_show_seat(&seat);
        assert!(rendered.contains("Id=seat0\n"));
        assert!(rendered.contains("ActiveSession=1\n"));
        assert!(rendered.contains("Sessions=1 2\n"));
    }

    #[test]
    fn seat_status_names_the_active_session() {
        let sessions = populated();
        let seat = sessions
            .seats()
            .into_iter()
            .find(|seat| seat.id == "seat0")
            .unwrap();
        let rendered = render_seat_status(&seat, &sessions);
        assert!(rendered.starts_with("seat0\n"));
        assert!(rendered.contains("Sessions:\n├─1 user (tty1)\n└─2 root (tty2)\n"));
        assert_eq!(rendered.lines().count(), 4);

        // An empty seat still names itself.
        let empty = Seat {
            id: "seat1".to_owned(),
            ..Seat::default()
        };
        assert_eq!(render_seat_status(&empty, &sessions), "seat1\n");
    }
}
