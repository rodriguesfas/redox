//! Network management for AIOS/Redox: the network manager and its tool.
//!
//! `networkd` answers "what should the network interfaces of this system look
//! like, and what has to be written where for them to look like that". What
//! the configuration means is decided in pure logic that is tested on the host:
//!
//! - [`addr`]: the addresses, and what makes one unusable
//! - [`config`]: the `*.network` files, one per interface
//! - [`plan`]: the configuration resolved against the interfaces that are there
//! - [`netcfg`]: `/scheme/netcfg` as a file tree, read and written
//! - [`state`]: the configuration and the stack held together, applied once
//! - [`watch`]: the look that applies a configuration that changed
//! - [`protocol`], [`service`] and [`client`]: the daemon at
//!   `/scheme/networkd` — what it is asked, what it answers, and how
//!   `networkctl` asks
//! - [`cli`] and [`render`]: the `networkctl` command line and its output
//!
//! The tool is `networkctl`: `list` and `status` say what the system is doing,
//! `show` says what a configuration file was understood to mean, and `apply`
//! makes it true. Nothing here guesses: an interface that no file matches is
//! left to DHCP, which is how the system comes up today.
//!
//! The daemon `networkd` owns the write: a boot starts it as a scheme service,
//! and `networkctl apply` asks it when it is running, so the stack has one writer
//! and a plan read before a write cannot be applied after it. The daemon also
//! answers who owns the DHCP client: the client asks it what the configuration
//! decides about the interface it is about to configure, so a lease cannot write
//! over an address the configuration gave.

#![deny(clippy::precedence)]

pub mod addr;
pub mod cli;
pub mod client;
pub mod config;
pub mod netcfg;
pub mod plan;
pub mod protocol;
pub mod render;
pub mod service;
pub mod state;
pub mod watch;

#[cfg(target_os = "redox")]
pub mod scheme;

pub use addr::{Ipv4Addr, Ipv4Cidr, MacAddr, ParseError};
pub use cli::{parse_args, parse_env_args, Cli, CliError, Command, ENV_CONFIG, USAGE};
pub use client::{writes_through_the_daemon, Client, DEFAULT_SCHEME, ENV_SCHEME};
pub use config::{Config, ConfigError, Match, Network, NetworkFile, Route, CONFIG_DIR, FILE_EXT};
pub use netcfg::{Netcfg, NetcfgError, DEFAULT_ROOT, ENV_ROOT};
pub use plan::{InterfacePlan, Mode, Observed, Plan, Write};
pub use protocol::{
    format_response, is_interface_name, parse_response, Decision, Outcome, ProtocolError, Request,
};
pub use render::{
    render_applied, render_config, render_delegated, render_interfaces, render_status,
    render_status_document, summarize,
};
pub use service::Service;
pub use state::{shared, Application, Networkd, NetworkdError, SharedNetworkd};
pub use watch::{parse_interval, Watcher, DEFAULT_INTERVAL, ENV_INTERVAL};
