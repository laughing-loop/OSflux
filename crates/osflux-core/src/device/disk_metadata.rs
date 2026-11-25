//! What Windows reports about each disk, as one tab-separated line per disk
//! from the PowerShell query in `windows.rs`. Parsed here, on every
//! platform, so the rules are tested everywhere.
//!
//! Anything that cannot be read leaves a drive less trusted, never more:
//! a disk with no line at all has no identity and no media type, so the
//! engine blocks it, and a disk whose system-volume check did not run is
//! treated as holding the system volume.

#![cfg_attr(not(windows), allow(dead_code))]

use super::{is_placeholder_serial, DeviceAccess, PhysicalDevice};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiskMetadata {
    pub index: u32,
    pub model: Option<String>,
    pub interface_type: Option<String>,
    pub media_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub pnp_device_id: Option<String>,
    pub serial_number: Option<String>,
    pub contains_system_volume: bool,
}

/// Parses `index, model, interface, media, size, PnP ID, system, serial`.
/// The system field counts as "no" only when it says `False`: an empty
/// field or `unknown`, written when the check failed, counts as "yes".
pub(crate) fn parse_disk_metadata_line(line: &str) -> Option<DiskMetadata> {
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() < 7 {
        return None;
    }

    let index = fields[0].trim().parse::<u32>().ok()?;

    Some(DiskMetadata {
        index,
        model: optional_field(fields[1]),
        interface_type: optional_field(fields[2]),
        media_type: optional_field(fields[3]),
        size_bytes: fields[4].trim().parse::<u64>().ok(),
        pnp_device_id: optional_field(fields[5]),
        contains_system_volume: !fields[6].trim().eq_ignore_ascii_case("false"),
        serial_number: fields
            .get(7)
            .copied()
            .and_then(optional_field)
            .filter(|serial| !is_placeholder_serial(serial)),
    })
}

fn optional_field(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

impl PhysicalDevice {
    /// A drive found by opening it, before (or without) what Windows reports
    /// about it: no model, serial, device ID or media type. On its own such
    /// a drive is never a write target, because it has no identity code and
    /// does not report removable media.
    pub fn found_without_metadata(
        index: u32,
        path: String,
        size_bytes: Option<u64>,
        access: DeviceAccess,
        note: Option<String>,
    ) -> Self {
        Self {
            index,
            path,
            size_bytes,
            model: None,
            interface_type: None,
            media_type: None,
            pnp_device_id: None,
            serial_number: None,
            contains_system_volume: false,
            access,
            note,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_disk_metadata_line;

    fn line(system: &str) -> String {
        format!(
            "2\tSanDisk Ultra\tUSB\tRemovable Media\t64023511040\tUSBSTOR\\DISK&VEN_SANDISK\\1\t{system}\t4C530001"
        )
    }

    #[test]
    fn reads_a_full_line() {
        let metadata = parse_disk_metadata_line(&line("False")).expect("a disk");
        assert_eq!(metadata.index, 2);
        assert_eq!(metadata.model.as_deref(), Some("SanDisk Ultra"));
        assert_eq!(metadata.media_type.as_deref(), Some("Removable Media"));
        assert_eq!(metadata.size_bytes, Some(64_023_511_040));
        assert_eq!(metadata.serial_number.as_deref(), Some("4C530001"));
        assert!(!metadata.contains_system_volume);
    }

    #[test]
    fn only_an_explicit_false_clears_the_system_volume() {
        assert!(
            parse_disk_metadata_line(&line("True"))
                .unwrap()
                .contains_system_volume
        );
        assert!(
            !parse_disk_metadata_line(&line("false"))
                .unwrap()
                .contains_system_volume
        );
        // The check failed, or wrote nothing: treated as the system disk.
        assert!(
            parse_disk_metadata_line(&line("unknown"))
                .unwrap()
                .contains_system_volume
        );
        assert!(
            parse_disk_metadata_line(&line(""))
                .unwrap()
                .contains_system_volume
        );
    }

    #[test]
    fn short_or_broken_lines_are_skipped() {
        assert!(parse_disk_metadata_line("").is_none());
        assert!(parse_disk_metadata_line("2\tSanDisk").is_none());
        assert!(parse_disk_metadata_line("x\ta\tb\tc\td\te\tFalse").is_none());
    }

    #[test]
    fn placeholder_serials_are_not_kept() {
        let text = line("False").replace("4C530001", "0000000000");
        assert_eq!(parse_disk_metadata_line(&text).unwrap().serial_number, None);
    }
}
