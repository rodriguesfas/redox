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
//! - [`cli`] and [`render`]: the `networkctl` command line and its output
//!
//! The tool is `networkctl`: `list` and `status` say what the system is doing,
//! `show` says what a configuration file was understood to mean, and `apply`
//! makes it true. Nothing here guesses: an interface that no file matches is
//! left to DHCP, which is how the system comes up today.

#![deny(clippy::precedence)]

pub mod addr;
pub mod cli;
pub mod config;
pub mod netcfg;
pub mod plan;
pub mod render;

pub use addr::{Ipv4Addr, Ipv4Cidr, MacAddr, ParseError};
pub use cli::{parse_args, parse_env_args, Cli, CliError, Command, ENV_CONFIG, USAGE};
pub use config::{Config, ConfigError, Match, Network, NetworkFile, Route, CONFIG_DIR, FILE_EXT};
pub use netcfg::{Netcfg, NetcfgError, DEFAULT_ROOT, ENV_ROOT};
pub use plan::{Mode, Observed, Plan, Write};
pub use render::{render_applied, render_config, render_interfaces, render_status, summarize};
