//! **Sprint 8 — N12 day 1** : CacheKey v2 — the 9-component discriminator.
//!
//! See `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` § 5.
//!
//! # Wire format (deterministic, byte-stable)
//!
//! ```text
//! SHA-256(
//!     "efe-nano-idempotency-v2"          ← domain prefix (1-shot rotation)
//!     \x00
//!     <tenant_id : 16 bytes raw UUID>
//!     <agent_id : UTF-8>
//!     \x00
//!     <resolved_version : UTF-8>          ← from catalog, NOT VersionReq
//!     \x00
//!     <execution_mode : "pure_rust" | "local_model" | "llm_augmented">
//!     \x00
//!     [if Mode C : <provider>:<model>\x00<prompt_template_id>]
//!     \x00
//!     <pii_strategy : "none" | "pre_llm_redact" | "strict">
//!     \x00
//!     <canonical_input_json>
//! )
//! ```
//!
//! Output : 64-char lowercase hex string (cheap clone + comparable).
//!
//! # The 5 N12 audit-driven fixes
//!
//! 1. **Execution-mode discrimination** — prevents Mode A/B/C cache
//!    collisions when customer agents share an id at different modes
//!    (Phase 4 customer authoring).
//! 2. **Mode C provider/model discrimination** — same agent dispatched
//!    to Anthropic vs OpenAI produces different cached outputs.
//! 3. **Prompt-template-id discrimination** — bumping the template
//!    (M21 prompt registry) busts the cache cleanly.
//! 4. **PII-strategy discrimination** — toggling `PiiStrategy` busts
//!    the cache so redacted-era queries don't hit un-redacted-era
//!    cached outputs (regulatory).
//! 5. **Domain prefix versioning** — `"v2"` suffix enables one-shot
//!    rotation of the entire cache (incident response / schema bump).

use efe_nano_sdk::{ExecutionMode, NanoAgent, TenantId};
use sha2::{Digest, Sha256};
use std::fmt;

// ─────────────────────────────────────────────────────────────────────
//  Constants
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12** : domain prefix.  Bumping `v2 → v3` mass-invalidates
/// every entry without enumeration.  See ADR-N12-003.
pub const CACHE_KEY_DOMAIN_PREFIX: &[u8] = b"efe-nano-idempotency-v2";

/// The pre-N12 (Sprint 4) domain prefix.  Kept for dual-read backward
/// compat during the 24h migration window — see
/// [`derive_key_v1_compat`].
pub const CACHE_KEY_DOMAIN_PREFIX_V1: &[u8] = b"efe-nano-idempotency-v1";

// ─────────────────────────────────────────────────────────────────────
//  CacheKey type
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12** : opaque cache key — a 64-char lowercase hex
/// SHA-256 digest.  Created via [`derive_key_v2`] ; compared by string
/// equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey(pub(crate) String);

impl CacheKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Test-only constructor for cases where the key needs to be hand-
    /// crafted (mock harnesses, replay-from-snapshot).  Production
    /// code MUST use [`derive_key_v2`].
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }
}

impl fmt::Display for CacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  derive_key_v2 — the heart of N12
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12** : derive a CacheKey from the dispatch context.
///
/// Takes `&NanoAgent` (the catalog-resolved agent) so all 9 discriminator
/// components are read from one source of truth — no caller can
/// accidentally drop a component.
pub fn derive_key_v2(tenant: TenantId, agent: &NanoAgent, canonical_input: &[u8]) -> CacheKey {
    let mut h = Sha256::new();

    // 1. Domain prefix (versioned, NUL-terminated).
    h.update(CACHE_KEY_DOMAIN_PREFIX);
    h.update([0u8]);

    // 2. Tenant (16 bytes raw UUID — fixed-width, no terminator needed).
    h.update(tenant.0.as_bytes());

    // 3. Agent id (UTF-8, NUL-terminated).  NanoAgentId's validator
    //    rejects NUL bytes, so the terminator is unambiguous.
    h.update(agent.id.as_str().as_bytes());
    h.update([0u8]);

    // 4. Resolved version (UTF-8, NUL-terminated).  Semver validator
    //    rejects NUL bytes.
    h.update(agent.version.as_str().as_bytes());
    h.update([0u8]);

    // 5. Execution mode discriminant (static strings, NUL-terminated).
    h.update(agent.execution_mode.mode_str().as_bytes());
    h.update([0u8]);

    // 6. Mode C provider/model + prompt template id (if Mode C, else
    //    empty section followed by NUL).
    if let ExecutionMode::LlmAugmented {
        provider,
        model,
        prompt_template_id,
        ..
    } = &agent.execution_mode
    {
        h.update(provider.as_bytes());
        h.update(b":");
        h.update(model.as_bytes());
        h.update([0u8]);
        h.update(prompt_template_id.as_bytes());
    }
    h.update([0u8]);

    // 7. PII strategy discriminant.
    h.update(agent.compliance.pii_redaction_strategy.as_str().as_bytes());
    h.update([0u8]);

    // 8. Canonical input bytes (caller pre-canonicalizes via
    //    `crate::idempotency::canonicalize_json`).
    h.update(canonical_input);

    // Finalize + hex-encode.
    let digest = h.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("write to String never fails");
    }
    CacheKey(hex)
}

/// **Sprint 8 — N12 migration helper** : derive a v1-compat cache key
/// (pre-N12 shape).  Used for one cycle during the migration window
/// so old v1 entries still serve hits.  Sprint 9 deprecates + removes.
pub fn derive_key_v1_compat(
    tenant: TenantId,
    agent_id: &efe_nano_sdk::NanoAgentId,
    version: &efe_nano_sdk::NanoAgentVersion,
    canonical_input: &[u8],
) -> CacheKey {
    let mut h = Sha256::new();
    h.update(CACHE_KEY_DOMAIN_PREFIX_V1);
    h.update([0u8]);
    h.update(tenant.0.as_bytes());
    h.update(agent_id.as_str().as_bytes());
    h.update([0u8]);
    h.update(version.as_str().as_bytes());
    h.update([0u8]);
    h.update(canonical_input);

    let digest = h.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(hex, "{byte:02x}").expect("write to String never fails");
    }
    CacheKey(hex)
}

// ─────────────────────────────────────────────────────────────────────
//  Tests — the 10 discrimination invariants
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, IdempotencyPolicy,
        NanoAgent, NanoAgentId, NanoAgentVersion, PiiStrategy, ResourceLimits, StabilityTier,
    };
    use serde_json::json;

    fn base_agent() -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test_agent").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "test".into(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "test_entry".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            error_schema: None,
            limits: ResourceLimits::default(),
            cost_estimate_cents: 0.0001,
            latency_p99_ms_target: 100,
            audit: AuditPolicy::default(),
            compliance: ComplianceTags::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn canonical(v: &serde_json::Value) -> Vec<u8> {
        super::super::canonicalize_json(v).into_bytes()
    }

    // ─────────────────────────────────────────────────────────────────
    //  Discrimination invariants (10 dimensions)
    // ─────────────────────────────────────────────────────────────────

    /// **D1** — different resolved versions → different keys.
    #[test]
    fn distinguishes_resolved_version() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        b.version = NanoAgentVersion::new("1.0.1").unwrap();
        // Force a non-version difference so the test isolates version.
        a.version = NanoAgentVersion::new("1.0.0").unwrap();
        let input = canonical(&json!({}));
        assert_ne!(derive_key_v2(t, &a, &input), derive_key_v2(t, &b, &input));
    }

    /// **D2** — different tenants → different keys.
    #[test]
    fn distinguishes_tenant() {
        let a = base_agent();
        let t1 = TenantId::new();
        let t2 = TenantId::new();
        let input = canonical(&json!({}));
        assert_ne!(
            derive_key_v2(t1, &a, &input),
            derive_key_v2(t2, &a, &input)
        );
    }

    /// **D3** — different agent ids → different keys.
    #[test]
    fn distinguishes_agent_id() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.id = NanoAgentId::new("agent_a").unwrap();
        b.id = NanoAgentId::new("agent_b").unwrap();
        let input = canonical(&json!({}));
        assert_ne!(derive_key_v2(t, &a, &input), derive_key_v2(t, &b, &input));
    }

    /// **D4** — different execution modes → different keys.
    /// Catches the latent Phase-4 bug : tenant T republishes
    /// `json_validate` from Mode A to Mode B with same version.
    #[test]
    fn distinguishes_execution_mode() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.execution_mode = ExecutionMode::PureRust {
            entry: "fn_a".into(),
        };
        b.execution_mode = ExecutionMode::LocalModel {
            model_id: "fn_a".into(),
            model_version: "1.0.0".into(),
        };
        let input = canonical(&json!({}));
        assert_ne!(derive_key_v2(t, &a, &input), derive_key_v2(t, &b, &input));
    }

    /// **D5** — Mode C : different LLM providers → different keys.
    #[test]
    fn distinguishes_mode_c_provider() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.execution_mode = ExecutionMode::LlmAugmented {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            prompt_template_id: "m50.test.v1".into(),
            max_tokens: 1000,
            temperature: 0.0,
        };
        b.execution_mode = ExecutionMode::LlmAugmented {
            provider: "openai".into(),
            model: "claude-sonnet-4-5".into(), // identical model name (semantically meaningless cross-provider)
            prompt_template_id: "m50.test.v1".into(),
            max_tokens: 1000,
            temperature: 0.0,
        };
        let input = canonical(&json!({}));
        assert_ne!(derive_key_v2(t, &a, &input), derive_key_v2(t, &b, &input));
    }

    /// **D6** — Mode C : different LLM models → different keys.
    #[test]
    fn distinguishes_mode_c_model() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.execution_mode = ExecutionMode::LlmAugmented {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            prompt_template_id: "m50.test.v1".into(),
            max_tokens: 1000,
            temperature: 0.0,
        };
        b.execution_mode = ExecutionMode::LlmAugmented {
            provider: "anthropic".into(),
            model: "claude-haiku-4-5".into(),
            prompt_template_id: "m50.test.v1".into(),
            max_tokens: 1000,
            temperature: 0.0,
        };
        let input = canonical(&json!({}));
        assert_ne!(derive_key_v2(t, &a, &input), derive_key_v2(t, &b, &input));
    }

    /// **D7** — Mode C : different prompt_template_id → different keys.
    /// Catches D3 from the audit : "no prompt-template-version
    /// discrimination" — the most insidious of the 5 audit defects.
    #[test]
    fn distinguishes_mode_c_prompt_template_id() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.execution_mode = ExecutionMode::LlmAugmented {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            prompt_template_id: "m50.summarize_strict.v1".into(),
            max_tokens: 1000,
            temperature: 0.0,
        };
        b.execution_mode = ExecutionMode::LlmAugmented {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            prompt_template_id: "m50.summarize_strict.v2".into(), // bumped
            max_tokens: 1000,
            temperature: 0.0,
        };
        let input = canonical(&json!({}));
        assert_ne!(derive_key_v2(t, &a, &input), derive_key_v2(t, &b, &input));
    }

    /// **D8** — different PII strategies → different keys.
    /// Catches D4 from the audit : un-redacted-era cached output
    /// served to redacted-era query.  Regulatory.
    #[test]
    fn distinguishes_pii_strategy() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.compliance = ComplianceTags {
            pii_redaction_strategy: PiiStrategy::None,
            ..ComplianceTags::default()
        };
        b.compliance = ComplianceTags {
            pii_redaction_strategy: PiiStrategy::PreLlmRedact,
            ..ComplianceTags::default()
        };
        let input = canonical(&json!({}));
        assert_ne!(derive_key_v2(t, &a, &input), derive_key_v2(t, &b, &input));
    }

    /// **D9** — different inputs → different keys (canonical bytes).
    #[test]
    fn distinguishes_canonical_input() {
        let t = TenantId::system();
        let a = base_agent();
        let i1 = canonical(&json!({"x": 1}));
        let i2 = canonical(&json!({"x": 2}));
        assert_ne!(derive_key_v2(t, &a, &i1), derive_key_v2(t, &a, &i2));
    }

    /// **D10** — same context → same key (idempotency).
    #[test]
    fn determinism_same_inputs_same_key() {
        let t = TenantId::system();
        let a = base_agent();
        let i = canonical(&json!({"x": 1}));
        assert_eq!(derive_key_v2(t, &a, &i), derive_key_v2(t, &a, &i));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Format invariants
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn key_is_64_hex_chars() {
        let t = TenantId::system();
        let a = base_agent();
        let k = derive_key_v2(t, &a, &canonical(&json!({})));
        assert_eq!(k.as_str().len(), 64);
        assert!(k.as_str().chars().all(|c| c.is_ascii_hexdigit()));
        assert!(k.as_str().chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn domain_prefix_v2_vs_v1_yields_different_keys() {
        // The same (tenant, agent, input) triple under v1 vs v2
        // produces different keys — proves domain prefix isolation.
        let t = TenantId::system();
        let a = base_agent();
        let i = canonical(&json!({"x": 1}));
        let k_v2 = derive_key_v2(t, &a, &i);
        let k_v1 = derive_key_v1_compat(t, &a.id, &a.version, &i);
        assert_ne!(k_v2, k_v1);
    }

    #[test]
    fn cache_key_into_string_round_trip() {
        let t = TenantId::system();
        let a = base_agent();
        let k = derive_key_v2(t, &a, &canonical(&json!({})));
        let raw = k.clone().into_string();
        assert_eq!(raw.len(), 64);
        assert_eq!(k.as_str(), raw);
    }

    #[test]
    fn display_matches_as_str() {
        let t = TenantId::system();
        let a = base_agent();
        let k = derive_key_v2(t, &a, &canonical(&json!({})));
        assert_eq!(format!("{k}"), k.as_str());
    }

    #[test]
    fn cache_key_hashable_in_hashmap() {
        use std::collections::HashMap;
        let t = TenantId::system();
        let a = base_agent();
        let k = derive_key_v2(t, &a, &canonical(&json!({})));
        let mut m: HashMap<CacheKey, i32> = HashMap::new();
        m.insert(k.clone(), 42);
        assert_eq!(m.get(&k), Some(&42));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Negative tests — what should NOT vary the key
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn description_does_not_affect_key() {
        // Cosmetic changes to the agent metadata don't bust the cache.
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.description = "first description".into();
        b.description = "different description".into();
        let i = canonical(&json!({}));
        assert_eq!(derive_key_v2(t, &a, &i), derive_key_v2(t, &b, &i));
    }

    #[test]
    fn cost_estimate_does_not_affect_key() {
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.cost_estimate_cents = 0.001;
        b.cost_estimate_cents = 0.999;
        let i = canonical(&json!({}));
        assert_eq!(derive_key_v2(t, &a, &i), derive_key_v2(t, &b, &i));
    }

    #[test]
    fn audit_policy_does_not_affect_key() {
        // audit.include_input changes what's stored in the trace, NOT
        // what the agent receives → same effective output → same key.
        let t = TenantId::system();
        let mut a = base_agent();
        let mut b = base_agent();
        a.audit = AuditPolicy::default();
        b.audit = AuditPolicy::hash_only();
        let i = canonical(&json!({}));
        assert_eq!(derive_key_v2(t, &a, &i), derive_key_v2(t, &b, &i));
    }
}
