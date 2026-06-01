//! Schema validation + per-tenant quotas.
//!
//! The trait surface in `registry::CatalogRegistry` does NOT inline
//! validation — instead, every impl is expected to call
//! `CatalogPolicy::validate_register` / `validate_evolve` before
//! committing the new version. This keeps the policy in one place
//! and identical across in-memory + (future) postgres impls.

use std::collections::HashMap;

use thiserror::Error;

use super::registry::{CatalogClass, CatalogContent, CatalogError, EvolveOp, ThresholdPolicy};

// ---------------------------------------------------------------------
// Tunable defaults (architecture doc § 11)
// ---------------------------------------------------------------------

/// Max number of `(tenant, name)` tuples per tenant. Architecture
/// document § 11 — tunable via `EFE_CATALOG_MAX_NAMES_PER_TENANT`.
pub const DEFAULT_MAX_NAMES_PER_TENANT: u32 = 100;

/// Max number of versions per `(tenant, name)`. Tunable via
/// `EFE_CATALOG_MAX_VERSIONS_PER_NAME`.
pub const DEFAULT_MAX_VERSIONS_PER_NAME: u32 = 1000;

/// Max number of classes per catalog. Tunable via
/// `EFE_CATALOG_MAX_CLASSES_PER_CATALOG`.
pub const DEFAULT_MAX_CLASSES_PER_CATALOG: u32 = 256;

/// Min number of classes per catalog. Hard-coded — a 1-class
/// catalog is degenerate.
pub const MIN_CLASSES_PER_CATALOG: u32 = 2;

/// Max number of canonical sentences per class. Tunable via
/// `EFE_CATALOG_MAX_SENTENCES_PER_CLASS`.
pub const DEFAULT_MAX_SENTENCES_PER_CLASS: u32 = 64;

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// Policy-side errors — raised before the storage layer is touched.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("schema invariant violated: {0}")]
    Schema(String),

    #[error("quota exceeded: {0}")]
    Quota(String),
}

impl From<PolicyError> for CatalogError {
    fn from(err: PolicyError) -> Self {
        match err {
            PolicyError::Schema(m) => Self::InvalidArgument(m),
            PolicyError::Quota(m) => Self::QuotaExceeded(m),
        }
    }
}

// ---------------------------------------------------------------------
// Policy bundle
// ---------------------------------------------------------------------

/// Tunable per-deployment policy.
#[derive(Debug, Clone)]
pub struct CatalogPolicy {
    pub max_names_per_tenant: u32,
    pub max_versions_per_name: u32,
    pub max_classes_per_catalog: u32,
    pub max_sentences_per_class: u32,
}

impl Default for CatalogPolicy {
    fn default() -> Self {
        Self {
            max_names_per_tenant: DEFAULT_MAX_NAMES_PER_TENANT,
            max_versions_per_name: DEFAULT_MAX_VERSIONS_PER_NAME,
            max_classes_per_catalog: DEFAULT_MAX_CLASSES_PER_CATALOG,
            max_sentences_per_class: DEFAULT_MAX_SENTENCES_PER_CLASS,
        }
    }
}

impl CatalogPolicy {
    // -----------------------------------------------------------------
    //  Schema validation (cheap, applied first)
    // -----------------------------------------------------------------

    /// **D14 audit fix C1** — validate that a tenant or catalog name
    /// matches the canonical regex `^[a-z0-9_-]{1,63}$`.
    ///
    /// Rationale : the parent_key CHECK constraint in
    /// `migrations/0001_init.sql` enforces this regex on the parent_key
    /// column. Without a matching invariant on `tenant` / `name`
    /// themselves, an operator could register `tenant="ACME"` v1
    /// successfully (parent_key='' passes the check) then have
    /// `evolve` fail with an opaque CHECK violation when v2 tries to
    /// insert `parent_key = "ACME/billing@v1"`.
    ///
    /// We refuse the register up-front with a clear `InvalidArgument`
    /// rather than letting the inconsistency surface later. Defense in
    /// depth at the SQL layer is added in migration `0002_*` (CHECK
    /// constraints `catalogs_tenant_format` + `catalogs_name_format`).
    pub fn validate_identifier(field: &str, value: &str) -> Result<(), PolicyError> {
        if value.is_empty() {
            return Err(PolicyError::Schema(format!("{field} must be non-empty")));
        }
        if value.len() > 63 {
            return Err(PolicyError::Schema(format!(
                "{field} {value:?} is {} chars long, max is 63",
                value.len()
            )));
        }
        // Regex `^[a-z0-9_-]+$` ; we hand-roll to avoid the regex dep.
        // First char must be `[a-z0-9_-]` (we don't even require leading
        // letter because tenant_id strings often start with a digit).
        for (i, b) in value.bytes().enumerate() {
            let ok = matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-');
            if !ok {
                return Err(PolicyError::Schema(format!(
                    "{field} {value:?} contains invalid byte {b:#x} at offset {i}; \
                     allowed: lowercase ascii [a-z0-9_-]",
                )));
            }
        }
        Ok(())
    }

    /// Validate the schema of a `CatalogContent`. Used both at
    /// register and after every evolve op.
    pub fn validate_content(&self, content: &CatalogContent) -> Result<(), PolicyError> {
        let n_classes = content.classes.len() as u32;
        if n_classes < MIN_CLASSES_PER_CATALOG {
            return Err(PolicyError::Schema(format!(
                "catalog must have ≥ {MIN_CLASSES_PER_CATALOG} classes (got {n_classes})"
            )));
        }
        if n_classes > self.max_classes_per_catalog {
            return Err(PolicyError::Schema(format!(
                "catalog has {n_classes} classes, max is {}",
                self.max_classes_per_catalog,
            )));
        }
        for cls in &content.classes {
            self.validate_class(cls)?;
        }
        // Every class name must be unique within the catalog.
        let mut seen: HashMap<&str, ()> = HashMap::with_capacity(content.classes.len());
        for cls in &content.classes {
            if seen.insert(cls.name.as_str(), ()).is_some() {
                return Err(PolicyError::Schema(format!(
                    "duplicate class name {:?}",
                    cls.name,
                )));
            }
        }
        self.validate_thresholds(&content.thresholds)?;
        if content.cone_policy.max_projection_distance <= 0.0
            || content.cone_policy.max_projection_distance > 1.0
        {
            return Err(PolicyError::Schema(format!(
                "cone_policy.max_projection_distance must be in (0, 1] (got {})",
                content.cone_policy.max_projection_distance,
            )));
        }
        if content.embedder.family.is_empty()
            || content.embedder.model.is_empty()
            || content.embedder.dim == 0
        {
            return Err(PolicyError::Schema(
                "embedder must carry non-empty family + model + dim ≥ 1".into(),
            ));
        }
        Ok(())
    }

    /// Validate a single class.
    pub fn validate_class(&self, cls: &CatalogClass) -> Result<(), PolicyError> {
        if cls.name.is_empty() {
            return Err(PolicyError::Schema("class.name must be non-empty".into()));
        }
        // Mirror of the regex `[a-z][a-z0-9_-]{0,62}` from the proto comment.
        let mut chars = cls.name.chars();
        let first = chars
            .next()
            .ok_or_else(|| PolicyError::Schema("class.name empty".into()))?;
        if !first.is_ascii_lowercase() {
            return Err(PolicyError::Schema(format!(
                "class.name {:?} must start with a-z",
                cls.name,
            )));
        }
        for c in chars {
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
                return Err(PolicyError::Schema(format!(
                    "class.name {:?} has invalid char {c:?}",
                    cls.name,
                )));
            }
        }
        if cls.name.len() > 63 {
            return Err(PolicyError::Schema(format!(
                "class.name {:?} length {} exceeds 63",
                cls.name,
                cls.name.len(),
            )));
        }
        let n_sent = cls.canonical_sentences.len() as u32;
        if n_sent == 0 {
            return Err(PolicyError::Schema(format!(
                "class {:?} must have ≥ 1 canonical sentence",
                cls.name,
            )));
        }
        if n_sent > self.max_sentences_per_class {
            return Err(PolicyError::Schema(format!(
                "class {:?} has {n_sent} sentences, max is {}",
                cls.name, self.max_sentences_per_class,
            )));
        }
        for (i, s) in cls.canonical_sentences.iter().enumerate() {
            if s.trim().is_empty() {
                return Err(PolicyError::Schema(format!(
                    "class {:?}.canonical_sentences[{i}] is empty / whitespace-only",
                    cls.name,
                )));
            }
        }
        Ok(())
    }

    /// Validate a `ThresholdPolicy` standalone (used by
    /// `UpdateThresholds`).
    pub fn validate_thresholds(&self, t: &ThresholdPolicy) -> Result<(), PolicyError> {
        if !(t.confidence_medium > 0.0 && t.confidence_medium < 1.0) {
            return Err(PolicyError::Schema(format!(
                "confidence_medium must be in (0, 1) (got {})",
                t.confidence_medium,
            )));
        }
        if !(t.confidence_high > 0.0 && t.confidence_high <= 1.0) {
            return Err(PolicyError::Schema(format!(
                "confidence_high must be in (0, 1] (got {})",
                t.confidence_high,
            )));
        }
        if t.confidence_high < t.confidence_medium {
            return Err(PolicyError::Schema(format!(
                "confidence_high ({}) must be ≥ confidence_medium ({})",
                t.confidence_high, t.confidence_medium,
            )));
        }
        Ok(())
    }

    // -----------------------------------------------------------------
    //  Quota enforcement (applied at the storage layer, post-schema)
    // -----------------------------------------------------------------

    /// Check whether a tenant is at the names quota.
    pub fn check_names_quota(&self, tenant: &str, current_n_names: u32) -> Result<(), PolicyError> {
        if current_n_names >= self.max_names_per_tenant {
            return Err(PolicyError::Quota(format!(
                "tenant {tenant:?} has {current_n_names} catalogs, max is {}",
                self.max_names_per_tenant,
            )));
        }
        Ok(())
    }

    /// Check whether a `(tenant, name)` is at the versions quota.
    pub fn check_versions_quota(
        &self,
        tenant: &str,
        name: &str,
        current_n_versions: u32,
    ) -> Result<(), PolicyError> {
        if current_n_versions >= self.max_versions_per_name {
            return Err(PolicyError::Quota(format!(
                "{tenant:?}/{name:?} has {current_n_versions} versions, max is {}",
                self.max_versions_per_name,
            )));
        }
        Ok(())
    }

    /// Validate an `EvolveOp` against an existing content. Used at
    /// the storage layer to compute the new content shape and reject
    /// bad ops cheaply (before allocating a new version slot).
    pub fn validate_evolve(
        &self,
        existing: &CatalogContent,
        op: &EvolveOp,
    ) -> Result<(), PolicyError> {
        match op {
            EvolveOp::AppendClass { class } => {
                self.validate_class(class)?;
                if existing.classes.iter().any(|c| c.name == class.name) {
                    return Err(PolicyError::Schema(format!(
                        "class {:?} already exists",
                        class.name,
                    )));
                }
                if existing.classes.len() as u32 + 1 > self.max_classes_per_catalog {
                    return Err(PolicyError::Schema(format!(
                        "appending would exceed max_classes_per_catalog {}",
                        self.max_classes_per_catalog,
                    )));
                }
                // NOTE — `CatalogClass` doesn't carry an embedder
                // declaration ; the embedder is at the
                // `CatalogContent` level. AppendClass therefore
                // CANNOT change the embedder fingerprint by
                // construction. If the operator wants a different
                // embedder, they must use `RECOMPILE` instead.
                Ok(())
            }
            EvolveOp::RemoveClass { class_name } => {
                if !existing.classes.iter().any(|c| &c.name == class_name) {
                    return Err(PolicyError::Schema(format!(
                        "class {class_name:?} not found"
                    )));
                }
                if (existing.classes.len() as u32).saturating_sub(1) < MIN_CLASSES_PER_CATALOG {
                    return Err(PolicyError::Schema(format!(
                        "removing {class_name:?} would leave the catalog with \
                         < {MIN_CLASSES_PER_CATALOG} classes"
                    )));
                }
                Ok(())
            }
            EvolveOp::UpdateThresholds { thresholds } => self.validate_thresholds(thresholds),
            EvolveOp::ReplaceClassAnchors {
                class_name,
                canonical_sentences,
            } => {
                if !existing.classes.iter().any(|c| &c.name == class_name) {
                    return Err(PolicyError::Schema(format!(
                        "class {class_name:?} not found"
                    )));
                }
                if canonical_sentences.is_empty() {
                    return Err(PolicyError::Schema(format!(
                        "REPLACE_CLASS_ANCHORS for {class_name:?} requires \
                         a non-empty canonical_sentences list"
                    )));
                }
                let n = canonical_sentences.len() as u32;
                if n > self.max_sentences_per_class {
                    return Err(PolicyError::Schema(format!(
                        "REPLACE_CLASS_ANCHORS for {class_name:?} would set \
                         {n} sentences, max is {}",
                        self.max_sentences_per_class,
                    )));
                }
                for (i, s) in canonical_sentences.iter().enumerate() {
                    if s.trim().is_empty() {
                        return Err(PolicyError::Schema(format!(
                            "canonical_sentences[{i}] is empty / whitespace-only"
                        )));
                    }
                }
                Ok(())
            }
            EvolveOp::Recompile { new_embedder } => {
                if new_embedder.family.is_empty()
                    || new_embedder.model.is_empty()
                    || new_embedder.dim == 0
                {
                    return Err(PolicyError::Schema(
                        "RECOMPILE.new_embedder must have non-empty family + \
                         model + dim ≥ 1"
                            .into(),
                    ));
                }
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::registry::{ConePolicy, EmbedderSpec};

    fn sample_class(name: &str) -> CatalogClass {
        CatalogClass {
            name: name.to_owned(),
            description: "demo".into(),
            canonical_sentences: vec!["one".into(), "two".into()],
            color: "#000".into(),
            deprecated: false,
        }
    }

    fn sample_content() -> CatalogContent {
        CatalogContent {
            embedder: EmbedderSpec {
                family: "st".into(),
                model: "mini".into(),
                dim: 384,
                normalize: "l2".into(),
            },
            classes: vec![sample_class("a"), sample_class("b")],
            cone_policy: ConePolicy {
                max_projection_distance: 0.7,
            },
            thresholds: ThresholdPolicy {
                confidence_high: 0.8,
                confidence_medium: 0.5,
            },
        }
    }

    #[test]
    fn happy_path() {
        let p = CatalogPolicy::default();
        p.validate_content(&sample_content()).unwrap();
    }

    #[test]
    fn rejects_one_class() {
        let p = CatalogPolicy::default();
        let mut c = sample_content();
        c.classes.pop();
        assert!(matches!(
            p.validate_content(&c),
            Err(PolicyError::Schema(_))
        ));
    }

    #[test]
    fn rejects_duplicate_class_name() {
        let p = CatalogPolicy::default();
        let mut c = sample_content();
        c.classes.push(sample_class("a"));
        let err = p.validate_content(&c).unwrap_err();
        assert!(format!("{err}").contains("duplicate"));
    }

    #[test]
    fn rejects_bad_class_name_chars() {
        let p = CatalogPolicy::default();
        let mut bad = sample_class("Bad!Name");
        bad.canonical_sentences = vec!["ok".into()];
        assert!(matches!(
            p.validate_class(&bad),
            Err(PolicyError::Schema(_))
        ));
    }

    #[test]
    fn rejects_inverted_thresholds() {
        let p = CatalogPolicy::default();
        let bad = ThresholdPolicy {
            confidence_high: 0.4,
            confidence_medium: 0.6,
        };
        assert!(matches!(
            p.validate_thresholds(&bad),
            Err(PolicyError::Schema(_))
        ));
    }

    #[test]
    fn evolve_append_rejects_duplicate() {
        let p = CatalogPolicy::default();
        let c = sample_content();
        let op = EvolveOp::AppendClass {
            class: sample_class("a"),
        };
        assert!(matches!(
            p.validate_evolve(&c, &op),
            Err(PolicyError::Schema(_))
        ));
    }

    #[test]
    fn evolve_remove_rejects_missing() {
        let p = CatalogPolicy::default();
        let c = sample_content();
        let op = EvolveOp::RemoveClass {
            class_name: "ghost".into(),
        };
        assert!(matches!(
            p.validate_evolve(&c, &op),
            Err(PolicyError::Schema(_))
        ));
    }

    #[test]
    fn evolve_remove_rejects_under_min() {
        let p = CatalogPolicy::default();
        let c = sample_content();
        let op = EvolveOp::RemoveClass {
            class_name: "a".into(),
        };
        // c has 2 classes ; removing one would leave 1 < MIN_CLASSES_PER_CATALOG=2.
        assert!(matches!(
            p.validate_evolve(&c, &op),
            Err(PolicyError::Schema(_))
        ));
    }

    #[test]
    fn quota_checks_trigger_at_bound() {
        let p = CatalogPolicy {
            max_names_per_tenant: 3,
            max_versions_per_name: 5,
            ..Default::default()
        };
        assert!(p.check_names_quota("acme", 2).is_ok());
        assert!(matches!(
            p.check_names_quota("acme", 3),
            Err(PolicyError::Quota(_)),
        ));
        assert!(p.check_versions_quota("acme", "billing", 4).is_ok());
        assert!(matches!(
            p.check_versions_quota("acme", "billing", 5),
            Err(PolicyError::Quota(_)),
        ));
    }
}
