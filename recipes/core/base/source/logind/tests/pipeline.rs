//! End-to-end test of the login pipeline: sessions are registered, persisted to
//! a `utmp` file, read back, resolved against the account database, filtered
//! through the `loginctl` argument parser and rendered.

use std::fs;
use std::path::PathBuf;

use logind::cli::parse_args;
use logind::passwd::parse_uid;
use logind::render::{
    render_seat_status, render_seats, render_session_status, render_sessions, render_show_seat,
    render_show_session,
};
use logind::session::{LoginRequest, SessionType, Sessions};
use logind::utmp::{RecordType, UtmpFile, UtmpRecord, RECORD_SIZE};

/// A `passwd` file the tests resolve names against.
const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/ion
user:x:1000:1000:User:/home/user:/bin/ion
guest:x:1001:1001:Guest:/home/guest:/bin/ion
";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("logind-e2e-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_owned()).collect()
}

/// The state `loginctl` reconstructs from a file.
fn restored(path: &std::path::Path) -> Sessions {
    let mut sessions = Sessions::from_utmp(&UtmpFile::new(path).read_all().unwrap());
    sessions.assign_uids(|user| parse_uid(PASSWD, user));
    sessions
}

#[test]
fn logins_reach_the_file_and_loginctl_reports_them() {
    let dir = scratch("login");
    let path = dir.join("var/run/utmp");
    let file = UtmpFile::new(&path);

    // `logind` records the boot, then every session it tracks.
    file.put(&UtmpRecord::boot_time(1, 1_792_022_300)).unwrap();

    let mut sessions = Sessions::new();
    let first = sessions.login(&LoginRequest::tty("user", 1000, "tty1", 42, 1_792_022_400));
    let second = sessions.login(&LoginRequest::tty("root", 0, "tty2", 77, 1_792_022_500));
    assert_eq!((first.as_str(), second.as_str()), ("1", "2"));
    for session in sessions.list() {
        file.put(&session.to_utmp()).unwrap();
    }

    // The file is the standard one: the boot record is not a login, and every
    // record is exactly one record long.
    let records = file.read_all().unwrap();
    assert_eq!(records.len(), 3, "boot record plus two logins");
    assert_eq!(file.read_users().unwrap().len(), 2);
    assert_eq!(fs::metadata(&path).unwrap().len(), 3 * RECORD_SIZE as u64);

    // `loginctl list-sessions` works with no daemon: it reads the file.
    let cli = parse_args(args(&["--utmp", path.to_str().unwrap(), "list-sessions"])).unwrap();
    assert_eq!(cli.utmp, path);
    let sessions = restored(&cli.utmp);
    assert_eq!(sessions.len(), 2);

    let rendered = render_sessions(sessions.list(), cli.no_legend);
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "SESSION  UID USER SEAT  TTY");
    assert_eq!(lines[1], "      1 1000 user seat0 tty1");
    assert_eq!(lines[2], "      2    0 root seat0 tty2");

    // `session-status` describes one session, in the familiar block.
    let status = render_session_status(sessions.session("1").unwrap());
    assert!(status.starts_with("1 - user (1000)\n"));
    assert!(status.contains("Since: 2026-10-15 00:00:00\n"));
    assert!(status.contains("State: active\n"));
    assert!(status.contains("Seat: seat0\n"));
    assert!(status.contains("TTY: tty1\n"));
    assert!(status.contains("Leader: 42\n"));

    // The second session is online but does not own the console.
    let status = render_session_status(sessions.session("2").unwrap());
    assert!(status.contains("State: online\n"));

    // `show-session` is machine readable.
    let shown = render_show_session(sessions.session("2").unwrap());
    assert!(shown.contains("Id=2\n"));
    assert!(shown.contains("User=root\n"));
    assert!(shown.contains("UID=0\n"));
    assert!(shown.contains("State=online\n"));
    assert!(shown.contains("Active=no\n"));
    assert!(
        shown.lines().all(|line| line.contains('=')),
        "every line must be a property"
    );

    // Arms crossed: the seat knows which session owns it.
    let seats = sessions.seats();
    assert_eq!(seats.len(), 1);
    assert_eq!(seats[0].active.as_deref(), Some("1"));
    let rendered = render_seats(&seats, false);
    assert!(rendered.contains("seat0 1      1 2"));
    assert!(render_show_seat(&seats[0]).contains("ActiveSession=1\n"));
    assert!(render_seat_status(&seats[0], &sessions).contains("├─1 user (tty1)\n"));

    // And the whole set survives a round trip through the parser and the file
    // again, which is what happens on every `loginctl` call.
    let second_pass = restored(&cli.utmp);
    assert_eq!(second_pass.len(), sessions.len());
    assert_eq!(
        render_sessions(second_pass.list(), true),
        render_sessions(sessions.list(), true)
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn logging_out_hands_the_seat_over_and_clears_the_file() {
    let dir = scratch("logout");
    let path = dir.join("utmp");
    let file = UtmpFile::new(&path);

    let mut sessions = Sessions::new();
    sessions.login(&LoginRequest::tty("user", 1000, "tty1", 42, 100));
    sessions.login(&LoginRequest::tty("root", 0, "tty2", 77, 200));
    for session in sessions.list() {
        file.put(&session.to_utmp()).unwrap();
    }
    assert_eq!(file.read_users().unwrap().len(), 2);

    // The active session logs out: the seat is handed to the other one and the
    // terminal disappears from the file.
    let removed = sessions.logout("1").unwrap();
    file.put(&UtmpRecord::dead_process(&removed.tty, 300))
        .unwrap();

    let users = file.read_users().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].line, "tty2");

    let sessions = restored(&path);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions.active_on("seat0").unwrap().user, "root");

    // Logging out the last session leaves an empty file that still exists.
    sessions_round_trip_removes_everything(&path);
    assert!(file.read_users().unwrap().is_empty());
    assert!(path.exists());

    fs::remove_dir_all(&dir).unwrap();
}

/// A logout helper that writes through the same path `logind` would.
fn sessions_round_trip_removes_everything(path: &std::path::Path) {
    let file = UtmpFile::new(path);
    let mut sessions = restored(path);
    let last = sessions.list()[0].clone();
    sessions.logout(&last.id).unwrap();
    file.put(&UtmpRecord::dead_process(&last.tty, 400)).unwrap();
}

#[test]
fn seats_are_reported_separately() {
    let dir = scratch("seats");
    let path = dir.join("utmp");
    let file = UtmpFile::new(&path);

    let mut sessions = Sessions::new();
    sessions.login(&LoginRequest {
        user: "user".to_owned(),
        uid: 1000,
        tty: "tty1".to_owned(),
        leader: 42,
        host: String::new(),
        session_type: SessionType::Tty,
        seat: None,
        now: 1_792_022_400,
    });
    sessions.login(&LoginRequest {
        session_type: SessionType::Graphical,
        seat: Some("seat1".to_owned()),
        ..LoginRequest::tty("guest", 1001, "display0", 99, 1_792_022_500)
    });
    for session in sessions.list() {
        file.put(&session.to_utmp()).unwrap();
    }

    // The live registry keeps the seats apart: this is what the daemon serves.
    let seats = sessions.seats();
    assert_eq!(seats.len(), 2);
    assert_eq!(seats[0].id, "seat0");
    assert_eq!(seats[1].id, "seat1");

    let rendered = render_seats(&seats, false);
    assert!(rendered.contains("seat0"));
    assert!(rendered.contains("seat1"));

    // Each seat owns its own active session, and each is listed by name.
    assert_eq!(sessions.active_on("seat0").unwrap().user, "user");
    assert_eq!(sessions.active_on("seat1").unwrap().user, "guest");
    let status = render_seat_status(&seats[1], &sessions);
    assert!(status.starts_with("seat1\n"));
    assert!(status.contains("└─2 guest (display0)\n"));

    // The record format itself has no seat field, so a file-only view — what
    // `loginctl` falls back to without the daemon — puts every terminal on the
    // console seat. The daemon serves the exact state.
    let file_only = restored(&path);
    assert_eq!(file_only.seats().len(), 1);
    assert_eq!(file_only.seats()[0].id, "seat0");

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_damaged_record_does_not_hide_the_sessions() {
    let dir = scratch("damaged");
    let path = dir.join("utmp");

    let mut sessions = Sessions::new();
    sessions.login(&LoginRequest::tty("user", 1000, "tty1", 42, 100));
    sessions.login(&LoginRequest::tty("root", 0, "tty2", 77, 200));
    let records: Vec<UtmpRecord> = sessions.list().iter().map(|s| s.to_utmp()).collect();
    UtmpFile::new(&path).write_all(&records).unwrap();

    // A torn write at the end of the file, and an unknown record type.
    {
        let mut raw = fs::OpenOptions::new().append(true).open(&path).unwrap();
        use std::io::Write;
        raw.write_all(&[0x42; 17]).unwrap();
    }
    let mut contents = fs::read(&path).unwrap();
    contents.extend_from_slice(&[99, 0]); // record type 99, then padding
    contents.resize(contents.len() + RECORD_SIZE - 2, 0);
    fs::write(&path, contents).unwrap();

    let sessions = restored(&path);
    assert_eq!(sessions.len(), 2, "the readable records must survive");
    let rendered = render_sessions(sessions.list(), true);
    assert_eq!(rendered.lines().count(), 2);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_boot_record_is_not_a_session() {
    let dir = scratch("boot-only");
    let path = dir.join("utmp");
    UtmpFile::new(&path)
        .put(&UtmpRecord::boot_time(1, 1_792_022_300))
        .unwrap();

    let records = UtmpFile::new(&path).read_all().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record_type, RecordType::BootTime);
    assert!(!records[0].is_user());

    // Nothing to log in, but `loginctl` must still answer instead of failing.
    let sessions = restored(&path);
    assert!(sessions.is_empty());
    assert_eq!(
        render_sessions(sessions.list(), false),
        "SESSION UID USER SEAT TTY\n"
    );
    assert_eq!(
        render_seats(&sessions.seats(), false),
        "SEAT ACTIVE SESSIONS\n"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn loginctl_reads_the_file_selected_on_the_command_line() {
    let dir = scratch("cli");

    // Two different files, selected the two ways the parser supports.
    let first = dir.join("utmp.one");
    let second = dir.join("utmp.two");

    let mut sessions = Sessions::new();
    sessions.login(&LoginRequest::tty("user", 1000, "tty1", 42, 100));

    fs::write(&first, sessions.list()[0].to_utmp().encode()).unwrap();
    sessions.login(&LoginRequest::tty("root", 0, "tty2", 77, 200));
    let mut contents = fs::read(&first).unwrap();
    contents.extend_from_slice(&sessions.list()[1].to_utmp().encode());
    fs::write(&second, contents).unwrap();

    let cli = parse_args(args(&["--utmp", first.to_str().unwrap(), "list-sessions"])).unwrap();
    assert_eq!(restored(&cli.utmp).len(), 1);

    let cli = parse_args(args(&[
        &format!("--utmp={}", second.display()),
        "list-sessions",
    ]))
    .unwrap();
    assert_eq!(restored(&cli.utmp).len(), 2);

    // A session that does not exist is reported, not silently ignored.
    let sessions = restored(&second);
    assert!(sessions.session("42").is_none());
    assert!(logind::render::unknown_target("loginctl", "42").contains("42"));

    fs::remove_dir_all(&dir).unwrap();
}
