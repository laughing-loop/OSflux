use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use osflux_core::device::{BlockDevice, DeviceError, NativeBlockDevice};
use sha2::{Digest, Sha256};

use crate::audit::{AuditEvent, AuditEventKind, AuditLog};
use crate::clock::now_unix_seconds;
use crate::json::string_json;
use crate::target::{
    assess_target, TargetAssessment, TargetAssessmentError, TargetAssessmentStatus,
};
use crate::write_authorization::{
    AuthorizationProblem, AuthorizationStore, AuthorizationStoreError, ImageIdentity,
};
use crate::write_intent::confirmation_phrase;
use crate::write_mode_selection::{
    select_write_mode, SelectedWriteMode, WriteModeSelection, WriteModeSelectionError,
    WriteModeSelectionStatus,
};

/// Disks only accept writes whose offset and length are whole sectors.
/// 4096 bytes covers both 512-byte and 4K-sector drives.
pub const RAW_WRITE_ALIGNMENT_BYTES: u64 = 4096;
pub const MAX_RAW_WRITE_CHUNK_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawWritePlan {
    pub session_id: String,
    pub image_path: PathBuf,
    pub image_size_bytes: u64,
    pub target_index: u32,
    pub target_path: String,
    pub target_model: Option<String>,
    pub target_size_bytes: u64,
    pub target_identity_code: String,
    pub chunk_size_bytes: u64,
    /// When the one-time confirmation for this write expires; zero until
    /// `prepare_raw_write` has checked it.
    pub authorization_expires_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawWritePhase {
    Writing,
    Verifying,
}

impl RawWritePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Writing => "writing",
            Self::Verifying => "verifying",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawWriteProgress {
    pub phase: RawWritePhase,
    pub completed_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawWriteReport {
    pub session_id: String,
    pub image_path: String,
    pub target_path: String,
    pub target_identity_code: String,
    /// SHA-256 of the image as it was read for writing, in lowercase hex.
    pub image_sha256: String,
    pub bytes_written: u64,
    pub bytes_verified: u64,
    pub write_millis: u64,
    pub verify_millis: u64,
    /// Whether the outcome was added to the audit log.
    pub audit_recorded: bool,
}

impl RawWriteReport {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"written\",\n",
                "  \"sessionId\": {},\n",
                "  \"imageSource\": {},\n",
                "  \"imageSha256\": {},\n",
                "  \"targetPath\": {},\n",
                "  \"targetIdentityCode\": {},\n",
                "  \"bytesWritten\": {},\n",
                "  \"bytesVerified\": {},\n",
                "  \"writeMillis\": {},\n",
                "  \"verifyMillis\": {},\n",
                "  \"auditRecorded\": {}\n",
                "}}\n"
            ),
            string_json(&self.session_id),
            string_json(&self.image_path),
            string_json(&self.image_sha256),
            string_json(&self.target_path),
            string_json(&self.target_identity_code),
            self.bytes_written,
            self.bytes_verified,
            self.write_millis,
            self.verify_millis,
            self.audit_recorded,
        )
    }
}

#[derive(Debug)]
pub enum RawWriteError {
    /// The safety chain refused the write; nothing was opened.
    Blocked {
        reason: String,
    },
    /// The chunk size cannot be used for writing to a disk.
    InvalidChunkSize {
        chunk_size_bytes: u64,
    },
    /// The supplied phrase is not the one `--intent` prints for this drive.
    ConfirmationMismatch {
        target_index: u32,
    },
    /// The target no longer matches the drive that was confirmed; nothing was written.
    TargetChanged {
        reason: String,
    },
    /// There is no usable one-time confirmation from `--intent` for this write.
    Authorization {
        target_index: u32,
        problem: AuthorizationProblem,
    },
    AuthorizationStore(AuthorizationStoreError),
    /// The start of the write could not be recorded, so the drive was left alone.
    Audit {
        path: PathBuf,
        source: std::io::Error,
    },
    Plan(WriteModeSelectionError),
    Target(TargetAssessmentError),
    ImageChanged {
        expected_bytes: u64,
        actual_bytes: u64,
    },
    Image {
        path: PathBuf,
        source: std::io::Error,
    },
    Open(DeviceError),
    Device(DeviceError),
    VerifyMismatch {
        offset: u64,
    },
}

impl fmt::Display for RawWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocked { reason } => write!(formatter, "write blocked: {reason}"),
            Self::InvalidChunkSize { chunk_size_bytes } => write!(
                formatter,
                "chunk size {chunk_size_bytes} must be a multiple of {RAW_WRITE_ALIGNMENT_BYTES} bytes and at most {MAX_RAW_WRITE_CHUNK_BYTES} bytes"
            ),
            Self::ConfirmationMismatch { target_index } => write!(
                formatter,
                "the confirmation phrase does not match drive {target_index}; copy the exact phrase that --intent prints for it"
            ),
            Self::TargetChanged { reason } => {
                write!(formatter, "target changed before writing: {reason}")
            }
            Self::Authorization {
                target_index,
                problem,
            } => formatter.write_str(&problem.describe(*target_index)),
            Self::AuthorizationStore(error) => write!(formatter, "{error}"),
            Self::Audit { path, source } => write!(
                formatter,
                "could not add the write to the audit log {}: {source}; the drive was not touched",
                path.display()
            ),
            Self::Plan(error) => write!(formatter, "write planning failed: {error}"),
            Self::Target(error) => write!(formatter, "target check failed: {error}"),
            Self::ImageChanged {
                expected_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "image changed since it was planned: expected {expected_bytes} bytes, found {actual_bytes}"
            ),
            Self::Image { path, source } => {
                write!(formatter, "could not read image {}: {source}", path.display())
            }
            Self::Open(error) => write!(formatter, "could not open the target drive: {error}"),
            Self::Device(error) => write!(formatter, "target drive error: {error}"),
            Self::VerifyMismatch { offset } => write!(
                formatter,
                "verification failed: the drive differs from the image at byte {offset}"
            ),
        }
    }
}

impl std::error::Error for RawWriteError {}

/// Runs the full safety chain, then re-assesses the target and builds a plan
/// for writing a raw disk image. Nothing is opened for writing here.
pub fn prepare_raw_write(
    image_path: impl AsRef<Path>,
    target_index: u32,
    supplied_confirmation_phrase: &str,
    chunk_size_bytes: u64,
    authorizations: &AuthorizationStore,
) -> Result<RawWritePlan, RawWriteError> {
    let image_path = image_path.as_ref();
    validate_chunk_size(chunk_size_bytes)?;
    let selection = select_write_mode(
        image_path,
        target_index,
        Some(supplied_confirmation_phrase),
        chunk_size_bytes,
    )
    .map_err(RawWriteError::Plan)?;
    // A second, fresh look at the drive just before it is opened.
    let target = assess_target(target_index).map_err(RawWriteError::Target)?;

    let plan = build_raw_write_plan(
        image_path,
        &selection,
        &target,
        supplied_confirmation_phrase,
        chunk_size_bytes,
    )?;

    // The phrase alone stays valid for as long as the same drive sits at the
    // same index. The recorded confirmation also ties the write to this
    // image, a time limit, and a single use.
    let image = ImageIdentity::read(image_path).map_err(|source| RawWriteError::Image {
        path: image_path.to_path_buf(),
        source,
    })?;
    let authorization_error = |problem| RawWriteError::Authorization {
        target_index,
        problem,
    };
    let authorization = authorizations
        .load(target_index)
        .map_err(RawWriteError::AuthorizationStore)?
        .ok_or_else(|| authorization_error(AuthorizationProblem::Missing))?;
    authorization
        .check(
            plan.target_index,
            &plan.target_path,
            &plan.target_identity_code,
            &image,
            supplied_confirmation_phrase,
            now_unix_seconds(),
        )
        .map_err(authorization_error)?;

    Ok(RawWritePlan {
        authorization_expires_at_unix_seconds: authorization.expires_at_unix_seconds,
        ..plan
    })
}

pub fn build_raw_write_plan(
    image_path: &Path,
    selection: &WriteModeSelection,
    target: &TargetAssessment,
    supplied_confirmation_phrase: &str,
    chunk_size_bytes: u64,
) -> Result<RawWritePlan, RawWriteError> {
    validate_chunk_size(chunk_size_bytes)?;

    if target.index != selection.target_index || target.path != selection.target_path {
        return Err(RawWriteError::TargetChanged {
            reason: "the re-assessed target is not the planned target".to_string(),
        });
    }

    // The target and the phrase are checked before the rest of the chain, so
    // a blocked drive or a mistyped phrase is reported as itself rather than
    // as a downstream planning step that it blocked.
    if target.status == TargetAssessmentStatus::Blocked {
        return Err(RawWriteError::Blocked {
            reason: target.reason.clone(),
        });
    }

    let Some(identity_code) = target.identity_code.clone() else {
        return Err(RawWriteError::Blocked {
            reason: "target identity could not be read".to_string(),
        });
    };

    if confirmation_phrase(target.index, &identity_code) != supplied_confirmation_phrase {
        return Err(RawWriteError::ConfirmationMismatch {
            target_index: target.index,
        });
    }

    if selection.status != WriteModeSelectionStatus::Selected {
        return Err(RawWriteError::Blocked {
            reason: selection.reason.clone(),
        });
    }

    if selection.selected_mode != Some(SelectedWriteMode::RawBlockWrite) {
        return Err(RawWriteError::Blocked {
            reason: "only raw disk images and hybrid ISOs can be written so far; this ISO needs its files extracted, which is not supported yet"
                .to_string(),
        });
    }

    let Some(target_size_bytes) = target.size_bytes else {
        return Err(RawWriteError::Blocked {
            reason: "target size is unknown".to_string(),
        });
    };

    if selection.planned_bytes > target_size_bytes {
        return Err(RawWriteError::Blocked {
            reason: "image is larger than the target device".to_string(),
        });
    }

    Ok(RawWritePlan {
        session_id: selection.session_id.clone(),
        image_path: image_path.to_path_buf(),
        image_size_bytes: selection.planned_bytes,
        target_index: target.index,
        target_path: target.path.clone(),
        target_model: target.model.clone(),
        target_size_bytes,
        target_identity_code: identity_code,
        chunk_size_bytes,
        authorization_expires_at_unix_seconds: 0,
    })
}

/// Records the start in the audit log, uses up the confirmation, opens the
/// planned target, which locks and dismounts its volumes, then writes and
/// verifies the image and records the outcome. The drive is closed before
/// this returns.
pub fn write_image_to_target(
    plan: &RawWritePlan,
    authorizations: &AuthorizationStore,
    audit: &AuditLog,
    on_progress: impl FnMut(RawWriteProgress),
) -> Result<RawWriteReport, RawWriteError> {
    // No write without a record of it: when the start cannot be logged, the
    // confirmation and the drive are both left alone.
    audit
        .record(&AuditEvent::for_plan(AuditEventKind::WriteStarted, plan))
        .map_err(|source| RawWriteError::Audit {
            path: audit.path().to_path_buf(),
            source,
        })?;

    let outcome = consume_and_write(plan, authorizations, on_progress);

    // The write has already happened or failed by now, so a problem logging
    // its outcome is reported on the result instead of replacing it.
    let outcome_event = match &outcome {
        Ok(report) => {
            AuditEvent::for_plan(AuditEventKind::WriteSucceeded, plan).with_report(report)
        }
        Err(error) => {
            AuditEvent::for_plan(AuditEventKind::WriteFailed, plan).with_detail(error.to_string())
        }
    };
    let audit_recorded = audit.record(&outcome_event).is_ok();

    outcome.map(|report| RawWriteReport {
        audit_recorded,
        ..report
    })
}

fn consume_and_write(
    plan: &RawWritePlan,
    authorizations: &AuthorizationStore,
    on_progress: impl FnMut(RawWriteProgress),
) -> Result<RawWriteReport, RawWriteError> {
    // Used up before the drive is touched, so one confirmation can never
    // erase a drive twice, even when this write fails part-way.
    authorizations
        .consume(plan.target_index)
        .map_err(RawWriteError::AuthorizationStore)?;
    let device = NativeBlockDevice::open(&plan.target_path).map_err(RawWriteError::Open)?;
    execute_raw_write(plan, &device, on_progress)
}

/// Writes the image to `device` chunk by chunk, flushes, then reads every
/// byte back and compares it with the image.
pub fn execute_raw_write<D: BlockDevice>(
    plan: &RawWritePlan,
    device: &D,
    mut on_progress: impl FnMut(RawWriteProgress),
) -> Result<RawWriteReport, RawWriteError> {
    validate_chunk_size(plan.chunk_size_bytes)?;

    // The handle that was actually opened must be the drive that was confirmed.
    let device_size = device.size().map_err(RawWriteError::Device)?;
    if device_size != plan.target_size_bytes {
        return Err(RawWriteError::TargetChanged {
            reason: format!(
                "the opened drive holds {device_size} bytes but the confirmed target holds {} bytes",
                plan.target_size_bytes
            ),
        });
    }

    let image_error = |source| RawWriteError::Image {
        path: plan.image_path.clone(),
        source,
    };
    let mut image = File::open(&plan.image_path).map_err(image_error)?;
    let image_size = image.metadata().map_err(image_error)?.len();
    if image_size != plan.image_size_bytes {
        return Err(RawWriteError::ImageChanged {
            expected_bytes: plan.image_size_bytes,
            actual_bytes: image_size,
        });
    }

    let chunk_size = plan.chunk_size_bytes as usize;
    let mut image_buffer = vec![0u8; chunk_size];

    let write_started = Instant::now();
    // Hashed while writing so the report names exactly what went onto the
    // drive, without a second pass over the image.
    let mut image_hash = Sha256::new();
    let mut offset = 0u64;
    report(&mut on_progress, RawWritePhase::Writing, 0, image_size);
    while offset < image_size {
        let length = chunk_length(chunk_size, offset, image_size);
        image
            .read_exact(&mut image_buffer[..length])
            .map_err(image_error)?;
        image_hash.update(&image_buffer[..length]);
        let padded = padded_length(length, offset, device_size);
        image_buffer[length..padded].fill(0);

        device
            .write_at(offset, &image_buffer[..padded])
            .map_err(RawWriteError::Device)?;

        offset += length as u64;
        report(&mut on_progress, RawWritePhase::Writing, offset, image_size);
    }
    device.flush().map_err(RawWriteError::Device)?;
    let write_millis = write_started.elapsed().as_millis() as u64;
    let image_sha256 = image_hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let verify_started = Instant::now();
    let mut device_buffer = vec![0u8; chunk_size];
    image.seek(SeekFrom::Start(0)).map_err(image_error)?;
    offset = 0;
    report(&mut on_progress, RawWritePhase::Verifying, 0, image_size);
    while offset < image_size {
        let length = chunk_length(chunk_size, offset, image_size);
        image
            .read_exact(&mut image_buffer[..length])
            .map_err(image_error)?;
        let padded = padded_length(length, offset, device_size);

        device
            .read_at(offset, &mut device_buffer[..padded])
            .map_err(RawWriteError::Device)?;

        if let Some(position) = first_difference(&image_buffer[..length], &device_buffer[..length])
        {
            return Err(RawWriteError::VerifyMismatch {
                offset: offset + position as u64,
            });
        }

        offset += length as u64;
        report(
            &mut on_progress,
            RawWritePhase::Verifying,
            offset,
            image_size,
        );
    }
    let verify_millis = verify_started.elapsed().as_millis() as u64;

    Ok(RawWriteReport {
        session_id: plan.session_id.clone(),
        image_path: plan.image_path.display().to_string(),
        target_path: plan.target_path.clone(),
        target_identity_code: plan.target_identity_code.clone(),
        image_sha256,
        bytes_written: image_size,
        bytes_verified: image_size,
        write_millis,
        verify_millis,
        audit_recorded: false,
    })
}

fn validate_chunk_size(chunk_size_bytes: u64) -> Result<(), RawWriteError> {
    if chunk_size_bytes == 0
        || !chunk_size_bytes.is_multiple_of(RAW_WRITE_ALIGNMENT_BYTES)
        || chunk_size_bytes > MAX_RAW_WRITE_CHUNK_BYTES
    {
        return Err(RawWriteError::InvalidChunkSize { chunk_size_bytes });
    }

    Ok(())
}

fn chunk_length(chunk_size: usize, offset: u64, image_size: u64) -> usize {
    (chunk_size as u64).min(image_size - offset) as usize
}

/// The final chunk of an image is usually not a whole number of sectors, so
/// it is padded with zeros up to the alignment, without running past the end
/// of the drive.
fn padded_length(length: usize, offset: u64, device_size: u64) -> usize {
    let aligned = (length as u64).div_ceil(RAW_WRITE_ALIGNMENT_BYTES) * RAW_WRITE_ALIGNMENT_BYTES;
    aligned.min(device_size - offset) as usize
}

fn first_difference(expected: &[u8], actual: &[u8]) -> Option<usize> {
    expected
        .iter()
        .zip(actual)
        .position(|(expected, actual)| expected != actual)
}

fn report(
    on_progress: &mut impl FnMut(RawWriteProgress),
    phase: RawWritePhase,
    completed_bytes: u64,
    total_bytes: u64,
) {
    on_progress(RawWriteProgress {
        phase,
        completed_bytes,
        total_bytes,
    });
}

#[cfg(test)]
mod tests {
    use super::{
        build_raw_write_plan, execute_raw_write, write_image_to_target, RawWriteError,
        RawWritePhase, RawWritePlan, RawWriteProgress, RAW_WRITE_ALIGNMENT_BYTES,
    };
    use crate::audit::AuditLog;
    use crate::clock::now_unix_seconds;
    use crate::image_plan::{ImagePlanStatus, ImageWriteStrategy};
    use crate::target::{TargetAssessment, TargetAssessmentStatus};
    use crate::write_authorization::{AuthorizationStore, ImageIdentity, WriteAuthorization};
    use crate::write_mode_selection::{
        SelectedWriteMode, WriteModeSelection, WriteModeSelectionStatus,
    };
    use osflux_core::device::{BlockDevice, DeviceAccess, DeviceError, DeviceResult};
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};

    const MIB: u64 = 1024 * 1024;

    /// A drive held in memory. Like a real disk it rejects writes and reads
    /// that are not whole sectors, and it can corrupt one byte to exercise
    /// verification.
    struct MemoryDrive {
        data: RefCell<Vec<u8>>,
        corrupt_offset: Option<u64>,
        writes: RefCell<Vec<(u64, usize)>>,
    }

    impl MemoryDrive {
        fn new(size: u64) -> Self {
            Self {
                data: RefCell::new(vec![0xEE; size as usize]),
                corrupt_offset: None,
                writes: RefCell::new(Vec::new()),
            }
        }

        fn check_sectors(offset: u64, length: usize) -> DeviceResult<()> {
            if !offset.is_multiple_of(512) || !length.is_multiple_of(512) {
                return Err(DeviceError::Io {
                    operation: "sector alignment",
                    message: format!("offset {offset} length {length}"),
                });
            }

            Ok(())
        }
    }

    impl BlockDevice for MemoryDrive {
        fn open(_path: &str) -> DeviceResult<Self> {
            unreachable!("tests build the drive directly")
        }

        fn write_at(&self, offset: u64, buffer: &[u8]) -> DeviceResult<usize> {
            Self::check_sectors(offset, buffer.len())?;
            let start = offset as usize;
            let mut data = self.data.borrow_mut();
            data[start..start + buffer.len()].copy_from_slice(buffer);
            if let Some(corrupt) = self.corrupt_offset {
                if (offset..offset + buffer.len() as u64).contains(&corrupt) {
                    data[corrupt as usize] ^= 0xFF;
                }
            }
            self.writes.borrow_mut().push((offset, buffer.len()));
            Ok(buffer.len())
        }

        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> DeviceResult<usize> {
            Self::check_sectors(offset, buffer.len())?;
            let start = offset as usize;
            buffer.copy_from_slice(&self.data.borrow()[start..start + buffer.len()]);
            Ok(buffer.len())
        }

        fn flush(&self) -> DeviceResult<()> {
            Ok(())
        }

        fn size(&self) -> DeviceResult<u64> {
            Ok(self.data.borrow().len() as u64)
        }
    }

    #[test]
    fn writes_image_with_unaligned_tail_and_verifies_it() {
        let image = image_bytes((10 * MIB + 1000) as usize);
        let path = temp_image(&image);
        let drive = MemoryDrive::new(32 * MIB);
        let mut events = Vec::new();

        let report = execute_raw_write(
            &plan(&path, image.len() as u64, 32 * MIB),
            &drive,
            |event| events.push(event),
        )
        .unwrap();
        let _ = fs::remove_file(&path);

        let data = drive.data.borrow();
        assert_eq!(&data[..image.len()], &image[..]);
        assert_eq!(report.bytes_written, image.len() as u64);
        assert_eq!(report.bytes_verified, image.len() as u64);
        assert!(drive.writes.borrow().iter().all(|(offset, length)| offset
            .is_multiple_of(RAW_WRITE_ALIGNMENT_BYTES)
            && (*length as u64).is_multiple_of(RAW_WRITE_ALIGNMENT_BYTES)));
        assert_eq!(
            events.last(),
            Some(&RawWriteProgress {
                phase: RawWritePhase::Verifying,
                completed_bytes: image.len() as u64,
                total_bytes: image.len() as u64,
            })
        );
        assert!(events
            .iter()
            .any(|event| event.phase == RawWritePhase::Writing
                && event.completed_bytes == image.len() as u64));
    }

    #[test]
    fn reports_the_sha256_of_the_image_it_wrote() {
        let path = temp_image(b"abc");
        let drive = MemoryDrive::new(MIB);

        let report = execute_raw_write(&plan(&path, 3, MIB), &drive, |_| {}).unwrap();
        let _ = fs::remove_file(&path);

        // The published SHA-256 test vector for "abc".
        assert_eq!(
            report.image_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(report.to_json_pretty().contains(
            "\"imageSha256\": \"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\""
        ));
    }

    #[test]
    fn pads_final_chunk_with_zeros_up_to_the_alignment_only() {
        let image = image_bytes(5000);
        let path = temp_image(&image);
        let drive = MemoryDrive::new(MIB);

        execute_raw_write(&plan(&path, image.len() as u64, MIB), &drive, |_| {}).unwrap();
        let _ = fs::remove_file(&path);

        let data = drive.data.borrow();
        let padded_end = 2 * RAW_WRITE_ALIGNMENT_BYTES as usize;
        assert!(data[image.len()..padded_end].iter().all(|byte| *byte == 0));
        assert!(data[padded_end..].iter().all(|byte| *byte == 0xEE));
    }

    #[test]
    fn padding_never_runs_past_the_end_of_the_drive() {
        let drive_size = 3 * 512;
        let image = image_bytes(drive_size as usize - 100);
        let path = temp_image(&image);
        let drive = MemoryDrive::new(drive_size);

        execute_raw_write(&plan(&path, image.len() as u64, drive_size), &drive, |_| {}).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(*drive.writes.borrow(), vec![(0, drive_size as usize)]);
    }

    #[test]
    fn refuses_when_opened_drive_size_differs_from_confirmed_target() {
        let image = image_bytes(8192);
        let path = temp_image(&image);
        let drive = MemoryDrive::new(2 * MIB);

        let error =
            execute_raw_write(&plan(&path, image.len() as u64, MIB), &drive, |_| {}).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(matches!(error, RawWriteError::TargetChanged { .. }));
        assert!(drive.writes.borrow().is_empty());
    }

    #[test]
    fn refuses_when_image_changed_since_planning() {
        let image = image_bytes(8192);
        let path = temp_image(&image);
        let drive = MemoryDrive::new(MIB);

        let error = execute_raw_write(&plan(&path, 4096, MIB), &drive, |_| {}).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(matches!(
            error,
            RawWriteError::ImageChanged {
                expected_bytes: 4096,
                actual_bytes: 8192
            }
        ));
        assert!(drive.writes.borrow().is_empty());
    }

    #[test]
    fn reports_the_first_byte_that_fails_verification() {
        let image = image_bytes((6 * MIB) as usize);
        let path = temp_image(&image);
        let mut drive = MemoryDrive::new(8 * MIB);
        drive.corrupt_offset = Some(5 * MIB + 17);

        let error = execute_raw_write(&plan(&path, image.len() as u64, 8 * MIB), &drive, |_| {})
            .unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(matches!(
            error,
            RawWriteError::VerifyMismatch { offset } if offset == 5 * MIB + 17
        ));
    }

    #[test]
    fn uses_up_the_confirmation_even_when_the_drive_cannot_be_opened() {
        // Drive 250 never exists, so opening it fails without touching a disk.
        let image = image_bytes(8192);
        let path = temp_image(&image);
        let store = AuthorizationStore::new(path.with_extension("authorizations"));
        let mut write_plan = plan(&path, image.len() as u64, MIB);
        write_plan.target_index = 250;
        write_plan.target_path = r"\\.\PhysicalDrive250".to_string();
        store
            .save(&WriteAuthorization {
                target_index: 250,
                target_path: write_plan.target_path.clone(),
                target_identity_code: "7F3A".to_string(),
                confirmation_phrase: "OSFLUX ERASE TARGET 250 7F3A".to_string(),
                image: ImageIdentity::read(&path).unwrap(),
                issued_at_unix_seconds: now_unix_seconds(),
                expires_at_unix_seconds: now_unix_seconds() + 600,
            })
            .unwrap();

        let audit = AuditLog::new(path.with_extension("audit.log"));

        let first = write_image_to_target(&write_plan, &store, &audit, |_| {}).unwrap_err();
        let second = write_image_to_target(&write_plan, &store, &audit, |_| {}).unwrap_err();
        let log = fs::read_to_string(audit.path()).unwrap();
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(audit.path());
        let _ = fs::remove_dir_all(store.directory());

        assert!(matches!(first, RawWriteError::Open(_)), "{first}");
        assert!(
            matches!(second, RawWriteError::AuthorizationStore(_)),
            "{second}"
        );
        let events: Vec<_> = log
            .lines()
            .map(|line| {
                line.split("\"event\":")
                    .nth(1)
                    .unwrap()
                    .split(',')
                    .next()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            events,
            [
                "\"write-started\"",
                "\"write-failed\"",
                "\"write-started\"",
                "\"write-failed\""
            ]
        );
    }

    #[test]
    fn leaves_the_confirmation_alone_when_the_audit_log_cannot_be_written() {
        let image = image_bytes(8192);
        let path = temp_image(&image);
        let store = AuthorizationStore::new(path.with_extension("authorizations"));
        let mut write_plan = plan(&path, image.len() as u64, MIB);
        write_plan.target_index = 250;
        write_plan.target_path = r"\\.\PhysicalDrive250".to_string();
        let authorization = WriteAuthorization {
            target_index: 250,
            target_path: write_plan.target_path.clone(),
            target_identity_code: "7F3A".to_string(),
            confirmation_phrase: "OSFLUX ERASE TARGET 250 7F3A".to_string(),
            image: ImageIdentity::read(&path).unwrap(),
            issued_at_unix_seconds: now_unix_seconds(),
            expires_at_unix_seconds: now_unix_seconds() + 600,
        };
        store.save(&authorization).unwrap();
        // A folder where the log file should be makes the log unwritable.
        let audit_path = path.with_extension("audit-dir");
        fs::create_dir_all(&audit_path).unwrap();

        let error = write_image_to_target(&write_plan, &store, &AuditLog::new(&audit_path), |_| {})
            .unwrap_err();
        let still_authorized = store.load(250).unwrap();
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&audit_path);
        let _ = fs::remove_dir_all(store.directory());

        assert!(matches!(error, RawWriteError::Audit { .. }), "{error}");
        assert_eq!(still_authorized, Some(authorization));
    }

    #[test]
    fn plans_a_raw_write_for_a_confirmed_removable_target() {
        let plan = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &target(Some("7F3A")),
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
        )
        .unwrap();

        assert_eq!(plan.target_path, "\\\\.\\PhysicalDrive2");
        assert_eq!(plan.target_identity_code, "7F3A");
        assert_eq!(plan.image_size_bytes, 6 * MIB);
        assert_eq!(plan.target_size_bytes, 16 * MIB);
    }

    #[test]
    fn refuses_iso_images_for_now() {
        let error = build_raw_write_plan(
            Path::new("disk.iso"),
            &selection(Some(SelectedWriteMode::IsoFilesystemBootSetup)),
            &target(Some("7F3A")),
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
        )
        .unwrap_err();

        assert!(matches!(error, RawWriteError::Blocked { .. }));
    }

    #[test]
    fn refuses_when_a_different_drive_now_holds_the_index() {
        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &target(Some("0B21")),
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RawWriteError::ConfirmationMismatch { target_index: 2 }
        ));
    }

    #[test]
    fn reports_a_mistyped_phrase_instead_of_the_step_it_blocked() {
        // A wrong phrase also blocks mode selection, but the phrase is what
        // the operator needs to hear about.
        let mut blocked = selection(Some(SelectedWriteMode::RawBlockWrite));
        blocked.status = WriteModeSelectionStatus::Blocked;
        blocked.reason = "write mode selection blocked: chunk plan blocked because write confirmation did not match".to_string();

        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &blocked,
            &target(Some("2E85")),
            "OSFLUX ERASE TARGET 2 XXXX",
            4 * MIB,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RawWriteError::ConfirmationMismatch { target_index: 2 }
        ));
        assert!(!error.to_string().contains("2E85"));
    }

    #[test]
    fn reports_a_blocked_target_before_the_phrase() {
        let mut external_disk = target(Some("34B5"));
        external_disk.status = TargetAssessmentStatus::Blocked;
        external_disk.reason = "target is an external hard disk or SSD".to_string();

        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &external_disk,
            "OSFLUX ERASE TARGET 2 XXXX",
            4 * MIB,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RawWriteError::Blocked { reason } if reason == "target is an external hard disk or SSD"
        ));
    }

    #[test]
    fn refuses_when_mode_selection_did_not_pass() {
        let mut blocked = selection(Some(SelectedWriteMode::RawBlockWrite));
        blocked.status = WriteModeSelectionStatus::Blocked;
        blocked.reason = "write mode selection blocked by chunk plan".to_string();

        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &blocked,
            &target(Some("7F3A")),
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RawWriteError::Blocked { reason } if reason.contains("chunk plan")
        ));
    }

    #[test]
    fn refuses_chunk_sizes_a_disk_cannot_take() {
        for chunk_size in [0, 1000, 128 * MIB] {
            let error = build_raw_write_plan(
                Path::new("disk.img"),
                &selection(Some(SelectedWriteMode::RawBlockWrite)),
                &target(Some("7F3A")),
                "OSFLUX ERASE TARGET 2 7F3A",
                chunk_size,
            )
            .unwrap_err();

            assert!(matches!(error, RawWriteError::InvalidChunkSize { .. }));
        }
    }

    fn plan(image_path: &Path, image_size: u64, target_size: u64) -> RawWritePlan {
        RawWritePlan {
            session_id: "OSFLUX-SESSION-TEST".to_string(),
            image_path: image_path.to_path_buf(),
            image_size_bytes: image_size,
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            target_model: Some("Test Stick".to_string()),
            target_size_bytes: target_size,
            target_identity_code: "7F3A".to_string(),
            chunk_size_bytes: 4 * MIB,
            authorization_expires_at_unix_seconds: 0,
        }
    }

    fn selection(selected_mode: Option<SelectedWriteMode>) -> WriteModeSelection {
        WriteModeSelection {
            status: WriteModeSelectionStatus::Selected,
            reason: "selected".to_string(),
            selected_mode,
            image_source: "disk.img".to_string(),
            image_kind: "raw-disk-image".to_string(),
            image_status: ImagePlanStatus::Ready,
            image_write_strategy: ImageWriteStrategy::RawBlockWrite,
            session_id: "OSFLUX-SESSION-TEST".to_string(),
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            chunk_count: 2,
            planned_bytes: 6 * MIB,
            dry_run: true,
            write_access_attempted: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn target(identity_code: Option<&str>) -> TargetAssessment {
        TargetAssessment {
            index: 2,
            path: "\\\\.\\PhysicalDrive2".to_string(),
            status: TargetAssessmentStatus::NeedsConfirmation,
            reason: "target looks eligible but still requires explicit confirmation".to_string(),
            size_bytes: Some(16 * MIB),
            model: Some("Test Stick".to_string()),
            interface_type: Some("USB".to_string()),
            media_type: Some("Removable Media".to_string()),
            serial_number: None,
            identity_code: identity_code.map(str::to_string),
            access: DeviceAccess::Readable,
            contains_system_volume: false,
            likely_removable: true,
            requires_confirmation: true,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn image_bytes(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index % 251) as u8).collect()
    }

    fn temp_image(bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "osflux-raw-write-test-{}-{}.img",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, bytes).unwrap();
        path
    }
}
