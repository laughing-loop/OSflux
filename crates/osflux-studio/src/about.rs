//! What the About dialog shows: links, the platform, and the licence texts.

pub const WEBSITE: &str = "https://hodtechnologies.com";
pub const SOURCE: &str = "https://github.com/laughing-loop/OSflux";
pub const ISSUES: &str = "https://github.com/laughing-loop/OSflux/issues";
/// Where releases, and their third-party notices, are published.
pub const RELEASES: &str = "https://github.com/laughing-loop/OSflux/releases";
/// The licences of every crate built into the programs, shipped next to
/// osflux-studio.exe in the release zip.
pub const NOTICES_FILE: &str = "THIRD-PARTY-NOTICES.html";

/// The third-party notices to open: the file next to the program when it is
/// there, as in a release, or else the releases page.
pub fn third_party_notices(program: Option<&std::path::Path>) -> String {
    program
        .and_then(std::path::Path::parent)
        .map(|folder| folder.join(NOTICES_FILE))
        .filter(|file| file.is_file())
        .map(|file| file.display().to_string())
        .unwrap_or_else(|| RELEASES.to_string())
}

/// The full GNU GPL, version 3, from the repository's LICENSE.
pub const GPL_TEXT: &str = include_str!("../../../LICENSE");
/// The SIL Open Font License that covers the embedded IBM Plex fonts.
pub const FONT_LICENCE_TEXT: &str = include_str!("../ui/fonts/OFL.txt");

/// The URL behind an About link: "website", "source" or "issues".
pub fn link(id: &str) -> Option<&'static str> {
    match id {
        "website" => Some(WEBSITE),
        "source" => Some(SOURCE),
        "issues" => Some(ISSUES),
        _ => None,
    }
}

/// "Windows · x86_64", as the About dialog lists it.
pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "linux" => "Linux",
        "macos" => "macOS",
        other => other,
    };
    format!("{os} · {}", std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::{link, platform, third_party_notices, FONT_LICENCE_TEXT, GPL_TEXT, RELEASES};

    #[test]
    fn third_party_notices_open_from_next_to_the_program() {
        let folder = std::env::temp_dir().join(format!("osflux-notices-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join("osflux-studio.exe");
        assert_eq!(third_party_notices(Some(&program)), RELEASES, "no file yet");
        let notices = folder.join(super::NOTICES_FILE);
        std::fs::write(&notices, "<html></html>").unwrap();
        let opened = third_party_notices(Some(&program));
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(opened, notices.display().to_string());
        assert_eq!(third_party_notices(None), RELEASES);
    }

    #[test]
    fn links_point_to_the_website_and_the_repository() {
        assert_eq!(link("website"), Some("https://hodtechnologies.com"));
        assert_eq!(
            link("source"),
            Some("https://github.com/laughing-loop/OSflux")
        );
        assert_eq!(
            link("issues"),
            Some("https://github.com/laughing-loop/OSflux/issues")
        );
        assert_eq!(link("anything else"), None);
    }

    #[test]
    fn licence_texts_are_the_real_ones() {
        assert!(GPL_TEXT.contains("GNU GENERAL PUBLIC LICENSE"));
        assert!(GPL_TEXT.contains("Version 3, 29 June 2007"));
        assert!(FONT_LICENCE_TEXT.contains("SIL OPEN FONT LICENSE Version 1.1"));
        assert!(FONT_LICENCE_TEXT.contains("Reserved Font Name \"Plex\""));
    }

    #[test]
    fn the_platform_names_the_system_and_architecture() {
        let platform = platform();
        assert!(platform.contains(" · "));
        assert!(platform.ends_with(std::env::consts::ARCH));
    }
}
