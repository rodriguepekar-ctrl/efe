//! Sprint **D16** — `manifest.json` parsing + invariants.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 4.
//!
//! The manifest is the **autoritative** metadata blob inside every
//! D16 bundle. Its SHA-256 IS the canonical `bundle_fingerprint`
//! that catalogs (D12) bind themselves to.
//!
//! # Invariants enforced at parse time
//!
//! | INV | Field | Rule |
//! |---|---|---|
//! | INV-16-1 | `manifest_version` | == "1.0" |
//! | INV-16-3 | `dim` | ∈ canonical embedding dims (see below) |
//! | INV-16-4/5 | `framework` ↔ `components[].path` | onnx⇒weights/model.onnx ; safetensors⇒weights/model.safetensors |
//! | INV-16-6 | `runtime.max_seq_len` | ≤ 16384 |
//! | INV-16-7 | tokenizer present + required | tokenizer/tokenizer.json must be in components with required=true |
//! | INV-16-8 | `bundle_id` regex | matches §3.2 |
//!
//! Invariants needing filesystem access (INV-16-2, INV-16-9,
//! INV-16-10) live in `verify.rs` / `signature.rs`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hard-coded manifest schema version this binary supports.
/// Bump on incompatible schema changes (D17+).
pub const SUPPORTED_MANIFEST_VERSION: &str = "1.0";

/// Whitelist of supported embedding dimensions (powers of 2 +
/// the conventional 384/768/1024 of the sentence-transformers
/// family). Architecture § 4.3 INV-16-3.
pub const CANONICAL_DIMS: &[usize] = &[64, 128, 256, 384, 512, 768, 1024, 1536, 2048, 4096];

/// Maximum tokenizer truncation length (anti-DoS bound).
/// Architecture § 4.3 INV-16-6.
pub const MAX_SEQ_LEN_HARD_CAP: usize = 16_384;

// =====================================================================
// Manifest top-level
// =====================================================================

/// `manifest.json` shape — strict serde with `deny_unknown_fields`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Manifest schema version. Must equal
    /// [`SUPPORTED_MANIFEST_VERSION`] in v0.1.16.
    pub manifest_version: String,
    /// Canonical bundle id : `family/name@semver/framework/dim`.
    /// Architecture § 3.2.
    pub bundle_id: String,
    /// Weight format — `onnx` or `safetensors`.
    pub framework: Framework,
    /// Output embedding dimension. Pinned at fingerprint level.
    pub dim: usize,
    /// Per-file metadata (path + sha256 + size + required flag).
    pub components: Vec<Component>,
    /// Inference-time runtime parameters.
    pub runtime: RuntimeConfig,
    /// Build-time provenance (CI URL, git SHA, source-date-epoch).
    pub build: BuildInfo,
    /// License metadata (SPDX id + URL + redistribution flag).
    pub license: LicenseInfo,
    /// Optional model-origin trail (HF repo + revision).
    #[serde(default)]
    pub model_origin: Option<ModelOrigin>,
    /// Optional path to the SBOM file (relative to bundle root).
    #[serde(default)]
    pub sbom_path: Option<String>,
}

/// Allowed weight-format values.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Framework {
    /// ONNX format (preferred — runtime-portable, GPU EP support).
    Onnx,
    /// safetensors format (fallback — for HF-only models without
    /// official ONNX export).
    Safetensors,
}

impl Framework {
    /// Canonical lowercase name (matches the `bundle_id` segment).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Framework::Onnx => "onnx",
            Framework::Safetensors => "safetensors",
        }
    }

    /// Path the loader expects to find the weights at, relative to
    /// the bundle root.
    #[must_use]
    pub fn weights_path(self) -> &'static str {
        match self {
            Framework::Onnx => "weights/model.onnx",
            Framework::Safetensors => "weights/model.safetensors",
        }
    }
}

/// One file in the bundle, with its hash + required flag.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Component {
    /// Path relative to bundle root, e.g. `weights/model.onnx`.
    pub path: String,
    /// Lowercase hex SHA-256 of the file bytes.
    pub sha256: String,
    /// Byte size as written.
    pub size_bytes: u64,
    /// If true, [`super::verify`] fails when this file is absent
    /// or hash mismatches. If false, only logs a warning.
    pub required: bool,
}

/// Per-bundle inference configuration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Tokenizer truncation length. Tokens past this are silently
    /// dropped (and counted by `efe_embedder_tokenizer_truncations_total`).
    pub max_seq_len: usize,
    /// Token-pooling strategy applied to per-token outputs.
    pub pooling: Pooling,
    /// L2-normalise the output before return.
    pub normalize: bool,
    /// Output tensor dtype.
    pub dtype: Dtype,
}

/// Token-pooling strategy.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Pooling {
    /// Mean over non-padded tokens (most common — sentence-transformers).
    Mean,
    /// CLS-token output (BERT-style).
    Cls,
    /// Max over non-padded tokens.
    Max,
    /// Attention-weighted mean (some recent retrievers).
    WeightedMean,
}

/// Output tensor dtype.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Dtype {
    /// IEEE 754 single precision (default).
    Float32,
    /// IEEE 754 half precision (saves bandwidth ; may lose accuracy).
    Float16,
    /// Brain float (Google) — same range as f32, half precision.
    Bfloat16,
}

/// Build-time provenance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildInfo {
    /// URI of the CI run that produced the bundle.
    pub builder: String,
    /// `built_at_unix_ms = source_date_epoch * 1000` (reproducibility).
    pub built_at_unix_ms: u64,
    /// Git commit SHA of the EFE repo at build time (40 hex chars).
    pub git_sha: String,
    /// `SOURCE_DATE_EPOCH` used to fix `built_at_unix_ms`. Pinned
    /// for reproducibility tests.
    pub source_date_epoch: u64,
}

/// License metadata.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LicenseInfo {
    /// SPDX identifier, e.g. `Apache-2.0`, `MIT`, `OpenRAIL-M`.
    pub spdx_id: String,
    /// Optional URL pointing at the full license text.
    #[serde(default)]
    pub url: Option<String>,
    /// Whether the license permits redistribution. If `false`, the
    /// loader refuses to load the bundle in production unless the
    /// operator sets `EFE_BUNDLE_ACK_NONREDIST=true`.
    #[serde(default = "default_redist_true")]
    pub redistribution_allowed: bool,
}

fn default_redist_true() -> bool {
    true
}

/// Optional model provenance (where the weights came from).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelOrigin {
    /// HuggingFace repo identifier (`org/model_name`).
    #[serde(default)]
    pub huggingface_repo: Option<String>,
    /// Resolved revision SHA (40 hex chars).
    #[serde(default)]
    pub huggingface_revision: Option<String>,
    /// Model card URL.
    #[serde(default)]
    pub model_card_url: Option<String>,
}

// =====================================================================
// Errors
// =====================================================================

/// Manifest-level validation error. Surfaced by [`Manifest::validate`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    /// The `manifest_version` field doesn't equal
    /// [`SUPPORTED_MANIFEST_VERSION`].
    #[error("unsupported manifest_version {got:?} (this binary supports {expected:?})")]
    UnsupportedVersion {
        /// Actual value found.
        got: String,
        /// Value expected by the binary.
        expected: &'static str,
    },

    /// The `bundle_id` doesn't match the canonical regex
    /// `family/name@semver/framework/dim`.
    #[error("malformed bundle_id {0:?} (expected family/name@semver/framework/dim)")]
    MalformedBundleId(String),

    /// The `dim` is not in [`CANONICAL_DIMS`].
    #[error("dim {got} not in canonical set {expected:?}")]
    DimNotCanonical {
        /// Actual dim.
        got: usize,
        /// Allowed dims.
        expected: &'static [usize],
    },

    /// The `dim` parsed from `bundle_id` doesn't match the `dim`
    /// field — typo / build pipeline bug.
    #[error("dim mismatch : bundle_id says {from_id}, dim field says {from_field}")]
    DimMismatchInBundleId {
        /// Dim parsed from the bundle_id segment.
        from_id: usize,
        /// Dim from the explicit field.
        from_field: usize,
    },

    /// The `framework` parsed from `bundle_id` doesn't match the
    /// `framework` field.
    #[error(
        "framework mismatch : bundle_id says {from_id:?}, framework field says {from_field:?}"
    )]
    FrameworkMismatchInBundleId {
        /// Parsed from the bundle_id segment.
        from_id: String,
        /// From the explicit field.
        from_field: String,
    },

    /// `runtime.max_seq_len` exceeds [`MAX_SEQ_LEN_HARD_CAP`].
    #[error("max_seq_len {0} exceeds hard cap {MAX_SEQ_LEN_HARD_CAP}")]
    MaxSeqLenTooLarge(usize),

    /// `runtime.max_seq_len` is below the sane minimum (16).
    #[error("max_seq_len {0} below sane minimum 16")]
    MaxSeqLenTooSmall(usize),

    /// The weights file expected by the framework isn't listed in
    /// `components[]`.
    #[error("framework={framework:?} but components[] does not list {expected:?}")]
    WeightsMissingFromComponents {
        /// Framework declared by the manifest.
        framework: String,
        /// Expected path in components.
        expected: &'static str,
    },

    /// `tokenizer/tokenizer.json` isn't in `components[]` or isn't
    /// `required: true`. INV-16-7.
    #[error("tokenizer/tokenizer.json missing from components or marked required=false")]
    TokenizerMissing,

    /// Two `components[].path` are equal — the manifest is
    /// internally inconsistent.
    #[error("duplicate component path {0:?}")]
    DuplicateComponentPath(String),

    /// The `sbom_path` field references a path that isn't listed
    /// in `components[]`. The architecture intent (§ 3.1, § 6.5)
    /// is that the SBOM file IS in components so its bytes get
    /// SHA-256-verified at load time. Without this cross-check, a
    /// bundle could declare `sbom_path: "SBOM.json"` but ship no
    /// such file (or ship a tampered one), and the loader would
    /// silently accept it. **Audit fix D16-C4.**
    #[error(
        "sbom_path {sbom_path:?} is not listed in components[] — \
         the SBOM file MUST be a component so its hash is verified"
    )]
    SbomPathNotInComponents {
        /// The path declared by `sbom_path`.
        sbom_path: String,
    },

    /// A `components[].sha256` isn't 64 lowercase hex chars.
    #[error("invalid sha256 hex on component {path:?} : {got:?}")]
    InvalidSha256Hex {
        /// Component path.
        path: String,
        /// Actual value.
        got: String,
    },

    /// `build.git_sha` isn't 40 hex chars.
    #[error("invalid git_sha {0:?} (expected 40 hex chars)")]
    InvalidGitSha(String),

    /// `manifest.json` deserialised but a runtime constant has been
    /// reassigned (e.g. `dim = 0`).
    #[error("nonsensical value: {0}")]
    NonsensicalValue(String),
}

// =====================================================================
// Validation
// =====================================================================

impl Manifest {
    /// Run all in-memory invariants. Cf. architecture § 4.3.
    /// Returns the FIRST violation found (deterministic ordering).
    pub fn validate(&self) -> Result<(), ManifestError> {
        // INV-16-1.
        if self.manifest_version != SUPPORTED_MANIFEST_VERSION {
            return Err(ManifestError::UnsupportedVersion {
                got: self.manifest_version.clone(),
                expected: SUPPORTED_MANIFEST_VERSION,
            });
        }

        // INV-16-8 — bundle_id regex.
        let parsed = parse_bundle_id(&self.bundle_id)
            .map_err(|()| ManifestError::MalformedBundleId(self.bundle_id.clone()))?;
        if parsed.framework != self.framework.as_str() {
            return Err(ManifestError::FrameworkMismatchInBundleId {
                from_id: parsed.framework.to_string(),
                from_field: self.framework.as_str().to_string(),
            });
        }
        if parsed.dim != self.dim {
            return Err(ManifestError::DimMismatchInBundleId {
                from_id: parsed.dim,
                from_field: self.dim,
            });
        }

        // INV-16-3.
        if !CANONICAL_DIMS.contains(&self.dim) {
            return Err(ManifestError::DimNotCanonical {
                got: self.dim,
                expected: CANONICAL_DIMS,
            });
        }

        // INV-16-6 + sane minimum.
        if self.runtime.max_seq_len < 16 {
            return Err(ManifestError::MaxSeqLenTooSmall(self.runtime.max_seq_len));
        }
        if self.runtime.max_seq_len > MAX_SEQ_LEN_HARD_CAP {
            return Err(ManifestError::MaxSeqLenTooLarge(self.runtime.max_seq_len));
        }

        // INV-16-4/5 — weights file is in components.
        let weights_path = self.framework.weights_path();
        let mut seen_paths: HashSet<&str> = HashSet::new();
        let mut weights_seen = false;
        let mut tokenizer_seen_required = false;
        for c in &self.components {
            if !seen_paths.insert(c.path.as_str()) {
                return Err(ManifestError::DuplicateComponentPath(c.path.clone()));
            }
            // sha256 hex shape (64 chars, lowercase, hex).
            if !is_lowercase_hex_64(&c.sha256) {
                return Err(ManifestError::InvalidSha256Hex {
                    path: c.path.clone(),
                    got: c.sha256.clone(),
                });
            }
            if c.path == weights_path {
                weights_seen = true;
            }
            if c.path == "tokenizer/tokenizer.json" && c.required {
                tokenizer_seen_required = true;
            }
        }
        if !weights_seen {
            return Err(ManifestError::WeightsMissingFromComponents {
                framework: self.framework.as_str().to_string(),
                expected: weights_path,
            });
        }
        // INV-16-7.
        if !tokenizer_seen_required {
            return Err(ManifestError::TokenizerMissing);
        }

        // build.git_sha — 40 hex chars.
        if !is_lowercase_hex_n(&self.build.git_sha, 40) {
            return Err(ManifestError::InvalidGitSha(self.build.git_sha.clone()));
        }

        // **Audit fix D16-C4** — when sbom_path is set, the SBOM
        // file MUST also be in components[] so its bytes get
        // SHA-256-verified at load time. Otherwise the operator
        // could declare a path that points nowhere (or to a
        // tampered file) and the loader wouldn't catch it.
        if let Some(sbom_path) = &self.sbom_path {
            if !seen_paths.contains(sbom_path.as_str()) {
                return Err(ManifestError::SbomPathNotInComponents {
                    sbom_path: sbom_path.clone(),
                });
            }
        }

        // **Audit fix D16-M4** — note: the dim==0 check is now
        // redundant (caught earlier by `DimNotCanonical`) and
        // intentionally omitted. Defense in depth would re-add it
        // but the canonical-dim check provides better diagnostics.

        Ok(())
    }

    /// Convenience: parse + validate the manifest from raw JSON bytes.
    /// Returns the parsed [`Manifest`] AND the canonical bytes for
    /// downstream signature-verify and fingerprint computations.
    pub fn parse_and_validate(bytes: &[u8]) -> Result<Manifest, ManifestParseError> {
        let m: Manifest =
            serde_json::from_slice(bytes).map_err(|e| ManifestParseError::Json(e.to_string()))?;
        m.validate().map_err(ManifestParseError::Validation)?;
        Ok(m)
    }
}

/// Top-level error covering both serde parse + invariant violations.
#[derive(Debug, Error)]
pub enum ManifestParseError {
    /// JSON parsing failed (syntax / type / unknown field).
    #[error("manifest JSON parse: {0}")]
    Json(String),
    /// JSON OK but a manifest invariant was violated.
    #[error("manifest invariant violated: {0}")]
    Validation(#[from] ManifestError),
}

// =====================================================================
// Helpers
// =====================================================================

/// Result of parsing a `bundle_id` string into its 4 segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBundleId<'a> {
    /// `family/name` part.
    pub family_name: &'a str,
    /// `semver` part — three dot-separated unsigned integers.
    pub version: &'a str,
    /// `framework` part — typically `onnx` or `safetensors`.
    pub framework: &'a str,
    /// `dim` part as parsed integer.
    pub dim: usize,
}

/// Parse a `bundle_id` of the form `family/name@semver/framework/dim`.
///
/// Used by [`Manifest::validate`] to cross-check the
/// implicit (id) and explicit (`framework`, `dim`) values.
pub fn parse_bundle_id(s: &str) -> Result<ParsedBundleId<'_>, ()> {
    // family/name@semver/framework/dim
    let (family_at_ver, fmwk_dim) = s
        .rsplit_once('/')
        .and_then(|(a, dim)| {
            a.rsplit_once('/')
                .map(|(prefix, fmwk)| ((prefix, fmwk), dim))
        })
        .ok_or(())?;
    let (family_name_at_ver, framework) = family_at_ver;
    let (family_name, version) = family_name_at_ver.rsplit_once('@').ok_or(())?;
    let dim: usize = fmwk_dim.parse().map_err(|_| ())?;
    if family_name.is_empty() || version.is_empty() || framework.is_empty() || !is_semver_3(version)
    {
        return Err(());
    }
    // Architecture § 3.2 : family_name must contain at least one
    // slash (the `family/name` shape). A bare `x` would otherwise
    // pass the rest of the parser. Refuse.
    if !family_name.contains('/') {
        return Err(());
    }
    Ok(ParsedBundleId {
        family_name,
        version,
        framework,
        dim,
    })
}

/// Three dot-separated non-empty unsigned-integer components.
fn is_semver_3(s: &str) -> bool {
    let mut parts = s.split('.');
    let a = parts.next();
    let b = parts.next();
    let c = parts.next();
    let extra = parts.next();
    matches!(
        (a, b, c, extra),
        (Some(a), Some(b), Some(c), None)
            if !a.is_empty()
                && !b.is_empty()
                && !c.is_empty()
                && a.bytes().all(|x| x.is_ascii_digit())
                && b.bytes().all(|x| x.is_ascii_digit())
                && c.bytes().all(|x| x.is_ascii_digit())
    )
}

fn is_lowercase_hex_64(s: &str) -> bool {
    is_lowercase_hex_n(s, 64)
}

fn is_lowercase_hex_n(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn good_manifest() -> Manifest {
        Manifest {
            manifest_version: "1.0".into(),
            bundle_id: "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384".into(),
            framework: Framework::Onnx,
            dim: 384,
            components: vec![
                Component {
                    path: "weights/model.onnx".into(),
                    sha256: "0".repeat(64),
                    size_bytes: 100,
                    required: true,
                },
                Component {
                    path: "tokenizer/tokenizer.json".into(),
                    sha256: "1".repeat(64),
                    size_bytes: 50,
                    required: true,
                },
            ],
            runtime: RuntimeConfig {
                max_seq_len: 256,
                pooling: Pooling::Mean,
                normalize: true,
                dtype: Dtype::Float32,
            },
            build: BuildInfo {
                builder: "https://ci/run/123".into(),
                built_at_unix_ms: 1_745_875_200_000,
                git_sha: "0123456789abcdef0123456789abcdef01234567".into(),
                source_date_epoch: 1_745_875_200,
            },
            license: LicenseInfo {
                spdx_id: "Apache-2.0".into(),
                url: None,
                redistribution_allowed: true,
            },
            model_origin: None,
            sbom_path: None,
        }
    }

    #[test]
    fn validate_accepts_canonical_manifest() {
        good_manifest().validate().expect("happy path");
    }

    #[test]
    fn validate_rejects_unsupported_version() {
        let mut m = good_manifest();
        m.manifest_version = "2.0".into();
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::UnsupportedVersion { .. }));
    }

    #[test]
    fn validate_rejects_malformed_bundle_id() {
        let mut m = good_manifest();
        m.bundle_id = "not-a-bundle-id".into();
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::MalformedBundleId(_)));
    }

    #[test]
    fn validate_rejects_dim_not_canonical() {
        let mut m = good_manifest();
        m.bundle_id = "x/y@1.0.0/onnx/300".into();
        m.dim = 300;
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::DimNotCanonical { .. }));
    }

    #[test]
    fn validate_rejects_dim_mismatch_id_vs_field() {
        let mut m = good_manifest();
        // bundle_id says 384 but dim says 768.
        m.dim = 768;
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::DimMismatchInBundleId { .. }));
    }

    #[test]
    fn validate_rejects_framework_mismatch_id_vs_field() {
        let mut m = good_manifest();
        m.framework = Framework::Safetensors;
        // bundle_id still says "onnx" → mismatch
        let err = m.validate().unwrap_err();
        assert!(matches!(
            err,
            ManifestError::FrameworkMismatchInBundleId { .. }
        ));
    }

    #[test]
    fn validate_rejects_max_seq_len_too_large() {
        let mut m = good_manifest();
        m.runtime.max_seq_len = 32_000;
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::MaxSeqLenTooLarge(_)));
    }

    #[test]
    fn validate_rejects_max_seq_len_too_small() {
        let mut m = good_manifest();
        m.runtime.max_seq_len = 8;
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::MaxSeqLenTooSmall(_)));
    }

    #[test]
    fn validate_rejects_weights_missing_in_components() {
        let mut m = good_manifest();
        m.components.retain(|c| c.path != "weights/model.onnx");
        // Need to keep tokenizer required so we test ordering :
        // weights missing surfaces first in validate().
        let err = m.validate().unwrap_err();
        assert!(matches!(
            err,
            ManifestError::WeightsMissingFromComponents { .. }
        ));
    }

    #[test]
    fn validate_rejects_tokenizer_missing() {
        let mut m = good_manifest();
        m.components
            .retain(|c| c.path != "tokenizer/tokenizer.json");
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::TokenizerMissing));
    }

    #[test]
    fn validate_rejects_tokenizer_not_required() {
        let mut m = good_manifest();
        for c in &mut m.components {
            if c.path == "tokenizer/tokenizer.json" {
                c.required = false;
            }
        }
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::TokenizerMissing));
    }

    #[test]
    fn validate_rejects_duplicate_components() {
        let mut m = good_manifest();
        m.components.push(Component {
            path: "weights/model.onnx".into(),
            sha256: "2".repeat(64),
            size_bytes: 100,
            required: true,
        });
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::DuplicateComponentPath(_)));
    }

    #[test]
    fn validate_rejects_bad_sha256_hex() {
        let mut m = good_manifest();
        m.components[0].sha256 = "ZZZ_not_hex".into();
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::InvalidSha256Hex { .. }));
    }

    #[test]
    fn validate_rejects_bad_git_sha() {
        let mut m = good_manifest();
        m.build.git_sha = "abc".into();
        let err = m.validate().unwrap_err();
        assert!(matches!(err, ManifestError::InvalidGitSha(_)));
    }

    #[test]
    fn parse_and_validate_via_json() {
        let m = good_manifest();
        let bytes = serde_json::to_vec(&m).unwrap();
        let parsed = Manifest::parse_and_validate(&bytes).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn parse_rejects_unknown_field() {
        let bytes = br#"{
            "manifest_version": "1.0",
            "bundle_id": "x/y@1.0.0/onnx/384",
            "framework": "onnx",
            "dim": 384,
            "components": [],
            "runtime": {"max_seq_len":256,"pooling":"mean","normalize":true,"dtype":"float32"},
            "build": {"builder":"x","built_at_unix_ms":1,"git_sha":"0123456789abcdef0123456789abcdef01234567","source_date_epoch":1},
            "license": {"spdx_id":"Apache-2.0"},
            "EXTRA_TYPO_FIELD": "should_be_rejected"
        }"#;
        let err = Manifest::parse_and_validate(bytes).unwrap_err();
        assert!(matches!(err, ManifestParseError::Json(_)));
    }

    #[test]
    fn parse_bundle_id_canonical() {
        let p = parse_bundle_id("sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384").unwrap();
        assert_eq!(p.family_name, "sentence-transformers/all-MiniLM-L6-v2");
        assert_eq!(p.version, "1.0.0");
        assert_eq!(p.framework, "onnx");
        assert_eq!(p.dim, 384);
    }

    #[test]
    fn parse_bundle_id_rejects_missing_segments() {
        for s in [
            "no-at",
            "x/y@1.0.0",
            "x/y@1.0.0/onnx",
            "x@1.0.0/onnx/384",   // no family/name slash
            "x/y@1.0/onnx/384",   // bad semver
            "x/y@a.b.c/onnx/384", // bad semver
        ] {
            assert!(parse_bundle_id(s).is_err(), "{s:?} should be malformed");
        }
    }

    #[test]
    fn dim_canonical_set_includes_common_values() {
        // Pin the architecture-doc § 4.3 list against drift.
        assert!(CANONICAL_DIMS.contains(&384));
        assert!(CANONICAL_DIMS.contains(&768));
        assert!(CANONICAL_DIMS.contains(&1024));
        assert!(CANONICAL_DIMS.contains(&1536));
    }

    #[test]
    fn pooling_serdes_snake_case() {
        let p = Pooling::WeightedMean;
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"weighted_mean\"");
    }
}
