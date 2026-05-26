// SPDX-License-Identifier: UNLICENSED
//! `ApiKey` — wrapper sécurisé pour clés API (invariant I14).

use std::fmt;

use crate::error::{CoordError, CoordResult};

/// Clé API en mémoire avec redaction automatique au Debug/Display.
///
/// La clé brute n'est exposée QUE via `expose()`, méthode explicite
/// utilisée uniquement au moment de construire la requête HTTP.
#[derive(Clone)]
pub struct ApiKey {
    inner: String,
}

impl ApiKey {
    /// Construit depuis une string (validation minimale).
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self { inner: key.into() }
    }

    /// Construit depuis une variable d'environnement.
    ///
    /// Erreur `Config` si :
    /// - La variable n'existe pas
    /// - La valeur est vide
    /// - Le préfixe attendu (`expected_prefix`) ne match pas
    pub fn from_env(var_name: &str, expected_prefix: &str) -> CoordResult<Self> {
        let v = std::env::var(var_name).map_err(|_| {
            CoordError::Config(format!("env var {var_name} not set"))
        })?;
        if v.is_empty() {
            return Err(CoordError::Config(format!("env var {var_name} is empty")));
        }
        if !v.starts_with(expected_prefix) {
            return Err(CoordError::Config(format!(
                "env var {var_name} does not start with expected prefix '{expected_prefix}'"
            )));
        }
        Ok(Self::new(v))
    }

    /// Expose la clé brute — à n'appeler QUE pour insérer dans un header HTTP.
    ///
    /// **JAMAIS** logger / printer / inclure dans une Error.
    pub fn expose(&self) -> &str {
        &self.inner
    }

    /// Suffixe redacté pour observabilité (les 4 derniers chars).
    #[must_use]
    pub fn suffix_redacted(&self) -> String {
        let n = self.inner.len();
        if n <= 4 {
            "***".to_string()
        } else {
            format!("***{}", &self.inner[n - 4..])
        }
    }
}

// Invariant I14 — Debug et Display redactent toujours.
impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey({})", self.suffix_redacted())
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.suffix_redacted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_full_key() {
        let k = ApiKey::new("sk-ant-abc1234567890xyz");
        let dbg = format!("{k:?}");
        assert!(!dbg.contains("abc1234567890"));
        assert!(dbg.contains("***"));
        assert!(dbg.contains("0xyz"), "should show last 4 chars: {dbg}");
    }

    #[test]
    fn display_redacts() {
        let k = ApiKey::new("AIzaSyABCD1234EFGH5678");
        let d = format!("{k}");
        assert!(!d.contains("ABCD1234"));
        assert!(d.contains("***"));
    }

    #[test]
    fn expose_returns_raw() {
        let k = ApiKey::new("sk-ant-secret");
        assert_eq!(k.expose(), "sk-ant-secret");
    }

    #[test]
    fn from_env_missing_var_errors() {
        // S'assure que la variable n'existe pas pour ce test.
        std::env::remove_var("EFE_TEST_NONEXISTENT_KEY_XYZ");
        let r = ApiKey::from_env("EFE_TEST_NONEXISTENT_KEY_XYZ", "sk-");
        assert!(matches!(r, Err(CoordError::Config(_))));
    }

    #[test]
    fn from_env_wrong_prefix_errors() {
        std::env::set_var("EFE_TEST_BAD_PREFIX", "wrong-format-key");
        let r = ApiKey::from_env("EFE_TEST_BAD_PREFIX", "sk-ant-");
        assert!(matches!(r, Err(CoordError::Config(_))));
        std::env::remove_var("EFE_TEST_BAD_PREFIX");
    }

    #[test]
    fn from_env_valid_succeeds() {
        std::env::set_var("EFE_TEST_GOOD_KEY", "sk-ant-fake-but-valid-format");
        let r = ApiKey::from_env("EFE_TEST_GOOD_KEY", "sk-ant-");
        assert!(r.is_ok());
        assert_eq!(r.unwrap().expose(), "sk-ant-fake-but-valid-format");
        std::env::remove_var("EFE_TEST_GOOD_KEY");
    }

    #[test]
    fn suffix_redacted_short_key() {
        let k = ApiKey::new("ab");
        assert_eq!(k.suffix_redacted(), "***");
    }
}
