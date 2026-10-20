//! Timestamps in the forms agents write them, without a date library.

pub fn timestamp_now() -> String {
    let ms = crate::now_ms();
    format_timestamp(ms)
}

/// RFC 3339 in UTC with milliseconds, as agents write them.
pub fn format_timestamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}


/// Local wall-clock offset is not known without a time zone database, so
/// file names that agents date in local time use UTC; agents find their
/// sessions by id, not by file name.
pub fn file_stamp(ms: i64) -> String {
    format_timestamp(ms)[..19].replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(format_timestamp(1_760_961_600_123), "2025-10-20T12:00:00.123Z");
        assert_eq!(format_timestamp(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(file_stamp(1_760_961_600_123), "2025-10-20T12-00-00");
    }
}
