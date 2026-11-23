//! The region and keyboard settings of the signed-in user, for copying into
//! a Windows Setup answer file.

use std::fmt;

/// Region and keyboard settings in the form Windows Setup answer files use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSettings {
    /// The user's keyboards, default first, separated by semicolons. Each is
    /// `<language id>:<keyboard layout id>`, such as `0809:00020409` for
    /// English (United Kingdom) typed on the US-International keyboard, or
    /// `<language id>:{<text service>}{<profile>}` for an input method editor.
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

/// One of the user's languages and its keyboards, each with the position
/// Windows stores for it, as found under
/// `HKCU\Control Panel\International\User Profile\<language>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageInputMethods {
    pub language: String,
    pub input_methods: Vec<(String, u32)>,
}

/// Whether `tip` names a keyboard the way answer files do: `0809:00020409`,
/// or `0411:{GUID}{GUID}` for an input method editor.
pub fn is_input_method_tip(tip: &str) -> bool {
    let Some((language, method)) = tip.split_once(':') else {
        return false;
    };
    let hex = |text: &str| text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if language.len() != 4 || !hex(language) {
        return false;
    }
    if method.len() == 8 {
        return hex(method);
    }
    // Two braced GUIDs, 38 characters each.
    method.len() == 76 && method.split_inclusive('}').all(is_braced_guid)
}

fn is_braced_guid(text: &str) -> bool {
    let Some(inner) = text
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return false;
    };
    let groups: Vec<&str> = inner.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, length)| {
            group.len() == length && group.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

/// The answer-file keyboard list for the user's languages: each language's
/// keyboards in the order Windows keeps them, languages in the user's
/// order, with `default_override` first when it is one of them. Values
/// that are not keyboards, such as a language's cached display name, are
/// left out.
pub fn input_locale_list(
    languages: &[LanguageInputMethods],
    default_override: Option<&str>,
) -> Result<String, RegionError> {
    let mut tips: Vec<String> = Vec::new();
    for language in languages {
        let mut methods: Vec<&(String, u32)> = language
            .input_methods
            .iter()
            .filter(|(tip, _)| is_input_method_tip(tip))
            .collect();
        methods.sort_by_key(|(_, position)| *position);
        for (tip, _) in methods {
            if !tips.iter().any(|known| known.eq_ignore_ascii_case(tip)) {
                tips.push(tip.clone());
            }
        }
    }
    if let Some(preferred) = default_override.filter(|tip| is_input_method_tip(tip)) {
        if let Some(index) = tips
            .iter()
            .position(|tip| tip.eq_ignore_ascii_case(preferred))
        {
            let tip = tips.remove(index);
            tips.insert(0, tip);
        }
    }
    if tips.is_empty() {
        return Err(RegionError(
            "Windows lists no keyboards for the signed-in user".to_string(),
        ));
    }
    Ok(tips.join(";"))
}

/// Reads the signed-in user's keyboards and locales, and the system locale.
#[cfg(windows)]
pub fn read_region_settings() -> Result<RegionSettings, RegionError> {
    use windows::Win32::Globalization::{GetSystemDefaultLocaleName, GetUserDefaultLocaleName};

    // LOCALE_NAME_MAX_LENGTH, including the terminator.
    const LOCALE_NAME_CHARS: usize = 85;

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

    Ok(RegionSettings {
        input_locale: registry::read_user_input_locales()?,
        system_locale: from_wide(&system),
        user_locale: from_wide(&user),
    })
}

/// Reading the region needs Windows.
#[cfg(not(windows))]
pub fn read_region_settings() -> Result<RegionSettings, RegionError> {
    Err(RegionError(
        "copying the region and keyboard needs Windows, where OSflux reads them from the signed-in user".into(),
    ))
}

#[cfg(windows)]
fn from_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(windows)]
mod registry {
    //! The user's language list, as Settings and `Get-WinUserLanguageList`
    //! keep it.

    use super::{from_wide, input_locale_list, LanguageInputMethods, RegionError};
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        REG_DWORD, REG_ROUTINE_FLAGS, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ,
    };

    const USER_PROFILE: &str = r"Control Panel\International\User Profile";

    pub(super) fn read_user_input_locales() -> Result<String, RegionError> {
        let unreadable = |what: &str| {
            RegionError(format!(
                "could not read the signed-in user's {what} from HKCU\\{USER_PROFILE}"
            ))
        };
        let languages = read_string(USER_PROFILE, "Languages", RRF_RT_REG_MULTI_SZ)
            .ok_or_else(|| unreadable("language list"))?;
        let mut lists = Vec::new();
        for language in languages.split('\0').filter(|name| !name.is_empty()) {
            let input_methods = read_dword_values(&format!(r"{USER_PROFILE}\{language}"))
                .ok_or_else(|| unreadable(&format!("keyboards for {language}")))?;
            lists.push(LanguageInputMethods {
                language: language.to_string(),
                input_methods,
            });
        }
        let default_override = read_string(USER_PROFILE, "InputMethodOverride", RRF_RT_REG_SZ);
        input_locale_list(&lists, default_override.as_deref())
    }

    /// A string or multi-string value; a multi-string keeps its NUL
    /// separators.
    fn read_string(subkey: &str, value: &str, kind: REG_ROUTINE_FLAGS) -> Option<String> {
        let subkey = HSTRING::from(subkey);
        let value = HSTRING::from(value);
        let mut bytes = 0u32;
        // SAFETY: a size query; no buffer is passed.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &subkey,
                &value,
                kind,
                None,
                None,
                Some(&mut bytes),
            )
        };
        if status != ERROR_SUCCESS || bytes == 0 {
            return None;
        }
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
        // SAFETY: the buffer holds `bytes` bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &subkey,
                &value,
                kind,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        buffer.truncate((bytes as usize) / 2);
        Some(
            String::from_utf16_lossy(&buffer)
                .trim_end_matches('\0')
                .to_string(),
        )
    }

    /// Every DWORD value under `subkey`, by name.
    fn read_dword_values(subkey: &str) -> Option<Vec<(String, u32)>> {
        let mut key = HKEY::default();
        // SAFETY: `key` receives the opened handle, closed below.
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(subkey),
                0,
                KEY_READ,
                &mut key,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut values = Vec::new();
        let mut index = 0;
        let outcome = loop {
            let mut name = [0u16; 256];
            let mut name_chars = name.len() as u32;
            let mut kind = 0u32;
            let mut data = [0u8; 4];
            let mut data_bytes = data.len() as u32;
            // SAFETY: every buffer is valid for the length passed with it.
            let status = unsafe {
                RegEnumValueW(
                    key,
                    index,
                    PWSTR(name.as_mut_ptr()),
                    &mut name_chars,
                    None,
                    Some(&mut kind),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_bytes),
                )
            };
            index += 1;
            if status == ERROR_NO_MORE_ITEMS {
                break Some(values);
            }
            // A value too large for four bytes is not a DWORD, so not a
            // keyboard; skip it.
            if status == ERROR_MORE_DATA {
                continue;
            }
            if status != ERROR_SUCCESS {
                break None;
            }
            if kind == REG_DWORD.0 && data_bytes == 4 {
                values.push((from_wide(&name), u32::from_le_bytes(data)));
            }
        };
        // SAFETY: `key` was opened above and is closed once.
        unsafe {
            let _ = RegCloseKey(key);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::{input_locale_list, is_input_method_tip, LanguageInputMethods};

    fn language(name: &str, methods: &[(&str, u32)]) -> LanguageInputMethods {
        LanguageInputMethods {
            language: name.to_string(),
            input_methods: methods
                .iter()
                .map(|(tip, position)| (tip.to_string(), *position))
                .collect(),
        }
    }

    const JAPANESE_IME: &str =
        "0411:{03B5835F-F03C-411B-9CE2-AA23E1171E36}{A76C93D9-5523-4E90-AAFA-4DB112F9AC76}";

    #[test]
    fn recognises_keyboards_and_input_method_editors() {
        for good in [
            "0809:00000809",
            "0809:00020409",
            "0411:E0010411",
            JAPANESE_IME,
        ] {
            assert!(is_input_method_tip(good), "{good}");
        }
        for bad in [
            "0809",
            "0809-00000809",
            "080:00000809",
            "0809:0000080G",
            "CachedLanguageName",
            "0411:{03B5835F-F03C-411B-9CE2-AA23E1171E36}",
        ] {
            assert!(!is_input_method_tip(bad), "{bad}");
        }
    }

    #[test]
    fn keeps_every_keyboard_in_the_users_order() {
        // English (United Kingdom) with US-International first and UK second,
        // as Get-WinUserLanguageList showed on the test PC.
        let languages = [language(
            "en-GB",
            &[
                ("0809:00000809", 2),
                ("CachedLanguageName", 0),
                ("0809:00020409", 1),
            ],
        )];

        assert_eq!(
            input_locale_list(&languages, None).unwrap(),
            "0809:00020409;0809:00000809"
        );
    }

    #[test]
    fn follows_the_languages_in_order() {
        let languages = [
            language("fr-FR", &[("040c:0000040c", 1)]),
            language("ja", &[(JAPANESE_IME, 1)]),
        ];

        assert_eq!(
            input_locale_list(&languages, None).unwrap(),
            format!("040c:0000040c;{JAPANESE_IME}")
        );
    }

    #[test]
    fn puts_the_chosen_default_first() {
        let languages = [language(
            "en-GB",
            &[("0809:00020409", 1), ("0809:00000809", 2)],
        )];

        assert_eq!(
            input_locale_list(&languages, Some("0809:00000809")).unwrap(),
            "0809:00000809;0809:00020409"
        );
        // An override that is not one of the user's keyboards changes nothing.
        assert_eq!(
            input_locale_list(&languages, Some("0409:00000409")).unwrap(),
            "0809:00020409;0809:00000809"
        );
    }

    #[test]
    fn refuses_an_empty_list() {
        assert!(
            input_locale_list(&[language("en-GB", &[("CachedLanguageName", 0)])], None).is_err()
        );
        assert!(input_locale_list(&[], None).is_err());
    }
}
