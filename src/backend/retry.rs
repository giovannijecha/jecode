//! Shared HTTP Retry-After parsing.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) fn retry_delay(text: &str) -> Option<Duration> {
    if let Ok(seconds) = text.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    // IMF-fixdate, as used by HTTP Retry-After. Compare UTC wall time once;
    // the recovery loop subsequently waits on a monotonic clock.
    let parts: Vec<_> = text.split_whitespace().collect();
    if parts.len() != 6 || parts[5] != "GMT" {
        return None;
    }
    let day: i64 = parts[1].parse().ok()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|month| *month == parts[2])? as i64
        + 1;
    let year: i64 = parts[3].parse().ok()?;
    let time: Vec<i64> = parts[4]
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if !(1970..=9999).contains(&year)
        || !(1..=31).contains(&day)
        || time.len() != 3
        || !(0..24).contains(&time[0])
        || !(0..60).contains(&time[1])
        || !(0..60).contains(&time[2])
    {
        return None;
    }
    let mut days = 0i64;
    let leap = |year| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    for year in 1970..year {
        days += if leap(year) { 366 } else { 365 };
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day > months[(month - 1) as usize] {
        return None;
    }
    days += months.iter().take((month - 1) as usize).sum::<i64>() + day - 1;
    let target = (days * 86400 + time[0] * 3600 + time[1] * 60 + time[2]) as u64;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(target.saturating_sub(now)))
}
