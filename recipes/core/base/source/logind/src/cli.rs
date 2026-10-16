//! Command line parsing for `loginctl`.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::client::DEFAULT_SCHEME;
use crate::utmp::DEFAULT_PATH;

/// Environment variable overriding the login record file.
pub const ENV_UTMP: &str = "LOGIND_UTMP";

/// Environment variable overriding the login manager mount point.
pub const ENV_SCHEME: &str = "LOGIND_SCHEME";

/// Help text.
pub const USAGE: &str = "\
loginctl — control the login manager (sessions and seats)

Usage:
  loginctl [OPTIONS...] COMMAND [NAME...]

Commands:
  list-sessions            List all current sessions
  list-seats               List all seats
  session-status [ID...]   Show terse runtime status of one or more sessions
  seat-status [NAME...]    Show terse runtime status of one or more seats
  show-session [ID...]     Show properties of one or more sessions
  show-seat [NAME...]      Show properties of one or more seats
  terminate-session ID...  Stop one or more sessions
  activate-session ID...   Give one or more sessions the console of their seat

Options:
      --scheme PATH        Login manager mount point (default $LOGIND_SCHEME or
                           /scheme/logind)
      --utmp PATH          Login record file (default $LOGIND_UTMP or
                           /var/run/utmp); naming a file reads it and ignores
                           the login manager
  -n, --no-legend          Do not print the column headers
  -h, --help               Show this help
      --version            Show package version

Sessions are read from the login manager when it is running, and from the
record file otherwise, so a listing works from a rescue shell too. Names may
also be given after `--`.
";

/// Commands, corresponding to the `loginctl` verbs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Command {
    /// No command given.
    #[default]
    None,
    /// `list-sessions`
    ListSessions,
    /// `list-seats`
    ListSeats,
    /// `session-status`
    SessionStatus,
    /// `seat-status`
    SeatStatus,
    /// `show-session`
    ShowSession,
    /// `show-seat`
    ShowSeat,
    /// `terminate-session`
    TerminateSession,
    /// `activate-session`
    ActivateSession,
}

impl Command {
    /// Parses a command word, including the short aliases `systemctl` tools
    /// usually accept.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "list-sessions" | "sessions" => Self::ListSessions,
            "list-seats" | "seats" => Self::ListSeats,
            "session-status" => Self::SessionStatus,
            "seat-status" => Self::SeatStatus,
            "show-session" | "session-show" => Self::ShowSession,
            "show-seat" | "seat-show" => Self::ShowSeat,
            "terminate-session" | "kill-session" => Self::TerminateSession,
            "activate-session" | "activate" => Self::ActivateSession,
            _ => return None,
        })
    }

    /// Whether the command takes session or seat names.
    pub fn takes_targets(self) -> bool {
        matches!(
            self,
            Self::SessionStatus
                | Self::SeatStatus
                | Self::ShowSession
                | Self::ShowSeat
                | Self::TerminateSession
                | Self::ActivateSession
        )
    }

    /// Whether the command needs at least one name.
    pub fn needs_targets(self) -> bool {
        matches!(self, Self::TerminateSession | Self::ActivateSession)
    }

    /// Whether the command changes the state of a session.
    pub fn is_action(self) -> bool {
        self.needs_targets()
    }
}

/// A parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    /// Command to run.
    pub command: Command,
    /// Session identifiers or seat names.
    pub targets: Vec<String>,
    /// Login record file to read.
    pub utmp: PathBuf,
    /// Whether the record file was named on the command line or in the
    /// environment: a named file is read instead of asking the login manager.
    pub utmp_given: bool,
    /// Login manager mount point.
    pub scheme: PathBuf,
    /// Whether to omit column headers.
    pub no_legend: bool,
    /// Whether `--help` was requested.
    pub help: bool,
    /// Whether `--version` was requested.
    pub version: bool,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            command: Command::None,
            targets: Vec::new(),
            utmp: PathBuf::from(DEFAULT_PATH),
            utmp_given: false,
            scheme: PathBuf::from(DEFAULT_SCHEME),
            no_legend: false,
            help: false,
            version: false,
        }
    }
}

/// Returned when the command line cannot be understood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

fn err<T>(message: impl Into<String>) -> Result<T, CliError> {
    Err(CliError(message.into()))
}

/// Parses `loginctl` arguments, using the environment for defaults.
pub fn parse_env_args<I>(args: I) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = parse_args(args)?;
    if cli.utmp == Path::new(DEFAULT_PATH) {
        if let Ok(path) = std::env::var(ENV_UTMP) {
            if !path.trim().is_empty() {
                cli.utmp = PathBuf::from(path);
                // A record file named by the environment is as explicit as one
                // named on the command line.
                cli.utmp_given = true;
            }
        }
    }
    if cli.scheme == Path::new(DEFAULT_SCHEME) {
        if let Ok(scheme) = std::env::var(ENV_SCHEME) {
            if !scheme.trim().is_empty() {
                cli.scheme = PathBuf::from(scheme);
            }
        }
    }
    Ok(cli)
}

/// Parses `loginctl` arguments.
pub fn parse_args<I>(args: I) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = Cli::default();
    let mut positional_only = false;

    let mut pending = args.into_iter().peekable();
    while let Some(arg) = pending.next() {
        if arg == "--" {
            positional_only = true;
            continue;
        }

        if !positional_only {
            if let Some(rest) = arg.strip_prefix("--") {
                let (name, value) = match rest.split_once('=') {
                    Some((name, value)) => (name, Some(value.to_owned())),
                    None => (rest, None),
                };

                match name {
                    "help" => {
                        cli.help = true;
                        return Ok(cli);
                    }
                    "version" => {
                        cli.version = true;
                        return Ok(cli);
                    }
                    "no-legend" => cli.no_legend = true,
                    "scheme" => {
                        let value = match value {
                            Some(value) => value,
                            None => match pending.next() {
                                Some(value) => value,
                                None => {
                                    return err(format!("option --{name} requires an argument"));
                                }
                            },
                        };
                        if value.is_empty() {
                            return err("--scheme requires a non-empty path");
                        }
                        cli.scheme = PathBuf::from(value);
                    }
                    "utmp" | "file" => {
                        let value = match value {
                            Some(value) => value,
                            None => match pending.next() {
                                Some(value) => value,
                                None => {
                                    return err(format!("option --{name} requires an argument"));
                                }
                            },
                        };
                        if value.is_empty() {
                            return err("--utmp requires a non-empty path");
                        }
                        cli.utmp = PathBuf::from(value);
                        cli.utmp_given = true;
                    }
                    _ => return err(format!("unknown option --{name}")),
                }
                continue;
            }

            if arg.len() > 1 && arg.starts_with('-') && arg != "-" {
                for short in arg.chars().skip(1) {
                    match short {
                        'h' => {
                            cli.help = true;
                            return Ok(cli);
                        }
                        'n' => cli.no_legend = true,
                        _ => return err(format!("unknown option -{short}")),
                    }
                }
                continue;
            }
        }

        // The first bare word is the command, later ones are names.
        if cli.command == Command::None {
            match Command::parse(&arg) {
                Some(command) => {
                    cli.command = command;
                    continue;
                }
                None => return err(format!("unknown command {arg:?}")),
            }
        }

        cli.targets.push(arg);
    }

    if cli.command == Command::None {
        return err("no command given");
    }
    if !cli.targets.is_empty() && !cli.command.takes_targets() {
        return err("this command does not take names");
    }
    if cli.targets.is_empty() && cli.command.needs_targets() {
        return err("this command needs at least one session");
    }

    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn commands_and_aliases_are_accepted() {
        assert_eq!(
            parse_args(args(&["list-sessions"])).unwrap().command,
            Command::ListSessions
        );
        assert_eq!(
            parse_args(args(&["sessions"])).unwrap().command,
            Command::ListSessions
        );
        assert_eq!(
            parse_args(args(&["list-seats"])).unwrap().command,
            Command::ListSeats
        );
        assert_eq!(
            parse_args(args(&["seats"])).unwrap().command,
            Command::ListSeats
        );
        assert_eq!(
            parse_args(args(&["session-status"])).unwrap().command,
            Command::SessionStatus
        );
        assert_eq!(
            parse_args(args(&["seat-show"])).unwrap().command,
            Command::ShowSeat
        );
        assert!(parse_args(args(&["bogus"])).is_err());
        assert!(parse_args(args(&[])).is_err());
    }

    #[test]
    fn targets_follow_the_command() {
        let cli = parse_args(args(&["session-status", "1", "2"])).unwrap();
        assert_eq!(cli.targets, vec!["1", "2"]);

        let cli = parse_args(args(&["seat-status", "--", "-weird"])).unwrap();
        assert_eq!(cli.targets, vec!["-weird"]);

        let cli = parse_args(args(&["list-seats", "--no-legend"])).unwrap();
        assert!(cli.no_legend);

        // Flags are order independent.
        let cli = parse_args(args(&["-n", "list-sessions"])).unwrap();
        assert!(cli.no_legend);
        assert_eq!(cli.command, Command::ListSessions);

        let cli = parse_args(args(&["-nh"])).unwrap();
        assert!(cli.no_legend);
        assert!(cli.help);
    }

    #[test]
    fn listing_commands_refuse_names() {
        assert!(parse_args(args(&["list-sessions", "1"])).is_err());
        assert!(parse_args(args(&["list-seats", "seat0"])).is_err());
    }

    #[test]
    fn the_utmp_file_can_be_selected() {
        let cli = parse_args(args(&["--utmp", "/tmp/utmp", "list-sessions"])).unwrap();
        assert_eq!(cli.utmp, PathBuf::from("/tmp/utmp"));
        assert!(
            cli.utmp_given,
            "a named file replaces the login manager as the source"
        );

        let cli = parse_args(args(&["--utmp=/tmp/other", "list-sessions"])).unwrap();
        assert_eq!(cli.utmp, PathBuf::from("/tmp/other"));

        let cli = parse_args(args(&["--file", "/tmp/third", "list-sessions"])).unwrap();
        assert_eq!(cli.utmp, PathBuf::from("/tmp/third"));

        assert!(parse_args(args(&["--utmp"])).is_err());
        assert!(parse_args(args(&["--utmp="])).is_err());

        let cli = parse_args(args(&["list-sessions"])).unwrap();
        assert_eq!(cli.utmp, PathBuf::from(DEFAULT_PATH));
        assert!(!cli.utmp_given, "by default the daemon is asked first");
        assert_eq!(cli.scheme, PathBuf::from(DEFAULT_SCHEME));
    }

    #[test]
    fn the_login_manager_can_be_located() {
        let cli = parse_args(args(&["--scheme", "/tmp/logind", "list-sessions"])).unwrap();
        assert_eq!(cli.scheme, PathBuf::from("/tmp/logind"));
        assert!(
            !cli.utmp_given,
            "locating the daemon does not select a file"
        );

        let cli = parse_args(args(&["--scheme=/tmp/other", "list-sessions"])).unwrap();
        assert_eq!(cli.scheme, PathBuf::from("/tmp/other"));

        assert!(parse_args(args(&["--scheme"])).is_err());
        assert!(parse_args(args(&["--scheme="])).is_err());
    }

    #[test]
    fn sessions_can_be_stopped_and_activated() {
        let cli = parse_args(args(&["terminate-session", "1"])).unwrap();
        assert_eq!(cli.command, Command::TerminateSession);
        assert_eq!(cli.targets, vec!["1"]);
        assert!(cli.command.is_action());

        for command in ["activate-session", "activate"] {
            let cli = parse_args(args(&[command, "tty1"])).unwrap();
            assert_eq!(cli.command, Command::ActivateSession);
            assert_eq!(cli.targets, vec!["tty1"]);
        }

        let cli = parse_args(args(&["kill-session", "1", "2"])).unwrap();
        assert_eq!(cli.command, Command::TerminateSession);
        assert_eq!(cli.targets, vec!["1", "2"]);

        // Those commands act on a session, so they need one.
        assert!(parse_args(args(&["terminate-session"])).is_err());
        assert!(parse_args(args(&["activate-session"])).is_err());
        assert!(parse_args(args(&["list-sessions", "1"])).is_err());
    }

    #[test]
    fn help_and_version_short_circuit() {
        let cli = parse_args(args(&["--help"])).unwrap();
        assert!(cli.help);
        assert_eq!(cli.command, Command::None);

        let cli = parse_args(args(&["--version"])).unwrap();
        assert!(cli.version);

        // `--help` wins over anything that follows it.
        let cli = parse_args(args(&["--help", "bogus"])).unwrap();
        assert!(cli.help);
    }

    #[test]
    fn unknown_options_are_reported() {
        let error = parse_args(args(&["--nope"])).unwrap_err();
        assert!(error.0.contains("--nope"));
        let error = parse_args(args(&["-z"])).unwrap_err();
        assert!(error.0.contains("-z"));
    }

    #[test]
    fn usage_documents_every_command() {
        for command in [
            "list-sessions",
            "list-seats",
            "session-status",
            "seat-status",
            "show-session",
            "show-seat",
            "terminate-session",
            "activate-session",
        ] {
            assert!(USAGE.contains(command), "usage is missing {command}");
        }
    }
}
