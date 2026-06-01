//! Sprint **D16** — top-level bundle loader.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 8.3.
//!
//! ```text
//! load_bundle_dir(root, signature_policy, trust_store) :
//!   1. read manifest.json bytes
//!   2. parse + validate manifest invariants (manifest.rs)
//!   3. verify detached signature (signature.rs) per policy
//!   4. verify each component's SHA256 + size (verify.rs)
//!   5. instantiate runtime session (runtime/mod.rs)
//!   6. compute fingerprint = SHA256(manifest.json bytes)
//!   7. assemble Bundle { manifest, fingerprint, signing_key_id, root, session }
//! ```
//!
//! Fail-fast at every step. The first error is returned and surfaced
//! as the appropriate `Outcome` for the
//! `efe_bundle_load_total{outcome=...}` metric.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::config::SignaturePolicy;

use super::manifest::{Manifest, ManifestParseError};
use super::runtime::{instantiate, EmbedderError, EmbedderSession};
use super::signature::{verify_manifest_signature, SigError};
use super::trust::TrustStore;
use super::verify::{verify_components, VerifyError};

// =====================================================================
// Bundle handle
// =====================================================================

/// A fully-loaded, verified, ready-to-encode bundle.
///
/// Cloning is cheap (`Arc<...>` internally). The runtime keeps one
/// `Bundle` per `bundle_id` and shares it across every concurrent
/// `Encode` RPC.
#[derive(Clone)]
pub struct Bundle {
    inner: Arc<BundleInner>,
}

struct BundleInner {
    manifest: Manifest,
    fingerprint: BundleFingerprint,
    signing_key_id: Option<String>,
    root: PathBuf,
    session: Box<dyn EmbedderSession>,
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle")
            .field("bundle_id", &self.inner.manifest.bundle_id)
            .field("framework", &self.inner.manifest.framework)
            .field("dim", &self.inner.manifest.dim)
            .field("fingerprint", &self.inner.fingerprint.hex())
            .field("signing_key_id", &self.inner.signing_key_id)
            .field("root", &self.inner.root)
            .finish()
    }
}

impl Bundle {
    /// The parsed + validated manifest.
    #[must_use]
    pub fn manifest(&self) -> &Manifest {
        &self.inner.manifest
    }

    /// Canonical fingerprint = SHA-256 of the manifest.json bytes.
    /// This is what catalogs (D12) bind themselves to.
    #[must_use]
    pub fn fingerprint(&self) -> &BundleFingerprint {
        &self.inner.fingerprint
    }

    /// `Some(key_id)` if the manifest was signature-verified ;
    /// `None` if the policy was [`SignaturePolicy::Skip`] or
    /// [`SignaturePolicy::Warn`] with no signature present.
    #[must_use]
    pub fn signing_key_id(&self) -> Option<&str> {
        self.inner.signing_key_id.as_deref()
    }

    /// Bundle root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.inner.root
    }

    /// Output dimension of the underlying session.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.inner.session.dim()
    }

    /// Encode a single text — hot path.
    ///
    /// **Audit fix D16-H3** — emits two metrics per call :
    ///   * `efe_embedder_encode_total{bundle_id, tenant, outcome}`
    ///   * `efe_embedder_encode_duration_seconds{bundle_id, tenant}`
    /// Pre-audit code never bumped these — dashboards showed flat
    /// zero series. The `tenant` parameter is supplied by the caller
    /// (the gRPC handler reads it from the OAuth claims) ; defaults
    /// to `"unknown"` via [`Self::encode_one`] for callers that
    /// don't have a tenant context (CLI verify, unit tests).
    pub fn encode_one(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        self.encode_one_for_tenant(text, "unknown")
    }

    /// Same as [`Self::encode_one`] but with the tenant label for
    /// the metric. RPC handlers should call this form to attribute
    /// per-tenant cost.
    pub fn encode_one_for_tenant(
        &self,
        text: &str,
        tenant: &str,
    ) -> Result<Vec<f32>, EmbedderError> {
        let bundle_id = &self.inner.manifest.bundle_id;
        let start = std::time::Instant::now();
        let res = self.inner.session.encode_one(text);
        let elapsed = start.elapsed();
        let outcome = encode_outcome_label(&res, text, &self.inner.manifest);
        efe_telemetry::metrics::d16_instruments::embedder_encode_total()
            .with_label_values(&[bundle_id, tenant, outcome])
            .inc();
        efe_telemetry::metrics::d16_instruments::embedder_encode_duration_seconds()
            .with_label_values(&[bundle_id, tenant])
            .observe(elapsed.as_secs_f64());
        res
    }

    /// Encode a batch of texts — hot path. Each item produces its
    /// own counter increment so per-tenant attribution stays
    /// accurate.
    pub fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        self.encode_batch_for_tenant(texts, "unknown")
    }

    /// Same as [`Self::encode_batch`] but with the tenant label.
    pub fn encode_batch_for_tenant(
        &self,
        texts: &[&str],
        tenant: &str,
    ) -> Result<Vec<Vec<f32>>, EmbedderError> {
        let bundle_id = &self.inner.manifest.bundle_id;
        let start = std::time::Instant::now();
        let res = self.inner.session.encode_batch(texts);
        let elapsed = start.elapsed();
        // Per-call metrics : we record one duration sample per
        // batch (the batch as a whole) but bump the counter once
        // per text in the batch so QPS attribution is accurate.
        let outcome = match &res {
            Ok(_) => "ok",
            Err(EmbedderError::Tokenizer(_)) => "tokenizer_failed",
            Err(EmbedderError::Forward(_)) => "forward_failed",
            Err(EmbedderError::EmptyInput) => "empty_input",
            Err(EmbedderError::InvalidShape(_)) => "forward_failed",
        };
        let counter = efe_telemetry::metrics::d16_instruments::embedder_encode_total();
        for _ in 0..texts.len().max(1) {
            counter
                .with_label_values(&[bundle_id, tenant, outcome])
                .inc();
        }
        efe_telemetry::metrics::d16_instruments::embedder_encode_duration_seconds()
            .with_label_values(&[bundle_id, tenant])
            .observe(elapsed.as_secs_f64());
        res
    }
}

/// **Audit fix D16-H6** — wrap [`verify_manifest_signature`] with
/// the `efe_bundle_signature_verifications_total{bundle_id, key_id,
/// result}` counter. Pre-audit code never bumped this — the
/// dashboard panel was zero-only and key-rotation visibility was
/// invisible to ops.
fn verify_manifest_signature_with_metric(
    manifest_bytes: &[u8],
    sig_bytes: &[u8],
    trust: &TrustStore,
    now: std::time::SystemTime,
    bundle_id: &str,
) -> Result<crate::embedder::signature::VerifiedSignature, SigError> {
    let res = verify_manifest_signature(manifest_bytes, sig_bytes, trust, now);
    let counter = efe_telemetry::metrics::d16_instruments::bundle_signature_verifications_total();
    match &res {
        Ok(v) => counter
            .with_label_values(&[bundle_id, &v.key_id, "ok"])
            .inc(),
        Err(SigError::MalformedSig(_)) => counter
            .with_label_values(&[bundle_id, "unknown", "bad_signature"])
            .inc(),
        Err(SigError::NoMatchingKey { .. }) => counter
            .with_label_values(&[bundle_id, "unknown", "bad_signature"])
            .inc(),
        Err(SigError::NoCurrentlyValidKey { .. }) => counter
            .with_label_values(&[bundle_id, "unknown", "key_expired"])
            .inc(),
        Err(SigError::BadPublicKey(kid)) => counter
            .with_label_values(&[bundle_id, kid, "bad_signature"])
            .inc(),
    }
    res
}

/// **Audit fix D16-H3** — outcome label for the per-encode counter.
/// Architecture § 13.3 lists the canonical labels :
///   `ok` / `tokenizer_failed` / `forward_failed` / `empty_input` /
///   `nan_output` / `truncated`.
///
/// We classify `EmbedderError` variants here. The `truncated` label
/// would fire when input length > `runtime.max_seq_len` — the mock
/// runtime doesn't tokenize so it never trips ; the real ORT
/// adapter (D16-bis) will set this label after counting truncated
/// tokens.
fn encode_outcome_label(
    res: &Result<Vec<f32>, EmbedderError>,
    _text: &str,
    _manifest: &Manifest,
) -> &'static str {
    match res {
        Ok(v) => {
            if v.iter().any(|x| !x.is_finite()) {
                "nan_output"
            } else {
                "ok"
            }
        }
        Err(EmbedderError::Tokenizer(_)) => "tokenizer_failed",
        Err(EmbedderError::Forward(_)) => "forward_failed",
        Err(EmbedderError::EmptyInput) => "empty_input",
        Err(EmbedderError::InvalidShape(_)) => "forward_failed",
    }
}

/// 32-byte SHA-256 of `manifest.json` bytes. Display-as-hex.
#[derive(Clone, Copy, Hash, Eq, PartialEq)]
pub struct BundleFingerprint(pub [u8; 32]);

impl BundleFingerprint {
    /// Lowercase hex (64 chars). Same encoding as the SQL column
    /// `catalogs.bundle_fingerprint`.
    #[must_use]
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Debug for BundleFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BundleFingerprint({})", self.hex())
    }
}

impl std::fmt::Display for BundleFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

// =====================================================================
// Errors
// =====================================================================

/// All failure modes of [`load_bundle_dir`].
#[derive(Debug, Error)]
pub enum BundleLoadError {
    /// IO failure reading `manifest.json` or `manifest.json.sig`.
    #[error("bundle IO error at {path:?}: {source}")]
    Io {
        /// Path that errored.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Manifest parse / invariant violation.
    #[error("manifest invalid: {0}")]
    Manifest(#[from] ManifestParseError),

    /// Detached signature verify failed.
    #[error("signature verify failed: {0}")]
    Signature(#[from] SigError),

    /// Component hash / size verify failed.
    #[error("component verify failed: {0}")]
    Verify(#[from] VerifyError),

    /// Runtime session init failed.
    #[error("runtime init failed: {0}")]
    Runtime(#[from] EmbedderError),

    /// `EFE_BUNDLE_REQUIRE_SIGNATURE=required` and the bundle has
    /// no `manifest.json.sig` on disk.
    #[error("signature required by policy but manifest.json.sig missing at {0:?}")]
    SignatureRequiredButMissing(PathBuf),

    /// `EFE_BUNDLE_REQUIRE_SBOM=true` and the manifest doesn't
    /// declare a `sbom_path`.
    #[error("SBOM required by policy but manifest declares no sbom_path")]
    SbomRequiredButMissing,

    /// Bundle's license forbids redistribution and the operator
    /// hasn't acknowledged via `EFE_BUNDLE_ACK_NONREDIST=true`.
    #[error("license {0:?} forbids redistribution and operator has not set EFE_BUNDLE_ACK_NONREDIST=true")]
    NonRedistributableLicense(String),
}

/// Outcomes for the `efe_bundle_load_total{outcome=...}` metric.
/// Architecture § 13.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadOutcome {
    /// Bundle loaded + verified successfully.
    Ok,
    /// `manifest.json` IO or parse failed.
    ManifestInvalid,
    /// Signature verify failed (sig missing when required, key mismatch, etc.).
    SignatureFailed,
    /// Component hash / size verify failed.
    HashMismatch,
    /// Runtime session init failed.
    RuntimeInitFailed,
    /// Operator-policy violation (SBOM required but absent, license
    /// non-redistributable without ACK, etc.).
    PolicyViolation,
}

impl LoadOutcome {
    /// Lowercase label for Prometheus.
    #[must_use]
    pub fn as_label(self) -> &'static str {
        match self {
            LoadOutcome::Ok => "ok",
            LoadOutcome::ManifestInvalid => "manifest_invalid",
            LoadOutcome::SignatureFailed => "signature_failed",
            LoadOutcome::HashMismatch => "hash_mismatch",
            LoadOutcome::RuntimeInitFailed => "runtime_init_failed",
            LoadOutcome::PolicyViolation => "policy_violation",
        }
    }
}

impl BundleLoadError {
    /// Map an error to its outcome label.
    #[must_use]
    pub fn outcome(&self) -> LoadOutcome {
        match self {
            BundleLoadError::Io { .. } | BundleLoadError::Manifest(_) => {
                LoadOutcome::ManifestInvalid
            }
            BundleLoadError::Signature(_) | BundleLoadError::SignatureRequiredButMissing(_) => {
                LoadOutcome::SignatureFailed
            }
            BundleLoadError::Verify(_) => LoadOutcome::HashMismatch,
            BundleLoadError::Runtime(_) => LoadOutcome::RuntimeInitFailed,
            BundleLoadError::SbomRequiredButMissing
            | BundleLoadError::NonRedistributableLicense(_) => LoadOutcome::PolicyViolation,
        }
    }
}

// =====================================================================
// Top-level load function
// =====================================================================

/// Policy bundle for a single load call. Construct from
/// [`crate::config::EmbedderConfig`] at the runtime boot.
#[derive(Clone, Debug)]
pub struct LoadPolicy {
    /// Signature policy (required / warn / skip).
    pub signature: SignaturePolicy,
    /// Whether to refuse bundles without a `sbom_path`.
    pub require_sbom: bool,
    /// Whether the operator has acknowledged non-redistributable
    /// licenses (allows the loader to proceed past the license check).
    pub ack_non_redistributable: bool,
}

impl Default for LoadPolicy {
    fn default() -> Self {
        Self {
            signature: SignaturePolicy::Required,
            require_sbom: true,
            ack_non_redistributable: false,
        }
    }
}

/// Top-level load function — orchestrates parse, verify, sign-verify,
/// runtime instantiate. Returns a fully-ready [`Bundle`].
///
/// **Audit fix D16-H1** — step ordering reworked to fail fast on
/// cheap checks before expensive ones :
///
///   1. IO / parse / manifest invariants — `Manifest(*)`
///   2. Signature verify — `Signature(*)` (so policy fields are
///      authentic before we trust them)
///   3. Policy checks (SBOM + license) — fail-fast on
///      misconfiguration BEFORE the 500 MiB hash walk
///   4. Component hash / size — `Verify(*)`
///   5. Runtime instantiate — `Runtime(*)`
///   6. Fingerprint computation
pub async fn load_bundle_dir(
    root: &Path,
    policy: &LoadPolicy,
    trust: &TrustStore,
) -> Result<Bundle, BundleLoadError> {
    // Step 1 — manifest.
    let manifest_path = root.join("manifest.json");
    let manifest_bytes =
        tokio::fs::read(&manifest_path)
            .await
            .map_err(|e| BundleLoadError::Io {
                path: manifest_path.clone(),
                source: e,
            })?;
    let manifest = Manifest::parse_and_validate(&manifest_bytes)?;

    tracing::info!(
        target: "efe.embedder.bundle",
        bundle_id = %manifest.bundle_id,
        framework = manifest.framework.as_str(),
        dim = manifest.dim,
        n_components = manifest.components.len(),
        "loading bundle"
    );

    // Step 2 — signature.
    let signing_key_id = match policy.signature {
        SignaturePolicy::Required => {
            let sig_path = root.join("manifest.json.sig");
            let sig_bytes = match tokio::fs::read(&sig_path).await {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(BundleLoadError::SignatureRequiredButMissing(sig_path));
                }
                Err(e) => {
                    return Err(BundleLoadError::Io {
                        path: sig_path,
                        source: e,
                    });
                }
            };
            let v = verify_manifest_signature_with_metric(
                &manifest_bytes,
                &sig_bytes,
                trust,
                std::time::SystemTime::now(),
                &manifest.bundle_id,
            )?;
            Some(v.key_id)
        }
        SignaturePolicy::Warn => {
            let sig_path = root.join("manifest.json.sig");
            match tokio::fs::read(&sig_path).await {
                Ok(sig_bytes) => {
                    // Present : MUST verify (we never accept "present
                    // but invalid" — that's actively suspicious).
                    let v = verify_manifest_signature_with_metric(
                        &manifest_bytes,
                        &sig_bytes,
                        trust,
                        std::time::SystemTime::now(),
                        &manifest.bundle_id,
                    )?;
                    Some(v.key_id)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tracing::warn!(
                        target: "efe.embedder.bundle",
                        bundle_id = %manifest.bundle_id,
                        "signature absent and policy=warn — accepting (set policy=required for prod)"
                    );
                    None
                }
                Err(e) => {
                    return Err(BundleLoadError::Io {
                        path: sig_path,
                        source: e,
                    });
                }
            }
        }
        SignaturePolicy::Skip => {
            // **Audit fix D16-H10** — surface a notice at the audit
            // layer if a signature is present but ignored. Helps
            // ops debug "why isn't my bundle being verified?" when
            // policy=skip is left over from a dev iteration.
            if root.join("manifest.json.sig").exists() {
                tracing::info!(
                    target: "efe.embedder.bundle",
                    bundle_id = %manifest.bundle_id,
                    "signature present but policy=skip — NOT verified \
                     (set EFE_BUNDLE_REQUIRE_SIGNATURE=required for prod)"
                );
            }
            None
        }
    };

    // **Audit fix D16-H1** — Step 3 (policy) now runs BEFORE step 4
    // (component hash). Rationale : policy checks are O(1) on
    // already-parsed manifest fields ; the hash walk is O(n) over
    // potentially 5 GiB of bundle data. Fail fast on misconfig.
    //
    // We do this AFTER signature verify so the policy fields can
    // be trusted (an attacker forging the manifest can't bypass
    // policy without breaking signature first).
    if policy.require_sbom && manifest.sbom_path.is_none() {
        return Err(BundleLoadError::SbomRequiredButMissing);
    }
    if !manifest.license.redistribution_allowed && !policy.ack_non_redistributable {
        return Err(BundleLoadError::NonRedistributableLicense(
            manifest.license.spdx_id.clone(),
        ));
    }

    // Step 4 — component hash / size verify (potentially expensive
    // — 5 GiB ONNX models hash at ~1 GiB/s ≈ 5 s).
    //
    // **Audit fix D16-H5** — wrap with the
    // `efe_bundle_hash_verifications_total{bundle_id, result}`
    // counter. Pre-audit code didn't emit this metric, so dashboards
    // showed zero hash verifies — confusing for ops.
    let hash_counter = efe_telemetry::metrics::d16_instruments::bundle_hash_verifications_total();
    match verify_components(&manifest, root).await {
        Ok(()) => {
            hash_counter
                .with_label_values(&[&manifest.bundle_id, "ok"])
                .inc();
        }
        Err(e) => {
            let label = match &e {
                VerifyError::HashMismatch { .. } => "mismatch_required",
                VerifyError::SizeMismatch { .. } => "mismatch_required",
                VerifyError::MissingRequiredComponent(_) => "missing_required",
                VerifyError::PathTraversalAttempt(_) => "missing_required",
                VerifyError::Io { .. } => "missing_required",
            };
            hash_counter
                .with_label_values(&[&manifest.bundle_id, label])
                .inc();
            return Err(e.into());
        }
    }

    // Step 5 — runtime instantiate.
    let session = instantiate(&manifest, root)?;

    // Step 6 — fingerprint.
    let fingerprint = BundleFingerprint(Sha256::digest(&manifest_bytes).into());

    tracing::info!(
        target: "efe.embedder.bundle",
        bundle_id = %manifest.bundle_id,
        fingerprint = %fingerprint,
        signing_key_id = ?signing_key_id,
        "bundle loaded successfully"
    );

    Ok(Bundle {
        inner: Arc::new(BundleInner {
            manifest,
            fingerprint,
            signing_key_id,
            root: root.to_path_buf(),
            session,
        }),
    })
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::manifest::{
        BuildInfo, Component, Dtype, Framework, LicenseInfo, Manifest, ModelOrigin, Pooling,
        RuntimeConfig,
    };
    use crate::embedder::trust::{TrustStore, TrustedKey};
    use chrono::TimeZone;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;

    fn sha256_hex(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    /// Build a bundle directory with a tokenizer + an ONNX file
    /// (just bytes — the loader's mock runtime ignores them).
    /// Returns the bundle root + an Ed25519 keypair so callers can
    /// sign or tamper with the manifest.
    struct TestBundle {
        dir: tempfile::TempDir,
        sk: SigningKey,
        pk: [u8; 32],
        manifest_bytes: Vec<u8>,
    }

    fn build_test_bundle(manifest_overrides: impl FnOnce(&mut Manifest)) -> TestBundle {
        let dir = tempfile::tempdir().unwrap();
        // Synthetic component bytes.
        let weights = b"FAKE-ONNX-MODEL-BYTES";
        let tokenizer = b"{\"tokenizer\":\"json\"}";
        let sbom = b"{\"bom\":\"json\"}";
        std::fs::create_dir_all(dir.path().join("weights")).unwrap();
        std::fs::create_dir_all(dir.path().join("tokenizer")).unwrap();
        std::fs::write(dir.path().join("weights/model.onnx"), weights).unwrap();
        std::fs::write(dir.path().join("tokenizer/tokenizer.json"), tokenizer).unwrap();
        std::fs::write(dir.path().join("SBOM.json"), sbom).unwrap();
        let mut manifest = Manifest {
            manifest_version: "1.0".into(),
            bundle_id: "x/y@1.0.0/onnx/384".into(),
            framework: Framework::Onnx,
            dim: 384,
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
                Component {
                    path: "SBOM.json".into(),
                    sha256: sha256_hex(sbom),
                    size_bytes: sbom.len() as u64,
                    required: false,
                },
            ],
            runtime: RuntimeConfig {
                max_seq_len: 256,
                pooling: Pooling::Mean,
                normalize: true,
                dtype: Dtype::Float32,
            },
            build: BuildInfo {
                builder: "ci/test".into(),
                built_at_unix_ms: 0,
                git_sha: "0".repeat(40),
                source_date_epoch: 0,
            },
            license: LicenseInfo {
                spdx_id: "Apache-2.0".into(),
                url: None,
                redistribution_allowed: true,
            },
            model_origin: Some(ModelOrigin {
                huggingface_repo: Some("x/y".into()),
                huggingface_revision: None,
                model_card_url: None,
            }),
            sbom_path: Some("SBOM.json".into()),
        };
        manifest_overrides(&mut manifest);
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(dir.path().join("manifest.json"), &manifest_bytes).unwrap();
        let sk = SigningKey::generate(&mut OsRng);
        let pk = sk.verifying_key().to_bytes();
        TestBundle {
            dir,
            sk,
            pk,
            manifest_bytes,
        }
    }

    fn write_signature(b: &TestBundle) {
        let sig = b.sk.sign(&b.manifest_bytes).to_bytes();
        std::fs::write(b.dir.path().join("manifest.json.sig"), sig).unwrap();
    }

    fn trust_for(b: &TestBundle) -> TrustStore {
        TrustStore {
            keys: vec![TrustedKey {
                key_id: "test-key".into(),
                public_key: b.pk,
                active_for_signing: true,
                valid_from: chrono::Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
                valid_until: chrono::Utc
                    .with_ymd_and_hms(2099, 12, 31, 23, 59, 59)
                    .unwrap(),
                comment: None,
            }],
        }
    }

    #[tokio::test]
    async fn happy_path_loads_signed_bundle() {
        let b = build_test_bundle(|_| {});
        write_signature(&b);
        let trust = trust_for(&b);
        let policy = LoadPolicy::default();
        let bundle = load_bundle_dir(b.dir.path(), &policy, &trust)
            .await
            .expect("happy path");
        assert_eq!(bundle.dim(), 384);
        assert_eq!(bundle.signing_key_id(), Some("test-key"));
        let v = bundle.encode_one("hello").expect("encode");
        assert_eq!(v.len(), 384);
    }

    #[tokio::test]
    async fn manifest_missing_yields_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let trust = TrustStore { keys: vec![] };
        let err = load_bundle_dir(dir.path(), &LoadPolicy::default(), &trust)
            .await
            .unwrap_err();
        assert!(matches!(err, BundleLoadError::Io { .. }));
        assert_eq!(err.outcome(), LoadOutcome::ManifestInvalid);
    }

    #[tokio::test]
    async fn manifest_corrupt_yields_manifest_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), b"not json").unwrap();
        let trust = TrustStore { keys: vec![] };
        let err = load_bundle_dir(dir.path(), &LoadPolicy::default(), &trust)
            .await
            .unwrap_err();
        assert!(matches!(err, BundleLoadError::Manifest(_)));
        assert_eq!(err.outcome(), LoadOutcome::ManifestInvalid);
    }

    #[tokio::test]
    async fn signature_required_but_missing_fails() {
        let b = build_test_bundle(|_| {});
        // Don't write_signature.
        let trust = trust_for(&b);
        let err = load_bundle_dir(b.dir.path(), &LoadPolicy::default(), &trust)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BundleLoadError::SignatureRequiredButMissing(_)
        ));
        assert_eq!(err.outcome(), LoadOutcome::SignatureFailed);
    }

    #[tokio::test]
    async fn signature_warn_with_no_sig_succeeds() {
        let b = build_test_bundle(|_| {});
        let trust = trust_for(&b);
        let policy = LoadPolicy {
            signature: SignaturePolicy::Warn,
            ..LoadPolicy::default()
        };
        let bundle = load_bundle_dir(b.dir.path(), &policy, &trust)
            .await
            .expect("warn allows missing");
        assert_eq!(bundle.signing_key_id(), None);
    }

    #[tokio::test]
    async fn signature_warn_with_invalid_sig_fails() {
        let b = build_test_bundle(|_| {});
        // Write garbage signature bytes (right length, wrong key).
        std::fs::write(b.dir.path().join("manifest.json.sig"), [0u8; 64]).unwrap();
        let trust = trust_for(&b);
        let policy = LoadPolicy {
            signature: SignaturePolicy::Warn,
            ..LoadPolicy::default()
        };
        let err = load_bundle_dir(b.dir.path(), &policy, &trust)
            .await
            .unwrap_err();
        assert!(matches!(err, BundleLoadError::Signature(_)));
        assert_eq!(err.outcome(), LoadOutcome::SignatureFailed);
    }

    #[tokio::test]
    async fn signature_skip_ignores_invalid_sig() {
        let b = build_test_bundle(|_| {});
        // Even an obviously-broken sig is ignored under Skip.
        std::fs::write(b.dir.path().join("manifest.json.sig"), [0u8; 64]).unwrap();
        let trust = TrustStore { keys: vec![] };
        let policy = LoadPolicy {
            signature: SignaturePolicy::Skip,
            ..LoadPolicy::default()
        };
        let bundle = load_bundle_dir(b.dir.path(), &policy, &trust)
            .await
            .expect("skip ignores sig");
        assert_eq!(bundle.signing_key_id(), None);
    }

    #[tokio::test]
    async fn tampered_weights_rejected() {
        let b = build_test_bundle(|_| {});
        write_signature(&b);
        // Tamper with the weights AFTER signing — simulates T-16-3.
        std::fs::write(b.dir.path().join("weights/model.onnx"), b"TAMPERED").unwrap();
        let trust = trust_for(&b);
        let err = load_bundle_dir(b.dir.path(), &LoadPolicy::default(), &trust)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BundleLoadError::Verify(VerifyError::SizeMismatch { .. })
                | BundleLoadError::Verify(VerifyError::HashMismatch { .. })
        ));
        assert_eq!(err.outcome(), LoadOutcome::HashMismatch);
    }

    #[tokio::test]
    async fn sbom_required_but_missing_fails() {
        let b = build_test_bundle(|m| {
            m.sbom_path = None;
        });
        write_signature(&b);
        let trust = trust_for(&b);
        let err = load_bundle_dir(b.dir.path(), &LoadPolicy::default(), &trust)
            .await
            .unwrap_err();
        assert!(matches!(err, BundleLoadError::SbomRequiredButMissing));
        assert_eq!(err.outcome(), LoadOutcome::PolicyViolation);
    }

    #[tokio::test]
    async fn nonredist_license_without_ack_fails() {
        let b = build_test_bundle(|m| {
            m.license.redistribution_allowed = false;
            m.license.spdx_id = "CC-BY-NC-4.0".into();
        });
        write_signature(&b);
        let trust = trust_for(&b);
        let err = load_bundle_dir(b.dir.path(), &LoadPolicy::default(), &trust)
            .await
            .unwrap_err();
        assert!(matches!(err, BundleLoadError::NonRedistributableLicense(_)));
        assert_eq!(err.outcome(), LoadOutcome::PolicyViolation);
    }

    #[tokio::test]
    async fn nonredist_license_with_ack_succeeds() {
        let b = build_test_bundle(|m| {
            m.license.redistribution_allowed = false;
            m.license.spdx_id = "CC-BY-NC-4.0".into();
        });
        write_signature(&b);
        let trust = trust_for(&b);
        let policy = LoadPolicy {
            ack_non_redistributable: true,
            ..LoadPolicy::default()
        };
        load_bundle_dir(b.dir.path(), &policy, &trust)
            .await
            .expect("ack allows nonredist");
    }

    #[tokio::test]
    async fn fingerprint_is_sha256_of_manifest_bytes() {
        let b = build_test_bundle(|_| {});
        write_signature(&b);
        let trust = trust_for(&b);
        let bundle = load_bundle_dir(b.dir.path(), &LoadPolicy::default(), &trust)
            .await
            .unwrap();
        let expected = hex::encode(Sha256::digest(&b.manifest_bytes));
        assert_eq!(bundle.fingerprint().hex(), expected);
    }

    #[tokio::test]
    async fn outcome_label_strings_match_architecture() {
        // Architecture § 13.3 lists exactly these 6 outcomes.
        assert_eq!(LoadOutcome::Ok.as_label(), "ok");
        assert_eq!(LoadOutcome::ManifestInvalid.as_label(), "manifest_invalid");
        assert_eq!(LoadOutcome::SignatureFailed.as_label(), "signature_failed");
        assert_eq!(LoadOutcome::HashMismatch.as_label(), "hash_mismatch");
        assert_eq!(
            LoadOutcome::RuntimeInitFailed.as_label(),
            "runtime_init_failed"
        );
        assert_eq!(LoadOutcome::PolicyViolation.as_label(), "policy_violation");
    }
}
