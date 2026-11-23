use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use osflux_core::device::{BlockDevice, DeviceAccess, DeviceError, NativeBlockDevice};
use sha2::{Digest, Sha256};

use crate::audit::{AuditEvent, AuditEventKind, AuditLog};
use crate::clock::now_unix_seconds;
use crate::drive_contents::inspect_drive_contents;
use crate::file_copy::{lay_out_uefi_fat32, prepare_copy, FileCopyError, UefiFat32Layout};
use crate::image_io::ImageRead;
use crate::json::string_json;
use crate::target::{
    assess_target, TargetAssessment, TargetAssessmentError, TargetAssessmentStatus,
};
use crate::write_authorization::{
    AuthorizationProblem, AuthorizationStore, AuthorizationStoreError, ImageIdentity,
};
use crate::write_intent::{confirmation_phrase, match_confirmation, ConfirmationForm};
use crate::write_mode_selection::{
    select_write_mode, SelectedWriteMode, WriteModeSelection, WriteModeSelectionError,
    WriteModeSelectionStatus,
};

/// Disks only accept writes whose offset and length are whole sectors.
/// 4096 bytes covers both 512-byte and 4K-sector drives.
pub const RAW_WRITE_ALIGNMENT_BYTES: u64 = 4096;
pub const MAX_RAW_WRITE_CHUNK_BYTES: u64 = 64 * 1024 * 1024;
/// How much of the end of the drive is cleared when files are copied, so an
/// old backup GPT there cannot be mistaken for the drive's partition table.
const TRAILING_WIPE_BYTES: u64 = 1024 * 1024;

/// How the image becomes the drive's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteLayout {
    /// The image is copied byte for byte: raw disk images and hybrid ISOs.
    Raw,
    /// The ISO's files are copied onto a new MBR drive with one FAT32
    /// partition that starts in UEFI mode, as Windows installers need.
    UefiFat32(UefiFat32Layout),
}

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
    /// Which confirmation was typed. With the identity code, the drive is
    /// checked again once it is locked, and the write stops if it is no
    /// longer empty.
    pub confirmation_form: ConfirmationForm,
    pub layout: WriteLayout,
    /// When the one-time confirmation for this write expires; zero until
    /// `prepare_raw_write` has checked it.
    pub authorization_expires_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawWritePhase {
    /// Reading the ISO to report its SHA-256, before copying its files.
    Hashing,
    Writing,
    Verifying,
}

impl RawWritePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hashing => "hashing",
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
    /// SHA-256 of the image file, in lowercase hex: the bytes written for a
    /// raw copy, the ISO itself when its files were copied.
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
    /// The drive at the target index is not one OSflux writes to, such as an
    /// external hard disk. Named, because drive numbers move when drives are
    /// unplugged and plugged back in.
    TargetBlocked {
        target_index: u32,
        target_model: Option<String>,
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
    /// The identity code was typed for a drive that is not empty, so only the
    /// full phrase can confirm the write.
    ShortConfirmationRefused {
        target_index: u32,
        reason: String,
    },
    /// OSflux can only read the drive's details, not open it, so it stops
    /// before the final prompt and leaves the confirmation unused.
    NeedsAdministrator {
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
            Self::TargetBlocked {
                target_index,
                target_model,
                reason,
            } => write!(
                formatter,
                "write blocked: drive {target_index} is {}, and {reason}",
                target_model.as_deref().unwrap_or("a drive of unknown model")
            ),
            Self::InvalidChunkSize { chunk_size_bytes } => write!(
                formatter,
                "chunk size {chunk_size_bytes} must be a multiple of {RAW_WRITE_ALIGNMENT_BYTES} bytes and at most {MAX_RAW_WRITE_CHUNK_BYTES} bytes"
            ),
            Self::ConfirmationMismatch { target_index } => write!(
                formatter,
                "the confirmation phrase does not match drive {target_index}; copy the exact phrase that --intent prints for it"
            ),
            Self::ShortConfirmationRefused {
                target_index,
                reason,
            } => write!(
                formatter,
                "the identity code alone only confirms a write to an empty drive, and drive {target_index} is not empty ({reason}); type the full phrase that --intent prints instead"
            ),
            Self::NeedsAdministrator { target_index } => write!(
                formatter,
                "writing to drive {target_index} needs Administrator rights, which this window does not have; the confirmation from --intent was not used up"
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

    let session_id = selection.session_id.clone();
    let target_path = target.path.clone();
    let plan = build_raw_write_plan(
        image_path,
        &selection,
        &target,
        supplied_confirmation_phrase,
        chunk_size_bytes,
        |drive_bytes| {
            let sector_bytes = read_sector_size(&target_path)?;
            lay_out_uefi_fat32(
                image_path,
                drive_bytes,
                sector_bytes,
                &session_id,
                now_unix_seconds(),
            )
            .map_err(|error| file_copy_error(image_path, error))
        },
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

/// Checks the target, the confirmation and the selected mode, then plans
/// the write. For an ISO whose files are copied, `lay_out_files` plans the
/// FAT32 drive for a drive of the given size; it only runs once every other
/// check has passed, so a blocked drive or a mistyped phrase is reported as
/// itself.
pub fn build_raw_write_plan(
    image_path: &Path,
    selection: &WriteModeSelection,
    target: &TargetAssessment,
    supplied_confirmation_phrase: &str,
    chunk_size_bytes: u64,
    lay_out_files: impl FnOnce(u64) -> Result<UefiFat32Layout, RawWriteError>,
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
        return Err(RawWriteError::TargetBlocked {
            target_index: target.index,
            target_model: target.model.clone(),
            reason: target.reason.clone(),
        });
    }

    // Without Administrator rights the drive cannot be opened. Stopping here
    // keeps the user from answering the final prompt and using up the
    // confirmation for a write that was always going to be refused.
    if target.access == DeviceAccess::MetadataOnly {
        return Err(RawWriteError::NeedsAdministrator {
            target_index: target.index,
        });
    }

    let Some(identity_code) = target.identity_code.clone() else {
        return Err(RawWriteError::Blocked {
            reason: "target identity could not be read".to_string(),
        });
    };

    // The short code only counts while the drive is empty, judged from this
    // fresh look at it rather than from when `--intent` ran.
    let full_phrase = confirmation_phrase(target.index, &identity_code);
    let short_code =
        (target.status == TargetAssessmentStatus::Eligible).then_some(identity_code.as_str());
    let Some(confirmation_form) =
        match_confirmation(Some(&full_phrase), short_code, supplied_confirmation_phrase)
    else {
        if supplied_confirmation_phrase
            .trim()
            .eq_ignore_ascii_case(&identity_code)
        {
            return Err(RawWriteError::ShortConfirmationRefused {
                target_index: target.index,
                reason: target
                    .contents
                    .as_ref()
                    .map(|contents| contents.reason.clone())
                    .unwrap_or_else(|| "its contents could not be checked".to_string()),
            });
        }
        return Err(RawWriteError::ConfirmationMismatch {
            target_index: target.index,
        });
    };

    if selection.status != WriteModeSelectionStatus::Selected {
        return Err(RawWriteError::Blocked {
            reason: selection.reason.clone(),
        });
    }

    let Some(target_size_bytes) = target.size_bytes else {
        return Err(RawWriteError::Blocked {
            reason: "target size is unknown".to_string(),
        });
    };

    let layout = match selection.selected_mode {
        Some(SelectedWriteMode::RawBlockWrite) => {
            if selection.planned_bytes > target_size_bytes {
                return Err(RawWriteError::Blocked {
                    reason: "image is larger than the target device".to_string(),
                });
            }
            WriteLayout::Raw
        }
        Some(SelectedWriteMode::IsoFilesystemBootSetup) => {
            WriteLayout::UefiFat32(lay_out_files(target_size_bytes)?)
        }
        None => {
            return Err(RawWriteError::Blocked {
                reason: "no write mode was selected for this image".to_string(),
            })
        }
    };

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
        confirmation_form,
        layout,
        authorization_expires_at_unix_seconds: 0,
    })
}

/// The drive's sector size, which the FAT32 layout depends on. Read through
/// a read-only handle, so nothing is locked yet.
fn read_sector_size(target_path: &str) -> Result<u32, RawWriteError> {
    use osflux_core::device::{open_read_only, DeviceRead};
    open_read_only(target_path)
        .and_then(|drive| drive.sector_size_bytes())
        .map_err(|error| RawWriteError::Blocked {
            reason: format!("could not read the drive's sector size: {error}"),
        })
}

fn file_copy_error(image_path: &Path, error: FileCopyError) -> RawWriteError {
    match error {
        FileCopyError::Image(source) => RawWriteError::Image {
            path: image_path.to_path_buf(),
            source,
        },
        other => RawWriteError::Blocked {
            reason: other.to_string(),
        },
    }
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
    let confirmed_with = match plan.confirmation_form {
        ConfirmationForm::FullPhrase => "confirmed with the full phrase",
        ConfirmationForm::IdentityCode => {
            "confirmed with the drive's identity code, because the drive was empty"
        }
    };
    let confirmed_with = match &plan.layout {
        WriteLayout::Raw => format!("{confirmed_with}; image copied byte for byte"),
        WriteLayout::UefiFat32(layout) => {
            let mut detail = format!(
                "{confirmed_with}; {} files copied onto a new FAT32 partition labelled {}",
                layout.file_count, layout.label
            );
            for split in &layout.split_files {
                detail.push_str(&format!(
                    "; {} split into {} parts",
                    split.path,
                    split.parts.len()
                ));
            }
            detail
        }
    };
    audit
        .record(
            &AuditEvent::for_plan(AuditEventKind::WriteStarted, plan).with_detail(confirmed_with),
        )
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

/// Writes the planned bytes to `device` chunk by chunk, flushes, then reads
/// every byte back and compares it with the same bytes generated again.
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

    // The short code was accepted because the drive was empty. It is locked
    // and dismounted now, so nothing can be added any more: this is the last
    // moment to check, and nothing has been written yet.
    if plan.confirmation_form == ConfirmationForm::IdentityCode {
        let contents = inspect_drive_contents(device);
        if !contents.state.allows_short_confirmation() {
            return Err(RawWriteError::ShortConfirmationRefused {
                target_index: plan.target_index,
                reason: contents.reason,
            });
        }
    }

    let image_error = |source| RawWriteError::Image {
        path: plan.image_path.clone(),
        source,
    };
    let image = File::open(&plan.image_path).map_err(image_error)?;
    let image_size = image.metadata().map_err(image_error)?.len();
    if image_size != plan.image_size_bytes {
        return Err(RawWriteError::ImageChanged {
            expected_bytes: plan.image_size_bytes,
            actual_bytes: image_size,
        });
    }

    let stream = WriteStream {
        device,
        device_size,
        chunk_size: plan.chunk_size_bytes as usize,
    };
    let (image_sha256, written) = match &plan.layout {
        WriteLayout::Raw => {
            // Hashed while writing so the report names exactly what went
            // onto the drive, without a second pass over the image.
            let mut image_hash = Sha256::new();
            let written = stream.write_and_verify(
                &[(0, image_size)],
                |offset, buffer| image.read_image_at(offset, buffer).map_err(image_error),
                |bytes| image_hash.update(bytes),
                &mut on_progress,
            )?;
            (hex(&image_hash.finalize()), written)
        }
        WriteLayout::UefiFat32(layout) => {
            let sector_bytes = device
                .logical_sector_size()
                .map_err(RawWriteError::Device)?;
            if sector_bytes != layout.sector_bytes {
                return Err(RawWriteError::TargetChanged {
                    reason: format!(
                        "the opened drive uses {sector_bytes}-byte sectors but the plan was made for {}-byte sectors",
                        layout.sector_bytes
                    ),
                });
            }

            let image_sha256 = hash_image(&image, image_size, stream.chunk_size, &mut on_progress)
                .map_err(image_error)?;
            let copy = prepare_copy(&plan.image_path, device_size, layout)
                .map_err(|error| file_copy_error(&plan.image_path, error))?;
            let image_bytes = copy.image.image_len();
            if image_bytes != layout.image_bytes {
                return Err(RawWriteError::ImageChanged {
                    expected_bytes: layout.image_bytes,
                    actual_bytes: image_bytes,
                });
            }

            let mut regions = vec![(0, image_bytes)];
            regions.extend(trailing_wipe(image_bytes, device_size));
            let written = stream.write_and_verify(
                &regions,
                |offset, buffer| {
                    // The wipe region lies past the image and is all zeros.
                    if offset >= image_bytes {
                        buffer.fill(0);
                        Ok(())
                    } else {
                        copy.read_at(offset, buffer).map_err(image_error)
                    }
                },
                |_| {},
                &mut on_progress,
            )?;
            (image_sha256, written)
        }
    };

    Ok(RawWriteReport {
        session_id: plan.session_id.clone(),
        image_path: plan.image_path.display().to_string(),
        target_path: plan.target_path.clone(),
        target_identity_code: plan.target_identity_code.clone(),
        image_sha256,
        bytes_written: written.bytes,
        bytes_verified: written.bytes,
        write_millis: written.write_millis,
        verify_millis: written.verify_millis,
        audit_recorded: false,
    })
}

/// The part at the end of the drive that is cleared after copying files:
/// the last MiB, or less when the image itself reaches into it.
fn trailing_wipe(image_bytes: u64, device_size: u64) -> Option<(u64, u64)> {
    let image_end = image_bytes.div_ceil(RAW_WRITE_ALIGNMENT_BYTES) * RAW_WRITE_ALIGNMENT_BYTES;
    let tail = device_size.saturating_sub(TRAILING_WIPE_BYTES) / RAW_WRITE_ALIGNMENT_BYTES
        * RAW_WRITE_ALIGNMENT_BYTES;
    let start = image_end.max(tail);
    (start < device_size).then_some((start, device_size - start))
}

/// Reads the whole image once to report its SHA-256.
fn hash_image(
    image: &File,
    image_size: u64,
    chunk_size: usize,
    on_progress: &mut impl FnMut(RawWriteProgress),
) -> std::io::Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; chunk_size];
    let mut reader = image;
    let mut offset = 0u64;
    report(on_progress, RawWritePhase::Hashing, 0, image_size);
    while offset < image_size {
        let length = chunk_length(chunk_size, offset, image_size);
        reader.read_exact(&mut buffer[..length])?;
        hash.update(&buffer[..length]);
        offset += length as u64;
        report(on_progress, RawWritePhase::Hashing, offset, image_size);
    }
    Ok(hex(&hash.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

struct Written {
    bytes: u64,
    write_millis: u64,
    verify_millis: u64,
}

/// Writes regions of a drive from bytes that can be generated more than
/// once, then reads them back and compares.
struct WriteStream<'a, D: BlockDevice> {
    device: &'a D,
    device_size: u64,
    chunk_size: usize,
}

impl<D: BlockDevice> WriteStream<'_, D> {
    /// `regions` are `(start, length)` pairs in drive order; `fill` produces
    /// the bytes for any piece of them; `on_written` sees each piece as it is
    /// written.
    fn write_and_verify(
        &self,
        regions: &[(u64, u64)],
        mut fill: impl FnMut(u64, &mut [u8]) -> Result<(), RawWriteError>,
        mut on_written: impl FnMut(&[u8]),
        on_progress: &mut impl FnMut(RawWriteProgress),
    ) -> Result<Written, RawWriteError> {
        let total: u64 = regions.iter().map(|(_, length)| length).sum();
        let mut buffer = vec![0u8; self.chunk_size];

        let write_started = Instant::now();
        let mut done = 0u64;
        report(on_progress, RawWritePhase::Writing, 0, total);
        for &(start, length) in regions {
            let end = start + length;
            let mut offset = start;
            while offset < end {
                let piece = chunk_length(self.chunk_size, offset, end);
                fill(offset, &mut buffer[..piece])?;
                on_written(&buffer[..piece]);
                let padded = padded_length(piece, offset, self.device_size);
                buffer[piece..padded].fill(0);
                self.device
                    .write_at(offset, &buffer[..padded])
                    .map_err(RawWriteError::Device)?;
                offset += piece as u64;
                done += piece as u64;
                report(on_progress, RawWritePhase::Writing, done, total);
            }
        }
        self.device.flush().map_err(RawWriteError::Device)?;
        let write_millis = write_started.elapsed().as_millis() as u64;

        let verify_started = Instant::now();
        let mut device_buffer = vec![0u8; self.chunk_size];
        done = 0;
        report(on_progress, RawWritePhase::Verifying, 0, total);
        for &(start, length) in regions {
            let end = start + length;
            let mut offset = start;
            while offset < end {
                let piece = chunk_length(self.chunk_size, offset, end);
                fill(offset, &mut buffer[..piece])?;
                let padded = padded_length(piece, offset, self.device_size);
                self.device
                    .read_at(offset, &mut device_buffer[..padded])
                    .map_err(RawWriteError::Device)?;
                if let Some(position) = first_difference(&buffer[..piece], &device_buffer[..piece])
                {
                    return Err(RawWriteError::VerifyMismatch {
                        offset: offset + position as u64,
                    });
                }
                offset += piece as u64;
                done += piece as u64;
                report(on_progress, RawWritePhase::Verifying, done, total);
            }
        }
        let verify_millis = verify_started.elapsed().as_millis() as u64;

        Ok(Written {
            bytes: total,
            write_millis,
            verify_millis,
        })
    }
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

fn chunk_length(chunk_size: usize, offset: u64, end: u64) -> usize {
    (chunk_size as u64).min(end - offset) as usize
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
        build_raw_write_plan, execute_raw_write, trailing_wipe, write_image_to_target,
        RawWriteError, RawWritePhase, RawWritePlan, RawWriteProgress, WriteLayout,
        RAW_WRITE_ALIGNMENT_BYTES,
    };
    use crate::audit::AuditLog;
    use crate::clock::now_unix_seconds;
    use crate::file_copy::UefiFat32Layout;
    use crate::image_plan::{ImagePlanStatus, ImageWriteStrategy};
    use crate::target::{TargetAssessment, TargetAssessmentStatus};
    use crate::write_authorization::{AuthorizationStore, ImageIdentity, WriteAuthorization};
    use crate::write_intent::ConfirmationForm;
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
    fn writes_to_a_drive_confirmed_with_its_identity_code_while_it_is_still_empty() {
        let image = image_bytes(8192);
        let path = temp_image(&image);
        let drive = MemoryDrive::new(8 * MIB);
        drive.data.borrow_mut().fill(0);
        let mut short = plan(&path, image.len() as u64, 8 * MIB);
        short.confirmation_form = ConfirmationForm::IdentityCode;

        let report = execute_raw_write(&short, &drive, |_| {}).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(report.bytes_written, image.len() as u64);
        assert_eq!(&drive.data.borrow()[..image.len()], &image[..]);
    }

    #[test]
    fn stops_before_writing_when_a_drive_confirmed_with_its_code_is_no_longer_empty() {
        let image = image_bytes(8192);
        let path = temp_image(&image);
        // MemoryDrive starts filled with 0xEE, which is not an empty drive.
        let drive = MemoryDrive::new(8 * MIB);
        let mut short = plan(&path, image.len() as u64, 8 * MIB);
        short.confirmation_form = ConfirmationForm::IdentityCode;

        let error = execute_raw_write(&short, &drive, |_| {}).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(
            matches!(
                error,
                RawWriteError::ShortConfirmationRefused {
                    target_index: 2,
                    ..
                }
            ),
            "{error}"
        );
        assert!(drive.writes.borrow().is_empty(), "nothing may be written");
        assert!(drive.data.borrow().iter().all(|&byte| byte == 0xEE));
    }

    #[test]
    fn the_full_phrase_writes_to_a_drive_that_holds_data() {
        let image = image_bytes(8192);
        let path = temp_image(&image);
        let drive = MemoryDrive::new(8 * MIB);

        let report =
            execute_raw_write(&plan(&path, image.len() as u64, 8 * MIB), &drive, |_| {}).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(report.bytes_written, image.len() as u64);
    }

    #[test]
    fn accepts_the_identity_code_for_an_empty_drive() {
        let mut empty = target(Some("7F3A"));
        empty.status = TargetAssessmentStatus::Eligible;

        let plan = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &empty,
            " 7f3a ",
            4 * MIB,
            no_file_copy,
        )
        .unwrap();

        assert_eq!(plan.confirmation_form, ConfirmationForm::IdentityCode);
    }

    #[test]
    fn refuses_the_identity_code_for_a_drive_that_holds_data() {
        let mut with_data = target(Some("7F3A"));
        with_data.contents = Some(crate::drive_contents::DriveContents {
            state: crate::drive_contents::DriveContentsState::HasData,
            reason: "the FAT32 partition's root folder holds \"Holiday Photo.jpg\"".to_string(),
            partition_scheme: None,
            partitions: Vec::new(),
        });

        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &with_data,
            "7F3A",
            4 * MIB,
            no_file_copy,
        )
        .unwrap_err();

        assert!(
            matches!(
                &error,
                RawWriteError::ShortConfirmationRefused {
                    target_index: 2,
                    ..
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains("Holiday Photo.jpg"), "{error}");
        assert!(error.to_string().contains("full phrase"), "{error}");
    }

    #[test]
    fn the_full_phrase_still_works_for_an_empty_drive() {
        let mut empty = target(Some("7F3A"));
        empty.status = TargetAssessmentStatus::Eligible;

        let plan = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &empty,
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
            no_file_copy,
        )
        .unwrap();

        assert_eq!(plan.confirmation_form, ConfirmationForm::FullPhrase);
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
                short_confirmation: None,
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
            short_confirmation: None,
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
            no_file_copy,
        )
        .unwrap();

        assert_eq!(plan.target_path, "\\\\.\\PhysicalDrive2");
        assert_eq!(plan.target_identity_code, "7F3A");
        assert_eq!(plan.image_size_bytes, 6 * MIB);
        assert_eq!(plan.target_size_bytes, 16 * MIB);
    }

    /// For raw images the file copy step must never run.
    fn no_file_copy(_: u64) -> Result<UefiFat32Layout, RawWriteError> {
        panic!("the file copy step ran for a write that does not copy files")
    }

    fn sample_layout() -> UefiFat32Layout {
        UefiFat32Layout {
            label: "CCCOMA_X64F".to_string(),
            file_count: 3,
            file_bytes: 10,
            largest_file: Some(("setup.exe".to_string(), 8)),
            split_files: Vec::new(),
            image_bytes: 4 * MIB,
            sector_bytes: 512,
            cluster_bytes: 4096,
            volume_id: 1,
            disk_signature: 3,
            created_unix_seconds: 0,
        }
    }

    #[test]
    fn plans_a_file_copy_for_an_iso_that_needs_one() {
        let mut asked_for = None;

        let plan = build_raw_write_plan(
            Path::new("windows.iso"),
            &selection(Some(SelectedWriteMode::IsoFilesystemBootSetup)),
            &target(Some("7F3A")),
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
            |drive_bytes| {
                asked_for = Some(drive_bytes);
                Ok(sample_layout())
            },
        )
        .unwrap();

        assert_eq!(asked_for, Some(16 * MIB));
        assert_eq!(plan.layout, WriteLayout::UefiFat32(sample_layout()));
    }

    #[test]
    fn reports_a_file_copy_problem_only_after_the_other_checks() {
        let mut blocked = target(Some("7F3A"));
        blocked.status = TargetAssessmentStatus::Blocked;
        blocked.reason = "target contains the system volume".to_string();

        let error = build_raw_write_plan(
            Path::new("windows.iso"),
            &selection(Some(SelectedWriteMode::IsoFilesystemBootSetup)),
            &blocked,
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
            no_file_copy,
        )
        .unwrap_err();
        assert!(
            matches!(error, RawWriteError::TargetBlocked { .. }),
            "{error}"
        );

        let error = build_raw_write_plan(
            Path::new("windows.iso"),
            &selection(Some(SelectedWriteMode::IsoFilesystemBootSetup)),
            &target(Some("7F3A")),
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
            |_| {
                Err(RawWriteError::Blocked {
                    reason: "sources/install.wim is 5.00 GiB".to_string(),
                })
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("install.wim"), "{error}");
    }

    #[test]
    fn copies_an_isos_files_onto_a_uefi_fat32_drive_and_verifies_them() {
        use crate::drive_contents::{inspect_drive_contents, FilesystemKind};
        use sha2::{Digest, Sha256};

        let iso = crate::file_copy::tests::windows_like_iso();
        let drive_bytes = 64 * MIB;
        let layout = crate::file_copy::lay_out_uefi_fat32(
            &iso,
            drive_bytes,
            512,
            "OSFLUX-SESSION-TEST",
            1_795_022_016,
        )
        .unwrap();
        let iso_bytes = fs::read(&iso).unwrap();
        let mut copy_plan = plan(&iso, iso_bytes.len() as u64, drive_bytes);
        copy_plan.layout = WriteLayout::UefiFat32(layout.clone());
        // Starts filled with 0xEE, so untouched bytes stand out.
        let drive = MemoryDrive::new(drive_bytes);
        let mut phases = Vec::new();

        let report = execute_raw_write(&copy_plan, &drive, |event| {
            if phases.last() != Some(&event.phase) {
                phases.push(event.phase);
            }
        })
        .unwrap();
        let _ = fs::remove_file(&iso);

        let expected_hash: String = Sha256::digest(&iso_bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(report.image_sha256, expected_hash);
        assert_eq!(report.bytes_written, layout.image_bytes + MIB);
        assert_eq!(
            phases,
            [
                RawWritePhase::Hashing,
                RawWritePhase::Writing,
                RawWritePhase::Verifying
            ]
        );

        let contents = inspect_drive_contents(&drive);
        assert_eq!(contents.partitions.len(), 1);
        assert_eq!(
            contents.partitions[0].filesystem,
            Some(FilesystemKind::Fat32)
        );
        assert_eq!(contents.partitions[0].label.as_deref(), Some("CCCOMA_X64F"));
        assert!(
            contents.reason.contains("autorun.inf"),
            "{}",
            contents.reason
        );

        let data = drive.data.borrow();
        let last_mib = (drive_bytes - MIB) as usize;
        assert!(
            data[last_mib..].iter().all(|&byte| byte == 0),
            "last MiB cleared"
        );
        let free_space = layout.image_bytes.div_ceil(4096) as usize * 4096 + 4096;
        assert!(free_space < last_mib);
        assert_eq!(data[free_space], 0xEE, "free space is not written");
    }

    #[test]
    fn refuses_a_file_copy_when_the_drive_reports_other_sectors() {
        let iso = crate::file_copy::tests::windows_like_iso();
        let mut layout = crate::file_copy::lay_out_uefi_fat32(&iso, 64 * MIB, 512, "S", 0).unwrap();
        layout.sector_bytes = 4096;
        let size = fs::metadata(&iso).unwrap().len();
        let mut copy_plan = plan(&iso, size, 64 * MIB);
        copy_plan.layout = WriteLayout::UefiFat32(layout);
        let drive = MemoryDrive::new(64 * MIB);

        let error = execute_raw_write(&copy_plan, &drive, |_| {}).unwrap_err();
        let _ = fs::remove_file(&iso);

        assert!(
            matches!(error, RawWriteError::TargetChanged { .. }),
            "{error}"
        );
        assert!(drive.writes.borrow().is_empty());
    }

    #[test]
    fn refuses_a_file_copy_that_no_longer_matches_its_plan() {
        let iso = crate::file_copy::tests::windows_like_iso();
        let mut layout = crate::file_copy::lay_out_uefi_fat32(&iso, 64 * MIB, 512, "S", 0).unwrap();
        layout.image_bytes += 4096;
        let size = fs::metadata(&iso).unwrap().len();
        let mut copy_plan = plan(&iso, size, 64 * MIB);
        copy_plan.layout = WriteLayout::UefiFat32(layout);
        let drive = MemoryDrive::new(64 * MIB);

        let error = execute_raw_write(&copy_plan, &drive, |_| {}).unwrap_err();
        let _ = fs::remove_file(&iso);

        assert!(
            matches!(error, RawWriteError::ImageChanged { .. }),
            "{error}"
        );
        assert!(drive.writes.borrow().is_empty());
    }

    #[test]
    fn clears_the_last_mib_after_the_image_without_overlapping_it() {
        assert_eq!(trailing_wipe(4 * MIB, 64 * MIB), Some((63 * MIB, MIB)));
        // An image reaching into the last MiB: only what follows it.
        assert_eq!(
            trailing_wipe(63 * MIB + 5000, 64 * MIB),
            Some((63 * MIB + 8192, MIB - 8192))
        );
        assert_eq!(trailing_wipe(64 * MIB, 64 * MIB), None);
    }

    #[test]
    fn refuses_when_a_different_drive_now_holds_the_index() {
        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &target(Some("0B21")),
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
            no_file_copy,
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
            no_file_copy,
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
        external_disk.model = Some("SAMSUNG SSD PM830 USB Device".to_string());
        external_disk.status = TargetAssessmentStatus::Blocked;
        external_disk.reason = "target is an external hard disk or SSD".to_string();

        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &external_disk,
            "OSFLUX ERASE TARGET 2 XXXX",
            4 * MIB,
            no_file_copy,
        )
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "write blocked: drive 2 is SAMSUNG SSD PM830 USB Device, and target is an external hard disk or SSD"
        );
        assert!(matches!(
            error,
            RawWriteError::TargetBlocked {
                target_index: 2,
                ..
            }
        ));
    }

    #[test]
    fn refuses_before_the_prompt_without_administrator_rights() {
        let mut metadata_only = target(Some("7F3A"));
        metadata_only.access = DeviceAccess::MetadataOnly;

        let error = build_raw_write_plan(
            Path::new("disk.img"),
            &selection(Some(SelectedWriteMode::RawBlockWrite)),
            &metadata_only,
            "OSFLUX ERASE TARGET 2 7F3A",
            4 * MIB,
            no_file_copy,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RawWriteError::NeedsAdministrator { target_index: 2 }
        ));
        assert!(error.to_string().contains("not used up"));
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
            no_file_copy,
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
                no_file_copy,
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
            confirmation_form: ConfirmationForm::FullPhrase,
            layout: WriteLayout::Raw,
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
            contents: None,
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
