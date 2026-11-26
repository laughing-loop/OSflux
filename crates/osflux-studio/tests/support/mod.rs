//! A Studio window without a display, and an engine stand-in with sample
//! images and drives, so the steps can be driven like a person would.

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use osflux_engine::{
    build_safety_check, build_write_intent, build_write_preflight, BootCatalogPlatform,
    BootEmulation, BootStyle, CancelToken, DetectionMethod, DeviceAccess, DriveContents,
    DriveContentsState, DrivePartition, ElToritoBootInfo, FilesystemKind, ImageFamily, ImageKind,
    ImagePlan, ImagePlanStatus, ImageSignature, ImageWriteStrategy, IsoBootAsset, IsoBootAssetKind,
    IsoBootMetadata, IsoDirectoryMetadata, IsoFilesystemStrategy, PartitionScheme, RawWritePhase,
    RawWriteProgress, RawWriteReport, SafetyCheck, TargetAssessment, TargetAssessmentStatus,
    Verification, WriteIntentStatus, WriteMode,
};
use osflux_studio::confirm_step::{ConfirmFailure, Confirmation};
use osflux_studio::drive_step::ChosenDrive;
use osflux_studio::image_step::ImageAnalysis;
use osflux_studio::safety_step::SafetyFailure;
use osflux_studio::session::{Engine, Jobs, Session};
use osflux_studio::settings::Settings;
use osflux_studio::shell::{EngineSetup, Shell};
use osflux_studio::write_step::{WriteFailure, WriteRequest};
use osflux_studio::AppWindow;

thread_local! {
    // The testing backend is set up once per test thread.
    static BACKEND: () = i_slint_backend_testing::init_no_event_loop();
}

pub fn window() -> AppWindow {
    BACKEND.with(|_| ());
    AppWindow::new().expect("the Studio window builds")
}

pub const WINDOWS_ISO: &str = "D:/Images/Windows11_Client_x64_en-gb_26300_9457.iso";
pub const LINUX_ISO: &str = "D:/Images/ubuntu-24.04.1-live-server-amd64.iso";
pub const BROKEN_ISO: &str = "D:/Images/recovery-tools-v3.iso";
pub const MISSING_ISO: &str = "D:/Images/missing.iso";

/// The stand-in engine's clock starts here; confirmations last 600 s.
pub const START_TIME: u64 = 1_800_000_000;

/// Sample images by path, and drives that tests can plug in and pull out.
pub struct FakeEngine {
    pub drives: Mutex<Vec<TargetAssessment>>,
    /// What `now_unix_seconds` returns, so tests can let time pass.
    pub now: Mutex<u64>,
    /// How the next write ends: `None` writes (and verifies) successfully.
    pub write_failure: Mutex<Option<WriteFailure>>,
    /// Every write asked for.
    pub writes: Mutex<Vec<WriteRequest>>,
}

/// The SHA-256 a successful sample write reports.
pub const WRITTEN_SHA256: &str = "2a1ab0d46d7285f738d082858925d3303f3dc430f0a4eab5aa238325a4274906";
const WRITTEN_BYTES: u64 = 8 * 1024 * 1024 * 1024;

impl FakeEngine {
    pub fn new(drives: Vec<TargetAssessment>) -> Self {
        Self {
            drives: Mutex::new(drives),
            now: Mutex::new(START_TIME),
            write_failure: Mutex::new(None),
            writes: Mutex::new(Vec::new()),
        }
    }
}

impl Engine for FakeEngine {
    /// Reports writing (and verifying) in halves, then the outcome set in
    /// `write_failure`. Nothing is written anywhere.
    fn write(
        &self,
        request: &WriteRequest,
        _cancel: &CancelToken,
        progress: &mut dyn FnMut(RawWriteProgress),
    ) -> Result<RawWriteReport, WriteFailure> {
        self.writes.lock().unwrap().push(request.clone());
        let mut phases = vec![RawWritePhase::Writing];
        if request.verify {
            phases.push(RawWritePhase::Verifying);
        }
        for phase in phases {
            for completed_bytes in [0, WRITTEN_BYTES / 2, WRITTEN_BYTES] {
                progress(RawWriteProgress {
                    phase,
                    completed_bytes,
                    total_bytes: WRITTEN_BYTES,
                });
            }
        }
        if let Some(failure) = self.write_failure.lock().unwrap().take() {
            return Err(failure);
        }
        let verification = if request.verify {
            Verification::ReadBack
        } else {
            Verification::Skipped
        };
        Ok(RawWriteReport {
            session_id: "OSFLUX-SESSION-TEST".to_string(),
            image_path: request.image.display().to_string(),
            target_path: format!("\\\\.\\PhysicalDrive{}", request.target_index),
            target_identity_code: String::new(),
            image_sha256: WRITTEN_SHA256.to_string(),
            bytes_written: WRITTEN_BYTES,
            bytes_verified: if request.verify { WRITTEN_BYTES } else { 0 },
            verification,
            forced_dismount: false,
            write_millis: 451_000,
            verify_millis: if request.verify { 451_000 } else { 0 },
            audit_recorded: true,
        })
    }

    fn analyze_image(&self, path: &Path) -> Result<ImageAnalysis, String> {
        match path.to_str() {
            Some(WINDOWS_ISO) => Ok(windows_iso()),
            Some(LINUX_ISO) => Ok(linux_iso()),
            Some(BROKEN_ISO) => Ok(broken_iso()),
            _ => Err(format!("image path does not exist: {}", path.display())),
        }
    }

    fn scan_drives(&self) -> Vec<TargetAssessment> {
        self.drives.lock().unwrap().clone()
    }

    fn safety_check(
        &self,
        image: &Path,
        drive: &ChosenDrive,
    ) -> Result<SafetyCheck, SafetyFailure> {
        let plan = self
            .analyze_image(image)
            .map_err(SafetyFailure::Image)?
            .plan;
        let target = self
            .drives
            .lock()
            .unwrap()
            .iter()
            .find(|target| target.index == drive.index)
            .cloned()
            .ok_or(SafetyFailure::DriveGone)?;
        let changed = target.identity_code != drive.identity;
        Ok(build_safety_check(plan, target, changed))
    }

    /// The engine's own intent for the sample drive, without recording it.
    fn confirm(&self, image: &Path, drive: &ChosenDrive) -> Result<Confirmation, ConfirmFailure> {
        let plan = self
            .analyze_image(image)
            .map_err(ConfirmFailure::Failed)?
            .plan;
        let target = self
            .drives
            .lock()
            .unwrap()
            .iter()
            .find(|target| target.index == drive.index)
            .cloned()
            .ok_or(ConfirmFailure::DriveChanged)?;
        if target.identity_code != drive.identity {
            return Err(ConfirmFailure::DriveChanged);
        }
        let intent = build_write_intent(&build_write_preflight(&plan, &target));
        if intent.status != WriteIntentStatus::ConfirmationRequired {
            return Err(ConfirmFailure::NotAllowed(intent.reason.clone()));
        }
        Ok(Confirmation {
            intent,
            expires_at_unix_seconds: self.now_unix_seconds() + 600,
        })
    }

    fn now_unix_seconds(&self) -> u64 {
        *self.now.lock().unwrap()
    }
}

pub struct Studio {
    pub window: AppWindow,
    pub shell: Rc<RefCell<Shell>>,
    pub engine: Arc<FakeEngine>,
}

impl Studio {
    pub fn session(&self) -> Rc<RefCell<Session>> {
        self.shell.borrow().session()
    }

    pub fn open(&self, path: &str) {
        self.session().borrow_mut().open_image(PathBuf::from(path));
    }

    pub fn studio(&self) -> osflux_studio::Studio<'_> {
        use slint::ComponentHandle;
        self.window.global::<osflux_studio::Studio>()
    }
}

/// A window wired to the stand-in engine, with the sample drives of
/// frame 05 plugged in.
pub fn studio_with(settings: Settings, drives: Vec<TargetAssessment>) -> Studio {
    let window = window();
    let engine = Arc::new(FakeEngine::new(drives));
    let shell = Shell::install_with(
        &window,
        settings,
        None,
        true,
        EngineSetup {
            engine: engine.clone(),
            jobs: Jobs::Inline,
            clock: || "13:28".to_string(),
        },
    );
    Studio {
        window,
        shell,
        engine,
    }
}

pub fn studio() -> Studio {
    studio_with(Settings::default(), frame_05_drives())
}

// ---- Sample drives ------------------------------------------------------

pub fn drive(
    index: u32,
    model: &str,
    size: u64,
    status: TargetAssessmentStatus,
    reason: &str,
) -> TargetAssessment {
    TargetAssessment {
        index,
        path: format!("\\\\.\\PhysicalDrive{index}"),
        status,
        reason: reason.to_string(),
        size_bytes: Some(size),
        model: Some(model.to_string()),
        interface_type: Some("USB".to_string()),
        media_type: Some("Removable Media".to_string()),
        serial_number: Some(format!("SERIAL{index}")),
        identity_code: Some(format!("{:04X}", 0x7F30 + index)),
        access: DeviceAccess::Readable,
        contains_system_volume: false,
        likely_removable: true,
        requires_confirmation: true,
        contents: None,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// One FAT32 partition with files, which the engine can name.
pub fn with_files(mut target: TargetAssessment) -> TargetAssessment {
    let size = target.size_bytes.unwrap_or(1 << 30);
    target.contents = Some(DriveContents {
        state: DriveContentsState::HasData,
        reason: "one FAT32 partition with files".to_string(),
        partition_scheme: Some(PartitionScheme::Mbr),
        partitions: vec![DrivePartition {
            number: 1,
            start_bytes: 1 << 20,
            size_bytes: size - (1 << 20),
            filesystem: Some(FilesystemKind::Fat32),
            label: Some("PHOTOS".to_string()),
            used_bytes: Some(2 << 30),
        }],
    });
    target
}

/// A drive OSflux could not read, so what it holds cannot be named.
pub fn with_unreadable_contents(mut target: TargetAssessment) -> TargetAssessment {
    target.contents = Some(DriveContents {
        state: DriveContentsState::Unknown,
        reason: "the drive could not be read".to_string(),
        partition_scheme: None,
        partitions: Vec::new(),
    });
    target
}

pub fn system_drive() -> TargetAssessment {
    let mut target = drive(
        0,
        "Samsung SSD 980 1TB",
        1_000_204_886_016,
        TargetAssessmentStatus::Blocked,
        "target contains the system volume",
    );
    target.contains_system_volume = true;
    target.interface_type = Some("SCSI".to_string());
    target.media_type = Some("Fixed hard disk media".to_string());
    target
}

pub fn frame_05_drives() -> Vec<TargetAssessment> {
    vec![
        system_drive(),
        drive(
            2,
            "SanDisk Ultra 64GB",
            64_023_511_040,
            TargetAssessmentStatus::Eligible,
            "target is empty, so its identity code is enough to confirm a write",
        ),
        drive(
            3,
            "Kingston DataTraveler 8GB",
            8_004_304_896,
            TargetAssessmentStatus::Eligible,
            "target is empty, so its identity code is enough to confirm a write",
        ),
        with_files(drive(
            4,
            "Generic SD/MMC USB Device",
            15_931_539_456,
            TargetAssessmentStatus::NeedsConfirmation,
            "target looks eligible but still requires explicit confirmation",
        )),
    ]
}

// ---- Sample images ------------------------------------------------------

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

fn directory(assets: &[(&str, IsoBootAssetKind)]) -> IsoDirectoryMetadata {
    IsoDirectoryMetadata {
        root_extent_lba: 18,
        root_data_length: 2048,
        scanned_directories: 3,
        entries: Vec::new(),
        boot_assets: assets
            .iter()
            .map(|(path, kind)| IsoBootAsset {
                kind: *kind,
                path: path.to_string(),
            })
            .collect(),
        truncated: false,
    }
}

fn plan(path: &str, size: u64) -> ImagePlan {
    let path = PathBuf::from(path);
    ImagePlan {
        file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
        source: path,
        size_bytes: size,
        status: ImagePlanStatus::Ready,
        status_reason: "image plan is trusted for the next safety phase".to_string(),
        kind: ImageKind::Iso,
        detection_method: DetectionMethod::Signature,
        signatures: vec![ImageSignature::Iso9660, ImageSignature::Udf],
        hybrid: false,
        boot_metadata: None,
        write_mode: WriteMode::FileSystem,
        write_strategy: ImageWriteStrategy::IsoFileSystemBootSetup,
        iso_files: None,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

pub fn windows_iso() -> ImageAnalysis {
    let mut plan = plan(WINDOWS_ISO, 9_030_170_624);
    plan.boot_metadata = Some(IsoBootMetadata {
        el_torito: Some(el_torito(BootStyle::Hybrid)),
        directory: Some(directory(&[
            ("BOOTMGR", IsoBootAssetKind::BootManager),
            ("EFI/BOOT/BOOTX64.EFI", IsoBootAssetKind::UefiBootloader),
            ("SOURCES/INSTALL.WIM", IsoBootAssetKind::InstallerImage),
        ])),
    });
    ImageAnalysis {
        plan,
        family: ImageFamily::WindowsInstaller,
        filesystem_strategy: Some(IsoFilesystemStrategy::WindowsInstallerBootSetup),
    }
}

pub fn linux_iso() -> ImageAnalysis {
    let mut plan = plan(LINUX_ISO, 2_832_413_696);
    plan.signatures = vec![ImageSignature::Iso9660, ImageSignature::Mbr];
    plan.hybrid = true;
    plan.write_mode = WriteMode::Raw;
    plan.write_strategy = ImageWriteStrategy::HybridIsoRawWrite;
    plan.boot_metadata = Some(IsoBootMetadata {
        el_torito: Some(el_torito(BootStyle::Hybrid)),
        directory: Some(directory(&[(
            "CASPER/VMLINUZ",
            IsoBootAssetKind::LinuxKernel,
        )])),
    });
    ImageAnalysis {
        plan,
        family: ImageFamily::LinuxInstaller,
        filesystem_strategy: Some(IsoFilesystemStrategy::LinuxInstallerBootSetup),
    }
}

pub fn broken_iso() -> ImageAnalysis {
    let mut plan = plan(BROKEN_ISO, 1_266_679_808);
    plan.signatures = vec![ImageSignature::Iso9660];
    plan.status = ImagePlanStatus::NeedsInspection;
    plan.status_reason = "ISO boot metadata needs deeper inspection".to_string();
    plan.write_strategy = ImageWriteStrategy::IsoNeedsDeeperInspection;
    ImageAnalysis {
        plan,
        family: ImageFamily::Unknown,
        filesystem_strategy: Some(IsoFilesystemStrategy::NeedsDeeperInspection),
    }
}
