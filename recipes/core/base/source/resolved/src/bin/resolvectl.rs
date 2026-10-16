//! `resolvectl`: inspect and set the resolver of the system.
//!
//! Two places hold a resolver: the network stack, which learns one from the
//! network, and `/etc/resolv.conf`, which is what a program reads. `status` and
//! `dns` read both and report; `update` writes the file to agree with the
//! stack; `set-dns` decides both. `--dry-run` prints what would be written
//! instead of writing it.

use std::process::ExitCode;

use networkd::netcfg::Netcfg;
use resolved::cli::parse_env_args;
use resolved::render::{render_dns, render_status, render_sync};
use resolved::state::Resolved;
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
            let sync = resolved.sync(cli.dry_run).map_err(|err| err.0)?;
            print!("{}", render_sync(&sync, cli.dry_run));
            Ok(())
        }
        Command::SetDns => {
            let addr = cli.resolver.ok_or("set-dns needs an address")?;
            report_warnings(&resolved.status().map_err(|err| err.0)?);
            let (write, sync) = resolved.set_dns(addr, cli.dry_run).map_err(|err| err.0)?;
            println!("{}", write.render(resolved.netcfg().root()));
            print!("{}", render_sync(&sync, cli.dry_run));
            Ok(())
        }
        Command::None => Err("no command given".to_owned()),
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
