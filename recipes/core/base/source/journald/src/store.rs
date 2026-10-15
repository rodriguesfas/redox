//! Journal storage: an append-only list of entries with filtering, boot
//! tracking, size limits and vacuuming, plus the file format used on disk.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::entry::{JournalEntry, BOOT_ID};
use crate::query::Query;

/// Length of a boot identifier, in hex characters.
pub const BOOT_ID_LEN: usize = 32;

/// Formats a boot identifier from two 64-bit halves, matching the 32 character
/// lower-case hex form used by systemd.
pub fn format_boot_id(high: u64, low: u64) -> String {
    format!("{high:016x}{low:016x}")
}

/// True if `id` is a well-formed boot identifier.
pub fn is_valid_boot_id(id: &str) -> bool {
    id.len() == BOOT_ID_LEN && id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

/// An in-memory journal with optional capacity limits.
#[derive(Debug, Default)]
pub struct JournalStore {
    entries: Vec<JournalEntry>,
    current_boot: Option<String>,
    max_bytes: Option<usize>,
    bytes: usize,
}

impl JournalStore {
    /// Creates an unbounded store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a store that rotates once the encoded size exceeds `max_bytes`.
    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            max_bytes: Some(max_bytes),
            ..Self::default()
        }
    }

    /// The configured size limit, if any.
    pub fn max_bytes(&self) -> Option<usize> {
        self.max_bytes
    }

    /// The current encoded size of the journal.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Every entry, oldest first.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Number of stored entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing has been logged yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The boot currently being recorded.
    pub fn current_boot(&self) -> Option<&str> {
        self.current_boot.as_deref()
    }

    /// Marks the start of a new boot. Further entries are tagged with `id` by
    /// the daemon and `journalctl -b` selects them.
    pub fn begin_boot(&mut self, id: impl Into<String>) {
        self.current_boot = Some(id.into());
    }

    /// Appends an entry, rotating the journal if it grew past its limit.
    pub fn append(&mut self, entry: JournalEntry) {
        self.bytes += entry.encoded_len();
        self.entries.push(entry);
        self.rotate();
    }

    /// Appends a raw log line, parsed into an entry.
    pub fn append_line(&mut self, line: &str, source: &crate::syslog::Source) -> JournalEntry {
        let entry = crate::syslog::parse_line(line, source);
        self.append(entry.clone());
        entry
    }

    fn rotate(&mut self) {
        let Some(max_bytes) = self.max_bytes else {
            return;
        };
        while self.bytes > max_bytes && self.entries.len() > 1 {
            self.bytes -= self.entries.remove(0).encoded_len();
        }
    }

    /// Every entry matching `query`, oldest first. [`BootFilter::Current`] is
    /// resolved against [`JournalStore::current_boot`].
    pub fn query(&self, query: &Query) -> Vec<&JournalEntry> {
        let current_boot = self.current_boot.as_deref();
        self.entries
            .iter()
            .filter(|entry| query.matches_with(entry, current_boot))
            .collect()
    }

    /// The last `lines` entries matching `query`, oldest first.
    pub fn query_tail(&self, query: &Query, lines: usize) -> Vec<&JournalEntry> {
        let mut matched = self.query(query);
        if matched.len() > lines {
            matched.drain(..matched.len() - lines);
        }
        matched
    }

    /// Distinct boot identifiers, in order of first appearance.
    pub fn boot_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for entry in &self.entries {
            if let Some(id) = entry.boot_id() {
                if !ids.iter().any(|existing| existing == id) {
                    ids.push(id.to_owned());
                }
            }
        }
        ids
    }

    /// Total number of entries per boot identifier, in order of first
    /// appearance. Used by `journalctl --list-boots`.
    pub fn boot_summary(&self) -> Vec<(String, usize)> {
        let mut summary: Vec<(String, usize)> = Vec::new();
        for entry in &self.entries {
            let Some(id) = entry.boot_id() else { continue };
            match summary.iter_mut().find(|(existing, _)| existing == id) {
                Some((_, count)) => *count += 1,
                None => summary.push((id.to_owned(), 1)),
            }
        }
        summary
    }

    /// Drops entries strictly older than `before`. Returns how many were
    /// removed. Entries without a timestamp are kept.
    pub fn vacuum_before(&mut self, before: i64) -> usize {
        let mut removed = 0;
        self.entries.retain(|entry| {
            let stale = entry.realtime().is_some_and(|realtime| realtime < before);
            if stale {
                self.bytes -= entry.encoded_len();
                removed += 1;
            }
            !stale
        });
        removed
    }

    /// Drops the oldest entries until the journal fits in `max_bytes`,
    /// always keeping at least one entry.
    pub fn vacuum_to_bytes(&mut self, max_bytes: usize) -> usize {
        let mut removed = 0;
        while self.bytes > max_bytes && self.entries.len() > 1 {
            self.bytes -= self.entries.remove(0).encoded_len();
            removed += 1;
        }
        removed
    }

    /// Replaces the storage limit.
    pub fn set_max_bytes(&mut self, max_bytes: Option<usize>) {
        self.max_bytes = max_bytes;
        self.rotate();
    }

    /// Renders the whole journal in the export format.
    pub fn encode(&self) -> String {
        let mut out = String::with_capacity(self.bytes);
        for entry in &self.entries {
            out.push_str(&entry.encode());
        }
        out
    }

    /// Parses a journal from the export format.
    pub fn decode(input: &str) -> Result<JournalStore, crate::entry::EntryError> {
        let entries = JournalEntry::decode_all(input)?;
        let mut store = JournalStore::new();
        for entry in entries {
            store.append(entry);
        }
        store.adopt_latest_boot();
        Ok(store)
    }

    /// Treats the boot of the newest entry as the current one.
    ///
    /// A journal read from disk does not record which boot is running, so
    /// `journalctl -b` selects the most recent boot present in the file.
    fn adopt_latest_boot(&mut self) {
        if let Some(boot_id) = self
            .entries
            .iter()
            .rev()
            .find_map(|entry| entry.boot_id())
            .map(str::to_owned)
        {
            self.current_boot = Some(boot_id);
        }
    }

    /// Reads a journal file. A missing file yields an empty journal.
    pub fn read_from_path(path: &Path) -> io::Result<JournalStore> {
        match fs::read_to_string(path) {
            Ok(contents) => JournalStore::decode(&contents)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(JournalStore::new()),
            Err(err) => Err(err),
        }
    }

    /// Writes the whole journal to `path`, creating parent directories.
    pub fn write_to_path(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.encode())
    }

    /// Appends a single entry to `path`, creating the file if needed.
    pub fn append_to_path(path: &Path, entry: &JournalEntry) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(entry.encode().as_bytes())
    }
}

/// Parses the upper-case boot identifier from an entry, if present.
pub fn entry_boot_id(entry: &JournalEntry) -> Option<&str> {
    entry.get(BOOT_ID)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::entry::{MESSAGE, PRIORITY, REALTIME_TIMESTAMP, SYSLOG_IDENTIFIER};
    use crate::priority::Priority;
    use crate::query::BootFilter;
    use crate::syslog::Source;

    fn entry(
        boot: &str,
        identifier: &str,
        priority: u8,
        realtime: i64,
        message: &str,
    ) -> JournalEntry {
        let mut entry = JournalEntry::new();
        entry.set_field(BOOT_ID, boot);
        entry.set_field(SYSLOG_IDENTIFIER, identifier);
        entry.set_field(PRIORITY, priority.to_string());
        entry.set_field(REALTIME_TIMESTAMP, realtime.to_string());
        entry.set_field(MESSAGE, message);
        entry
    }

    fn populated() -> JournalStore {
        let mut store = JournalStore::new();
        store.begin_boot("boot-b");
        store.append(entry("boot-a", "init", 6, 100, "early boot"));
        store.append(entry("boot-a", "smolnetd", 4, 200, "link down"));
        store.append(entry("boot-b", "smolnetd", 3, 300, "dhcp failed"));
        store.append(entry("boot-b", "getty", 6, 400, "welcome"));
        store
    }

    #[test]
    fn boot_id_helpers() {
        let id = format_boot_id(0xdead_beef, 0x1234_5678_9abc_def0);
        assert_eq!(id, "00000000deadbeef123456789abcdef0");
        assert!(is_valid_boot_id(&id));
        assert!(!is_valid_boot_id("short"));
        assert!(!is_valid_boot_id(&id.to_uppercase()));
        assert!(!is_valid_boot_id(&"g".repeat(BOOT_ID_LEN)));
    }

    #[test]
    fn append_and_count() {
        let mut store = JournalStore::new();
        assert!(store.is_empty());
        let source = Source {
            boot_id: Some("boot-a".to_owned()),
            hostname: None,
            now: 10,
            ..Source::default()
        };
        let entry = store.append_line("init[1]: hello", &source);
        assert_eq!(entry.message(), Some("hello"));
        assert_eq!(store.len(), 1);
        assert_eq!(store.bytes(), entry.encoded_len());
        assert!(!store.is_empty());
    }

    #[test]
    fn query_filters_by_unit_priority_time_and_boot() {
        let store = populated();

        let all = store.query(&Query::new());
        assert_eq!(all.len(), 4);

        let smolnetd = store.query(&Query {
            unit: Some("smolnetd".to_owned()),
            ..Query::new()
        });
        assert_eq!(smolnetd.len(), 2);

        let errors = store.query(&Query {
            priority: Some(Priority::Err),
            ..Query::new()
        });
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message(), Some("dhcp failed"));

        let recent = store.query(&Query {
            since: Some(200),
            until: Some(300),
            ..Query::new()
        });
        assert_eq!(recent.len(), 2);

        let current = store.query(&Query {
            boot: BootFilter::Current,
            ..Query::new()
        });
        assert_eq!(current.len(), 2, "current boot is boot-b");

        let first_boot = store.query(&Query {
            boot: BootFilter::Id("boot-a".to_owned()),
            ..Query::new()
        });
        assert_eq!(first_boot.len(), 2);
    }

    #[test]
    fn tail_returns_the_newest_matching_entries() {
        let store = populated();
        let tail = store.query_tail(&Query::new(), 2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].message(), Some("dhcp failed"));
        assert_eq!(tail[1].message(), Some("welcome"));

        assert_eq!(store.query_tail(&Query::new(), 99).len(), 4);
        assert_eq!(store.query_tail(&Query::new(), 0).len(), 0);
    }

    #[test]
    fn boot_tracking() {
        let store = populated();
        assert_eq!(store.current_boot(), Some("boot-b"));
        assert_eq!(store.boot_ids(), vec!["boot-a", "boot-b"]);
        assert_eq!(
            store.boot_summary(),
            vec![("boot-a".to_owned(), 2), ("boot-b".to_owned(), 2)]
        );
        assert_eq!(entry_boot_id(&store.entries()[0]), Some("boot-a"));
    }

    #[test]
    fn begin_boot_switches_current_boot() {
        let mut store = populated();
        store.begin_boot("boot-c");
        assert_eq!(store.current_boot(), Some("boot-c"));
        store.append(entry("boot-c", "getty", 6, 500, "new session"));
        assert_eq!(store.query(&Query::default()).len(), 5);
        assert_eq!(store.boot_ids().len(), 3);
    }

    #[test]
    fn vacuum_before_drops_old_entries() {
        let mut store = populated();
        assert_eq!(store.vacuum_before(300), 2);
        assert_eq!(store.len(), 2);
        assert_eq!(store.entries()[0].message(), Some("dhcp failed"));
        assert_eq!(
            store.bytes(),
            store
                .entries()
                .iter()
                .map(|e| e.encoded_len())
                .sum::<usize>()
        );
        assert_eq!(store.vacuum_before(0), 0);
    }

    #[test]
    fn vacuum_keeps_untimestamped_entries() {
        let mut store = JournalStore::new();
        let mut entry = JournalEntry::new();
        entry.set_field(MESSAGE, "no timestamp");
        store.append(entry);
        assert_eq!(store.vacuum_before(i64::MAX), 0);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn size_limit_rotates_on_append() {
        let first = entry("boot-a", "init", 6, 1, "aaaaaaaaaaaaaaaaaaaa");
        let second = entry("boot-a", "init", 6, 2, "bbbbbbbbbbbbbbbbbbbb");
        let third = entry("boot-a", "init", 6, 3, "cccccccccccccccccccc");
        let limit = first.encoded_len() + second.encoded_len();

        let mut store = JournalStore::with_max_bytes(limit);
        assert_eq!(store.max_bytes(), Some(limit));
        store.append(first.clone());
        store.append(second.clone());
        assert_eq!(store.len(), 2);
        assert_eq!(store.bytes(), limit);

        store.append(third.clone());
        assert_eq!(store.len(), 2, "the oldest entry was rotated out");
        assert_eq!(store.entries()[0].message(), Some("bbbbbbbbbbbbbbbbbbbb"));
        assert_eq!(store.bytes(), second.encoded_len() + third.encoded_len());
    }

    #[test]
    fn vacuum_to_bytes_keeps_at_least_one_entry() {
        let mut store = populated();
        let removed = store.vacuum_to_bytes(1);
        assert_eq!(removed, 3);
        assert_eq!(store.len(), 1);
        assert_eq!(store.entries()[0].message(), Some("welcome"));
        assert_eq!(store.vacuum_to_bytes(1), 0);
    }

    #[test]
    fn encode_and_decode_roundtrip() {
        let store = populated();
        let document = store.encode();
        let restored = JournalStore::decode(&document).unwrap();
        assert_eq!(restored.len(), store.len());
        for (left, right) in store.entries().iter().zip(restored.entries()) {
            assert_eq!(left, right);
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("journald-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("journal")
    }

    #[test]
    fn file_roundtrip() {
        let path = temp_path("roundtrip");
        let store = populated();
        store.write_to_path(&path).unwrap();

        let read = JournalStore::read_from_path(&path).unwrap();
        assert_eq!(read.len(), 4);
        assert_eq!(read.entries()[3].message(), Some("welcome"));

        JournalStore::append_to_path(&path, &entry("boot-b", "getty", 6, 500, "appended")).unwrap();
        let read = JournalStore::read_from_path(&path).unwrap();
        assert_eq!(read.len(), 5);
        assert_eq!(read.entries()[4].message(), Some("appended"));

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn reading_a_missing_file_yields_an_empty_journal() {
        let path = temp_path("missing");
        let store = JournalStore::read_from_path(&path).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn reading_a_corrupt_file_fails() {
        let path = temp_path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not a journal line\n\n").unwrap();
        let err = JournalStore::read_from_path(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
