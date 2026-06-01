//! [`NanoAgent`] — the central catalog entry.
//!
//! Plus supporting enums : [`Author`], [`Category`], [`StabilityTier`],
//! [`ExecutionMode`], [`DeterminismLevel`], [`IdempotencyPolicy`],
//! [`McpToolMetadata`], [`NanoAgentRef`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{
    AuditPolicy, ComplianceTags, CoPublishingPolicy, GoldSetRef, NanoAgentId, NanoAgentVersion,
    PublisherSignature, ResourceLimits, TenantId, VersionReq,
};

// ─────────────────────────────────────────────────────────────────────
//  Author of a nano agent
// ─────────────────────────────────────────────────────────────────────

/// Who authored this nano agent ?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Author {
    /// Shipped by EFE itself (the 20 starter pack).
    Efe,

    /// Published by a customer tenant.
    Tenant { tenant_id: TenantId },

    /// Published by a community member (third-party).
    Community { name: String },

    /// **Sprint 10 — N18** : Published by a registered identity in
    /// the publisher registry.  Carries `publisher_id` (opaque UUID
    /// resolved via [`crate::types::PublisherIdentity`]) + the
    /// snapshot of the publisher's KYC tier at publish time.  The
    /// snapshot is important : even if the publisher's tier changes
    /// later, the trace remains accurate to "what was the tier when
    /// the agent shipped".
    Publisher {
        publisher_id: super::PublisherId,
        kyc_tier: super::KycTier,
    },
}

impl Author {
    pub fn is_efe_shipped(&self) -> bool {
        matches!(self, Author::Efe)
    }

    /// **N18** : True if this author is a registered publisher
    /// (i.e. governed by the N18 registry).
    pub fn is_publisher(&self) -> bool {
        matches!(self, Author::Publisher { .. })
    }

    /// **N18** : Extract the publisher id if this is a `Publisher`
    /// variant ; returns `None` for `Efe` / `Tenant` / `Community`.
    pub fn publisher_id(&self) -> Option<super::PublisherId> {
        if let Author::Publisher { publisher_id, .. } = self {
            Some(*publisher_id)
        } else {
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Category — closed taxonomy of nano agent purposes
// ─────────────────────────────────────────────────────────────────────

/// Closed taxonomy of nano agent categories.
///
/// Used for catalog filtering and MCP tool grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// Extract structured data from raw input (PDF, HTML, XML, etc.).
    Extraction,
    /// Validate input against a schema (JSON, XML, SQL).
    Validation,
    /// Classify input into a predefined set of labels.
    Classification,
    /// Transform input format-to-format (dates, currencies, etc.).
    Transformation,
    /// Look up data in an external store (DB, vector index).
    Lookup,
    /// Named entity recognition.
    EntityRecognition,
    /// Summarize input (Mode C / LLM-augmented).
    Summarization,
    /// Structured extraction with LLM (Mode C).
    StructuredExtract,
    /// Other categories.
    Other,
}

impl Category {
    /// Returns the canonical snake_case name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::Extraction => "extraction",
            Category::Validation => "validation",
            Category::Classification => "classification",
            Category::Transformation => "transformation",
            Category::Lookup => "lookup",
            Category::EntityRecognition => "entity_recognition",
            Category::Summarization => "summarization",
            Category::StructuredExtract => "structured_extract",
            Category::Other => "other",
        }
    }

    /// All categories (used for filtering / UI).
    pub fn all() -> &'static [Category] {
        &[
            Category::Extraction,
            Category::Validation,
            Category::Classification,
            Category::Transformation,
            Category::Lookup,
            Category::EntityRecognition,
            Category::Summarization,
            Category::StructuredExtract,
            Category::Other,
        ]
    }
}

// ─────────────────────────────────────────────────────────────────────
//  StabilityTier — promotion ladder
// ─────────────────────────────────────────────────────────────────────

/// Stability tier — promotion ladder controlling schema freeze guarantees.
///
/// - **TierExperimental** : no guarantees, may break or disappear.
/// - **Tier1** : schema frozen for 12 months.
/// - **Tier2** : schema frozen for 36 months.
/// - **TierAudit** : schema frozen for 7+ years + cosign signed audit.
///
/// Promotion from one tier to the next requires :
/// - ≥ 90 days in production with successful calls (customer agents)
/// - Explicit promotion via [`crate::traits::Catalog::promote`]
///
/// EFE-shipped starter pack agents bypass the 90-day rule (audited internally).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StabilityTier {
    TierExperimental = 0,
    Tier1 = 1,
    Tier2 = 2,
    TierAudit = 3,
}

impl StabilityTier {
    /// Returns the canonical snake_case name.
    pub fn as_str(&self) -> &'static str {
        match self {
            StabilityTier::TierExperimental => "tier_experimental",
            StabilityTier::Tier1 => "tier_1",
            StabilityTier::Tier2 => "tier_2",
            StabilityTier::TierAudit => "tier_audit",
        }
    }

    /// Returns the schema-freeze duration in months for this tier.
    /// `None` means "no guarantee".
    pub fn freeze_duration_months(&self) -> Option<u32> {
        match self {
            StabilityTier::TierExperimental => None,
            StabilityTier::Tier1 => Some(12),
            StabilityTier::Tier2 => Some(36),
            StabilityTier::TierAudit => Some(84),  // 7 years
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  ExecutionMode
// ─────────────────────────────────────────────────────────────────────

/// How this nano agent is executed.
///
/// Note : does not derive `Eq` because the `LlmAugmented` variant
/// contains an `f32` field (temperature).  Use `PartialEq` for comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ExecutionMode {
    /// **Mode A** : pure Rust function in tokio thread pool.
    ///
    /// `entry` is an identifier looked up in the runtime's function registry.
    /// Used by deterministic, CPU-bound agents (validation, regex, etc.).
    PureRust { entry: String },

    /// **Mode B** : Firecracker microVM with pre-loaded ML model.
    ///
    /// `model_id` and `model_version` identify the cached model.
    /// Used by NER, sentiment, classification, embedding agents.
    LocalModel {
        model_id: String,
        model_version: String,
    },

    /// **Mode C** : Firecracker microVM with LLM call via M21 router.
    ///
    /// Used for summarization, structured extraction, etc.
    LlmAugmented {
        provider: String,
        model: String,
        prompt_template_id: String,
        max_tokens: u32,
        temperature: f32,
    },
}

impl ExecutionMode {
    /// Returns the mode discriminator string (for catalog filtering).
    pub fn mode_str(&self) -> &'static str {
        match self {
            ExecutionMode::PureRust { .. } => "pure_rust",
            ExecutionMode::LocalModel { .. } => "local_model",
            ExecutionMode::LlmAugmented { .. } => "llm_augmented",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  DeterminismLevel  (Sprint 7 — N5 split)
// ─────────────────────────────────────────────────────────────────────

/// Determinism guarantees for this nano agent's output.
///
/// **Sprint 7 — N5** : The pre-Sprint-7 `Probabilistic` variant
/// conflated three distinct contracts (Mode B seeded vs Mode C
/// replayable-via-cache vs Mode C ephemeral).  This caused the
/// `DeterminismChecker` to dispatch on the `execution_mode: String`
/// at replay time — a string-typed contract is brittle.  Sprint 7
/// splits into 5 explicit variants with closed exhaustive matching.
///
/// | Variant         | Mode | Replay behaviour                                     |
/// |-----------------|------|------------------------------------------------------|
/// | FullDeterministic | A  | Strict hash match expected ⇒ `Match` or `Mismatch` |
/// | Seeded            | B  | Seeded RNG ⇒ strict hash match expected            |
/// | LlmReplayable     | C  | Hit M27 cache ⇒ `Match` ; miss ⇒ `ReplayWindowExpired` |
/// | LlmEphemeral      | C  | Skipped at replay ⇒ `SkippedNonDeterministic`      |
/// | SideEffect        | *  | Skipped at replay ⇒ `SkippedNonDeterministic`      |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismLevel {
    /// Same input → same output, every time.
    /// Typically Mode A + validated.
    FullDeterministic,

    /// Mode B with seeded RNG.  Replay strict hash match expected.
    /// (Sprint 7 — N5 : was implicit under `Probabilistic`.)
    Seeded,

    /// Mode C with replay-via-cache.  The LLM response was cached at
    /// original call time ; replay within the configured window will
    /// hit the cache and produce a strict hash match.  Outside the
    /// window, replay surfaces `ReplayWindowExpired`.
    /// (Sprint 7 — N3 : enables Mode C audit closure.)
    LlmReplayable,

    /// Mode C without replay caching — intentionally non-replayable to
    /// avoid the cache cost.  Use for low-criticality high-volume Mode C
    /// agents where the regulatory replay guarantee is waived.
    /// (Sprint 7 — N5 : was implicit under `Probabilistic`.)
    LlmEphemeral,

    /// External I/O makes output time-dependent.
    SideEffect,
}

impl DeterminismLevel {
    /// True if replay is expected to produce a strict hash match
    /// (FullDeterministic + Seeded + LlmReplayable when cache hit).
    pub fn is_strict_replayable(self) -> bool {
        matches!(
            self,
            DeterminismLevel::FullDeterministic
                | DeterminismLevel::Seeded
                | DeterminismLevel::LlmReplayable
        )
    }
}

// ─────────────────────────────────────────────────────────────────────
//  IdempotencyPolicy
// ─────────────────────────────────────────────────────────────────────

/// How invocations of this nano agent are deduplicated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IdempotencyPolicy {
    /// Auto-hash input and cache the response for `ttl_sec`.
    Auto { ttl_sec: u32 },

    /// Customer provides an idempotency key on the call.
    Manual { ttl_sec: u32 },

    /// Every invocation executes anew (no caching).
    Disabled,
}

impl IdempotencyPolicy {
    /// Convenience : auto-hash with default 24h TTL.
    pub fn auto_24h() -> Self {
        Self::Auto { ttl_sec: 86_400 }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  McpToolMetadata
// ─────────────────────────────────────────────────────────────────────

/// MCP exposure metadata (used by Sprint 7+ to register tools with MCP server).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolMetadata {
    /// Tool name as exposed via MCP (e.g. "efe_nano_pdf_extract_text").
    pub mcp_name: String,

    /// Description shown in MCP client tool palette.
    pub mcp_description: String,

    /// Visible by default in Claude Desktop / Cursor palette ?
    pub visible_by_default: bool,
}

// ─────────────────────────────────────────────────────────────────────
//  NanoAgentRef — versioned reference to an agent
// ─────────────────────────────────────────────────────────────────────

/// A reference to a nano agent : its id + version requirement.
///
/// Used in [`crate::types::Operation`] to identify which agent to invoke.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoAgentRef {
    pub id: NanoAgentId,

    /// Version requirement (e.g. "^1.0", "=1.2.3", "*").
    /// Defaults to "*" if absent.
    #[serde(default)]
    pub version: VersionReq,
}

impl NanoAgentRef {
    pub fn new(id: NanoAgentId) -> Self {
        Self {
            id,
            version: VersionReq::any(),
        }
    }

    pub fn with_version(id: NanoAgentId, version: VersionReq) -> Self {
        Self { id, version }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  NanoAgent — the central catalog entry
// ─────────────────────────────────────────────────────────────────────

/// JSON Schema (draft 2020-12).
///
/// Stored as a serde_json::Value to keep dependencies minimal in the SDK.
/// Validation is performed by the runtime crate.
pub type JsonSchema = serde_json::Value;

/// Catalog entry for a nano agent.
///
/// Identity is `(id, version)` — different versions of the same id
/// coexist in the catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoAgent {
    pub id: NanoAgentId,
    pub version: NanoAgentVersion,

    /// Human-readable description (≤ 256 chars).
    pub description: String,

    pub author: Author,
    pub category: Category,
    pub stability_tier: StabilityTier,

    pub execution_mode: ExecutionMode,
    pub determinism: DeterminismLevel,
    pub idempotency: IdempotencyPolicy,

    /// JSON Schema for the input (validated before execute).
    pub input_schema: JsonSchema,

    /// JSON Schema for the output (validated after execute).
    pub output_schema: JsonSchema,

    /// Optional JSON Schema for typed error payloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_schema: Option<JsonSchema>,

    pub limits: ResourceLimits,

    /// Estimated cost per call in cents.
    pub cost_estimate_cents: f64,

    /// Target latency p99 in milliseconds (used for SLA monitoring).
    pub latency_p99_ms_target: u32,

    pub audit: AuditPolicy,
    pub compliance: ComplianceTags,

    /// MCP exposure metadata (Sprint 7+).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_tool_metadata: Option<McpToolMetadata>,

    /// **Sprint 10 — N18** : Ed25519 publisher signature over the
    /// canonical body (see architecture § 7.2 + annex A2).
    ///
    /// - `None` for pre-N18 agents (treated as Tier 0 anonymous per
    ///   ADR-9 + F13 fallback path).
    /// - `Some(_)` after the publisher signs.  Verification is done by
    ///   `efe-publisher-identity::SignatureVerifier` against the
    ///   publisher's `active_key` at publish time (or the matching
    ///   historical key after rotation — I7).
    ///
    /// The signature is **excluded from the canonical body hash**
    /// itself (chicken-and-egg).  It is stored alongside.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<PublisherSignature>,

    /// **Sprint 10 — N18** : N-of-M co-publishing policy (ADR-7 + G8).
    ///
    /// `None` (default) means single-signer.  `Some` carries the
    /// signer set + threshold ; runtime enforcement of the threshold
    /// is Sprint 11 — the wire format is frozen Sprint 10 so already
    /// published agents validate forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub co_publishing: Option<CoPublishingPolicy>,

    /// **Sprint 11 — N19** : reference to the operator-approved
    /// gold set that gates this agent's Tier 1 promotion.  `None`
    /// for pre-N19 agents (grandfathered for 90 days per ADR-4) or
    /// agents that are not eligible for Tier 1 (e.g.
    /// `DeterminismLevel::SideEffect` per I9).
    ///
    /// The reference carries both the `gold_set_id` and the content
    /// hash — at run time the gold gate verifies that the registry's
    /// stored gold set still hashes to this value (defense against
    /// silent gold-set rewrites).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold_set_ref: Option<GoldSetRef>,

    pub created_at: DateTime<Utc>,
}

impl NanoAgent {
    /// Returns true if this is an EFE-shipped agent.
    pub fn is_efe_shipped(&self) -> bool {
        self.author.is_efe_shipped()
    }

    /// Returns true if this agent's compliance tags require DPIA.
    pub fn requires_dpia(&self) -> bool {
        self.compliance.requires_dpia()
    }

    /// **Sprint 10 — N18** : true if this agent carries a publisher
    /// signature.  Pre-N18 agents return `false` (per F13 fallback,
    /// they're treated as Tier 0 anonymous).
    pub fn is_signed(&self) -> bool {
        self.signature.is_some()
    }

    /// **Sprint 10 — N18** : true if this agent has co-publishers.
    pub fn is_co_published(&self) -> bool {
        self.co_publishing.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent() -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test_agent").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "Sample test agent".to_string(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "test_agent::run".to_string(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.01,
            latency_p99_ms_target: 50,
            audit: AuditPolicy::default(),
            compliance: ComplianceTags::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn category_as_str_matches_serde() {
        for cat in Category::all() {
            let serialized = serde_json::to_string(cat).unwrap();
            // serialized = "\"extraction\""
            let expected = format!(r#""{}""#, cat.as_str());
            assert_eq!(serialized, expected);
        }
    }

    #[test]
    fn category_round_trip() {
        for cat in Category::all() {
            let s = serde_json::to_string(cat).unwrap();
            let c2: Category = serde_json::from_str(&s).unwrap();
            assert_eq!(*cat, c2);
        }
    }

    #[test]
    fn stability_tier_ordering() {
        assert!(StabilityTier::TierExperimental < StabilityTier::Tier1);
        assert!(StabilityTier::Tier1 < StabilityTier::Tier2);
        assert!(StabilityTier::Tier2 < StabilityTier::TierAudit);
    }

    #[test]
    fn stability_tier_freeze_durations() {
        assert_eq!(StabilityTier::TierExperimental.freeze_duration_months(), None);
        assert_eq!(StabilityTier::Tier1.freeze_duration_months(), Some(12));
        assert_eq!(StabilityTier::Tier2.freeze_duration_months(), Some(36));
        assert_eq!(StabilityTier::TierAudit.freeze_duration_months(), Some(84));
    }

    #[test]
    fn execution_mode_str_discriminator() {
        let a = ExecutionMode::PureRust {
            entry: "x".to_string(),
        };
        let b = ExecutionMode::LocalModel {
            model_id: "m".to_string(),
            model_version: "1".to_string(),
        };
        let c = ExecutionMode::LlmAugmented {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            prompt_template_id: "summarize_strict".to_string(),
            max_tokens: 1024,
            temperature: 0.2,
        };
        assert_eq!(a.mode_str(), "pure_rust");
        assert_eq!(b.mode_str(), "local_model");
        assert_eq!(c.mode_str(), "llm_augmented");
    }

    #[test]
    fn nano_agent_serde_round_trip() {
        let a = sample_agent();
        let s = serde_json::to_string(&a).unwrap();
        let a2: NanoAgent = serde_json::from_str(&s).unwrap();
        assert_eq!(a.id, a2.id);
        assert_eq!(a.version, a2.version);
        assert_eq!(a.category, a2.category);
        assert_eq!(a.stability_tier, a2.stability_tier);
    }

    #[test]
    fn author_efe_detection() {
        let a = Author::Efe;
        assert!(a.is_efe_shipped());

        let t = Author::Tenant {
            tenant_id: TenantId::system(),
        };
        assert!(!t.is_efe_shipped());

        let c = Author::Community {
            name: "Acme".to_string(),
        };
        assert!(!c.is_efe_shipped());
    }

    #[test]
    fn nano_agent_ref_default_version_is_any() {
        let r = NanoAgentRef::new(NanoAgentId::new("foo").unwrap());
        assert_eq!(r.version.as_str(), "*");
    }

    #[test]
    fn idempotency_auto_24h() {
        let p = IdempotencyPolicy::auto_24h();
        if let IdempotencyPolicy::Auto { ttl_sec } = p {
            assert_eq!(ttl_sec, 86_400);
        } else {
            panic!("expected Auto variant");
        }
    }

    #[test]
    fn nano_agent_efe_shipped_helper() {
        let a = sample_agent();
        assert!(a.is_efe_shipped());
    }

    #[test]
    fn nano_agent_requires_dpia_when_tagged() {
        let mut a = sample_agent();
        a.compliance.dpia_required = true;
        assert!(a.requires_dpia());
    }
}
