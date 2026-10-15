//! The daemon side: appending log lines to a journal file.
//!
//! `logd` owns `/scheme/log`, so it sees every line written by the kernel and
//! by the standard output of every service. Each line is parsed into an entry
//! and appended to a journal file, which [`crate::follow`] and `journalctl`
//! read back.
//!
//! The writer is deliberately defensive: it runs inside the boot-critical
//! logging daemon, so a journal that cannot be opened or written must never
//! stop logging, panic, or turn every line into a failing syscall. Failures are
//! reported once and retried, first eagerly and then at a low rate.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::cli::DEFAULT_JOURNAL_PATH;
use crate::entry::JournalEntry;
use crate::follow;
use crate::store::{format_boot_id, JournalStore};
use crate::syslog::{self, Source};
use crate::time::unix_now;

/// Journal size before the oldest entries are dropped.
pub const DEFAULT_MAX_BYTES: u64 = 1 << 20;

/// Environment variable selecting the journal file.
pub const ENV_PATH: &str = "JOURNAL_PATH";
/// Environment variable setting the size limit, in bytes.
pub const ENV_MAX_BYTES: &str = "JOURNAL_MAX_BYTES";
/// Environment variable giving the current boot identifier.
pub const ENV_BOOT_ID: &str = "JOURNAL_BOOT_ID";
/// Environment variable overriding the host name recorded in entries.
pub const ENV_HOSTNAME: &str = "JOURNAL_HOSTNAME";
/// Environment variable set to `0` to disable journaling entirely.
pub const ENV_ENABLED: &str = "JOURNAL_ENABLED";

/// How many times an unopenable journal is retried in a row before the retries
/// are spaced out.
const MAX_IMMEDIATE_ATTEMPTS: u32 = 8;
/// Retry rate once the immediate attempts are exhausted.
const RETRY_INTERVAL: u32 = 256;

/// Where and how to write the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalConfig {
    /// Journal file to append to.
    pub path: PathBuf,
    /// Size limit in bytes; the oldest entries are dropped past it.
    pub max_bytes: u64,
    /// Identifier of the current boot, if the caller already knows it.
    pub boot_id: Option<String>,
    /// Host name recorded in every entry.
    pub hostname: Option<String>,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from(DEFAULT_JOURNAL_PATH),
            max_bytes: DEFAULT_MAX_BYTES,
            boot_id: None,
            hostname: None,
        }
    }
}

impl JournalConfig {
    /// Reads the configuration from the process environment.
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the configuration from an arbitrary environment lookup.
    ///
    /// Returning `None` means journaling is disabled.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if let Some(value) = lookup(ENV_ENABLED) {
            let value = value.trim().to_ascii_lowercase();
            if matches!(value.as_str(), "0" | "no" | "off" | "false") {
                return None;
            }
        }

        let path = lookup(ENV_PATH)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_JOURNAL_PATH.to_owned());

        let max_bytes = lookup(ENV_MAX_BYTES)
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_BYTES);

        let boot_id = lookup(ENV_BOOT_ID).filter(|value| crate::store::is_valid_boot_id(value));
        let hostname = lookup(ENV_HOSTNAME).or_else(|| lookup("HOSTNAME"));

        Some(Self {
            path: PathBuf::from(path),
            max_bytes,
            boot_id,
            hostname,
        })
    }

    /// The source metadata every entry written with this configuration carries.
    pub fn source(&self) -> Source {
        Source {
            boot_id: self.boot_id.clone(),
            hostname: self.hostname.clone(),
            pid: None,
            now: 0,
        }
    }
}

/// The boot identifier of the newest entry in an existing journal file.
///
/// Only complete records are considered, so a journal interrupted mid-write is
/// still readable. Any error is treated as "no journal yet".
pub fn existing_boot_id(path: &Path) -> Option<String> {
    let contents = fs::read(path).ok()?;
    let (entries, _) = follow::decode_complete(&contents);
    entries
        .iter()
        .rev()
        .find_map(|entry| entry.boot_id())
        .map(str::to_owned)
}

/// Builds a boot identifier from a timestamp and a salt.
pub fn generate_boot_id(now: i64, salt: u64) -> String {
    format_boot_id(now.max(0) as u64, salt)
}

/// Appends parsed log lines to a journal file.
#[derive(Debug)]
pub struct JournalWriter {
    config: JournalConfig,
    boot_id: String,
    file: Option<File>,
    written: u64,
    failed_opens: u32,
    retries: u32,
    reported_error: bool,
}

impl JournalWriter {
    /// Opens (or reuses) a journal file, appending to it.
    ///
    /// When no boot identifier is configured, the one already recorded in the
    /// journal is reused: the journal lives in a volatile filesystem, so a
    /// journal found at startup belongs to the running boot. Only a genuinely
    /// new boot starts a new identifier.
    pub fn new(config: JournalConfig) -> Self {
        let boot_id = config.boot_id.clone().unwrap_or_else(|| {
            existing_boot_id(&config.path)
                .unwrap_or_else(|| generate_boot_id(unix_now(), u64::from(std::process::id())))
        });
        Self {
            config,
            boot_id,
            file: None,
            written: 0,
            failed_opens: 0,
            retries: 0,
            reported_error: false,
        }
    }

    /// Builds a writer from the environment, or `None` when disabled.
    pub fn from_env() -> Option<Self> {
        JournalConfig::from_env().map(Self::new)
    }

    /// The journal file.
    pub fn path(&self) -> &Path {
        &self.config.path
    }

    /// The size limit.
    pub fn max_bytes(&self) -> u64 {
        self.config.max_bytes
    }

    /// The identifier recorded in every entry.
    pub fn boot_id(&self) -> &str {
        &self.boot_id
    }

    /// Bytes appended so far, including anything already in the file.
    pub fn written_bytes(&self) -> u64 {
        self.written
    }

    /// Whether the journal file is currently open.
    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    /// Parses and appends one line, without its trailing newline.
    ///
    /// Returns whether the line reached the journal. Empty lines are ignored.
    pub fn write_line(&mut self, line: &str) -> bool {
        self.write_line_from(line, None)
    }

    /// Like [`JournalWriter::write_line`], recording the writing process.
    pub fn write_line_from(&mut self, line: &str, pid: Option<u32>) -> bool {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            return false;
        }

        let mut source = self.config.source();
        source.boot_id = Some(self.boot_id.clone());
        source.pid = pid;
        source.now = unix_now();

        let entry = syslog::parse_line(line, &source);
        self.append(&entry).is_ok()
    }

    /// Appends a ready-made entry.
    pub fn append(&mut self, entry: &JournalEntry) -> io::Result<()> {
        if !self.ensure_open() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                format!("journal {} is not available", self.config.path.display()),
            ));
        }

        let encoded = entry.encode();
        let result = match self.file.as_mut() {
            Some(file) => file
                .write_all(encoded.as_bytes())
                // Flushing keeps entries visible to a following reader, and
                // makes a crash lose at most the entry being written.
                .and_then(|()| file.flush()),
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "journal closed",
            )),
        };

        if let Err(err) = result {
            // Drop the handle so the next line reopens the journal.
            self.file = None;
            self.report(&err);
            return Err(err);
        }

        self.written += encoded.len() as u64;
        if self.written > self.config.max_bytes {
            // Rotation is best effort: a full journal must not lose the line
            // that was just written.
            let _ = self.compact();
        }

        Ok(())
    }

    /// Drops the oldest entries so the file fits in half the size limit.
    pub fn compact(&mut self) -> io::Result<usize> {
        self.file = None;

        let contents = match fs::read(&self.config.path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err),
        };

        // Only complete records are kept, so a journal interrupted mid-write is
        // recovered rather than rejected.
        let (entries, _) = follow::decode_complete(&contents);
        let mut store = JournalStore::new();
        for entry in entries {
            store.append(entry);
        }
        let removed = store.vacuum_to_bytes((self.config.max_bytes / 2).max(1) as usize);

        // The file is rewritten in place rather than replaced by a rename:
        // `/scheme` filesystems do not all support rename, and a reader that
        // sees the shorter file restarts from the beginning.
        {
            let mut file = File::create(&self.config.path)?;
            file.write_all(store.encode().as_bytes())?;
            file.flush()?;
        }
        self.written = store.bytes() as u64;
        if let Err(err) = self.open() {
            self.report(&err);
        }
        Ok(removed)
    }

    /// Opens the journal for appending, remembering its current size.
    fn open(&mut self) -> io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.config.path)?;
        self.written = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        self.file = Some(file);
        Ok(())
    }

    /// Ensures the journal is open, retrying failures without flooding.
    fn ensure_open(&mut self) -> bool {
        if self.file.is_some() {
            return true;
        }

        if self.failed_opens >= MAX_IMMEDIATE_ATTEMPTS {
            self.retries = self.retries.wrapping_add(1);
            if !self.retries.is_multiple_of(RETRY_INTERVAL) {
                return false;
            }
        }

        match self.open() {
            Ok(()) => {
                self.failed_opens = 0;
                self.retries = 0;
                true
            }
            Err(err) => {
                self.failed_opens = self.failed_opens.saturating_add(1);
                self.report(&err);
                false
            }
        }
    }

    fn report(&mut self, err: &io::Error) {
        if self.reported_error {
            return;
        }
        self.reported_error = true;
        eprintln!(
            "journald: cannot write {}: {err}",
            self.config.path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("journald-writer-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn config(path: PathBuf) -> JournalConfig {
        JournalConfig {
            path,
            max_bytes: DEFAULT_MAX_BYTES,
            boot_id: Some("00000000deadbeef123456789abcdef0".to_owned()),
            hostname: Some("aios".to_owned()),
        }
    }

    #[test]
    fn classified_lines_are_appended_as_structured_entries() {
        let dir = scratch("append");
        let path = dir.join("journal");

        let mut writer = JournalWriter::new(config(path.clone()));
        assert!(!writer.is_open());
        assert!(writer.write_line("<6>init[1]: Starting 10_smolnetd.service"));
        assert!(writer.write_line("smolnetd: interface eth0 up"));
        assert!(writer.write_line("<4>smolnetd: link down on eth0"));
        assert!(writer.is_open());
        assert!(writer.written_bytes() > 0);

        let store = JournalStore::read_from_path(&path).unwrap();
        assert_eq!(store.len(), 3);
        assert_eq!(store.boot_ids(), vec!["00000000deadbeef123456789abcdef0"]);
        assert_eq!(store.entries()[1].get("_HOSTNAME"), Some("aios"));
        assert_eq!(
            store.entries()[1].get("SYSLOG_IDENTIFIER"),
            Some("smolnetd")
        );
        assert_eq!(store.entries()[2].priority(), crate::Priority::Warning);
        assert_eq!(
            store.current_boot(),
            Some("00000000deadbeef123456789abcdef0")
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trailing_newlines_are_stripped_and_blank_lines_skipped() {
        let dir = scratch("crlf");
        let path = dir.join("journal");
        let mut writer = JournalWriter::new(config(path.clone()));

        // `logd` forwards lines terminated with CRLF.
        assert!(writer.write_line("getty[42]: welcome\r\n"));
        assert!(!writer.write_line("\r\n"));
        assert!(!writer.write_line(""));
        assert!(writer.write_line("getty[42]: second\r"));

        let store = JournalStore::read_from_path(&path).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.entries()[0].message(), Some("welcome"));
        assert_eq!(store.entries()[1].message(), Some("second"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writer_pid_is_recorded() {
        let dir = scratch("pid");
        let path = dir.join("journal");
        let mut writer = JournalWriter::new(config(path.clone()));

        assert!(writer.write_line_from("smolnetd: link up", Some(1234)));
        let store = JournalStore::read_from_path(&path).unwrap();
        assert_eq!(store.entries()[0].get("_PID"), Some("1234"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotation_keeps_the_journal_bounded_and_the_newest_lines() {
        let dir = scratch("rotate");
        let path = dir.join("journal");

        let mut writer = JournalWriter::new(JournalConfig {
            max_bytes: 512,
            ..config(path.clone())
        });

        for index in 0..64 {
            assert!(writer.write_line(&format!("logd[1]: entry {index}")));
        }

        let store = JournalStore::read_from_path(&path).unwrap();
        assert!(!store.is_empty(), "rotation must never empty the journal");
        assert!(store.len() < 64, "old entries should have been dropped");
        assert_eq!(
            store.entries().last().unwrap().message(),
            Some("entry 63"),
            "the newest entry must survive rotation"
        );
        // The file shrinks back towards the limit instead of growing forever.
        assert!(
            writer.written_bytes() <= 512,
            "journal grew to {} bytes",
            writer.written_bytes()
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_journal_is_not_fatal_and_is_retried() {
        let dir = scratch("missing");
        // The journal lives in a directory that does not exist yet.
        let path = dir.join("scheme-logging").join("journal");

        let mut writer = JournalWriter::new(config(path.clone()));
        assert!(!writer.write_line("logd[1]: too early"));
        assert!(!writer.is_open());

        // The log filesystem appears; the writer picks it up on the next line.
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        assert!(writer.write_line("logd[1]: now writable"));

        let store = JournalStore::read_from_path(&path).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.entries()[0].message(), Some("now writable"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unwritable_journal_does_not_loop_forever() {
        let dir = scratch("unwritable");
        let path = dir.join("missing").join("journal");
        let mut writer = JournalWriter::new(config(path));

        // Far more lines than the immediate attempt budget: the writer must
        // degrade to occasional retries instead of one failed open per line.
        assert!(!writer.write_line("a"));
        assert!(!writer.write_line("b"));
        assert!(!writer.write_line("c"));
        assert!(!writer.is_open());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compact_recovers_a_truncated_journal() {
        let dir = scratch("truncated");
        let path = dir.join("journal");

        let mut writer = JournalWriter::new(JournalConfig {
            max_bytes: 256,
            ..config(path.clone())
        });
        for index in 0..16 {
            writer.write_line(&format!("logd[1]: entry {index}"));
        }

        // A crash leaves a half-written record behind.
        let mut contents = fs::read(&path).unwrap();
        contents.extend_from_slice(b"MESSAGE=half written");
        fs::write(&path, &contents).unwrap();

        writer.compact().unwrap();

        let store = JournalStore::read_from_path(&path).unwrap();
        assert!(!store.is_empty());
        assert_eq!(store.entries().last().unwrap().message(), Some("entry 15"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn configuration_comes_from_the_environment() {
        let vars = |pairs: Vec<(&'static str, &'static str)>| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            }
        };

        let config = JournalConfig::from_lookup(vars(vec![])).unwrap();
        assert_eq!(config.path, PathBuf::from(DEFAULT_JOURNAL_PATH));
        assert_eq!(config.max_bytes, DEFAULT_MAX_BYTES);
        assert!(config.boot_id.is_none());

        let config = JournalConfig::from_lookup(vars(vec![
            (ENV_PATH, "/scheme/logging/journal"),
            (ENV_MAX_BYTES, "4096"),
            (ENV_HOSTNAME, "aios"),
        ]))
        .unwrap();
        assert_eq!(config.path, PathBuf::from("/scheme/logging/journal"));
        assert_eq!(config.max_bytes, 4096);
        assert_eq!(config.hostname.as_deref(), Some("aios"));

        // An invalid boot identifier is ignored rather than recorded.
        let config = JournalConfig::from_lookup(vars(vec![(ENV_BOOT_ID, "nope")])).unwrap();
        assert!(config.boot_id.is_none());

        // A zero size limit falls back to the default.
        let config = JournalConfig::from_lookup(vars(vec![(ENV_MAX_BYTES, "0")])).unwrap();
        assert_eq!(config.max_bytes, DEFAULT_MAX_BYTES);

        for value in ["0", "false", "OFF"] {
            assert!(JournalConfig::from_lookup(vars(vec![(ENV_ENABLED, value)])).is_none());
        }
        assert!(JournalConfig::from_lookup(vars(vec![(ENV_ENABLED, "1")])).is_some());
    }

    #[test]
    fn boot_id_is_generated_when_the_environment_has_none() {
        let mut config = config(PathBuf::from("/tmp/unused"));
        config.boot_id = None;
        let writer = JournalWriter::new(config);
        assert!(crate::store::is_valid_boot_id(writer.boot_id()));
        assert!(writer.boot_id() != generate_boot_id(1, 2));
    }

    #[test]
    fn a_restarted_daemon_keeps_the_boot_id_of_the_existing_journal() {
        let dir = scratch("boot-id");
        let path = dir.join("journal");

        let mut first = JournalWriter::new(config(path.clone()));
        first.write_line("logd[1]: first instance");
        let boot_id = first.boot_id().to_owned();

        // The journal is volatile, so a journal found at startup belongs to the
        // running boot: a restart must not look like a reboot to `journalctl -b`.
        let mut reopened = config(path.clone());
        reopened.boot_id = None;
        let second = JournalWriter::new(reopened);
        assert_eq!(second.boot_id(), boot_id);

        // An explicitly configured identifier still wins.
        let configured = JournalConfig {
            boot_id: Some("00000000deadbeef123456789abcde99".to_owned()),
            ..config(path.clone())
        };
        assert_eq!(
            JournalWriter::new(configured).boot_id(),
            "00000000deadbeef123456789abcde99"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn existing_boot_id_tolerates_a_partially_written_journal() {
        let dir = scratch("boot-id-partial");
        let path = dir.join("journal");

        fs::write(&path, "MESSAGE=half").unwrap();
        assert_eq!(existing_boot_id(&path), None);

        fs::write(&path, "garbage\n\n").unwrap();
        assert_eq!(existing_boot_id(&path), None);

        assert_eq!(existing_boot_id(&dir.join("missing")), None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reopening_an_existing_journal_appends_to_it() {
        let dir = scratch("reopen");
        let path = dir.join("journal");

        let mut first = JournalWriter::new(config(path.clone()));
        first.write_line("logd[1]: before restart");
        let size = first.written_bytes();

        // A restarted daemon must not truncate the journal nor lose track of
        // how much it holds.
        let mut second = JournalWriter::new(config(path.clone()));
        assert!(second.write_line("logd[1]: after restart"));
        assert!(
            second.written_bytes() > size,
            "a reopened journal must know about what it already holds"
        );

        let store = JournalStore::read_from_path(&path).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.entries()[0].message(), Some("before restart"));
        assert_eq!(store.entries()[1].message(), Some("after restart"));

        fs::remove_dir_all(&dir).unwrap();
    }
}
