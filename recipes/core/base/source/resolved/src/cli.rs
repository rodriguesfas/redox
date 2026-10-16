//! Command line parsing for `resolvectl`.

use std::fmt;
use std::path::PathBuf;

use networkd::Ipv4Addr;

use crate::client::{DEFAULT_SCHEME, ENV_SCHEME};
use crate::state::{ENV_FILE, ENV_NETCFG};

/// Help text.
pub const USAGE: &str = "\
resolvectl — control the resolver of the system

Usage:
  resolvectl [OPTIONS...] COMMAND [ADDRESS]

Commands:
  status              Show the resolver, and where it comes from
  dns                 Show the resolvers a lookup would use
  update              Write the resolver of the network stack to the file
  set-dns ADDRESS     Set the resolver, in the network stack and in the file

Options:
      --file PATH     File to manage (default $RESOLVED_FILE or /etc/resolv.conf)
      --netcfg DIR    Where the network stack is mounted (default
                      $NETWORKD_NETCFG or /scheme/netcfg)
      --scheme DIR    Where the resolver daemon is mounted (default
                      $RESOLVED_SCHEME or /scheme/resolved)
      --dry-run       Print the change instead of doing it
  -n, --no-legend     Do not print the column headers
  -h, --help          Show this help
      --version       Show package version

The network stack holds the resolver the system learned (from DHCP or from a
`*.network` file); the file is what a program reads. `update` makes the second
follow the first, `set-dns` decides both. A file that is not there is a system
where nothing ever wrote a resolver, which is what `update` fixes.

When the resolver daemon is running it is the one that writes, so that the file
has a single writer; `resolvectl` says so when it answers. A file named on the
command line is written directly: the daemon manages the file of the system.
";

/// The commands of `resolvectl`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Command {
    /// No command given.
    #[default]
    None,
    /// `status`
    Status,
    /// `dns`
    Dns,
    /// `update`
    Update,
    /// `set-dns`
    SetDns,
}

impl Command {
    /// Parses a command word, including the names the other tools use.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "status" | "show" => Self::Status,
            "dns" | "list-dns" => Self::Dns,
            "update" | "sync" | "reload" => Self::Update,
            "set-dns" | "set-resolver" => Self::SetDns,
            _ => return None,
        })
    }

    /// Whether the command takes an address.
    pub fn takes_address(self) -> bool {
        self == Self::SetDns
    }

    /// Whether the command changes something.
    pub fn is_action(self) -> bool {
        matches!(self, Self::Update | Self::SetDns)
    }
}

/// A parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    /// Command to run.
    pub command: Command,
    /// The resolver given to `set-dns`.
    pub resolver: Option<Ipv4Addr>,
    /// The file that is managed.
    pub file: PathBuf,
    /// Whether the file is the one the command line or the environment named.
    pub file_given: bool,
    /// Where the network stack is mounted.
    pub netcfg: PathBuf,
    /// Whether the mount point is the one the command line or the environment
    /// named. The daemon writes to its own mounts, so a mount point that was
    /// named is never handed to it.
    pub netcfg_given: bool,
    /// Where the resolver daemon is mounted.
    pub scheme: PathBuf,
    /// Whether the change is printed instead of done.
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
            resolver: None,
            file: PathBuf::from(crate::resolv::FILE),
            file_given: false,
            netcfg: PathBuf::from(networkd::DEFAULT_ROOT),
            netcfg_given: false,
            scheme: PathBuf::from(crate::client::DEFAULT_SCHEME),
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

/// Parses `resolvectl` arguments, using the environment for defaults.
pub fn parse_env_args<I>(args: I) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut cli = parse_args(args)?;

    if !cli.file_given {
        if let Ok(file) = std::env::var(ENV_FILE) {
            if !file.trim().is_empty() {
                cli.file = PathBuf::from(file);
                cli.file_given = true;
            }
        }
    }
    if cli.netcfg == std::path::Path::new(networkd::DEFAULT_ROOT) {
        if let Ok(root) = std::env::var(ENV_NETCFG) {
            if !root.trim().is_empty() {
                cli.netcfg = PathBuf::from(root);
                cli.netcfg_given = true;
            }
        }
    }
    if cli.scheme == std::path::Path::new(DEFAULT_SCHEME) {
        if let Ok(scheme) = std::env::var(ENV_SCHEME) {
            if !scheme.trim().is_empty() {
                cli.scheme = PathBuf::from(scheme);
            }
        }
    }

    Ok(cli)
}

/// Parses `resolvectl` arguments.
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
                    "file" => {
                        cli.file = PathBuf::from(path(name, value)?);
                        cli.file_given = true;
                    }
                    "netcfg" => {
                        cli.netcfg = PathBuf::from(path(name, value)?);
                        cli.netcfg_given = true;
                    }
                    "scheme" => cli.scheme = PathBuf::from(path(name, value)?),
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

        if cli.command == Command::None {
            match Command::parse(&arg) {
                Some(command) => {
                    cli.command = command;
                    continue;
                }
                None => return err(format!("unknown command {arg:?}")),
            }
        }

        if !cli.command.takes_address() {
            return err("this command does not take an address");
        }
        if cli.resolver.is_some() {
            return err("this command takes one address at most");
        }
        cli.resolver = Some(arg.parse().map_err(|err| CliError(format!("{err}")))?);
    }

    if cli.command == Command::None {
        return err("no command given");
    }
    if cli.command == Command::SetDns && cli.resolver.is_none() {
        return err("set-dns needs an address");
    }
    if cli.dry_run && !cli.command.is_action() {
        return err("--dry-run means something for `update` and `set-dns` only");
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
        assert_eq!(Command::parse("status"), Some(Command::Status));
        assert_eq!(Command::parse("dns"), Some(Command::Dns));
        assert_eq!(Command::parse("update"), Some(Command::Update));
        assert_eq!(Command::parse("sync"), Some(Command::Update));
        assert_eq!(Command::parse("set-dns"), Some(Command::SetDns));
        assert_eq!(Command::parse("flush-caches"), None);
    }

    #[test]
    fn a_command_is_read_after_its_options() {
        let cli = parse(&["--dry-run", "update"]).unwrap();
        assert_eq!(cli.command, Command::Update);
        assert!(cli.dry_run);
        assert_eq!(cli.file, PathBuf::from(crate::resolv::FILE));
        assert!(!cli.file_given);
        assert_eq!(cli.netcfg, PathBuf::from(networkd::DEFAULT_ROOT));
        assert!(!cli.netcfg_given);
        assert_eq!(cli.scheme, PathBuf::from(DEFAULT_SCHEME));
        assert!(cli.command.is_action());
    }

    #[test]
    fn the_daemon_is_not_written_to_when_a_target_was_named() {
        // The daemon manages the mounts of the system: a command that names a
        // file or a mount point is about that one, and is answered directly.
        let cli = parse(&["update"]).unwrap();
        assert!(!cli.file_given && !cli.netcfg_given);
        assert!(!cli.dry_run);

        assert!(parse(&["--file", "/tmp/x", "update"]).unwrap().file_given);
        assert!(
            parse(&["--netcfg", "/tmp/x", "update"])
                .unwrap()
                .netcfg_given
        );
    }

    #[test]
    fn set_dns_takes_the_address() {
        let cli = parse(&["set-dns", "9.9.9.9"]).unwrap();
        assert_eq!(cli.command, Command::SetDns);
        assert_eq!(cli.resolver.unwrap().to_string(), "9.9.9.9");
        assert!(cli.command.takes_address());
    }

    #[test]
    fn the_paths_can_be_named_two_ways() {
        let cli = parse(&[
            "--file=/tmp/resolv.conf",
            "--netcfg",
            "/tmp/netcfg",
            "--scheme=/tmp/scheme",
            "status",
        ])
        .unwrap();
        assert_eq!(cli.file, PathBuf::from("/tmp/resolv.conf"));
        assert!(cli.file_given);
        assert_eq!(cli.netcfg, PathBuf::from("/tmp/netcfg"));
        assert!(cli.netcfg_given);
        assert_eq!(cli.scheme, PathBuf::from("/tmp/scheme"));
    }

    #[test]
    fn what_does_not_make_sense_is_refused() {
        for (args, message) in [
            (vec!["status", "9.9.9.9"], "does not take an address"),
            (vec!["set-dns"], "set-dns needs an address"),
            (vec!["set-dns", "banana"], "not an IPv4 address"),
            (vec!["set-dns", "9.9.9.9", "1.1.1.1"], "one address at most"),
            (vec!["flush-caches"], "unknown command"),
            (
                vec!["--frobnicate", "status"],
                "unknown option --frobnicate",
            ),
            (vec!["-x", "status"], "unknown option -x"),
            (vec![], "no command given"),
            (vec!["--file"], "requires an argument"),
            (vec!["--file=", "status"], "non-empty path"),
            (
                vec!["--dry-run", "dns"],
                "means something for `update` and `set-dns` only",
            ),
        ] {
            let error = parse(&args).unwrap_err();
            assert!(error.0.contains(message), "{args:?}: {error}");
        }
    }

    #[test]
    fn an_address_that_is_not_one_is_refused_before_anything_else() {
        let error = parse(&["set-dns", "10.0.0.300"]).unwrap_err();
        assert!(error.0.contains("octet out of range"), "{error}");
    }

    #[test]
    fn help_and_version_win_over_the_rest() {
        let cli = parse(&["--help", "update"]).unwrap();
        assert!(cli.help);
        assert_eq!(cli.command, Command::None);

        let cli = parse(&["--version"]).unwrap();
        assert!(cli.version);
        assert_eq!(cli.command, Command::None);
    }

    #[test]
    fn the_no_legend_flag_is_read_in_both_spellings() {
        assert!(parse(&["-n", "dns"]).unwrap().no_legend);
        assert!(parse(&["--no-legend", "dns"]).unwrap().no_legend);
        assert!(!parse(&["dns"]).unwrap().no_legend);
    }
}
