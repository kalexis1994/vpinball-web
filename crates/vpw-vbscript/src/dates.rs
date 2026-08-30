//! Dates, as VBScript counts them.
//!
//! A date is a **number**: whole days since the thirtieth of December 1899,
//! with the time of day in the fraction. Noon on the day it counts from is
//! 0.5, the next day is 1, and the day before it is −1. That is not a
//! representation this port chose; it is what a VBScript date *is*, which is
//! why `Now() + 1` is tomorrow and why a date compares against a number
//! without complaint.
//!
//! # The awkward corner
//!
//! Negative dates — anything before 1899 — do not carry the fraction the way
//! you would expect. The day is negative and the time of day is still counted
//! forwards, so −1.25 is six in the morning on the twenty-ninth, not eighteen
//! hundred on it. Nothing in a pinball table goes near that; it is written
//! down because the sign handling below looks wrong until you know.

/// The day the count starts from, as days since the Unix epoch.
///
/// The thirtieth of December 1899 is 25569 days before the first of January
/// 1970, which is the other epoch every clock this port can reach counts from.
const EPOCH_OFFSET_DAYS: f64 = 25569.0;

const MS_PER_DAY: f64 = 86_400_000.0;

/// A wall-clock reading, in milliseconds since the Unix epoch, as a VBScript
/// date.
pub fn from_unix_millis(ms: f64) -> f64 {
    ms / MS_PER_DAY + EPOCH_OFFSET_DAYS
}

/// The whole days in a date, rounded **towards zero**, which is what leaves
/// the time of day in the fraction on both sides of the epoch.
fn whole_days(date: f64) -> i64 {
    date.trunc() as i64
}

/// The time of day, in seconds, from a date.
fn seconds_of_day(date: f64) -> f64 {
    let fraction = date.fract().abs();
    (fraction * 86_400.0).round()
}

/// Midnight of the day a date falls on.
pub fn date_part(date: f64) -> f64 {
    date.trunc()
}

/// The time of day on its own, as a date in the first day.
pub fn time_part(date: f64) -> f64 {
    date.fract().abs()
}

pub fn hour(date: f64) -> i32 {
    (seconds_of_day(date) / 3600.0) as i32 % 24
}

pub fn minute(date: f64) -> i32 {
    (seconds_of_day(date) / 60.0) as i32 % 60
}

pub fn second(date: f64) -> i32 {
    seconds_of_day(date) as i32 % 60
}

/// The year, month and day a date falls on.
///
/// Days to a civil date by Howard Hinnant's algorithm, which is the one
/// everybody uses because it is exact for the whole proleptic Gregorian
/// calendar and has no loops in it.
pub fn civil(date: f64) -> (i32, u32, u32) {
    // Rebased on the first of March, which is what makes the leap day the
    // last day of the year and the arithmetic fall out.
    let days = whole_days(date) - i64::from(EPOCH_OFFSET_DAYS as i32) + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year as i32, month, day)
}

/// Sunday is 1, which is VBScript's default and the only one a table uses.
pub fn weekday(date: f64) -> i32 {
    // The thirtieth of December 1899 was a Saturday, so day 0 is a Saturday
    // and Sunday is day 1.
    let days = whole_days(date);
    (days.rem_euclid(7) + 6) as i32 % 7 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The day the count starts from, and the two either side of it.
    #[test]
    fn the_epoch_is_the_thirtieth_of_december_1899() {
        assert_eq!(civil(0.0), (1899, 12, 30));
        assert_eq!(civil(1.0), (1899, 12, 31));
        assert_eq!(civil(2.0), (1900, 1, 1));
        // Which was a Saturday, and Sunday is one.
        assert_eq!(weekday(0.0), 7);
        assert_eq!(weekday(1.0), 1);
    }

    #[test]
    fn the_fraction_is_the_time_of_day() {
        assert_eq!(hour(0.5), 12);
        assert_eq!(minute(0.5), 0);
        assert_eq!(second(0.5), 0);

        // Twenty-three fifty-nine and fifty-nine.
        let end = 86_399.0 / 86_400.0;
        assert_eq!(hour(end), 23);
        assert_eq!(minute(end), 59);
        assert_eq!(second(end), 59);
    }

    /// The one number worth checking against something outside this file: the
    /// spreadsheet epoch, which counts the same way and which everybody has a
    /// copy of.
    #[test]
    fn a_known_date_lands_where_it_should() {
        // The first of January 2000 is 36526 in every program that counts
        // this way.
        assert_eq!(civil(36_526.0), (2000, 1, 1));
        // And the Unix epoch is 25569.
        assert_eq!(civil(from_unix_millis(0.0)), (1970, 1, 1));
        assert_eq!(hour(from_unix_millis(0.0)), 0);
    }

    #[test]
    fn a_wall_clock_reading_becomes_a_date() {
        // Midday on the first of January 2000, UTC.
        let ms = 946_728_000_000.0;
        let d = from_unix_millis(ms);
        assert_eq!(civil(d), (2000, 1, 1));
        assert_eq!(hour(d), 12);
        assert_eq!(date_part(d), 36_526.0);
        assert!((time_part(d) - 0.5).abs() < 1e-9);
    }
}
