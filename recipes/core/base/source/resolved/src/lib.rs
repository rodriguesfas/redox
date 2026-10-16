//! The resolver configuration of AIOS/Redox: the file, the daemon that owns it,
//! and the tool that talks to it.
//!
//! When a program asks for a name, the answer starts with one line: the
//! `nameserver` the C library reads from `/etc/resolv.conf`. On Redox that file
//! is read by `relibc` (`get_dns_server` takes the **first** `nameserver` line
//! and nothing else) and, until `resolved`, nothing wrote it — DHCP learned a
//! resolver and the network stack kept it, but the file a program looks at was
//! never created, so a name could not be resolved at all.
//!
//! The pieces:
//!
//! - [`resolv`]: `/etc/resolv.conf` itself — tolerant parsing, canonical
//!   rendering, and what is wrong with a file
//! - [`state`]: the resolver the network stack holds and the file it has to
//!   agree with, and the operations that make them agree
//! - [`watch`]: the look that keeps the file in step with the stack while the
//!   system runs, because a lease arrives when it arrives
//! - [`protocol`], [`service`] and [`client`]: the daemon at
//!   `/scheme/resolved` — what it is asked, what it answers, and how
//!   `resolvectl` asks
//! - [`cli`] and [`render`]: the `resolvectl` command line and its output
//!
//! The file is written the way a generated file should be: only the
//! `nameserver` lines are ours, the rest of the file is kept as it was found.
//! There is one writer — the daemon, when it runs, and the tool otherwise —
//! and it writes only when the resolvers differ.

#![deny(clippy::precedence)]

pub mod cli;
pub mod client;
pub mod protocol;
pub mod render;
pub mod resolv;
pub mod service;
pub mod state;
pub mod watch;

#[cfg(target_os = "redox")]
pub mod scheme;

pub use cli::{parse_args, parse_env_args, Cli, CliError, Command, USAGE};
pub use client::{writes_through_the_daemon, Client, DEFAULT_SCHEME, ENV_SCHEME};
pub use protocol::{format_response, parse_response, Outcome, ProtocolError, Request};
pub use resolv::{parse_with_diagnostics, ResolvConf, FILE};
pub use service::Service;
pub use state::{
    shared, Resolved, ResolvedError, Stack, Status, Sync, ENV_FILE, ENV_NETCFG, NETCFG_PATH,
};
pub use watch::{parse_interval, Watcher, DEFAULT_INTERVAL, ENV_INTERVAL};
