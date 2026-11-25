//! The local time of day, for "Drives scanned 13:28" in the status bar.

/// The local time as `HH:MM`, or an empty string when it cannot be read.
#[cfg(windows)]
pub fn local_time_of_day() -> String {
    // SAFETY: GetLocalTime only fills in the structure it returns.
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:02}:{:02}", now.wHour, now.wMinute)
}

/// The local time as `HH:MM`, or an empty string when it cannot be read.
#[cfg(unix)]
pub fn local_time_of_day() -> String {
    // SAFETY: `time` and `localtime_r` only write to the values passed in,
    // and `tm` is fully initialised by a successful `localtime_r`.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut local).is_null() {
            return String::new();
        }
        format!("{:02}:{:02}", local.tm_hour, local.tm_min)
    }
}

/// The local time as `HH:MM`, or an empty string when it cannot be read.
#[cfg(not(any(windows, unix)))]
pub fn local_time_of_day() -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::local_time_of_day;

    #[test]
    fn the_time_is_hours_and_minutes() {
        let time = local_time_of_day();
        assert_eq!(time.len(), 5, "{time}");
        let (hours, minutes) = time.split_once(':').expect("HH:MM");
        assert!(hours.parse::<u8>().unwrap() < 24);
        assert!(minutes.parse::<u8>().unwrap() < 60);
    }
}
