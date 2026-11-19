//! Small helpers for the hand-written JSON that reports and logs produce.

pub(crate) fn escape_json_string(value: &str) -> String {
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

pub(crate) fn string_json(value: &str) -> String {
    format!("\"{}\"", escape_json_string(value))
}

pub(crate) fn optional_string_json(value: Option<&str>) -> String {
    value.map(string_json).unwrap_or_else(|| "null".to_string())
}

pub(crate) fn optional_number_json(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::{optional_number_json, optional_string_json, string_json};

    #[test]
    fn escapes_quotes_backslashes_and_control_characters() {
        assert_eq!(
            string_json("C:\\path \"x\"\n\u{01}"),
            "\"C:\\\\path \\\"x\\\"\\n\\u0001\""
        );
        assert_eq!(optional_string_json(None), "null");
        assert_eq!(optional_number_json(Some(42)), "42");
    }
}
