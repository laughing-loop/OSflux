use std::fmt::Write as _;
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

impl PlanReport {
    pub fn to_json_pretty(&self) -> String {
        let mut output = String::new();
        write_report_json(&mut output, self, 0);
        output
    }
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
            .field("status", plan.status.as_str())
            .field("status reason", plan.status_reason.clone())
            .field("kind", plan.kind.as_str())
            .field("hybrid", if plan.hybrid { "yes" } else { "no" })
            .field("detection", plan.detection_method.as_str())
            .field("write mode", plan.write_mode.as_str())
            .field("write strategy", plan.write_strategy.as_str()),
    );

    match &plan.iso_files {
        Some(Ok(summary)) => {
            let largest = summary
                .largest_file
                .as_ref()
                .map(|(path, size)| format!("{path} ({})", format_size(*size)))
                .unwrap_or_else(|| "none".to_string());
            let fits = if !summary.cannot_copy.is_empty() {
                format!("no: {}", summary.cannot_copy.join("; "))
            } else if summary.split_files.is_empty() {
                "yes".to_string()
            } else {
                let splits = summary
                    .split_files
                    .iter()
                    .map(|split| {
                        format!(
                            "{} ({}) into {} parts",
                            split.path,
                            format_size(split.size_bytes),
                            split.parts.len()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                format!("yes, after splitting {splits}")
            };
            sections.push(
                PlanReportSection::new("files")
                    .field("filesystem", "UDF")
                    .field("label", summary.label.clone())
                    .field(
                        "files",
                        format!(
                            "{} ({})",
                            summary.file_count,
                            format_size(summary.file_bytes)
                        ),
                    )
                    .field("largest file", largest)
                    .field(
                        "uefi loader",
                        summary
                            .uefi_loader
                            .clone()
                            .unwrap_or_else(|| "missing".to_string()),
                    )
                    .field("fits FAT32", fits),
            );
        }
        Some(Err(reason)) => sections.push(
            PlanReportSection::new("files")
                .field("filesystem", "UDF")
                .field("problem", reason.clone()),
        ),
        None => {}
    }

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

        if let Some(directory) = metadata.directory.as_ref() {
            sections.push(
                PlanReportSection::new("iso directory")
                    .field("root extent lba", directory.root_extent_lba.to_string())
                    .field(
                        "root data length",
                        format_size(u64::from(directory.root_data_length)),
                    )
                    .field(
                        "scanned directories",
                        directory.scanned_directories.to_string(),
                    )
                    .field("entries", directory.entries.len().to_string())
                    .field("truncated", yes_no(directory.truncated)),
            );

            let boot_assets = directory.boot_assets.iter().fold(
                PlanReportSection::new("iso boot assets"),
                |section, asset| {
                    section.item(
                        PlanReportItem::new(asset.path.clone()).field("kind", asset.kind.as_str()),
                    )
                },
            );
            if boot_assets.has_content() {
                sections.push(boot_assets);
            }
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

fn write_report_json(output: &mut String, report: &PlanReport, indent: usize) {
    write_line(output, indent, "{");
    write_json_property(output, indent + 2, "title", &report.title, true);
    write_line(output, indent + 2, "\"sections\": [");

    for (section_index, section) in report.sections.iter().enumerate() {
        write_section_json(
            output,
            section,
            indent + 4,
            section_index + 1 < report.sections.len(),
        );
    }

    write_line(output, indent + 2, "]");
    write_line(output, indent, "}");
}

fn write_section_json(
    output: &mut String,
    section: &PlanReportSection,
    indent: usize,
    trailing_comma: bool,
) {
    write_line(output, indent, "{");
    write_json_property(output, indent + 2, "title", &section.title, true);
    write_line(output, indent + 2, "\"fields\": [");

    for (field_index, field) in section.fields.iter().enumerate() {
        write_field_json(
            output,
            field,
            indent + 4,
            field_index + 1 < section.fields.len(),
        );
    }

    write_line(output, indent + 2, "],");
    write_line(output, indent + 2, "\"items\": [");

    for (item_index, item) in section.items.iter().enumerate() {
        write_item_json(
            output,
            item,
            indent + 4,
            item_index + 1 < section.items.len(),
        );
    }

    write_line(output, indent + 2, "]");
    write!(
        output,
        "{}}}{}",
        " ".repeat(indent),
        if trailing_comma { ",\n" } else { "\n" }
    )
    .expect("writing JSON to string should not fail");
}

fn write_item_json(
    output: &mut String,
    item: &PlanReportItem,
    indent: usize,
    trailing_comma: bool,
) {
    write_line(output, indent, "{");
    write_json_property(output, indent + 2, "title", &item.title, true);
    write_line(output, indent + 2, "\"fields\": [");

    for (field_index, field) in item.fields.iter().enumerate() {
        write_field_json(
            output,
            field,
            indent + 4,
            field_index + 1 < item.fields.len(),
        );
    }

    write_line(output, indent + 2, "]");
    write!(
        output,
        "{}}}{}",
        " ".repeat(indent),
        if trailing_comma { ",\n" } else { "\n" }
    )
    .expect("writing JSON to string should not fail");
}

fn write_field_json(
    output: &mut String,
    field: &PlanReportField,
    indent: usize,
    trailing_comma: bool,
) {
    write_line(output, indent, "{");
    write_json_property(output, indent + 2, "label", &field.label, true);
    write_json_property(output, indent + 2, "value", &field.value, false);
    write!(
        output,
        "{}}}{}",
        " ".repeat(indent),
        if trailing_comma { ",\n" } else { "\n" }
    )
    .expect("writing JSON to string should not fail");
}

fn write_json_property(
    output: &mut String,
    indent: usize,
    key: &str,
    value: &str,
    trailing_comma: bool,
) {
    writeln!(
        output,
        "{}\"{}\": \"{}\"{}",
        " ".repeat(indent),
        escape_json_string(key),
        escape_json_string(value),
        if trailing_comma { "," } else { "" }
    )
    .expect("writing JSON to string should not fail");
}

fn write_line(output: &mut String, indent: usize, value: &str) {
    writeln!(output, "{}{}", " ".repeat(indent), value)
        .expect("writing JSON to string should not fail");
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
    use super::{build_image_plan_report, PlanReport, PlanReportField, PlanReportSection};
    use crate::file_copy::{IsoFilesSummary, SplitFile};
    use crate::image_plan::{
        DetectionMethod, ImageKind, ImagePlan, ImagePlanStatus, ImageSignature, ImageWriteStrategy,
        WriteMode,
    };

    #[test]
    fn builds_summary_and_diagnostic_sections() {
        let plan = ImagePlan {
            source: "C:\\images\\sample.img".into(),
            file_name: "sample.img".to_string(),
            size_bytes: 4096,
            status: ImagePlanStatus::Ready,
            status_reason: "image plan is trusted for the next safety phase".to_string(),
            kind: ImageKind::RawDiskImage,
            detection_method: DetectionMethod::Signature,
            signatures: vec![ImageSignature::Gpt],
            hybrid: false,
            boot_metadata: None,
            write_mode: WriteMode::Raw,
            write_strategy: ImageWriteStrategy::RawBlockWrite,
            iso_files: None,
            notes: vec!["ready for target safety checks".to_string()],
            warnings: vec!["example warning".to_string()],
        };

        let report = build_image_plan_report(&plan);

        assert_eq!(report.title, "OSflux image plan");
        assert_eq!(section(&report.sections, "summary").fields.len(), 10);
        assert_eq!(field(section(&report.sections, "summary"), "hybrid"), "no");
        assert_eq!(
            field(section(&report.sections, "summary"), "status"),
            "ready"
        );
        assert_eq!(
            field(section(&report.sections, "summary"), "write strategy"),
            "raw-block-write"
        );
        assert_eq!(section(&report.sections, "signatures").items.len(), 1);
        assert_eq!(section(&report.sections, "notes").items.len(), 1);
        assert_eq!(section(&report.sections, "warnings").items.len(), 1);
    }

    fn windows_plan(summary: IsoFilesSummary) -> ImagePlan {
        ImagePlan {
            source: "C:\\images\\Win11.iso".into(),
            file_name: "Win11.iso".to_string(),
            size_bytes: 6 * 1024 * 1024 * 1024,
            status: ImagePlanStatus::Ready,
            status_reason: "ready".to_string(),
            kind: ImageKind::Iso,
            detection_method: DetectionMethod::Signature,
            signatures: vec![ImageSignature::Udf],
            hybrid: false,
            boot_metadata: None,
            write_mode: WriteMode::Raw,
            write_strategy: ImageWriteStrategy::RawBlockWrite,
            iso_files: Some(Ok(summary)),
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn summary() -> IsoFilesSummary {
        IsoFilesSummary {
            label: "CCCOMA_X64FRE_EN-US_DV9".to_string(),
            file_count: 1000,
            file_bytes: 6 * 1024 * 1024 * 1024,
            largest_file: Some(("sources/install.wim".to_string(), 5_500_000_000)),
            uefi_loader: Some("efi/boot/bootx64.efi".to_string()),
            split_files: Vec::new(),
            cannot_copy: Vec::new(),
        }
    }

    fn fits(summary: IsoFilesSummary) -> String {
        let report = build_image_plan_report(&windows_plan(summary));
        field(section(&report.sections, "files"), "fits FAT32").to_string()
    }

    #[test]
    fn says_whether_an_isos_files_fit_on_fat32() {
        assert_eq!(fits(summary()), "yes");

        let mut split = summary();
        split.split_files.push(SplitFile {
            path: "sources/install.wim".to_string(),
            size_bytes: 5_500_000_000,
            parts: vec!["install.swm".to_string(), "install2.swm".to_string()],
        });
        assert_eq!(
            fits(split),
            "yes, after splitting sources/install.wim (5.12 GiB) into 2 parts"
        );

        let mut refused = summary();
        refused
            .cannot_copy
            .push("sources/install.esd is too large for FAT32 and cannot be split".to_string());
        assert_eq!(
            fits(refused),
            "no: sources/install.esd is too large for FAT32 and cannot be split"
        );
    }

    #[test]
    fn serializes_report_as_json() {
        let report = PlanReport {
            title: "OSflux \"image\" plan".to_string(),
            sections: vec![PlanReportSection {
                title: "summary".to_string(),
                fields: vec![PlanReportField {
                    label: "source".to_string(),
                    value: "C:\\images\\sample.iso".to_string(),
                }],
                items: Vec::new(),
            }],
        };

        let json = report.to_json_pretty();

        assert!(json.starts_with("{\n"));
        assert!(json.contains("\"title\": \"OSflux \\\"image\\\" plan\""));
        assert!(json.contains("\"value\": \"C:\\\\images\\\\sample.iso\""));
        assert!(json.contains("\"sections\": ["));
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
