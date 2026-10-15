//! Journal entries: the unit of storage, plus the text and JSON encodings.
//!
//! An entry is a set of `FIELD=value` pairs, using the same naming rules as
//! systemd's journal: field names are upper-case ASCII letters, digits and
//! underscores, and names starting with a single underscore carry trusted
//! metadata added by the logging daemon itself.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde_json::{Map, Value};

use crate::priority::Priority;

/// Log message text.
pub const MESSAGE: &str = "MESSAGE";
/// Syslog severity, `0` (emerg) through `7` (debug).
pub const PRIORITY: &str = "PRIORITY";
/// Syslog facility number.
pub const SYSLOG_FACILITY: &str = "SYSLOG_FACILITY";
/// Program name that emitted the message.
pub const SYSLOG_IDENTIFIER: &str = "SYSLOG_IDENTIFIER";
/// Unit that produced the message, as set by `init`.
pub const SYSTEMD_UNIT: &str = "_SYSTEMD_UNIT";
/// Process id of the sender.
pub const PID: &str = "_PID";
/// Boot identifier, used by `journalctl -b`.
pub const BOOT_ID: &str = "_BOOT_ID";
/// Wall-clock timestamp in seconds since the Unix epoch.
pub const REALTIME_TIMESTAMP: &str = "_SOURCE_REALTIME_TIMESTAMP";
/// Host name.
pub const HOSTNAME: &str = "_HOSTNAME";

/// Reasons a field name, escape sequence or entry could not be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryError {
    /// The field name is empty, starts with a digit, or has invalid characters.
    InvalidFieldName(String),
    /// A field name uses the reserved `__` prefix.
    ReservedFieldName(String),
    /// A line in the export format is missing the `=` separator.
    MalformedLine(String),
    /// An escape sequence in a line is not one of `\\`, `\n` or `\r`.
    InvalidEscape(String),
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryError::InvalidFieldName(name) => write!(f, "invalid field name {name:?}"),
            EntryError::ReservedFieldName(name) => {
                write!(f, "field name {name:?} uses the reserved '__' prefix")
            }
            EntryError::MalformedLine(line) => write!(f, "malformed journal line {line:?}"),
            EntryError::InvalidEscape(escape) => write!(f, "invalid escape {escape:?}"),
        }
    }
}

impl std::error::Error for EntryError {}

/// Validates a journal field name.
pub fn validate_field_name(name: &str) -> Result<(), EntryError> {
    let mut chars = name.chars();
    match chars.next() {
        None => return Err(EntryError::InvalidFieldName(name.to_owned())),
        Some(first) if first == '_' || first.is_ascii_uppercase() => {}
        Some(_) => return Err(EntryError::InvalidFieldName(name.to_owned())),
    }

    if name.starts_with("__") {
        return Err(EntryError::ReservedFieldName(name.to_owned()));
    }

    if !name
        .chars()
        .all(|c| c == '_' || c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Err(EntryError::InvalidFieldName(name.to_owned()));
    }

    Ok(())
}

/// A single structured log record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JournalEntry {
    fields: BTreeMap<String, String>,
}

impl JournalEntry {
    /// Creates an empty entry.
    pub fn new() -> Self {
        Self::default()
    }

    /// True if the entry has no fields at all.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Number of fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Sets a field, rejecting invalid names.
    pub fn set(
        &mut self,
        name: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<&mut Self, EntryError> {
        let name = name.as_ref();
        validate_field_name(name)?;
        self.fields.insert(name.to_owned(), value.into());
        Ok(self)
    }

    /// Sets a field, panicking if the name is invalid. Intended for tests and
    /// for the daemon writing its own trusted fields.
    #[track_caller]
    pub fn set_field(&mut self, name: &str, value: impl Into<String>) -> &mut Self {
        validate_field_name(name).expect("invalid journal field name");
        self.fields.insert(name.to_owned(), value.into());
        self
    }

    /// Reads a field.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// Removes a field, returning its previous value.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        self.fields.remove(name)
    }

    /// Iterates over fields in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The message text.
    pub fn message(&self) -> Option<&str> {
        self.get(MESSAGE)
    }

    /// The recorded severity, defaulting to [`Priority::Info`] when absent or
    /// unparseable (matching systemd's behaviour for untagged messages).
    pub fn priority(&self) -> Priority {
        self.get(PRIORITY)
            .and_then(|value| {
                Priority::from_str(value)
                    .ok()
                    .or_else(|| Priority::from_syslog(value.trim().parse().ok()?))
            })
            .unwrap_or(Priority::Info)
    }

    /// The wall-clock timestamp in seconds, if recorded.
    pub fn realtime(&self) -> Option<i64> {
        self.get(REALTIME_TIMESTAMP)
            .and_then(|value| value.trim().parse().ok())
    }

    /// The unit or identifier this entry came from.
    pub fn unit(&self) -> Option<&str> {
        self.get(SYSTEMD_UNIT)
            .or_else(|| self.get(SYSLOG_IDENTIFIER))
    }

    /// The boot identifier, if recorded.
    pub fn boot_id(&self) -> Option<&str> {
        self.get(BOOT_ID)
    }

    /// Encodes the entry in the `journalctl -o export` text format.
    ///
    /// Fields are emitted one per line, sorted by name, with `\`, newline and
    /// carriage return escaped. A record ends with an empty line.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        for (name, value) in &self.fields {
            out.push_str(name);
            out.push('=');
            for c in value.chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    c => out.push(c),
                }
            }
            out.push('\n');
        }
        out.push('\n');
        out
    }

    /// Decodes every record in an export-format document.
    pub fn decode_all(input: &str) -> Result<Vec<JournalEntry>, EntryError> {
        let mut entries = Vec::new();
        let mut current = JournalEntry::new();

        for line in input.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                if !current.is_empty() {
                    entries.push(std::mem::take(&mut current));
                }
                continue;
            }

            let Some((name, value)) = line.split_once('=') else {
                return Err(EntryError::MalformedLine(line.to_owned()));
            };
            validate_field_name(name)?;
            current.fields.insert(name.to_owned(), unescape(value)?);
        }

        if !current.is_empty() {
            entries.push(current);
        }

        Ok(entries)
    }

    /// Renders the entry as a JSON object of field/value pairs.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        for (name, value) in &self.fields {
            map.insert(name.to_owned(), Value::String(value.clone()));
        }
        Value::Object(map)
    }

    /// Renders the entry as a JSON string.
    pub fn to_json_string(&self, pretty: bool) -> String {
        let value = self.to_json_value();
        if pretty {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned())
        } else {
            serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned())
        }
    }

    /// The size of [`JournalEntry::encode`]'s output, used for journal rotation.
    pub fn encoded_len(&self) -> usize {
        self.fields
            .iter()
            .map(|(name, value)| name.len() + 1 + escaped_len(value) + 1)
            .sum::<usize>()
            + 1
    }
}

fn escaped_len(value: &str) -> usize {
    value
        .chars()
        .map(|c| match c {
            '\\' | '\n' | '\r' => 2,
            c => c.len_utf8(),
        })
        .sum()
}

fn unescape(value: &str) -> Result<String, EntryError> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            other => {
                let mut escape = String::from("\\");
                if let Some(other) = other {
                    escape.push(other);
                }
                return Err(EntryError::InvalidEscape(escape));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> JournalEntry {
        let mut entry = JournalEntry::new();
        entry.set_field(MESSAGE, "hello world");
        entry.set_field(PRIORITY, "6");
        entry.set_field(SYSLOG_IDENTIFIER, "logd");
        entry.set_field(REALTIME_TIMESTAMP, "1700000000");
        entry.set_field(SYSTEMD_UNIT, "10_smolnetd.service");
        entry
    }

    #[test]
    fn field_name_validation() {
        assert!(validate_field_name("MESSAGE").is_ok());
        assert!(validate_field_name("_PID").is_ok());
        assert!(validate_field_name("A_1").is_ok());
        assert_eq!(
            validate_field_name("message"),
            Err(EntryError::InvalidFieldName("message".to_owned()))
        );
        assert_eq!(
            validate_field_name("1BAD"),
            Err(EntryError::InvalidFieldName("1BAD".to_owned()))
        );
        assert_eq!(
            validate_field_name(""),
            Err(EntryError::InvalidFieldName(String::new()))
        );
        assert_eq!(
            validate_field_name("BAD-NAME"),
            Err(EntryError::InvalidFieldName("BAD-NAME".to_owned()))
        );
        assert_eq!(
            validate_field_name("__RESERVED"),
            Err(EntryError::ReservedFieldName("__RESERVED".to_owned()))
        );
    }

    #[test]
    fn set_rejects_bad_names() {
        let mut entry = JournalEntry::new();
        assert!(entry.set("lower", "x").is_err());
        assert!(entry.is_empty());
        assert!(entry.set(MESSAGE, "x").is_ok());
        assert_eq!(entry.len(), 1);
    }

    #[test]
    fn encode_is_sorted_and_record_terminated() {
        let mut entry = JournalEntry::new();
        entry.set_field("ZZZ", "last");
        entry.set_field("AAA", "first");
        assert_eq!(entry.encode(), "AAA=first\nZZZ=last\n\n");
    }

    #[test]
    fn encode_escapes_specials() {
        let mut entry = JournalEntry::new();
        entry.set_field(MESSAGE, "a\nb\\c\rd");
        let encoded = entry.encode();
        assert_eq!(encoded, "MESSAGE=a\\nb\\\\c\\rd\n\n");
        assert_eq!(entry.encoded_len(), encoded.len());
    }

    #[test]
    fn export_roundtrip() {
        let entry = sample();
        let encoded = entry.encode();
        let decoded = JournalEntry::decode_all(&encoded).unwrap();
        assert_eq!(decoded, vec![entry]);
    }

    #[test]
    fn roundtrip_with_specials_and_unicode() {
        let mut entry = JournalEntry::new();
        entry.set_field(MESSAGE, "linha 1\nlinha 2\ttab ça");
        entry.set_field("EMPTY_VALUE", "");

        let encoded = entry.encode();
        assert_eq!(JournalEntry::decode_all(&encoded).unwrap(), vec![entry]);
    }

    #[test]
    fn decode_multiple_records() {
        let mut first = JournalEntry::new();
        first.set_field(MESSAGE, "one");
        let mut second = JournalEntry::new();
        second.set_field(MESSAGE, "two");

        let document = format!("{}{}", first.encode(), second.encode());
        assert_eq!(
            JournalEntry::decode_all(&document).unwrap(),
            vec![first, second]
        );
    }

    #[test]
    fn decode_handles_missing_trailing_blank_line() {
        let entries = JournalEntry::decode_all("MESSAGE=no-terminator\n").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), Some("no-terminator"));
    }

    #[test]
    fn decode_empty_value_and_empty_input() {
        let entries = JournalEntry::decode_all("MESSAGE=\n\n").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message(), Some(""));
        assert_eq!(JournalEntry::decode_all("").unwrap(), vec![]);
        assert_eq!(JournalEntry::decode_all("\n\n").unwrap(), vec![]);
    }

    #[test]
    fn decode_rejects_malformed_input() {
        assert_eq!(
            JournalEntry::decode_all("not-a-field\n\n").unwrap_err(),
            EntryError::MalformedLine("not-a-field".to_owned())
        );
        assert_eq!(
            JournalEntry::decode_all("MESSAGE=bad\\qescape\n\n").unwrap_err(),
            EntryError::InvalidEscape("\\q".to_owned())
        );
        assert!(JournalEntry::decode_all("__BAD=x\n\n").is_err());
    }

    #[test]
    fn accessors_read_well_known_fields() {
        let entry = sample();
        assert_eq!(entry.message(), Some("hello world"));
        assert_eq!(entry.priority(), Priority::Info);
        assert_eq!(entry.realtime(), Some(1_700_000_000));
        assert_eq!(entry.unit(), Some("10_smolnetd.service"));
        assert_eq!(entry.boot_id(), None);
    }

    #[test]
    fn priority_defaults_to_info_and_accepts_names() {
        assert_eq!(JournalEntry::new().priority(), Priority::Info);

        let mut entry = JournalEntry::new();
        entry.set_field(PRIORITY, "3");
        assert_eq!(entry.priority(), Priority::Err);

        entry.set_field(PRIORITY, "debug");
        assert_eq!(entry.priority(), Priority::Debug);

        entry.set_field(PRIORITY, "garbage");
        assert_eq!(entry.priority(), Priority::Info);
    }

    #[test]
    fn unit_falls_back_to_syslog_identifier() {
        let mut entry = JournalEntry::new();
        entry.set_field(SYSLOG_IDENTIFIER, "getty");
        assert_eq!(entry.unit(), Some("getty"));

        entry.set_field(SYSTEMD_UNIT, "30_getty.service");
        assert_eq!(entry.unit(), Some("30_getty.service"));
    }

    #[test]
    fn json_encoding_carries_fields() {
        let entry = sample();
        let value: Value = serde_json::from_str(&entry.to_json_string(false)).unwrap();
        assert_eq!(value["MESSAGE"], "hello world");
        assert_eq!(value["PRIORITY"], "6");
        assert!(entry.to_json_string(true).contains('\n'));
    }

    #[test]
    fn remove_and_iterate() {
        let mut entry = sample();
        assert_eq!(entry.remove(MESSAGE), Some("hello world".to_owned()));
        assert_eq!(entry.message(), None);
        let names: Vec<&str> = entry.iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            vec![
                "PRIORITY",
                "SYSLOG_IDENTIFIER",
                "_SOURCE_REALTIME_TIMESTAMP",
                "_SYSTEMD_UNIT"
            ]
        );
    }
}
