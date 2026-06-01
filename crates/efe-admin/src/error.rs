//! Typed errors for `efe-admin`. Each variant maps to a documented
//! exit code (see `main.rs`). Network/IO/JSON errors collapse via
//! `From` impls to keep handlers concise.

use thiserror::Error;

/// All errors surfaced by the CLI. Variants intentionally narrow so
/// the binary can return the right sysexits-compliant exit code.
#[derive(Debug, Error)]
pub enum AdminError {
    /// User pressed N at confirmation prompt.
    #[error("user aborted")]
    UserAbort,

    /// Bad input from user (slug shape, missing required, etc).
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Auth failure (token expired, IdP rejection, missing scope).
    #[error("auth failed: {0}")]
    AuthFailed(String),

    /// Backend returned a structured error.
    #[error("backend error [{code}]: {message}")]
    Backend {
        /// Stable error code from the backend (e.g. `conflict`,
        /// `not_found`, `confirm_required`).
        code: String,
        /// Human-readable message.
        message: String,
    },

    /// Config load/parse failure.
    #[error("config error: {0}")]
    Config(String),

    /// Network/HTTP failure (transport-level).
    #[error("network error: {0}")]
    Network(String),

    /// IO failure (file read, write, etc).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization failure.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// YAML serialization failure.
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// TOML serialization failure.
    #[error("toml error: {0}")]
    Toml(String),

    /// Unexpected internal invariant violation. Should never happen.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<reqwest::Error> for AdminError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network(e.to_string())
    }
}

impl From<toml::de::Error> for AdminError {
    fn from(e: toml::de::Error) -> Self {
        Self::Toml(e.to_string())
    }
}

impl From<toml::ser::Error> for AdminError {
    fn from(e: toml::ser::Error) -> Self {
        Self::Toml(e.to_string())
    }
}

impl AdminError {
    /// Construct a `Backend` error from a `(code, message)` pair.
    #[must_use]
    pub fn backend(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Backend {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Helper for tests / handlers.
    #[must_use]
    pub fn invalid<S: Into<String>>(s: S) -> Self {
        Self::InvalidInput(s.into())
    }
}
