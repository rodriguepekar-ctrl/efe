//! Sprint D17 — backup manifest types + Ed25519 sign/verify.
//!
//! Cf. architecture § 6 (manifest schema) + § 14 (env tag inside
//! the signature) + § 27 (manifest version pinning).
//!
//! Each snapshot produces a JSON manifest that lists every file
//! shipped to the backup repo, with per-file SHA-256, plus a
//! detached Ed25519 signature over the canonical bytes of the
//! manifest body. The signature key lives in a JSON trust store
//! identical in shape to D16's `pubkeys.json`.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey, SIGNATURE_LENGTH};
use serde::{Deserialize, Serialize};

use crate::config::RestoreTargetEnv;
use crate::error::BackupError;
use crate::MANIFEST_VERSION;

/// One file or sub-resource recorded in a manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestComponent {
    /// Symbolic name of the source (e.g. "postgres", "vault", "bundles").
    pub source: String,
    /// File path inside the snapshot repo (relative to the source root).
    pub path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Hex-lowercase SHA-256 of the file content.
    pub sha256: String,
}

/// Top-level backup manifest. Serialised as JSON, signed by
/// [`sign_manifest_payload`], stored in the backup repo as
/// `manifest.json` + `manifest.json.sig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    /// Schema version. Must match [`crate::MANIFEST_VERSION`] for a
    /// strict reader to accept it (architecture § 27).
    pub version: String,

    /// Symbolic snapshot label (e.g. "20260428T060000Z-full").
    pub label: String,

    /// Environment tag (`prod`, `staging`, `drill`, `dev`). Embedded
    /// inside the signature ; cross-env restore guard refuses
    /// `manifest.env != target_env`. T-DR-6 mitigation.
    pub env: String,

    /// Cluster identifier (e.g. "efe-prod-eu-west-1").
    pub cluster_id: String,

    /// UTC timestamp of snapshot start.
    pub created_at: DateTime<Utc>,

    /// One entry per file/sub-resource in the snapshot.
    pub components: Vec<ManifestComponent>,

    /// Hex SHA-256 of the entire manifest *body* (the JSON of this
    /// struct excluding the `sha256_self` and `ed25519_sig`
    /// fields). Used for tamper detection at the repo level.
    pub sha256_self: String,

    /// Detached Ed25519 signature in base64. Verified against a
    /// trust store at restore time.
    pub ed25519_sig: String,

    /// `key_id` of the trust-store entry that signed this manifest.
    /// Allows trust-store rotation without invalidating older
    /// backups.
    pub key_id: String,
}

impl BackupManifest {
    /// Construct a draft manifest **without** the signature. The
    /// caller must invoke [`sign_manifest_payload`] to populate
    /// `sha256_self` + `ed25519_sig` + `key_id` before persisting.
    pub fn draft(
        label: impl Into<String>,
        env: RestoreTargetEnv,
        cluster_id: impl Into<String>,
        components: Vec<ManifestComponent>,
    ) -> Self {
        Self {
            version: MANIFEST_VERSION.to_string(),
            label: label.into(),
            env: env.as_str().to_string(),
            cluster_id: cluster_id.into(),
            created_at: Utc::now(),
            components,
            sha256_self: String::new(),
            ed25519_sig: String::new(),
            key_id: String::new(),
        }
    }

    /// Validate static invariants. Called by `verify_manifest_payload`
    /// before checking the signature so corrupt manifests fail fast.
    pub fn check_invariants(&self) -> Result<(), BackupError> {
        if self.version != MANIFEST_VERSION {
            return Err(BackupError::UnknownManifestVersion {
                got: self.version.clone(),
                supported: MANIFEST_VERSION,
            });
        }
        if self.label.is_empty() {
            return Err(BackupError::ManifestInvariant(
                "manifest.label must not be empty".into(),
            ));
        }
        if self.env.is_empty() {
            return Err(BackupError::ManifestInvariant(
                "manifest.env must not be empty (T-DR-6 cross-env guard)".into(),
            ));
        }
        if self.components.is_empty() {
            return Err(BackupError::ManifestInvariant(
                "manifest.components must not be empty".into(),
            ));
        }
        // Each component must have a non-empty source and path.
        for (idx, c) in self.components.iter().enumerate() {
            if c.source.is_empty() {
                return Err(BackupError::ManifestInvariant(format!(
                    "components[{idx}].source must not be empty"
                )));
            }
            if c.path.is_empty() {
                return Err(BackupError::ManifestInvariant(format!(
                    "components[{idx}].path must not be empty"
                )));
            }
            if c.sha256.len() != 64 || !c.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(BackupError::ManifestInvariant(format!(
                    "components[{idx}].sha256 must be 64 hex chars"
                )));
            }
        }
        Ok(())
    }
}

/// Public-key entry in the trust store JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustStoreEntry {
    /// Symbolic identifier (e.g. "backup-2026-q2").
    pub key_id: String,
    /// Hex-lowercase Ed25519 public key (32 bytes → 64 hex chars).
    pub pubkey_hex: String,
    /// ISO-8601 validity window start. Manifests older than this
    /// are rejected.
    pub valid_from: DateTime<Utc>,
    /// Optional validity window end. `None` means "currently valid".
    pub valid_until: Option<DateTime<Utc>>,
}

/// Trust store loaded from a JSON file. Maps `key_id → entry`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    entries: HashMap<String, TrustStoreEntry>,
}

impl TrustStore {
    /// Build from raw entries.
    pub fn from_entries(entries: Vec<TrustStoreEntry>) -> Result<Self, BackupError> {
        let mut by_id = HashMap::new();
        for entry in entries {
            // Validate the pubkey shape.
            if entry.pubkey_hex.len() != 64 {
                return Err(BackupError::config(format!(
                    "trust-store entry {:?} has pubkey_hex of {} chars (expected 64)",
                    entry.key_id,
                    entry.pubkey_hex.len()
                )));
            }
            if !entry.pubkey_hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(BackupError::config(format!(
                    "trust-store entry {:?} has non-hex pubkey",
                    entry.key_id,
                )));
            }
            if by_id.contains_key(&entry.key_id) {
                return Err(BackupError::config(format!(
                    "trust-store has duplicate key_id {:?}",
                    entry.key_id
                )));
            }
            by_id.insert(entry.key_id.clone(), entry);
        }
        Ok(Self { entries: by_id })
    }

    /// Load from a JSON file. Format :
    /// ```json
    /// { "entries": [ { "key_id": "...", "pubkey_hex": "...",
    ///                  "valid_from": "...", "valid_until": null }, ... ] }
    /// ```
    pub fn load_file(path: &Path) -> Result<Self, BackupError> {
        let bytes = std::fs::read(path)?;
        #[derive(Deserialize)]
        struct File {
            entries: Vec<TrustStoreEntry>,
        }
        let file: File = serde_json::from_slice(&bytes)?;
        Self::from_entries(file.entries)
    }

    /// Look up an entry by `key_id`, asserting current validity.
    pub fn lookup_active(&self, key_id: &str, now: DateTime<Utc>) -> Option<&TrustStoreEntry> {
        let entry = self.entries.get(key_id)?;
        if now < entry.valid_from {
            return None;
        }
        if let Some(end) = entry.valid_until {
            if now > end {
                return None;
            }
        }
        Some(entry)
    }

    /// Number of entries currently in the store (active or not).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` iff the store has zero entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Compute the canonical body bytes of a manifest by serialising
/// it with `sha256_self` and `ed25519_sig` blanked.
fn canonical_body_bytes(manifest: &BackupManifest) -> Result<Vec<u8>, BackupError> {
    let mut copy = manifest.clone();
    copy.sha256_self = String::new();
    copy.ed25519_sig = String::new();
    copy.key_id = String::new();
    let body = serde_json::to_vec(&copy)?;
    Ok(body)
}

/// Sign a draft manifest in-place. Populates `sha256_self`,
/// `ed25519_sig`, and `key_id` fields on success.
pub fn sign_manifest_payload(
    manifest: &mut BackupManifest,
    signing_key: &SigningKey,
    key_id: &str,
) -> Result<(), BackupError> {
    let body = canonical_body_bytes(manifest)?;

    // SHA-256 of the canonical body.
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&body);
    let digest = hasher.finalize();
    let sha_hex = hex::encode(digest);

    // Ed25519 detached signature over the canonical body.
    let sig: Signature = signing_key.sign(&body);
    let sig_b64 = base64_encode(&sig.to_bytes());

    manifest.sha256_self = sha_hex;
    manifest.ed25519_sig = sig_b64;
    manifest.key_id = key_id.to_string();

    Ok(())
}

/// Verify a signed manifest against a trust store.
///
/// Returns `Ok(())` iff :
///   * `manifest.version == MANIFEST_VERSION`
///   * static invariants hold
///   * `manifest.key_id` is in the trust store and currently valid
///   * `manifest.sha256_self` matches the canonical body digest
///   * `manifest.ed25519_sig` verifies under the trust-store pubkey
pub fn verify_manifest_payload(
    manifest: &BackupManifest,
    trust: &TrustStore,
) -> Result<(), BackupError> {
    manifest.check_invariants()?;

    let entry = trust
        .lookup_active(&manifest.key_id, Utc::now())
        .ok_or_else(|| {
            BackupError::VerifyManifest(format!(
                "key_id {:?} not in active trust store",
                manifest.key_id
            ))
        })?;

    // Re-compute the canonical body bytes + digest.
    let body = canonical_body_bytes(manifest)?;
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&body);
    let digest = hasher.finalize();
    let sha_hex = hex::encode(digest);
    if sha_hex != manifest.sha256_self {
        return Err(BackupError::VerifyManifest(
            "sha256_self mismatch (manifest tampered)".into(),
        ));
    }

    // Decode + verify the Ed25519 signature.
    let sig_bytes = base64_decode(&manifest.ed25519_sig).map_err(|e| {
        BackupError::VerifyManifest(format!("ed25519_sig is not base64: {e}"))
    })?;
    if sig_bytes.len() != SIGNATURE_LENGTH {
        return Err(BackupError::VerifyManifest(format!(
            "ed25519_sig has {} bytes, expected {}",
            sig_bytes.len(),
            SIGNATURE_LENGTH
        )));
    }
    let mut sig_arr = [0u8; SIGNATURE_LENGTH];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&sig_arr);

    let pubkey_bytes = hex::decode(&entry.pubkey_hex).map_err(|e| {
        BackupError::VerifyManifest(format!("trust pubkey not hex: {e}"))
    })?;
    if pubkey_bytes.len() != 32 {
        return Err(BackupError::VerifyManifest(format!(
            "trust pubkey has {} bytes, expected 32",
            pubkey_bytes.len()
        )));
    }
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(&pubkey_bytes);
    let vk = VerifyingKey::from_bytes(&pk_arr).map_err(|e| {
        BackupError::VerifyManifest(format!("trust pubkey is not valid Ed25519: {e}"))
    })?;

    vk.verify(&body, &sig)
        .map_err(|_| BackupError::VerifyManifest("ed25519 verify failed".into()))?;

    Ok(())
}

// ----------------------------------------------------------------------
// Minimal base64 helpers — we don't pull a base64 crate to keep the
// BOM minimal. Only producer + consumer is this module itself.
// ----------------------------------------------------------------------

/// **Audit fix § D17-AC3** — public helper used by
/// [`crate::metadata::SnapshotMetadata::sign`] to encode the
/// 64-byte Ed25519 signature.
pub(crate) fn base64_encode_signature(bytes: &[u8]) -> String {
    base64_encode(bytes)
}

/// **Audit fix § D17-AC3** — public helper for verifying a snapshot
/// metadata signature against a trust store. Mirrors
/// [`verify_manifest_payload`] but operates on `SnapshotMetadata`.
pub fn verify_snapshot_metadata(
    meta: &crate::metadata::SnapshotMetadata,
    trust: &TrustStore,
    key_id: &str,
) -> Result<(), BackupError> {
    use sha2::Digest;
    let entry = trust
        .lookup_active(key_id, Utc::now())
        .ok_or_else(|| {
            BackupError::VerifyManifest(format!(
                "key_id {:?} not in active trust store",
                key_id
            ))
        })?;
    let mut canon = meta.clone();
    canon.sha256_manifest = String::new();
    canon.ed25519_sig = String::new();
    canon.error_message = None;
    let body = serde_json::to_vec(&canon)?;

    let mut hasher = sha2::Sha256::new();
    hasher.update(&body);
    let sha_hex = hex::encode(hasher.finalize());
    if sha_hex != meta.sha256_manifest {
        return Err(BackupError::VerifyManifest(
            "snapshot metadata sha mismatch (tampered)".into(),
        ));
    }

    let sig_bytes = base64_decode(&meta.ed25519_sig)
        .map_err(|e| BackupError::VerifyManifest(format!("ed25519_sig not base64: {e}")))?;
    if sig_bytes.len() != SIGNATURE_LENGTH {
        return Err(BackupError::VerifyManifest(format!(
            "sig has {} bytes, expected {}",
            sig_bytes.len(),
            SIGNATURE_LENGTH
        )));
    }
    let mut sig_arr = [0u8; SIGNATURE_LENGTH];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&sig_arr);

    let pubkey_bytes = hex::decode(&entry.pubkey_hex)
        .map_err(|e| BackupError::VerifyManifest(format!("pubkey not hex: {e}")))?;
    if pubkey_bytes.len() != 32 {
        return Err(BackupError::VerifyManifest(format!(
            "pubkey has {} bytes, expected 32",
            pubkey_bytes.len()
        )));
    }
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(&pubkey_bytes);
    let vk = VerifyingKey::from_bytes(&pk_arr).map_err(|e| {
        BackupError::VerifyManifest(format!("pubkey not valid Ed25519: {e}"))
    })?;
    vk.verify(&body, &sig)
        .map_err(|_| BackupError::VerifyManifest("ed25519 verify failed".into()))?;
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHA: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let triple = (u32::from(bytes[i]) << 16)
            | (u32::from(bytes[i + 1]) << 8)
            | u32::from(bytes[i + 2]);
        out.push(ALPHA[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHA[((triple >> 12) & 63) as usize] as char);
        out.push(ALPHA[((triple >> 6) & 63) as usize] as char);
        out.push(ALPHA[(triple & 63) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let triple = u32::from(bytes[i]) << 16;
        out.push(ALPHA[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHA[((triple >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let triple = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
        out.push(ALPHA[((triple >> 18) & 63) as usize] as char);
        out.push(ALPHA[((triple >> 12) & 63) as usize] as char);
        out.push(ALPHA[((triple >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: &[u8] = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("base64 length must be multiple of 4".into());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut i = 0;
    while i < bytes.len() {
        let chunk = &bytes[i..i + 4];
        let mut q = [0u8; 4];
        let mut pad = 0u32;
        for (idx, c) in chunk.iter().enumerate() {
            if *c == b'=' {
                pad += 1;
                q[idx] = 0;
            } else {
                q[idx] =
                    val(*c).ok_or_else(|| format!("invalid base64 char: {:?}", *c as char))?;
            }
        }
        let triple = (u32::from(q[0]) << 18)
            | (u32::from(q[1]) << 12)
            | (u32::from(q[2]) << 6)
            | u32::from(q[3]);
        out.push((triple >> 16) as u8);
        if pad < 2 {
            out.push((triple >> 8) as u8);
        }
        if pad < 1 {
            out.push(triple as u8);
        }
        i += 4;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand_core::OsRng;

    fn make_components() -> Vec<ManifestComponent> {
        vec![
            ManifestComponent {
                source: "postgres".into(),
                path: "main/full/20260428T060000Z/manifest.json".into(),
                size_bytes: 4096,
                sha256: "a".repeat(64),
            },
            ManifestComponent {
                source: "vault".into(),
                path: "vault-snapshots/20260428T063000Z.snap".into(),
                size_bytes: 1024,
                sha256: "b".repeat(64),
            },
        ]
    }

    #[test]
    fn draft_pre_sign_has_empty_signature_fields() {
        let m = BackupManifest::draft(
            "test-label",
            RestoreTargetEnv::Prod,
            "efe-prod",
            make_components(),
        );
        assert_eq!(m.version, MANIFEST_VERSION);
        assert_eq!(m.env, "prod");
        assert!(m.sha256_self.is_empty());
        assert!(m.ed25519_sig.is_empty());
        assert!(m.key_id.is_empty());
    }

    #[test]
    fn check_invariants_rejects_empty_components() {
        let m = BackupManifest::draft(
            "x",
            RestoreTargetEnv::Dev,
            "c",
            vec![],
        );
        assert!(matches!(
            m.check_invariants(),
            Err(BackupError::ManifestInvariant(_))
        ));
    }

    #[test]
    fn check_invariants_rejects_bad_sha256() {
        let mut m = BackupManifest::draft(
            "x",
            RestoreTargetEnv::Dev,
            "c",
            make_components(),
        );
        m.components[0].sha256 = "tooshort".into();
        assert!(matches!(
            m.check_invariants(),
            Err(BackupError::ManifestInvariant(_))
        ));
    }

    #[test]
    fn check_invariants_rejects_unknown_version() {
        let mut m = BackupManifest::draft(
            "x",
            RestoreTargetEnv::Dev,
            "c",
            make_components(),
        );
        m.version = "9.9.9".into();
        assert!(matches!(
            m.check_invariants(),
            Err(BackupError::UnknownManifestVersion { .. })
        ));
    }

    #[test]
    fn sign_then_verify_round_trip() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let pubkey_hex = hex::encode(vk.to_bytes());

        let trust = TrustStore::from_entries(vec![TrustStoreEntry {
            key_id: "test".into(),
            pubkey_hex,
            valid_from: Utc::now() - chrono::Duration::days(1),
            valid_until: None,
        }])
        .unwrap();

        let mut m = BackupManifest::draft(
            "round-trip",
            RestoreTargetEnv::Prod,
            "efe-prod",
            make_components(),
        );
        sign_manifest_payload(&mut m, &sk, "test").unwrap();

        assert!(!m.sha256_self.is_empty());
        assert!(!m.ed25519_sig.is_empty());
        assert_eq!(m.key_id, "test");

        verify_manifest_payload(&m, &trust).unwrap();
    }

    #[test]
    fn verify_rejects_tampered_label() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let trust = TrustStore::from_entries(vec![TrustStoreEntry {
            key_id: "test".into(),
            pubkey_hex: hex::encode(vk.to_bytes()),
            valid_from: Utc::now() - chrono::Duration::days(1),
            valid_until: None,
        }])
        .unwrap();

        let mut m = BackupManifest::draft(
            "original",
            RestoreTargetEnv::Prod,
            "efe-prod",
            make_components(),
        );
        sign_manifest_payload(&mut m, &sk, "test").unwrap();

        // Tamper the label after signing.
        m.label = "tampered".into();

        let err = verify_manifest_payload(&m, &trust).unwrap_err();
        assert!(matches!(err, BackupError::VerifyManifest(_)));
    }

    #[test]
    fn verify_rejects_unknown_key_id() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let trust = TrustStore::from_entries(vec![TrustStoreEntry {
            key_id: "other".into(),
            pubkey_hex: hex::encode(vk.to_bytes()),
            valid_from: Utc::now() - chrono::Duration::days(1),
            valid_until: None,
        }])
        .unwrap();
        let mut m = BackupManifest::draft(
            "x",
            RestoreTargetEnv::Prod,
            "c",
            make_components(),
        );
        sign_manifest_payload(&mut m, &sk, "test").unwrap();

        let err = verify_manifest_payload(&m, &trust).unwrap_err();
        assert!(matches!(err, BackupError::VerifyManifest(_)));
        assert!(err.to_string().contains("not in active trust store"));
    }

    #[test]
    fn verify_rejects_expired_key() {
        let sk = SigningKey::generate(&mut OsRng);
        let vk = sk.verifying_key();
        let trust = TrustStore::from_entries(vec![TrustStoreEntry {
            key_id: "test".into(),
            pubkey_hex: hex::encode(vk.to_bytes()),
            valid_from: Utc::now() - chrono::Duration::days(10),
            valid_until: Some(Utc::now() - chrono::Duration::days(1)),
        }])
        .unwrap();
        let mut m = BackupManifest::draft(
            "x",
            RestoreTargetEnv::Prod,
            "c",
            make_components(),
        );
        sign_manifest_payload(&mut m, &sk, "test").unwrap();

        let err = verify_manifest_payload(&m, &trust).unwrap_err();
        assert!(matches!(err, BackupError::VerifyManifest(_)));
    }

    #[test]
    fn trust_store_rejects_duplicate_key_id() {
        let entries = vec![
            TrustStoreEntry {
                key_id: "k".into(),
                pubkey_hex: "0".repeat(64),
                valid_from: Utc::now(),
                valid_until: None,
            },
            TrustStoreEntry {
                key_id: "k".into(),
                pubkey_hex: "1".repeat(64),
                valid_from: Utc::now(),
                valid_until: None,
            },
        ];
        let err = TrustStore::from_entries(entries).unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[test]
    fn trust_store_rejects_short_pubkey() {
        let entries = vec![TrustStoreEntry {
            key_id: "k".into(),
            pubkey_hex: "abc".into(),
            valid_from: Utc::now(),
            valid_until: None,
        }];
        let err = TrustStore::from_entries(entries).unwrap_err();
        assert!(matches!(err, BackupError::Config(_)));
    }

    #[test]
    fn base64_encode_decode_round_trip() {
        let cases: &[&[u8]] = &[
            b"",
            b"a",
            b"ab",
            b"abc",
            b"abcd",
            b"hello world",
            &[0u8; 64],
            &[255u8; 33],
        ];
        for c in cases {
            let enc = base64_encode(c);
            let dec = base64_decode(&enc).unwrap();
            assert_eq!(&dec[..], *c, "round-trip failed for {c:?}");
        }
    }

    #[test]
    fn base64_decode_rejects_invalid_chars() {
        assert!(base64_decode("!!!!").is_err());
        assert!(base64_decode("aaa").is_err()); // not multiple of 4
    }
}
