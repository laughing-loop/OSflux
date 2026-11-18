use std::fmt;
use std::path::Path;

use crate::preflight::WritePreflightError;
use crate::write_command::WriteCommandStatus;
use crate::write_session::{prepare_write_session, WriteSession, WriteSessionStatus};

pub const DEFAULT_CHUNK_SIZE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPlan {
    pub status: ChunkPlanStatus,
    pub reason: String,
    pub session_id: String,
    pub session_status: WriteSessionStatus,
    pub image_source: String,
    pub target_index: u32,
    pub target_path: String,
    pub image_size_bytes: u64,
    pub chunk_size_bytes: u64,
    pub chunk_count: u64,
    pub final_chunk_size_bytes: u64,
    pub planned_bytes: u64,
    pub dry_run: bool,
    pub write_access_attempted: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl ChunkPlan {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"sessionId\": \"{}\",\n",
                "  \"sessionStatus\": \"{}\",\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"targetIndex\": {},\n",
                "  \"targetPath\": \"{}\",\n",
                "  \"imageSizeBytes\": {},\n",
                "  \"chunkSizeBytes\": {},\n",
                "  \"chunkCount\": {},\n",
                "  \"finalChunkSizeBytes\": {},\n",
                "  \"plannedBytes\": {},\n",
                "  \"dryRun\": {},\n",
                "  \"writeAccessAttempted\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            escape_json_string(&self.reason),
            escape_json_string(&self.session_id),
            self.session_status.as_str(),
            escape_json_string(&self.image_source),
            self.target_index,
            escape_json_string(&self.target_path),
            self.image_size_bytes,
            self.chunk_size_bytes,
            self.chunk_count,
            self.final_chunk_size_bytes,
            self.planned_bytes,
            self.dry_run,
            self.write_access_attempted,
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkPlanStatus {
    Planned,
    Blocked,
}

impl ChunkPlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for ChunkPlanStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub enum ChunkPlanError {
    Session(WritePreflightError),
    ImageMetadata {
        path: String,
        source: std::io::Error,
    },
    InvalidChunkSize,
}

impl fmt::Display for ChunkPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(error) => write!(formatter, "write session planning failed: {error}"),
            Self::ImageMetadata { path, source } => {
                write!(
                    formatter,
                    "could not read image metadata for {path}: {source}"
                )
            }
            Self::InvalidChunkSize => write!(formatter, "chunk size must be greater than zero"),
        }
    }
}

impl std::error::Error for ChunkPlanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Session(error) => Some(error),
            Self::ImageMetadata { source, .. } => Some(source),
            Self::InvalidChunkSize => None,
        }
    }
}

pub fn plan_write_chunks(
    image_path: impl AsRef<Path>,
    target_index: u32,
    confirmation_phrase: Option<&str>,
    chunk_size_bytes: u64,
) -> Result<ChunkPlan, ChunkPlanError> {
    if chunk_size_bytes == 0 {
        return Err(ChunkPlanError::InvalidChunkSize);
    }

    let image_path = image_path.as_ref();
    let session = prepare_write_session(image_path, target_index, confirmation_phrase)
        .map_err(ChunkPlanError::Session)?;
    let image_size_bytes = std::fs::metadata(image_path)
        .map_err(|source| ChunkPlanError::ImageMetadata {
            path: image_path.display().to_string(),
            source,
        })?
        .len();

    Ok(build_chunk_plan(
        &session,
        image_size_bytes,
        chunk_size_bytes,
    ))
}

pub fn build_chunk_plan(
    session: &WriteSession,
    image_size_bytes: u64,
    chunk_size_bytes: u64,
) -> ChunkPlan {
    let status = classify_chunk_plan(session, image_size_bytes, chunk_size_bytes);
    let chunk_count = if status == ChunkPlanStatus::Planned {
        calculate_chunk_count(image_size_bytes, chunk_size_bytes)
    } else {
        0
    };
    let final_chunk_size_bytes =
        calculate_final_chunk_size(image_size_bytes, chunk_size_bytes, chunk_count);
    let planned_bytes = if status == ChunkPlanStatus::Planned {
        image_size_bytes
    } else {
        0
    };

    let mut notes = Vec::new();
    notes.push("Chunk plan is metadata-only; no target writes were attempted.".to_string());
    notes.push("Chunk offsets are logical write offsets starting at byte 0.".to_string());
    notes.push(
        "Future write execution can use this plan for progress accounting and scheduling."
            .to_string(),
    );

    let mut warnings = session.warnings.clone();
    if status == ChunkPlanStatus::Planned && final_chunk_size_bytes < chunk_size_bytes {
        warnings.push("Final chunk is smaller than the configured chunk size.".to_string());
    }

    ChunkPlan {
        status,
        reason: chunk_plan_reason(session, image_size_bytes, chunk_size_bytes).to_string(),
        session_id: session.session_id.clone(),
        session_status: session.status,
        image_source: session.image_source.clone(),
        target_index: session.target_index,
        target_path: session.target_path.clone(),
        image_size_bytes,
        chunk_size_bytes,
        chunk_count,
        final_chunk_size_bytes,
        planned_bytes,
        dry_run: true,
        write_access_attempted: false,
        notes,
        warnings,
    }
}

fn classify_chunk_plan(
    session: &WriteSession,
    image_size_bytes: u64,
    chunk_size_bytes: u64,
) -> ChunkPlanStatus {
    if session.status != WriteSessionStatus::Prepared {
        return ChunkPlanStatus::Blocked;
    }

    if image_size_bytes == 0 || chunk_size_bytes == 0 {
        return ChunkPlanStatus::Blocked;
    }

    ChunkPlanStatus::Planned
}

fn chunk_plan_reason(
    session: &WriteSession,
    image_size_bytes: u64,
    chunk_size_bytes: u64,
) -> &'static str {
    if session.status != WriteSessionStatus::Prepared {
        return match session.command_status {
            WriteCommandStatus::ConfirmationMissing => {
                "chunk plan blocked because write confirmation is missing"
            }
            WriteCommandStatus::ConfirmationMismatch => {
                "chunk plan blocked because write confirmation did not match"
            }
            WriteCommandStatus::Blocked => "chunk plan blocked by write command planning",
            WriteCommandStatus::ReadyDryRun => "chunk plan blocked by write session status",
        };
    }

    if image_size_bytes == 0 {
        return "chunk plan blocked because image has no bytes";
    }

    if chunk_size_bytes == 0 {
        return "chunk plan blocked because chunk size is zero";
    }

    "chunk plan prepared from locked write session"
}

fn calculate_chunk_count(image_size_bytes: u64, chunk_size_bytes: u64) -> u64 {
    if image_size_bytes == 0 || chunk_size_bytes == 0 {
        return 0;
    }

    image_size_bytes.div_ceil(chunk_size_bytes)
}

fn calculate_final_chunk_size(
    image_size_bytes: u64,
    chunk_size_bytes: u64,
    chunk_count: u64,
) -> u64 {
    if chunk_count == 0 {
        return 0;
    }

    let remainder = image_size_bytes % chunk_size_bytes;
    if remainder == 0 {
        chunk_size_bytes
    } else {
        remainder
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
    use super::{build_chunk_plan, ChunkPlanStatus};
    use crate::write_command::WriteCommandStatus;
    use crate::write_session::{WriteSession, WriteSessionStatus};

    #[test]
    fn plans_chunks_for_prepared_session() {
        let plan = build_chunk_plan(
            &session(WriteSessionStatus::Prepared),
            10 * 1024 * 1024,
            4 * 1024 * 1024,
        );

        assert_eq!(plan.status, ChunkPlanStatus::Planned);
        assert_eq!(plan.chunk_count, 3);
        assert_eq!(plan.final_chunk_size_bytes, 2 * 1024 * 1024);
        assert_eq!(plan.planned_bytes, 10 * 1024 * 1024);
        assert!(!plan.write_access_attempted);
    }

    #[test]
    fn uses_full_final_chunk_for_exact_division() {
        let plan = build_chunk_plan(
            &session(WriteSessionStatus::Prepared),
            8 * 1024 * 1024,
            4 * 1024 * 1024,
        );

        assert_eq!(plan.chunk_count, 2);
        assert_eq!(plan.final_chunk_size_bytes, 4 * 1024 * 1024);
    }

    #[test]
    fn blocks_when_session_is_blocked() {
        let plan = build_chunk_plan(
            &session(WriteSessionStatus::Blocked),
            10 * 1024 * 1024,
            4 * 1024 * 1024,
        );

        assert_eq!(plan.status, ChunkPlanStatus::Blocked);
        assert_eq!(plan.chunk_count, 0);
        assert_eq!(plan.final_chunk_size_bytes, 0);
        assert_eq!(plan.planned_bytes, 0);
    }

    #[test]
    fn blocks_empty_images() {
        let plan = build_chunk_plan(&session(WriteSessionStatus::Prepared), 0, 4 * 1024 * 1024);

        assert_eq!(plan.status, ChunkPlanStatus::Blocked);
        assert_eq!(plan.chunk_count, 0);
        assert_eq!(plan.final_chunk_size_bytes, 0);
    }

    #[test]
    fn serializes_chunk_plan_as_json() {
        let plan = build_chunk_plan(
            &session(WriteSessionStatus::Prepared),
            10 * 1024 * 1024,
            4 * 1024 * 1024,
        );
        let json = plan.to_json_pretty();

        assert!(json.contains("\"status\": \"planned\""));
        assert!(json.contains("\"chunkCount\": 3"));
        assert!(json.contains("\"writeAccessAttempted\": false"));
    }

    fn session(status: WriteSessionStatus) -> WriteSession {
        WriteSession {
            session_id: "OSFLUX-SESSION-1700000000-2-TEST".to_string(),
            status,
            reason: status.as_str().to_string(),
            created_at_unix_seconds: 1_700_000_000,
            image_source: "C:\\images\\sample.iso".to_string(),
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            target_identity_code: Some("7F3A".to_string()),
            command_status: if status == WriteSessionStatus::Prepared {
                WriteCommandStatus::ReadyDryRun
            } else {
                WriteCommandStatus::ConfirmationMissing
            },
            confirmed: status == WriteSessionStatus::Prepared,
            locked: status == WriteSessionStatus::Prepared,
            dry_run: true,
            write_access_attempted: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
