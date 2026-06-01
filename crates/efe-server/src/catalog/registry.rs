//! Trait + value types for the catalog registry.
//!
//! These are pure Rust types — they intentionally do NOT depend
//! on the generated `tonic::include_proto!` types. The proto layer
//! converts in/out at the gRPC handler boundary (deferred to
//! D12-bis). Decoupling the storage surface from the wire format
//! lets us swap impls (in-memory, postgres, sled, …) without
//! touching the proto bindings, and lets unit tests construct
//! values without going through prost's strict validation.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------
// Canonical key
// ---------------------------------------------------------------------

/// Canonical identifier of a registered catalog version.
///
/// Format on the wire : `"<tenant>/<name>@v<version>"`. The integer
/// is monotonic per `(tenant, name)` ; the server assigns it on
/// register/evolve.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CatalogKey {
    /// Owning tenant (matches the JWT `tenant_id` claim).
    pub tenant: String,
    /// Catalog name, lowercase regex `[a-z][a-z0-9_-]{0,62}`.
    pub name: String,
    /// Integer version, monotonic per `(tenant, name)`.
    pub version: u32,
}

impl CatalogKey {
    /// Render as the wire-form canonical key `"tenant/name@vN"`.
    #[must_use]
    pub fn to_wire(&self) -> String {
        format!("{}/{}@v{}", self.tenant, self.name, self.version)
    }

    /// Parse the wire-form canonical key. Accepts both `"...@vN"` and
    /// `"...@N"` (the SDK is lenient).
    pub fn parse(wire: &str) -> Result<Self, CatalogError> {
        let (tenant, rest) = wire.split_once('/').ok_or_else(|| {
            CatalogError::InvalidArgument(format!("key {wire:?} missing '/' separator"))
        })?;
        let (name, version_str) = rest.rsplit_once('@').ok_or_else(|| {
            CatalogError::InvalidArgument(format!("key {wire:?} missing '@vN' segment"))
        })?;
        if tenant.is_empty() || name.is_empty() || version_str.is_empty() {
            return Err(CatalogError::InvalidArgument(format!(
                "key {wire:?} has an empty segment"
            )));
        }
        let trimmed = version_str
            .strip_prefix('v')
            .or_else(|| version_str.strip_prefix('V'))
            .unwrap_or(version_str);
        let version: u32 = trimmed.parse().map_err(|_| {
            CatalogError::InvalidArgument(format!(
                "version segment {trimmed:?} of key {wire:?} is not an integer"
            ))
        })?;
        if version < 1 {
            return Err(CatalogError::InvalidArgument(format!(
                "version of key {wire:?} must be ≥ 1"
            )));
        }
        Ok(Self {
            tenant: tenant.to_owned(),
            name: name.to_owned(),
            version,
        })
    }
}

// ---------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------

/// Embedder model declaration. Mirror of `efe.v1.catalog.EmbedderSpec`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedderSpec {
    /// Embedder family, e.g. `"sentence-transformers"`.
    pub family: String,
    /// Model name within the family, e.g. `"paraphrase-multilingual-MiniLM-L12-v2"`.
    pub model: String,
    /// Output embedding dimension (must match the model).
    pub dim: u32,
    /// Normalisation policy, e.g. `"l2"`.
    pub normalize: String,
}

/// One class of the catalog. Mirror of `efe.v1.catalog.CatalogClass`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogClass {
    /// Class name, lowercase regex `[a-z][a-z0-9_-]{0,62}`.
    pub name: String,
    /// Free-form description.
    pub description: String,
    /// Canonical example sentences for this class (≥ 1 entry).
    pub canonical_sentences: Vec<String>,
    /// Optional UI color hint.
    pub color: String,
    /// True if the class is retired but kept for backward compatibility.
    pub deprecated: bool,
}

/// Cone policy. Mirror of `efe.v1.catalog.ConePolicy`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConePolicy {
    /// Max cosine distance from the cone axis (∈ (0, 1]).
    pub max_projection_distance: f32,
}

/// Confidence threshold policy. Mirror of `efe.v1.catalog.ThresholdPolicy`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThresholdPolicy {
    /// "High confidence" cutoff (≥ `confidence_medium`).
    pub confidence_high: f32,
    /// "Medium confidence" cutoff (> 0).
    pub confidence_medium: f32,
}

/// Full content body. Mirror of `efe.v1.catalog.CatalogContent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogContent {
    /// Embedder declaration.
    pub embedder: EmbedderSpec,
    /// Class definitions (≥ 2 entries).
    pub classes: Vec<CatalogClass>,
    /// Cone-policy parameters.
    pub cone_policy: ConePolicy,
    /// Confidence thresholds.
    pub thresholds: ThresholdPolicy,
}

// ---------------------------------------------------------------------
// Audit metadata
// ---------------------------------------------------------------------

/// Hex-encoded content digest. Witness against mid-flight tampering.
///
/// In v0.1.12 (scaffolding), the in-memory impl stores a placeholder
/// digest (`"placeholder-<len>"`). D12-bis will swap in a real
/// SHA-256 once `sha2` is added to the crate's deps and the gRPC
/// handler is wired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDigest(pub String);

impl ContentDigest {
    /// Render as a hex string suitable for the wire.
    #[must_use]
    pub fn as_hex(&self) -> &str {
        &self.0
    }
}

/// Audit envelope stamped at register/evolve time. Mirror of
/// `efe.v1.catalog.CatalogMetadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogMetadata {
    /// Unix epoch milliseconds — gRPC handler converts to/from
    /// `google.protobuf.Timestamp`.
    pub created_at_unix_ms: i64,
    /// `sub` claim of the JWT.
    pub created_by: String,
    /// Free-text operator-supplied note.
    pub description: String,
    /// Canonical key of the previous version (empty for v1).
    pub parent_key: String,
    /// `"register"` or one of the 5 evolve op tags.
    pub evolution_op: String,
    /// Free-form key-value labels.
    pub labels: BTreeMap<String, String>,
    /// Hex-encoded content digest.
    pub content_sha256: ContentDigest,
}

// ---------------------------------------------------------------------
// Compiled class
// ---------------------------------------------------------------------

/// Server-side compiled anchors. Mirror of `efe.v1.catalog.CompiledClass`.
///
/// Currently a placeholder ; the actual anchor blob bytes are
/// produced by the embedding pool (out of scope for D12 scaffolding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledClass {
    /// Class name (matches `CatalogClass.name`).
    pub class_name: String,
    /// Row-major float32 little-endian (N × dim) anchors.
    pub anchors_blob: Vec<u8>,
    /// Number of anchor rows.
    pub num_anchors: u32,
    /// Embedder fingerprint (model+dim+normalize) under which the
    /// blob was compiled.
    pub embedder_fingerprint: String,
}

// ---------------------------------------------------------------------
// Returned form
// ---------------------------------------------------------------------

/// Full registered catalog : content + audit + (optional) compiled.
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredCatalog {
    /// Canonical key (`tenant/name@vN`).
    pub key: CatalogKey,
    /// YAML-shaped content.
    pub content: CatalogContent,
    /// Audit envelope.
    pub metadata: CatalogMetadata,
    /// Per-class compiled anchors, populated only when the caller
    /// passed `include_compiled=true` AND has the read scope.
    pub compiled: Option<Vec<CompiledClass>>,
}

/// Lightweight summary returned by `list_catalogs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogRefSummary {
    /// Owning tenant.
    pub tenant: String,
    /// Catalog name.
    pub name: String,
    /// Highest integer version known for this `(tenant, name)`.
    pub latest_version: u32,
    /// Number of classes in the latest version.
    pub num_classes: u32,
    /// Aggregate number of anchor sentences across all classes.
    pub num_anchors: u32,
}

// ---------------------------------------------------------------------
// Evolution ops
// ---------------------------------------------------------------------

/// Discriminated tag for the 5 evolve operations.
#[derive(Debug, Clone, PartialEq)]
pub enum EvolveOp {
    /// Append a new class to the catalog.
    AppendClass {
        /// The class to add.
        class: CatalogClass,
    },
    /// Remove a class by name.
    RemoveClass {
        /// Name of the class to remove.
        class_name: String,
    },
    /// Replace the threshold policy.
    UpdateThresholds {
        /// New thresholds.
        thresholds: ThresholdPolicy,
    },
    /// Replace one class's canonical sentences (re-embeds only this class).
    ReplaceClassAnchors {
        /// Target class name.
        class_name: String,
        /// New canonical sentences (≥ 1).
        canonical_sentences: Vec<String>,
    },
    /// Recompile every class against a new embedder.
    Recompile {
        /// New embedder declaration.
        new_embedder: EmbedderSpec,
    },
}

impl EvolveOp {
    /// Audit tag used in `CatalogMetadata.evolution_op`.
    #[must_use]
    pub fn audit_tag(&self) -> &'static str {
        match self {
            Self::AppendClass { .. } => "append_class",
            Self::RemoveClass { .. } => "remove_class",
            Self::UpdateThresholds { .. } => "update_thresholds",
            Self::ReplaceClassAnchors { .. } => "replace_class_anchors",
            Self::Recompile { .. } => "recompile",
        }
    }
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// Error type returned by every `CatalogRegistry` method.
///
/// The gRPC handler maps these to gRPC `Code` values per the
/// architecture document § 7 :
///
/// | Variant            | gRPC code              |
/// |--------------------|------------------------|
/// | `NotFound`         | `NotFound`             |
/// | `InvalidArgument`  | `InvalidArgument`      |
/// | `PermissionDenied` | `PermissionDenied`     |
/// | `QuotaExceeded`    | `ResourceExhausted`    |
/// | `Conflict`         | `FailedPrecondition`   |
/// | `Internal(_)`      | `Internal`             |
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum CatalogError {
    /// The requested key (or `(tenant, name)`) is not registered.
    #[error("catalog not found: {0}")]
    NotFound(String),

    /// Schema or domain invariant violated by the request.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Caller's JWT lacks the required scope or tenant claim.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Per-tenant quota hit (names/versions/classes).
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    /// Operation conflicts with the current state (e.g. delete of a
    /// referenced version without `force`).
    #[error("conflict: {0}")]
    Conflict(String),

    /// Unexpected server-side failure ; not retryable from the
    /// client's perspective.
    #[error("internal: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------

/// Storage abstraction for versioned catalogs.
///
/// All methods are async + `Send + Sync` so the trait object can
/// live in a `tonic` handler's shared state (`Arc<dyn ...>`).
///
/// # Tenant scoping
///
/// The trait does NOT enforce tenant scoping ; that's the gRPC
/// handler's job (it checks the JWT's `tenant_id` claim against
/// the request's `tenant` field BEFORE delegating to the registry).
/// Bypassing the handler — as integration tests sometimes do — is
/// allowed for testing convenience but means the test must
/// construct keys with the right tenant.
#[async_trait]
pub trait CatalogRegistry: Send + Sync {
    /// Register a new catalog version. Atomic. Assigns the next
    /// integer version monotonically per `(tenant, name)`.
    async fn register(
        &self,
        tenant: &str,
        name: &str,
        content: CatalogContent,
        metadata: CatalogMetadata,
    ) -> Result<RegisteredCatalog, CatalogError>;

    /// Apply an evolution op, producing a new version (incremented
    /// +1 from the source).
    async fn evolve(
        &self,
        from: &CatalogKey,
        op: EvolveOp,
        metadata_override: CatalogMetadata,
    ) -> Result<RegisteredCatalog, CatalogError>;

    /// Fetch one catalog version. With `include_compiled=true`,
    /// the response carries per-class compiled anchor metadata.
    async fn get(
        &self,
        key: &CatalogKey,
        include_compiled: bool,
    ) -> Result<RegisteredCatalog, CatalogError>;

    /// List one row per `(tenant, name)` for the given tenant.
    /// `page_token` is an opaque continuation cursor ; in-memory
    /// impls may ignore it (returning all rows in one page).
    async fn list_catalogs(
        &self,
        tenant: &str,
        page_size: u32,
        page_token: Option<&str>,
    ) -> Result<CatalogPage, CatalogError>;

    /// List all versions for `(tenant, name)`. Sorted ascending.
    async fn list_versions(
        &self,
        tenant: &str,
        name: &str,
    ) -> Result<Vec<CatalogKey>, CatalogError>;

    /// Delete a single version. With `force=true`, cascade-delete
    /// every transitive descendant (any version whose parent chain
    /// reaches `key`).
    ///
    /// Audit attribution requires the caller to thread the JWT
    /// `sub` claim through the `principal` parameter — the trait
    /// itself doesn't see the JWT (the gRPC handler is the only
    /// component that has it). For tests / direct callers,
    /// `"(test)"` or any sentinel string is acceptable.
    async fn delete(
        &self,
        key: &CatalogKey,
        force: bool,
        principal: &str,
    ) -> Result<(), CatalogError>;
}

/// Pagination envelope for `list_catalogs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPage {
    /// Page entries (one summary per `(tenant, name)`).
    pub catalogs: Vec<CatalogRefSummary>,
    /// Empty when the page is the last.
    pub next_page_token: String,
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_canonical_key_round_trips() {
        let k = CatalogKey {
            tenant: "acme".into(),
            name: "billing".into(),
            version: 7,
        };
        let wire = k.to_wire();
        assert_eq!(wire, "acme/billing@v7");
        assert_eq!(CatalogKey::parse(&wire).unwrap(), k);
    }

    #[test]
    fn parse_canonical_key_v_prefix_optional() {
        let k = CatalogKey::parse("acme/billing@7").unwrap();
        assert_eq!(k.version, 7);
        let k2 = CatalogKey::parse("acme/billing@V42").unwrap();
        assert_eq!(k2.version, 42);
    }

    #[test]
    fn parse_canonical_key_rejects_malformed() {
        assert!(matches!(
            CatalogKey::parse("no-at-sign"),
            Err(CatalogError::InvalidArgument(_)),
        ));
        assert!(matches!(
            CatalogKey::parse("acme/billing@vX"),
            Err(CatalogError::InvalidArgument(_)),
        ));
        assert!(matches!(
            CatalogKey::parse("acme/billing@v0"),
            Err(CatalogError::InvalidArgument(_)),
        ));
        assert!(matches!(
            CatalogKey::parse("/billing@v1"),
            Err(CatalogError::InvalidArgument(_)),
        ));
    }

    #[test]
    fn evolve_op_audit_tag_matches_proto_oneof_names() {
        let cls = CatalogClass {
            name: "x".into(),
            description: String::new(),
            canonical_sentences: vec!["s".into()],
            color: String::new(),
            deprecated: false,
        };
        assert_eq!(
            EvolveOp::AppendClass { class: cls.clone() }.audit_tag(),
            "append_class"
        );
        assert_eq!(
            EvolveOp::RemoveClass {
                class_name: "x".into()
            }
            .audit_tag(),
            "remove_class"
        );
        assert_eq!(
            EvolveOp::UpdateThresholds {
                thresholds: ThresholdPolicy {
                    confidence_high: 0.8,
                    confidence_medium: 0.5
                },
            }
            .audit_tag(),
            "update_thresholds",
        );
        assert_eq!(
            EvolveOp::ReplaceClassAnchors {
                class_name: "x".into(),
                canonical_sentences: vec!["s".into()],
            }
            .audit_tag(),
            "replace_class_anchors",
        );
        assert_eq!(
            EvolveOp::Recompile {
                new_embedder: EmbedderSpec {
                    family: "f".into(),
                    model: "m".into(),
                    dim: 4,
                    normalize: "l2".into(),
                },
            }
            .audit_tag(),
            "recompile",
        );
    }
}
