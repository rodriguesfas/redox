//! `loginctl`: inspect the sessions and seats recorded by `logind`.
//!
//! Without a running daemon the state comes from the login record file
//! (`/var/run/utmp`), which `logind` keeps up to date, so the tool is useful
//! from the first boot line — and from a rescue shell.

use std::process::ExitCode;

use logind::cli::{parse_env_args, USAGE};
use logind::passwd::lookup_uid;
use logind::render::{
    render_seat_status, render_seats, render_session_status, render_sessions, render_show_seat,
    render_show_session, unknown_target,
};
use logind::session::Sessions;
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

    let records = match UtmpFile::new(cli.utmp.clone()).read_all() {
        Ok(records) => records,
        Err(err) => {
            eprintln!("loginctl: cannot read {}: {err}", cli.utmp.display());
            return ExitCode::FAILURE;
        }
    };

    // `utmp` has no user id, so names are resolved again from the account
    // database to make the listings readable.
    let mut sessions = Sessions::from_utmp(&records);
    sessions.assign_uids(lookup_uid);

    let mut out = String::new();
    let mut failed = false;

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
    }

    print!("{out}");

    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
