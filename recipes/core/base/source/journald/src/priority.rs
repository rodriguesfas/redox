//! Syslog priorities (RFC 5424 severities) and their textual names.

use std::fmt;
use std::str::FromStr;

/// Syslog severity, from most to least severe.
///
/// The numeric values match the syslog severity codes, which also makes the
/// derived ordering useful for filtering: `-p err` accepts everything with a
/// value less than or equal to [`Priority::Err`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Priority {
    Emerg = 0,
    Alert = 1,
    Crit = 2,
    Err = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

impl Priority {
    /// Every priority, ordered from most to least severe.
    pub const ALL: [Priority; 8] = [
        Priority::Emerg,
        Priority::Alert,
        Priority::Crit,
        Priority::Err,
        Priority::Warning,
        Priority::Notice,
        Priority::Info,
        Priority::Debug,
    ];

    /// Converts a syslog severity number into a priority.
    pub const fn from_syslog(value: u8) -> Option<Priority> {
        Some(match value {
            0 => Priority::Emerg,
            1 => Priority::Alert,
            2 => Priority::Crit,
            3 => Priority::Err,
            4 => Priority::Warning,
            5 => Priority::Notice,
            6 => Priority::Info,
            7 => Priority::Debug,
            _ => return None,
        })
    }

    /// The syslog severity number for this priority.
    pub const fn as_syslog(self) -> u8 {
        self as u8
    }

    /// Upper-case name used in `journalctl` output and `PRIORITY=` filters.
    pub const fn name(self) -> &'static str {
        match self {
            Priority::Emerg => "EMERG",
            Priority::Alert => "ALERT",
            Priority::Crit => "CRIT",
            Priority::Err => "ERR",
            Priority::Warning => "WARNING",
            Priority::Notice => "NOTICE",
            Priority::Info => "INFO",
            Priority::Debug => "DEBUG",
        }
    }
}

/// Parses the names accepted by `journalctl -p`.
///
/// Aliases follow the ones documented for systemd: `err`/`error`,
/// `warning`/`warn`, `emerg`/`panic` and `crit`/`critical`.
pub fn parse_priority(name: &str) -> Option<Priority> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "emerg" | "emergency" | "panic" => Priority::Emerg,
        "alert" => Priority::Alert,
        "crit" | "critical" => Priority::Crit,
        "err" | "error" => Priority::Err,
        "warning" | "warn" => Priority::Warning,
        "notice" => Priority::Notice,
        "info" | "informational" | "information" => Priority::Info,
        "debug" => Priority::Debug,
        // Numeric severities are accepted too, matching the syslog encoding.
        other => Priority::from_syslog(other.parse().ok()?)?,
    })
}

impl FromStr for Priority {
    type Err = ParsePriorityError;

    fn from_str(s: &str) -> Result<Self, ParsePriorityError> {
        parse_priority(s).ok_or(ParsePriorityError)
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Returned when a priority name is not recognised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsePriorityError;

impl fmt::Display for ParsePriorityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unknown priority")
    }
}

impl std::error::Error for ParsePriorityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syslog_roundtrip() {
        for priority in Priority::ALL {
            assert_eq!(
                Priority::from_syslog(priority.as_syslog()),
                Some(priority),
                "{priority}"
            );
        }
        assert_eq!(Priority::from_syslog(8), None);
        assert_eq!(Priority::from_syslog(255), None);
    }

    #[test]
    fn numeric_values_match_syslog() {
        assert_eq!(Priority::Emerg.as_syslog(), 0);
        assert_eq!(Priority::Err.as_syslog(), 3);
        assert_eq!(Priority::Warning.as_syslog(), 4);
        assert_eq!(Priority::Info.as_syslog(), 6);
        assert_eq!(Priority::Debug.as_syslog(), 7);
    }

    #[test]
    fn ordering_is_by_severity() {
        // Derived `Ord` must be syslog order, so `-p err` means "value <= 3".
        assert!(Priority::Emerg < Priority::Alert);
        assert!(Priority::Err < Priority::Warning);
        assert!(Priority::Info < Priority::Debug);
        assert!(Priority::Debug > Priority::Err);
    }

    #[test]
    fn name_aliases_parse() {
        assert_eq!(parse_priority("err"), Some(Priority::Err));
        assert_eq!(parse_priority("ERROR"), Some(Priority::Err));
        assert_eq!(parse_priority(" warn "), Some(Priority::Warning));
        assert_eq!(parse_priority("panic"), Some(Priority::Emerg));
        assert_eq!(parse_priority("critical"), Some(Priority::Crit));
        assert_eq!(parse_priority("3"), Some(Priority::Err));
        assert_eq!(parse_priority("0"), Some(Priority::Emerg));
        assert_eq!(parse_priority("9"), None);
        assert_eq!(parse_priority("watch"), None);
    }

    #[test]
    fn display_and_from_str() {
        assert_eq!(Priority::Warning.to_string(), "WARNING");
        assert_eq!("debug".parse::<Priority>(), Ok(Priority::Debug));
        assert!("nope".parse::<Priority>().is_err());
    }
}
