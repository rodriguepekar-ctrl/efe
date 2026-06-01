// SPDX-License-Identifier: UNLICENSED
//! M30 — Input guard for multimodal payloads.
//!
//! Every multimodal request flows through `InputGuard::inspect` before
//! a single byte of preprocessing happens. The guard enforces three
//! orthogonal invariants :
//!
//! 1. **Magic byte sniffing** — refuse any payload whose first bytes
//!    do not match the per-modality allow-list. This is a defense in
//!    depth on top of `Content-Type` since the latter can be spoofed.
//! 2. **Size cap** — reject inputs larger than the per-tenant
//!    `max_bytes` (default : 10 MB vision, 25 MB audio, 1 MB long text).
//! 3. **Duration estimate** — for audio (WAV, MP3), compute a
//!    conservative duration estimate from the header and reject if it
//!    exceeds the per-tenant `max_audio_ms` cap (default : 60 s).
//!
//! The guard also emits a *SHA-256 input hash* (first 16 hex chars
//! prefix, never the raw bytes) for audit-chain attribution. Per the
//! M29 PII redactor contract, raw input bytes never leave this crate.

use sha2::{Digest, Sha256};

use crate::error::{MultimodalError, Result};

/// Modality discriminator used by the input guard. Distinct from the
/// runtime `Modality` enum because the guard treats long-text payloads
/// as plain UTF-8 strings (no magic sniffing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardModality {
    /// Still image bytes (JPEG, PNG, WebP).
    Vision,
    /// Audio bytes (WAV, MP3, FLAC).
    Audio,
    /// UTF-8 text (no magic sniffing, only size cap).
    LongText,
}

impl GuardModality {
    /// Static name of the modality (used as a metrics label).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Vision => "vision",
            Self::Audio => "audio",
            Self::LongText => "long_text",
        }
    }
}

/// Per-modality guard policy. Loaded from `tenant_modality_policy` rows
/// at request time and cached for `current_request_lifetime`.
#[derive(Debug, Clone)]
pub struct GuardPolicy {
    /// Maximum payload size in bytes.
    pub max_bytes: u64,
    /// Maximum audio duration in milliseconds (audio-only).
    pub max_audio_ms: u64,
    /// Allowed MIME types (vision / audio).
    pub allowed_mimes: Vec<&'static str>,
}

impl GuardPolicy {
    /// Conservative default policy for vision : 10 MB, JPEG / PNG /
    /// WebP only. Used when the tenant has no override row.
    #[must_use]
    pub fn default_vision() -> Self {
        Self {
            max_bytes: 10 * 1024 * 1024,
            max_audio_ms: 0,
            allowed_mimes: vec!["image/jpeg", "image/png", "image/webp"],
        }
    }

    /// Conservative default policy for audio : 25 MB, 60 s, WAV /
    /// MP3 / FLAC only.
    #[must_use]
    pub fn default_audio() -> Self {
        Self {
            max_bytes: 25 * 1024 * 1024,
            max_audio_ms: 60_000,
            allowed_mimes: vec!["audio/wav", "audio/mpeg", "audio/flac"],
        }
    }

    /// Conservative default policy for long text : 1 MB, no MIME check.
    #[must_use]
    pub fn default_long_text() -> Self {
        Self {
            max_bytes: 1024 * 1024,
            max_audio_ms: 0,
            allowed_mimes: vec!["text/plain", "text/markdown"],
        }
    }
}

/// Output of the input guard. Carries the detected MIME, the SHA-256
/// prefix (audit-safe), and a duration estimate (audio only).
#[derive(Debug, Clone)]
pub struct InputMetadata {
    /// Detected MIME type (from magic bytes).
    pub detected_mime: &'static str,
    /// SHA-256 hash of the input, hex-encoded (full 64 chars). Use
    /// `input_hash_prefix(&meta.sha256_hex, 16)` for log lines.
    pub sha256_hex: String,
    /// Estimated duration in milliseconds (audio only ; 0 for vision /
    /// long text).
    pub duration_ms_estimate: u64,
    /// Size in bytes of the inspected payload (echo of `bytes.len()`).
    pub size_bytes: u64,
}

/// Input guard façade.
pub struct InputGuard;

impl InputGuard {
    /// Inspect a payload against a `GuardPolicy`. Returns
    /// `InputMetadata` on success ; returns a typed `MultimodalError`
    /// otherwise.
    ///
    /// `bytes` is treated as opaque (no decoding). Magic sniffing reads
    /// at most the first 16 bytes ; size + hash require a single
    /// linear pass.
    pub fn inspect(
        modality: GuardModality,
        bytes: &[u8],
        policy: &GuardPolicy,
    ) -> Result<InputMetadata> {
        // 1) Size cap (cheap : O(1)).
        let size_bytes = bytes.len() as u64;
        if size_bytes > policy.max_bytes {
            return Err(MultimodalError::InputTooLarge {
                observed_bytes: size_bytes,
                cap_bytes: policy.max_bytes,
            });
        }

        // 2) Magic sniffing.
        let detected_mime = match modality {
            GuardModality::Vision => sniff_vision_mime(bytes)?,
            GuardModality::Audio => sniff_audio_mime(bytes)?,
            GuardModality::LongText => {
                // For long text we accept any valid UTF-8 ; assert it.
                if std::str::from_utf8(bytes).is_err() {
                    return Err(MultimodalError::InvalidFormat {
                        detected_mime: "binary".into(),
                        allowed_mimes: policy.allowed_mimes.clone(),
                    });
                }
                "text/plain"
            }
        };
        if !policy.allowed_mimes.contains(&detected_mime) {
            return Err(MultimodalError::InvalidFormat {
                detected_mime: detected_mime.to_string(),
                allowed_mimes: policy.allowed_mimes.clone(),
            });
        }

        // 3) Duration estimate (audio only).
        let duration_ms_estimate = match modality {
            GuardModality::Audio => estimate_audio_duration_ms(detected_mime, bytes),
            _ => 0,
        };
        if matches!(modality, GuardModality::Audio) && duration_ms_estimate > policy.max_audio_ms {
            return Err(MultimodalError::InvalidDuration {
                observed_ms: duration_ms_estimate,
                cap_ms: policy.max_audio_ms,
            });
        }

        // 4) SHA-256 hash (audit-chain).
        let sha256_hex = sha256_hex(bytes);

        Ok(InputMetadata {
            detected_mime,
            sha256_hex,
            duration_ms_estimate,
            size_bytes,
        })
    }
}

// ────────────────── magic-byte sniffers ──────────────────

fn sniff_vision_mime(bytes: &[u8]) -> Result<&'static str> {
    if bytes.len() < 4 {
        return Err(MultimodalError::InvalidFormat {
            detected_mime: "too_short".into(),
            allowed_mimes: vec!["image/jpeg", "image/png", "image/webp"],
        });
    }
    // JPEG : FF D8 FF
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("image/jpeg");
    }
    // PNG : 89 50 4E 47 0D 0A 1A 0A
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Ok("image/png");
    }
    // WebP : starts with "RIFF" .... "WEBP"
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok("image/webp");
    }
    Err(MultimodalError::InvalidFormat {
        detected_mime: format!(
            "unknown_vision_{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        ),
        allowed_mimes: vec!["image/jpeg", "image/png", "image/webp"],
    })
}

/// Minimum byte count for a syntactically well-formed WAV : 12 bytes
/// for RIFF/WAVE + 24 bytes for the `fmt ` chunk + 8 bytes for the
/// `data` chunk header = 44 bytes. Anything below this is a malformed
/// or adversarial header and is rejected by the guard.
pub const WAV_MIN_HEADER_BYTES: usize = 44;

fn sniff_audio_mime(bytes: &[u8]) -> Result<&'static str> {
    if bytes.len() < 4 {
        return Err(MultimodalError::InvalidFormat {
            detected_mime: "too_short".into(),
            allowed_mimes: vec!["audio/wav", "audio/mpeg", "audio/flac"],
        });
    }
    // WAV : "RIFF" .... "WAVE" — but also require ≥ 44 bytes so the
    // duration estimator has a complete header to read (M30 audit
    // fix: previously a 12-byte "RIFFxxxxWAVE" file passed the magic
    // sniff and reached Triton with duration_ms_estimate = 0).
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        if bytes.len() < WAV_MIN_HEADER_BYTES {
            return Err(MultimodalError::InvalidFormat {
                detected_mime: format!("audio/wav-truncated-{}b", bytes.len()),
                allowed_mimes: vec!["audio/wav", "audio/mpeg", "audio/flac"],
            });
        }
        return Ok("audio/wav");
    }
    // FLAC : "fLaC"
    if bytes.starts_with(b"fLaC") {
        return Ok("audio/flac");
    }
    // MP3 : ID3 tag or sync word FF FB / FF F3 / FF F2.
    if bytes.starts_with(b"ID3") {
        return Ok("audio/mpeg");
    }
    if bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0 {
        return Ok("audio/mpeg");
    }
    Err(MultimodalError::InvalidFormat {
        detected_mime: format!(
            "unknown_audio_{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3]
        ),
        allowed_mimes: vec!["audio/wav", "audio/mpeg", "audio/flac"],
    })
}

/// Conservative duration estimator. For WAV we read the data chunk
/// size and divide by the byte rate. For MP3 / FLAC we use a
/// loose upper bound (bytes / 8 kB-per-sec = pessimistic) so that the
/// guard rejects obviously-too-long inputs but lets borderline cases
/// reach the real decoder.
fn estimate_audio_duration_ms(mime: &'static str, bytes: &[u8]) -> u64 {
    match mime {
        "audio/wav" => estimate_wav_duration_ms(bytes),
        // For lossy / VBR formats we use a defensive byte-rate.
        _ => (bytes.len() as u64).saturating_mul(1000) / 8_000,
    }
}

fn estimate_wav_duration_ms(bytes: &[u8]) -> u64 {
    // The fmt chunk byte rate sits at offset 28 (little endian u32).
    // Reference : https://docs.fileformat.com/audio/wav/
    if bytes.len() < 44 {
        return 0;
    }
    let byte_rate = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    if byte_rate == 0 {
        return 0;
    }
    // The data chunk starts at offset 36 (44 bytes of header).
    let data_bytes = bytes.len() as u64 - 44;
    (data_bytes * 1000) / (byte_rate as u64)
}

/// Compute the SHA-256 hex string of `bytes`.
///
/// Made public so that REST handlers can compute the audit hash once
/// and pass it through to the downstream agent runner without
/// re-hashing.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    hex::encode(digest)
}

/// Return the first `n` hex characters of an already-computed SHA-256
/// digest, suitable for inclusion in log lines (audit-safe).
#[must_use]
pub fn input_hash_prefix(sha256_hex: &str, n: usize) -> &str {
    let bound = sha256_hex.len().min(n);
    &sha256_hex[..bound]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal valid headers for each format.
    fn jpeg_bytes() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.extend_from_slice(&[0u8; 1024]);
        v
    }
    fn png_bytes() -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(&[0u8; 1024]);
        v
    }
    fn webp_bytes() -> Vec<u8> {
        let mut v: Vec<u8> = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&[0x00, 0x10, 0x00, 0x00]);
        v.extend_from_slice(b"WEBP");
        v.extend_from_slice(&[0u8; 1024]);
        v
    }
    fn wav_bytes(byte_rate: u32, data_bytes: usize) -> Vec<u8> {
        let mut v: Vec<u8> = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((36 + data_bytes) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&16_000u32.to_le_bytes()); // sample rate
        v.extend_from_slice(&byte_rate.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data_bytes as u32).to_le_bytes());
        v.extend(vec![0u8; data_bytes]);
        v
    }

    #[test]
    fn vision_accepts_jpeg_png_webp() {
        let policy = GuardPolicy::default_vision();
        for bytes in [jpeg_bytes(), png_bytes(), webp_bytes()] {
            let meta = InputGuard::inspect(GuardModality::Vision, &bytes, &policy).unwrap();
            assert!(meta.detected_mime.starts_with("image/"));
            assert_eq!(meta.sha256_hex.len(), 64);
            assert_eq!(meta.duration_ms_estimate, 0);
        }
    }

    #[test]
    fn vision_rejects_random_bytes() {
        let policy = GuardPolicy::default_vision();
        let bytes = vec![0u8; 1024];
        let err = InputGuard::inspect(GuardModality::Vision, &bytes, &policy).unwrap_err();
        assert!(matches!(err, MultimodalError::InvalidFormat { .. }));
        assert_eq!(err.error_code(), "multimodal.invalid_format");
    }

    #[test]
    fn vision_rejects_oversized() {
        let mut policy = GuardPolicy::default_vision();
        policy.max_bytes = 100;
        let bytes = jpeg_bytes(); // > 100
        let err = InputGuard::inspect(GuardModality::Vision, &bytes, &policy).unwrap_err();
        assert!(matches!(err, MultimodalError::InputTooLarge { .. }));
    }

    #[test]
    fn audio_wav_accepts_valid_header_and_estimates_duration() {
        let policy = GuardPolicy::default_audio();
        // 32000 byte_rate × 2 seconds → 64000 bytes of audio data.
        let bytes = wav_bytes(32_000, 64_000);
        let meta = InputGuard::inspect(GuardModality::Audio, &bytes, &policy).unwrap();
        assert_eq!(meta.detected_mime, "audio/wav");
        assert!((1_900..=2_100).contains(&meta.duration_ms_estimate));
    }

    #[test]
    fn audio_rejects_too_long_audio() {
        let mut policy = GuardPolicy::default_audio();
        policy.max_audio_ms = 1_000;
        let bytes = wav_bytes(32_000, 64_000); // ~ 2 s
        let err = InputGuard::inspect(GuardModality::Audio, &bytes, &policy).unwrap_err();
        assert!(matches!(err, MultimodalError::InvalidDuration { .. }));
    }

    #[test]
    fn audio_rejects_truncated_wav_header() {
        // A bare 12-byte "RIFFxxxxWAVE" file would have previously
        // slipped past the sniffer with duration 0. Now it must be
        // rejected as InvalidFormat.
        let policy = GuardPolicy::default_audio();
        let bytes: Vec<u8> = b"RIFF\x00\x00\x00\x00WAVE".to_vec();
        let err = InputGuard::inspect(GuardModality::Audio, &bytes, &policy).unwrap_err();
        match err {
            MultimodalError::InvalidFormat { ref detected_mime, .. } => {
                assert!(detected_mime.starts_with("audio/wav-truncated"));
            }
            other => panic!("expected InvalidFormat truncated, got {:?}", other),
        }
    }

    #[test]
    fn long_text_requires_valid_utf8() {
        let policy = GuardPolicy::default_long_text();
        let bytes = b"hello world".to_vec();
        let meta = InputGuard::inspect(GuardModality::LongText, &bytes, &policy).unwrap();
        assert_eq!(meta.detected_mime, "text/plain");
        // Invalid UTF-8.
        let bytes = vec![0xFF, 0xFE, 0xFD];
        let err = InputGuard::inspect(GuardModality::LongText, &bytes, &policy).unwrap_err();
        assert!(matches!(err, MultimodalError::InvalidFormat { .. }));
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        let h1 = sha256_hex(b"abc");
        let h2 = sha256_hex(b"abc");
        assert_eq!(h1, h2);
        // Sanity : different inputs ⇒ different hashes.
        let h3 = sha256_hex(b"abd");
        assert_ne!(h1, h3);
    }

    #[test]
    fn input_hash_prefix_is_bounded() {
        let h = sha256_hex(b"abc");
        assert_eq!(input_hash_prefix(&h, 16).len(), 16);
        // Boundary : asking for more than 64 hex chars returns ≤ 64.
        assert!(input_hash_prefix(&h, 999).len() <= 64);
    }
}
