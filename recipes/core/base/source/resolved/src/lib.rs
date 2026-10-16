//! The resolver configuration of AIOS/Redox.
//!
//! When a program asks for a name, the answer starts with one line: the
//! `nameserver` the C library reads from `/etc/resolv.conf`. On Redox that file
//! is read by `relibc` (`get_dns_server` takes the **first** `nameserver` line
//! and nothing else) and, until `resolved`, nothing wrote it — DHCP learned a
//! resolver and the network stack kept it, but the file a program looks at was
//! never created, so a name could not be resolved at all.
//!
//! This crate is the missing link in both directions:
//!
//! - [`resolv`]: `/etc/resolv.conf` itself — tolerant parsing, canonical
//!   rendering, and what is wrong with a file
//! - [`state`]: the resolver the network stack holds and the file it has to
//!   agree with, and the operations that make them agree
//! - [`cli`] and [`render`]: the `resolvectl` command line and its output
//!
//! The file is written the way a generated file should be: only the
//! `nameserver` lines are ours, the rest of the file is kept as it was found.

pub mod cli;
pub mod render;
pub mod resolv;
pub mod state;

pub use cli::{parse_args, parse_env_args, Cli, CliError, Command, USAGE};
pub use resolv::{parse_with_diagnostics, ResolvConf, FILE};
pub use state::{Resolved, ResolvedError, Stack, Status, Sync, ENV_FILE, ENV_NETCFG, NETCFG_PATH};
