//! Canonical app-wide time primitives and formatting helpers built on Jiff.

use std::time::Duration;

use jiff::{Timestamp, tz::TimeZone};

pub type AppTimestamp = Timestamp;

#[inline]
pub fn now() -> AppTimestamp {
    Timestamp::now()
}

#[inline]
pub fn now_unix_seconds() -> i64 {
    now().as_second()
}

#[inline]
pub fn parse_timestamp(value: &str) -> Option<AppTimestamp> {
    value.parse().ok()
}

#[inline]
pub fn format_rfc3339(timestamp: AppTimestamp) -> String {
    timestamp.to_string()
}

#[inline]
pub fn format_utc(timestamp: AppTimestamp, fmt: &str) -> String {
    timestamp.strftime(fmt).to_string()
}

#[inline]
pub fn format_local(timestamp: AppTimestamp, fmt: &str) -> String {
    timestamp
        .to_zoned(TimeZone::system())
        .strftime(fmt)
        .to_string()
}

#[inline]
pub fn age_seconds_since(timestamp: AppTimestamp, now_unix: i64) -> i64 {
    now_unix.saturating_sub(timestamp.as_second()).max(0)
}

#[inline]
pub fn non_negative_duration_between(earlier: AppTimestamp, later: AppTimestamp) -> Duration {
    let secs = later.as_second().saturating_sub(earlier.as_second()).max(0) as u64;
    Duration::from_secs(secs)
}

#[inline]
pub fn age_duration_from_timestamp(
    created_at: Option<AppTimestamp>,
    now: AppTimestamp,
) -> Option<Duration> {
    created_at.map(|timestamp| non_negative_duration_between(timestamp, now))
}
