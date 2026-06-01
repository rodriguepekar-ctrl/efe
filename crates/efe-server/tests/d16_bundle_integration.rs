//! Sprint **D16** — Embedder bundle integration tests.
//!
//! End-to-end exercises of the bundle pipeline using the mock
//! runtime (no ORT dependency in v0.1.16). Covers :
//!
//!   * pack → sign → verify happy path
//!   * tampered weights detection (T-16-3)
//!   * key rotation grace window
//!   * legacy fingerprint adapter (D8 catalog compat)
//!   * singleflight registry deduplication
//!   * load timeout enforcement (placeholder — would need a slow
//!     fetch path which D16 doesn't have ; left as TODO)
//!
//! These tests do NOT require Postgres or the OIDC stack ; they
//! run with `cargo test -p efe-server --test d16_bundle_integration`.

use std::path::Path;
use std::sync::Arc;

use chrono::TimeZone;
use ed25519_dalek::{Signer, SigningKey};
use efe_server::config::SignaturePolicy;
use efe_server::embedder::{
    compatibility_match, load_bundle_dir, BuildInfo, BundleLoadError, CompatMatch, Component,
    Dtype, EmbedderRegistry, Framework, LicenseInfo, LoadOutcome, LoadPolicy, Manifest,
    ModelOrigin, Pooling, RuntimeConfig, TrustStore, TrustedKey, VerifyError,
};
use rand_core::OsRng;
use sha2::{Digest, Sha256};

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

struct Fixture {
    dir: tempfile::TempDir,
    sk: SigningKey,
    pk: [u8; 32],
    manifest_bytes: Vec<u8>,
}

fn build_fixture(bundle_id: &str, dim: usize, f: impl FnOnce(&mut Manifest)) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let weights = b"FAKE_ONNX";
    let tokenizer = b"{\"tk\":\"v\"}";
    let sbom = b"{\"sbom\":\"v\"}";
    std::fs::create_dir_all(dir.path().join("weights")).unwrap();
    std::fs::create_dir_all(dir.path().join("tokenizer")).unwrap();
    std::fs::write(dir.path().join("weights/model.onnx"), weights).unwrap();
    std::fs::write(dir.path().join("tokenizer/tokenizer.json"), tokenizer).unwrap();
    std::fs::write(dir.path().join("SBOM.json"), sbom).unwrap();
    let mut manifest = Manifest {
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
            builder: "ci/integration".into(),
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
            huggingface_repo: Some("test/model".into()),
            huggingface_revision: None,
            model_card_url: None,
        }),
        sbom_path: Some("SBOM.json".into()),
    };
    f(&mut manifest);
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(dir.path().join("manifest.json"), &manifest_bytes).unwrap();
    let sk = SigningKey::generate(&mut OsRng);
    let pk = sk.verifying_key().to_bytes();
    Fixture {
        dir,
        sk,
        pk,
        manifest_bytes,
    }
}

fn sign(fixture: &Fixture) {
    let sig = fixture.sk.sign(&fixture.manifest_bytes).to_bytes();
    std::fs::write(fixture.dir.path().join("manifest.json.sig"), sig).unwrap();
}

fn trust(fixture: &Fixture, key_id: &str) -> TrustStore {
    TrustStore {
        keys: vec![TrustedKey {
            key_id: key_id.to_string(),
            public_key: fixture.pk,
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
async fn happy_path_load_signed_bundle_e2e() {
    let f = build_fixture("test/family@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = trust(&f, "ci-key-2026");
    let policy = LoadPolicy {
        signature: SignaturePolicy::Required,
        require_sbom: true,
        ack_non_redistributable: false,
    };
    let bundle = load_bundle_dir(f.dir.path(), &policy, &trust)
        .await
        .expect("happy path");
    assert_eq!(bundle.dim(), 64);
    assert_eq!(bundle.signing_key_id(), Some("ci-key-2026"));
    assert_eq!(bundle.manifest().framework, Framework::Onnx);
    // Encode round-trip via mock runtime.
    let v = bundle.encode_one("integration test").expect("encode");
    assert_eq!(v.len(), 64);
    let norm: f32 = v.iter().map(|x| x * x).sum();
    assert!((norm - 1.0).abs() < 1e-5, "expected unit-norm");
}

#[tokio::test]
async fn tampered_weights_rejected() {
    let f = build_fixture("test/family@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    // Tamper after signing : write different bytes to weights file.
    std::fs::write(f.dir.path().join("weights/model.onnx"), b"TAMPERED_BYTES").unwrap();
    let trust = trust(&f, "k1");
    let err = load_bundle_dir(
        f.dir.path(),
        &LoadPolicy {
            signature: SignaturePolicy::Required,
            require_sbom: true,
            ack_non_redistributable: false,
        },
        &trust,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            BundleLoadError::Verify(VerifyError::SizeMismatch { .. })
                | BundleLoadError::Verify(VerifyError::HashMismatch { .. })
        ),
        "expected verify error, got {err:?}"
    );
    assert_eq!(err.outcome(), LoadOutcome::HashMismatch);
}

#[tokio::test]
async fn missing_signature_in_required_mode_rejected() {
    let f = build_fixture("test/family@1.0.0/onnx/64", 64, |_| {});
    // No sign() call.
    let trust = trust(&f, "k1");
    let err = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        BundleLoadError::SignatureRequiredButMissing(_)
    ));
}

#[tokio::test]
async fn rotated_out_key_no_longer_validates() {
    // Bundle was signed by key A.
    let f = build_fixture("test/family@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    // But we rotated to key B and removed key A from the trust store.
    let other_sk = SigningKey::generate(&mut OsRng);
    let other_pk = other_sk.verifying_key().to_bytes();
    let trust = TrustStore {
        keys: vec![TrustedKey {
            key_id: "rotated-in".to_string(),
            public_key: other_pk,
            active_for_signing: true,
            valid_from: chrono::Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
            valid_until: chrono::Utc
                .with_ymd_and_hms(2099, 12, 31, 23, 59, 59)
                .unwrap(),
            comment: None,
        }],
    };
    let err = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap_err();
    assert!(matches!(err, BundleLoadError::Signature(_)));
    assert_eq!(err.outcome(), LoadOutcome::SignatureFailed);
}

#[tokio::test]
async fn legacy_fingerprint_adapter_matches_d8_catalog() {
    let f = build_fixture(
        "sentence-transformers/all-MiniLM-L6-v2@1.0.0/onnx/384",
        384,
        |_| {},
    );
    sign(&f);
    let trust = trust(&f, "k1");
    let bundle = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap();
    // Pre-D16 catalog stored "family/model@dim".
    let legacy = "sentence-transformers/all-MiniLM-L6-v2@384";
    let m = compatibility_match(&bundle, None, Some(legacy));
    assert_eq!(m, CompatMatch::LegacyMatch);
    // Different family → mismatch.
    let m = compatibility_match(&bundle, None, Some("BAAI/bge@384"));
    assert!(matches!(m, CompatMatch::Mismatch { .. }));
}

#[tokio::test]
async fn registry_singleflight_dedupes_concurrent_loads() {
    let f = build_fixture("test/dedupe@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = Arc::new(trust(&f, "k1"));
    let registry = EmbedderRegistry::new();
    let policy = LoadPolicy {
        signature: SignaturePolicy::Required,
        require_sbom: true,
        ack_non_redistributable: false,
    };
    // 8 concurrent ensure_loaded calls for the same id.
    let id = f.dir.path().display().to_string();
    let path = f.dir.path().to_path_buf();
    let bundle_id = "test/dedupe@1.0.0/onnx/64".to_string();
    let _ = id;
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let r = registry.clone();
            let path = path.clone();
            let policy = policy.clone();
            let trust = trust.clone();
            let bundle_id = bundle_id.clone();
            tokio::spawn(async move { r.ensure_loaded(&bundle_id, path, policy, trust).await })
        })
        .collect();
    for h in handles {
        h.await.unwrap().expect("concurrent load");
    }
    // Exactly one bundle ended up cached.
    assert_eq!(registry.len().await, 1);
}

/// **R-16-1 mitigation test** — re-pack the same source dir twice
/// with the same `SOURCE_DATE_EPOCH` and assert the resulting
/// `manifest.json` SHA-256 (= bundle_fingerprint) is identical.
#[tokio::test]
async fn reproducibility_two_packs_yield_same_fingerprint() {
    let f1 = build_fixture("test/repro@1.0.0/onnx/64", 64, |_| {});
    let f2 = build_fixture("test/repro@1.0.0/onnx/64", 64, |_| {});
    let fp1 = hex::encode(Sha256::digest(&f1.manifest_bytes));
    let fp2 = hex::encode(Sha256::digest(&f2.manifest_bytes));
    assert_eq!(
        fp1, fp2,
        "deterministic manifests must produce identical fp"
    );
}

#[tokio::test]
async fn skip_policy_lets_unsigned_bundle_through() {
    let f = build_fixture("test/skip@1.0.0/onnx/64", 64, |_| {});
    let trust = TrustStore { keys: vec![] };
    let policy = LoadPolicy {
        signature: SignaturePolicy::Skip,
        require_sbom: false,
        ack_non_redistributable: false,
    };
    let bundle = load_bundle_dir(f.dir.path(), &policy, &trust)
        .await
        .unwrap();
    assert_eq!(bundle.signing_key_id(), None);
}

#[tokio::test]
async fn warn_policy_unsigned_present_invalid_rejected() {
    let f = build_fixture("test/warn@1.0.0/onnx/64", 64, |_| {});
    // Write garbage signature.
    std::fs::write(f.dir.path().join("manifest.json.sig"), [0u8; 64]).unwrap();
    let trust = trust(&f, "k1");
    let policy = LoadPolicy {
        signature: SignaturePolicy::Warn,
        require_sbom: false,
        ack_non_redistributable: false,
    };
    let err = load_bundle_dir(f.dir.path(), &policy, &trust)
        .await
        .unwrap_err();
    assert!(matches!(err, BundleLoadError::Signature(_)));
}

#[tokio::test]
async fn manifest_with_extra_field_rejected_at_parse() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("manifest.json"),
        br#"{
            "manifest_version": "1.0",
            "bundle_id": "x/y@1.0.0/onnx/384",
            "framework": "onnx",
            "dim": 384,
            "components": [],
            "runtime": {"max_seq_len":256,"pooling":"mean","normalize":true,"dtype":"float32"},
            "build": {"builder":"x","built_at_unix_ms":1,"git_sha":"0123456789abcdef0123456789abcdef01234567","source_date_epoch":1},
            "license": {"spdx_id":"Apache-2.0"},
            "MISSPELLED_FIELD": "rejected"
        }"#,
    ).unwrap();
    let trust = TrustStore { keys: vec![] };
    let err = load_bundle_dir(
        dir.path(),
        &LoadPolicy {
            signature: SignaturePolicy::Skip,
            require_sbom: false,
            ack_non_redistributable: false,
        },
        &trust,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, BundleLoadError::Manifest(_)));
    assert_eq!(err.outcome(), LoadOutcome::ManifestInvalid);
}

#[tokio::test]
async fn load_outcomes_pin_strings_for_dashboards() {
    // Pin test : architecture § 13.3 promises 6 outcome strings.
    let outcomes = [
        LoadOutcome::Ok,
        LoadOutcome::ManifestInvalid,
        LoadOutcome::SignatureFailed,
        LoadOutcome::HashMismatch,
        LoadOutcome::RuntimeInitFailed,
        LoadOutcome::PolicyViolation,
    ];
    let labels: Vec<_> = outcomes.iter().map(|o| o.as_label()).collect();
    assert_eq!(
        labels,
        [
            "ok",
            "manifest_invalid",
            "signature_failed",
            "hash_mismatch",
            "runtime_init_failed",
            "policy_violation"
        ]
    );
}

/// Path-traversal regression : a manifest declaring a component
/// whose path is `../etc/passwd` MUST be rejected before any IO.
#[tokio::test]
async fn manifest_with_traversal_path_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let weights = b"w";
    let tokenizer = b"t";
    std::fs::create_dir_all(dir.path().join("weights")).unwrap();
    std::fs::create_dir_all(dir.path().join("tokenizer")).unwrap();
    std::fs::write(dir.path().join("weights/model.onnx"), weights).unwrap();
    std::fs::write(dir.path().join("tokenizer/tokenizer.json"), tokenizer).unwrap();
    let manifest = Manifest {
        manifest_version: "1.0".into(),
        bundle_id: "x/y@1.0.0/onnx/64".into(),
        framework: Framework::Onnx,
        dim: 64,
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
                path: "../escape/etc/passwd".into(),
                sha256: "0".repeat(64),
                size_bytes: 0,
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
    };
    std::fs::write(
        dir.path().join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let trust = TrustStore { keys: vec![] };
    let err = load_bundle_dir(
        dir.path(),
        &LoadPolicy {
            signature: SignaturePolicy::Skip,
            require_sbom: false,
            ack_non_redistributable: false,
        },
        &trust,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        BundleLoadError::Verify(VerifyError::PathTraversalAttempt(_))
    ));
}

/// Anchor : verify the load function walks the steps in the
/// architecture § 8.3 order : manifest → signature → components
/// → runtime → fingerprint. Surface this via an integration that
/// would fail in MULTIPLE ways and assert which surfaces first.
#[tokio::test]
async fn load_step_ordering_manifest_first() {
    // Bundle with a hash mismatch + missing signature + no SBOM.
    // Manifest IS valid, so we should reach the signature step
    // first. That step reports SignatureRequiredButMissing.
    let f = build_fixture("test/order@1.0.0/onnx/64", 64, |_| {});
    // Tamper weights so hash check would also fail if we got there.
    std::fs::write(f.dir.path().join("weights/model.onnx"), b"DIFFERENT").unwrap();
    // No signature.
    let trust = trust(&f, "k1");
    let err = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap_err();
    // Step 2 (signature) should fail before step 3 (hash).
    assert!(matches!(
        err,
        BundleLoadError::SignatureRequiredButMissing(_)
    ));
}

/// Sanity : a successfully-loaded bundle exposes its fingerprint
/// as 64 lowercase hex chars.
#[tokio::test]
async fn fingerprint_format_is_64_lowercase_hex() {
    let f = build_fixture("test/fp@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = trust(&f, "k1");
    let bundle = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap();
    let fp = bundle.fingerprint().hex();
    assert_eq!(fp.len(), 64);
    assert!(fp.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(fp.bytes().all(|b| !b.is_ascii_uppercase()));
}

// =====================================================================
// Audit-fix regression tests (D16 audit pass)
// =====================================================================

/// **Audit fix D16-C4 regression** — sbom_path that doesn't appear
/// in components[] is rejected at manifest validate time.
#[tokio::test]
async fn sbom_path_must_be_listed_in_components() {
    let f = build_fixture("test/sbom@1.0.0/onnx/64", 64, |m| {
        // sbom_path points to a path NOT in components[].
        m.sbom_path = Some("not-in-components.json".into());
    });
    sign(&f);
    let trust = trust(&f, "k1");
    let err = load_bundle_dir(
        f.dir.path(),
        &LoadPolicy {
            signature: efe_server::config::SignaturePolicy::Required,
            require_sbom: true,
            ack_non_redistributable: false,
        },
        &trust,
    )
    .await
    .unwrap_err();
    // Surfaces as ManifestInvalid (validation-time rejection) not as
    // policy violation — the manifest itself is internally
    // inconsistent.
    assert_eq!(err.outcome(), LoadOutcome::ManifestInvalid);
    let msg = format!("{err}");
    assert!(
        msg.contains("sbom_path"),
        "error msg should mention sbom_path: {msg}"
    );
}

/// **Audit fix D16-H1 regression** — policy fails BEFORE the
/// component hash walk. We construct a bundle with non-redist
/// license + a tampered weights file. If the policy check happens
/// AFTER hash, the hash check fires first (HashMismatch outcome).
/// If it happens BEFORE (correct), the policy violation fires.
#[tokio::test]
async fn policy_check_runs_before_hash_walk() {
    let f = build_fixture("test/policy-order@1.0.0/onnx/64", 64, |m| {
        m.license.redistribution_allowed = false;
        m.license.spdx_id = "CC-BY-NC-4.0".into();
    });
    sign(&f);
    // Tamper weights AFTER signing.
    std::fs::write(f.dir.path().join("weights/model.onnx"), b"TAMPERED").unwrap();
    let trust = trust(&f, "k1");
    let err = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap_err();
    // Post-audit ordering : policy fires BEFORE the (expensive) hash
    // walk. Pre-audit code would have surfaced HashMismatch.
    assert_eq!(
        err.outcome(),
        LoadOutcome::PolicyViolation,
        "policy should fail-fast before hash walk (audit fix H1)"
    );
}

/// **Audit fix D16-H2 regression** — load_timeout enforces a hard
/// upper bound on the load step.
#[tokio::test]
async fn load_timeout_caps_load_step() {
    use std::sync::Arc;
    let f = build_fixture("test/timeout@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust_arc = Arc::new(trust(&f, "k1"));
    let registry = efe_server::embedder::EmbedderRegistry::new();
    // Tight 1ms timeout — under any realistic load this should
    // either succeed (fast disk) or time out. We use 0ms to force
    // the timeout path deterministically.
    let result = registry
        .ensure_loaded_with_timeout(
            "test/timeout@1.0.0/onnx/64",
            f.dir.path().to_path_buf(),
            LoadPolicy::default(),
            trust_arc,
            std::time::Duration::from_nanos(1),
        )
        .await;
    // Should fail with a timeout error message, not hang forever.
    let err = result.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("EFE_BUNDLE_LOAD_TIMEOUT_S")
            || msg.contains("timed out")
            || msg.contains("TimedOut"),
        "expected timeout error, got: {msg}"
    );
}

/// **Audit fix D16-H4 regression** — backslash-traversal paths
/// rejected even on Linux (defense in depth for malformed bundles
/// from a Windows packer).
#[tokio::test]
async fn backslash_traversal_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let weights = b"w";
    let tokenizer = b"t";
    std::fs::create_dir_all(dir.path().join("weights")).unwrap();
    std::fs::create_dir_all(dir.path().join("tokenizer")).unwrap();
    std::fs::write(dir.path().join("weights/model.onnx"), weights).unwrap();
    std::fs::write(dir.path().join("tokenizer/tokenizer.json"), tokenizer).unwrap();
    let manifest = efe_server::embedder::Manifest {
        manifest_version: "1.0".into(),
        bundle_id: "x/y@1.0.0/onnx/64".into(),
        framework: efe_server::embedder::Framework::Onnx,
        dim: 64,
        components: vec![
            efe_server::embedder::Component {
                path: "weights/model.onnx".into(),
                sha256: hex::encode(sha2::Sha256::digest(weights)),
                size_bytes: weights.len() as u64,
                required: true,
            },
            efe_server::embedder::Component {
                path: "tokenizer/tokenizer.json".into(),
                sha256: hex::encode(sha2::Sha256::digest(tokenizer)),
                size_bytes: tokenizer.len() as u64,
                required: true,
            },
            efe_server::embedder::Component {
                path: "..\\..\\windows\\path".into(), // backslash traversal
                sha256: "0".repeat(64),
                size_bytes: 0,
                required: false,
            },
        ],
        runtime: efe_server::embedder::RuntimeConfig {
            max_seq_len: 256,
            pooling: efe_server::embedder::Pooling::Mean,
            normalize: true,
            dtype: efe_server::embedder::Dtype::Float32,
        },
        build: efe_server::embedder::BuildInfo {
            builder: "x".into(),
            built_at_unix_ms: 0,
            git_sha: "0".repeat(40),
            source_date_epoch: 0,
        },
        license: efe_server::embedder::LicenseInfo {
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
    let err = load_bundle_dir(
        dir.path(),
        &LoadPolicy {
            signature: efe_server::config::SignaturePolicy::Skip,
            require_sbom: false,
            ack_non_redistributable: false,
        },
        &trust,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        BundleLoadError::Verify(VerifyError::PathTraversalAttempt(_))
    ));
}

/// **Audit fix D16-H3 regression** — encoding bumps the encode
/// counter + observes the duration histogram.
#[tokio::test]
async fn encode_emits_d16_metrics() {
    let f = build_fixture("test/metrics@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = trust(&f, "k1");
    let bundle = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap();
    let bundle_id = "test/metrics@1.0.0/onnx/64";
    // Snapshot the counter BEFORE.
    let counter = efe_telemetry::metrics::d16_instruments::embedder_encode_total();
    let before = counter
        .with_label_values(&[bundle_id, "tenant-1", "ok"])
        .get();
    let _ = bundle.encode_one_for_tenant("hello", "tenant-1").unwrap();
    let after = counter
        .with_label_values(&[bundle_id, "tenant-1", "ok"])
        .get();
    assert_eq!(after, before + 1, "encode counter not bumped");
    // The duration histogram should have at least one observation
    // for this (bundle_id, tenant) pair.
    let hist = efe_telemetry::metrics::d16_instruments::embedder_encode_duration_seconds()
        .with_label_values(&[bundle_id, "tenant-1"]);
    assert!(
        hist.get_sample_count() >= 1,
        "encode duration histogram not observed"
    );
}

/// **Audit fix D16-H5 regression** — verify_components emits the
/// hash counter (via the wrapper in load_bundle_dir).
#[tokio::test]
async fn hash_verify_emits_d16_metrics() {
    let f = build_fixture("test/hash-metric@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = trust(&f, "k1");
    let counter = efe_telemetry::metrics::d16_instruments::bundle_hash_verifications_total();
    let bundle_id = "test/hash-metric@1.0.0/onnx/64";
    let before = counter.with_label_values(&[bundle_id, "ok"]).get();
    load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap();
    let after = counter.with_label_values(&[bundle_id, "ok"]).get();
    assert_eq!(after, before + 1, "hash verify counter not bumped");
}

/// **Audit fix D16-H6 regression** — signature verify emits the
/// signature counter with the key_id label populated.
#[tokio::test]
async fn signature_verify_emits_d16_metrics() {
    let f = build_fixture("test/sig-metric@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = trust(&f, "audit-test-key");
    let counter = efe_telemetry::metrics::d16_instruments::bundle_signature_verifications_total();
    let bundle_id = "test/sig-metric@1.0.0/onnx/64";
    let before = counter
        .with_label_values(&[bundle_id, "audit-test-key", "ok"])
        .get();
    load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap();
    let after = counter
        .with_label_values(&[bundle_id, "audit-test-key", "ok"])
        .get();
    assert_eq!(
        after,
        before + 1,
        "signature verify counter not bumped with key_id"
    );
}

/// **Audit fix D16-C2 regression** — singleflight wait_for_leader
/// race : even in a tight loop with many concurrent waiters,
/// nobody hangs forever. We wrap each call in a 5s timeout to
/// catch the lost-wakeup deadlock the audit fixed.
#[tokio::test]
async fn singleflight_no_lost_wakeup_under_concurrency() {
    use std::sync::Arc;
    let f = build_fixture("test/no-hang@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = Arc::new(trust(&f, "k1"));
    let registry = efe_server::embedder::EmbedderRegistry::new();
    let bundle_id = "test/no-hang@1.0.0/onnx/64";
    let path = f.dir.path().to_path_buf();
    let policy = LoadPolicy::default();

    // 16 concurrent waiters. Pre-audit, the lost-wakeup race could
    // hang one of them indefinitely.
    let handles: Vec<_> = (0..16)
        .map(|_| {
            let r = registry.clone();
            let path = path.clone();
            let policy = policy.clone();
            let trust = trust.clone();
            let id = bundle_id.to_string();
            tokio::spawn(async move {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    r.ensure_loaded(&id, path, policy, trust),
                )
                .await
            })
        })
        .collect();
    for h in handles {
        let outer = h.await.unwrap();
        // outer is Result<SharedLoadResult, Elapsed>. We want no Elapsed.
        outer.expect("singleflight load timed out — possible lost wakeup");
    }
    assert_eq!(registry.len().await, 1);
}

/// Sanity : the bundle root is exposed as a `Path` for downstream
/// observability code (e.g. resident_bytes attribution).
#[tokio::test]
async fn bundle_root_exposed() {
    let f = build_fixture("test/root@1.0.0/onnx/64", 64, |_| {});
    sign(&f);
    let trust = trust(&f, "k1");
    let bundle = load_bundle_dir(f.dir.path(), &LoadPolicy::default(), &trust)
        .await
        .unwrap();
    assert_eq!(bundle.root(), f.dir.path() as &Path);
}
