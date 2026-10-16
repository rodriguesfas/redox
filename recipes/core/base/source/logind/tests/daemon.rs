//! End-to-end test of the login manager: the requests `login` and `loginctl`
//! send, the state they produce, and the three views of it — the live records
//! the scheme serves, the `utmp` file tools read, and what `loginctl` prints.
//!
//! The scheme itself only runs on Redox; everything under it is tested here,
//! including the real `loginctl` binary reading a mounted state.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use logind::client::{Client, Origin};
use logind::protocol::{format_response, parse_response, Request};
use logind::registry::Registry;
use logind::render::{render_seat_status, render_seats, render_show_session};
use logind::session::{LoginRequest, Sessions};
use logind::utmp::{RecordType, UtmpFile};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("logind-daemon-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn login(user: &str, uid: u32, tty: &str, pid: i32) -> Request {
    Request::Login {
        user: user.to_owned(),
        uid,
        tty: tty.to_owned(),
        leader: pid,
        seat: None,
    }
}

/// What the daemon publishes for the readers: the sessions file of the scheme.
fn publish(scheme: &Path, registry: &Registry) {
    fs::create_dir_all(scheme).unwrap();
    fs::write(scheme.join("sessions"), registry.sessions().encode()).unwrap();
}

#[test]
fn a_login_flow_reaches_every_view() {
    let dir = scratch("flow");
    let scheme = dir.join("scheme");
    let utmp = UtmpFile::new(dir.join("utmp"));

    // The daemon starts: it owns the record file from the first boot record on.
    let mut registry = Registry::boot(utmp.clone(), "boot-test", 1_000).unwrap();
    assert_eq!(registry.utmp().read_all().unwrap().len(), 1);
    assert_eq!(
        registry.utmp().read_all().unwrap()[0].record_type,
        RecordType::BootTime
    );

    // `login` announces the session it is starting, and gets its identifier.
    let response = format_response(
        &registry
            .handle(&login("user", 1000, "tty1", 4242), 1_100)
            .map(|outcome| outcome.message),
    );
    assert_eq!(response, "ok 1\n");
    assert_eq!(parse_response(&response).unwrap(), "1");

    // The scheme serves the live state...
    publish(&scheme, &registry);
    let client = Client::new(&scheme);
    let live = client.sessions().unwrap();
    assert_eq!(live.list().len(), 1);
    assert_eq!(live.list()[0].user, "user");
    assert_eq!(live.list()[0].uid, 1000);
    assert_eq!(live.list()[0].leader, 4242);
    assert_eq!(live.list()[0].seat, "seat0");
    assert!(live.list()[0].is_active());

    // ... and so does the record file, for tools that know nothing of the
    // daemon: the two views agree.
    let (file, origin) = Client::new(dir.join("no-daemon"))
        .sessions_or_file(&utmp)
        .unwrap();
    assert_eq!(origin, Origin::File);
    assert_eq!(file.list().len(), 1);
    assert_eq!(file.list()[0].tty, live.list()[0].tty);

    // A second login waits for the console; the daemon knows, and says so.
    registry
        .handle(&login("root", 0, "tty2", 5150), 1_200)
        .unwrap();
    publish(&scheme, &registry);
    let live = Client::new(&scheme).sessions().unwrap();
    assert_eq!(live.seats().len(), 1);
    assert_eq!(live.seats()[0].active.as_deref(), Some("1"));

    // `loginctl terminate-session` is a request: the daemon marks the session
    // closing and reports the process to signal.
    let outcome = registry
        .handle(
            &Request::Terminate {
                target: "tty1".to_owned(),
            },
            1_300,
        )
        .unwrap();
    assert_eq!(outcome.message, "1");
    assert_eq!(outcome.signal, Some(4242));

    // The login that was stopped reports itself as over, and the seat hands the
    // console to the session that was waiting.
    let outcome = registry
        .handle(
            &Request::Logout {
                target: "1".to_owned(),
            },
            1_400,
        )
        .unwrap();
    assert_eq!(outcome.message, "tty1");
    assert_eq!(registry.sessions().active_on("seat0").unwrap().id, "2");

    // The file and the scheme agree on what is left.
    publish(&scheme, &registry);
    assert_eq!(Client::new(&scheme).sessions().unwrap().len(), 1);
    assert_eq!(utmp.read_users().unwrap().len(), 1);
    assert_eq!(utmp.read_users().unwrap()[0].line, "tty2");

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_login_that_died_is_pruned_from_both_views() {
    let dir = scratch("prune");
    let utmp = UtmpFile::new(dir.join("utmp"));
    let mut registry = Registry::boot(utmp.clone(), "boot-prune", 1_000).unwrap();
    registry
        .handle(&login("user", 1000, "tty1", 4242), 1_100)
        .unwrap();
    registry
        .handle(&login("root", 0, "tty2", 5150), 1_200)
        .unwrap();

    // `loginctl` asks the daemon to drop what it can see is gone before it acts
    // on a request; only the session whose leader is missing disappears.
    let removed = registry.prune(|pid| pid == 4242);
    assert_eq!(removed, vec!["tty2".to_owned()]);
    assert_eq!(utmp.read_users().unwrap().len(), 1);
    assert_eq!(
        utmp.read_users().unwrap()[0].line,
        "tty1",
        "a live session is never hidden"
    );

    // A callback that cannot tell keeps every session: the daemon would rather
    // show a finished session than drop a live one.
    assert!(registry.prune(|_| true).is_empty());
    assert_eq!(registry.sessions().len(), 1);

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_seats_of_the_daemon_are_renderable() {
    let dir = scratch("seats");
    let mut registry =
        Registry::boot(UtmpFile::new(dir.join("utmp")), "boot-seats", 1_000).unwrap();
    registry
        .handle(&login("user", 1000, "tty1", 4242), 1_100)
        .unwrap();
    registry
        .handle(&login("root", 0, "tty2", 5150), 1_200)
        .unwrap();

    let sessions = registry.sessions();
    let seats = sessions.seats();
    assert_eq!(seats[0].active.as_deref(), Some("1"));
    assert_eq!(
        render_seats(&seats, false),
        "SEAT  ACTIVE SESSIONS\nseat0 1      1 2\n"
    );
    assert_eq!(
        render_seat_status(&seats[0], sessions),
        "seat0\n      Sessions:\n├─1 user (tty1)\n└─2 root (tty2)\n"
    );
    assert!(render_show_session(sessions.session("2").unwrap()).contains("State=online\n"));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn loginctl_reads_the_daemon_when_it_is_mounted() {
    let dir = scratch("binary");
    let scheme = dir.join("scheme");

    let mut registry =
        Registry::boot(UtmpFile::new(dir.join("utmp")), "boot-binary", 1_000).unwrap();
    registry
        .handle(&login("user", 1000, "tty1", 4242), 1_100)
        .unwrap();
    registry
        .handle(&login("root", 0, "tty2", 5150), 1_200)
        .unwrap();
    publish(&scheme, &registry);

    let run = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_loginctl"))
            .args(args)
            .output()
            .unwrap();
        (
            output.status.success(),
            String::from_utf8(output.stdout).unwrap(),
            String::from_utf8(output.stderr).unwrap(),
        )
    };

    let scheme = scheme.to_str().unwrap();
    let (ok, out, err) = run(&["--scheme", scheme, "list-sessions"]);
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "SESSION  UID USER SEAT  TTY\n\
         \x20     1 1000 user seat0 tty1\n\
         \x20     2    0 root seat0 tty2\n"
    );

    let (ok, out, _) = run(&["--scheme", scheme, "-n", "list-seats"]);
    assert!(ok);
    assert_eq!(out, "seat0 1      1 2\n");

    let (ok, out, _) = run(&["--scheme", scheme, "show-session", "2"]);
    assert!(ok);
    assert!(out.contains("Id=2\n"));
    assert!(out.contains("User=root\n"));
    assert!(out.contains("TTY=tty2\n"));

    let (ok, out, _) = run(&["--scheme", scheme, "session-status", "tty1"]);
    assert!(!ok, "a terminal name is not a session identifier");
    assert!(out.is_empty());

    // A session the daemon does not know is an error, not an empty listing.
    let (ok, _, err) = run(&["--scheme", scheme, "show-session", "9"]);
    assert!(!ok);
    assert!(err.contains("9"), "{err}");

    // Acting on a session needs the daemon: without one, the failure is
    // reported instead of silently doing nothing.
    let (ok, _, err) = run(&[
        "--scheme",
        dir.join("nothing").to_str().unwrap(),
        "terminate-session",
        "1",
    ]);
    assert!(!ok);
    assert!(err.contains("1"), "{err}");

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn loginctl_falls_back_to_the_record_file() {
    let dir = scratch("fallback");
    let utmp = dir.join("utmp");

    // A file written the way the daemon writes it, with no daemon in sight.
    let mut sessions = Sessions::new();
    sessions.login(&LoginRequest::tty("user", 1000, "tty1", 4242, 1_100));
    let file = UtmpFile::new(&utmp);
    file.put(&sessions.list()[0].to_utmp()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_loginctl"))
        .args([
            "--scheme",
            dir.join("nothing").to_str().unwrap(),
            "--utmp",
            utmp.to_str().unwrap(),
            "list-sessions",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let out = String::from_utf8(output.stdout).unwrap();
    assert!(out.contains("tty1"), "{out}");
    assert!(out.contains("user"), "{out}");

    fs::remove_dir_all(&dir).unwrap();
}
