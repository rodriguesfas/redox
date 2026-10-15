//! Argument parsing for the `journalctl` front end.

use std::fmt;
use std::path::PathBuf;

use crate::priority::parse_priority;
use crate::query::{BootFilter, FieldMatch, Query};
use crate::store::is_valid_boot_id;
use crate::time::parse_timestamp;

/// Output formats, corresponding to `journalctl -o`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum OutputMode {
    /// Timestamp, host, unit and message on one line.
    #[default]
    Short,
    /// `MESSAGE=value` style pairs, one record per entry.
    Export,
    /// One JSON object per line.
    Json,
    /// Pretty-printed JSON.
    JsonPretty,
    /// Message text only.
    Cat,
}

impl OutputMode {
    /// Parses an `-o` value.
    pub fn parse(value: &str) -> Option<OutputMode> {
        Some(match value {
            "short" => OutputMode::Short,
            "export" => OutputMode::Export,
            "json" => OutputMode::Json,
            "json-pretty" => OutputMode::JsonPretty,
            "cat" => OutputMode::Cat,
            _ => return None,
        })
    }
}

/// Default journal file, matching the location used by the daemon.
pub const DEFAULT_JOURNAL_PATH: &str = "/scheme/log/journal";

/// Help text.
pub const USAGE: &str = "\
journalctl - query the AIOS/Redox structured journal

Usage: journalctl [OPTIONS] [FIELD=value...]

Options:
  -u, --unit UNIT        Show entries for a unit or identifier
  -p, --priority PRI     Show entries at this severity or more severe
  -g, --grep PATTERN     Show entries whose message contains PATTERN
      --case-sensitive   Make --grep case sensitive
  -S, --since TIME       Start of the window (2026-10-15, -1h, @1234567890)
  -U, --until TIME       End of the window
  -n, --lines N          Show only the last N matches
  -b, --boot [ID]        Show entries from this boot (default: current)
      --list-boots       List boots present in the journal
      --invert           Show entries that do not match
  -o, --output MODE      short, export, json, json-pretty or cat
  -f, --follow           Follow the journal
      --file PATH        Journal file to read
  -h, --help             Print this help
      --version          Print the version
";

/// A fully parsed `journalctl` invocation.
#[derive(Clone, Debug)]
pub struct Cli {
    /// Filters to apply.
    pub query: Query,
    /// Output format.
    pub output: OutputMode,
    /// Show only the last `lines` matches.
    pub lines: Option<usize>,
    /// Keep printing entries as they arrive.
    pub follow: bool,
    /// List the boots present in the journal instead of entries.
    pub list_boots: bool,
    /// Journal file to read, overriding the default.
    pub file: Option<PathBuf>,
    /// Host name to show in the short format when an entry has none.
    pub hostname: String,
    /// `-h`/`--help` was given.
    pub help: bool,
    /// `--version` was given.
    pub version: bool,
}

/// Returned for any argument that cannot be understood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

/// Short options that take a value, and may therefore be written glued to it
/// (`-n5` is the same as `-n 5`).
const VALUE_SHORTS: [&str; 8] = ["-u", "-p", "-g", "-S", "-U", "-o", "-n", "-b"];
/// Short options that take no value, and may be grouped (`-fh`).
const FLAG_SHORTS: &str = "fh";

/// Splits glued short options (`-n5`, `-fh`) into separate tokens.
///
/// Only recognised options are split, so that values such as `-1h` are left
/// alone when they follow an option that takes a value.
fn expand_glued(args: Vec<String>) -> Result<Vec<String>, CliError> {
    let mut expanded = Vec::with_capacity(args.len());
    for arg in args {
        let is_short = arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2;
        if !is_short {
            expanded.push(arg);
            continue;
        }

        let (short, rest) = arg.split_at(2);
        let leading = short.chars().nth(1).unwrap_or(' ');
        if VALUE_SHORTS.contains(&short) {
            expanded.push(short.to_owned());
            expanded.push(rest.to_owned());
        } else if FLAG_SHORTS.contains(leading)
            && !rest.is_empty()
            && rest.chars().all(|c| FLAG_SHORTS.contains(c))
        {
            expanded.push(short.to_owned());
            for flag in rest.chars() {
                expanded.push(format!("-{flag}"));
            }
        } else {
            // Not a recognised glued option: leave it for the main loop to
            // reject, or to keep as a value.
            expanded.push(arg);
        }
    }
    Ok(expanded)
}

fn take_value<I: Iterator<Item = String>>(
    args: &mut I,
    inline: Option<&str>,
    name: &str,
) -> Result<String, CliError> {
    match inline {
        Some(value) => Ok(value.to_owned()),
        None => args
            .next()
            .ok_or_else(|| CliError(format!("{name} requires a value"))),
    }
}

/// Parses command line arguments, excluding the program name. `now` is used to
/// resolve relative timestamps such as `-1h`.
pub fn parse_args<I>(args: I, now: i64) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = Cli {
        query: Query::new(),
        output: OutputMode::Short,
        lines: None,
        follow: false,
        list_boots: false,
        file: None,
        hostname: "localhost".to_owned(),
        help: false,
        version: false,
    };

    let mut args = expand_glued(args.into_iter().collect())?
        .into_iter()
        .peekable();
    let mut positionals_only = false;

    while let Some(arg) = args.next() {
        if positionals_only {
            push_positional(&mut cli, &arg)?;
            continue;
        }

        // Support both `--flag value` and `--flag=value`.
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (arg.clone(), None),
        };
        let inline = inline.as_deref();

        match flag.as_str() {
            "--" => positionals_only = true,
            "-h" | "--help" => cli.help = true,
            "--version" => cli.version = true,
            "-f" | "--follow" => cli.follow = true,
            "--list-boots" => cli.list_boots = true,
            "--invert" => cli.query.inverted = true,
            "--case-sensitive" => cli.query.grep_case_sensitive = true,
            "-b" | "--boot" => {
                cli.query.boot = match inline {
                    Some(id) => BootFilter::Id(id.to_owned()),
                    // `-b` takes an optional argument: only a boot identifier is
                    // consumed, so `-b MESSAGE=x` still works.
                    None => match args.peek().is_some_and(|token| is_valid_boot_id(token)) {
                        true => BootFilter::Id(args.next().unwrap()),
                        false => BootFilter::Current,
                    },
                };
            }
            "-u" | "--unit" => cli.query.unit = Some(take_value(&mut args, inline, "--unit")?),
            "-p" | "--priority" => {
                let raw = take_value(&mut args, inline, "--priority")?;
                cli.query.priority = Some(
                    parse_priority(&raw)
                        .ok_or_else(|| CliError(format!("unknown priority {raw:?}")))?,
                );
            }
            "-g" | "--grep" => cli.query.grep = Some(take_value(&mut args, inline, "--grep")?),
            "-S" | "--since" => {
                let raw = take_value(&mut args, inline, "--since")?;
                cli.query.since =
                    Some(parse_timestamp(&raw, now).map_err(|err| CliError(err.to_string()))?);
            }
            "-U" | "--until" => {
                let raw = take_value(&mut args, inline, "--until")?;
                cli.query.until =
                    Some(parse_timestamp(&raw, now).map_err(|err| CliError(err.to_string()))?);
            }
            "-n" | "--lines" => {
                let raw = take_value(&mut args, inline, "--lines")?;
                cli.lines = Some(
                    raw.parse()
                        .map_err(|_| CliError(format!("invalid line count {raw:?}")))?,
                );
            }
            "-o" | "--output" => {
                let raw = take_value(&mut args, inline, "--output")?;
                cli.output = OutputMode::parse(&raw)
                    .ok_or_else(|| CliError(format!("unknown output mode {raw:?}")))?;
            }
            "--file" => cli.file = Some(PathBuf::from(take_value(&mut args, inline, "--file")?)),
            _ => {
                if flag.starts_with('-') && flag != "-" {
                    return Err(CliError(format!("unknown option {flag:?}")));
                }
                push_positional(&mut cli, &arg)?;
            }
        }
    }

    Ok(cli)
}

fn push_positional(cli: &mut Cli, arg: &str) -> Result<(), CliError> {
    let predicate: FieldMatch = arg
        .parse()
        .map_err(|err: crate::query::QueryError| CliError(err.to_string()))?;
    cli.query.push_match(predicate);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::priority::Priority;

    fn parse(args: &[&str]) -> Cli {
        let now = 1_800_000_000;
        parse_args(args.iter().map(|s| (*s).to_owned()), now).unwrap()
    }

    fn parse_err(args: &[&str]) -> CliError {
        let now = 1_800_000_000;
        parse_args(args.iter().map(|s| (*s).to_owned()), now).unwrap_err()
    }

    #[test]
    fn defaults() {
        let cli = parse(&[]);
        assert_eq!(cli.output, OutputMode::Short);
        assert_eq!(cli.query, Query::new());
        assert_eq!(cli.lines, None);
        assert!(!cli.follow);
        assert!(!cli.list_boots);
        assert_eq!(cli.file, None);
        assert!(!cli.help);
        assert!(!cli.version);
    }

    #[test]
    fn unit_and_priority() {
        let cli = parse(&["-u", "smolnetd", "-p", "err"]);
        assert_eq!(cli.query.unit.as_deref(), Some("smolnetd"));
        assert_eq!(cli.query.priority, Some(Priority::Err));

        let cli = parse(&["--unit=getty", "--priority=debug"]);
        assert_eq!(cli.query.unit.as_deref(), Some("getty"));
        assert_eq!(cli.query.priority, Some(Priority::Debug));
    }

    #[test]
    fn glued_short_options() {
        let cli = parse(&["-n5", "-ugetty"]);
        assert_eq!(cli.lines, Some(5));
        assert_eq!(cli.query.unit.as_deref(), Some("getty"));

        let cli = parse(&["-fh"]);
        assert!(cli.follow);
        assert!(cli.help);
    }

    #[test]
    fn relative_times_use_now() {
        let cli = parse(&["--since", "-1h", "--until", "2026-10-15 00:00:00"]);
        assert_eq!(cli.query.since, Some(1_800_000_000 - 3600));
        assert_eq!(cli.query.until, Some(1_792_022_400));

        assert_eq!(parse(&["-S-1h"]).query.since, Some(1_800_000_000 - 3600));
    }

    #[test]
    fn invalid_time_reports_an_error() {
        assert!(parse_err(&["--since", "garbage"])
            .0
            .contains("invalid timestamp"));
    }

    #[test]
    fn output_modes() {
        assert_eq!(parse(&["-o", "json"]).output, OutputMode::Json);
        assert_eq!(parse(&["-o", "export"]).output, OutputMode::Export);
        assert_eq!(parse(&["--output=cat"]).output, OutputMode::Cat);
        assert!(parse_err(&["-o", "xml"]).0.contains("unknown output mode"));
    }

    #[test]
    fn boot_selection() {
        assert_eq!(parse(&["-b"]).query.boot, BootFilter::Current);

        let id = "00000000deadbeef123456789abcdef0";
        assert_eq!(parse(&["-b", id]).query.boot, BootFilter::Id(id.to_owned()));
        assert_eq!(
            parse(&["--boot", id]).query.boot,
            BootFilter::Id(id.to_owned())
        );

        // `-b` followed by a predicate keeps the current-boot default.
        let cli = parse(&["-b", "MESSAGE=hi"]);
        assert_eq!(cli.query.boot, BootFilter::Current);
        assert_eq!(cli.query.matches.len(), 1);
    }

    #[test]
    fn list_boots_and_follow_flags() {
        let cli = parse(&["--list-boots", "-f"]);
        assert!(cli.list_boots);
        assert!(cli.follow);
    }

    #[test]
    fn positional_predicates() {
        let cli = parse(&["MESSAGE~timeout", "PRIORITY!=6"]);
        assert_eq!(cli.query.matches.len(), 2);
        assert_eq!(cli.query.matches[0].field, "MESSAGE");
        assert_eq!(cli.query.matches[1].field, "PRIORITY");
    }

    #[test]
    fn separators_and_inversion() {
        let cli = parse(&["--invert", "--", "MESSAGE=hi"]);
        assert!(cli.query.inverted);
        assert_eq!(cli.query.matches.len(), 1);
    }

    #[test]
    fn help_and_version() {
        assert!(parse(&["-h"]).help);
        assert!(parse(&["--help"]).help);
        assert!(parse(&["--version"]).version);
    }

    #[test]
    fn file_override_and_grep_case() {
        let cli = parse(&["--file", "/tmp/j", "--grep", "Up", "--case-sensitive"]);
        assert_eq!(cli.file, Some(PathBuf::from("/tmp/j")));
        assert_eq!(cli.query.grep.as_deref(), Some("Up"));
        assert!(cli.query.grep_case_sensitive);
    }

    #[test]
    fn missing_values_and_unknown_flags_are_rejected() {
        assert!(parse_err(&["-u"]).0.contains("requires a value"));
        assert!(parse_err(&["--lines", "x"])
            .0
            .contains("invalid line count"));
        assert!(parse_err(&["--nope"]).0.contains("unknown option"));
        assert!(parse_err(&["-n5x"]).0.contains("invalid line count"));
        assert!(parse_err(&["-Z"]).0.contains("unknown option"));
        assert!(parse_err(&["-fZ"]).0.contains("unknown option"));
        assert!(parse_err(&["notapredicate"]).0.contains("expected FIELD"));
        assert!(parse_err(&["-p", "nope"]).0.contains("unknown priority"));
    }
}
