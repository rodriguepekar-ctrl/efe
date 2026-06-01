//! `ConeKey` — identifiant stable, versionné, d'un cône dans le registre.
//!
//! Format canonique `tenant/task@v<version>`. Les composants `tenant` et
//! `task` sont des identifiants opaques respectant le regex
//! `[A-Za-z0-9_-]+`, bornés en longueur. La version est un `u32`
//! strictement positif — les versions commencent à 1, jamais 0.
//!
//! # Sérialisation
//!
//! `Serialize` / `Deserialize` produisent un object JSON
//! `{"tenant": ..., "task": ..., "version": N}`, stable et auditable.
//! Pour la forme canonique compacte, utiliser son impl [`std::fmt::Display`]
//! (c.-à-d. `key.to_string()` ou `format!("{key}")`).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};

/// Longueur maximale d'un `tenant`.
pub const TENANT_MAX_LEN: usize = 64;
/// Longueur maximale d'un `task`.
pub const TASK_MAX_LEN: usize = 128;

/// Identifiant stable d'un cône : `(tenant, task, version)`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConeKey {
    /// Tenant logique (ex. `acme`).
    pub tenant: String,
    /// Tâche logique (ex. `chat-qa`).
    pub task: String,
    /// Version numérique (≥ 1). Monotone croissante par `(tenant, task)`.
    pub version: u32,
}

impl ConeKey {
    /// Construit une clé en validant tous les champs.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] si `tenant` ou `task` est vide,
    ///   excède la longueur maximale, ou contient un caractère illégal.
    /// - [`EfeError::InvalidConfig`] si `version == 0`.
    pub fn new(tenant: impl Into<String>, task: impl Into<String>, version: u32) -> Result<Self> {
        let tenant = tenant.into();
        let task = task.into();
        validate_component("tenant", &tenant, TENANT_MAX_LEN)?;
        validate_component("task", &task, TASK_MAX_LEN)?;
        if version == 0 {
            return Err(EfeError::InvalidConfig(
                "ConeKey: version must be >= 1 (versions start at 1)".into(),
            ));
        }
        Ok(Self {
            tenant,
            task,
            version,
        })
    }

    /// Parse la forme canonique `tenant/task@v<N>`.
    ///
    /// Accepté : `acme/chat-qa@v3`, `a/b@v1`, `_test_/_task_@v42`.
    /// Refusé : tout ce qui ne matche pas exactement le format.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] avec un message qui identifie la
    /// partie fautive (tenant, task, ou version).
    pub fn parse(s: &str) -> Result<Self> {
        // Format: <tenant>/<task>@v<version>
        let (tenant_task, version_part) = s.split_once("@v").ok_or_else(|| {
            EfeError::InvalidConfig(format!("ConeKey::parse: missing '@v' separator in {s:?}"))
        })?;
        let (tenant, task) = tenant_task.split_once('/').ok_or_else(|| {
            EfeError::InvalidConfig(format!("ConeKey::parse: missing '/' separator in {s:?}"))
        })?;
        // Rejette une forme "a/b/c@v1" explicitement — la validation
        // aval des caractères interdirait déjà le '/' dans `task` mais
        // avec un message peu clair.
        if task.contains('/') {
            return Err(EfeError::InvalidConfig(format!(
                "ConeKey::parse: too many '/' separators in {s:?}"
            )));
        }
        if version_part.is_empty() {
            return Err(EfeError::InvalidConfig(format!(
                "ConeKey::parse: missing version number after '@v' in {s:?}"
            )));
        }
        let version: u32 = version_part.parse().map_err(|_| {
            EfeError::InvalidConfig(format!(
                "ConeKey::parse: version {version_part:?} is not a valid u32"
            ))
        })?;
        Self::new(tenant, task, version)
    }

    /// Retourne la clé de regroupement `tenant/task` (sans version).
    #[must_use]
    pub fn task_id(&self) -> String {
        format!("{}/{}", self.tenant, self.task)
    }

    /// Retourne `(tenant, task)` en tuple de `&str`.
    #[must_use]
    pub fn task_tuple(&self) -> (&str, &str) {
        (self.tenant.as_str(), self.task.as_str())
    }
}

impl fmt::Display for ConeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}@v{}", self.tenant, self.task, self.version)
    }
}

fn validate_component(name: &'static str, value: &str, max_len: usize) -> Result<()> {
    if value.is_empty() {
        return Err(EfeError::InvalidConfig(format!(
            "ConeKey: {name} must not be empty"
        )));
    }
    if value.len() > max_len {
        return Err(EfeError::InvalidConfig(format!(
            "ConeKey: {name} length {} exceeds max {max_len}",
            value.len()
        )));
    }
    for c in value.chars() {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(EfeError::InvalidConfig(format!(
                "ConeKey: {name} {value:?} contains illegal char {c:?} \
                 (allowed: [A-Za-z0-9_-])"
            )));
        }
    }
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_happy_path() {
        let k = ConeKey::new("acme", "chat-qa", 3).unwrap();
        assert_eq!(k.tenant, "acme");
        assert_eq!(k.task, "chat-qa");
        assert_eq!(k.version, 3);
    }

    #[test]
    fn display_is_canonical() {
        let k = ConeKey::new("acme", "chat-qa", 7).unwrap();
        assert_eq!(k.to_string(), "acme/chat-qa@v7");
    }

    #[test]
    fn parse_roundtrip() {
        for s in ["acme/chat-qa@v1", "a/b@v42", "tenant_1/task-2@v999"] {
            let k = ConeKey::parse(s).unwrap();
            assert_eq!(k.to_string(), s, "roundtrip for {s}");
        }
    }

    #[test]
    fn parse_rejects_missing_at_v() {
        let err = ConeKey::parse("acme/chat-qa").unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn parse_rejects_missing_slash() {
        let err = ConeKey::parse("acme@v1").unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn parse_rejects_non_numeric_version() {
        let err = ConeKey::parse("acme/chat@vABC").unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_empty_tenant() {
        let err = ConeKey::new("", "t", 1).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_empty_task() {
        let err = ConeKey::new("a", "", 1).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_zero_version() {
        let err = ConeKey::new("a", "b", 0).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_illegal_chars() {
        for bad in ["ac.me", "a b", "a/c", "a@c", "a:c", "tenant!"] {
            let err = ConeKey::new(bad, "t", 1).unwrap_err();
            assert!(
                matches!(err, EfeError::InvalidConfig(_)),
                "should reject {bad}"
            );
        }
    }

    // Sécurité : non-ASCII (UTF-8 multi-byte) doit être rejeté — on ne
    // veut pas de clés avec accents / emojis / CJK qui pourraient
    // créer des collisions de normalisation ou exploiter des pipelines
    // de logs mal échappés.
    #[test]
    fn rejects_non_ascii_chars() {
        for bad in ["café", "tâsk", "中文", "🚀emoji", "año"] {
            let err = ConeKey::new(bad, "t", 1).unwrap_err();
            assert!(
                matches!(err, EfeError::InvalidConfig(_)),
                "should reject non-ASCII {bad}"
            );
        }
    }

    // parse("a/b/c@v1") — doit signaler "too many slashes", pas
    // un message confus sur les caractères.
    #[test]
    fn parse_rejects_multiple_slashes() {
        let err = ConeKey::parse("a/b/c@v1").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("too many") || msg.contains("illegal"),
            "got: {msg}"
        );
    }

    // parse("a/b@v") — doit signaler "missing version number".
    #[test]
    fn parse_rejects_empty_version() {
        let err = ConeKey::parse("a/b@v").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing version number"), "got: {msg}");
    }

    // parse("a/b@v0") — passe split_once mais ConeKey::new rejette la
    // version zéro.
    #[test]
    fn parse_rejects_version_zero() {
        let err = ConeKey::parse("a/b@v0").unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_too_long_tenant() {
        let long = "a".repeat(TENANT_MAX_LEN + 1);
        let err = ConeKey::new(long, "t", 1).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_too_long_task() {
        let long = "b".repeat(TASK_MAX_LEN + 1);
        let err = ConeKey::new("a", long, 1).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn task_id_format() {
        let k = ConeKey::new("acme", "chat-qa", 3).unwrap();
        assert_eq!(k.task_id(), "acme/chat-qa");
    }

    #[test]
    fn task_tuple_accessor() {
        let k = ConeKey::new("a", "b", 5).unwrap();
        assert_eq!(k.task_tuple(), ("a", "b"));
    }

    #[test]
    fn hash_eq_stable() {
        use std::collections::HashSet;
        let k1 = ConeKey::new("a", "b", 1).unwrap();
        let k2 = ConeKey::new("a", "b", 1).unwrap();
        let k3 = ConeKey::new("a", "b", 2).unwrap();
        let mut s: HashSet<ConeKey> = HashSet::new();
        s.insert(k1.clone());
        assert!(s.contains(&k2));
        assert!(!s.contains(&k3));
    }

    #[test]
    fn serde_json_roundtrip() {
        let k = ConeKey::new("acme", "chat-qa", 3).unwrap();
        let json = serde_json::to_string(&k).unwrap();
        let k2: ConeKey = serde_json::from_str(&json).unwrap();
        assert_eq!(k, k2);
    }
}
