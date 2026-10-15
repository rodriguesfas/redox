//! Timestamp parsing and formatting for `journalctl --since/--until` and for
//! rendering entry timestamps.
//!
//! All timestamps are seconds since the Unix epoch in UTC. Supported inputs
//! mirror the common `journalctl` forms: absolute dates (`2026-10-15`,
//! `2026-10-15 12:34:56`, `...T12:34:56Z`), relative offsets (`-1h`, `-30m`,
//! `-2d`), the `@<seconds>` epoch form, and the keywords `now`, `today`,
//! `yesterday` and `tomorrow`.

use std::fmt;

/// One day in seconds.
pub const DAY: i64 = 86_400;
/// One hour in seconds.
pub const HOUR: i64 = 3_600;
/// One minute in seconds.
pub const MINUTE: i64 = 60;
/// One week in seconds.
pub const WEEK: i64 = 7 * DAY;

const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Returned when a timestamp string cannot be understood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeError(pub String);

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid timestamp {:?}", self.0)
    }
}

impl std::error::Error for TimeError {}

fn err<T>(input: &str) -> Result<T, TimeError> {
    Err(TimeError(input.to_owned()))
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
///
/// Uses the well-known civil-from-days algorithm; valid for the whole range of
/// dates that can appear in a log.
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * month_index + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`].
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn split_time(rest: &str) -> Result<(i64, i64, i64), TimeError> {
    let rest = rest.trim_start_matches('T').trim_end_matches('Z');
    if rest.is_empty() {
        return Ok((0, 0, 0));
    }

    let mut parts = rest.split(':');
    let hour = parts.next().unwrap_or("0").trim();
    let minute = parts.next().unwrap_or("0").trim();
    let second = parts.next().unwrap_or("0").trim();
    if parts.next().is_some() {
        return err(rest);
    }

    let hour: i64 = hour.parse().map_err(|_| TimeError(rest.to_owned()))?;
    let minute: i64 = minute.parse().map_err(|_| TimeError(rest.to_owned()))?;
    let second: i64 = second.parse().map_err(|_| TimeError(rest.to_owned()))?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..=60).contains(&second) {
        return err(rest);
    }
    Ok((hour, minute, second))
}

/// Parses a timestamp string relative to `now` (seconds since the epoch).
pub fn parse_timestamp(input: &str, now: i64) -> Result<i64, TimeError> {
    let input = input.trim();
    if input.is_empty() {
        return err(input);
    }

    match input.to_ascii_lowercase().as_str() {
        "now" => return Ok(now),
        "today" => return Ok(now.div_euclid(DAY) * DAY),
        "yesterday" => return Ok(now.div_euclid(DAY) * DAY - DAY),
        "tomorrow" => return Ok(now.div_euclid(DAY) * DAY + DAY),
        _ => {}
    }

    if let Some(seconds) = input.strip_prefix('@') {
        return seconds
            .trim()
            .parse()
            .map_err(|_| TimeError(input.to_owned()));
    }

    if let Some(rest) = input.strip_prefix('-').or_else(|| input.strip_prefix('+')) {
        let negative = input.starts_with('-');
        let (digits, unit) = rest.split_at(rest.len().saturating_sub(1));
        let multiplier = match unit {
            "s" => 1,
            "m" => MINUTE,
            "h" => HOUR,
            "d" => DAY,
            "w" => WEEK,
            _ => return err(input),
        };
        let amount: i64 = digits
            .trim()
            .parse()
            .map_err(|_| TimeError(input.to_owned()))?;
        let offset = amount * multiplier;
        return Ok(if negative { now - offset } else { now + offset });
    }

    let (date, time) = match input.split_once([' ', 'T']) {
        Some((date, time)) => (date, Some(time)),
        None => (input, None),
    };

    let mut date_parts = date.split('-');
    let year: i64 = date_parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| TimeError(input.to_owned()))?;
    let month: u32 = date_parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| TimeError(input.to_owned()))?;
    let day: u32 = date_parts
        .next()
        .and_then(|part| part.parse().ok())
        .ok_or_else(|| TimeError(input.to_owned()))?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return err(input);
    }

    let (hour, minute, second) = match time {
        Some(time) => split_time(time)?,
        None => (0, 0, 0),
    };

    Ok(days_from_civil(year, month, day) * DAY + hour * HOUR + minute * MINUTE + second)
}

/// Formats a timestamp as `YYYY-MM-DD HH:MM:SS` in UTC.
pub fn format_timestamp(timestamp: i64) -> String {
    let days = timestamp.div_euclid(DAY);
    let seconds = timestamp.rem_euclid(DAY);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        seconds / HOUR,
        (seconds % HOUR) / MINUTE,
        seconds % MINUTE
    )
}

/// Formats a timestamp as `YYYY-MM-DDTHH:MM:SSZ` (ISO 8601).
pub fn format_iso(timestamp: i64) -> String {
    let days = timestamp.div_euclid(DAY);
    let seconds = timestamp.rem_euclid(DAY);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / HOUR,
        (seconds % HOUR) / MINUTE,
        seconds % MINUTE
    )
}

/// Formats a timestamp the way `journalctl` does in its short output mode,
/// for example `Jul 15 12:00:00`.
pub fn format_short(timestamp: i64) -> String {
    let days = timestamp.div_euclid(DAY);
    let seconds = timestamp.rem_euclid(DAY);
    let (_, month, day) = civil_from_days(days);
    format!(
        "{} {day:02} {:02}:{:02}:{:02}",
        MONTH_NAMES[(month - 1) as usize],
        seconds / HOUR,
        (seconds % HOUR) / MINUTE,
        seconds % MINUTE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_anchors() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11017), (2000, 3, 1));
        assert_eq!(parse_timestamp("1970-01-01", 0).unwrap(), 0);
    }

    #[test]
    fn known_date_anchor() {
        // 2026-10-15T00:00:00Z
        assert_eq!(
            parse_timestamp("2026-10-15 00:00:00", 0).unwrap(),
            1_792_022_400
        );
        assert_eq!(parse_timestamp("2026-10-15", 0).unwrap(), 1_792_022_400);
        assert_eq!(format_timestamp(1_792_022_400), "2026-10-15 00:00:00");
        assert_eq!(format_iso(1_792_022_400), "2026-10-15T00:00:00Z");
        assert_eq!(format_short(1_792_022_400), "Oct 15 00:00:00");
    }

    #[test]
    fn civil_days_roundtrip() {
        for days in [-1000i64, -1, 0, 1, 10_000, 20_741, 100_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days, "{days}");
        }
    }

    #[test]
    fn absolute_time_forms() {
        assert_eq!(
            parse_timestamp("2026-10-15 12:34:56", 0).unwrap(),
            1_792_022_400 + 12 * HOUR + 34 * MINUTE + 56
        );
        assert_eq!(
            parse_timestamp("2026-10-15T12:34:56Z", 0).unwrap(),
            1_792_022_400 + 12 * HOUR + 34 * MINUTE + 56
        );
        assert_eq!(
            parse_timestamp("2026-10-15 12:34", 0).unwrap(),
            1_792_022_400 + 12 * HOUR + 34 * MINUTE
        );
        assert_eq!(parse_timestamp("@100", 0).unwrap(), 100);
    }

    #[test]
    fn relative_forms() {
        let now = 1_800_000_000;
        assert_eq!(parse_timestamp("now", now).unwrap(), now);
        assert_eq!(parse_timestamp("-30m", now).unwrap(), now - 30 * MINUTE);
        assert_eq!(parse_timestamp("-1h", now).unwrap(), now - HOUR);
        assert_eq!(parse_timestamp("-2d", now).unwrap(), now - 2 * DAY);
        assert_eq!(parse_timestamp("-1w", now).unwrap(), now - WEEK);
        assert_eq!(parse_timestamp("+15s", now).unwrap(), now + 15);
        assert_eq!(
            parse_timestamp("today", now).unwrap(),
            now.div_euclid(DAY) * DAY
        );
        assert_eq!(
            parse_timestamp("yesterday", now).unwrap(),
            now.div_euclid(DAY) * DAY - DAY
        );
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "   ",
            "not-a-date",
            "2026-13-01",
            "2026-10-99",
            "2026-10-15 25:00:00",
            "2026-10-15 12:60:00",
            "2026-10-15 12:00:00:00",
            "-1x",
            "2026",
            "2026-10",
        ] {
            assert!(parse_timestamp(bad, 0).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn negative_timestamps_are_handled() {
        // Pre-epoch timestamps must floor-divide, not truncate towards zero.
        assert_eq!(format_timestamp(-1), "1969-12-31 23:59:59");
        assert_eq!(parse_timestamp("1969-12-31 23:59:59", 0).unwrap(), -1);
    }
}
