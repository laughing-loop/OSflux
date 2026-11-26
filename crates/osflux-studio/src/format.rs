//! How sizes, counts and long names are written on screen.

/// A size for the screens, in binary units labelled the way Windows
/// Explorer labels them, so a "64GB" stick reads "59.6 GB" in both: three
/// significant digits. The exact byte count goes in the technical details.
/// The engine writes sizes this way too, in the confirmation that names a
/// drive, so the screen and the confirmation agree.
pub fn size_short(bytes: u64) -> String {
    osflux_engine::size_label(bytes)
}

/// The exact size for technical details: `2,832,413,696 B (2.64 GiB)`.
pub fn bytes_exact(bytes: u64) -> String {
    let short = size_short(bytes);
    if bytes < 1024 {
        return with_thousands(bytes) + " B";
    }
    // The technical panels name binary units exactly.
    let binary = short
        .replace(" KB", " KiB")
        .replace(" MB", " MiB")
        .replace(" GB", " GiB")
        .replace(" TB", " TiB");
    format!("{} B ({binary})", with_thousands(bytes))
}

/// `2832413696` as `2,832,413,696`.
pub fn with_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Shortens a file name in the middle, keeping its start and its ending
/// (with the extension) so similar names stay distinguishable:
/// `ubuntu-24.04.1-…-amd64.iso`. Cuts fall on `-`, `_`, `.` or spaces when
/// one is close enough. Only names in the rail and step subtitles are
/// shortened; sizes, paths and codes never are.
pub fn middle_truncate(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let count = chars.len();
    if count <= max_chars || max_chars < 5 {
        return text.to_string();
    }
    let is_separator = |c: char| matches!(c, '-' | '_' | '.' | ' ');
    let keep = max_chars - 1;

    // The ending: the longest part that starts at a separator and fits in
    // half the room, unless that would waste more than half of it.
    let tail_room = keep / 2;
    let tail = (1..=tail_room)
        .rev()
        .find(|&length| is_separator(chars[count - length]))
        .filter(|&length| length >= tail_room / 2)
        .unwrap_or(tail_room);

    // The start: the longest part that ends at a separator in the rest.
    let head_room = keep - tail;
    let head = (1..=head_room)
        .rev()
        .find(|&length| is_separator(chars[length - 1]))
        .filter(|&length| length >= head_room / 2)
        .unwrap_or(head_room);

    let start: String = chars[..head].iter().collect();
    let end: String = chars[count - tail..].iter().collect();
    format!("{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::{bytes_exact, middle_truncate, size_short, with_thousands};

    #[test]
    fn sizes_match_the_design() {
        assert_eq!(size_short(2_832_413_696), "2.64 GB");
        assert_eq!(size_short(64_023_511_040), "59.6 GB");
        assert_eq!(size_short(31_037_849_600), "28.9 GB");
        assert_eq!(size_short(15_931_539_456), "14.8 GB");
        assert_eq!(size_short(1_000_204_886_016), "932 GB");
        assert_eq!(size_short(2_000_398_934_016), "1.82 TB");
    }

    #[test]
    fn small_and_boundary_sizes() {
        assert_eq!(size_short(0), "0 B");
        assert_eq!(size_short(1023), "1023 B");
        assert_eq!(size_short(1024), "1.00 KB");
        assert_eq!(size_short(1_258_496), "1.20 MB");
        assert_eq!(size_short(4 * 1024 * 1024), "4.00 MB");
        // Just under 1 GiB rounds up to the next unit instead of "1024 MB".
        assert_eq!(size_short(1024 * 1024 * 1024 - 1), "1.00 GB");
        assert_eq!(size_short(10 * 1024 * 1024 * 1024 - 1), "10.0 GB");
    }

    #[test]
    fn exact_sizes_for_technical_details() {
        assert_eq!(bytes_exact(2_832_413_696), "2,832,413,696 B (2.64 GiB)");
        assert_eq!(bytes_exact(64_023_511_040), "64,023,511,040 B (59.6 GiB)");
        assert_eq!(bytes_exact(512), "512 B");
        assert_eq!(with_thousands(4_194_304), "4,194,304");
        assert_eq!(with_thousands(999), "999");
        assert_eq!(with_thousands(1000), "1,000");
    }

    #[test]
    fn long_names_are_shortened_in_the_middle_at_separators() {
        assert_eq!(
            middle_truncate("ubuntu-24.04.1-live-server-amd64.iso", 26),
            "ubuntu-24.04.1-…-amd64.iso"
        );
        assert_eq!(
            middle_truncate("Win11_25H2_EnglishInternational_x64.iso", 24),
            "Win11_25H2_…_x64.iso"
        );
        assert_eq!(middle_truncate("short.iso", 26), "short.iso");
    }

    #[test]
    fn names_without_separators_are_cut_by_length() {
        let shortened = middle_truncate("abcdefghijklmnopqrstuvwxyz0123456789", 11);
        assert_eq!(shortened, "abcde…56789");
        assert_eq!(shortened.chars().count(), 11);
    }
}
