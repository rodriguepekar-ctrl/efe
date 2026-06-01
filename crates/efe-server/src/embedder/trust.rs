//! Sprint **D16** — Ed25519 trust store for bundle signatures.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 5.2.
//!
//! The trust store is a JSON file (typically mounted via K8s
//! Secret at `/etc/efe/bundles/pubkeys.json`) listing every
//! Ed25519 public key the runtime is willing to accept as a valid
//! signer for a bundle manifest.
//!
//! Multiple keys coexist to support rotation : when the CI rolls
//! a new signing key, the old key stays in the trust store until
//! every bundle signed by the old key has been retired
//! (architecture § 5.3).
//!
//! # Format
//!
//! ```jsonc
//! {
//!   "schema_version": "1.0",
//!   "keys": [
//!     {
//!       "key_id": "efe-prod-bundle-2026q1",
//!       "ed25519_public": "<base64-url-no-pad of 32 raw bytes>",
//!       "active_for_signing": false,
//!       "valid_from": "2026-01-01T00:00:00Z",
//!       "valid_until": "2026-12-31T23:59:59Z",
//!       "comment": "..."
//!     }
//!   ]
//! }
//! ```
//!
//! # Validation
//!
//! At load time the runtime verifies :
//!
//!   * Schema version equals [`SUPPORTED_TRUST_STORE_VERSION`] ;
//!   * Each `ed25519_public` is exactly 32 bytes after base64
//!     decoding (Ed25519 public keys are 32 bytes per RFC 8032) ;
//!   * `valid_from <= valid_until` ;
//!   * `key_id` is unique within the file ;
//!   * At least one key is present.

use std::collections::HashSet;
use std::path::Path;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hard-coded trust-store schema version this binary supports.
pub const SUPPORTED_TRUST_STORE_VERSION: &str = "1.0";

// =====================================================================
// Schema types
// =====================================================================

/// Top-level trust store JSON document.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustStoreDocument {
    /// Schema version (currently only `"1.0"`).
    pub schema_version: String,
    /// List of Ed25519 verifying keys.
    pub keys: Vec<TrustedKeyDoc>,
}

/// One trusted key as serialised in the JSON file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustedKeyDoc {
    /// Operator-chosen identifier (e.g. `efe-prod-bundle-2026q1`).
    /// Used as the `key_id` Prometheus label on the
    /// `efe_bundle_signature_verifications_total` counter.
    pub key_id: String,
    /// Base64url-no-pad 32-byte Ed25519 public key.
    pub ed25519_public: String,
    /// Whether this key is the one currently used by the CI to
    /// sign new bundles. Informational only — the verifier tries
    /// every key. Useful for `efe-bundle inspect` and dashboards.
    #[serde(default)]
    pub active_for_signing: bool,
    /// Earliest UTC instant at which this key is considered valid.
    pub valid_from: DateTime<Utc>,
    /// Latest UTC instant at which this key is considered valid.
    /// Bundles signed *after* `valid_until` are rejected even if
    /// the signature crypto is otherwise correct.
    pub valid_until: DateTime<Utc>,
    /// Free-form note (audit trail).
    #[serde(default)]
    pub comment: Option<String>,
}

/// Runtime form — the public-key bytes are decoded once into a 32-byte
/// array, so the verify path is O(1) per key with no allocation.
#[derive(Clone, Debug)]
pub struct TrustedKey {
    /// Operator-chosen id for this key.
    pub key_id: String,
    /// Decoded 32-byte Ed25519 public key.
    pub public_key: [u8; 32],
    /// Whether this is the currently-active signing key (informational).
    pub active_for_signing: bool,
    /// Earliest valid moment (UTC).
    pub valid_from: DateTime<Utc>,
    /// Latest valid moment (UTC).
    pub valid_until: DateTime<Utc>,
    /// Free-form comment.
    pub comment: Option<String>,
}

/// Trust store loaded into memory, ready for fast verify.
#[derive(Clone, Debug)]
pub struct TrustStore {
    /// All loaded keys, in insertion order. The verify path tries
    /// each in turn ; ordering is operator-controlled.
    pub keys: Vec<TrustedKey>,
}

impl TrustStore {
    /// True iff the store has at least one key.
    #[must_use]
    pub fn is_non_empty(&self) -> bool {
        !self.keys.is_empty()
    }

    /// Number of loaded keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// True iff there are no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Iterator over keys whose validity window covers `now`.
    pub fn currently_valid<'a>(
        &'a self,
        now: DateTime<Utc>,
    ) -> impl Iterator<Item = &'a TrustedKey> {
        self.keys
            .iter()
            .filter(move |k| k.valid_from <= now && now <= k.valid_until)
    }
}

// =====================================================================
// Errors
// =====================================================================

/// Failure modes of the trust-store loader.
#[derive(Debug, Error)]
pub enum TrustStoreLoadError {
    /// File IO failure (permission, missing path, etc.).
    #[error("trust store IO error at {path:?}: {source}")]
    Io {
        /// Path that was being read.
        path: std::path::PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Body wasn't valid JSON / had unknown field.
    #[error("trust store at {path:?} is not valid JSON: {source}")]
    Json {
        /// Path that was being read.
        path: std::path::PathBuf,
        /// Underlying serde error.
        #[source]
        source: serde_json::Error,
    },

    /// Schema version isn't [`SUPPORTED_TRUST_STORE_VERSION`].
    #[error("trust store schema_version {got:?} unsupported (this binary supports {expected:?})")]
    UnsupportedSchemaVersion {
        /// Actual value.
        got: String,
        /// Value the binary supports.
        expected: &'static str,
    },

    /// `keys[]` was empty — refuse, otherwise the operator's deploy
    /// would silently accept *no* signatures (since the verifier
    /// tries every key in the loop and fails when the loop finishes
    /// empty-handed).
    #[error("trust store at {0:?} has no keys")]
    Empty(std::path::PathBuf),

    /// Two keys share the same `key_id` — operator typo / merge
    /// conflict.
    #[error("duplicate key_id {0:?} in trust store")]
    DuplicateKeyId(String),

    /// The base64-decoded public key isn't 32 bytes (Ed25519 spec
    /// — 32 bytes for the key, 64 for the signature).
    #[error("key {key_id:?}: public key wrong size (expected 32 bytes, got {got})")]
    WrongPublicKeyLength {
        /// Key id at fault.
        key_id: String,
        /// Decoded length.
        got: usize,
    },

    /// `ed25519_public` isn't valid base64url.
    #[error("key {key_id:?}: public key isn't valid base64url: {source}")]
    BadBase64 {
        /// Key id at fault.
        key_id: String,
        /// Underlying decode error.
        #[source]
        source: base64::DecodeError,
    },

    /// `valid_from > valid_until` — operator typo.
    #[error("key {key_id:?}: valid_from ({from:?}) is later than valid_until ({until:?})")]
    InvalidValidityWindow {
        /// Key id at fault.
        key_id: String,
        /// Earlier instant.
        from: DateTime<Utc>,
        /// Later instant.
        until: DateTime<Utc>,
    },
}

// =====================================================================
// Loader
// =====================================================================

/// Load + validate a trust store from disk.
///
/// Errors covered :
///
///   * filesystem permission / not-found ;
///   * malformed JSON / unknown field ;
///   * unsupported schema version ;
///   * 0 keys (architecture rule §5.2 : refuse) ;
///   * duplicate key_id ;
///   * 32-byte rule on the decoded Ed25519 public key ;
///   * `valid_from > valid_until`.
pub fn load_trust_store_file(path: &Path) -> Result<TrustStore, TrustStoreLoadError> {
    let raw = std::fs::read_to_string(path).map_err(|e| TrustStoreLoadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let doc: TrustStoreDocument =
        serde_json::from_str(&raw).map_err(|e| TrustStoreLoadError::Json {
            path: path.to_path_buf(),
            source: e,
        })?;
    if doc.schema_version != SUPPORTED_TRUST_STORE_VERSION {
        return Err(TrustStoreLoadError::UnsupportedSchemaVersion {
            got: doc.schema_version,
            expected: SUPPORTED_TRUST_STORE_VERSION,
        });
    }
    if doc.keys.is_empty() {
        return Err(TrustStoreLoadError::Empty(path.to_path_buf()));
    }
    let mut keys: Vec<TrustedKey> = Vec::with_capacity(doc.keys.len());
    let mut seen: HashSet<String> = HashSet::with_capacity(doc.keys.len());
    for k in doc.keys {
        if !seen.insert(k.key_id.clone()) {
            return Err(TrustStoreLoadError::DuplicateKeyId(k.key_id));
        }
        if k.valid_from > k.valid_until {
            return Err(TrustStoreLoadError::InvalidValidityWindow {
                key_id: k.key_id,
                from: k.valid_from,
                until: k.valid_until,
            });
        }
        let pk_bytes = URL_SAFE_NO_PAD
            .decode(k.ed25519_public.as_bytes())
            .map_err(|e| TrustStoreLoadError::BadBase64 {
                key_id: k.key_id.clone(),
                source: e,
            })?;
        if pk_bytes.len() != 32 {
            return Err(TrustStoreLoadError::WrongPublicKeyLength {
                key_id: k.key_id,
                got: pk_bytes.len(),
            });
        }
        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(&pk_bytes);
        keys.push(TrustedKey {
            key_id: k.key_id,
            public_key,
            active_for_signing: k.active_for_signing,
            valid_from: k.valid_from,
            valid_until: k.valid_until,
            comment: k.comment,
        });
    }
    Ok(TrustStore { keys })
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::io::Write;

    fn write_tmp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    fn good_doc() -> &'static str {
        r#"{
  "schema_version": "1.0",
  "keys": [
    {
      "key_id": "efe-prod-bundle-2026q1",
      "ed25519_public": "rZQzS5L3TpYzMbU6lTQzMbU6lTQzMbU6lTQzMbU6lTQ",
      "active_for_signing": true,
      "valid_from": "2026-01-01T00:00:00Z",
      "valid_until": "2026-12-31T23:59:59Z",
      "comment": "production key"
    }
  ]
}"#
    }

    #[test]
    fn load_canonical_trust_store() {
        let f = write_tmp(good_doc());
        let store = load_trust_store_file(f.path()).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.keys[0].key_id, "efe-prod-bundle-2026q1");
        assert!(store.keys[0].active_for_signing);
    }

    #[test]
    fn rejects_missing_file() {
        let err = load_trust_store_file(Path::new("/nonexistent/x.json")).unwrap_err();
        assert!(matches!(err, TrustStoreLoadError::Io { .. }));
    }

    #[test]
    fn rejects_invalid_json() {
        let f = write_tmp("not json");
        let err = load_trust_store_file(f.path()).unwrap_err();
        assert!(matches!(err, TrustStoreLoadError::Json { .. }));
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let f = write_tmp(r#"{"schema_version":"2.0","keys":[]}"#);
        let err = load_trust_store_file(f.path()).unwrap_err();
        assert!(matches!(
            err,
            TrustStoreLoadError::UnsupportedSchemaVersion { .. }
        ));
    }

    #[test]
    fn rejects_empty_keys() {
        let f = write_tmp(r#"{"schema_version":"1.0","keys":[]}"#);
        let err = load_trust_store_file(f.path()).unwrap_err();
        assert!(matches!(err, TrustStoreLoadError::Empty(_)));
    }

    #[test]
    fn rejects_duplicate_key_id() {
        let json = r#"{
  "schema_version": "1.0",
  "keys": [
    {
      "key_id": "k1",
      "ed25519_public": "rZQzS5L3TpYzMbU6lTQzMbU6lTQzMbU6lTQzMbU6lTQ",
      "valid_from": "2026-01-01T00:00:00Z",
      "valid_until": "2026-12-31T23:59:59Z"
    },
    {
      "key_id": "k1",
      "ed25519_public": "rZQzS5L3TpYzMbU6lTQzMbU6lTQzMbU6lTQzMbU6lTQ",
      "valid_from": "2026-01-01T00:00:00Z",
      "valid_until": "2026-12-31T23:59:59Z"
    }
  ]
}"#;
        let f = write_tmp(json);
        let err = load_trust_store_file(f.path()).unwrap_err();
        assert!(matches!(err, TrustStoreLoadError::DuplicateKeyId(_)));
    }

    #[test]
    fn rejects_wrong_pubkey_length() {
        let json = r#"{
  "schema_version": "1.0",
  "keys": [
    {
      "key_id": "k1",
      "ed25519_public": "AA",
      "valid_from": "2026-01-01T00:00:00Z",
      "valid_until": "2026-12-31T23:59:59Z"
    }
  ]
}"#;
        let f = write_tmp(json);
        let err = load_trust_store_file(f.path()).unwrap_err();
        assert!(matches!(
            err,
            TrustStoreLoadError::WrongPublicKeyLength { got: 1, .. }
        ));
    }

    #[test]
    fn rejects_bad_base64() {
        let json = r#"{
  "schema_version": "1.0",
  "keys": [
    {
      "key_id": "k1",
      "ed25519_public": "!!!not-base64!!!",
      "valid_from": "2026-01-01T00:00:00Z",
      "valid_until": "2026-12-31T23:59:59Z"
    }
  ]
}"#;
        let f = write_tmp(json);
        let err = load_trust_store_file(f.path()).unwrap_err();
        assert!(matches!(err, TrustStoreLoadError::BadBase64 { .. }));
    }

    #[test]
    fn rejects_inverted_validity_window() {
        let json = r#"{
  "schema_version": "1.0",
  "keys": [
    {
      "key_id": "k1",
      "ed25519_public": "rZQzS5L3TpYzMbU6lTQzMbU6lTQzMbU6lTQzMbU6lTQ",
      "valid_from": "2026-12-31T23:59:59Z",
      "valid_until": "2026-01-01T00:00:00Z"
    }
  ]
}"#;
        let f = write_tmp(json);
        let err = load_trust_store_file(f.path()).unwrap_err();
        assert!(matches!(
            err,
            TrustStoreLoadError::InvalidValidityWindow { .. }
        ));
    }

    #[test]
    fn currently_valid_filters_by_window() {
        let f = write_tmp(good_doc());
        let store = load_trust_store_file(f.path()).unwrap();
        let mid = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let before = Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2027, 6, 1, 0, 0, 0).unwrap();
        assert_eq!(store.currently_valid(mid).count(), 1);
        assert_eq!(store.currently_valid(before).count(), 0);
        assert_eq!(store.currently_valid(after).count(), 0);
    }
}
