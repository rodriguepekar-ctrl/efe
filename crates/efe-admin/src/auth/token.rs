//! Bearer-token provider.

use std::path::PathBuf;
use std::sync::Arc;

use crate::error::AdminError;

const KUBE_SA_PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

#[derive(Clone)]
pub struct TokenProvider {
    inner: Arc<TokenInner>,
}

enum TokenInner {
    None,
    Static(String),
    File(PathBuf),
}

impl TokenProvider {
    #[must_use]
    pub fn none() -> Self {
        Self { inner: Arc::new(TokenInner::None) }
    }

    pub fn file(path: PathBuf) -> Result<Self, AdminError> {
        if !path.exists() {
            return Err(AdminError::AuthFailed(format!(
                "token file not found: {}",
                path.display()
            )));
        }
        Ok(Self { inner: Arc::new(TokenInner::File(path)) })
    }

    pub fn kube_sa() -> Result<Self, AdminError> {
        let p = PathBuf::from(KUBE_SA_PATH);
        if !p.exists() {
            return Err(AdminError::AuthFailed(
                "kube-sa: not running inside a cluster (no service-account token mounted)"
                    .into(),
            ));
        }
        Ok(Self { inner: Arc::new(TokenInner::File(p)) })
    }

    /// Construct from a raw token string (used in tests).
    #[must_use]
    pub fn from_static(token: impl Into<String>) -> Self {
        Self { inner: Arc::new(TokenInner::Static(token.into())) }
    }

    /// Returns the current bearer token, freshly read each call (for
    /// File-backed providers). `None` means « no auth header at all ».
    pub fn bearer(&self) -> Result<Option<String>, AdminError> {
        match self.inner.as_ref() {
            TokenInner::None => Ok(None),
            TokenInner::Static(s) => Ok(Some(s.clone())),
            TokenInner::File(p) => {
                let raw = std::fs::read_to_string(p).map_err(|e| {
                    AdminError::AuthFailed(format!("read {}: {e}", p.display()))
                })?;
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    return Err(AdminError::AuthFailed("token file is empty".into()));
                }
                Ok(Some(trimmed))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn none_returns_no_token() {
        let p = TokenProvider::none();
        assert!(p.bearer().unwrap().is_none());
    }

    #[test]
    fn static_returns_token() {
        let p = TokenProvider::from_static("abc123");
        assert_eq!(p.bearer().unwrap(), Some("abc123".to_string()));
    }

    #[test]
    fn file_token_strips_whitespace() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"  mytoken\n").unwrap();
        let p = TokenProvider::file(tmp.path().into()).unwrap();
        assert_eq!(p.bearer().unwrap(), Some("mytoken".to_string()));
    }

    #[test]
    fn file_missing_errors() {
        let res = TokenProvider::file("/nonexistent/path".into());
        assert!(matches!(res, Err(AdminError::AuthFailed(_))));
    }
}
