// SPDX-License-Identifier: UNLICENSED
//! M31 — Enumerated meters.
//!
//! Every billable operation in EFE belongs to exactly one
//! [`Meter`]. The set is closed (~ 8 values) so that the
//! Prometheus cardinality budget (cf. ADR-M31-005) is respected.
//!
//! Adding a new meter requires :
//!
//! 1. A new variant on this enum.
//! 2. A row in `pricing_plan` for every plan.
//! 3. A test in `efe-billing::pricing::tests::all_meters_priced`.
//! 4. A doc-table update in `EFE_M31_FINOPS_BILLING_ARCHITECTURE.md`.

use serde::{Deserialize, Serialize};

/// Enumerated billable resources tracked by M31. See module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Meter {
    /// Synchronous classify-text RPC (mode-1 or mode-2).
    ClassifyRequests,
    /// Agent run steps actually executed (excluding skipped / failed
    /// approval steps).
    AgentRunSteps,
    /// Wall-clock seconds of agent run sandbox uptime.
    AgentRunSeconds,
    /// SigLIP vision inference calls.
    MultimodalVisionRequests,
    /// Whisper audio inference seconds.
    MultimodalAudioSeconds,
    /// Webhook delivery attempts (successful + retried).
    WebhookDeliveries,
    /// Rows processed in a batch classify job.
    BatchClassifiedRows,
    /// Audit-export gigabytes pushed to the customer sink.
    AuditExportGb,
    // ──────────────────────────────────────────────────────────────
    //  M50 Phase 2 Sprint 5 — Nano Agent meters
    //  (additive per decision G.5 in EFE_M50_PHASE_2_ARCHITECTURE.md)
    // ──────────────────────────────────────────────────────────────
    /// Total nano agent calls across Mode A + B + C.  Emitted by the
    /// M50 nano runtime on every successful call.
    NanoAgentCalls,
    /// Tokens consumed by Mode C (LLM-augmented) nano agents.  Sum of
    /// `tokens_in + tokens_out` reported by the LLM provider.
    NanoAgentLlmTokens,
    /// Mode B/C VM wall-clock seconds.  Sum of elapsed seconds spent
    /// inside a VM pool lease.
    NanoAgentVmSeconds,
}

impl Meter {
    /// Static label for Prometheus + logging.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::ClassifyRequests => "classify_requests",
            Self::AgentRunSteps => "agent_run_steps",
            Self::AgentRunSeconds => "agent_run_seconds",
            Self::MultimodalVisionRequests => "multimodal_vision_requests",
            Self::MultimodalAudioSeconds => "multimodal_audio_seconds",
            Self::WebhookDeliveries => "webhook_deliveries",
            Self::BatchClassifiedRows => "batch_classified_rows",
            Self::AuditExportGb => "audit_export_gb",
            // Sprint 5 (M50) additions :
            Self::NanoAgentCalls => "nano_agent_calls",
            Self::NanoAgentLlmTokens => "nano_agent_llm_tokens",
            Self::NanoAgentVmSeconds => "nano_agent_vm_seconds",
        }
    }

    /// All meters (in stable enum order). Useful for iteration over
    /// the meter catalogue (rollup loops, pricing reports).
    pub fn all() -> &'static [Meter] {
        &[
            Meter::ClassifyRequests,
            Meter::AgentRunSteps,
            Meter::AgentRunSeconds,
            Meter::MultimodalVisionRequests,
            Meter::MultimodalAudioSeconds,
            Meter::WebhookDeliveries,
            Meter::BatchClassifiedRows,
            Meter::AuditExportGb,
            // Sprint 5 (M50) additions :
            Meter::NanoAgentCalls,
            Meter::NanoAgentLlmTokens,
            Meter::NanoAgentVmSeconds,
        ]
    }

    /// Parse a meter from its canonical string name.
    /// Returns `None` if `name` is not recognised.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "classify_requests" => Some(Self::ClassifyRequests),
            "agent_run_steps" => Some(Self::AgentRunSteps),
            "agent_run_seconds" => Some(Self::AgentRunSeconds),
            "multimodal_vision_requests" => Some(Self::MultimodalVisionRequests),
            "multimodal_audio_seconds" => Some(Self::MultimodalAudioSeconds),
            "webhook_deliveries" => Some(Self::WebhookDeliveries),
            "batch_classified_rows" => Some(Self::BatchClassifiedRows),
            "audit_export_gb" => Some(Self::AuditExportGb),
            // Sprint 5 (M50) additions :
            "nano_agent_calls" => Some(Self::NanoAgentCalls),
            "nano_agent_llm_tokens" => Some(Self::NanoAgentLlmTokens),
            "nano_agent_vm_seconds" => Some(Self::NanoAgentVmSeconds),
            _ => None,
        }
    }

    /// Returns `true` if the meter represents a duration (in
    /// seconds) rather than a discrete count. Used by the rollup
    /// renderer to format reports.
    #[must_use]
    pub fn is_duration(self) -> bool {
        matches!(
            self,
            Self::AgentRunSeconds
                | Self::MultimodalAudioSeconds
                | Self::NanoAgentVmSeconds
        )
    }
}

/// Convenience constant : the number of meters in M31.  Bumped to 11 in
/// Sprint 5 with the addition of `NanoAgentCalls`, `NanoAgentLlmTokens`,
/// and `NanoAgentVmSeconds` (decision G.5 in M50 Phase 2 architecture).
pub const METER_COUNT: usize = 11;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_count_matches_constant() {
        assert_eq!(Meter::all().len(), METER_COUNT);
    }

    #[test]
    fn name_roundtrip_for_every_meter() {
        for m in Meter::all() {
            assert_eq!(Meter::from_name(m.name()), Some(*m));
        }
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(Meter::from_name("foo"), None);
        assert_eq!(Meter::from_name(""), None);
    }

    #[test]
    fn duration_set_contains_expected() {
        assert!(Meter::AgentRunSeconds.is_duration());
        assert!(Meter::MultimodalAudioSeconds.is_duration());
        assert!(!Meter::ClassifyRequests.is_duration());
    }

    #[test]
    fn serialization_uses_snake_case() {
        let s = serde_json::to_string(&Meter::ClassifyRequests).unwrap();
        assert_eq!(s, "\"classify_requests\"");
    }

    // ─────────────────────────────────────────────────────────────────
    //  M50 Sprint 5 — Nano Agent meter variants (decision G.5)
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn nano_agent_meters_present_in_all() {
        let all = Meter::all();
        assert!(all.contains(&Meter::NanoAgentCalls));
        assert!(all.contains(&Meter::NanoAgentLlmTokens));
        assert!(all.contains(&Meter::NanoAgentVmSeconds));
    }

    #[test]
    fn nano_agent_meters_have_snake_case_names() {
        assert_eq!(Meter::NanoAgentCalls.name(), "nano_agent_calls");
        assert_eq!(Meter::NanoAgentLlmTokens.name(), "nano_agent_llm_tokens");
        assert_eq!(Meter::NanoAgentVmSeconds.name(), "nano_agent_vm_seconds");
    }

    #[test]
    fn nano_agent_meters_round_trip_via_from_name() {
        for m in [
            Meter::NanoAgentCalls,
            Meter::NanoAgentLlmTokens,
            Meter::NanoAgentVmSeconds,
        ] {
            assert_eq!(Meter::from_name(m.name()), Some(m), "round-trip {m:?}");
        }
    }

    #[test]
    fn nano_agent_vm_seconds_is_duration() {
        assert!(Meter::NanoAgentVmSeconds.is_duration());
        assert!(!Meter::NanoAgentCalls.is_duration());
        assert!(!Meter::NanoAgentLlmTokens.is_duration());
    }

    #[test]
    fn nano_agent_meters_serialize_with_snake_case() {
        let s = serde_json::to_string(&Meter::NanoAgentCalls).unwrap();
        assert_eq!(s, "\"nano_agent_calls\"");
        let s = serde_json::to_string(&Meter::NanoAgentLlmTokens).unwrap();
        assert_eq!(s, "\"nano_agent_llm_tokens\"");
        let s = serde_json::to_string(&Meter::NanoAgentVmSeconds).unwrap();
        assert_eq!(s, "\"nano_agent_vm_seconds\"");
    }

    #[test]
    fn meter_count_is_eleven_after_sprint_5() {
        assert_eq!(METER_COUNT, 11);
        assert_eq!(Meter::all().len(), 11);
    }
}
