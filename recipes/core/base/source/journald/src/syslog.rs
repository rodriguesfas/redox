//! Best-effort parsing of syslog-style lines into entries.
//!
//! `logd` receives plain text from the kernel log and from the standard output
//! of every service. This module turns those lines into structured entries:
//! RFC 3164/5424 `<PRI>` prefixes become `PRIORITY` and `SYSLOG_FACILITY`,
//! structured-data blocks become fields, and the common
//! `identifier[pid]: message` shape is split into `SYSLOG_IDENTIFIER`, `_PID`
//! and `MESSAGE`. Parsing never fails: unrecognised input is kept verbatim in
//! `MESSAGE`.

use crate::entry::{
    validate_field_name, JournalEntry, BOOT_ID, HOSTNAME, MESSAGE, PID, PRIORITY,
    REALTIME_TIMESTAMP, SYSLOG_FACILITY, SYSLOG_IDENTIFIER,
};
use crate::priority::Priority;

/// Metadata that the logging daemon knows about a line but that is not encoded
/// in the text itself.
#[derive(Clone, Debug, Default)]
pub struct Source {
    /// Identifier of the current boot.
    pub boot_id: Option<String>,
    /// Host name of this machine.
    pub hostname: Option<String>,
    /// Wall-clock seconds since the epoch when the line was received.
    pub now: i64,
}

impl Source {
    /// A source with no metadata, for tests and one-off parsing.
    pub fn bare(now: i64) -> Self {
        Self {
            boot_id: None,
            hostname: None,
            now,
        }
    }
}

/// Splits a syslog priority value into facility and severity.
///
/// `PRI` is `facility * 8 + severity`, as defined by RFC 5424.
pub fn split_priority(pri: u32) -> (u8, Option<Priority>) {
    (
        (pri / 8).min(u8::MAX as u32) as u8,
        Priority::from_syslog((pri % 8) as u8),
    )
}

/// Parses one line, without the trailing newline.
pub fn parse_line(line: &str, source: &Source) -> JournalEntry {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);

    let mut entry = JournalEntry::new();
    entry.set_field(REALTIME_TIMESTAMP, source.now.to_string());
    if let Some(boot_id) = &source.boot_id {
        entry.set_field(BOOT_ID, boot_id.clone());
    }
    if let Some(hostname) = &source.hostname {
        entry.set_field(HOSTNAME, hostname.clone());
    }

    let mut rest = line;

    // `<PRI>` prefix.
    if let Some(after) = rest.strip_prefix('<') {
        if let Some((digits, after_gt)) = after.split_once('>') {
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                if let Ok(pri) = digits.parse::<u32>() {
                    let (facility, severity) = split_priority(pri);
                    entry.set_field(SYSLOG_FACILITY, facility.to_string());
                    if let Some(severity) = severity {
                        entry.set_field(PRIORITY, severity.as_syslog().to_string());
                    }
                    rest = after_gt;
                    // A single space after the priority is a separator.
                    rest = rest.strip_prefix(' ').unwrap_or(rest);
                }
            }
        }
    }

    // Structured data: one or more `[ID ...]` blocks.
    //
    // The tokens following the identifier are `name=value` pairs. A block whose
    // only token is a bare word uses that word as the value of a field named
    // after the identifier, so `[UNIT logd]` sets `UNIT=logd`.
    while let Some(after) = rest.strip_prefix('[') {
        let Some(end) = after.find(']') else { break };
        let block = &after[..end];
        let mut parts = block.split_whitespace();
        let identifier = parts.next().unwrap_or("");
        let params: Vec<&str> = parts.collect();

        if params.is_empty() {
            if validate_field_name(identifier).is_ok() {
                entry.set_field(identifier, "");
            }
        } else if params.len() == 1 && !params[0].contains('=') {
            if validate_field_name(identifier).is_ok() {
                entry.set_field(identifier, params[0].trim_matches('"'));
            }
        } else {
            for param in params {
                let (name, value) = param.split_once('=').unwrap_or((param, ""));
                let name = name.to_ascii_uppercase();
                if validate_field_name(&name).is_ok() {
                    entry.set_field(&name, value.trim_matches('"'));
                }
            }
        }

        rest = after[end + 1..]
            .strip_prefix(' ')
            .unwrap_or(&after[end + 1..]);
    }

    // `identifier[pid]: message` and `identifier: message`.
    if let Some((head, tail)) = split_identifier(rest) {
        entry.set_field(SYSLOG_IDENTIFIER, head.0);
        if let Some(pid) = head.1 {
            entry.set_field(PID, pid);
        }
        entry.set_field(MESSAGE, tail);
    } else {
        entry.set_field(MESSAGE, rest);
    }

    entry
}

/// Splits `getty[42]: text` into `("getty", Some("42"))` and `text`.
fn split_identifier(line: &str) -> Option<((String, Option<String>), String)> {
    let (head, tail) = line.split_once(": ")?;

    // Reject anything that looks like a sentence rather than a tag, and
    // anything with spaces in the tag.
    if head.is_empty() || head.contains(' ') {
        return None;
    }

    if let Some((name, pid)) = head.strip_suffix(']').and_then(|h| h.split_once('[')) {
        if name.is_empty() || pid.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return Some(((name.to_owned(), Some(pid.to_owned())), tail.to_owned()));
    }

    if head
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
    {
        return Some(((head.to_owned(), None), tail.to_owned()));
    }

    None
}

/// Parses every line of a chunk of text.
pub fn parse_lines(text: &str, source: &Source) -> Vec<JournalEntry> {
    text.lines().map(|line| parse_line(line, source)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Source {
        Source {
            boot_id: Some("boot1".to_owned()),
            hostname: Some("aios".to_owned()),
            now: 1_700_000_000,
        }
    }

    #[test]
    fn priority_prefix_is_decoded() {
        // <13> = facility 1 (user) * 8 + severity 5 (notice)
        let entry = parse_line("<13>hello", &source());
        assert_eq!(entry.get(PRIORITY), Some("5"));
        assert_eq!(entry.priority(), Priority::Notice);
        assert_eq!(entry.get(SYSLOG_FACILITY), Some("1"));
        assert_eq!(entry.message(), Some("hello"));

        // <0> = facility 0 (kern), severity 0 (emerg)
        let entry = parse_line("<0>panic", &source());
        assert_eq!(entry.get(PRIORITY), Some("0"));
        assert_eq!(entry.get(SYSLOG_FACILITY), Some("0"));
        assert_eq!(entry.priority(), Priority::Emerg);

        // <191> = facility 23 (local7) * 8 + 7 (debug)
        let entry = parse_line("<191>trace", &source());
        assert_eq!(entry.priority(), Priority::Debug);
        assert_eq!(entry.get(SYSLOG_FACILITY), Some("23"));
    }

    #[test]
    fn malformed_priority_prefix_is_kept_as_text() {
        let entry = parse_line("<not-a-pri>hello", &source());
        assert_eq!(entry.message(), Some("<not-a-pri>hello"));
        assert_eq!(entry.get(PRIORITY), None);
    }

    #[test]
    fn metadata_fields_are_added() {
        let entry = parse_line("plain line", &source());
        assert_eq!(entry.get(BOOT_ID), Some("boot1"));
        assert_eq!(entry.get(HOSTNAME), Some("aios"));
        assert_eq!(entry.realtime(), Some(1_700_000_000));
        assert_eq!(entry.priority(), Priority::Info);
    }

    #[test]
    fn identifier_with_pid_is_split() {
        let entry = parse_line("getty[42]: welcome", &source());
        assert_eq!(entry.get(SYSLOG_IDENTIFIER), Some("getty"));
        assert_eq!(entry.get(PID), Some("42"));
        assert_eq!(entry.message(), Some("welcome"));
        assert_eq!(entry.unit(), Some("getty"));
    }

    #[test]
    fn bare_identifier_is_split() {
        let entry = parse_line("smolnetd: interface up", &source());
        assert_eq!(entry.get(SYSLOG_IDENTIFIER), Some("smolnetd"));
        assert_eq!(entry.get(PID), None);
        assert_eq!(entry.message(), Some("interface up"));
    }

    #[test]
    fn sentences_are_not_mistaken_for_identifiers() {
        let entry = parse_line("init: failed: to do something with spaces", &source());
        assert_eq!(entry.message(), Some("failed: to do something with spaces"));
        assert_eq!(entry.get(SYSLOG_IDENTIFIER), Some("init"));

        let entry = parse_line("this is a sentence: with a colon", &source());
        assert_eq!(entry.get(SYSLOG_IDENTIFIER), None);
        assert_eq!(entry.message(), Some("this is a sentence: with a colon"));
    }

    #[test]
    fn structured_data_becomes_fields() {
        let entry = parse_line(
            r#"[UNIT smolnetd][NET iface="eth0" up="1"]link is up"#,
            &source(),
        );
        assert_eq!(entry.get("UNIT"), Some("smolnetd"));
        assert_eq!(entry.get("IFACE"), Some("eth0"));
        assert_eq!(entry.get("UP"), Some("1"));
        assert_eq!(entry.message(), Some("link is up"));
    }

    #[test]
    fn structured_data_ignores_invalid_names() {
        let entry = parse_line("[lower bad-name=x]text", &source());
        assert_eq!(entry.get("BAD-NAME"), None);
        assert_eq!(entry.get("lower"), None);
        assert_eq!(entry.message(), Some("text"));
    }

    #[test]
    fn structured_data_before_priority() {
        let entry = parse_line("<6>[UNIT logd]ready", &source());
        assert_eq!(entry.priority(), Priority::Info);
        assert_eq!(entry.get("UNIT"), Some("logd"));
        assert_eq!(entry.message(), Some("ready"));
    }

    #[test]
    fn empty_line_still_yields_an_entry() {
        let entry = parse_line("", &source());
        assert_eq!(entry.message(), Some(""));
        assert!(entry.realtime().is_some());
    }

    #[test]
    fn crlf_is_stripped() {
        let entry = parse_line("logd[1]: hello\r\n", &source());
        assert_eq!(entry.message(), Some("hello"));
    }

    #[test]
    fn lines_are_parsed_in_order() {
        let source = source();
        let entries = parse_lines("a[1]: one\nb: two\nplain\n", &source);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].message(), Some("one"));
        assert_eq!(entries[1].message(), Some("two"));
        assert_eq!(entries[2].message(), Some("plain"));
        // Round-trips through the export format.
        for entry in &entries {
            assert_eq!(
                JournalEntry::decode_all(&entry.encode()).unwrap(),
                vec![entry.clone()]
            );
        }
    }
}
