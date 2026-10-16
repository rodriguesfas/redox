//! Command line parsing for `networkctl`.

use std::fmt;
use std::path::PathBuf;

use crate::config::CONFIG_DIR;
use crate::netcfg::{DEFAULT_ROOT, ENV_ROOT};

/// Environment variable overriding the configuration directory.
pub const ENV_CONFIG: &str = "NETWORKD_CONFIG";

/// Help text.
pub const USAGE: &str = "\
networkctl — control the network interfaces of the system

Usage:
  networkctl [OPTIONS...] COMMAND [NAME]

Commands:
  list                    List the interfaces the network stack serves
  status [INTERFACE]      Show what the configuration makes the stack do
  show [FILE]             Show a configuration file as it was understood
  apply                   Write the configuration to the network stack

Options:
      --config-dir DIR    Directory of *.network files (default
                          $NETWORKD_CONFIG or /etc/networkd)
      --netcfg DIR        Where the network stack is mounted (default
                          $NETWORKD_NETCFG or /scheme/netcfg)
      --dry-run           Print the writes instead of applying them
  -n, --no-legend         Do not print the column headers
  -h, --help              Show this help
      --version           Show package version

An interface no file matches is left to DHCP, the way the system comes up
today. `status` and `show` never touch the network stack: they read the
configuration and report the writes it would make.
";

/// The commands of `networkctl`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Command {
    /// No command given.
    #[default]
    None,
    /// `list`
    List,
    /// `status`
    Status,
    /// `show`
    Show,
    /// `apply`
    Apply,
}

impl Command {
    /// Parses a command word, including the aliases the other `*ctl` tools
    /// usually accept.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "list" | "list-interfaces" | "list-links" => Self::List,
            "status" | "show-status" => Self::Status,
            "show" | "show-config" | "cat" => Self::Show,
            "apply" | "reload" => Self::Apply,
            _ => return None,
        })
    }

    /// Whether the command takes an interface name or a file.
    pub fn takes_target(self) -> bool {
        matches!(self, Self::Status | Self::Show)
    }

    /// Whether the command changes the network.
    pub fn is_action(self) -> bool {
        self == Self::Apply
    }
}

/// A parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    /// Command to run.
    pub command: Command,
    /// The interface to report on, or the file to read.
    pub target: Option<String>,
    /// Directory of `*.network` files.
    pub config_dir: PathBuf,
    /// Whether the configuration directory was named on the command line or in
    /// the environment.
    pub config_dir_given: bool,
    /// Where the network stack is mounted.
    pub netcfg: PathBuf,
    /// Whether the writes are printed instead of applied.
    pub dry_run: bool,
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
            target: None,
            config_dir: PathBuf::from(CONFIG_DIR),
            config_dir_given: false,
            netcfg: PathBuf::from(DEFAULT_ROOT),
            dry_run: false,
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

/// Parses `networkctl` arguments, using the environment for defaults.
pub fn parse_env_args<I>(args: I) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = parse_args(args)?;

    if !cli.config_dir_given {
        if let Ok(dir) = std::env::var(ENV_CONFIG) {
            if !dir.trim().is_empty() {
                cli.config_dir = PathBuf::from(dir);
                cli.config_dir_given = true;
            }
        }
    }
    if cli.netcfg == std::path::Path::new(DEFAULT_ROOT) {
        if let Ok(root) = std::env::var(ENV_ROOT) {
            if !root.trim().is_empty() {
                cli.netcfg = PathBuf::from(root);
            }
        }
    }

    Ok(cli)
}

/// Parses `networkctl` arguments.
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

                // The value of an option that takes one, from `=` or from the
                // next argument.
                let mut path = |name: &str, value: Option<String>| -> Result<String, CliError> {
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
                        return err(format!("--{name} requires a non-empty path"));
                    }
                    Ok(value)
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
                    "dry-run" => cli.dry_run = true,
                    "config-dir" => {
                        cli.config_dir = PathBuf::from(path(name, value)?);
                        cli.config_dir_given = true;
                    }
                    "netcfg" => cli.netcfg = PathBuf::from(path(name, value)?),
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

        // The first bare word is the command, a later one is its name.
        if cli.command == Command::None {
            match Command::parse(&arg) {
                Some(command) => {
                    cli.command = command;
                    continue;
                }
                None => return err(format!("unknown command {arg:?}")),
            }
        }

        if !cli.command.takes_target() {
            return err("this command does not take a name");
        }
        if cli.target.is_some() {
            return err("this command takes one name at most");
        }
        cli.target = Some(arg);
    }

    if cli.command == Command::None {
        return err("no command given");
    }
    if cli.dry_run && cli.command != Command::Apply {
        return err("--dry-run means something for `apply` only");
    }

    Ok(cli)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, CliError> {
        parse_args(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn the_commands_and_their_aliases_are_known() {
        assert_eq!(Command::parse("list"), Some(Command::List));
        assert_eq!(Command::parse("list-interfaces"), Some(Command::List));
        assert_eq!(Command::parse("status"), Some(Command::Status));
        assert_eq!(Command::parse("show"), Some(Command::Show));
        assert_eq!(Command::parse("cat"), Some(Command::Show));
        assert_eq!(Command::parse("apply"), Some(Command::Apply));
        assert_eq!(Command::parse("reload"), Some(Command::Apply));
        assert_eq!(Command::parse("reboot"), None);
    }

    #[test]
    fn a_command_is_read_after_its_options() {
        let cli = parse(&["--no-legend", "list"]).unwrap();
        assert_eq!(cli.command, Command::List);
        assert!(cli.no_legend);
        assert_eq!(cli.config_dir, PathBuf::from(CONFIG_DIR));
        assert_eq!(cli.netcfg, PathBuf::from(DEFAULT_ROOT));
    }

    #[test]
    fn a_status_can_name_an_interface() {
        let cli = parse(&["status", "eth0"]).unwrap();
        assert_eq!(cli.command, Command::Status);
        assert_eq!(cli.target.as_deref(), Some("eth0"));

        let cli = parse(&["status"]).unwrap();
        assert_eq!(cli.target, None);
    }

    #[test]
    fn the_paths_can_be_named_two_ways() {
        let cli = parse(&["--config-dir=/tmp/conf", "--netcfg", "/tmp/netcfg", "show"]).unwrap();
        assert_eq!(cli.config_dir, PathBuf::from("/tmp/conf"));
        assert!(cli.config_dir_given);
        assert_eq!(cli.netcfg, PathBuf::from("/tmp/netcfg"));
    }

    #[test]
    fn what_does_not_make_sense_is_refused() {
        for (args, message) in [
            (vec!["list", "eth0"], "does not take a name"),
            (vec!["status", "eth0", "wlan0"], "one name at most"),
            (vec!["reboot"], "unknown command"),
            (vec!["--frobnicate", "list"], "unknown option --frobnicate"),
            (vec!["-x", "list"], "unknown option -x"),
            (vec![], "no command given"),
            (vec!["--config-dir"], "requires an argument"),
            (vec!["--config-dir=", "list"], "non-empty path"),
            (vec!["--netcfg"], "requires an argument"),
            (
                vec!["--dry-run", "list"],
                "--dry-run means something for `apply` only",
            ),
        ] {
            let error = parse(&args).unwrap_err();
            assert!(error.0.contains(message), "{args:?}: {error}");
        }
    }

    #[test]
    fn apply_takes_the_dry_run_flag() {
        let cli = parse(&["apply", "--dry-run"]).unwrap();
        assert_eq!(cli.command, Command::Apply);
        assert!(cli.dry_run);
        assert!(cli.command.is_action());
    }

    #[test]
    fn help_and_version_win_over_the_rest() {
        let cli = parse(&["--help", "list"]).unwrap();
        assert!(cli.help);
        assert_eq!(cli.command, Command::None);

        let cli = parse(&["--version"]).unwrap();
        assert!(cli.version);
        assert_eq!(cli.command, Command::None);
    }

    #[test]
    fn a_name_after_the_separator_is_still_a_name() {
        let cli = parse(&["status", "--", "--weird-name"]).unwrap();
        assert_eq!(cli.target.as_deref(), Some("--weird-name"));
    }
}
