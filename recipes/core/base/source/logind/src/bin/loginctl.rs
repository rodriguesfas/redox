//! `loginctl`: inspect and control the sessions and seats of `logind`.
//!
//! Reads come from the running login manager when it answers, and from the
//! record file (`/var/run/utmp`) otherwise — the file is kept up to date by the
//! daemon, so a listing works from a rescue shell, before the daemon starts, or
//! on a system that does not run it at all. Stopping a session needs the
//! daemon: a session is a live process, not a line in a file.

use std::process::ExitCode;

use logind::cli::{parse_env_args, USAGE};
use logind::client::Client;
use logind::render::{
    render_seat_status, render_seats, render_session_status, render_sessions, render_show_seat,
    render_show_session, unknown_target,
};
use logind::utmp::UtmpFile;
use logind::Command;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let cli = match parse_env_args(args) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("loginctl: {err}");
            eprintln!("Try 'loginctl --help' for more information.");
            return ExitCode::FAILURE;
        }
    };

    if cli.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    if cli.version {
        println!("loginctl {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let client = Client::new(cli.scheme.clone());
    let utmp = UtmpFile::new(cli.utmp.clone());

    let mut out = String::new();
    let mut failed = false;

    // Control verbs are sent to the daemon and nothing else is needed: the
    // record file cannot stop a process.
    if cli.command.is_action() {
        for target in &cli.targets {
            let result = if cli.command == Command::TerminateSession {
                client.terminate(target)
            } else {
                client.activate(target)
            };

            match result {
                Ok(id) => out.push_str(&format!("{target} -> {id}\n")),
                Err(err) => {
                    eprintln!("loginctl: {target}: {err}");
                    failed = true;
                }
            }
        }

        print!("{out}");
        return if failed {
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }

    // A named file is read as it is; otherwise the live view of the daemon is
    // preferred, with the file as the fallback.
    let sessions = if cli.utmp_given {
        match logind::client::file_sessions(&utmp) {
            Ok(sessions) => sessions,
            Err(err) => {
                eprintln!("loginctl: cannot read {}: {err}", cli.utmp.display());
                return ExitCode::FAILURE;
            }
        }
    } else {
        match client.sessions_or_file(&utmp) {
            Ok((sessions, _origin)) => sessions,
            Err(err) => {
                eprintln!(
                    "loginctl: no session source: {} and {}: {err}",
                    client.scheme().display(),
                    cli.utmp.display()
                );
                return ExitCode::FAILURE;
            }
        }
    };

    match cli.command {
        Command::ListSessions => {
            out.push_str(&render_sessions(sessions.list(), cli.no_legend));
        }
        Command::ListSeats => {
            out.push_str(&render_seats(&sessions.seats(), cli.no_legend));
        }
        Command::SessionStatus | Command::ShowSession => {
            let ids: Vec<String> = if cli.targets.is_empty() {
                sessions
                    .list()
                    .iter()
                    .map(|session| session.id.clone())
                    .collect()
            } else {
                cli.targets.clone()
            };

            for id in &ids {
                let Some(session) = sessions.session(id) else {
                    eprintln!("{}", unknown_target("loginctl", id));
                    failed = true;
                    continue;
                };
                if cli.command == Command::SessionStatus {
                    out.push_str(&render_session_status(session));
                } else {
                    out.push_str(&render_show_session(session));
                }
            }
        }
        Command::SeatStatus | Command::ShowSeat => {
            let seats = sessions.seats();
            let names: Vec<String> = if cli.targets.is_empty() {
                seats.iter().map(|seat| seat.id.clone()).collect()
            } else {
                cli.targets.clone()
            };

            for name in &names {
                let Some(seat) = seats.iter().find(|seat| &seat.id == name) else {
                    eprintln!("{}", unknown_target("loginctl", name));
                    failed = true;
                    continue;
                };
                if cli.command == Command::SeatStatus {
                    out.push_str(&render_seat_status(seat, &sessions));
                } else {
                    out.push_str(&render_show_seat(seat));
                }
            }
        }
        Command::None => {
            eprintln!("loginctl: no command given");
            return ExitCode::FAILURE;
        }
        // Handled above, before anything is read.
        Command::TerminateSession | Command::ActivateSession => unreachable!(),
    }

    print!("{out}");

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
