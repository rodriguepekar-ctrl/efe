//! [`NanoAgentId`] — unique identifier for a nano agent.
//!
//! IDs are snake_case strings of 1-64 chars matching `^[a-z][a-z0-9_]{0,63}$`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::NanoAgentIdError;

/// Unique snake_case identifier for a nano agent.
///
/// Validates `^[a-z][a-z0-9_]{0,63}$` at construction.
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::NanoAgentId;
///
/// let id = NanoAgentId::new("json_validate").unwrap();
/// assert_eq!(id.as_str(), "json_validate");
///
/// assert!(NanoAgentId::new("").is_err());
/// assert!(NanoAgentId::new("JsonValidate").is_err());     // uppercase
/// assert!(NanoAgentId::new("123_foo").is_err());          // starts with digit
/// assert!(NanoAgentId::new("foo-bar").is_err());          // hyphen
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NanoAgentId(String);

impl NanoAgentId {
    /// Maximum length of a nano agent ID.
    pub const MAX_LEN: usize = 64;

    /// Construct a new [`NanoAgentId`], validating the format.
    pub fn new(s: impl Into<String>) -> Result<Self, NanoAgentIdError> {
        let s = s.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes self, returning the underlying String.
    pub fn into_inner(self) -> String {
        self.0
    }

    fn validate(s: &str) -> Result<(), NanoAgentIdError> {
        if s.is_empty() {
            return Err(NanoAgentIdError::Empty);
        }
        if s.len() > Self::MAX_LEN {
            return Err(NanoAgentIdError::TooLong { got: s.len() });
        }
        let mut chars = s.chars().enumerate();
        // First char must be a-z
        let (pos, first) = chars.next().expect("non-empty checked above");
        if !first.is_ascii_lowercase() {
            return Err(if first.is_ascii_digit() {
                NanoAgentIdError::InvalidFirstChar
            } else {
                NanoAgentIdError::InvalidChar { ch: first, pos }
            });
        }
        // Remaining chars must be a-z, 0-9, or _
        for (pos, ch) in chars {
            if !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && ch != '_' {
                return Err(NanoAgentIdError::InvalidChar { ch, pos });
            }
        }
        Ok(())
    }
}

impl fmt::Display for NanoAgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for NanoAgentId {
    type Err = NanoAgentIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl AsRef<str> for NanoAgentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for NanoAgentId {
    type Error = NanoAgentIdError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<NanoAgentId> for String {
    fn from(id: NanoAgentId) -> String {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple_id() {
        let id = NanoAgentId::new("json_validate").unwrap();
        assert_eq!(id.as_str(), "json_validate");
        assert_eq!(id.to_string(), "json_validate");
    }

    #[test]
    fn valid_with_digits_and_underscores() {
        assert!(NanoAgentId::new("a").is_ok());
        assert!(NanoAgentId::new("a_b_c_1_2_3").is_ok());
        assert!(NanoAgentId::new("ner_clinical_phase").is_ok());
        assert!(NanoAgentId::new("pdf_extract_text_v2").is_ok());
    }

    #[test]
    fn empty_string_rejected() {
        assert_eq!(NanoAgentId::new(""), Err(NanoAgentIdError::Empty));
    }

    #[test]
    fn too_long_rejected() {
        let long = "a".repeat(65);
        match NanoAgentId::new(&long) {
            Err(NanoAgentIdError::TooLong { got }) => assert_eq!(got, 65),
            _ => panic!("expected TooLong"),
        }
        // 64 chars is OK
        let max_ok = "a".repeat(64);
        assert!(NanoAgentId::new(&max_ok).is_ok());
    }

    #[test]
    fn uppercase_rejected() {
        assert!(matches!(
            NanoAgentId::new("JsonValidate"),
            Err(NanoAgentIdError::InvalidChar { ch: 'J', pos: 0 })
        ));
    }

    #[test]
    fn digit_first_rejected() {
        assert_eq!(
            NanoAgentId::new("123_foo"),
            Err(NanoAgentIdError::InvalidFirstChar)
        );
    }

    #[test]
    fn hyphen_rejected() {
        assert!(matches!(
            NanoAgentId::new("foo-bar"),
            Err(NanoAgentIdError::InvalidChar { ch: '-', pos: 3 })
        ));
    }

    #[test]
    fn from_str_works() {
        let id: NanoAgentId = "test_id".parse().unwrap();
        assert_eq!(id.as_str(), "test_id");
    }

    #[test]
    fn serde_round_trip() {
        let id = NanoAgentId::new("pdf_extract_text").unwrap();
        let s = serde_json::to_string(&id).unwrap();
        assert_eq!(s, r#""pdf_extract_text""#);
        let id2: NanoAgentId = serde_json::from_str(&s).unwrap();
        assert_eq!(id, id2);
    }

    #[test]
    fn serde_rejects_invalid() {
        let result: Result<NanoAgentId, _> = serde_json::from_str(r#""BadID""#);
        assert!(result.is_err());
    }
}
