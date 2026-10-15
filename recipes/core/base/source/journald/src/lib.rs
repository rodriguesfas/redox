//! Structured journal for AIOS/Redox: entry format, syslog parsing, filters,
//! storage and the `journalctl` front end.
//!
//! The crate is split so that everything except process spawning and the
//! scheme plumbing is pure logic that can be tested on the host:
//!
//! - [`entry`]: `FIELD=value` records and their text/JSON encodings
//! - [`syslog`]: turning plain log lines into structured entries
//! - [`priority`]: syslog severities and their names
//! - [`query`]: the filter language behind `journalctl -u/-p/--since`
//! - [`time`]: timestamp parsing and formatting
//! - [`store`]: in-memory and on-disk journals, rotation and vacuuming
//! - [`cli`] and [`render`]: the `journalctl` command line and its output

#![deny(clippy::precedence)]

pub mod cli;
pub mod entry;
pub mod priority;
pub mod query;
pub mod render;
pub mod store;
pub mod syslog;
pub mod time;

pub use cli::{parse_args, Cli, CliError, OutputMode, DEFAULT_JOURNAL_PATH, USAGE};
pub use entry::{EntryError, JournalEntry};
pub use priority::Priority;
pub use query::{BootFilter, FieldMatch, MatchOp, Query};
pub use store::{format_boot_id, is_valid_boot_id, JournalStore};
pub use syslog::Source;
