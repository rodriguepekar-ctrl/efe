//! Sprint **D14-bis** — HMAC key loader for the audit chain.
//!
//! Mirror of the M12 HS256 secrets pattern (cf.
//! `crates/efe-server/src/config.rs::load_hs256_secrets`) :
//!
//! * One file, one key per line, base64-url-encoded.
//! * The **first** non-comment line is the active signing key ; the
//!   rest are kept for verification of historical events
//!   (rotation in flight).
//! * Lines starting with `#` are comments ; empty lines are ignored.
//! * Each key MUST be exactly **32 raw bytes** after decode (the
//!   HMAC-SHA-256 block size optimum). Shorter keys are rejected
//!   to avoid accidental low-entropy secrets ; longer keys are
//!   rejected to keep `key_id` semantics meaningful.
//!
//! # Architecture reference
//!
//! `docs/bench/d14bis_hmac_audit_chain_architecture.md` § 13.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

/// Cryptographically-strong HMAC key (32 raw bytes).
///
/// The key material is wrapped in this newtype so that :
///
/// * **`Debug` redaction** : printing an `HmacKey` shows only the
///   `key_id` ; the raw bytes never appear in logs / panic backtraces.
/// * **`expose()` is explicit** : reading the bytes requires calling
///   the dedicated method, so any leak is grep-able.
/// * **No `Clone`** : intentionally absent ; if you need a second
///   handle, share the `Arc<HashMap<…>>` from `LoadedKeys`.
///
/// We considered `secrecy::Secret<Box<[u8; 32]>>` but secrecy 0.8's
/// `Zeroize` requirement doesn't compose cleanly with `[u8; 32]`.
/// The custom newtype gives us the same audit-relevant properties
/// (no Debug leak, explicit expose) with zero external deps.
pub struct HmacKey {
    /// 8-character lowercase hex `key_id` (first 8 chars of
    /// SHA-256(material)). Stable across processes, derived
    /// deterministically from the key material.
    pub id: String,
    /// Raw key material, accessed only via [`Self::expose`].
    material: [u8; 32],
}

impl HmacKey {
    /// Construct from raw bytes. The `key_id` is computed from the
    /// material.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        let id = key_id_of(&bytes);
        Self {
            id,
            material: bytes,
        }
    }

    /// Expose the raw key bytes for HMAC computation. Intentionally
    /// the only path to read the material — keep call-sites
    /// minimal so `git grep "expose"` surfaces all uses.
    #[must_use]
    pub fn expose(&self) -> &[u8; 32] {
        &self.material
    }
}

impl std::fmt::Debug for HmacKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HmacKey")
            .field("id", &self.id)
            .field("material", &"<redacted>")
            .finish()
    }
}

impl Drop for HmacKey {
    fn drop(&mut self) {
        // Best-effort zero-on-drop. The compiler is allowed to elide
        // a "dead" zero, but with `volatile` semantics this becomes
        // a known-good pattern. Use `std::ptr::write_volatile` to
        // ensure the writes are observable.
        let p = self.material.as_mut_ptr();
        for i in 0..self.material.len() {
            // SAFETY: `material` is owned by self, p is in-bounds.
            unsafe { std::ptr::write_volatile(p.add(i), 0) };
        }
    }
}

/// Compute the deterministic 8-char hex `key_id` of an HMAC key.
///
/// First 8 chars of `SHA-256(key_material)`. Collision probability
/// ~2⁻³² per pair of distinct keys — negligible for the < 100 keys
/// that ever live in the rotation file.
#[must_use]
pub fn key_id_of(material: &[u8; 32]) -> String {
    let h = Sha256::digest(material);
    let mut out = String::with_capacity(8);
    for &b in &h[..4] {
        let hi = (b >> 4) & 0x0F;
        let lo = b & 0x0F;
        out.push(hex_nibble(hi));
        out.push(hex_nibble(lo));
    }
    out
}

#[inline]
fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => unreachable!(),
    }
}

/// Errors produced by [`load_keys_file`].
#[derive(Debug, Error)]
pub enum LoadKeysError {
    #[error("audit HMAC keys file IO error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("audit HMAC keys file at {path:?} contains no usable keys (empty or only comments)")]
    Empty { path: PathBuf },

    #[error("audit HMAC keys file at {path:?} line {lineno}: {detail}")]
    BadLine {
        path: PathBuf,
        lineno: usize,
        detail: String,
    },

    #[error("audit HMAC keys file at {path:?} contains duplicate key on lines {first}/{second}")]
    Duplicate {
        path: PathBuf,
        first: usize,
        second: usize,
    },
}

/// Result of loading the keys file.
///
/// `active` is the first non-comment line — the key used for **new**
/// signatures. `all` contains every key indexed by `key_id`,
/// including `active` ; the verify path looks up keys by the
/// `key_id` column stored in `audit_events` so historical events
/// signed by rotated-out keys still verify.
#[derive(Debug)]
pub struct LoadedKeys {
    pub active_id: String,
    pub all: HashMap<String, HmacKey>,
}

/// Parse the keys file into [`LoadedKeys`].
///
/// **Error message hygiene** : the offending base64 string is
/// **never** echoed in error messages — only the line number — so
/// logs do not leak even a malformed key. Same invariant as the
/// audit fix in M12 `load_hs256_secrets` (cf.
/// `test_load_hs256_secrets_rejects_non_base64_line_without_leaking_material`).
pub fn load_keys_file(path: &Path) -> Result<LoadedKeys, LoadKeysError> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    let raw = std::fs::read_to_string(path).map_err(|e| LoadKeysError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    let mut active_id: Option<String> = None;
    let mut all: HashMap<String, HmacKey> = HashMap::new();
    let mut id_to_lineno: HashMap<String, usize> = HashMap::new();

    for (idx, line) in raw.lines().enumerate() {
        let lineno = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let bytes =
            URL_SAFE_NO_PAD
                .decode(trimmed.as_bytes())
                .map_err(|e| LoadKeysError::BadLine {
                    path: path.to_path_buf(),
                    lineno,
                    // No echo of the line content — preserves the M12
                    // invariant that secret material doesn't reach logs.
                    detail: format!("not valid base64-url: {e}"),
                })?;

        if bytes.len() != 32 {
            return Err(LoadKeysError::BadLine {
                path: path.to_path_buf(),
                lineno,
                detail: format!(
                    "key must be exactly 32 raw bytes (256 bits) ; got {} bytes",
                    bytes.len()
                ),
            });
        }

        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        let id = key_id_of(&arr);

        if let Some(prev_lineno) = id_to_lineno.get(&id) {
            return Err(LoadKeysError::Duplicate {
                path: path.to_path_buf(),
                first: *prev_lineno,
                second: lineno,
            });
        }
        id_to_lineno.insert(id.clone(), lineno);

        if active_id.is_none() {
            active_id = Some(id.clone());
        }
        all.insert(id.clone(), HmacKey::from_bytes(arr));
    }

    let active_id = active_id.ok_or_else(|| LoadKeysError::Empty {
        path: path.to_path_buf(),
    })?;
    Ok(LoadedKeys { active_id, all })
}

// =====================================================================
// Tests — pure unit, no DB / no FS dependencies (we use tempfile).
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    /// Helper to encode a 32-byte key as base64-url-no-pad.
    fn b64(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn write_keys_file(content: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn key_id_is_deterministic_for_same_material() {
        let m = [0x42u8; 32];
        let a = key_id_of(&m);
        let b = key_id_of(&m);
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
        // hex lowercase
        assert!(a
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
    }

    #[test]
    fn key_id_differs_for_different_material() {
        let a = key_id_of(&[0x01; 32]);
        let b = key_id_of(&[0x02; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn loads_single_key_file() {
        let key = [0xab; 32];
        let f = write_keys_file(&format!("{}\n", b64(&key)));
        let loaded = load_keys_file(f.path()).unwrap();
        assert_eq!(loaded.all.len(), 1);
        assert_eq!(loaded.active_id, key_id_of(&key));
    }

    #[test]
    fn loads_two_keys_active_is_first() {
        let k1 = [0x01; 32];
        let k2 = [0x02; 32];
        let f = write_keys_file(&format!("{}\n{}\n", b64(&k1), b64(&k2)));
        let loaded = load_keys_file(f.path()).unwrap();
        assert_eq!(loaded.all.len(), 2);
        assert_eq!(loaded.active_id, key_id_of(&k1));
        // Both ids are present.
        assert!(loaded.all.contains_key(&key_id_of(&k1)));
        assert!(loaded.all.contains_key(&key_id_of(&k2)));
    }

    #[test]
    fn skips_comments_and_blanks() {
        let key = [0xcc; 32];
        let content = format!(
            "# active key below\n\n{}\n# trailing comment\n\n",
            b64(&key)
        );
        let f = write_keys_file(&content);
        let loaded = load_keys_file(f.path()).unwrap();
        assert_eq!(loaded.all.len(), 1);
    }

    #[test]
    fn rejects_empty_file() {
        let f = write_keys_file("# only a comment\n\n");
        let err = load_keys_file(f.path()).unwrap_err();
        assert!(matches!(err, LoadKeysError::Empty { .. }));
    }

    #[test]
    fn rejects_missing_file() {
        let err = load_keys_file(Path::new("/nonexistent/efe-audit-keys")).unwrap_err();
        assert!(matches!(err, LoadKeysError::Io { .. }));
    }

    #[test]
    fn rejects_bad_base64_without_leaking_material() {
        let f = write_keys_file("$$$invalid base64$$$\n");
        let err = load_keys_file(f.path()).unwrap_err();
        let msg = format!("{err}");
        assert!(matches!(err, LoadKeysError::BadLine { .. }));
        assert!(
            !msg.contains("$$$invalid"),
            "leaked material in error: {msg}"
        );
    }

    #[test]
    fn rejects_wrong_length_key() {
        // 31 bytes → bad
        let f = write_keys_file(&format!("{}\n", b64(&[0xee; 31])));
        let err = load_keys_file(f.path()).unwrap_err();
        match err {
            LoadKeysError::BadLine { detail, .. } => assert!(detail.contains("32 raw bytes")),
            other => panic!("expected BadLine, got {other:?}"),
        }
    }

    #[test]
    fn rejects_too_long_key() {
        // 64 bytes → bad
        let f = write_keys_file(&format!("{}\n", b64(&[0xff; 64])));
        let err = load_keys_file(f.path()).unwrap_err();
        assert!(matches!(err, LoadKeysError::BadLine { .. }));
    }

    #[test]
    fn rejects_duplicate_keys() {
        let key = [0x77; 32];
        let f = write_keys_file(&format!("{0}\n{0}\n", b64(&key)));
        let err = load_keys_file(f.path()).unwrap_err();
        match err {
            LoadKeysError::Duplicate { first, second, .. } => {
                assert_eq!(first, 1);
                assert_eq!(second, 2);
            }
            other => panic!("expected Duplicate, got {other:?}"),
        }
    }

    #[test]
    fn hmac_key_debug_redacts_material() {
        let k = HmacKey::from_bytes([0xde; 32]);
        let s = format!("{k:?}");
        assert!(s.contains("HmacKey"));
        assert!(
            s.contains("redacted"),
            "Debug must redact material; got: {s}"
        );
        // The key bytes must NEVER appear textually
        assert!(!s.contains("0xde"));
    }
}
