//! The region and keyboard settings of the signed-in user, for copying into
//! a Windows Setup answer file.

use std::fmt;

/// Region and keyboard settings in the form Windows Setup answer files use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSettings {
    /// The keyboard as `<language id>:<keyboard layout id>`, such as
    /// `0809:00000809` for the United Kingdom keyboard.
    pub input_locale: String,
    /// The locale for programs that do not use Unicode, such as `en-GB`.
    pub system_locale: String,
    /// The user's formats for dates, times, numbers and currency, such as
    /// `en-GB`.
    pub user_locale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionError(pub String);

impl fmt::Display for RegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RegionError {}

/// Reads the signed-in user's keyboard and locales, and the system locale.
#[cfg(windows)]
pub fn read_region_settings() -> Result<RegionSettings, RegionError> {
    use windows::Win32::Globalization::{GetSystemDefaultLocaleName, GetUserDefaultLocaleName};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutNameW;

    // LOCALE_NAME_MAX_LENGTH and KL_NAMELENGTH, including the terminator.
    const LOCALE_NAME_CHARS: usize = 85;
    const LAYOUT_NAME_CHARS: usize = 9;

    fn wide(buffer: &[u16]) -> String {
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    let mut user = [0u16; LOCALE_NAME_CHARS];
    // SAFETY: the buffer is valid for its whole length.
    if unsafe { GetUserDefaultLocaleName(&mut user) } == 0 {
        return Err(RegionError(
            "Windows did not report the user's locale".into(),
        ));
    }
    let mut system = [0u16; LOCALE_NAME_CHARS];
    // SAFETY: as above.
    if unsafe { GetSystemDefaultLocaleName(&mut system) } == 0 {
        return Err(RegionError(
            "Windows did not report the system locale".into(),
        ));
    }
    let mut layout = [0u16; LAYOUT_NAME_CHARS];
    // SAFETY: KL_NAMELENGTH characters, as the function requires.
    unsafe { GetKeyboardLayoutNameW(&mut layout) }.map_err(|error| {
        RegionError(format!(
            "Windows did not report the keyboard layout: {error}"
        ))
    })?;

    Ok(RegionSettings {
        input_locale: input_locale(&wide(&layout))?,
        system_locale: wide(&system),
        user_locale: wide(&user),
    })
}

/// A keyboard layout id such as `00020409` ends with its language id, here
/// `0409`; answer files name the keyboard as `0409:00020409`.
pub fn input_locale(layout_id: &str) -> Result<String, RegionError> {
    if layout_id.len() != 8 || !layout_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RegionError(format!(
            "the keyboard layout id {layout_id:?} is not eight hexadecimal digits"
        )));
    }
    let layout_id = layout_id.to_ascii_uppercase();
    Ok(format!("{}:{layout_id}", &layout_id[4..]))
}

/// Reading the region needs Windows.
#[cfg(not(windows))]
pub fn read_region_settings() -> Result<RegionSettings, RegionError> {
    Err(RegionError(
        "copying the region and keyboard needs Windows, where OSflux reads them from the signed-in user".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::input_locale;

    #[test]
    fn names_the_keyboard_with_its_language() {
        assert_eq!(input_locale("00000809").unwrap(), "0809:00000809");
        assert_eq!(input_locale("00020409").unwrap(), "0409:00020409");
        assert_eq!(input_locale("e0010411").unwrap(), "0411:E0010411");
        assert!(input_locale("0809").is_err());
        assert!(input_locale("0000080G").is_err());
    }
}
