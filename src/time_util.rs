//! Time range parsing and bucket-flooring helpers used by reports.

use anyhow::{Result, bail};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};

use crate::models::Granularity;

/// Parses a `--period` value into a concrete `[since, until)` UTC range,
/// anchored to "now".
pub fn parse_period(period: &str, now: DateTime<Utc>) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let p = period.to_ascii_lowercase();
    let today_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();

    let range = match p.as_str() {
        "today" => (today_start, now),
        "yesterday" => (today_start - Duration::days(1), today_start),
        "24h" | "day" => (now - Duration::hours(24), now),
        "7d" | "week" => (now - Duration::days(7), now),
        "30d" => (now - Duration::days(30), now),
        "90d" => (now - Duration::days(90), now),
        "month" => (month_start(now), now),
        "last-month" => {
            let this_month = month_start(now);
            let last_month_end = this_month;
            let last_month_start = month_start(this_month - Duration::days(1));
            (last_month_start, last_month_end)
        }
        "year" => (year_start(now), now),
        "all" => (Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap(), now),
        other => bail!(
            "unknown period '{other}' (expected: today|yesterday|24h|7d|30d|90d|month|last-month|year|all)"
        ),
    };
    Ok(range)
}

fn month_start(dt: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(dt.year(), dt.month(), 1, 0, 0, 0)
        .unwrap()
}

fn year_start(dt: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(dt.year(), 1, 1, 0, 0, 0).unwrap()
}

/// Resolves `--period` / `--from` / `--to` into a validated range. An explicit
/// `--from` overrides the period.
pub fn resolve_range(
    period: &str,
    from: Option<&str>,
    to: Option<&str>,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>)> {
    let (since, until) = match (from, to) {
        (Some(f), Some(t)) => (parse_timestamp(f)?, parse_timestamp(t)?),
        (Some(f), None) => (parse_timestamp(f)?, now),
        (None, Some(_)) => bail!("--to requires --from"),
        (None, None) => parse_period(period, now)?,
    };
    if since >= until {
        bail!("invalid range: start ({since}) must be before end ({until})");
    }
    Ok((since, until))
}

/// Picks a sensible default bucket granularity for a given range span.
pub fn auto_granularity(since: DateTime<Utc>, until: DateTime<Utc>) -> Granularity {
    let span = until - since;
    if span <= Duration::hours(48) {
        Granularity::Hour
    } else if span <= Duration::days(120) {
        Granularity::Day
    } else {
        Granularity::Month
    }
}

/// Floors a timestamp down to the start of its bucket for the given
/// granularity (UTC).
pub fn floor_to_bucket(dt: DateTime<Utc>, granularity: Granularity) -> DateTime<Utc> {
    match granularity {
        Granularity::Hour => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), 0, 0)
            .unwrap(),
        Granularity::Day => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), 0, 0, 0)
            .unwrap(),
        Granularity::Month => month_start(dt),
    }
}

/// Formats a bucket start timestamp for display, according to granularity.
pub fn format_bucket_label(dt: DateTime<Utc>, granularity: Granularity) -> String {
    match granularity {
        Granularity::Hour => dt.format("%Y-%m-%d %H:00").to_string(),
        Granularity::Day => dt.format("%Y-%m-%d").to_string(),
        Granularity::Month => dt.format("%Y-%m").to_string(),
    }
}

/// Parses an explicit `--from`/`--to` value: RFC3339 timestamp or `YYYY-MM-DD`.
pub fn parse_timestamp(s: &str) -> Result<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    bail!("could not parse timestamp '{s}' (expected RFC3339 or YYYY-MM-DD)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_to_bucket_hour() {
        let dt = Utc.with_ymd_and_hms(2026, 3, 15, 14, 37, 22).unwrap();
        assert_eq!(
            floor_to_bucket(dt, Granularity::Hour),
            Utc.with_ymd_and_hms(2026, 3, 15, 14, 0, 0).unwrap()
        );
    }

    #[test]
    fn floor_to_bucket_day_and_month() {
        let dt = Utc.with_ymd_and_hms(2026, 3, 15, 14, 37, 22).unwrap();
        assert_eq!(
            floor_to_bucket(dt, Granularity::Day),
            Utc.with_ymd_and_hms(2026, 3, 15, 0, 0, 0).unwrap()
        );
        assert_eq!(
            floor_to_bucket(dt, Granularity::Month),
            Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn auto_granularity_picks_sensible_buckets() {
        let now = Utc::now();
        assert_eq!(
            auto_granularity(now - Duration::hours(6), now),
            Granularity::Hour
        );
        assert_eq!(
            auto_granularity(now - Duration::days(10), now),
            Granularity::Day
        );
        assert_eq!(
            auto_granularity(now - Duration::days(400), now),
            Granularity::Month
        );
    }

    #[test]
    fn parse_period_today_starts_at_midnight_utc() {
        let now = Utc.with_ymd_and_hms(2026, 6, 10, 18, 30, 0).unwrap();
        let (since, until) = parse_period("today", now).unwrap();
        assert_eq!(since, Utc.with_ymd_and_hms(2026, 6, 10, 0, 0, 0).unwrap());
        assert_eq!(until, now);
    }

    #[test]
    fn parse_period_rejects_unknown_value() {
        assert!(parse_period("nonsense", Utc::now()).is_err());
    }

    #[test]
    fn parse_timestamp_accepts_date_and_rfc3339() {
        let a = parse_timestamp("2026-01-05").unwrap();
        assert_eq!(a, Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap());
        let b = parse_timestamp("2026-01-05T10:00:00Z").unwrap();
        assert_eq!(b, Utc.with_ymd_and_hms(2026, 1, 5, 10, 0, 0).unwrap());
        assert!(parse_timestamp("not-a-date").is_err());
    }

    #[test]
    fn granularity_from_str_roundtrip() {
        assert_eq!("hour".parse::<Granularity>().unwrap(), Granularity::Hour);
        assert_eq!("daily".parse::<Granularity>().unwrap(), Granularity::Day);
        assert_eq!("month".parse::<Granularity>().unwrap(), Granularity::Month);
        assert!("fortnight".parse::<Granularity>().is_err());
    }
}
