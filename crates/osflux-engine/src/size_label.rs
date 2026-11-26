//! Sizes written for people, the way Windows Explorer writes them.

/// A size in binary units labelled the way Windows Explorer labels them, so
/// a "64GB" stick reads "59.6 GB" in both: three significant digits. Studio
/// shows sizes this way, and the confirmation that names a drive uses it, so
/// both say the same thing.
pub fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    // Round first, so 1023.96 MB is shown as 1.00 GB, not 1024 MB.
    let decimals = decimals_for(value);
    let rounded = round_to(value, decimals);
    if rounded >= 1024.0 && unit < UNITS.len() - 1 {
        value = rounded / 1024.0;
        unit += 1;
    }
    let decimals = decimals_for(value);
    format!("{value:.decimals$} {}", UNITS[unit])
}

fn decimals_for(value: f64) -> usize {
    if round_to(value, 2) < 10.0 {
        2
    } else if round_to(value, 1) < 100.0 {
        1
    } else {
        0
    }
}

fn round_to(value: f64, decimals: usize) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::size_label;

    #[test]
    fn labels_sizes_as_explorer_does() {
        assert_eq!(size_label(15_931_539_456), "14.8 GB");
        assert_eq!(size_label(64_023_511_040), "59.6 GB");
        assert_eq!(size_label(1023), "1023 B");
        assert_eq!(size_label(1024 * 1024 * 1024 - 1), "1.00 GB");
    }
}
