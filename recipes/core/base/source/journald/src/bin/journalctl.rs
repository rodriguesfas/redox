//! `journalctl`: query a journal file produced by the logging daemon.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use journald::cli::{parse_args, DEFAULT_JOURNAL_PATH, USAGE};
use journald::follow::Follower;
use journald::render::{render, render_boot_list};
use journald::store::JournalStore;
use journald::time::unix_now;

/// How long `--follow` waits between polls of the journal file.
const FOLLOW_INTERVAL: Duration = Duration::from_millis(250);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let cli = match parse_args(args, unix_now()) {
        Ok(cli) => cli,
        Err(err) => {
            eprintln!("journalctl: {err}");
            eprintln!("Try 'journalctl --help' for more information.");
            return ExitCode::FAILURE;
        }
    };

    if cli.help {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    if cli.version {
        println!("journalctl {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let path = cli
        .file
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_JOURNAL_PATH));

    // `--follow` starts at the end of the journal, so it needs the current size
    // before the file is read. Setting it up first also means entries written
    // between reading the backlog and following are not lost.
    let mut follower = if cli.follow {
        match Follower::new(&path, cli.query.clone(), cli.output, cli.hostname.clone()) {
            Ok(follower) => Some(follower),
            Err(err) => {
                eprintln!("journalctl: cannot follow {}: {err}", path.display());
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    let store = match JournalStore::read_from_path(&path) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("journalctl: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };

    if cli.list_boots {
        print!("{}", render_boot_list(&store.boot_summary()));
        return ExitCode::SUCCESS;
    }

    let entries = match cli.lines {
        Some(lines) => store.query_tail(&cli.query, lines),
        None => store.query(&cli.query),
    };

    print!("{}", render(&entries, cli.output, &cli.hostname));

    let Some(follower) = follower.as_mut() else {
        return ExitCode::SUCCESS;
    };

    let _ = std::io::stdout().flush();

    // Print every entry appended from now on, until interrupted.
    let mut stdout = std::io::stdout();
    loop {
        match follower.render(&mut stdout) {
            Ok(0) => std::thread::sleep(FOLLOW_INTERVAL),
            Ok(_) => continue,
            Err(err) => {
                eprintln!("journalctl: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
}
