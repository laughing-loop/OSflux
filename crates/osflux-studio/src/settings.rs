//! Studio's settings: the theme and whether writes are read back. Kept in
//! a small text file next to OSflux's other state, one `key = value` per
//! line, so a damaged or older file falls back to the defaults line by line.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "studio-settings.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSetting {
    /// Dark or light, as Windows' app mode is set.
    System,
    Dark,
    Light,
}

impl ThemeSetting {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub theme: ThemeSetting,
    /// Read every written byte back and compare it. On unless turned off.
    pub verify_after_writing: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeSetting::System,
            verify_after_writing: true,
        }
    }
}

impl Settings {
    /// Reads settings from text, keeping the default for any line that is
    /// missing or not understood.
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match (key.trim(), value.trim()) {
                ("theme", value) => {
                    if let Some(theme) = ThemeSetting::parse(value) {
                        settings.theme = theme;
                    }
                }
                ("verify-after-writing", "true") => settings.verify_after_writing = true,
                ("verify-after-writing", "false") => settings.verify_after_writing = false,
                _ => {}
            }
        }
        settings
    }

    pub fn to_text(self) -> String {
        format!(
            "# OSflux Studio settings\ntheme = {}\nverify-after-writing = {}\n",
            self.theme.as_str(),
            self.verify_after_writing
        )
    }

    /// The defaults when the file does not exist or cannot be read.
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, self.to_text())
    }

    /// `studio-settings.txt` in OSflux's state folder.
    pub fn default_path() -> Option<PathBuf> {
        osflux_engine::state_directory().map(|directory| directory.join(FILE_NAME))
    }
}

#[cfg(test)]
mod tests {
    use super::{Settings, ThemeSetting};

    #[test]
    fn defaults_follow_windows_and_verify() {
        let settings = Settings::default();
        assert_eq!(settings.theme, ThemeSetting::System);
        assert!(settings.verify_after_writing);
    }

    #[test]
    fn round_trips_through_text() {
        for theme in [
            ThemeSetting::System,
            ThemeSetting::Dark,
            ThemeSetting::Light,
        ] {
            for verify_after_writing in [true, false] {
                let settings = Settings {
                    theme,
                    verify_after_writing,
                };
                assert_eq!(Settings::parse(&settings.to_text()), settings);
            }
        }
    }

    #[test]
    fn unknown_or_damaged_lines_keep_the_defaults() {
        let settings = Settings::parse("theme = purple\nverify-after-writing = maybe\nnonsense\n");
        assert_eq!(settings, Settings::default());

        let settings = Settings::parse("  theme=light  \nextra = 1\n");
        assert_eq!(settings.theme, ThemeSetting::Light);
        assert!(settings.verify_after_writing);
    }

    #[test]
    fn saves_and_loads_a_file() {
        let directory = std::env::temp_dir().join(format!(
            "osflux-studio-settings-test-{}",
            std::process::id()
        ));
        let path = directory.join("nested").join("studio-settings.txt");
        let settings = Settings {
            theme: ThemeSetting::Dark,
            verify_after_writing: false,
        };
        settings.save(&path).expect("settings save");
        assert_eq!(Settings::load(&path), settings);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let path = std::env::temp_dir().join("osflux-studio-settings-that-does-not-exist.txt");
        assert_eq!(Settings::load(&path), Settings::default());
    }
}
