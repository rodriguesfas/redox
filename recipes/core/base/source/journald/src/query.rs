//! Filtering: the query language behind `journalctl -u`, `-p`, `--since`,
//! `-b` and `FIELD=value` arguments.

use std::str::FromStr;

use crate::entry::{JournalEntry, MESSAGE, PRIORITY, SYSLOG_IDENTIFIER, SYSTEMD_UNIT};
use crate::priority::Priority;

/// How a [`FieldMatch`] compares a field against its value.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MatchOp {
    /// `FIELD=value`
    Equals,
    /// `FIELD!=value`
    NotEquals,
    /// `FIELD~value`, a case-insensitive substring test
    Substring,
    /// `FIELD!~value`
    NotSubstring,
}

/// A single `FIELD=value` style predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldMatch {
    pub field: String,
    pub op: MatchOp,
    pub value: String,
}

impl FieldMatch {
    /// Builds an equality match.
    pub fn equals(field: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            op: MatchOp::Equals,
            value: value.into(),
        }
    }

    /// True if the entry satisfies this predicate.
    ///
    /// A predicate about a field the entry does not have never matches, even
    /// for the negated operators: `PID!=1` only selects entries that actually
    /// record a pid.
    pub fn matches(&self, entry: &JournalEntry) -> bool {
        let Some(actual) = entry.get(&self.field) else {
            return false;
        };
        match self.op {
            MatchOp::Equals => actual == self.value,
            MatchOp::NotEquals => actual != self.value,
            MatchOp::Substring => contains_ignoring_case(actual, &self.value),
            MatchOp::NotSubstring => !contains_ignoring_case(actual, &self.value),
        }
    }
}

impl FromStr for FieldMatch {
    type Err = QueryError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Longest operators first so that `!=` is not read as `=` with a `!`.
        for (token, op) in [
            ("!~", MatchOp::NotSubstring),
            ("!=", MatchOp::NotEquals),
            ("=", MatchOp::Equals),
            ("~", MatchOp::Substring),
        ] {
            if let Some((field, value)) = s.split_once(token) {
                if field.is_empty() {
                    return Err(QueryError(format!("missing field name in {s:?}")));
                }
                return Ok(FieldMatch {
                    field: field.to_ascii_uppercase(),
                    op,
                    value: value.to_owned(),
                });
            }
        }
        Err(QueryError(format!(
            "expected FIELD=value, FIELD!=value or FIELD~value, got {s:?}"
        )))
    }
}

/// Case-insensitive substring test, used by the `~` and `!~` operators.
pub fn contains_ignoring_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle.is_ascii() {
        // Avoid allocating for the common case.
        let haystack = haystack.as_bytes();
        let needle = needle.as_bytes();
        return haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle));
    }
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Which boot(s) to read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BootFilter {
    /// Every boot present in the journal.
    #[default]
    Any,
    /// Only the current boot, resolved by the caller.
    Current,
    /// A specific boot identifier.
    Id(String),
}

/// A parsed filter set. All predicates must match (logical AND).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Match `_SYSTEMD_UNIT` or `SYSLOG_IDENTIFIER`.
    pub unit: Option<String>,
    /// Match `SYSLOG_IDENTIFIER` only.
    pub identifier: Option<String>,
    /// Match entries at least this severe (`PRIORITY <= value`).
    pub priority: Option<Priority>,
    /// Inclusive lower bound on `_SOURCE_REALTIME_TIMESTAMP`.
    pub since: Option<i64>,
    /// Inclusive upper bound on `_SOURCE_REALTIME_TIMESTAMP`.
    pub until: Option<i64>,
    /// Boot selection.
    pub boot: BootFilter,
    /// Free-text search in `MESSAGE`.
    pub grep: Option<String>,
    /// Whether [`Query::grep`] respects case.
    pub grep_case_sensitive: bool,
    /// Additional `FIELD=value` predicates.
    pub matches: Vec<FieldMatch>,
    /// Invert the whole result (`journalctl -x`-style "not").
    pub inverted: bool,
}

impl Query {
    /// An empty query, which matches every entry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a predicate.
    pub fn push_match(&mut self, predicate: FieldMatch) -> &mut Self {
        self.matches.push(predicate);
        self
    }

    /// Looks up a well-known field, so that `-u getty`, `PRIORITY=3` and
    /// `--since` agree on the same value.
    pub fn field_value(&self, field: &str) -> Option<String> {
        Some(match field {
            MESSAGE => self.grep.clone()?,
            PRIORITY => self.priority?.as_syslog().to_string(),
            SYSTEMD_UNIT => self.unit.clone()?,
            SYSLOG_IDENTIFIER => self.identifier.clone().or_else(|| self.unit.clone())?,
            _ => return None,
        })
    }

    /// True if the entry passes every predicate. `current_boot` resolves
    /// [`BootFilter::Current`]; when it is `None`, that filter matches nothing.
    pub fn matches_with(&self, entry: &JournalEntry, current_boot: Option<&str>) -> bool {
        let matched = self.matches_inner(entry, current_boot);
        matched != self.inverted
    }

    /// True if the entry passes every predicate, using `matches_with` without a
    /// known current boot.
    pub fn matches(&self, entry: &JournalEntry) -> bool {
        self.matches_with(entry, None)
    }

    fn matches_inner(&self, entry: &JournalEntry, current_boot: Option<&str>) -> bool {
        if let Some(unit) = &self.unit {
            if entry.unit() != Some(unit.as_str()) {
                return false;
            }
        }

        if let Some(identifier) = &self.identifier {
            if entry.get(SYSLOG_IDENTIFIER) != Some(identifier.as_str()) {
                return false;
            }
        }

        if let Some(priority) = self.priority {
            if entry.priority() > priority {
                return false;
            }
        }

        if let Some(since) = self.since {
            match entry.realtime() {
                Some(realtime) if realtime >= since => {}
                _ => return false,
            }
        }

        if let Some(until) = self.until {
            match entry.realtime() {
                Some(realtime) if realtime <= until => {}
                _ => return false,
            }
        }

        match &self.boot {
            BootFilter::Any => {}
            BootFilter::Current => match (current_boot, entry.boot_id()) {
                (Some(current), Some(boot_id)) if current == boot_id => {}
                _ => return false,
            },
            BootFilter::Id(boot_id) => {
                if entry.boot_id() != Some(boot_id.as_str()) {
                    return false;
                }
            }
        }

        if let Some(grep) = &self.grep {
            let Some(message) = entry.message() else {
                return false;
            };
            let hit = if self.grep_case_sensitive {
                message.contains(grep.as_str())
            } else {
                contains_ignoring_case(message, grep)
            };
            if !hit {
                return false;
            }
        }

        self.matches
            .iter()
            .all(|predicate| predicate.matches(entry))
    }
}

/// Returned when a filter expression cannot be parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError(pub String);

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for QueryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{BOOT_ID, REALTIME_TIMESTAMP};

    fn entry(
        unit: &str,
        identifier: &str,
        priority: u8,
        realtime: i64,
        message: &str,
    ) -> JournalEntry {
        let mut entry = JournalEntry::new();
        if !unit.is_empty() {
            entry.set_field(SYSTEMD_UNIT, unit);
        }
        if !identifier.is_empty() {
            entry.set_field(SYSLOG_IDENTIFIER, identifier);
        }
        entry.set_field(PRIORITY, priority.to_string());
        entry.set_field(REALTIME_TIMESTAMP, realtime.to_string());
        entry.set_field(BOOT_ID, "boot-a");
        entry.set_field(MESSAGE, message);
        entry
    }

    #[test]
    fn empty_query_matches_everything() {
        let query = Query::new();
        assert!(query.matches(&entry("u.service", "u", 6, 10, "hi")));
        assert!(query.matches(&JournalEntry::new()));
    }

    #[test]
    fn unit_matches_unit_or_identifier() {
        let query = Query {
            unit: Some("getty".to_owned()),
            ..Query::new()
        };
        assert!(query.matches(&entry("", "getty", 6, 10, "x")));
        assert!(query.matches(&entry("getty", "other", 6, 10, "x")));
        assert!(!query.matches(&entry("other", "other", 6, 10, "x")));
    }

    #[test]
    fn identifier_only_matches_syslog_identifier() {
        let query = Query {
            identifier: Some("getty".to_owned()),
            ..Query::new()
        };
        assert!(query.matches(&entry("", "getty", 6, 10, "x")));
        assert!(!query.matches(&entry("getty", "", 6, 10, "x")));
    }

    #[test]
    fn priority_is_an_upper_bound() {
        let query = Query {
            priority: Some(Priority::Err),
            ..Query::new()
        };
        assert!(query.matches(&entry("", "a", 0, 0, "x")));
        assert!(query.matches(&entry("", "a", 3, 0, "x")));
        assert!(!query.matches(&entry("", "a", 4, 0, "x")));
        assert!(!query.matches(&entry("", "a", 7, 0, "x")));
    }

    #[test]
    fn time_range_is_inclusive() {
        let query = Query {
            since: Some(100),
            until: Some(200),
            ..Query::new()
        };
        assert!(!query.matches(&entry("", "a", 6, 99, "x")));
        assert!(query.matches(&entry("", "a", 6, 100, "x")));
        assert!(query.matches(&entry("", "a", 6, 150, "x")));
        assert!(query.matches(&entry("", "a", 6, 200, "x")));
        assert!(!query.matches(&entry("", "a", 6, 201, "x")));
    }

    #[test]
    fn entries_without_timestamps_fail_time_filters() {
        let query = Query {
            since: Some(0),
            ..Query::new()
        };
        assert!(!query.matches(&JournalEntry::new()));
    }

    #[test]
    fn grep_is_case_insensitive_by_default() {
        let query = Query {
            grep: Some("LINK".to_owned()),
            ..Query::new()
        };
        assert!(query.matches(&entry("", "a", 6, 0, "link is up")));
        assert!(!query.matches(&entry("", "a", 6, 0, "interface down")));
        assert!(!query.matches(&JournalEntry::new()));
    }

    #[test]
    fn grep_can_be_case_sensitive() {
        let query = Query {
            grep: Some("Link".to_owned()),
            grep_case_sensitive: true,
            ..Query::new()
        };
        assert!(query.matches(&entry("", "a", 6, 0, "Link is up")));
        assert!(!query.matches(&entry("", "a", 6, 0, "link is up")));
    }

    #[test]
    fn boot_filter() {
        let any = Query {
            boot: BootFilter::Any,
            ..Query::new()
        };
        assert!(any.matches(&entry("", "a", 6, 0, "x")));

        let current = Query {
            boot: BootFilter::Current,
            ..Query::new()
        };
        assert!(current.matches_with(&entry("", "a", 6, 0, "x"), Some("boot-a")));
        assert!(!current.matches_with(&entry("", "a", 6, 0, "x"), Some("boot-b")));
        assert!(!current.matches_with(&entry("", "a", 6, 0, "x"), None));

        let by_id = Query {
            boot: BootFilter::Id("boot-a".to_owned()),
            ..Query::new()
        };
        assert!(by_id.matches(&entry("", "a", 6, 0, "x")));
    }

    #[test]
    fn predicates_are_combined_with_and() {
        let query = Query {
            unit: Some("smolnetd".to_owned()),
            priority: Some(Priority::Warning),
            since: Some(50),
            grep: Some("down".to_owned()),
            ..Query::new()
        };
        assert!(query.matches(&entry("smolnetd", "", 4, 60, "link down")));
        assert!(!query.matches(&entry("smolnetd", "", 6, 60, "link down")));
        assert!(!query.matches(&entry("smolnetd", "", 4, 10, "link down")));
        assert!(!query.matches(&entry("smolnetd", "", 4, 60, "link up")));
        assert!(!query.matches(&entry("other", "", 4, 60, "link down")));
    }

    #[test]
    fn inverted_query_flips_the_result() {
        let query = Query {
            unit: Some("getty".to_owned()),
            inverted: true,
            ..Query::new()
        };
        assert!(!query.matches(&entry("getty", "", 6, 0, "x")));
        assert!(query.matches(&entry("other", "", 6, 0, "x")));
    }

    #[test]
    fn field_match_parsing() {
        assert_eq!(
            "MESSAGE=hello".parse::<FieldMatch>().unwrap(),
            FieldMatch {
                field: "MESSAGE".to_owned(),
                op: MatchOp::Equals,
                value: "hello".to_owned(),
            }
        );
        assert_eq!(
            "message~ello".parse::<FieldMatch>().unwrap().op,
            MatchOp::Substring
        );
        assert_eq!(
            "PRIORITY!=3".parse::<FieldMatch>().unwrap().op,
            MatchOp::NotEquals
        );
        assert_eq!(
            "MESSAGE!~hello".parse::<FieldMatch>().unwrap().op,
            MatchOp::NotSubstring
        );
        assert!("justtext".parse::<FieldMatch>().is_err());
        assert!("=value".parse::<FieldMatch>().is_err());
    }

    #[test]
    fn field_match_semantics() {
        let entry = entry("", "a", 6, 0, "Link is up");

        assert!(FieldMatch::equals(MESSAGE, "Link is up").matches(&entry));
        assert!(!FieldMatch::equals(MESSAGE, "Link is down").matches(&entry));
        assert!(FieldMatch {
            field: PRIORITY.to_owned(),
            op: MatchOp::NotEquals,
            value: "7".to_owned()
        }
        .matches(&entry));
        assert!(!FieldMatch {
            field: PRIORITY.to_owned(),
            op: MatchOp::NotEquals,
            value: "6".to_owned()
        }
        .matches(&entry));
        assert!(
            "MESSAGE~LINK"
                .parse::<FieldMatch>()
                .unwrap()
                .matches(&entry),
            "substring matching is case-insensitive"
        );
        assert!("MESSAGE!~down"
            .parse::<FieldMatch>()
            .unwrap()
            .matches(&entry));
        // Missing fields never match, even against a "not equal" predicate.
        assert!(!"NOSUCHFIELD!=value"
            .parse::<FieldMatch>()
            .unwrap()
            .matches(&entry));
    }

    #[test]
    fn substring_helper_is_case_insensitive() {
        assert!(contains_ignoring_case("Link is UP", "link"));
        assert!(contains_ignoring_case("link", "LINK"));
        assert!(contains_ignoring_case("link", ""));
        assert!(contains_ignoring_case("Ação", "ção"));
        assert!(!contains_ignoring_case("link", "down"));
        assert!(!contains_ignoring_case("", "x"));
    }

    #[test]
    fn field_values_are_exposed() {
        let query = Query {
            unit: Some("getty".to_owned()),
            priority: Some(Priority::Err),
            grep: Some("oops".to_owned()),
            ..Query::new()
        };
        assert_eq!(query.field_value(SYSTEMD_UNIT).as_deref(), Some("getty"));
        assert_eq!(
            query.field_value(SYSLOG_IDENTIFIER).as_deref(),
            Some("getty")
        );
        assert_eq!(query.field_value(PRIORITY).as_deref(), Some("3"));
        assert_eq!(query.field_value(MESSAGE).as_deref(), Some("oops"));
        assert_eq!(query.field_value("NOPE"), None);
    }
}
