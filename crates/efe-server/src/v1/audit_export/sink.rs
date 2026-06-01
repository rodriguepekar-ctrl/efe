//! Sink type definitions + URL validation.

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SinkKind {
    S3,
    Gcs,
    Azure,
    Webhook,
}

impl SinkKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Gcs => "gcs",
            Self::Azure => "azure",
            Self::Webhook => "webhook",
        }
    }
}

impl FromStr for SinkKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "s3" => Ok(Self::S3),
            "gcs" => Ok(Self::Gcs),
            "azure" => Ok(Self::Azure),
            "webhook" => Ok(Self::Webhook),
            other => Err(format!("unknown sink kind : {}", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SinkStatus {
    Active,
    Degraded,
    Disabled,
}

impl SinkStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Degraded => "degraded",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Error)]
pub enum SinkUrlError {
    #[error("URL must use scheme matching the sink kind")]
    BadScheme,
    #[error("URL is malformed : {0}")]
    Malformed(String),
}

/// Validate that the URL matches the sink kind. Light validation —
/// the worker does full network-level validation at runtime.
pub fn validate_url(kind: SinkKind, url: &str) -> Result<(), SinkUrlError> {
    let prefix = match kind {
        SinkKind::S3 => "s3://",
        SinkKind::Gcs => "gs://",
        SinkKind::Azure => "az://",
        SinkKind::Webhook => "https://",
    };
    if !url.starts_with(prefix) {
        return Err(SinkUrlError::BadScheme);
    }
    if url.len() < prefix.len() + 3 {
        return Err(SinkUrlError::Malformed("URL too short".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sink_kind_parse() {
        assert_eq!("s3".parse::<SinkKind>().unwrap(), SinkKind::S3);
        assert_eq!("S3".parse::<SinkKind>().unwrap(), SinkKind::S3);
        assert_eq!("webhook".parse::<SinkKind>().unwrap(), SinkKind::Webhook);
        assert!("bogus".parse::<SinkKind>().is_err());
    }

    #[test]
    fn url_validation_s3() {
        assert!(validate_url(SinkKind::S3, "s3://bucket/prefix").is_ok());
        assert!(matches!(
            validate_url(SinkKind::S3, "https://bucket/prefix"),
            Err(SinkUrlError::BadScheme)
        ));
    }

    #[test]
    fn url_validation_webhook_requires_https() {
        assert!(validate_url(SinkKind::Webhook, "https://example.com/sink").is_ok());
        assert!(matches!(
            validate_url(SinkKind::Webhook, "http://example.com/sink"),
            Err(SinkUrlError::BadScheme)
        ));
    }

    #[test]
    fn url_validation_too_short() {
        assert!(matches!(
            validate_url(SinkKind::S3, "s3://"),
            Err(SinkUrlError::Malformed(_))
        ));
    }
}
