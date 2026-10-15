//! Rendering of entries for the different `-o` output modes.

use crate::cli::OutputMode;
use crate::entry::{JournalEntry, PID, SYSLOG_IDENTIFIER, SYSTEMD_UNIT};
use crate::time::format_short;

/// Renders a batch of entries as one string.
pub fn render(entries: &[&JournalEntry], mode: OutputMode, hostname: &str) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&render_entry(entry, mode, hostname));
    }
    out
}

/// Renders a single entry in the given mode.
pub fn render_entry(entry: &JournalEntry, mode: OutputMode, hostname: &str) -> String {
    match mode {
        OutputMode::Short => format!("{}\n", render_short(entry, hostname)),
        OutputMode::Cat => format!("{}\n", render_cat(entry)),
        OutputMode::Export => entry.encode(),
        OutputMode::Json => format!("{}\n", entry.to_json_string(false)),
        OutputMode::JsonPretty => format!("{}\n", entry.to_json_string(true)),
    }
}

/// `Jul 15 12:00:00 host unit[pid]: message`
pub fn render_short(entry: &JournalEntry, hostname: &str) -> String {
    let timestamp = entry
        .realtime()
        .map(format_short)
        .unwrap_or_else(|| "-".to_owned());
    let host = entry
        .get(crate::entry::HOSTNAME)
        .unwrap_or(hostname)
        .to_owned();
    let unit = unit_name(entry);
    let pid = entry
        .get(PID)
        .map(|pid| format!("[{pid}]"))
        .unwrap_or_default();
    let message = entry.message().unwrap_or("<no message>");
    format!("{timestamp} {host} {unit}{pid}: {message}")
}

/// The message text, or a placeholder when the entry has none.
pub fn render_cat(entry: &JournalEntry) -> String {
    entry.message().unwrap_or("<no message>").to_owned()
}

/// The short unit label: the syslog identifier, or the unit file name with its
/// `NN_` ordering prefix removed.
pub fn unit_name(entry: &JournalEntry) -> String {
    if let Some(identifier) = entry.get(SYSLOG_IDENTIFIER) {
        return identifier.to_owned();
    }
    match entry.get(SYSTEMD_UNIT) {
        Some(unit) => {
            let unit = unit.strip_suffix(".service").unwrap_or(unit);
            match unit.split_once('_') {
                Some((prefix, rest))
                    if !prefix.is_empty()
                        && prefix.len() <= 2
                        && prefix.chars().all(|c| c.is_ascii_digit()) =>
                {
                    rest.to_owned()
                }
                _ => unit.to_owned(),
            }
        }
        None => "unknown".to_owned(),
    }
}

/// Renders `journalctl --list-boots` output from a boot summary.
pub fn render_boot_list(boots: &[(String, usize)]) -> String {
    if boots.is_empty() {
        return "No boots found.\n".to_owned();
    }
    let mut out = format!("{:>3}  {:32}  {}\n", "IDX", "BOOT ID", "ENTRIES");
    for (index, (id, count)) in boots.iter().enumerate() {
        out.push_str(&format!("{index:>3}  {id:32}  {count}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{BOOT_ID, MESSAGE, PRIORITY, REALTIME_TIMESTAMP};
    use crate::priority::Priority;

    fn entry() -> JournalEntry {
        let mut entry = JournalEntry::new();
        entry.set_field(REALTIME_TIMESTAMP, "1792022400");
        entry.set_field(BOOT_ID, "boot-a");
        entry.set_field(PRIORITY, "4");
        entry.set_field(SYSLOG_IDENTIFIER, "smolnetd");
        entry.set_field(PID, "12");
        entry.set_field(MESSAGE, "link down");
        entry
    }

    #[test]
    fn short_output_has_time_host_unit_pid_and_message() {
        let line = render_short(&entry(), "aios");
        assert_eq!(line, "Oct 15 00:00:00 aios smolnetd[12]: link down");
    }

    #[test]
    fn short_output_uses_the_entry_hostname_when_present() {
        let mut entry = entry();
        entry.set_field(crate::entry::HOSTNAME, "box");
        assert!(render_short(&entry, "fallback").contains(" box "));
    }

    #[test]
    fn short_output_falls_back_for_missing_fields() {
        let line = render_short(&JournalEntry::new(), "aios");
        assert_eq!(line, "- aios unknown: <no message>");
    }

    #[test]
    fn short_output_shortens_unit_file_names() {
        let mut entry = JournalEntry::new();
        entry.set_field(SYSTEMD_UNIT, "10_smolnetd.service");
        entry.set_field(MESSAGE, "up");
        assert_eq!(unit_name(&entry), "smolnetd");

        let mut entry = JournalEntry::new();
        entry.set_field(SYSTEMD_UNIT, "multi-user.target");
        assert_eq!(unit_name(&entry), "multi-user.target");

        // No spurious truncation when the name before `_` is not an ordering
        // prefix.
        let mut entry = JournalEntry::new();
        entry.set_field(SYSTEMD_UNIT, "my_thing.service");
        assert_eq!(unit_name(&entry), "my_thing");
    }

    #[test]
    fn cat_output_is_the_message() {
        assert_eq!(render_cat(&entry()), "link down");
        assert_eq!(render_cat(&JournalEntry::new()), "<no message>");
    }

    #[test]
    fn export_output_round_trips() {
        let entry = entry();
        let rendered = render_entry(&entry, OutputMode::Export, "aios");
        assert_eq!(JournalEntry::decode_all(&rendered).unwrap(), vec![entry]);
    }

    #[test]
    fn json_outputs_parse() {
        let entry = entry();
        let rendered = render_entry(&entry, OutputMode::Json, "aios");
        let value: serde_json::Value = serde_json::from_str(rendered.trim()).unwrap();
        assert_eq!(value["MESSAGE"], "link down");
        assert_eq!(value["PRIORITY"], "4");
        assert_eq!(entry.priority(), Priority::Warning);

        let pretty = render_entry(&entry, OutputMode::JsonPretty, "aios");
        assert!(pretty.contains("\n  \""));
        serde_json::from_str::<serde_json::Value>(pretty.trim()).unwrap();
    }

    #[test]
    fn batch_rendering_concatenates_entries() {
        let first = entry();
        let mut second = JournalEntry::new();
        second.set_field(MESSAGE, "second");
        let rendered = render(&[&first, &second], OutputMode::Cat, "aios");
        assert_eq!(rendered, "link down\nsecond\n");
    }

    #[test]
    fn boot_list_rendering() {
        assert_eq!(render_boot_list(&[]), "No boots found.\n");
        let rendered = render_boot_list(&[("boot-a".to_owned(), 2), ("boot-b".to_owned(), 5)]);
        assert!(rendered.starts_with("IDX"));
        assert!(rendered.contains("boot-a"));
        assert!(rendered.contains('5'));
    }
}
