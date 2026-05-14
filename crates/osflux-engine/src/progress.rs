use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use crate::chunk_plan::{plan_write_chunks, ChunkPlan, ChunkPlanError, ChunkPlanStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressPreview {
    pub status: ProgressPreviewStatus,
    pub reason: String,
    pub session_id: String,
    pub chunk_plan_status: ChunkPlanStatus,
    pub total_chunks: u64,
    pub total_bytes: u64,
    pub event_count: u64,
    pub preview_events: Vec<ProgressEvent>,
    pub dry_run: bool,
    pub write_access_attempted: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl ProgressPreview {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"sessionId\": \"{}\",\n",
                "  \"chunkPlanStatus\": \"{}\",\n",
                "  \"totalChunks\": {},\n",
                "  \"totalBytes\": {},\n",
                "  \"eventCount\": {},\n",
                "  \"previewEvents\": {},\n",
                "  \"dryRun\": {},\n",
                "  \"writeAccessAttempted\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            escape_json_string(&self.reason),
            escape_json_string(&self.session_id),
            self.chunk_plan_status.as_str(),
            self.total_chunks,
            self.total_bytes,
            self.event_count,
            progress_events_json(&self.preview_events),
            self.dry_run,
            self.write_access_attempted,
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEvent {
    pub label: String,
    pub completed_chunks: u64,
    pub completed_bytes: u64,
    pub percent_complete_basis_points: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressPreviewStatus {
    Ready,
    Blocked,
}

impl ProgressPreviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for ProgressPreviewStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn preview_write_progress(
    image_path: impl AsRef<Path>,
    target_index: u32,
    confirmation_phrase: Option<&str>,
    chunk_size_bytes: u64,
) -> Result<ProgressPreview, ChunkPlanError> {
    let chunk_plan = plan_write_chunks(
        image_path,
        target_index,
        confirmation_phrase,
        chunk_size_bytes,
    )?;

    Ok(build_progress_preview(&chunk_plan))
}

pub fn build_progress_preview(chunk_plan: &ChunkPlan) -> ProgressPreview {
    let status = classify_progress_preview(chunk_plan);
    let preview_events = if status == ProgressPreviewStatus::Ready {
        build_preview_events(chunk_plan)
    } else {
        Vec::new()
    };
    let event_count = if status == ProgressPreviewStatus::Ready {
        chunk_plan.chunk_count.saturating_add(1)
    } else {
        0
    };

    let mut notes = Vec::new();
    notes.push("Progress preview is metadata-only; no target writes were attempted.".to_string());
    notes.push(
        "Progress percentages use integer basis points where 10000 means 100.00%.".to_string(),
    );
    notes.push(
        "Preview events are sampled from the full event stream to avoid huge output.".to_string(),
    );

    let mut warnings = chunk_plan.warnings.clone();
    if status == ProgressPreviewStatus::Ready && preview_events.len() as u64 != event_count {
        warnings
            .push("Preview events are sampled; full progress stream has more events.".to_string());
    }

    ProgressPreview {
        status,
        reason: progress_reason(chunk_plan).to_string(),
        session_id: chunk_plan.session_id.clone(),
        chunk_plan_status: chunk_plan.status,
        total_chunks: chunk_plan.chunk_count,
        total_bytes: chunk_plan.planned_bytes,
        event_count,
        preview_events,
        dry_run: true,
        write_access_attempted: false,
        notes,
        warnings,
    }
}

fn classify_progress_preview(chunk_plan: &ChunkPlan) -> ProgressPreviewStatus {
    if chunk_plan.status != ChunkPlanStatus::Planned {
        return ProgressPreviewStatus::Blocked;
    }

    if chunk_plan.chunk_count == 0 || chunk_plan.planned_bytes == 0 {
        return ProgressPreviewStatus::Blocked;
    }

    ProgressPreviewStatus::Ready
}

fn progress_reason(chunk_plan: &ChunkPlan) -> &'static str {
    if chunk_plan.status != ChunkPlanStatus::Planned {
        return "progress preview blocked because chunk plan is not ready";
    }

    if chunk_plan.chunk_count == 0 || chunk_plan.planned_bytes == 0 {
        return "progress preview blocked because chunk plan has no work";
    }

    "progress preview prepared from chunk plan"
}

fn build_preview_events(chunk_plan: &ChunkPlan) -> Vec<ProgressEvent> {
    let mut completed_chunks = BTreeSet::new();
    completed_chunks.insert(0);
    completed_chunks.insert(chunk_plan.chunk_count);

    if chunk_plan.chunk_count >= 1 {
        completed_chunks.insert(1);
    }

    if chunk_plan.chunk_count > 2 {
        completed_chunks.insert(chunk_plan.chunk_count / 2);
        completed_chunks.insert(chunk_plan.chunk_count.saturating_sub(1));
    }

    completed_chunks
        .into_iter()
        .map(|completed_chunks| progress_event(chunk_plan, completed_chunks))
        .collect()
}

fn progress_event(chunk_plan: &ChunkPlan, completed_chunks: u64) -> ProgressEvent {
    let completed_bytes = completed_chunks
        .saturating_mul(chunk_plan.chunk_size_bytes)
        .min(chunk_plan.planned_bytes);

    ProgressEvent {
        label: progress_label(completed_chunks, chunk_plan.chunk_count),
        completed_chunks,
        completed_bytes,
        percent_complete_basis_points: percent_basis_points(
            completed_bytes,
            chunk_plan.planned_bytes,
        ),
    }
}

fn progress_label(completed_chunks: u64, total_chunks: u64) -> String {
    if completed_chunks == 0 {
        "start".to_string()
    } else if completed_chunks == total_chunks {
        "complete".to_string()
    } else {
        format!("chunk-{completed_chunks}-complete")
    }
}

fn percent_basis_points(completed_bytes: u64, total_bytes: u64) -> u32 {
    if total_bytes == 0 {
        return 0;
    }

    ((u128::from(completed_bytes) * 10_000) / u128::from(total_bytes)) as u32
}

fn progress_events_json(events: &[ProgressEvent]) -> String {
    let values = events
        .iter()
        .map(|event| {
            format!(
                concat!(
                    "{{",
                    "\"label\":\"{}\",",
                    "\"completedChunks\":{},",
                    "\"completedBytes\":{},",
                    "\"percentCompleteBasisPoints\":{}",
                    "}}"
                ),
                escape_json_string(&event.label),
                event.completed_chunks,
                event.completed_bytes,
                event.percent_complete_basis_points,
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
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
    use super::{build_progress_preview, ProgressPreviewStatus};
    use crate::chunk_plan::{ChunkPlan, ChunkPlanStatus};
    use crate::write_session::WriteSessionStatus;

    #[test]
    fn builds_progress_preview_from_planned_chunks() {
        let preview = build_progress_preview(&chunk_plan(
            ChunkPlanStatus::Planned,
            10 * 1024 * 1024,
            4 * 1024 * 1024,
            3,
        ));

        assert_eq!(preview.status, ProgressPreviewStatus::Ready);
        assert_eq!(preview.total_chunks, 3);
        assert_eq!(preview.total_bytes, 10 * 1024 * 1024);
        assert_eq!(preview.event_count, 4);
        assert_eq!(preview.preview_events.first().unwrap().label, "start");
        assert_eq!(preview.preview_events.last().unwrap().label, "complete");
        assert!(!preview.write_access_attempted);
    }

    #[test]
    fn calculates_integer_percentages_from_bytes() {
        let preview = build_progress_preview(&chunk_plan(
            ChunkPlanStatus::Planned,
            10 * 1024 * 1024,
            4 * 1024 * 1024,
            3,
        ));
        let first_chunk = preview
            .preview_events
            .iter()
            .find(|event| event.completed_chunks == 1)
            .unwrap();

        assert_eq!(first_chunk.percent_complete_basis_points, 4_000);
    }

    #[test]
    fn blocks_when_chunk_plan_is_blocked() {
        let preview =
            build_progress_preview(&chunk_plan(ChunkPlanStatus::Blocked, 0, 4 * 1024 * 1024, 0));

        assert_eq!(preview.status, ProgressPreviewStatus::Blocked);
        assert_eq!(preview.event_count, 0);
        assert!(preview.preview_events.is_empty());
    }

    #[test]
    fn serializes_progress_preview_as_json() {
        let preview = build_progress_preview(&chunk_plan(
            ChunkPlanStatus::Planned,
            10 * 1024 * 1024,
            4 * 1024 * 1024,
            3,
        ));
        let json = preview.to_json_pretty();

        assert!(json.contains("\"status\": \"ready\""));
        assert!(json.contains("\"eventCount\": 4"));
        assert!(json.contains("\"percentCompleteBasisPoints\":4000"));
    }

    fn chunk_plan(
        status: ChunkPlanStatus,
        planned_bytes: u64,
        chunk_size_bytes: u64,
        chunk_count: u64,
    ) -> ChunkPlan {
        ChunkPlan {
            status,
            reason: status.as_str().to_string(),
            session_id: "OSFLUX-SESSION-1700000000-2-TEST".to_string(),
            session_status: if status == ChunkPlanStatus::Planned {
                WriteSessionStatus::Prepared
            } else {
                WriteSessionStatus::Blocked
            },
            image_source: "C:\\images\\sample.iso".to_string(),
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            image_size_bytes: planned_bytes,
            chunk_size_bytes,
            chunk_count,
            final_chunk_size_bytes: if chunk_count > 0 { 2 * 1024 * 1024 } else { 0 },
            planned_bytes,
            dry_run: true,
            write_access_attempted: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
