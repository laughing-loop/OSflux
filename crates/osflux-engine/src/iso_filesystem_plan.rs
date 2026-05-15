use std::fmt;
use std::path::Path;

use crate::image_family::{detect_image_family, ImageFamily, ImageFamilyError, ImageFamilyReport};
use crate::image_plan::{plan_image, ImageKind, ImagePlan, ImagePlanError, ImagePlanStatus};
use crate::iso::{BootStyle, IsoBootAssetKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoFilesystemPlan {
    pub status: IsoFilesystemPlanStatus,
    pub strategy: IsoFilesystemStrategy,
    pub reason: String,
    pub image_source: String,
    pub image_status: ImagePlanStatus,
    pub detected_family: ImageFamily,
    pub planned_family: ImageFamily,
    pub boot_style: BootStyle,
    pub capabilities: IsoFilesystemCapabilities,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl IsoFilesystemPlan {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"strategy\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"imageStatus\": \"{}\",\n",
                "  \"detectedFamily\": \"{}\",\n",
                "  \"plannedFamily\": \"{}\",\n",
                "  \"bootStyle\": \"{}\",\n",
                "  \"capabilities\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            self.strategy.as_str(),
            escape_json_string(&self.reason),
            escape_json_string(&self.image_source),
            self.image_status.as_str(),
            self.detected_family.as_str(),
            self.planned_family.as_str(),
            self.boot_style.as_str(),
            self.capabilities.to_json(),
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoFilesystemPlanStatus {
    Planned,
    NeedsInspection,
    Blocked,
}

impl IsoFilesystemPlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::NeedsInspection => "needs-inspection",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoFilesystemStrategy {
    WindowsInstallerBootSetup,
    LinuxInstallerBootSetup,
    MultiFamilyBootSetup,
    NeedsDeeperInspection,
    Unsupported,
}

impl IsoFilesystemStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowsInstallerBootSetup => "windows-installer-boot-setup",
            Self::LinuxInstallerBootSetup => "linux-installer-boot-setup",
            Self::MultiFamilyBootSetup => "multi-family-boot-setup",
            Self::NeedsDeeperInspection => "needs-deeper-inspection",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoFilesystemCapabilities {
    pub has_el_torito_catalog: bool,
    pub el_torito_validation_valid: bool,
    pub boot_image_ranges_valid: bool,
    pub directory_scanned: bool,
    pub directory_truncated: bool,
    pub boot_asset_count: usize,
    pub has_boot_manager: bool,
    pub has_uefi_bootloader: bool,
    pub has_installer_image: bool,
    pub has_linux_bootloader: bool,
    pub has_linux_kernel: bool,
    pub has_initrd: bool,
    pub has_live_filesystem: bool,
    pub has_grub_boot_path: bool,
    pub windows_asset_count: usize,
    pub linux_asset_count: usize,
}

impl IsoFilesystemCapabilities {
    pub fn has_valid_boot_catalog(&self) -> bool {
        self.has_el_torito_catalog
            && self.el_torito_validation_valid
            && self.boot_image_ranges_valid
    }

    pub fn has_windows_assets(&self) -> bool {
        self.windows_asset_count > 0
    }

    pub fn has_linux_assets(&self) -> bool {
        self.linux_asset_count > 0
    }

    fn to_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"hasElToritoCatalog\":{},",
                "\"elToritoValidationValid\":{},",
                "\"bootImageRangesValid\":{},",
                "\"directoryScanned\":{},",
                "\"directoryTruncated\":{},",
                "\"bootAssetCount\":{},",
                "\"hasBootManager\":{},",
                "\"hasUefiBootloader\":{},",
                "\"hasInstallerImage\":{},",
                "\"hasLinuxBootloader\":{},",
                "\"hasLinuxKernel\":{},",
                "\"hasInitrd\":{},",
                "\"hasLiveFilesystem\":{},",
                "\"hasGrubBootPath\":{},",
                "\"windowsAssetCount\":{},",
                "\"linuxAssetCount\":{}",
                "}}"
            ),
            self.has_el_torito_catalog,
            self.el_torito_validation_valid,
            self.boot_image_ranges_valid,
            self.directory_scanned,
            self.directory_truncated,
            self.boot_asset_count,
            self.has_boot_manager,
            self.has_uefi_bootloader,
            self.has_installer_image,
            self.has_linux_bootloader,
            self.has_linux_kernel,
            self.has_initrd,
            self.has_live_filesystem,
            self.has_grub_boot_path,
            self.windows_asset_count,
            self.linux_asset_count,
        )
    }
}

#[derive(Debug)]
pub enum IsoFilesystemPlanError {
    Image(ImagePlanError),
    Family(ImageFamilyError),
}

impl fmt::Display for IsoFilesystemPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => write!(formatter, "image planning failed: {error}"),
            Self::Family(error) => write!(formatter, "image family detection failed: {error}"),
        }
    }
}

impl std::error::Error for IsoFilesystemPlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Image(error) => Some(error),
            Self::Family(error) => Some(error),
        }
    }
}

pub fn plan_iso_filesystem(
    path: impl AsRef<Path>,
) -> Result<IsoFilesystemPlan, IsoFilesystemPlanError> {
    let source = path.as_ref();
    let image_plan = plan_image(source).map_err(IsoFilesystemPlanError::Image)?;
    let family_report = detect_image_family(source).map_err(IsoFilesystemPlanError::Family)?;
    Ok(build_iso_filesystem_plan(&image_plan, &family_report))
}

pub fn build_iso_filesystem_plan(
    image_plan: &ImagePlan,
    family_report: &ImageFamilyReport,
) -> IsoFilesystemPlan {
    let capabilities = collect_capabilities(image_plan);
    let planned_family = merge_family(family_report.family, &capabilities);
    let strategy = select_strategy(image_plan, planned_family, &capabilities);
    let status = select_status(image_plan, strategy, &capabilities);
    let mut notes = Vec::new();
    let mut warnings = image_plan.warnings.clone();

    notes.push("ISO filesystem planning is read-only; no target access was attempted.".to_string());
    notes.push(format!(
        "Detected family: {}; planned family: {}.",
        family_report.family.as_str(),
        planned_family.as_str()
    ));

    if capabilities.directory_scanned {
        notes.push(format!(
            "Directory metadata exposed {} boot asset path(s).",
            capabilities.boot_asset_count
        ));
    } else {
        warnings
            .push("ISO directory metadata was not available for filesystem planning.".to_string());
    }

    if capabilities.directory_truncated {
        warnings.push("ISO directory metadata was truncated by safety limits.".to_string());
    }

    if !capabilities.has_valid_boot_catalog() {
        warnings.push(
            "Filesystem plan still needs a valid boot catalog and boot image ranges.".to_string(),
        );
    }

    if strategy == IsoFilesystemStrategy::NeedsDeeperInspection {
        warnings.push("Filesystem plan needs stronger family and boot asset evidence.".to_string());
    }

    if family_report.family != ImageFamily::Unknown && family_report.family != planned_family {
        warnings.push(
            "Raw family markers and ISO directory boot assets point to different families."
                .to_string(),
        );
    }

    IsoFilesystemPlan {
        status,
        strategy,
        reason: status_reason(status, strategy, &capabilities).to_string(),
        image_source: image_plan.source.display().to_string(),
        image_status: image_plan.status,
        detected_family: family_report.family,
        planned_family,
        boot_style: image_plan
            .boot_metadata
            .as_ref()
            .map(|metadata| metadata.boot_style())
            .unwrap_or(BootStyle::None),
        capabilities,
        notes,
        warnings,
    }
}

fn collect_capabilities(image_plan: &ImagePlan) -> IsoFilesystemCapabilities {
    let mut capabilities = IsoFilesystemCapabilities {
        has_el_torito_catalog: false,
        el_torito_validation_valid: false,
        boot_image_ranges_valid: false,
        directory_scanned: false,
        directory_truncated: false,
        boot_asset_count: 0,
        has_boot_manager: false,
        has_uefi_bootloader: false,
        has_installer_image: false,
        has_linux_bootloader: false,
        has_linux_kernel: false,
        has_initrd: false,
        has_live_filesystem: false,
        has_grub_boot_path: false,
        windows_asset_count: 0,
        linux_asset_count: 0,
    };

    let Some(metadata) = image_plan.boot_metadata.as_ref() else {
        return capabilities;
    };

    if let Some(info) = metadata.el_torito.as_ref() {
        capabilities.has_el_torito_catalog = true;
        capabilities.el_torito_validation_valid = info.validation_entry_valid;
        capabilities.boot_image_ranges_valid = info.boot_image_ranges_valid();
    }

    if let Some(directory) = metadata.directory.as_ref() {
        capabilities.directory_scanned = true;
        capabilities.directory_truncated = directory.truncated;
        capabilities.boot_asset_count = directory.boot_assets.len();

        for asset in &directory.boot_assets {
            let path = asset.path.to_ascii_lowercase();
            match asset.kind {
                IsoBootAssetKind::BootManager => {
                    capabilities.has_boot_manager = true;
                    capabilities.windows_asset_count += 1;
                }
                IsoBootAssetKind::UefiBootloader => {
                    capabilities.has_uefi_bootloader = true;
                    if path.starts_with("efi/microsoft/") {
                        capabilities.windows_asset_count += 1;
                    }
                }
                IsoBootAssetKind::InstallerImage => {
                    capabilities.has_installer_image = true;
                    capabilities.windows_asset_count += 1;
                }
                IsoBootAssetKind::LinuxBootloader => {
                    capabilities.has_linux_bootloader = true;
                    capabilities.linux_asset_count += 1;
                }
                IsoBootAssetKind::LinuxKernel => {
                    capabilities.has_linux_kernel = true;
                    capabilities.linux_asset_count += 1;
                }
                IsoBootAssetKind::Initrd => {
                    capabilities.has_initrd = true;
                    capabilities.linux_asset_count += 1;
                }
                IsoBootAssetKind::LiveFilesystem => {
                    capabilities.has_live_filesystem = true;
                    capabilities.linux_asset_count += 1;
                }
                IsoBootAssetKind::GrubBootPath => {
                    capabilities.has_grub_boot_path = true;
                    capabilities.linux_asset_count += 1;
                }
            }
        }
    }

    capabilities
}

fn merge_family(
    detected_family: ImageFamily,
    capabilities: &IsoFilesystemCapabilities,
) -> ImageFamily {
    let has_windows = capabilities.has_windows_assets();
    let has_linux = capabilities.has_linux_assets();

    match (detected_family, has_windows, has_linux) {
        (ImageFamily::WindowsInstaller, _, true)
        | (ImageFamily::LinuxInstaller, true, _)
        | (ImageFamily::MultiFamilyImage, _, _)
        | (ImageFamily::Unknown, true, true) => ImageFamily::MultiFamilyImage,
        (ImageFamily::Unknown, true, false) => ImageFamily::WindowsInstaller,
        (ImageFamily::Unknown, false, true) => ImageFamily::LinuxInstaller,
        (family, _, _) => family,
    }
}

fn select_strategy(
    image_plan: &ImagePlan,
    planned_family: ImageFamily,
    capabilities: &IsoFilesystemCapabilities,
) -> IsoFilesystemStrategy {
    if image_plan.kind != ImageKind::Iso {
        return IsoFilesystemStrategy::Unsupported;
    }

    match planned_family {
        ImageFamily::WindowsInstaller if windows_setup_assets_ready(capabilities) => {
            IsoFilesystemStrategy::WindowsInstallerBootSetup
        }
        ImageFamily::LinuxInstaller if linux_setup_assets_ready(capabilities) => {
            IsoFilesystemStrategy::LinuxInstallerBootSetup
        }
        ImageFamily::MultiFamilyImage
            if capabilities.has_windows_assets() && capabilities.has_linux_assets() =>
        {
            IsoFilesystemStrategy::MultiFamilyBootSetup
        }
        ImageFamily::RawDiskImage => IsoFilesystemStrategy::Unsupported,
        _ => IsoFilesystemStrategy::NeedsDeeperInspection,
    }
}

fn windows_setup_assets_ready(capabilities: &IsoFilesystemCapabilities) -> bool {
    capabilities.has_installer_image
        && (capabilities.has_boot_manager || capabilities.has_uefi_bootloader)
}

fn linux_setup_assets_ready(capabilities: &IsoFilesystemCapabilities) -> bool {
    (capabilities.has_linux_bootloader
        || capabilities.has_grub_boot_path
        || capabilities.has_linux_kernel)
        && (capabilities.has_initrd
            || capabilities.has_live_filesystem
            || capabilities.has_linux_bootloader)
}

fn select_status(
    image_plan: &ImagePlan,
    strategy: IsoFilesystemStrategy,
    capabilities: &IsoFilesystemCapabilities,
) -> IsoFilesystemPlanStatus {
    if image_plan.kind != ImageKind::Iso
        || image_plan.status == ImagePlanStatus::Blocked
        || strategy == IsoFilesystemStrategy::Unsupported
    {
        return IsoFilesystemPlanStatus::Blocked;
    }

    if image_plan.status == ImagePlanStatus::NeedsInspection
        || strategy == IsoFilesystemStrategy::NeedsDeeperInspection
        || !capabilities.has_valid_boot_catalog()
        || capabilities.directory_truncated
    {
        return IsoFilesystemPlanStatus::NeedsInspection;
    }

    IsoFilesystemPlanStatus::Planned
}

fn status_reason(
    status: IsoFilesystemPlanStatus,
    strategy: IsoFilesystemStrategy,
    capabilities: &IsoFilesystemCapabilities,
) -> &'static str {
    match status {
        IsoFilesystemPlanStatus::Blocked => "ISO filesystem planning is blocked for this image",
        IsoFilesystemPlanStatus::NeedsInspection if !capabilities.has_valid_boot_catalog() => {
            "boot catalog metadata still needs deeper inspection"
        }
        IsoFilesystemPlanStatus::NeedsInspection
            if strategy == IsoFilesystemStrategy::NeedsDeeperInspection =>
        {
            "filesystem plan needs stronger boot asset evidence"
        }
        IsoFilesystemPlanStatus::NeedsInspection => "filesystem plan needs deeper inspection",
        IsoFilesystemPlanStatus::Planned => {
            "filesystem-aware plan is ready for the next safety phase"
        }
    }
}

fn string_array_json(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("\"{}\"", escape_json_string(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn escape_json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                write!(escaped, "\\u{:04x}", character as u32)
                    .expect("writing JSON escape should not fail");
            }
            character => escaped.push(character),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::{build_iso_filesystem_plan, IsoFilesystemPlanStatus, IsoFilesystemStrategy};
    use crate::image_family::{FamilyEvidence, ImageFamily, ImageFamilyReport, ImageFamilyStatus};
    use crate::image_plan::{
        DetectionMethod, ImageKind, ImagePlan, ImagePlanStatus, ImageSignature, ImageWriteStrategy,
        WriteMode,
    };
    use crate::iso::{
        BootCatalogPlatform, BootEmulation, BootEntrySource, BootImageRange, BootImageRangeStatus,
        BootStyle, ElToritoBootEntry, ElToritoBootInfo, IsoBootAsset, IsoBootAssetKind,
        IsoBootMetadata, IsoDirectoryMetadata,
    };

    #[test]
    fn plans_windows_installer_setup_from_family_and_assets() {
        let image = iso_image_with_assets(vec![
            asset(IsoBootAssetKind::BootManager, "BOOTMGR"),
            asset(IsoBootAssetKind::InstallerImage, "SOURCES/INSTALL.WIM"),
            asset(IsoBootAssetKind::UefiBootloader, "EFI/BOOT/BOOTX64.EFI"),
        ]);
        let family = family_report(ImageFamily::WindowsInstaller);

        let plan = build_iso_filesystem_plan(&image, &family);

        assert_eq!(plan.status, IsoFilesystemPlanStatus::Planned);
        assert_eq!(
            plan.strategy,
            IsoFilesystemStrategy::WindowsInstallerBootSetup
        );
        assert_eq!(plan.planned_family, ImageFamily::WindowsInstaller);
        assert!(plan.capabilities.has_installer_image);
    }

    #[test]
    fn infers_linux_plan_from_directory_assets_when_marker_family_is_unknown() {
        let image = iso_image_with_assets(vec![
            asset(IsoBootAssetKind::LinuxKernel, "CASPER/VMLINUZ"),
            asset(IsoBootAssetKind::Initrd, "CASPER/INITRD"),
            asset(
                IsoBootAssetKind::LiveFilesystem,
                "CASPER/FILESYSTEM.SQUASHFS",
            ),
            asset(IsoBootAssetKind::GrubBootPath, "BOOT/GRUB/GRUB.CFG"),
        ]);
        let family = family_report(ImageFamily::Unknown);

        let plan = build_iso_filesystem_plan(&image, &family);

        assert_eq!(plan.status, IsoFilesystemPlanStatus::Planned);
        assert_eq!(
            plan.strategy,
            IsoFilesystemStrategy::LinuxInstallerBootSetup
        );
        assert_eq!(plan.planned_family, ImageFamily::LinuxInstaller);
        assert!(plan.capabilities.has_linux_kernel);
    }

    #[test]
    fn blocks_non_iso_images() {
        let mut image = iso_image_with_assets(Vec::new());
        image.kind = ImageKind::RawDiskImage;
        let family = family_report(ImageFamily::RawDiskImage);

        let plan = build_iso_filesystem_plan(&image, &family);

        assert_eq!(plan.status, IsoFilesystemPlanStatus::Blocked);
        assert_eq!(plan.strategy, IsoFilesystemStrategy::Unsupported);
    }

    #[test]
    fn serializes_plan_as_json() {
        let image = iso_image_with_assets(vec![
            asset(IsoBootAssetKind::BootManager, "BOOTMGR"),
            asset(IsoBootAssetKind::InstallerImage, "SOURCES/INSTALL.WIM"),
        ]);
        let family = family_report(ImageFamily::WindowsInstaller);

        let json = build_iso_filesystem_plan(&image, &family).to_json_pretty();

        assert!(json.contains("\"status\": \"planned\""));
        assert!(json.contains("\"strategy\": \"windows-installer-boot-setup\""));
        assert!(json.contains("\"plannedFamily\": \"windows-installer\""));
        assert!(json.contains("\"hasInstallerImage\":true"));
    }

    fn iso_image_with_assets(assets: Vec<IsoBootAsset>) -> ImagePlan {
        ImagePlan {
            source: "C:\\images\\sample.iso".into(),
            file_name: "sample.iso".to_string(),
            size_bytes: 4096,
            status: ImagePlanStatus::Ready,
            status_reason: "image plan is trusted for the next safety phase".to_string(),
            kind: ImageKind::Iso,
            detection_method: DetectionMethod::Signature,
            signatures: vec![ImageSignature::Iso9660],
            boot_metadata: Some(IsoBootMetadata {
                el_torito: Some(valid_el_torito()),
                directory: Some(IsoDirectoryMetadata {
                    root_extent_lba: 20,
                    root_data_length: 2048,
                    scanned_directories: 1,
                    entries: Vec::new(),
                    boot_assets: assets,
                    truncated: false,
                }),
            }),
            write_mode: WriteMode::FileSystem,
            write_strategy: ImageWriteStrategy::IsoFileSystemBootSetup,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn valid_el_torito() -> ElToritoBootInfo {
        ElToritoBootInfo {
            catalog_lba: 20,
            platform: BootCatalogPlatform::Efi,
            validation_entry_valid: true,
            default_entry_bootable: true,
            emulation: BootEmulation::NoEmulation,
            load_segment: 0,
            system_type: 0,
            sector_count: 4,
            load_rba: 24,
            boot_style: BootStyle::Uefi,
            boot_entries: vec![ElToritoBootEntry {
                source: BootEntrySource::Default,
                platform: BootCatalogPlatform::Efi,
                bootable: true,
                emulation: BootEmulation::NoEmulation,
                load_segment: 0,
                system_type: 0,
                sector_count: 4,
                load_rba: 24,
                image_range: BootImageRange {
                    start_offset: Some(49152),
                    required_bytes: Some(2048),
                    end_offset: Some(51200),
                    status: BootImageRangeStatus::Valid,
                },
            }],
        }
    }

    fn family_report(family: ImageFamily) -> ImageFamilyReport {
        ImageFamilyReport {
            status: if family == ImageFamily::Unknown {
                ImageFamilyStatus::NeedsInspection
            } else {
                ImageFamilyStatus::Detected
            },
            family,
            reason: "test family".to_string(),
            image_source: "C:\\images\\sample.iso".to_string(),
            image_kind: ImageKind::Iso,
            image_status: ImagePlanStatus::Ready,
            detection_method: DetectionMethod::Signature,
            scanned_bytes: 4096,
            scan_limit_bytes: 4096,
            evidence: vec![FamilyEvidence {
                family,
                marker: family.as_str().to_string(),
                description: "test evidence".to_string(),
            }],
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn asset(kind: IsoBootAssetKind, path: &str) -> IsoBootAsset {
        IsoBootAsset {
            kind,
            path: path.to_string(),
        }
    }
}
