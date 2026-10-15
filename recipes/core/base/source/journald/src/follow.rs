//! Reading a journal while it is being written: record boundaries, byte
//! offsets and the `journalctl --follow` loop.
//!
//! A journal on disk is a sequence of records, each terminated by an empty
//! line. A reader tailing the file must never consume a record that is still
//! being written, so only the prefix ending on a record boundary is decoded
//! and the offset advances by exactly that many bytes.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::cli::OutputMode;
use crate::entry::JournalEntry;
use crate::query::Query;
use crate::render::render;

/// Length in bytes of the longest prefix of `input` that ends on a record
/// boundary.
///
/// Records are separated by an empty line, so this is the offset just after the
/// last `\n\n`. A partially written trailing record is therefore never
/// returned, and becomes visible once its terminating blank line arrives.
pub fn complete_prefix_len(input: &[u8]) -> usize {
    match input.windows(2).rposition(|window| window == b"\n\n") {
        Some(index) => index + 2,
        None => 0,
    }
}

/// Decodes the complete records at the start of `input`.
///
/// Returns the entries and how many bytes were consumed. Records that cannot be
/// parsed are skipped rather than aborting the read: a logging daemon must not
/// stop following a journal because of one malformed record.
pub fn decode_complete(input: &[u8]) -> (Vec<JournalEntry>, usize) {
    let consumed = complete_prefix_len(input);
    if consumed == 0 {
        return (Vec::new(), 0);
    }

    let text = String::from_utf8_lossy(&input[..consumed]);
    let mut entries = Vec::new();
    for record in text.split("\n\n") {
        if record.trim().is_empty() {
            continue;
        }
        // Records are decoded one at a time so a bad one only loses itself.
        match JournalEntry::decode_all(record) {
            Ok(mut decoded) => entries.append(&mut decoded),
            Err(_) => continue,
        }
    }

    (entries, consumed)
}

/// A position in a journal file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tail {
    offset: u64,
}

impl Tail {
    /// A tail starting at the beginning of the journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// A tail starting at `offset`.
    pub fn from_offset(offset: u64) -> Self {
        Self { offset }
    }

    /// A tail starting at the current end of `path`.
    pub fn at_end(path: &Path) -> io::Result<Self> {
        Ok(Self::from_offset(file_len(path)?))
    }

    /// The current offset.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Reads every complete record written since the last call.
    ///
    /// A missing file yields no entries. If the file became shorter than the
    /// offset (the writer compacted or recreated it), reading restarts from the
    /// beginning.
    pub fn read_new(&mut self, path: &Path) -> io::Result<Vec<JournalEntry>> {
        let length = file_len(path)?;
        if self.offset > length {
            self.offset = 0;
        }
        if self.offset == length {
            return Ok(Vec::new());
        }

        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        let (entries, consumed) = decode_complete(&buffer);
        self.offset += consumed as u64;
        Ok(entries)
    }
}

fn file_len(path: &Path) -> io::Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err),
    }
}

/// `journalctl --follow`: prints matching entries as they are appended.
pub struct Follower {
    path: PathBuf,
    tail: Tail,
    query: Query,
    output: OutputMode,
    hostname: String,
}

impl Follower {
    /// Starts following `path` from its current end, so that only entries
    /// written after this point are printed.
    pub fn new(
        path: impl Into<PathBuf>,
        query: Query,
        output: OutputMode,
        hostname: impl Into<String>,
    ) -> io::Result<Self> {
        let path = path.into();
        Ok(Self {
            tail: Tail::at_end(&path)?,
            path,
            query,
            output,
            hostname: hostname.into(),
        })
    }

    /// Entries appended since the last poll that match the query.
    pub fn poll(&mut self) -> io::Result<Vec<JournalEntry>> {
        let entries = self.tail.read_new(&self.path)?;
        Ok(entries
            .into_iter()
            .filter(|entry| self.query.matches(&entry))
            .collect())
    }

    /// Polls and writes the new matching entries to `out`, returning how many
    /// were written. Nothing is written when there is no new entry.
    pub fn render(&mut self, out: &mut dyn Write) -> io::Result<usize> {
        let entries = self.poll()?;
        if entries.is_empty() {
            return Ok(0);
        }
        let borrowed: Vec<&JournalEntry> = entries.iter().collect();
        out.write_all(render(&borrowed, self.output, &self.hostname).as_bytes())?;
        out.flush()?;
        Ok(entries.len())
    }

    /// The offset of the next unread byte.
    pub fn offset(&self) -> u64 {
        self.tail.offset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::MESSAGE;
    use crate::priority::Priority;
    use crate::query::Query;
    use crate::store::JournalStore;
    use crate::syslog::Source;

    fn record(message: &str) -> String {
        let mut entry = JournalEntry::new();
        entry.set_field(MESSAGE, message);
        entry.encode()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("journald-follow-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("journal")
    }

    #[test]
    fn complete_prefix_stops_at_the_last_blank_line() {
        assert_eq!(complete_prefix_len(b""), 0);
        assert_eq!(complete_prefix_len(b"MESSAGE=partial\n"), 0);
        assert_eq!(complete_prefix_len(b"MESSAGE=done\n\n"), 14);
        let two = format!("{}{}", record("one"), record("two"));
        assert_eq!(complete_prefix_len(two.as_bytes()), two.len());
        // A trailing partial record is not counted.
        let partial = format!("{}MESSAGE=par", record("one"));
        let expected = record("one").len();
        assert_eq!(complete_prefix_len(partial.as_bytes()), expected);
    }

    #[test]
    fn decode_complete_ignores_the_partial_tail() {
        let document = format!("{}MESSAGE=par", record("one"));
        let (entries, consumed) = decode_complete(document.as_bytes());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), Some("one"));
        assert_eq!(consumed, record("one").len());
    }

    #[test]
    fn decode_complete_skips_malformed_records() {
        let document = format!("garbage line\n\n{}", record("good"));
        let (entries, consumed) = decode_complete(document.as_bytes());
        assert_eq!(consumed, document.len());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), Some("good"));
    }

    #[test]
    fn decode_complete_tolerates_multibyte_characters() {
        let document = record("ação concluída ✓");
        let (entries, consumed) = decode_complete(document.as_bytes());
        assert_eq!(consumed, document.len());
        assert_eq!(entries[0].message(), Some("ação concluída ✓"));
    }

    #[test]
    fn tail_reads_only_new_complete_records() {
        let path = scratch("tail");
        let mut tail = Tail::new();

        assert!(tail.read_new(&path).unwrap().is_empty(), "missing file");

        let mut file = File::create(&path).unwrap();
        // A partial record must not be returned yet.
        file.write_all(b"MESSAGE=first\n").unwrap();
        file.flush().unwrap();
        assert!(tail.read_new(&path).unwrap().is_empty());
        assert_eq!(tail.offset(), 0);

        // Completing the record makes it visible.
        file.write_all(b"\n").unwrap();
        file.flush().unwrap();
        let entries = tail.read_new(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), Some("first"));

        // Nothing new.
        assert!(tail.read_new(&path).unwrap().is_empty());

        // Two records at once.
        file.write_all(format!("{}{}", record("second"), record("third")).as_bytes())
            .unwrap();
        file.flush().unwrap();
        let entries = tail.read_new(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message(), Some("second"));
        assert_eq!(entries[1].message(), Some("third"));

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn tail_restarts_when_the_file_shrinks() {
        let path = scratch("shrink");
        fs::write(&path, record("one")).unwrap();

        let mut tail = Tail::new();
        assert_eq!(tail.read_new(&path).unwrap().len(), 1);
        assert!(tail.offset() > 0);

        // Compaction replaced the journal with a shorter one.
        fs::write(&path, record("x")).unwrap();
        let entries = tail.read_new(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), Some("x"));

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn follower_only_reports_entries_written_after_it_started() {
        let path = scratch("follower");
        fs::write(&path, record("before")).unwrap();

        let mut follower = Follower::new(&path, Query::new(), OutputMode::Short, "aios").unwrap();

        let mut out = Vec::new();
        assert_eq!(follower.render(&mut out).unwrap(), 0);
        assert!(out.is_empty());

        let source = Source::bare(1_792_022_400);
        let mut store = JournalStore::new();
        store.append_line("<4>smolnetd: link down", &source);
        fs::write(&path, format!("{}{}", record("before"), store.encode())).unwrap();

        assert_eq!(follower.render(&mut out).unwrap(), 1);
        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("link down"));
        assert!(!rendered.contains("before"));

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn follower_applies_the_query() {
        let path = scratch("follower-query");
        fs::write(&path, "").unwrap();

        let query = Query {
            priority: Some(Priority::Err),
            ..Query::new()
        };
        let mut follower = Follower::new(&path, query, OutputMode::Cat, "aios").unwrap();

        let source = Source::bare(1_792_022_400);
        let mut store = JournalStore::new();
        store.append_line("<6>smolnetd: informational", &source);
        store.append_line("<3>dhcpd: offer timed out", &source);
        fs::write(&path, store.encode()).unwrap();

        let mut out = Vec::new();
        assert_eq!(follower.render(&mut out).unwrap(), 1);
        assert_eq!(String::from_utf8(out).unwrap(), "offer timed out\n");

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
