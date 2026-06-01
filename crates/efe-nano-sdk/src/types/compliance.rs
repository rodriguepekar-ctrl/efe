//! [`AuditPolicy`] and [`ComplianceTags`].

use serde::{Deserialize, Serialize};

/// What the runtime records in the audit chain for each call.
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::AuditPolicy;
///
/// // Default : record full input + output, no PII redaction.
/// let p = AuditPolicy::default();
/// assert!(p.include_input);
/// assert!(p.include_output);
///
/// // PII-handling agents enable redaction.
/// let p2 = AuditPolicy::redact_pii_only();
/// assert!(p2.redact_pii);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditPolicy {
    /// Include the input value in the trace.
    pub include_input: bool,

    /// Include the output value in the trace.
    pub include_output: bool,

    /// Redact PII before logging the input/output.
    pub redact_pii: bool,
}

impl AuditPolicy {
    /// Default : log full input + output, no PII redaction.
    pub fn default_full_logging() -> Self {
        Self {
            include_input: true,
            include_output: true,
            redact_pii: false,
        }
    }

    /// Log only hashes (no input/output bodies).
    pub fn hash_only() -> Self {
        Self {
            include_input: false,
            include_output: false,
            redact_pii: false,
        }
    }

    /// Log with PII redaction applied.
    pub fn redact_pii_only() -> Self {
        Self {
            include_input: true,
            include_output: true,
            redact_pii: true,
        }
    }
}

impl Default for AuditPolicy {
    fn default() -> Self {
        Self::default_full_logging()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Compliance tags — which regulatory regimes apply to this agent
// ─────────────────────────────────────────────────────────────────────

/// PII handling strategy for Mode C agents (Sprint 5).
///
/// Default `None` for Mode A / Mode B agents.  Mode C starter agents set
/// `PreLlmRedact` explicitly in their manifest.  Customers can opt-out
/// per-agent via `None` after explicit tenant DPIA sign-off.
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::PiiStrategy;
///
/// // Default is None (no redaction). Mode A/B agents typically use this.
/// let s = PiiStrategy::default();
/// assert_eq!(s, PiiStrategy::None);
///
/// // Mode C agents default to pre-LLM redaction.
/// let pre = PiiStrategy::PreLlmRedact;
/// assert_ne!(pre, PiiStrategy::None);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PiiStrategy {
    /// No redaction (opt-out — requires explicit tenant DPIA sign-off).
    #[default]
    None,

    /// Pre-LLM redaction (Sprint 5 default for Mode C).
    /// Input PII is replaced by tokens BEFORE the LLM call.
    PreLlmRedact,

    /// Strict mode : pre-LLM redact + reject any LLM response that
    /// contains the redaction tokens (anti-leakage check).
    Strict,
}

impl PiiStrategy {
    /// True if any redaction is performed before the LLM call.
    pub fn redacts_input(self) -> bool {
        matches!(self, Self::PreLlmRedact | Self::Strict)
    }

    /// True if LLM output is checked for leaked redaction tokens.
    pub fn checks_output_for_leaks(self) -> bool {
        matches!(self, Self::Strict)
    }

    /// **Sprint 8 — N12 cache key hardening** : stable string
    /// discriminant used as a cache-key component.  Bumping this
    /// rename would invalidate every cached entry — keep stable.
    ///
    /// The serde representation (`#[serde(rename_all = "snake_case")]`)
    /// is intentionally aligned with this method so disk-serialized
    /// traces and cache-key components agree byte-for-byte.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PreLlmRedact => "pre_llm_redact",
            Self::Strict => "strict",
        }
    }
}

/// Compliance regimes this nano agent's outputs may be subject to.
///
/// At runtime, the dispatcher checks these tags against the tenant's
/// active compliance attestations before allowing execution.
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::ComplianceTags;
///
/// let none = ComplianceTags::default();
/// assert!(!none.has_any_high_risk());
///
/// let high = ComplianceTags { ai_act_high_risk: true, ..Default::default() };
/// assert!(high.has_any_high_risk());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ComplianceTags {
    /// GDPR (EU General Data Protection Regulation).
    #[serde(default)]
    pub gdpr: bool,

    /// HIPAA (US Health Insurance Portability and Accountability Act).
    #[serde(default)]
    pub hipaa: bool,

    /// GxP (Pharma : Good Manufacturing/Clinical/Laboratory Practice).
    #[serde(default)]
    pub gxp: bool,

    /// FINMA (Swiss Financial Market Supervisory Authority).
    #[serde(default)]
    pub finma: bool,

    /// AI Act Article 9 — high-risk AI system.
    #[serde(default)]
    pub ai_act_high_risk: bool,

    /// DPIA (Data Protection Impact Assessment) required before execution.
    #[serde(default)]
    pub dpia_required: bool,

    /// PII redaction strategy.  Default `None` for Mode A/B ;
    /// `PreLlmRedact` set by Mode C starter agent manifests (Sprint 5).
    #[serde(default)]
    pub pii_redaction_strategy: PiiStrategy,

    /// **Sprint 7 — N8** : Prompt-injection filter strategy.
    /// Default `Off` for Mode A/B (no LLM ⇒ no injection surface).
    /// Mode C starter agents should set `Block` (TIER-1 + TIER-AUDIT)
    /// or `Warn` (TIER-EXPERIMENTAL).
    #[serde(default)]
    pub prompt_injection_strategy: PromptInjectionStrategy,
}

/// **Sprint 7 — N8** : What to do when the Mode C input is flagged
/// as a suspected prompt-injection attempt.
///
/// The filter runs **after** PII redaction but **before** the LLM call.
/// See `efe-nano::runtime::prompt_injection` for the detector impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptInjectionStrategy {
    /// Filter disabled.  Used by Mode A/B (no LLM call) and by Mode C
    /// agents where the input is trusted (internal pipelines).
    #[default]
    Off,

    /// Run the filter ; on detection, **log + metric + continue**.
    /// Useful in TIER-EXPERIMENTAL while tuning the filter's threshold
    /// to avoid false-positive rejections in production.
    Warn,

    /// Run the filter ; on detection, **abort the call** with
    /// `OperationErrorKind::PromptInjectionSuspected`.  No LLM call.
    /// Default for TIER-1 and TIER-AUDIT Mode C agents.
    Block,
}

impl ComplianceTags {
    /// Returns true if ANY high-risk regime applies.
    ///
    /// High-risk = AI Act Art. 9, HIPAA, GxP, or FINMA.
    pub fn has_any_high_risk(&self) -> bool {
        self.ai_act_high_risk || self.hipaa || self.gxp || self.finma
    }

    /// Returns true if a DPIA must be signed before execution.
    pub fn requires_dpia(&self) -> bool {
        self.dpia_required
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_policy_default_logs_everything() {
        let p = AuditPolicy::default();
        assert!(p.include_input);
        assert!(p.include_output);
        assert!(!p.redact_pii);
    }

    #[test]
    fn audit_policy_hash_only() {
        let p = AuditPolicy::hash_only();
        assert!(!p.include_input);
        assert!(!p.include_output);
    }

    #[test]
    fn audit_policy_serde_round_trip() {
        let p = AuditPolicy::redact_pii_only();
        let s = serde_json::to_string(&p).unwrap();
        let p2: AuditPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(p, p2);
    }

    #[test]
    fn compliance_default_no_regimes() {
        let c = ComplianceTags::default();
        assert!(!c.gdpr);
        assert!(!c.hipaa);
        assert!(!c.gxp);
        assert!(!c.finma);
        assert!(!c.ai_act_high_risk);
        assert!(!c.dpia_required);
        assert!(!c.has_any_high_risk());
    }

    #[test]
    fn compliance_high_risk_detection() {
        for field in ["ai_act_high_risk", "hipaa", "gxp", "finma"] {
            let mut c = ComplianceTags::default();
            match field {
                "ai_act_high_risk" => c.ai_act_high_risk = true,
                "hipaa" => c.hipaa = true,
                "gxp" => c.gxp = true,
                "finma" => c.finma = true,
                _ => unreachable!(),
            }
            assert!(
                c.has_any_high_risk(),
                "field {field} should mark high-risk"
            );
        }
    }

    #[test]
    fn compliance_gdpr_alone_not_high_risk() {
        let c = ComplianceTags {
            gdpr: true,
            ..Default::default()
        };
        assert!(!c.has_any_high_risk());  // GDPR alone is data-protection, not high-risk AI
    }

    #[test]
    fn compliance_serde_omits_false_fields() {
        let c = ComplianceTags::default();
        let s = serde_json::to_string(&c).unwrap();
        // All fields false → still serialized (because Serialize derives default to include)
        // We rely on default(false) for deserialization tolerance, not omission.
        let c2: ComplianceTags = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }

    #[test]
    fn compliance_partial_deserialize_uses_defaults() {
        // Only gdpr present → others default to false
        let s = r#"{"gdpr": true}"#;
        let c: ComplianceTags = serde_json::from_str(s).unwrap();
        assert!(c.gdpr);
        assert!(!c.hipaa);
        assert!(!c.finma);
        // Sprint 5 : pii_redaction_strategy defaults to None.
        assert_eq!(c.pii_redaction_strategy, PiiStrategy::None);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 5 : PiiStrategy
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn pii_strategy_default_is_none() {
        assert_eq!(PiiStrategy::default(), PiiStrategy::None);
    }

    #[test]
    fn pii_strategy_redacts_input_predicate() {
        assert!(!PiiStrategy::None.redacts_input());
        assert!(PiiStrategy::PreLlmRedact.redacts_input());
        assert!(PiiStrategy::Strict.redacts_input());
    }

    #[test]
    fn pii_strategy_checks_output_predicate() {
        assert!(!PiiStrategy::None.checks_output_for_leaks());
        assert!(!PiiStrategy::PreLlmRedact.checks_output_for_leaks());
        assert!(PiiStrategy::Strict.checks_output_for_leaks());
    }

    #[test]
    fn pii_strategy_serde_round_trip() {
        for strategy in [
            PiiStrategy::None,
            PiiStrategy::PreLlmRedact,
            PiiStrategy::Strict,
        ] {
            let s = serde_json::to_string(&strategy).unwrap();
            let r: PiiStrategy = serde_json::from_str(&s).unwrap();
            assert_eq!(strategy, r);
        }
    }

    #[test]
    fn pii_strategy_serde_snake_case() {
        let s = serde_json::to_string(&PiiStrategy::PreLlmRedact).unwrap();
        assert_eq!(s, "\"pre_llm_redact\"");
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 8 N12 : `as_str` is the cache-key discriminant component.
    //  It MUST stay in lock-step with the serde rename — otherwise the
    //  on-disk trace `pii_redaction_strategy` string would diverge from
    //  what hashed into the cache key.
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn pii_strategy_as_str_matches_serde() {
        for strategy in [
            PiiStrategy::None,
            PiiStrategy::PreLlmRedact,
            PiiStrategy::Strict,
        ] {
            let serde_str = serde_json::to_string(&strategy).unwrap();
            // serde wraps in quotes : strip them.
            let trimmed = serde_str.trim_matches('"');
            assert_eq!(
                trimmed,
                strategy.as_str(),
                "as_str must match serde rep for {strategy:?}",
            );
        }
    }

    #[test]
    fn pii_strategy_as_str_values() {
        assert_eq!(PiiStrategy::None.as_str(), "none");
        assert_eq!(PiiStrategy::PreLlmRedact.as_str(), "pre_llm_redact");
        assert_eq!(PiiStrategy::Strict.as_str(), "strict");
    }

    #[test]
    fn compliance_with_pii_strategy_round_trip() {
        let c = ComplianceTags {
            gdpr: true,
            pii_redaction_strategy: PiiStrategy::Strict,
            ..Default::default()
        };
        let s = serde_json::to_string(&c).unwrap();
        let r: ComplianceTags = serde_json::from_str(&s).unwrap();
        assert_eq!(c, r);
    }
}
