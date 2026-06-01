//! PR #22 § 10 + AD26 — object-store abstraction.
//!
//! Phase-2 supports `file://` only ; S3 / GCS are stubs that return
//! `Unsupported` so operators can wire them via env-driven CLI flags
//! in Phase-3.

use std::path::PathBuf;

use crate::error::{SloError, SloResult};

/// Scheme-aware locator (file:// / s3:// / gs://).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectLocator {
    File(PathBuf),
    S3 { bucket: String, key: String },
    Gs { bucket: String, key: String },
}

impl ObjectLocator {
    /// Parse a URL-style string.
    pub fn parse(s: &str) -> SloResult<Self> {
        if let Some(rest) = s.strip_prefix("file://") {
            return Ok(Self::File(PathBuf::from(rest)));
        }
        if let Some(rest) = s.strip_prefix("s3://") {
            let (bucket, key) = rest
                .split_once('/')
                .ok_or_else(|| SloError::Config(format!("s3:// missing key in {s}")))?;
            return Ok(Self::S3 {
                bucket: bucket.into(),
                key: key.into(),
            });
        }
        if let Some(rest) = s.strip_prefix("gs://") {
            let (bucket, key) = rest
                .split_once('/')
                .ok_or_else(|| SloError::Config(format!("gs:// missing key in {s}")))?;
            return Ok(Self::Gs {
                bucket: bucket.into(),
                key: key.into(),
            });
        }
        // Bare path → file://.
        Ok(Self::File(PathBuf::from(s)))
    }

    #[must_use]
    pub fn scheme(&self) -> &'static str {
        match self {
            Self::File(_) => "file",
            Self::S3 { .. } => "s3",
            Self::Gs { .. } => "gs",
        }
    }

    pub fn require_file(&self) -> SloResult<&PathBuf> {
        match self {
            Self::File(p) => Ok(p),
            other => Err(SloError::Config(format!(
                "scheme {} not supported in Phase-2 (use file://) — got {other:?}",
                other.scheme()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file_scheme() {
        let l = ObjectLocator::parse("file:///var/run/slos").unwrap();
        assert_eq!(l, ObjectLocator::File(PathBuf::from("/var/run/slos")));
    }

    #[test]
    fn parse_s3_scheme() {
        let l = ObjectLocator::parse("s3://my-bucket/slos/v0.6.0").unwrap();
        assert_eq!(l, ObjectLocator::S3 {
            bucket: "my-bucket".into(),
            key: "slos/v0.6.0".into(),
        });
    }

    #[test]
    fn parse_gs_scheme() {
        let l = ObjectLocator::parse("gs://bucket/slos").unwrap();
        assert_eq!(l, ObjectLocator::Gs { bucket: "bucket".into(), key: "slos".into() });
    }

    #[test]
    fn parse_bare_path_is_file_scheme() {
        let l = ObjectLocator::parse("/tmp/foo").unwrap();
        assert!(matches!(l, ObjectLocator::File(_)));
    }

    #[test]
    fn require_file_rejects_s3() {
        let l = ObjectLocator::S3 { bucket: "b".into(), key: "k".into() };
        let e = l.require_file().unwrap_err();
        assert!(matches!(e, SloError::Config(_)));
    }

    #[test]
    fn s3_without_key_errors() {
        let e = ObjectLocator::parse("s3://onlybucket").unwrap_err();
        assert!(matches!(e, SloError::Config(_)));
    }
}
