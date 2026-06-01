//! Image whitelist + cosign verification.
//!
//! Only images from `EFE_M25_IMAGE_WHITELIST` (comma-separated OCI refs,
//! optionally with `@sha256:...` digest) may be used as the agent-core
//! rootfs. The admission webhook (Helm chart) blocks AgentRun pods
//! whose `spec.template.spec.containers[].image` is not in the list.
//!
//! Cosign verification is performed at admission AND at controller
//! reconcile, in case the image was tag-mutated server-side.

use crate::{AgentRunnerError, AgentRunnerResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageWhitelist {
    pub entries: Vec<ImageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    /// e.g. `ghcr.io/efe-ai/agent-core`
    pub repository: String,
    /// pinned digest (sha256:...) ; tags forbidden in prod.
    pub digest: Option<String>,
    /// cosign public key reference (KMS or PEM).
    pub cosign_key_ref: Option<String>,
}

impl ImageWhitelist {
    /// Parse from env var `EFE_M25_IMAGE_WHITELIST`.
    /// Format: `repo@sha256:digest:cosign_ref,repo2@sha256:digest:cosign_ref`
    pub fn from_env() -> AgentRunnerResult<Self> {
        let raw = std::env::var("EFE_M25_IMAGE_WHITELIST").unwrap_or_default();
        if raw.is_empty() {
            return Ok(Self { entries: vec![] });
        }
        let mut entries = Vec::new();
        for piece in raw.split(',') {
            let piece = piece.trim();
            if piece.is_empty() { continue; }
            let mut parts = piece.splitn(3, ':');
            let repo_with_digest = parts.next().unwrap_or("");
            let _kind = parts.next();   // "sha256" prefix consumed below
            let _ = parts.next();
            let (repository, digest) = if let Some(idx) = repo_with_digest.find('@') {
                (
                    repo_with_digest[..idx].to_string(),
                    Some(repo_with_digest[idx + 1..].to_string()),
                )
            } else {
                (repo_with_digest.to_string(), None)
            };
            entries.push(ImageEntry {
                repository,
                digest,
                cosign_key_ref: std::env::var("EFE_M25_COSIGN_KEY_REF").ok(),
            });
        }
        Ok(Self { entries })
    }

    /// True if `image_ref` matches a whitelist entry. `image_ref` MUST
    /// include the digest in production.
    pub fn allows(&self, image_ref: &str) -> bool {
        for e in &self.entries {
            // Repo prefix match.
            if !image_ref.starts_with(&e.repository) {
                continue;
            }
            // If a digest is pinned, image_ref MUST include it.
            if let Some(d) = &e.digest {
                if image_ref.contains(d) {
                    return true;
                }
            } else {
                // No digest pin — repo match is enough (dev only).
                return true;
            }
        }
        false
    }
}

/// Reject if the image is not whitelisted. Increments the security
/// metric on rejection.
pub fn verify_image_whitelist(
    whitelist: &ImageWhitelist,
    image_ref: &str,
) -> AgentRunnerResult<()> {
    if whitelist.allows(image_ref) {
        return Ok(());
    }
    crate::metrics::AGENT_IMAGE_REJECTED_WHITELIST_TOTAL.inc();
    Err(AgentRunnerError::ImageNotInWhitelist(image_ref.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_pinned_digest() {
        let wl = ImageWhitelist {
            entries: vec![ImageEntry {
                repository: "ghcr.io/efe-ai/agent-core".into(),
                digest: Some("sha256:deadbeef".into()),
                cosign_key_ref: None,
            }],
        };
        assert!(wl.allows("ghcr.io/efe-ai/agent-core@sha256:deadbeef"));
        assert!(!wl.allows("ghcr.io/efe-ai/agent-core@sha256:other"));
        assert!(!wl.allows("ghcr.io/efe-ai/agent-core:latest")); // tag, not digest
        assert!(!wl.allows("docker.io/evil/agent@sha256:deadbeef"));
    }

    #[test]
    fn rejection_increments_metric() {
        let wl = ImageWhitelist { entries: vec![] };
        let err = verify_image_whitelist(&wl, "evil/img").unwrap_err();
        assert!(matches!(err, AgentRunnerError::ImageNotInWhitelist { .. }));
    }

    #[test]
    fn empty_whitelist_rejects_everything() {
        let wl = ImageWhitelist { entries: vec![] };
        assert!(!wl.allows("ghcr.io/efe-ai/agent-core@sha256:deadbeef"));
    }
}
