//! `resolvectl`: inspect and set the resolver of the system.
//!
//! Two places hold a resolver: the network stack, which learns one from the
//! network, and `/etc/resolv.conf`, which is what a program reads. `status` and
//! `dns` read both and report; `update` writes the file to agree with the
//! stack; `set-dns` decides both. `--dry-run` prints what would be written
//! instead of writing it.
//!
//! A write is handed to the daemon when it is running, so that the file of the
//! system has one writer; when it answers, `resolvectl` says so. A command that
//! names its own file or mount point is written directly: the daemon manages
//! the file of the system, not the one of this command.

use std::process::ExitCode;

use networkd::netcfg::Netcfg;
use networkd::plan::Write;
use resolved::cli::parse_env_args;
use resolved::client::Client;
use resolved::protocol::{Outcome, Request};
use resolved::render::{render_delegated, render_dns, render_status, render_sync};
use resolved::state::{Resolved, NETCFG_PATH};
use resolved::{Command, Status, USAGE};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let cli = match parse_env_args(args) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("resolvectl: {err}");
            eprintln!("Try 'resolvectl --help' for more information.");
            return ExitCode::FAILURE;
        }
    };

    if cli.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    if cli.version {
        println!("resolvectl {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("resolvectl: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &resolved::Cli) -> Result<(), String> {
    let netcfg = Netcfg::new(cli.netcfg.to_string_lossy().into_owned());
    let resolved = Resolved::new(cli.file.clone(), netcfg);

    match cli.command {
        Command::Status => {
            let status = resolved.status().map_err(|err| err.0)?;
            print!("{}", render_status(&status));
            report_warnings(&status);
            Ok(())
        }
        Command::Dns => {
            let status = resolved.status().map_err(|err| err.0)?;
            print!("{}", render_dns(&status, cli.no_legend));
            report_warnings(&status);
            Ok(())
        }
        Command::Update => {
            // The file that is about to be written is the file that was read:
            // what is wrong with it is worth saying before it is replaced.
            report_warnings(&resolved.status().map_err(|err| err.0)?);

            if let Some(outcome) = ask_the_daemon(cli, &Request::Sync)? {
                print!("{}", render_delegated(&outcome));
                return Ok(());
            }

            let sync = resolved.sync(cli.dry_run).map_err(|err| err.0)?;
            print!("{}", render_sync(&sync, cli.dry_run));
            Ok(())
        }
        Command::SetDns => {
            let addr = cli.resolver.ok_or("set-dns needs an address")?;
            report_warnings(&resolved.status().map_err(|err| err.0)?);

            if let Some(outcome) = ask_the_daemon(cli, &Request::SetDns(addr))? {
                // The daemon writes the stack too, and it does it before the
                // file: the netcfg write is what it was asked to make true.
                let write = Write {
                    path: NETCFG_PATH.to_owned(),
                    value: format!("{addr}\n"),
                };
                println!("{}", write.render(resolved.netcfg().root()));
                print!("{}", render_delegated(&outcome));
                return Ok(());
            }

            let (write, sync) = resolved.set_dns(addr, cli.dry_run).map_err(|err| err.0)?;
            println!("{}", write.render(resolved.netcfg().root()));
            print!("{}", render_sync(&sync, cli.dry_run));
            Ok(())
        }
        Command::None => Err("no command given".to_owned()),
    }
}

/// Asks the daemon when it is the one that writes.
///
/// A dry run never writes, so it never asks; and a command that named a file or
/// a mount point of its own is about that one, so it is answered here. The
/// daemon also has to be talking about the same file: a daemon that answers
/// about another one is a daemon for another system, and its answer is not ours
/// to print.
fn ask_the_daemon(cli: &resolved::Cli, request: &Request) -> Result<Option<Outcome>, String> {
    if !resolved::client::writes_through_the_daemon(cli) {
        return Ok(None);
    }

    match Client::new(cli.scheme.clone()).request_if_mounted(request) {
        None => Ok(None),
        Some(Err(err)) => Err(err.to_string()),
        Some(Ok(outcome)) => {
            if outcome.describes(&cli.file) {
                Ok(Some(outcome))
            } else {
                // The daemon manages a file of its own: this command is about
                // another one, and the tool writes it.
                Ok(None)
            }
        }
    }
}

/// What is wrong with the file is worth saying: the file is usually written by
/// hand at some point in a system's life, and a line that was not understood is
/// a line whose author has to know.
fn report_warnings(status: &Status) {
    for warning in &status.warnings {
        eprintln!("resolvectl: warning: {warning}");
    }
}
