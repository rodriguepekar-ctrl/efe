// SPDX-License-Identifier: UNLICENSED
//! M30 — Per-tenant modality policy.
//!
//! Each tenant carries a row in `tenant_modality_policy` that controls:
//!
//! - `vision_enabled` (bool)
//! - `audio_enabled` (bool)
//! - `long_text_enabled` (bool)
//! - `max_vision_bytes`, `max_audio_bytes`, `max_long_text_bytes`
//! - `max_audio_ms`
//! - `allowed_vision_mimes`, `allowed_audio_mimes`
//! - `dpia_signed_at` — non-null required for `vision` or `audio`
//!   per AI Act Art. 9 / GDPR Art. 35.
//!
//! `ModalityPolicy::guard_for` materializes a `GuardPolicy` for a
//! given modality, applying the per-tenant cap on top of the system
//! defaults. The DPIA check is enforced first : if a tenant tries to
//! call `/v1/multimodal/vision/encode` without a signed DPIA, the
//! request is rejected with `ModalityDisabled` before any byte is
//! inspected.

use crate::error::{MultimodalError, Result};
use crate::input_guard::{GuardModality, GuardPolicy};

/// In-memory representation of a `tenant_modality_policy` row.
#[derive(Debug, Clone)]
pub struct TenantModalityPolicy {
    /// Tenant ULID (slug).
    pub tenant_id: String,
    /// Whether vision endpoints are enabled.
    pub vision_enabled: bool,
    /// Whether audio endpoints are enabled.
    pub audio_enabled: bool,
    /// Whether long-text endpoints are enabled.
    pub long_text_enabled: bool,
    /// Max vision payload in bytes.
    pub max_vision_bytes: u64,
    /// Max audio payload in bytes.
    pub max_audio_bytes: u64,
    /// Max long-text payload in bytes.
    pub max_long_text_bytes: u64,
    /// Max audio duration in milliseconds.
    pub max_audio_ms: u64,
    /// Allowed vision MIME types.
    pub allowed_vision_mimes: Vec<&'static str>,
    /// Allowed audio MIME types.
    pub allowed_audio_mimes: Vec<&'static str>,
    /// DPIA signature timestamp. `None` ⇒ DPIA not signed ⇒
    /// `vision` / `audio` are forbidden regardless of the
    /// `*_enabled` booleans.
    pub dpia_signed_at: Option<chrono_like_iso8601::Iso8601>,
    /// Operator / DPO identifier who signed the DPIA. Mirror of the
    /// `dpia_signed_by` column in `tenant_modality_policy`. Used for
    /// audit-trail attribution. `None` when DPIA is unsigned.
    pub dpia_signed_by: Option<String>,
}

impl Default for TenantModalityPolicy {
    fn default() -> Self {
        Self {
            tenant_id: "default".into(),
            vision_enabled: false,
            audio_enabled: false,
            long_text_enabled: true,
            max_vision_bytes: 10 * 1024 * 1024,
            max_audio_bytes: 25 * 1024 * 1024,
            max_long_text_bytes: 1024 * 1024,
            max_audio_ms: 60_000,
            allowed_vision_mimes: vec!["image/jpeg", "image/png", "image/webp"],
            allowed_audio_mimes: vec!["audio/wav", "audio/mpeg", "audio/flac"],
            dpia_signed_at: None,
            dpia_signed_by: None,
        }
    }
}

impl TenantModalityPolicy {
    /// Returns `Ok(GuardPolicy)` for the requested modality, or
    /// `Err(ModalityDisabled)` if the tenant cannot use it.
    pub fn guard_for(&self, modality: GuardModality) -> Result<GuardPolicy> {
        match modality {
            GuardModality::Vision => {
                self.check_dpia()?;
                if !self.vision_enabled {
                    return Err(MultimodalError::ModalityDisabled { modality: "vision" });
                }
                Ok(GuardPolicy {
                    max_bytes: self.max_vision_bytes,
                    max_audio_ms: 0,
                    allowed_mimes: self.allowed_vision_mimes.clone(),
                })
            }
            GuardModality::Audio => {
                self.check_dpia()?;
                if !self.audio_enabled {
                    return Err(MultimodalError::ModalityDisabled { modality: "audio" });
                }
                Ok(GuardPolicy {
                    max_bytes: self.max_audio_bytes,
                    max_audio_ms: self.max_audio_ms,
                    allowed_mimes: self.allowed_audio_mimes.clone(),
                })
            }
            GuardModality::LongText => {
                if !self.long_text_enabled {
                    return Err(MultimodalError::ModalityDisabled {
                        modality: "long_text",
                    });
                }
                Ok(GuardPolicy {
                    max_bytes: self.max_long_text_bytes,
                    max_audio_ms: 0,
                    allowed_mimes: vec!["text/plain", "text/markdown"],
                })
            }
        }
    }

    fn check_dpia(&self) -> Result<()> {
        if self.dpia_signed_at.is_none() {
            return Err(MultimodalError::ModalityDisabled {
                modality: "dpia_not_signed",
            });
        }
        Ok(())
    }
}

/// Minimal ISO-8601 wrapper to avoid pulling `chrono` into this crate.
pub mod chrono_like_iso8601 {
    use serde::{Deserialize, Serialize};

    /// Opaque ISO-8601 timestamp (UTC). Validated at the SQL boundary.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct Iso8601(pub String);

    impl Iso8601 {
        /// Construct from a string. The string is **not** validated
        /// here ; this is a thin newtype to keep the SQL boundary
        /// honest.
        pub fn from_string(s: impl Into<String>) -> Self {
            Self(s.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_policy() -> TenantModalityPolicy {
        TenantModalityPolicy {
            vision_enabled: true,
            audio_enabled: true,
            dpia_signed_at: Some(chrono_like_iso8601::Iso8601::from_string(
                "2026-01-01T00:00:00Z",
            )),
            dpia_signed_by: Some("dpo@efe.ai".into()),
            ..Default::default()
        }
    }

    #[test]
    fn default_policy_blocks_vision_and_audio() {
        let p = TenantModalityPolicy::default();
        assert!(matches!(
            p.guard_for(GuardModality::Vision).unwrap_err(),
            MultimodalError::ModalityDisabled { .. }
        ));
        assert!(matches!(
            p.guard_for(GuardModality::Audio).unwrap_err(),
            MultimodalError::ModalityDisabled { .. }
        ));
        // Long text is allowed by default.
        p.guard_for(GuardModality::LongText).unwrap();
    }

    #[test]
    fn dpia_unsigned_blocks_vision_even_if_enabled() {
        let mut p = TenantModalityPolicy::default();
        p.vision_enabled = true;
        assert!(matches!(
            p.guard_for(GuardModality::Vision).unwrap_err(),
            MultimodalError::ModalityDisabled { modality: "dpia_not_signed" }
        ));
    }

    #[test]
    fn dpia_signed_unblocks_vision_when_enabled() {
        let p = signed_policy();
        let g = p.guard_for(GuardModality::Vision).unwrap();
        assert!(g.allowed_mimes.contains(&"image/jpeg"));
    }

    #[test]
    fn long_text_can_be_disabled() {
        let mut p = TenantModalityPolicy::default();
        p.long_text_enabled = false;
        assert!(matches!(
            p.guard_for(GuardModality::LongText).unwrap_err(),
            MultimodalError::ModalityDisabled { .. }
        ));
    }
}
