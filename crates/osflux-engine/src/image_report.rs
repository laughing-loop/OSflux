use std::path::Path;

use crate::image_plan::{plan_image, ImagePlan, ImagePlanError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReport {
    pub title: String,
    pub sections: Vec<PlanReportSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReportSection {
    pub title: String,
    pub fields: Vec<PlanReportField>,
    pub items: Vec<PlanReportItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReportField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReportItem {
    pub title: String,
    pub fields: Vec<PlanReportField>,
}

impl PlanReportSection {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            fields: Vec::new(),
            items: Vec::new(),
        }
    }

    fn field(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push(PlanReportField {
            label: label.into(),
            value: value.into(),
        });
        self
    }

    fn item(mut self, item: PlanReportItem) -> Self {
        self.items.push(item);
        self
    }

    fn has_content(&self) -> bool {
        !self.fields.is_empty() || !self.items.is_empty()
    }
}

impl PlanReportItem {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            fields: Vec::new(),
        }
    }

    fn field(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push(PlanReportField {
            label: label.into(),
            value: value.into(),
        });
        self
    }
}

pub fn plan_image_report(path: impl AsRef<Path>) -> Result<PlanReport, ImagePlanError> {
    let plan = plan_image(path)?;
    Ok(build_image_plan_report(&plan))
}

pub fn build_image_plan_report(plan: &ImagePlan) -> PlanReport {
    let mut sections = Vec::new();

    sections.push(
        PlanReportSection::new("summary")
            .field("source", plan.source.display().to_string())
            .field("file", plan.file_name.clone())
            .field("size", format_size(plan.size_bytes))
            .field("kind", plan.kind.as_str())
            .field("detection", plan.detection_method.as_str())
            .field("write mode", plan.write_mode.as_str())
            .field("write strategy", plan.write_strategy.as_str()),
    );

    let signatures = plan.signatures.iter().fold(
        PlanReportSection::new("signatures"),
        |section, signature| section.item(PlanReportItem::new(signature.as_str())),
    );
    if signatures.has_content() {
        sections.push(signatures);
    }

    if let Some(metadata) = plan.boot_metadata.as_ref() {
        let mut boot_metadata = PlanReportSection::new("boot metadata")
            .field("bootable", yes_no(metadata.is_bootable()))
            .field("boot style", metadata.boot_style().as_str());

        if let Some(info) = metadata.el_torito.as_ref() {
            boot_metadata = boot_metadata
                .field("boot catalog", "found")
                .field("catalog lba", info.catalog_lba.to_string())
                .field("platform", info.platform.as_str())
                .field(
                    "validation entry",
                    if info.validation_entry_valid {
                        "valid"
                    } else {
                        "invalid"
                    },
                )
                .field(
                    "default entry",
                    if info.default_entry_bootable {
                        "bootable"
                    } else {
                        "not bootable"
                    },
                )
                .field("emulation", info.emulation.as_str())
                .field("load rba", info.load_rba.to_string())
                .field("sector count", info.sector_count.to_string());

            sections.push(boot_metadata);

            let boot_entries = info.boot_entries.iter().fold(
                PlanReportSection::new("boot entries"),
                |section, entry| {
                    let boot_state = if entry.bootable {
                        "bootable"
                    } else {
                        "not bootable"
                    };
                    let range = entry.image_range;
                    let mut item = PlanReportItem::new(format!(
                        "{} | {} | {} | {}",
                        entry.source.as_str(),
                        entry.platform.as_str(),
                        boot_state,
                        entry.emulation.as_str()
                    ))
                    .field("load rba", entry.load_rba.to_string())
                    .field("sector count", entry.sector_count.to_string())
                    .field("range status", range.status.as_str());

                    if let Some(start_offset) = range.start_offset {
                        item = item.field("range offset", format_size(start_offset));
                    }

                    if let Some(required_bytes) = range.required_bytes {
                        item = item.field("range required", format_size(required_bytes));
                    }

                    if let Some(end_offset) = range.end_offset {
                        item = item.field("range end", format_size(end_offset));
                    }

                    section.item(item)
                },
            );
            if boot_entries.has_content() {
                sections.push(boot_entries);
            }
        } else {
            sections.push(boot_metadata.field("boot catalog", "not found"));
        }
    }

    let notes = plan
        .notes
        .iter()
        .fold(PlanReportSection::new("notes"), |section, note| {
            section.item(PlanReportItem::new(note))
        });
    if notes.has_content() {
        sections.push(notes);
    }

    let warnings = plan
        .warnings
        .iter()
        .fold(PlanReportSection::new("warnings"), |section, warning| {
            section.item(PlanReportItem::new(warning))
        });
    if warnings.has_content() {
        sections.push(warnings);
    }

    PlanReport {
        title: "OSflux image plan".to_string(),
        sections,
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;

    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", bytes as f64 / GIB)
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::{build_image_plan_report, PlanReportSection};
    use crate::image_plan::{
        DetectionMethod, ImageKind, ImagePlan, ImageSignature, ImageWriteStrategy, WriteMode,
    };

    #[test]
    fn builds_summary_and_diagnostic_sections() {
        let plan = ImagePlan {
            source: "C:\\images\\sample.img".into(),
            file_name: "sample.img".to_string(),
            size_bytes: 4096,
            kind: ImageKind::RawDiskImage,
            detection_method: DetectionMethod::Signature,
            signatures: vec![ImageSignature::Gpt],
            boot_metadata: None,
            write_mode: WriteMode::Raw,
            write_strategy: ImageWriteStrategy::RawBlockWrite,
            notes: vec!["ready for target safety checks".to_string()],
            warnings: vec!["example warning".to_string()],
        };

        let report = build_image_plan_report(&plan);

        assert_eq!(report.title, "OSflux image plan");
        assert_eq!(section(&report.sections, "summary").fields.len(), 7);
        assert_eq!(
            field(section(&report.sections, "summary"), "write strategy"),
            "raw-block-write"
        );
        assert_eq!(section(&report.sections, "signatures").items.len(), 1);
        assert_eq!(section(&report.sections, "notes").items.len(), 1);
        assert_eq!(section(&report.sections, "warnings").items.len(), 1);
    }

    fn section<'a>(sections: &'a [PlanReportSection], title: &str) -> &'a PlanReportSection {
        sections
            .iter()
            .find(|section| section.title == title)
            .expect("report section should exist")
    }

    fn field<'a>(section: &'a PlanReportSection, label: &str) -> &'a str {
        section
            .fields
            .iter()
            .find(|field| field.label == label)
            .map(|field| field.value.as_str())
            .expect("report field should exist")
    }
}
