use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlan {
    pub source: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub kind: ImageKind,
    pub write_mode: WriteMode,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Iso,
    RawDiskImage,
    Unknown,
}

impl ImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Iso => "iso",
            Self::RawDiskImage => "raw-disk-image",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    FileSystem,
    Raw,
    Unknown,
}

impl WriteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileSystem => "filesystem",
            Self::Raw => "raw",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImagePlanError {
    NotFound(PathBuf),
    NotFile(PathBuf),
    Metadata { path: PathBuf, message: String },
}

impl fmt::Display for ImagePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => {
                write!(formatter, "image path does not exist: {}", path.display())
            }
            Self::NotFile(path) => {
                write!(formatter, "image path is not a file: {}", path.display())
            }
            Self::Metadata { path, message } => {
                write!(
                    formatter,
                    "could not read metadata for {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ImagePlanError {}

pub fn plan_image(path: impl AsRef<Path>) -> Result<ImagePlan, ImagePlanError> {
    let source = path.as_ref().to_path_buf();

    if !source.exists() {
        return Err(ImagePlanError::NotFound(source));
    }

    let metadata = source
        .metadata()
        .map_err(|error| ImagePlanError::Metadata {
            path: source.clone(),
            message: error.to_string(),
        })?;

    if !metadata.is_file() {
        return Err(ImagePlanError::NotFile(source));
    }

    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unnamed>")
        .to_string();

    let kind = classify_image(&source);
    let write_mode = write_mode_for(kind);
    let mut notes = Vec::new();
    let mut warnings = Vec::new();

    match kind {
        ImageKind::Iso => {
            notes.push(
                "ISO detected; later milestones will inspect boot catalog and OS family."
                    .to_string(),
            );
            notes.push("Initial mode is filesystem planning because ISO content often needs extraction and boot setup.".to_string());
        }
        ImageKind::RawDiskImage => {
            notes.push("Raw disk image detected; initial mode is direct block writing after target safety checks.".to_string());
        }
        ImageKind::Unknown => {
            warnings.push("Image type is unknown; OSflux will require deeper inspection before write support.".to_string());
        }
    }

    if metadata.len() == 0 {
        warnings.push("Image file is empty.".to_string());
    }

    Ok(ImagePlan {
        source,
        file_name,
        size_bytes: metadata.len(),
        kind,
        write_mode,
        notes,
        warnings,
    })
}

fn classify_image(path: &Path) -> ImageKind {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "iso" => ImageKind::Iso,
        "img" | "raw" | "bin" => ImageKind::RawDiskImage,
        _ => ImageKind::Unknown,
    }
}

fn write_mode_for(kind: ImageKind) -> WriteMode {
    match kind {
        ImageKind::Iso => WriteMode::FileSystem,
        ImageKind::RawDiskImage => WriteMode::Raw,
        ImageKind::Unknown => WriteMode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{plan_image, ImageKind, WriteMode};
    use std::fs;

    #[test]
    fn classifies_iso_images() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"not a real iso yet").unwrap();

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::Iso);
        assert_eq!(plan.write_mode, WriteMode::FileSystem);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn classifies_raw_images() {
        let path = std::env::temp_dir().join(unique_name("img"));
        fs::write(&path, b"raw data").unwrap();

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::RawDiskImage);
        assert_eq!(plan.write_mode, WriteMode::Raw);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_missing_images() {
        let path = std::env::temp_dir().join(unique_name("iso"));

        let error = plan_image(&path).unwrap_err();

        assert!(error.to_string().contains("does not exist"));
    }

    fn unique_name(extension: &str) -> String {
        format!(
            "osflux-test-{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension
        )
    }
}
