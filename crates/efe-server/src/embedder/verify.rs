//! Sprint **D16** — per-component SHA-256 verification.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 8.3
//! step 3. After the manifest signature is verified, the loader walks
//! the `components[]` list and re-hashes each file on disk. A
//! mismatch on a `required = true` component fails the load
//! (`Outcome::HashMismatch`).
//!
//! # Threat-model role
//!
//! Even with a valid manifest signature, the bundle directory could
//! be partially corrupted (PVC fsync truncation, partial NFS read,
//! tampered after mount). Per-file hash check closes T-16-3 :
//! disk-tampering detection is independent of the signature.
//!
//! # Performance
//!
//! Hashing happens at boot only. For a 500 MiB bundle this costs
//! ~0.5 s of CPU at 1 GiB/s SHA-256 throughput. Well under the
//! `EFE_BUNDLE_LOAD_TIMEOUT_S` default of 60 s.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncReadExt;

use super::manifest::{Component, Manifest};

/// Bundle-wide hash-verify outcome surfaced by [`verify_components`].
#[derive(Debug, Error)]
pub enum VerifyError {
    /// File listed in `components[]` and `required = true` is absent
    /// from disk.
    #[error("required component {0:?} missing from bundle root")]
    MissingRequiredComponent(String),

    /// File on disk has a different SHA-256 than what the manifest
    /// promised.
    #[error("hash mismatch at {path:?}: expected {expected}, got {actual}")]
    HashMismatch {
        /// Component path.
        path: String,
        /// Manifest-declared hex sha256.
        expected: String,
        /// Hex sha256 we just computed on disk.
        actual: String,
    },

    /// File on disk has a different size than the manifest claimed.
    /// Detected before the hash check (cheaper) ; primarily a
    /// fail-fast for partial reads.
    #[error("size mismatch at {path:?}: expected {expected_bytes} bytes, got {actual_bytes}")]
    SizeMismatch {
        /// Component path.
        path: String,
        /// Manifest-declared bytes.
        expected_bytes: u64,
        /// Bytes we found on disk.
        actual_bytes: u64,
    },

    /// File-IO failure walking the bundle root.
    #[error("io error reading {path:?}: {source}")]
    Io {
        /// Path that errored.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// A component path tries to escape the bundle root via `..` or
    /// absolute path. Defense in depth — should be caught by the
    /// manifest schema's path regex but we re-check at verify time.
    #[error("component path {0:?} contains traversal segments — refusing")]
    PathTraversalAttempt(String),
}

/// Outcome of verifying a single component.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ComponentVerifyOutcome {
    /// File present + size + hash all match.
    Match,
    /// File absent and `required = false` — log + skip.
    OptionalMissing,
    /// File present but hash mismatches and `required = false` —
    /// log + skip. Surfaces `efe_bundle_hash_verifications_total{result="mismatch_optional"}`.
    OptionalHashMismatch,
}

/// Verify every component in the manifest against the on-disk bundle.
///
/// Required components must :
///
///   1. Exist on disk.
///   2. Have the size declared in the manifest.
///   3. Have the SHA-256 declared in the manifest.
///
/// Optional components missing or mismatching produce a
/// `tracing::warn!` and continue.
///
/// Path-traversal hardening : any `path` containing `..`, leading
/// `/`, or absolute components is refused as
/// [`VerifyError::PathTraversalAttempt`] (defense in depth — the
/// manifest schema regex already rejects these).
pub async fn verify_components(manifest: &Manifest, bundle_root: &Path) -> Result<(), VerifyError> {
    for c in &manifest.components {
        verify_component(c, bundle_root).await?;
    }
    Ok(())
}

async fn verify_component(
    component: &Component,
    bundle_root: &Path,
) -> Result<ComponentVerifyOutcome, VerifyError> {
    if path_has_traversal(&component.path) {
        return Err(VerifyError::PathTraversalAttempt(component.path.clone()));
    }
    let full = bundle_root.join(&component.path);
    let meta = match tokio::fs::metadata(&full).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if component.required {
                return Err(VerifyError::MissingRequiredComponent(
                    component.path.clone(),
                ));
            }
            tracing::warn!(
                target: "efe.embedder.verify",
                path = %component.path,
                "optional component missing — continuing"
            );
            return Ok(ComponentVerifyOutcome::OptionalMissing);
        }
        Err(e) => {
            return Err(VerifyError::Io {
                path: full,
                source: e,
            });
        }
    };
    if meta.len() != component.size_bytes {
        return Err(VerifyError::SizeMismatch {
            path: component.path.clone(),
            expected_bytes: component.size_bytes,
            actual_bytes: meta.len(),
        });
    }
    let actual_hex = hash_file_streaming(&full).await?;
    if actual_hex == component.sha256 {
        return Ok(ComponentVerifyOutcome::Match);
    }
    if component.required {
        return Err(VerifyError::HashMismatch {
            path: component.path.clone(),
            expected: component.sha256.clone(),
            actual: actual_hex,
        });
    }
    tracing::warn!(
        target: "efe.embedder.verify",
        path = %component.path,
        expected = %component.sha256,
        actual = %actual_hex,
        "optional component hash mismatch — continuing"
    );
    Ok(ComponentVerifyOutcome::OptionalHashMismatch)
}

/// Stream a file through SHA-256 in 64 KiB chunks (memory-bounded
/// even for 5 GiB ONNX models).
async fn hash_file_streaming(path: &Path) -> Result<String, VerifyError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|e| VerifyError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).await.map_err(|e| VerifyError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Reject paths containing `..` segments, absolute roots, or
/// Windows-style drives. The manifest JSON schema already pins the
/// path to `^[a-zA-Z0-9._/-]+$` but we double-check at verify time
/// — defense in depth.
///
/// **Audit fix D16-H4** — the previous implementation only split on
/// the forward-slash separator, so a path like `..\..\etc\passwd`
/// was treated as one segment containing literal `..\` and slipped
/// through. We now check `..` after normalising both `\` and `/`
/// to the same separator before splitting.
fn path_has_traversal(p: &str) -> bool {
    if p.is_empty() {
        return true;
    }
    if p.starts_with('/') || p.starts_with('\\') {
        return true;
    }
    // Windows drive letter (C:\, etc.).
    if p.len() >= 2 && p.as_bytes()[1] == b':' {
        return true;
    }
    // Normalise backslashes to forward-slash before splitting so
    // Windows-style traversal (`..\..\etc\passwd`) is detected on
    // any platform.
    let normalised = p.replace('\\', "/");
    for seg in normalised.split('/') {
        if seg == ".." || seg == "." || seg.is_empty() {
            return true;
        }
    }
    false
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::manifest::{
        BuildInfo, Dtype, Framework, LicenseInfo, Manifest, Pooling, RuntimeConfig,
    };

    fn build_test_bundle_dir(contents: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, body) in contents {
            let full = dir.path().join(rel);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, body).unwrap();
        }
        dir
    }

    fn manifest_for_components(components: Vec<Component>) -> Manifest {
        Manifest {
            manifest_version: "1.0".into(),
            bundle_id: "x/y@1.0.0/onnx/384".into(),
            framework: Framework::Onnx,
            dim: 384,
            components,
            runtime: RuntimeConfig {
                max_seq_len: 256,
                pooling: Pooling::Mean,
                normalize: true,
                dtype: Dtype::Float32,
            },
            build: BuildInfo {
                builder: "x".into(),
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
        }
    }

    fn sha256_hex(b: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(b);
        hex::encode(h.finalize())
    }

    #[tokio::test]
    async fn verify_happy_path() {
        let body_a = b"hello-a";
        let body_b = b"hello-b";
        let dir = build_test_bundle_dir(&[
            ("weights/model.onnx", body_a),
            ("tokenizer/tokenizer.json", body_b),
        ]);
        let manifest = manifest_for_components(vec![
            Component {
                path: "weights/model.onnx".into(),
                sha256: sha256_hex(body_a),
                size_bytes: body_a.len() as u64,
                required: true,
            },
            Component {
                path: "tokenizer/tokenizer.json".into(),
                sha256: sha256_hex(body_b),
                size_bytes: body_b.len() as u64,
                required: true,
            },
        ]);
        verify_components(&manifest, dir.path()).await.unwrap();
    }

    #[tokio::test]
    async fn verify_required_missing_fails() {
        let dir = build_test_bundle_dir(&[]);
        let manifest = manifest_for_components(vec![Component {
            path: "weights/model.onnx".into(),
            sha256: "0".repeat(64),
            size_bytes: 0,
            required: true,
        }]);
        let err = verify_components(&manifest, dir.path()).await.unwrap_err();
        assert!(matches!(err, VerifyError::MissingRequiredComponent(_)));
    }

    #[tokio::test]
    async fn verify_size_mismatch_fails() {
        let body = b"hello";
        let dir = build_test_bundle_dir(&[("weights/model.onnx", body)]);
        let manifest = manifest_for_components(vec![Component {
            path: "weights/model.onnx".into(),
            sha256: sha256_hex(body),
            size_bytes: 9999, // wrong
            required: true,
        }]);
        let err = verify_components(&manifest, dir.path()).await.unwrap_err();
        assert!(matches!(err, VerifyError::SizeMismatch { .. }));
    }

    #[tokio::test]
    async fn verify_hash_mismatch_required_fails() {
        let body = b"hello";
        let dir = build_test_bundle_dir(&[("weights/model.onnx", body)]);
        let manifest = manifest_for_components(vec![Component {
            path: "weights/model.onnx".into(),
            sha256: "0".repeat(64), // wrong hash
            size_bytes: body.len() as u64,
            required: true,
        }]);
        let err = verify_components(&manifest, dir.path()).await.unwrap_err();
        assert!(matches!(err, VerifyError::HashMismatch { .. }));
    }

    #[tokio::test]
    async fn verify_optional_missing_logs_and_continues() {
        let dir = build_test_bundle_dir(&[]);
        let manifest = manifest_for_components(vec![Component {
            path: "SBOM.json".into(),
            sha256: "0".repeat(64),
            size_bytes: 0,
            required: false,
        }]);
        verify_components(&manifest, dir.path()).await.unwrap();
    }

    #[tokio::test]
    async fn verify_rejects_path_traversal() {
        let dir = build_test_bundle_dir(&[]);
        for bad in [
            "../etc/passwd",
            "weights/../../etc/passwd",
            "/absolute/path",
            "C:\\windows",
            "",
        ] {
            let manifest = manifest_for_components(vec![Component {
                path: bad.into(),
                sha256: "0".repeat(64),
                size_bytes: 0,
                required: true,
            }]);
            let err = verify_components(&manifest, dir.path()).await.unwrap_err();
            assert!(
                matches!(err, VerifyError::PathTraversalAttempt(_)),
                "path {bad:?} should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn hash_file_streaming_handles_large_files() {
        // 1 MiB of zeros is enough to exercise the chunked read.
        let body = vec![0u8; 1024 * 1024];
        let dir = build_test_bundle_dir(&[("weights/model.onnx", &body)]);
        let path = dir.path().join("weights/model.onnx");
        let h = hash_file_streaming(&path).await.unwrap();
        // Pre-computed sha256 of 1 MiB of zero bytes :
        assert_eq!(
            h,
            "30e14955ebf1352266dc2ff8067e68104607e750abb9d3b36582b8af909fcb58"
        );
    }
}
