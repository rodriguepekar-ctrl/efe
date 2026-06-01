//! Sprint **D16** — anti-corruption layer for D8 catalog fingerprints.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 9.4
//! + Appendix C (§ 21).
//!
//! D8/D12 catalogs persisted `embedder_fingerprint` at the
//! `family/model@dim` granularity (e.g.
//! `sentence-transformers/all-MiniLM-L6-v2@384`). D16 introduces
//! byte-exact fingerprints (SHA-256 of the manifest), but we MUST
//! not invalidate every existing catalog at the v0.1.16 release.
//!
//! This module bridges the two worlds :
//!
//!   * [`extract_legacy_fingerprint`] — given a loaded D16 [`Bundle`],
//!     compute the matching `family/model@dim` string a D8 catalog
//!     would have stored.
//!
//!   * [`compatibility_match`] — three-state result :
//!     ByteExactMatch / LegacyMatch / Mismatch.
//!
//! The catalog handler uses [`compatibility_match`] at Register /
//! Evolve / Get RPC time. A `LegacyMatch` succeeds with a
//! `tracing::warn!` recommending re-fitting ; a `Mismatch` is a
//! hard reject.

use super::bundle::Bundle;
use super::manifest::parse_bundle_id;

/// Three-state outcome of comparing a catalog's stored fingerprint
/// against the active bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompatMatch {
    /// Catalog's `bundle_fingerprint` matches the active bundle's
    /// SHA-256 manifest fingerprint byte-for-byte. Hard pin.
    ByteExactMatch,
    /// Catalog has only the legacy `family/model@dim` form and it
    /// agrees with the bundle's id. Accept with operator warning.
    LegacyMatch,
    /// Catalog claims a different bundle / different family.
    /// Reject — operator must rotate the bundle or re-fit the
    /// catalog.
    Mismatch {
        /// What the catalog stored.
        stored: String,
        /// What the active bundle resolves to.
        bundle_says: String,
    },
}

/// Extract the legacy `family/model@dim` representation from a
/// D16 bundle. Used to soft-match catalogs fitted pre-D16.
///
/// Example :
///
/// ```text
/// "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384"
///   → "sentence-transformers/all-MiniLM-L6-v2@384"
/// ```
///
/// Returns `None` if the `bundle_id` cannot be parsed (the loader
/// would have already rejected such a bundle, but defensive).
#[must_use]
pub fn extract_legacy_fingerprint(bundle: &Bundle) -> Option<String> {
    let id = &bundle.manifest().bundle_id;
    let parsed = parse_bundle_id(id).ok()?;
    Some(format!("{}@{}", parsed.family_name, parsed.dim))
}

/// Compare a catalog's two fingerprint columns against the active
/// bundle.
///
/// At least one of `bundle_fp` / `legacy_fp` should be Some — the
/// SQL CHECK constraint enforces XOR + a both-NULL fallback for
/// pre-D12 rows (cf. migration 0005, audit fix C5).
///
/// **Audit fix D16-M1** — when this returns [`CompatMatch::LegacyMatch`],
/// callers should call [`warn_on_legacy_match`] so ops see a
/// "byte-exact pinning recommended" line in the logs and can
/// schedule a re-fit (`efe-onboard upgrade-catalog`). The warning
/// is opt-in (a separate function call) because some callers like
/// the CLI verify path don't need the operator-friendly hint.
pub fn compatibility_match(
    bundle: &Bundle,
    catalog_bundle_fp: Option<&str>,
    catalog_legacy_fp: Option<&str>,
) -> CompatMatch {
    let active_byte_exact = bundle.fingerprint().hex();
    if let Some(stored) = catalog_bundle_fp {
        if stored == active_byte_exact {
            return CompatMatch::ByteExactMatch;
        }
        return CompatMatch::Mismatch {
            stored: stored.to_string(),
            bundle_says: active_byte_exact,
        };
    }
    if let Some(stored) = catalog_legacy_fp {
        let computed = extract_legacy_fingerprint(bundle).unwrap_or_default();
        if stored == computed {
            return CompatMatch::LegacyMatch;
        }
        return CompatMatch::Mismatch {
            stored: stored.to_string(),
            bundle_says: computed,
        };
    }
    CompatMatch::Mismatch {
        stored: "<none>".to_string(),
        bundle_says: active_byte_exact,
    }
}

/// **Audit fix D16-M1** — emit a `tracing::warn!` line when a
/// catalog matches via the legacy `family/model@dim` path. The
/// architecture (§ 9.4) requires this so ops have a paper trail of
/// which catalogs need re-fitting onto byte-exact bundle
/// fingerprints. Suggested operator action :
/// `efe-onboard upgrade-catalog --tenant <t> --catalog-name <n>`.
pub fn warn_on_legacy_match(
    bundle: &Bundle,
    catalog_legacy_fp: &str,
    tenant: &str,
    catalog_name: &str,
) {
    tracing::warn!(
        target: "efe.embedder.legacy",
        bundle_id = %bundle.manifest().bundle_id,
        bundle_fingerprint = %bundle.fingerprint(),
        catalog_legacy_fingerprint = catalog_legacy_fp,
        tenant = tenant,
        catalog = catalog_name,
        "catalog uses legacy embedder fingerprint — byte-exact pinning \
         recommended (run `efe-onboard upgrade-catalog --tenant <t> --catalog-name <n>`)"
    );
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SignaturePolicy;
    use crate::embedder::bundle::{load_bundle_dir, LoadPolicy};
    use crate::embedder::manifest::{
        BuildInfo, Component, Dtype, Framework, LicenseInfo, Manifest, Pooling, RuntimeConfig,
    };
    use crate::embedder::trust::TrustStore;
    use sha2::{Digest, Sha256};

    fn sha256_hex(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    async fn build_bundle_with_id(bundle_id: &str, dim: usize) -> Bundle {
        let dir = tempfile::tempdir().unwrap();
        let weights = b"w";
        let tokenizer = b"t";
        std::fs::create_dir_all(dir.path().join("weights")).unwrap();
        std::fs::create_dir_all(dir.path().join("tokenizer")).unwrap();
        std::fs::write(dir.path().join("weights/model.onnx"), weights).unwrap();
        std::fs::write(dir.path().join("tokenizer/tokenizer.json"), tokenizer).unwrap();
        let manifest = Manifest {
            manifest_version: "1.0".into(),
            bundle_id: bundle_id.to_string(),
            framework: Framework::Onnx,
            dim,
            components: vec![
                Component {
                    path: "weights/model.onnx".into(),
                    sha256: sha256_hex(weights),
                    size_bytes: weights.len() as u64,
                    required: true,
                },
                Component {
                    path: "tokenizer/tokenizer.json".into(),
                    sha256: sha256_hex(tokenizer),
                    size_bytes: tokenizer.len() as u64,
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
                builder: "test".into(),
                built_at_unix_ms: 0,
                git_sha: "0".repeat(40),
                source_date_epoch: 0,
            },
            license: LicenseInfo {
                spdx_id: "Apache-2.0".into(),
                url: None,
                redistribution_allowed: true,
            },
            model_origin: None,
            sbom_path: None,
        };
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let trust = TrustStore { keys: vec![] };
        let policy = LoadPolicy {
            signature: SignaturePolicy::Skip,
            require_sbom: false,
            ack_non_redistributable: false,
        };
        let b = load_bundle_dir(dir.path(), &policy, &trust).await.unwrap();
        // Keep the tempdir alive by leaking — the test process exits
        // soon after this anyway.
        std::mem::forget(dir);
        b
    }

    #[tokio::test]
    async fn extract_legacy_strips_version_and_framework() {
        let b = build_bundle_with_id("sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384", 384)
            .await;
        let leg = extract_legacy_fingerprint(&b).unwrap();
        assert_eq!(leg, "sentence-transformers/all-MiniLM-L6-v2@384");
    }

    #[tokio::test]
    async fn byte_exact_match_when_fp_equal() {
        let b = build_bundle_with_id("x/y@1.0.0/onnx/64", 64).await;
        let stored = b.fingerprint().hex();
        let m = compatibility_match(&b, Some(&stored), None);
        assert_eq!(m, CompatMatch::ByteExactMatch);
    }

    #[tokio::test]
    async fn byte_exact_mismatch_yields_mismatch() {
        let b = build_bundle_with_id("x/y@1.0.0/onnx/64", 64).await;
        let m = compatibility_match(&b, Some(&"a".repeat(64)), None);
        assert!(matches!(m, CompatMatch::Mismatch { .. }));
    }

    #[tokio::test]
    async fn legacy_match_when_id_strips_match() {
        let b = build_bundle_with_id("sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384", 384)
            .await;
        let m = compatibility_match(&b, None, Some("sentence-transformers/all-MiniLM-L6-v2@384"));
        assert_eq!(m, CompatMatch::LegacyMatch);
    }

    #[tokio::test]
    async fn legacy_mismatch_yields_mismatch() {
        let b = build_bundle_with_id("sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384", 384)
            .await;
        let m = compatibility_match(
            &b,
            None,
            Some("BAAI/bge-large-en@1024"), // different family
        );
        assert!(matches!(m, CompatMatch::Mismatch { .. }));
    }

    #[tokio::test]
    async fn no_fingerprint_at_all_is_mismatch() {
        let b = build_bundle_with_id("x/y@1.0.0/onnx/64", 64).await;
        let m = compatibility_match(&b, None, None);
        assert!(matches!(m, CompatMatch::Mismatch { .. }));
    }

    #[tokio::test]
    async fn byte_exact_takes_priority_over_legacy() {
        // SQL CHECK enforces XOR but defensive : if BOTH are
        // somehow set, byte_exact is consulted first.
        let b = build_bundle_with_id("sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384", 384)
            .await;
        let stored = b.fingerprint().hex();
        let m = compatibility_match(
            &b,
            Some(&stored),
            Some("BAAI/bge-large-en@1024"), // ignored
        );
        assert_eq!(m, CompatMatch::ByteExactMatch);
    }
}
