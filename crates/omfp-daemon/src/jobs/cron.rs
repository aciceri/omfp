//! Cron schedules of declared jobs (DMN-323): five fields — minute, hour, day of month, month, day of week — or a
//! macro, matched in a time zone (the host's local one by default) where a local time a daylight-saving change skips
//! does not fire and one it repeats fires once, at its first occurrence.

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::{AmbiguousOffset, TimeZone};

/// How many years ahead a schedule is looked for: a time that matches is found well within (the 29th of February on
/// a given weekday repeats within 28 years), one that never matches (the 30th of February) gives up.
const SEARCH_YEARS: i16 = 30;

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// Why a cron expression is refused; [`CronError::Field`] names the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CronError {
    #[error(
        "expected 5 fields (minute hour day-of-month month day-of-week) or a macro such as @daily, found {0}"
    )]
    FieldCount(usize),
    #[error("@reboot is not supported: a job that must run when the daemon starts is a service")]
    Reboot,
    #[error(
        "unknown macro {0}: expected @yearly, @annually, @monthly, @weekly, @daily, @midnight or @hourly"
    )]
    UnknownMacro(String),
    #[error("{field}: {reason}")]
    Field { field: &'static str, reason: String },
}

/// A parsed cron expression (DMN-323).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    /// Bit `n` set: minute `n` (0–59) matches.
    minutes: u64,
    /// Bit `n` set: hour `n` (0–23) matches.
    hours: u32,
    /// Bit `n` set: day of month `n` (1–31) matches.
    days: u32,
    /// Bit `n` set: month `n` (1–12) matches.
    months: u16,
    /// Bit `n` set: weekday `n` (0–6, Sunday 0) matches.
    weekdays: u8,
    /// Whether the day-of-month field does not start with `*` (Vixie cron's rule for the two day fields).
    days_restricted: bool,
    /// Whether the day-of-week field does not start with `*`.
    weekdays_restricted: bool,
}

/// One field's rules: its name in errors, its range and the names it accepts (index + `name_base` = value).
struct Field {
    name: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
    name_base: u32,
}

const MINUTE: Field = Field {
    name: "minute",
    min: 0,
    max: 59,
    names: &[],
    name_base: 0,
};
const HOUR: Field = Field {
    name: "hour",
    min: 0,
    max: 23,
    names: &[],
    name_base: 0,
};
const DAY_OF_MONTH: Field = Field {
    name: "day of month",
    min: 1,
    max: 31,
    names: &[],
    name_base: 0,
};
const MONTH: Field = Field {
    name: "month",
    min: 1,
    max: 12,
    names: &MONTHS,
    name_base: 1,
};
const DAY_OF_WEEK: Field = Field {
    name: "day of week",
    min: 0,
    max: 7,
    names: &DAYS,
    name_base: 0,
};

impl Field {
    fn error(&self, reason: String) -> CronError {
        CronError::Field {
            field: self.name,
            reason,
        }
    }

    /// The set of values `text` selects, bit `n` for value `n`.
    fn parse(&self, text: &str) -> Result<u64, CronError> {
        let mut set = 0u64;
        for item in text.split(',') {
            let (base, step) = match item.split_once('/') {
                Some((base, step)) => (base, Some(self.step(step)?)),
                None => (item, None),
            };
            let (low, high) = if base == "*" {
                (self.min, self.max)
            } else if let Some((low, high)) = base.split_once('-') {
                let (low, high) = (self.value(low)?, self.value(high)?);
                if low > high {
                    return Err(self.error(format!("range {low}-{high} goes backwards")));
                }
                (low, high)
            } else if step.is_some() {
                return Err(self.error(format!("a step needs * or a range before it: {item}")));
            } else {
                let value = self.value(base)?;
                (value, value)
            };
            let step = step.unwrap_or(1);
            let mut value = low;
            while value <= high {
                set |= 1 << value;
                let Some(next) = value.checked_add(step) else {
                    break;
                };
                value = next;
            }
        }
        Ok(set)
    }

    fn step(&self, text: &str) -> Result<u32, CronError> {
        match text.parse::<u32>() {
            Ok(0) => Err(self.error("a step is at least 1".to_owned())),
            Ok(step) => Ok(step),
            Err(_) => Err(self.error(format!("step \"{text}\" is not a number"))),
        }
    }

    fn value(&self, text: &str) -> Result<u32, CronError> {
        if let Some(index) = self
            .names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(text))
        {
            return Ok(u32::try_from(index).unwrap_or(0) + self.name_base);
        }
        let Ok(value) = text.parse::<u32>() else {
            let reason = match self.names.first() {
                None => format!("\"{text}\" is not a number"),
                Some(first) => format!(
                    "\"{text}\" is not a number or a name ({first}…{})",
                    self.names.last().unwrap_or(first)
                ),
            };
            return Err(self.error(reason));
        };
        if value < self.min || value > self.max {
            return Err(self.error(format!("{value} is out of range {}-{}", self.min, self.max)));
        }
        Ok(value)
    }
}

/// The lowest set bit of `set` at or above `from`, below 64.
fn next_bit(set: u64, from: i8) -> Option<i8> {
    let from = u32::try_from(from).ok()?;
    if from >= 64 {
        return None;
    }
    let above = set >> from;
    (above != 0)
        .then(|| i8::try_from(from + above.trailing_zeros()).ok())
        .flatten()
}

fn has_bit(set: u64, value: i8) -> bool {
    u32::try_from(value).is_ok_and(|value| value < 64 && set & (1 << value) != 0)
}

impl Schedule {
    /// Parses a five-field cron expression or a macro (DMN-323).
    pub fn parse(expression: &str) -> Result<Self, CronError> {
        let expression = expression.trim();
        if let Some(name) = expression.strip_prefix('@') {
            let fields = match name.to_ascii_lowercase().as_str() {
                "yearly" | "annually" => "0 0 1 1 *",
                "monthly" => "0 0 1 * *",
                "weekly" => "0 0 * * 0",
                "daily" | "midnight" => "0 0 * * *",
                "hourly" => "0 * * * *",
                "reboot" => return Err(CronError::Reboot),
                _ => return Err(CronError::UnknownMacro(expression.to_owned())),
            };
            return Self::parse(fields);
        }
        let fields: Vec<&str> = expression.split_ascii_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return Err(CronError::FieldCount(fields.len()));
        };
        // Fields are checked in order, so the first broken one is named.
        Ok(Self {
            minutes: MINUTE.parse(minute)?,
            hours: u32::try_from(HOUR.parse(hour)?).unwrap_or(0),
            days: u32::try_from(DAY_OF_MONTH.parse(day)?).unwrap_or(0),
            months: u16::try_from(MONTH.parse(month)?).unwrap_or(0),
            // Day 7 is Sunday, like 0.
            weekdays: DAY_OF_WEEK
                .parse(weekday)
                .map(|days| u8::try_from((days | (days >> 7)) & 0x7f).unwrap_or(0))?,
            days_restricted: !day.starts_with('*'),
            weekdays_restricted: !weekday.starts_with('*'),
        })
    }

    /// Whether `date`'s day matches: with both day fields restricted, either one (Vixie cron), else both.
    fn day_matches(&self, date: Date) -> bool {
        let day = has_bit(u64::from(self.days), date.day());
        let weekday = has_bit(
            u64::from(self.weekdays),
            date.weekday().to_sunday_zero_offset(),
        );
        if self.days_restricted && self.weekdays_restricted {
            day || weekday
        } else {
            day && weekday
        }
    }

    /// The first local time at or after `from` (taken to the minute) that matches, `None` within
    /// [`SEARCH_YEARS`].
    fn next_local(&self, from: DateTime) -> Option<DateTime> {
        let limit = from.year().saturating_add(SEARCH_YEARS);
        let (mut date, mut hour, mut minute) = (from.date(), from.hour(), from.minute());
        loop {
            if date.year() > limit {
                return None;
            }
            if !has_bit(u64::from(self.months), date.month()) {
                date = date.last_of_month().tomorrow().ok()?;
                (hour, minute) = (0, 0);
                continue;
            }
            if !self.day_matches(date) {
                date = date.tomorrow().ok()?;
                (hour, minute) = (0, 0);
                continue;
            }
            let Some(next_hour) = next_bit(u64::from(self.hours), hour) else {
                date = date.tomorrow().ok()?;
                (hour, minute) = (0, 0);
                continue;
            };
            if next_hour != hour {
                (hour, minute) = (next_hour, 0);
            }
            match next_bit(self.minutes, minute) {
                Some(minute) => return Some(date.at(hour, minute, 0, 0)),
                None if hour < 23 => (hour, minute) = (hour + 1, 0),
                None => {
                    date = date.tomorrow().ok()?;
                    (hour, minute) = (0, 0);
                }
            }
        }
    }

    /// The next due time strictly after `after_ms` (Unix ms) in `tz`, as Unix ms: a local time a daylight-saving
    /// change skips does not fire, one it repeats fires once (its first occurrence). `None` when no time matches
    /// within 30 years (the 30th of February).
    #[must_use]
    pub fn next_after(&self, after_ms: i64, tz: &TimeZone) -> Option<i64> {
        let after = Timestamp::from_millisecond(after_ms).ok()?;
        let local = tz.to_datetime(after);
        // Local times before this one are all due before `after`: the first occurrence of a local time is never
        // later than the instant it names.
        let mut from = local.date().at(local.hour(), local.minute(), 0, 0);
        loop {
            let due = self.next_local(from)?;
            let offset = match tz.to_ambiguous_timestamp(due).offset() {
                AmbiguousOffset::Unambiguous { offset } => Some(offset),
                AmbiguousOffset::Fold { before, .. } => Some(before),
                AmbiguousOffset::Gap { .. } => None,
            };
            if let Some(at) = offset.and_then(|offset| offset.to_timestamp(due).ok()) {
                let at = at.as_millisecond();
                if at > after_ms {
                    return Some(at);
                }
            }
            from = due.checked_add(jiff::SignedDuration::from_mins(1)).ok()?;
        }
    }

    /// [`Schedule::next_after`] in the host's local time zone (DMN-323).
    #[must_use]
    pub fn next_after_local(&self, after_ms: i64) -> Option<i64> {
        self.next_after(after_ms, &TimeZone::system())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Europe/Rome` as its POSIX rule, so no test depends on the host's time zone database: CET, CEST from the last
    /// Sunday of March 02:00 to the last Sunday of October 03:00.
    fn rome() -> TimeZone {
        TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap()
    }

    /// `America/New_York` as its POSIX rule: EST, EDT from the second Sunday of March 02:00 to the first Sunday of
    /// November 02:00.
    fn new_york() -> TimeZone {
        TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap()
    }

    fn ms(instant: &str) -> i64 {
        instant.parse::<Timestamp>().unwrap().as_millisecond()
    }

    fn utc(ms: i64) -> String {
        Timestamp::from_millisecond(ms).unwrap().to_string()
    }

    /// The next `count` due times after `after` in `tz`, as UTC instants.
    fn due(expression: &str, after: &str, tz: &TimeZone, count: usize) -> Vec<String> {
        let schedule = Schedule::parse(expression).unwrap();
        let mut at = ms(after);
        let mut times = Vec::new();
        for _ in 0..count {
            at = schedule.next_after(at, tz).unwrap();
            times.push(utc(at));
        }
        times
    }

    fn error(expression: &str) -> String {
        Schedule::parse(expression).unwrap_err().to_string()
    }

    // spec: DMN-323
    #[test]
    fn a_schedule_fires_at_the_matching_minutes_strictly_after_the_given_time() {
        let utc_zone = TimeZone::UTC;
        assert_eq!(
            due("0 8 * * 1-5", "2026-11-13T07:59:00Z", &utc_zone, 3),
            [
                "2026-11-13T08:00:00Z",
                "2026-11-16T08:00:00Z",
                "2026-11-17T08:00:00Z"
            ],
            "Friday, then Monday and Tuesday"
        );
        assert_eq!(
            due("0 8 * * 1-5", "2026-11-13T08:00:00Z", &utc_zone, 1),
            ["2026-11-16T08:00:00Z"],
            "strictly after"
        );
        assert_eq!(
            due("30 9 * * *", "2026-11-16T09:30:42.500Z", &utc_zone, 1),
            ["2026-11-17T09:30:00Z"],
            "seconds within the due minute are after it"
        );
        assert_eq!(
            due("*/20 * * * *", "2026-11-16T10:41:00Z", &utc_zone, 4),
            [
                "2026-11-16T11:00:00Z",
                "2026-11-16T11:20:00Z",
                "2026-11-16T11:40:00Z",
                "2026-11-16T12:00:00Z"
            ]
        );
        assert_eq!(
            due("5,10-20/5 2-3 * * *", "2026-11-16T00:00:00Z", &utc_zone, 5),
            [
                "2026-11-16T02:05:00Z",
                "2026-11-16T02:10:00Z",
                "2026-11-16T02:15:00Z",
                "2026-11-16T02:20:00Z",
                "2026-11-16T03:05:00Z"
            ],
            "lists, ranges with a step, hour ranges"
        );
        assert_eq!(
            due("0 0 31 * *", "2026-11-01T00:00:00Z", &utc_zone, 2),
            ["2026-12-31T00:00:00Z", "2027-01-31T00:00:00Z"],
            "months without a 31st are passed over"
        );
        assert_eq!(
            due("0 12 29 2 *", "2026-11-16T00:00:00Z", &utc_zone, 2),
            ["2028-02-29T12:00:00Z", "2032-02-29T12:00:00Z"]
        );
        assert_eq!(
            Schedule::parse("0 0 30 2 *")
                .unwrap()
                .next_after(ms("2026-11-16T00:00:00Z"), &utc_zone),
            None,
            "a time that never comes"
        );
    }

    // spec: DMN-323
    #[test]
    fn with_both_day_fields_restricted_either_one_matches() {
        let utc_zone = TimeZone::UTC;
        // The 13th, or any Friday (2026-11-13 is a Friday).
        assert_eq!(
            due("0 0 13 * 5", "2026-11-01T00:00:00Z", &utc_zone, 4),
            [
                "2026-11-06T00:00:00Z",
                "2026-11-13T00:00:00Z",
                "2026-11-20T00:00:00Z",
                "2026-11-27T00:00:00Z"
            ]
        );
        assert_eq!(
            due("0 0 1 * 3", "2026-11-26T00:00:00Z", &utc_zone, 3),
            [
                "2026-12-01T00:00:00Z",
                "2026-12-02T00:00:00Z",
                "2026-12-09T00:00:00Z"
            ],
            "the 1st (a Tuesday) and Wednesdays"
        );
        // One field `*`: only the other one restricts.
        assert_eq!(
            due("0 0 13 * *", "2026-11-01T00:00:00Z", &utc_zone, 2),
            ["2026-11-13T00:00:00Z", "2026-12-13T00:00:00Z"]
        );
        assert_eq!(
            due("0 0 * * 5", "2026-11-01T00:00:00Z", &utc_zone, 2),
            ["2026-11-06T00:00:00Z", "2026-11-13T00:00:00Z"]
        );
    }

    // spec: DMN-323
    #[test]
    fn names_are_case_insensitive_and_seven_is_sunday() {
        assert_eq!(
            Schedule::parse("0 9 * JAN,mar-May Mon-fri"),
            Schedule::parse("0 9 * 1,3-5 1-5")
        );
        assert_eq!(Schedule::parse("0 9 * * sun"), Schedule::parse("0 9 * * 7"));
        assert_eq!(Schedule::parse("0 9 * * 0"), Schedule::parse("0 9 * * 7"));
        assert_eq!(
            Schedule::parse("0 9 * * 5-7"),
            Schedule::parse("0 9 * * fri,sat,SUN")
        );
        let utc_zone = TimeZone::UTC;
        assert_eq!(
            due("0 9 * dec sun", "2026-11-16T00:00:00Z", &utc_zone, 2),
            ["2026-12-06T09:00:00Z", "2026-12-13T09:00:00Z"]
        );
    }

    // spec: DMN-323
    #[test]
    fn macros_stand_for_their_usual_expressions() {
        for (name, fields) in [
            ("@yearly", "0 0 1 1 *"),
            ("@annually", "0 0 1 1 *"),
            ("@monthly", "0 0 1 * *"),
            ("@weekly", "0 0 * * 0"),
            ("@daily", "0 0 * * *"),
            ("@midnight", "0 0 * * *"),
            ("@hourly", "0 * * * *"),
            ("@DAILY", "0 0 * * *"),
        ] {
            assert_eq!(Schedule::parse(name), Schedule::parse(fields), "{name}");
        }
        assert_eq!(
            due("@weekly", "2026-11-16T00:00:00Z", &TimeZone::UTC, 1),
            ["2026-11-22T00:00:00Z"]
        );
    }

    // spec: DMN-323
    #[test]
    fn an_invalid_expression_names_the_field() {
        for (expression, message) in [
            ("0 25 * * *", "hour: 25 is out of range 0-23"),
            ("60 * * * *", "minute: 60 is out of range 0-59"),
            ("0 0 0 * *", "day of month: 0 is out of range 1-31"),
            ("0 0 32 * *", "day of month: 32 is out of range 1-31"),
            ("0 0 * 13 *", "month: 13 is out of range 1-12"),
            ("0 0 * * 8", "day of week: 8 is out of range 0-7"),
            ("x * * * *", "minute: \"x\" is not a number"),
            (
                "0 0 * foo *",
                "month: \"foo\" is not a number or a name (jan…dec)",
            ),
            (
                "0 0 * * mon-xyz",
                "day of week: \"xyz\" is not a number or a name (sun…sat)",
            ),
            ("0 mon * * *", "hour: \"mon\" is not a number"),
            ("0 1,,2 * * *", "hour: \"\" is not a number"),
            ("0 5-3 * * *", "hour: range 5-3 goes backwards"),
            ("*/0 * * * *", "minute: a step is at least 1"),
            ("*/x * * * *", "minute: step \"x\" is not a number"),
            (
                "5/10 * * * *",
                "minute: a step needs * or a range before it: 5/10",
            ),
            (
                "0 0 * *",
                "expected 5 fields (minute hour day-of-month month day-of-week) or a macro such as @daily, found 4",
            ),
            (
                "0 0 0 * * *",
                "expected 5 fields (minute hour day-of-month month day-of-week) or a macro such as @daily, found 6",
            ),
            (
                "",
                "expected 5 fields (minute hour day-of-month month day-of-week) or a macro such as @daily, found 0",
            ),
            (
                "@reboot",
                "@reboot is not supported: a job that must run when the daemon starts is a service",
            ),
            (
                "@often",
                "unknown macro @often: expected @yearly, @annually, @monthly, @weekly, @daily, @midnight or @hourly",
            ),
        ] {
            assert_eq!(error(expression), message, "{expression:?}");
        }
    }

    // spec: DMN-323
    #[test]
    fn a_local_time_skipped_by_daylight_saving_does_not_fire() {
        // Rome, 2026-03-29: 02:00 CET jumps to 03:00 CEST.
        assert_eq!(
            due("30 2 * * *", "2026-03-28T00:00:00Z", &rome(), 2),
            ["2026-03-28T01:30:00Z", "2026-03-30T00:30:00Z"],
            "02:30 on the 28th (CET), none on the 29th, 02:30 CEST on the 30th"
        );
        assert_eq!(
            due("*/30 * * * *", "2026-03-29T00:15:00Z", &rome(), 2),
            ["2026-03-29T00:30:00Z", "2026-03-29T01:00:00Z"],
            "01:30 CET, then 03:00 CEST: 02:00 and 02:30 never happen"
        );
        // New York, 2026-03-08: 02:00 EST jumps to 03:00 EDT.
        assert_eq!(
            due("15 2 * * *", "2026-03-07T12:00:00Z", &new_york(), 2),
            ["2026-03-09T06:15:00Z", "2026-03-10T06:15:00Z"],
            "02:15 does not exist on the 8th"
        );
    }

    // spec: DMN-323
    #[test]
    fn a_local_time_repeated_by_daylight_saving_fires_once() {
        // Rome, 2026-10-25: 03:00 CEST goes back to 02:00 CET, so 02:00–02:59 happen twice.
        assert_eq!(
            due("30 2 * * *", "2026-10-24T12:00:00Z", &rome(), 2),
            ["2026-10-25T00:30:00Z", "2026-10-26T01:30:00Z"],
            "the first 02:30 (CEST) only"
        );
        assert_eq!(
            due("*/30 * * * *", "2026-10-24T23:45:00Z", &rome(), 4),
            [
                "2026-10-25T00:00:00Z",
                "2026-10-25T00:30:00Z",
                "2026-10-25T02:00:00Z",
                "2026-10-25T02:30:00Z"
            ],
            "02:00 and 02:30 CEST, not again in CET, then 03:00 and 03:30 CET"
        );
        assert_eq!(
            Schedule::parse("45 2 * * *")
                .unwrap()
                .next_after(ms("2026-10-25T01:10:00Z"), &rome())
                .map(utc),
            Some("2026-10-26T01:45:00Z".to_owned()),
            "within the repeated hour, its times already fired"
        );
        // New York, 2026-11-01: 02:00 EDT goes back to 01:00 EST.
        assert_eq!(
            due("30 1 * * *", "2026-10-31T12:00:00Z", &new_york(), 2),
            ["2026-11-01T05:30:00Z", "2026-11-02T06:30:00Z"]
        );
    }
}
