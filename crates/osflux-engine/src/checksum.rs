//! The expected SHA-256 an operator copies from a download page, and how it
//! is compared with the hash the engine computes while writing.

use std::fmt;

/// A SHA-256 written as 64 hexadecimal characters, kept in lower case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256Hex(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sha256ParseError {
    /// Not 64 characters once spaces are removed.
    WrongLength { characters: usize },
    /// A character other than 0–9 and A–F.
    NotHexadecimal,
}

impl fmt::Display for Sha256ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { characters } => write!(
                formatter,
                "a SHA-256 has 64 characters; this has {characters}"
            ),
            Self::NotHexadecimal => {
                formatter.write_str("a SHA-256 uses only the characters 0–9 and A–F")
            }
        }
    }
}

impl std::error::Error for Sha256ParseError {}

impl Sha256Hex {
    /// Reads a hash as download pages show it: either case, and with any
    /// spaces, tabs or line breaks from copying removed.
    pub fn parse(text: &str) -> Result<Self, Sha256ParseError> {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if !compact.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Sha256ParseError::NotHexadecimal);
        }
        if compact.len() != 64 {
            return Err(Sha256ParseError::WrongLength {
                characters: compact.chars().count(),
            });
        }
        Ok(Self(compact.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `actual`, a hash in hex as the engine reports it, is this one.
    pub fn matches(&self, actual: &str) -> bool {
        actual.eq_ignore_ascii_case(&self.0)
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Sha256Hex, Sha256ParseError};

    const MICROSOFT: &str = "2A1AB0D46D7285F738D082858925D3303F3DC430F0A4EAB5AA238325A4274906";

    #[test]
    fn reads_a_hash_in_either_case() {
        let hash = Sha256Hex::parse(MICROSOFT).expect("a valid hash");
        assert_eq!(hash.as_str(), MICROSOFT.to_ascii_lowercase());
        assert_eq!(Sha256Hex::parse(&MICROSOFT.to_ascii_lowercase()), Ok(hash));
    }

    #[test]
    fn ignores_spaces_and_line_breaks_from_copying() {
        let spaced = format!(" {}\n{} ", &MICROSOFT[..32], &MICROSOFT[32..]);
        assert!(Sha256Hex::parse(&spaced).is_ok());
    }

    #[test]
    fn refuses_wrong_lengths_and_other_characters() {
        assert_eq!(
            Sha256Hex::parse(&MICROSOFT[..63]),
            Err(Sha256ParseError::WrongLength { characters: 63 })
        );
        assert_eq!(
            Sha256Hex::parse(&format!("{MICROSOFT}0")),
            Err(Sha256ParseError::WrongLength { characters: 65 })
        );
        assert_eq!(
            Sha256Hex::parse(&MICROSOFT.replace('A', "G")),
            Err(Sha256ParseError::NotHexadecimal)
        );
        assert_eq!(
            Sha256Hex::parse(""),
            Err(Sha256ParseError::WrongLength { characters: 0 })
        );
    }

    #[test]
    fn compares_with_the_engines_lower_case_hash() {
        let expected = Sha256Hex::parse(MICROSOFT).unwrap();
        assert!(expected.matches(&MICROSOFT.to_ascii_lowercase()));
        assert!(!expected.matches(&"0".repeat(64)));
    }
}
