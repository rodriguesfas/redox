//! The classic `utmp`/`utmpx` record format and the file that holds it.
//!
//! `logind` keeps `/var/run/utmp` up to date so that the standard tools that
//! look for login records (`who`, `last`, `w`) find them, and `loginctl` reads
//! the same file so it works with or without the daemon running.
//!
//! The layout is the fixed 384-byte record used by glibc and musl on every
//! 32/64-bit architecture: a 32-bit pid, a 32-bit session id and a 32-bit
//! `timeval`, so a file written on one architecture is readable on another.
//! Padding is explicit here, and every field is encoded little-endian to match
//! every port that ships `utmp`.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Size of one record, fixed for cross-architecture compatibility.
pub const RECORD_SIZE: usize = 384;
/// `ut_line` length.
pub const LINE_SIZE: usize = 32;
/// `ut_id` length.
pub const ID_SIZE: usize = 4;
/// `ut_user` length.
pub const NAME_SIZE: usize = 32;
/// `ut_host` length.
pub const HOST_SIZE: usize = 256;

/// Default location, matching what portable tools read.
pub const DEFAULT_PATH: &str = "/var/run/utmp";

const OFF_TYPE: usize = 0;
const OFF_PID: usize = 4;
const OFF_LINE: usize = 8;
const OFF_ID: usize = OFF_LINE + LINE_SIZE;
const OFF_USER: usize = OFF_ID + ID_SIZE;
const OFF_HOST: usize = OFF_USER + NAME_SIZE;
const OFF_EXIT: usize = OFF_HOST + HOST_SIZE;
const OFF_SESSION: usize = OFF_EXIT + 4;
const OFF_TIME: usize = OFF_SESSION + 4;
const OFF_ADDR: usize = OFF_TIME + 8;

/// Record kinds, with the numeric values used on disk.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
#[repr(i16)]
pub enum RecordType {
    /// Unused record.
    #[default]
    Empty = 0,
    /// Change of run level.
    RunLevel = 1,
    /// Time of a reboot.
    BootTime = 2,
    /// Time of a clock change.
    NewTime = 3,
    /// Time of a clock change back.
    OldTime = 4,
    /// Process spawned by init, before login.
    InitProcess = 5,
    /// `getty` waiting for a login name.
    LoginProcess = 6,
    /// A logged-in user.
    UserProcess = 7,
    /// A session that ended.
    DeadProcess = 8,
}

impl RecordType {
    /// Converts the on-disk value into a record type.
    pub fn from_code(code: i16) -> Option<Self> {
        Some(match code {
            0 => Self::Empty,
            1 => Self::RunLevel,
            2 => Self::BootTime,
            3 => Self::NewTime,
            4 => Self::OldTime,
            5 => Self::InitProcess,
            6 => Self::LoginProcess,
            7 => Self::UserProcess,
            8 => Self::DeadProcess,
            _ => return None,
        })
    }

    /// The on-disk value.
    pub fn code(self) -> i16 {
        self as i16
    }

    /// Lower-case name, as used in listings.
    pub fn name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::RunLevel => "runlevel",
            Self::BootTime => "boot",
            Self::NewTime => "new time",
            Self::OldTime => "old time",
            Self::InitProcess => "init",
            Self::LoginProcess => "login",
            Self::UserProcess => "user",
            Self::DeadProcess => "dead",
        }
    }

    /// Whether this record describes a logged-in user.
    pub fn is_user(self) -> bool {
        self == Self::UserProcess
    }
}

impl fmt::Display for RecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Thrown when a record cannot be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UtmpError(pub String);

impl fmt::Display for UtmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid utmp record: {}", self.0)
    }
}

impl std::error::Error for UtmpError {}

/// One login record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UtmpRecord {
    /// What the record describes.
    pub record_type: RecordType,
    /// Process id of the session leader.
    pub pid: i32,
    /// Terminal name, without `/dev/`.
    pub line: String,
    /// Short terminal suffix, e.g. `t1` for `tty1`.
    pub id: String,
    /// Login name.
    pub user: String,
    /// Remote host, when the login is not local.
    pub host: String,
    /// `(termination, exit)` of a finished process.
    pub exit_status: (i16, i16),
    /// Session id, used for windowing.
    pub session: i32,
    /// Seconds since the epoch when the record was written.
    pub time: i64,
    /// Microseconds of `time`.
    pub usec: i32,
    /// Remote IPv6 address, all zero for local logins.
    pub addr_v6: [i32; 4],
}

impl UtmpRecord {
    /// An empty record.
    pub fn new() -> Self {
        Self::default()
    }

    /// A record marking the start of a boot.
    pub fn boot_time(pid: i32, now: i64) -> Self {
        Self {
            record_type: RecordType::BootTime,
            pid,
            time: now,
            ..Self::default()
        }
    }

    /// A `USER_PROCESS` record for a logged-in user.
    pub fn user_process(
        pid: i32,
        line: impl Into<String>,
        user: impl Into<String>,
        now: i64,
    ) -> Self {
        let line = line.into();
        Self {
            record_type: RecordType::UserProcess,
            pid,
            id: terminal_id(&line),
            line,
            user: user.into(),
            time: now,
            ..Self::default()
        }
    }

    /// A `DEAD_PROCESS` record for the given line.
    pub fn dead_process(line: impl Into<String>, now: i64) -> Self {
        let line = line.into();
        Self {
            record_type: RecordType::DeadProcess,
            id: terminal_id(&line),
            line,
            time: now,
            ..Self::default()
        }
    }

    /// Encodes the record into its fixed-size on-disk form.
    pub fn encode(&self) -> [u8; RECORD_SIZE] {
        let mut buf = [0u8; RECORD_SIZE];
        put_i16(&mut buf, OFF_TYPE, self.record_type.code());
        put_i32(&mut buf, OFF_PID, self.pid);
        put_str(&mut buf, OFF_LINE, LINE_SIZE, &self.line);
        put_str(&mut buf, OFF_ID, ID_SIZE, &self.id);
        put_str(&mut buf, OFF_USER, NAME_SIZE, &self.user);
        put_str(&mut buf, OFF_HOST, HOST_SIZE, &self.host);
        put_i16(&mut buf, OFF_EXIT, self.exit_status.0);
        put_i16(&mut buf, OFF_EXIT + 2, self.exit_status.1);
        put_i32(&mut buf, OFF_SESSION, self.session);
        put_i32(&mut buf, OFF_TIME, self.time as i32);
        put_i32(&mut buf, OFF_TIME + 4, self.usec);
        for (index, value) in self.addr_v6.iter().enumerate() {
            put_i32(&mut buf, OFF_ADDR + index * 4, *value);
        }
        buf
    }

    /// Decodes one record.
    pub fn decode(bytes: &[u8]) -> Result<Self, UtmpError> {
        if bytes.len() < RECORD_SIZE {
            return Err(UtmpError(format!(
                "expected {RECORD_SIZE} bytes, got {}",
                bytes.len()
            )));
        }

        let code = get_i16(bytes, OFF_TYPE);
        let record_type = RecordType::from_code(code)
            .ok_or_else(|| UtmpError(format!("unknown record type {code}")))?;

        let mut addr_v6 = [0i32; 4];
        for (index, value) in addr_v6.iter_mut().enumerate() {
            *value = get_i32(bytes, OFF_ADDR + index * 4);
        }

        Ok(Self {
            record_type,
            pid: get_i32(bytes, OFF_PID),
            line: get_str(bytes, OFF_LINE, LINE_SIZE),
            id: get_str(bytes, OFF_ID, ID_SIZE),
            user: get_str(bytes, OFF_USER, NAME_SIZE),
            host: get_str(bytes, OFF_HOST, HOST_SIZE),
            exit_status: (get_i16(bytes, OFF_EXIT), get_i16(bytes, OFF_EXIT + 2)),
            session: get_i32(bytes, OFF_SESSION),
            time: i64::from(get_i32(bytes, OFF_TIME)),
            usec: get_i32(bytes, OFF_TIME + 4),
            addr_v6,
        })
    }

    /// Whether the record describes a logged-in user.
    pub fn is_user(&self) -> bool {
        self.record_type.is_user()
    }

    /// Whether two records describe the same terminal.
    ///
    /// Follows `getutid`: records are matched by terminal id, so a `DEAD_PROCESS`
    /// replaces the `USER_PROCESS` it terminates.
    pub fn same_terminal(&self, other: &Self) -> bool {
        !self.id.is_empty() && self.id == other.id
    }
}

/// The short terminal suffix kept in `ut_id`: the digits of `tty1` are `1`.
///
/// Ports use this to find the record for a terminal without storing the whole
/// device name, and it is what `pututline` matches on.
pub fn terminal_id(line: &str) -> String {
    let trimmed = line.rsplit('/').next().unwrap_or(line);
    let suffix: String = trimmed
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .collect();
    let suffix = suffix.trim_end_matches(char::is_alphabetic);
    if suffix.is_empty() {
        trimmed.chars().take(ID_SIZE).collect()
    } else {
        suffix.chars().take(ID_SIZE).collect()
    }
}

fn put_i16(buf: &mut [u8], offset: usize, value: i16) {
    buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(buf: &mut [u8], offset: usize, value: i32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_i16(buf: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([buf[offset], buf[offset + 1]])
}

fn get_i32(buf: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

/// Writes a NUL-terminated string, truncating it to leave room for the NUL.
fn put_str(buf: &mut [u8], offset: usize, length: usize, value: &str) {
    let end = offset + length;
    buf[offset..end].fill(0);
    let bytes = value.as_bytes();
    let count = bytes.len().min(length.saturating_sub(1));
    buf[offset..offset + count].copy_from_slice(&bytes[..count]);
}

/// Reads a NUL-terminated string, replacing invalid UTF-8.
fn get_str(buf: &[u8], offset: usize, length: usize) -> String {
    let field = &buf[offset..offset + length];
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(length);
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// A `utmp` file, or any file holding fixed-size records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UtmpFile {
    path: PathBuf,
}

impl Default for UtmpFile {
    fn default() -> Self {
        Self::new(DEFAULT_PATH)
    }
}

impl UtmpFile {
    /// Uses an explicit path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The path of the file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads every record. A missing file reads as empty rather than failing,
    /// which is what portable tools do and what a freshly booted system needs.
    pub fn read_all(&self) -> io::Result<Vec<UtmpRecord>> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };

        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;

        let mut records = Vec::with_capacity(contents.len() / RECORD_SIZE);
        for chunk in contents.chunks_exact(RECORD_SIZE) {
            match UtmpRecord::decode(chunk) {
                Ok(record) => records.push(record),
                // A torn write or an unknown record must not hide the rest.
                Err(_) => continue,
            }
        }
        Ok(records)
    }

    /// The logged-in users, in the order they appear.
    pub fn read_users(&self) -> io::Result<Vec<UtmpRecord>> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(UtmpRecord::is_user)
            .collect())
    }

    /// Replaces the whole file.
    pub fn write_all(&self, records: &[UtmpRecord]) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut contents = Vec::with_capacity(records.len() * RECORD_SIZE);
        for record in records {
            contents.extend_from_slice(&record.encode());
        }
        fs::write(&self.path, contents)
    }

    /// Inserts a record, replacing the one for the same terminal.
    ///
    /// This mirrors `pututline`: an `EMPTY`/`DEAD_PROCESS` record clears the
    /// terminal's entry so a finished session disappears, while `USER_PROCESS`
    /// and `LOGIN_PROCESS` replace it in place. Anything for a new terminal is
    /// appended.
    pub fn put(&self, record: &UtmpRecord) -> io::Result<()> {
        let mut records = self.read_all()?;
        let existing = records
            .iter()
            .position(|candidate| candidate.same_terminal(record));

        let clearing = matches!(
            record.record_type,
            RecordType::Empty | RecordType::DeadProcess
        );

        match (existing, clearing) {
            (Some(index), true) => {
                records.remove(index);
            }
            (Some(index), false) => {
                records[index] = record.clone();
            }
            (None, true) => {}
            (None, false) => records.push(record.clone()),
        }

        self.write_all(&records)
    }

    /// Removes the entry of a terminal, returning whether anything was removed.
    pub fn remove(&self, line: &str) -> io::Result<bool> {
        let id = terminal_id(line);
        let mut records = self.read_all()?;
        let before = records.len();
        records.retain(|record| record.id != id || record.line != line);
        let removed = records.len() != before;
        if removed {
            self.write_all(&records)?;
        }
        Ok(removed)
    }

    /// Appends a record without touching the others, the way `wtmp` grows.
    pub fn append(&self, record: &UtmpRecord) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        // A short file would desynchronise every later record, so pad it.
        let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        let remainder = (length % RECORD_SIZE as u64) as usize;
        if remainder != 0 {
            let padding = RECORD_SIZE - remainder;
            file.write_all(&vec![0u8; padding])?;
        }

        file.write_all(&record.encode())?;
        file.flush()
    }

    /// Truncates the file, leaving it empty but present.
    pub fn reset(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let file = File::create(&self.path)?;
        file.set_len(0)?;
        // Keep the position consistent for callers that reopen the file.
        let mut file = file;
        file.seek(SeekFrom::Start(0))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("logind-utmp-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn record_size_matches_the_standard_layout() {
        assert_eq!(RECORD_SIZE, 384);
        assert_eq!(OFF_ID, 40);
        assert_eq!(OFF_USER, 44);
        assert_eq!(OFF_HOST, 76);
        assert_eq!(OFF_EXIT, 332);
        assert_eq!(OFF_SESSION, 336);
        assert_eq!(OFF_TIME, 340);
        assert_eq!(OFF_ADDR, 348);
        assert_eq!(OFF_ADDR + 16 + 20, RECORD_SIZE);
    }

    #[test]
    fn fields_are_encoded_at_their_standard_offsets() {
        let mut record = UtmpRecord::user_process(4321, "tty2", "root", 1_792_022_400);
        record.host = "example.org".to_owned();
        record.session = 17;
        record.usec = 250_000;

        let encoded = record.encode();
        assert_eq!(get_i16(&encoded, OFF_TYPE), 7);
        assert_eq!(get_i32(&encoded, OFF_PID), 4321);
        assert_eq!(&encoded[OFF_LINE..OFF_LINE + 6], b"tty2\0\0");
        assert_eq!(&encoded[OFF_ID..OFF_ID + 4], b"2\0\0\0");
        assert_eq!(&encoded[OFF_USER..OFF_USER + 4], b"root");
        assert_eq!(&encoded[OFF_HOST..OFF_HOST + 12], b"example.org\0");
        assert_eq!(get_i32(&encoded, OFF_SESSION), 17);
        assert_eq!(get_i32(&encoded, OFF_TIME), 1_792_022_400);
        assert_eq!(get_i32(&encoded, OFF_TIME + 4), 250_000);

        assert_eq!(UtmpRecord::decode(&encoded).unwrap(), record);
    }

    #[test]
    fn round_trip_through_bytes() {
        let records = [
            UtmpRecord::boot_time(1, 1_792_022_400),
            UtmpRecord::user_process(42, "tty1", "user", 1_792_022_500),
            UtmpRecord::user_process(77, "pty/3", "root", 1_792_022_600),
            UtmpRecord::dead_process("tty1", 1_792_022_700),
        ];

        for record in &records {
            let encoded = record.encode();
            assert_eq!(encoded.len(), RECORD_SIZE);
            assert_eq!(&UtmpRecord::decode(&encoded).unwrap(), record);
        }
    }

    #[test]
    fn long_fields_are_truncated_and_terminated() {
        let mut record = UtmpRecord::user_process(1, "tty1", "u".repeat(100), 0);
        record.host = "h".repeat(400);

        let decoded = UtmpRecord::decode(&record.encode()).unwrap();
        assert_eq!(decoded.user.len(), NAME_SIZE - 1);
        assert_eq!(decoded.host.len(), HOST_SIZE - 1);
        assert_eq!(decoded.user, "u".repeat(NAME_SIZE - 1));
        assert_eq!(decoded.host, "h".repeat(HOST_SIZE - 1));
    }

    #[test]
    fn short_and_unknown_records_are_rejected() {
        assert!(UtmpRecord::decode(&[0u8; 10]).is_err());

        let mut bytes = [0u8; RECORD_SIZE];
        put_i16(&mut bytes, OFF_TYPE, 99);
        assert!(UtmpRecord::decode(&bytes).is_err());
    }

    #[test]
    fn terminal_ids_follow_the_standard_shape() {
        assert_eq!(terminal_id("tty1"), "1");
        assert_eq!(terminal_id("tty12"), "12");
        assert_eq!(terminal_id("pty/7"), "7");
        assert_eq!(terminal_id("/scheme/pty/3"), "3");
        // No digits: keep the name short but usable.
        assert_eq!(terminal_id("console"), "cons");
        assert_eq!(terminal_id("ttyS0"), "0");
    }

    #[test]
    fn a_missing_file_reads_as_empty() {
        let dir = scratch("missing");
        let file = UtmpFile::new(dir.join("utmp"));
        assert!(file.read_all().unwrap().is_empty());
        assert!(file.read_users().unwrap().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn put_replaces_clears_and_appends() {
        let dir = scratch("put");
        let file = UtmpFile::new(dir.join("utmp"));

        // A first login is appended, creating the directory if needed.
        let nested = UtmpFile::new(dir.join("var/run/utmp"));
        nested
            .put(&UtmpRecord::user_process(10, "tty1", "user", 100))
            .unwrap();
        assert_eq!(nested.read_users().unwrap().len(), 1);

        // A second terminal is added.
        file.put(&UtmpRecord::user_process(10, "tty1", "user", 100))
            .unwrap();
        file.put(&UtmpRecord::user_process(20, "tty2", "root", 200))
            .unwrap();
        assert_eq!(file.read_users().unwrap().len(), 2);

        // The same terminal is replaced, not duplicated.
        file.put(&UtmpRecord::user_process(21, "tty2", "user2", 300))
            .unwrap();
        let users = file.read_users().unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!(users[1].user, "user2");
        assert_eq!(users[1].pid, 21);

        // A dead process removes the entry.
        file.put(&UtmpRecord::dead_process("tty2", 400)).unwrap();
        let users = file.read_users().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].line, "tty1");

        // Removing an absent terminal is not an error.
        assert!(!file.remove("tty9").unwrap());
        assert!(file.remove("tty1").unwrap());
        assert!(file.read_all().unwrap().is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn boot_records_do_not_collide_with_terminals() {
        let dir = scratch("boot");
        let file = UtmpFile::new(dir.join("utmp"));

        file.put(&UtmpRecord::boot_time(1, 100)).unwrap();
        file.put(&UtmpRecord::user_process(10, "tty1", "user", 200))
            .unwrap();

        let records = file.read_all().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].record_type, RecordType::BootTime);
        assert_eq!(records[1].record_type, RecordType::UserProcess);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn append_keeps_records_aligned() {
        let dir = scratch("append");
        let path = dir.join("wtmp");
        let file = UtmpFile::new(&path);

        file.append(&UtmpRecord::user_process(1, "tty1", "user", 10))
            .unwrap();
        // A torn tail must be padded, not concatenated.
        {
            let mut raw = OpenOptions::new().append(true).open(&path).unwrap();
            raw.write_all(b"junk").unwrap();
        }
        file.append(&UtmpRecord::user_process(2, "tty2", "root", 20))
            .unwrap();

        // The torn tail is padded so later records stay aligned, which is what
        // makes them readable at all.
        assert_eq!(fs::metadata(&path).unwrap().len() % RECORD_SIZE as u64, 0);
        let records = file.read_all().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].line, "tty1");
        assert_eq!(records[1].line, "tty2");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reset_truncates_and_recreates() {
        let dir = scratch("reset");
        let file = UtmpFile::new(dir.join("var/run/utmp"));
        file.put(&UtmpRecord::user_process(1, "tty1", "user", 10))
            .unwrap();
        assert_eq!(file.read_all().unwrap().len(), 1);

        file.reset().unwrap();
        assert!(file.read_all().unwrap().is_empty());
        assert!(dir.join("var/run/utmp").exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_torn_record_does_not_hide_whole_records() {
        let dir = scratch("torn");
        let path = dir.join("utmp");
        let file = UtmpFile::new(&path);
        file.put(&UtmpRecord::user_process(1, "tty1", "user", 10))
            .unwrap();
        file.append(&UtmpRecord::user_process(2, "tty2", "root", 20))
            .unwrap();

        // Lose the last byte of the last record.
        {
            let raw = OpenOptions::new().write(true).open(&path).unwrap();
            let length = raw.metadata().unwrap().len();
            raw.set_len(length - 1).unwrap();
        }

        let records = file.read_all().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].line, "tty1");

        fs::remove_dir_all(&dir).unwrap();
    }
}
