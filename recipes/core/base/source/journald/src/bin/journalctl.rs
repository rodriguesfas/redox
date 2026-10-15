//! `journalctl`: query a journal file produced by the logging daemon.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use journald::cli::{parse_args, DEFAULT_JOURNAL_PATH, USAGE};
use journald::render::{render, render_boot_list};
use journald::store::JournalStore;

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

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

    // `--follow` is accepted for compatibility; the daemon side of following is
    // implemented together with the journal writer.
    if cli.follow {
        eprintln!("journalctl: --follow is not implemented yet");
    }

    ExitCode::SUCCESS
}
