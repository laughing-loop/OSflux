//! What Windows reports about each disk, read straight from the storage
//! stack in `windows.rs`: the `STORAGE_DEVICE_DESCRIPTOR` that
//! `IOCTL_STORAGE_QUERY_PROPERTY` returns (bus, removable media, vendor,
//! product, serial), the disk's device-instance ID and friendly name, and
//! whether the system volume sits on it. The bytes are parsed and named
//! here, on every platform, so the rules are tested everywhere.
//!
//! Anything that cannot be read leaves a drive less trusted, never more:
//! a disk with no descriptor has no media type, so the engine blocks it,
//! and when the system volume cannot be located every disk is treated as
//! holding it.

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

/// The parts of a `STORAGE_DEVICE_DESCRIPTOR` OSflux uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageDescriptor {
    /// `STORAGE_BUS_TYPE`.
    pub bus_type: u32,
    /// The device's removable-media bit, as SCSI's RMB reports it.
    pub removable: bool,
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
}

const DESCRIPTOR_FIXED_BYTES: usize = 32;
pub(crate) const BUS_USB: u32 = 0x07;
pub(crate) const BUS_1394: u32 = 0x04;

/// Parses a `STORAGE_DEVICE_DESCRIPTOR`: `Version` and `Size` (`u32`s),
/// four one-byte fields with `RemovableMedia` at byte 10, the vendor,
/// product, revision and serial offsets (`u32`s at 12, 16, 20 and 24) and
/// `BusType` at 28. Each offset points to a NUL-terminated ASCII string in
/// the same buffer; zero means there is none.
pub(crate) fn parse_storage_descriptor(bytes: &[u8]) -> Option<StorageDescriptor> {
    if bytes.len() < DESCRIPTOR_FIXED_BYTES {
        return None;
    }
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    // Only the bytes the descriptor says it filled.
    let size = (word(4) as usize).clamp(DESCRIPTOR_FIXED_BYTES, bytes.len());
    let bytes = &bytes[..size];
    Some(StorageDescriptor {
        bus_type: word(28),
        removable: bytes[10] != 0,
        vendor: descriptor_string(bytes, word(12)),
        product: descriptor_string(bytes, word(16)),
        serial: descriptor_string(bytes, word(24)),
    })
}

fn descriptor_string(bytes: &[u8], offset: u32) -> Option<String> {
    let offset = offset as usize;
    // Zero means none; some drivers write an offset past the data.
    if offset < DESCRIPTOR_FIXED_BYTES || offset >= bytes.len() {
        return None;
    }
    let text = &bytes[offset..];
    let end = text
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(text.len());
    let text: String = text[..end]
        .iter()
        .map(|&byte| {
            if byte.is_ascii_graphic() || byte == b' ' {
                byte as char
            } else {
                ' '
            }
        })
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// `STORAGE_BUS_TYPE` in the words Windows' own tools use.
pub(crate) fn bus_name(bus_type: u32) -> Option<&'static str> {
    Some(match bus_type {
        0x01 => "SCSI",
        0x02 => "ATAPI",
        0x03 => "ATA",
        BUS_1394 => "1394",
        0x05 => "SSA",
        0x06 => "Fibre Channel",
        BUS_USB => "USB",
        0x08 => "RAID",
        0x09 => "iSCSI",
        0x0A => "SAS",
        0x0B => "SATA",
        0x0C => "SD",
        0x0D => "MMC",
        0x0E => "Virtual",
        0x0F => "File-backed virtual",
        0x10 => "Storage Spaces",
        0x11 => "NVMe",
        0x12 => "SCM",
        0x13 => "UFS",
        _ => return None,
    })
}

/// The media type in Windows' words, which the engine's rules read: a
/// drive that reports removable media is `Removable Media`; a fixed disk on
/// USB or FireWire is `External hard disk media`, which is never a target;
/// anything else is `Fixed hard disk media`.
pub(crate) fn media_type_name(descriptor: &StorageDescriptor) -> &'static str {
    if descriptor.removable {
        "Removable Media"
    } else if matches!(descriptor.bus_type, BUS_USB | BUS_1394) {
        "External hard disk media"
    } else {
        "Fixed hard disk media"
    }
}

/// What the storage stack and Plug and Play said about one disk, put
/// together. The friendly name, which Device Manager shows, is preferred to
/// the vendor and product strings; a serial of only zeros is dropped.
pub(crate) fn disk_metadata(
    index: u32,
    descriptor: Option<&StorageDescriptor>,
    friendly_name: Option<String>,
    instance_id: Option<String>,
    size_bytes: Option<u64>,
    contains_system_volume: bool,
) -> DiskMetadata {
    let model = friendly_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            let descriptor = descriptor?;
            let parts: Vec<&str> = [descriptor.vendor.as_deref(), descriptor.product.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            (!parts.is_empty()).then(|| parts.join(" "))
        });
    DiskMetadata {
        index,
        model,
        interface_type: descriptor
            .and_then(|descriptor| bus_name(descriptor.bus_type))
            .map(str::to_string),
        media_type: descriptor.map(|descriptor| media_type_name(descriptor).to_string()),
        size_bytes,
        pnp_device_id: instance_id.filter(|id| !id.trim().is_empty()),
        serial_number: descriptor
            .and_then(|descriptor| descriptor.serial.clone())
            .filter(|serial| !is_placeholder_serial(serial)),
        contains_system_volume,
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
    use super::{
        bus_name, disk_metadata, media_type_name, parse_storage_descriptor, StorageDescriptor,
        BUS_USB,
    };

    /// A descriptor as a USB stick returns it.
    fn descriptor_bytes(removable: bool, bus: u32, strings: [&str; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; 40];
        bytes[10] = removable as u8;
        bytes[28..32].copy_from_slice(&bus.to_le_bytes());
        for (field, text) in [12usize, 16, 20, 24].into_iter().zip(strings) {
            if text.is_empty() {
                continue;
            }
            let offset = bytes.len() as u32;
            bytes[field..field + 4].copy_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
        }
        let size = bytes.len() as u32;
        bytes[0..4].copy_from_slice(&1u32.to_le_bytes());
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes
    }

    #[test]
    fn reads_a_usb_sticks_descriptor() {
        let bytes = descriptor_bytes(
            true,
            BUS_USB,
            ["SanDisk ", "Ultra           ", "1.00", "4C530001"],
        );
        let descriptor = parse_storage_descriptor(&bytes).expect("a descriptor");

        assert!(descriptor.removable);
        assert_eq!(descriptor.bus_type, BUS_USB);
        assert_eq!(descriptor.vendor.as_deref(), Some("SanDisk"));
        assert_eq!(descriptor.product.as_deref(), Some("Ultra"));
        assert_eq!(descriptor.serial.as_deref(), Some("4C530001"));
    }

    #[test]
    fn missing_or_bad_strings_are_left_out() {
        let mut bytes = descriptor_bytes(false, 0x11, ["", "Samsung SSD 980", "", ""]);
        // A serial offset past the end of the data.
        bytes[24..28].copy_from_slice(&5000u32.to_le_bytes());
        let descriptor = parse_storage_descriptor(&bytes).unwrap();

        assert_eq!(descriptor.vendor, None);
        assert_eq!(descriptor.product.as_deref(), Some("Samsung SSD 980"));
        assert_eq!(descriptor.serial, None);
        assert!(parse_storage_descriptor(&bytes[..20]).is_none());
    }

    #[test]
    fn names_buses_and_media_the_way_the_rules_read_them() {
        let with = |removable, bus_type| StorageDescriptor {
            bus_type,
            removable,
            vendor: None,
            product: None,
            serial: None,
        };
        assert_eq!(bus_name(BUS_USB), Some("USB"));
        assert_eq!(bus_name(0x11), Some("NVMe"));
        assert_eq!(bus_name(0x0B), Some("SATA"));
        assert_eq!(bus_name(0x7F), None);

        assert_eq!(media_type_name(&with(true, BUS_USB)), "Removable Media");
        assert_eq!(media_type_name(&with(true, 0x0C)), "Removable Media");
        // A USB SSD or hard disk: never a target.
        assert_eq!(
            media_type_name(&with(false, BUS_USB)),
            "External hard disk media"
        );
        assert_eq!(media_type_name(&with(false, 0x11)), "Fixed hard disk media");
        // Internal eMMC, the laptop's own storage.
        assert_eq!(media_type_name(&with(false, 0x0D)), "Fixed hard disk media");
    }

    #[test]
    fn puts_a_disks_details_together() {
        let bytes = descriptor_bytes(true, BUS_USB, ["SanDisk", "Ultra", "1.00", "0000000000"]);
        let descriptor = parse_storage_descriptor(&bytes).unwrap();

        let metadata = disk_metadata(
            2,
            Some(&descriptor),
            Some("SanDisk Ultra USB Device".to_string()),
            Some(r"USBSTOR\DISK&VEN_SANDISK&PROD_ULTRA\4C530001&0".to_string()),
            Some(64_023_511_040),
            false,
        );
        assert_eq!(metadata.model.as_deref(), Some("SanDisk Ultra USB Device"));
        assert_eq!(metadata.interface_type.as_deref(), Some("USB"));
        assert_eq!(metadata.media_type.as_deref(), Some("Removable Media"));
        assert_eq!(metadata.serial_number, None, "a serial of zeros is dropped");
        assert!(metadata.pnp_device_id.is_some());

        let no_friendly_name = disk_metadata(2, Some(&descriptor), None, None, None, false);
        assert_eq!(no_friendly_name.model.as_deref(), Some("SanDisk Ultra"));
    }

    #[test]
    fn a_disk_with_no_descriptor_has_no_media_type() {
        // Then it is not removable as far as the engine knows, so it is
        // never a target.
        let metadata = disk_metadata(3, None, None, None, Some(1 << 30), false);
        assert_eq!(metadata.media_type, None);
        assert_eq!(metadata.interface_type, None);
        assert_eq!(metadata.model, None);
    }
}
