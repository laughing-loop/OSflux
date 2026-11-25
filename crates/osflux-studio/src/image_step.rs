//! Step 1 once an image is chosen (frames 02–04): what the engine found in
//! the image and how it will write it, in the words the design uses. The
//! verdict is the engine's; this module only words it.

use std::path::Path;

use osflux_engine::{
    build_iso_filesystem_plan, detect_plan_family, plan_image, BootStyle, DetectionMethod,
    ImageFamily, ImageKind, ImagePlan, ImagePlanStatus, ImageSignature, ImageWriteStrategy,
    IsoFilesystemStrategy,
};

use crate::format::size_short;
use crate::steps::Flow;
use crate::{HintIcon, Status};

/// The engine's findings for one image file.
#[derive(Debug, Clone)]
pub struct ImageAnalysis {
    pub plan: ImagePlan,
    /// The family the engine plans the write for: what its family scan
    /// found, refined by the boot files in the ISO's directory.
    pub family: ImageFamily,
    /// For ISOs, the family-specific setup the engine would use.
    pub filesystem_strategy: Option<IsoFilesystemStrategy>,
}

/// Reads the image with the engine. Nothing is written.
pub fn analyze_image(path: &Path) -> Result<ImageAnalysis, String> {
    let plan = plan_image(path).map_err(|error| error.to_string())?;
    let report = detect_plan_family(&plan).map_err(|error| error.to_string())?;
    let (family, filesystem_strategy) = if plan.kind == ImageKind::Iso {
        let filesystem = build_iso_filesystem_plan(&plan, &report);
        (filesystem.planned_family, Some(filesystem.strategy))
    } else {
        (report.family, None)
    };
    Ok(ImageAnalysis {
        plan,
        family,
        filesystem_strategy,
    })
}

impl ImageAnalysis {
    /// Windows installers get the Windows options step.
    pub fn flow(&self) -> Flow {
        if self.plan.status == ImagePlanStatus::Ready
            && self.plan.write_strategy == ImageWriteStrategy::IsoFileSystemBootSetup
            && self.family == ImageFamily::WindowsInstaller
        {
            Flow::Windows
        } else {
            Flow::Other
        }
    }

    pub fn can_continue(&self) -> bool {
        self.plan.status == ImagePlanStatus::Ready
    }
}

/// One line of the image plan.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRow {
    pub key: &'static str,
    pub value: String,
    pub mono: bool,
    /// A mark before the value, for a warning or a block.
    pub value_mark: Status,
    pub detail: String,
    /// A mark before the detail, for a check.
    pub detail_mark: Status,
    pub secondary: bool,
}

impl PlanRow {
    fn new(key: &'static str, value: impl Into<String>) -> Self {
        Self {
            key,
            value: value.into(),
            mono: false,
            value_mark: Status::None,
            detail: String::new(),
            detail_mark: Status::None,
            secondary: false,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    fn checked(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self.detail_mark = Status::Ready;
        self
    }

    fn marked(mut self, mark: Status) -> Self {
        self.value_mark = mark;
        self
    }

    fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    fn secondary(mut self) -> Self {
        self.secondary = true;
        self
    }
}

/// A note under the plan. `Status::None` shows the information icon.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub status: Status,
    pub text: String,
}

fn note(status: Status, text: impl Into<String>) -> Note {
    Note {
        status,
        text: text.into(),
    }
}

/// Everything the Image screen shows for a chosen image.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageView {
    pub file_name: String,
    pub path: String,
    pub size: String,
    pub status: Status,
    pub status_label: &'static str,
    pub title: &'static str,
    pub description: String,
    pub hint: &'static str,
    pub hint_icon: HintIcon,
    pub rows: Vec<PlanRow>,
    pub notes: Vec<Note>,
}

pub fn image_view(analysis: &ImageAnalysis) -> ImageView {
    let plan = &analysis.plan;
    let (status, status_label) = plan_status(plan.status);
    let (title, description, hint, hint_icon) = match plan.status {
        ImagePlanStatus::Ready => (
            "Image checked",
            "OSflux looked inside the image. This is what the engine found and how it will write it."
                .to_string(),
            "Next, choose the drive to write to.",
            HintIcon::None,
        ),
        ImagePlanStatus::NeedsInspection => (
            "Image needs a closer look",
            "The engine can't trust this image's boot information yet, so it can't be written. Choose another image."
                .to_string(),
            "This image can't be written yet.",
            HintIcon::Info,
        ),
        ImagePlanStatus::Blocked => (
            "Image can't be written",
            format!(
                "{} Choose another image.",
                blocked_image_reason(&plan.status_reason)
            ),
            "This image can't be written.",
            HintIcon::Info,
        ),
    };

    let mut rows = vec![
        PlanRow::new("Image family", family_label(analysis.family)),
        boot_style_row(analysis),
        format_row(plan),
        write_method_row(analysis),
    ];
    if plan.kind == ImageKind::Iso {
        rows.push(boot_files_row(plan));
    }

    ImageView {
        file_name: plan.file_name.clone(),
        path: plan.source.display().to_string(),
        size: size_short(plan.size_bytes),
        status,
        status_label,
        title,
        description,
        hint,
        hint_icon,
        rows,
        notes: notes(analysis),
    }
}

pub fn plan_status(status: ImagePlanStatus) -> (Status, &'static str) {
    match status {
        ImagePlanStatus::Ready => (Status::Ready, "Ready"),
        ImagePlanStatus::NeedsInspection => (Status::NeedsInspection, "Needs inspection"),
        ImagePlanStatus::Blocked => (Status::Blocked, "Blocked"),
    }
}

pub fn family_label(family: ImageFamily) -> &'static str {
    match family {
        ImageFamily::WindowsInstaller => "Windows installer",
        ImageFamily::LinuxInstaller => "Linux installer",
        ImageFamily::MultiFamilyImage => "Multi-family image",
        ImageFamily::RawDiskImage => "Raw disk image",
        ImageFamily::Unknown => "Unknown",
    }
}

/// The engine's reasons for blocking an image, in plain words.
fn blocked_image_reason(reason: &str) -> &'static str {
    match reason {
        "image file is empty" => "The file is empty.",
        "image type is unknown" => "OSflux doesn't recognise this file as an ISO or a disk image.",
        _ => "The engine blocked this image.",
    }
}

/// Whether the ISO's El Torito boot catalog was read and every boot image
/// it points to lies inside the file.
pub fn boot_catalog_valid(plan: &ImagePlan) -> bool {
    plan.boot_metadata
        .as_ref()
        .and_then(|metadata| metadata.el_torito.as_ref())
        .is_some_and(|info| info.validation_entry_valid && info.boot_image_ranges_valid())
}

fn boot_style(plan: &ImagePlan) -> BootStyle {
    plan.boot_metadata
        .as_ref()
        .map(|metadata| metadata.boot_style())
        .unwrap_or(BootStyle::None)
}

/// How the written drive will start: for a Windows installer, the FAT32
/// stick starts in UEFI mode whatever the ISO's catalog lists.
fn boot_style_row(analysis: &ImageAnalysis) -> PlanRow {
    let plan = &analysis.plan;
    let valid = boot_catalog_valid(plan);
    let with_catalog = |row: PlanRow| {
        if valid {
            row.checked("boot catalog valid")
        } else {
            row
        }
    };
    if plan.status == ImagePlanStatus::NeedsInspection
        || plan.write_strategy == ImageWriteStrategy::IsoNeedsDeeperInspection
    {
        return PlanRow::new("Boot style", "Boot catalog missing or invalid")
            .marked(Status::NeedsInspection);
    }
    match plan.write_strategy {
        ImageWriteStrategy::IsoFileSystemBootSetup => {
            with_catalog(PlanRow::new("Boot style", "UEFI"))
        }
        ImageWriteStrategy::HybridIsoRawWrite => {
            let value = match boot_style(plan) {
                BootStyle::Hybrid => "BIOS + UEFI hybrid",
                BootStyle::Uefi => "UEFI",
                BootStyle::Bios => "BIOS",
                BootStyle::None | BootStyle::Unknown => "Set by the image's partition table",
            };
            with_catalog(PlanRow::new("Boot style", value))
        }
        ImageWriteStrategy::RawBlockWrite => {
            PlanRow::new("Boot style", "Set by the disk image").detail("copied as it is")
        }
        ImageWriteStrategy::IsoNeedsDeeperInspection | ImageWriteStrategy::Unknown => {
            PlanRow::new("Boot style", "Not found").marked(Status::Blocked)
        }
    }
}

fn format_row(plan: &ImagePlan) -> PlanRow {
    let has = |signature| plan.signatures.contains(&signature);
    if plan.detection_method == DetectionMethod::Extension {
        let value = match plan.kind {
            ImageKind::Iso => "ISO image",
            ImageKind::RawDiskImage => "Raw disk image",
            ImageKind::Unknown => "Unknown",
        };
        return PlanRow::new("Format detected", value)
            .marked(Status::NeedsInspection)
            .detail("from the file name only");
    }
    match plan.kind {
        ImageKind::Iso => {
            let udf = has(ImageSignature::Udf);
            let value = if udf { "UDF" } else { "ISO 9660" };
            let mut details = Vec::new();
            if udf && has(ImageSignature::Iso9660) {
                details.push("with ISO 9660 bridge");
            } else {
                details.push("checked by file signature");
            }
            if !plan.hybrid && plan.status != ImagePlanStatus::Ready {
                details.push("not hybrid");
            }
            PlanRow::new("Format detected", value).detail(details.join(" · "))
        }
        ImageKind::RawDiskImage => {
            let value = if has(ImageSignature::Gpt) {
                "GPT disk image"
            } else {
                "MBR disk image"
            };
            PlanRow::new("Format detected", value).detail("checked by file signature")
        }
        ImageKind::Unknown => {
            PlanRow::new("Format detected", "Not recognised").marked(Status::Blocked)
        }
    }
}

fn write_method_row(analysis: &ImageAnalysis) -> PlanRow {
    let plan = &analysis.plan;
    if plan.status == ImagePlanStatus::NeedsInspection {
        return PlanRow::new("Write method", "Needs deeper inspection")
            .marked(Status::NeedsInspection);
    }
    match plan.write_strategy {
        ImageWriteStrategy::IsoFileSystemBootSetup => {
            let value = if analysis.family == ImageFamily::WindowsInstaller {
                "Windows installer: files copied to a FAT32 stick (UEFI)"
            } else {
                "Files copied to a FAT32 stick (UEFI)"
            };
            PlanRow::new("Write method", value)
        }
        ImageWriteStrategy::HybridIsoRawWrite => PlanRow::new(
            "Write method",
            "Hybrid ISO: copied to the drive byte for byte",
        ),
        ImageWriteStrategy::RawBlockWrite => PlanRow::new(
            "Write method",
            "Raw disk image: copied to the drive byte for byte",
        ),
        ImageWriteStrategy::IsoNeedsDeeperInspection => {
            PlanRow::new("Write method", "Needs deeper inspection").marked(Status::NeedsInspection)
        }
        ImageWriteStrategy::Unknown => {
            PlanRow::new("Write method", "None: the engine can't write this image")
                .marked(Status::Blocked)
        }
    }
}

/// How many boot files the plan names before "+N more". The rest are in
/// the safety check's technical details.
pub const BOOT_FILES_SHOWN: usize = 3;

/// The boot files the engine recognised in the ISO, as ISO paths. A folder
/// it recognised, such as /efi/microsoft/boot, is left out when a file
/// inside it is listed too.
pub fn boot_file_paths(plan: &ImagePlan) -> Vec<String> {
    let Some(directory) = plan
        .boot_metadata
        .as_ref()
        .and_then(|metadata| metadata.directory.as_ref())
    else {
        return Vec::new();
    };
    let paths: Vec<String> = directory
        .boot_assets
        .iter()
        .map(|asset| display_iso_path(&asset.path))
        .collect();
    paths
        .iter()
        .filter(|path| {
            let folder = format!("{path}/");
            !paths.iter().any(|other| other.starts_with(&folder))
        })
        .cloned()
        .collect()
}

/// The first three boot files and "+5 more", so the row stays on one line.
fn boot_files_row(plan: &ImagePlan) -> PlanRow {
    let paths = boot_file_paths(plan);
    if paths.is_empty() {
        return PlanRow::new("Boot files found", "None the engine recognises").secondary();
    }
    let mut value = paths
        .iter()
        .take(BOOT_FILES_SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");
    if paths.len() > BOOT_FILES_SHOWN {
        value.push_str(&format!(" · +{} more", paths.len() - BOOT_FILES_SHOWN));
    }
    PlanRow::new("Boot files found", value).mono()
}

/// `SOURCES/INSTALL.WIM;1` as `/sources/install.wim`. Plain ISO 9660 names
/// are upper case only and carry a version suffix; like the usual way of
/// mounting an ISO, they are shown in lower case without it. Names that
/// carry their own case are kept as they are.
pub fn display_iso_path(path: &str) -> String {
    let trimmed = path.trim_start_matches('/');
    let trimmed = trimmed.strip_suffix(";1").unwrap_or(trimmed);
    let shown = if trimmed.chars().any(|c| c.is_ascii_lowercase()) {
        trimmed.to_string()
    } else {
        trimmed.to_ascii_lowercase()
    };
    format!("/{shown}")
}

fn notes(analysis: &ImageAnalysis) -> Vec<Note> {
    let plan = &analysis.plan;
    match plan.status {
        ImagePlanStatus::Blocked => vec![note(
            Status::Blocked,
            format!(
                "{} OSflux won't write it.",
                blocked_image_reason(&plan.status_reason)
            ),
        )],
        ImagePlanStatus::NeedsInspection => {
            let first = if plan.detection_method == DetectionMethod::Extension {
                "OSflux recognised this image only by its file name, not by what is inside it."
            } else {
                "The image's boot catalog (El Torito) is missing or incomplete, so OSflux can't set up booting from it."
            };
            vec![
                note(Status::NeedsInspection, first),
                note(
                    Status::None,
                    "OSflux won't write this image until its boot information is complete and valid.",
                ),
            ]
        }
        ImagePlanStatus::Ready => ready_notes(analysis),
    }
}

fn ready_notes(analysis: &ImageAnalysis) -> Vec<Note> {
    let plan = &analysis.plan;
    let mut notes = Vec::new();
    match plan.write_strategy {
        ImageWriteStrategy::IsoFileSystemBootSetup => {
            match &plan.iso_files {
                Some(Ok(files)) => {
                    for split in &files.split_files {
                        let name = split.path.rsplit('/').next().unwrap_or(&split.path);
                        let setup = if analysis.family == ImageFamily::WindowsInstaller {
                            " Windows Setup reads them as normal."
                        } else {
                            ""
                        };
                        notes.push(note(
                            Status::None,
                            format!(
                                "{name} ({}) will be split into {} parts to fit FAT32.{setup}",
                                size_short(split.size_bytes),
                                split.parts.len()
                            ),
                        ));
                    }
                }
                Some(Err(reason)) => notes.push(note(
                    Status::NeedsInspection,
                    format!("OSflux couldn't read the files inside the image: {reason}."),
                )),
                None => {}
            }
            notes.push(note(
                Status::NeedsInspection,
                "Boots on UEFI PCs only. Legacy BIOS mode is not supported.",
            ));
        }
        ImageWriteStrategy::HybridIsoRawWrite => {
            match boot_style(plan) {
                BootStyle::Hybrid => notes.push(note(
                    Status::Ready,
                    "Starts on both older (BIOS) and newer (UEFI) PCs.",
                )),
                BootStyle::Uefi => notes.push(note(
                    Status::NeedsInspection,
                    "Boots on UEFI PCs only. Legacy BIOS mode is not supported.",
                )),
                BootStyle::Bios => notes.push(note(
                    Status::NeedsInspection,
                    "Boots on older (BIOS) PCs only. Newer PCs may need legacy boot turned on.",
                )),
                BootStyle::None | BootStyle::Unknown => {}
            }
            notes.push(note(
                Status::None,
                "This ISO is hybrid: it already holds a partition table, so OSflux copies it byte for byte.",
            ));
        }
        ImageWriteStrategy::RawBlockWrite => notes.push(note(
            Status::None,
            "Raw disk images are copied to the drive byte for byte, exactly as they are.",
        )),
        ImageWriteStrategy::IsoNeedsDeeperInspection | ImageWriteStrategy::Unknown => {}
    }
    notes
}

/// What the Image screen says when the file could not be read at all.
pub fn read_error_text(file_name: &str, error: &str) -> String {
    format!("OSflux couldn't read {file_name}: {error}.")
}

#[cfg(test)]
mod tests {
    use super::{display_iso_path, image_view, ImageAnalysis};
    use crate::steps::Flow;
    use crate::{HintIcon, Status};
    use osflux_engine::{
        BootCatalogPlatform, BootEmulation, BootStyle, DetectionMethod, ElToritoBootInfo,
        ImageFamily, ImageKind, ImagePlan, ImagePlanStatus, ImageSignature, ImageWriteStrategy,
        IsoBootAsset, IsoBootAssetKind, IsoBootMetadata, IsoDirectoryMetadata, IsoFilesSummary,
        IsoFilesystemStrategy, SplitFile, WriteMode,
    };
    use std::path::PathBuf;

    fn el_torito(style: BootStyle) -> ElToritoBootInfo {
        ElToritoBootInfo {
            catalog_lba: 20,
            platform: BootCatalogPlatform::X86,
            validation_entry_valid: true,
            default_entry_bootable: true,
            emulation: BootEmulation::NoEmulation,
            load_segment: 0,
            system_type: 0,
            sector_count: 4,
            load_rba: 21,
            boot_style: style,
            boot_entries: Vec::new(),
        }
    }

    fn assets(paths: &[(&str, IsoBootAssetKind)]) -> IsoDirectoryMetadata {
        IsoDirectoryMetadata {
            root_extent_lba: 18,
            root_data_length: 2048,
            scanned_directories: 3,
            entries: Vec::new(),
            boot_assets: paths
                .iter()
                .map(|(path, kind)| IsoBootAsset {
                    kind: *kind,
                    path: path.to_string(),
                })
                .collect(),
            truncated: false,
        }
    }

    fn windows_iso() -> ImageAnalysis {
        ImageAnalysis {
            plan: ImagePlan {
                source: PathBuf::from("D:/Images/Windows11_Client_x64_en-gb_26300_9457.iso"),
                file_name: "Windows11_Client_x64_en-gb_26300_9457.iso".into(),
                size_bytes: 9_030_170_624,
                status: ImagePlanStatus::Ready,
                status_reason: "image plan is trusted for the next safety phase".into(),
                kind: ImageKind::Iso,
                detection_method: DetectionMethod::Signature,
                signatures: vec![ImageSignature::Iso9660, ImageSignature::Udf],
                hybrid: false,
                boot_metadata: Some(IsoBootMetadata {
                    el_torito: Some(el_torito(BootStyle::Hybrid)),
                    directory: Some(assets(&[
                        ("BOOTMGR", IsoBootAssetKind::BootManager),
                        ("EFI/BOOT/BOOTX64.EFI", IsoBootAssetKind::UefiBootloader),
                        ("SOURCES/INSTALL.WIM", IsoBootAssetKind::InstallerImage),
                        ("EFI/BOOT", IsoBootAssetKind::UefiBootloader),
                    ])),
                }),
                write_mode: WriteMode::FileSystem,
                write_strategy: ImageWriteStrategy::IsoFileSystemBootSetup,
                iso_files: Some(Ok(IsoFilesSummary {
                    label: "CCCOMA_X64FRE_EN-GB_DV9".into(),
                    file_count: 900,
                    file_bytes: 9_000_000_000,
                    largest_file: Some(("sources/install.wim".into(), 6_657_199_308)),
                    uefi_loader: Some("efi/boot/bootx64.efi".into()),
                    split_files: vec![SplitFile {
                        path: "sources/install.wim".into(),
                        size_bytes: 6_657_199_308,
                        parts: vec!["install.swm".into(), "install2.swm".into()],
                    }],
                    cannot_copy: Vec::new(),
                })),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            family: ImageFamily::WindowsInstaller,
            filesystem_strategy: Some(IsoFilesystemStrategy::WindowsInstallerBootSetup),
        }
    }

    fn hybrid_linux_iso() -> ImageAnalysis {
        let mut analysis = windows_iso();
        let plan = &mut analysis.plan;
        plan.file_name = "ubuntu-24.04.1-live-server-amd64.iso".into();
        plan.size_bytes = 2_832_413_696;
        plan.signatures = vec![ImageSignature::Iso9660, ImageSignature::Mbr];
        plan.hybrid = true;
        plan.write_mode = WriteMode::Raw;
        plan.write_strategy = ImageWriteStrategy::HybridIsoRawWrite;
        plan.iso_files = None;
        plan.boot_metadata = Some(IsoBootMetadata {
            el_torito: Some(el_torito(BootStyle::Hybrid)),
            directory: Some(assets(&[
                ("boot/grub/grub.cfg", IsoBootAssetKind::GrubBootPath),
                ("CASPER/VMLINUZ", IsoBootAssetKind::LinuxKernel),
            ])),
        });
        analysis.family = ImageFamily::LinuxInstaller;
        analysis.filesystem_strategy = Some(IsoFilesystemStrategy::LinuxInstallerBootSetup);
        analysis
    }

    fn rows(analysis: &ImageAnalysis) -> Vec<(String, String, String)> {
        image_view(analysis)
            .rows
            .into_iter()
            .map(|row| (row.key.to_string(), row.value, row.detail))
            .collect()
    }

    #[test]
    fn a_windows_iso_matches_frame_02() {
        let analysis = windows_iso();
        assert_eq!(analysis.flow(), Flow::Windows);
        let view = image_view(&analysis);
        assert_eq!(view.title, "Image checked");
        assert_eq!(view.size, "8.41 GB");
        assert_eq!(view.status, Status::Ready);
        assert_eq!(view.hint, "Next, choose the drive to write to.");
        assert_eq!(
            rows(&analysis),
            [
                ("Image family".into(), "Windows installer".into(), "".into()),
                (
                    "Boot style".into(),
                    "UEFI".into(),
                    "boot catalog valid".into()
                ),
                (
                    "Format detected".into(),
                    "UDF".into(),
                    "with ISO 9660 bridge".into()
                ),
                (
                    "Write method".into(),
                    "Windows installer: files copied to a FAT32 stick (UEFI)".into(),
                    "".into()
                ),
                (
                    "Boot files found".into(),
                    "/bootmgr · /efi/boot/bootx64.efi · /sources/install.wim".into(),
                    "".into()
                ),
            ]
        );
        let notes: Vec<_> = view.notes.iter().map(|note| note.text.as_str()).collect();
        assert_eq!(
            notes,
            [
                "install.wim (6.20 GB) will be split into 2 parts to fit FAT32. Windows Setup reads them as normal.",
                "Boots on UEFI PCs only. Legacy BIOS mode is not supported.",
            ]
        );
        assert_eq!(view.notes[0].status, Status::None);
        assert_eq!(view.notes[1].status, Status::NeedsInspection);
    }

    #[test]
    fn a_hybrid_linux_iso_matches_frame_03() {
        let analysis = hybrid_linux_iso();
        assert_eq!(analysis.flow(), Flow::Other);
        let rows = rows(&analysis);
        assert_eq!(rows[0].1, "Linux installer");
        assert_eq!(rows[1].1, "BIOS + UEFI hybrid");
        assert_eq!(rows[2].1, "ISO 9660");
        assert_eq!(rows[2].2, "checked by file signature");
        assert_eq!(rows[3].1, "Hybrid ISO: copied to the drive byte for byte");
        assert_eq!(rows[4].1, "/boot/grub/grub.cfg · /casper/vmlinuz");
        let view = image_view(&analysis);
        assert_eq!(view.notes[0].status, Status::Ready);
        assert!(view.notes[1].text.starts_with("This ISO is hybrid"));
    }

    #[test]
    fn an_iso_without_a_boot_catalog_matches_frame_04() {
        let mut analysis = windows_iso();
        analysis.plan.file_name = "recovery-tools-v3.iso".into();
        analysis.plan.signatures = vec![ImageSignature::Iso9660];
        analysis.plan.status = ImagePlanStatus::NeedsInspection;
        analysis.plan.status_reason = "ISO boot metadata needs deeper inspection".into();
        analysis.plan.write_strategy = ImageWriteStrategy::IsoNeedsDeeperInspection;
        analysis.plan.boot_metadata = None;
        analysis.family = ImageFamily::Unknown;
        assert_eq!(analysis.flow(), Flow::Other);
        assert!(!analysis.can_continue());

        let view = image_view(&analysis);
        assert_eq!(view.title, "Image needs a closer look");
        assert_eq!(view.status_label, "Needs inspection");
        assert_eq!(view.hint, "This image can't be written yet.");
        assert_eq!(view.hint_icon, HintIcon::Info);
        let rows = view.rows;
        assert_eq!(rows[0].value, "Unknown");
        assert_eq!(rows[1].value, "Boot catalog missing or invalid");
        assert_eq!(rows[1].value_mark, Status::NeedsInspection);
        assert_eq!(rows[2].detail, "checked by file signature · not hybrid");
        assert_eq!(rows[3].value, "Needs deeper inspection");
        assert_eq!(rows[4].value, "None the engine recognises");
        assert!(rows[4].secondary);
        assert!(view.notes[0].text.contains("El Torito"));
    }

    #[test]
    fn blocked_and_raw_images() {
        let mut analysis = windows_iso();
        analysis.plan.status = ImagePlanStatus::Blocked;
        analysis.plan.status_reason = "image file is empty".into();
        analysis.plan.size_bytes = 0;
        let view = image_view(&analysis);
        assert_eq!(view.title, "Image can't be written");
        assert_eq!(view.description, "The file is empty. Choose another image.");
        assert_eq!(view.status, Status::Blocked);

        let mut analysis = windows_iso();
        analysis.plan.kind = ImageKind::RawDiskImage;
        analysis.plan.signatures = vec![ImageSignature::Mbr, ImageSignature::Gpt];
        analysis.plan.write_strategy = ImageWriteStrategy::RawBlockWrite;
        analysis.plan.boot_metadata = None;
        analysis.family = ImageFamily::RawDiskImage;
        let view = image_view(&analysis);
        assert_eq!(view.rows.len(), 4, "raw images have no boot files row");
        assert_eq!(view.rows[2].value, "GPT disk image");
        assert_eq!(
            view.rows[3].value,
            "Raw disk image: copied to the drive byte for byte"
        );
        assert_eq!(analysis.flow(), Flow::Other);
    }

    #[test]
    fn more_than_three_boot_files_end_in_a_count() {
        let mut analysis = hybrid_linux_iso();
        analysis.plan.boot_metadata = Some(IsoBootMetadata {
            el_torito: Some(el_torito(BootStyle::Hybrid)),
            directory: Some(assets(&[
                ("CASPER/INITRD", IsoBootAssetKind::Initrd),
                ("CASPER/VMLINUZ", IsoBootAssetKind::LinuxKernel),
                ("ISOLINUX/BOOT.CAT", IsoBootAssetKind::LinuxBootloader),
                ("ISOLINUX/ISOLINUX.BIN", IsoBootAssetKind::LinuxBootloader),
                ("ISOLINUX/LDLINUX.C32", IsoBootAssetKind::LinuxBootloader),
                ("BOOT/GRUB/EFI.IMG", IsoBootAssetKind::GrubBootPath),
                ("BOOT/GRUB/GRUB.CFG", IsoBootAssetKind::GrubBootPath),
                ("EFI/BOOT/BOOTX64.EFI", IsoBootAssetKind::UefiBootloader),
            ])),
        });
        let view = image_view(&analysis);
        assert_eq!(
            view.rows[4].value,
            "/casper/initrd · /casper/vmlinuz · /isolinux/boot.cat · +5 more"
        );
        assert_eq!(super::boot_file_paths(&analysis.plan).len(), 8);
    }

    #[test]
    fn iso_paths_are_shown_as_mounted() {
        assert_eq!(display_iso_path("BOOTMGR"), "/bootmgr");
        assert_eq!(
            display_iso_path("SOURCES/INSTALL.WIM;1"),
            "/sources/install.wim"
        );
        assert_eq!(
            display_iso_path("/EFI/boot/bootx64.efi"),
            "/EFI/boot/bootx64.efi"
        );
    }
}
