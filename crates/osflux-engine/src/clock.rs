use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// Formats Unix seconds as a UTC timestamp such as `2026-11-18 17:45:12 UTC`.
pub fn format_unix_utc(unix_seconds: u64) -> String {
    let (date, time) = utc_date_and_time(unix_seconds);
    format!("{date} {time} UTC")
}

/// Formats Unix seconds as an RFC 3339 UTC timestamp such as
/// `2026-11-18T17:45:12Z`, for logs that other tools parse.
pub fn format_unix_rfc3339(unix_seconds: u64) -> String {
    let (date, time) = utc_date_and_time(unix_seconds);
    format!("{date}T{time}Z")
}

fn utc_date_and_time(unix_seconds: u64) -> (String, String) {
    let seconds_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days((unix_seconds / 86_400) as i64);

    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!(
            "{:02}:{:02}:{:02}",
            seconds_of_day / 3600,
            (seconds_of_day % 3600) / 60,
            seconds_of_day % 60
        ),
    )
}

/// Converts days since 1970-01-01 into a proleptic Gregorian date, using
/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{format_unix_rfc3339, format_unix_utc};

    #[test]
    fn formats_known_timestamps_in_utc() {
        assert_eq!(format_unix_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_unix_utc(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(format_unix_utc(1_795_022_016), "2026-11-18 17:13:36 UTC");
        assert_eq!(format_unix_utc(4_102_444_799), "2099-12-31 23:59:59 UTC");
    }

    #[test]
    fn formats_rfc3339_timestamps() {
        assert_eq!(format_unix_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_rfc3339(1_795_022_016), "2026-11-18T17:13:36Z");
    }
}
