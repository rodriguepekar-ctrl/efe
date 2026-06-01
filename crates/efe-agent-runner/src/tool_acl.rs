//! Tool whitelist enforcement (3-layer defense-in-depth).
//!
//! Layer 1 (control plane) : when AgentRun is created, `tool_whitelist`
//! is copied from the template + caller-provided overrides ; we
//! validate against the global tool registry.
//!
//! Layer 2 (agent core) : the agent-core runtime in the microVM rejects
//! calls to tools not in `tool_whitelist`. Even if the LLM hallucinates
//! a tool name, agent-core refuses to dispatch it.
//!
//! Layer 3 (mcp-server) : the embedded mcp-server validates the call
//! against the run's `tool_whitelist` before execution. If layers 1+2
//! were bypassed, this is the last gate.
//!
//! This module implements layers 1 and 3 (Rust). Layer 2 is in
//! `agent-core/agent_core/main.py`.

use crate::{AgentRunnerError, AgentRunnerResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Catalog of supported tool kinds (matched against client-provided
/// `tool_whitelist` strings). Non-listed tools are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    /// Read-only — no approval required.
    WebSearch,
    FetchUrl,
    /// Read-only.
    EfeClassify,
    /// Read-only.
    EfeQueryEvents,
    /// Read/write — requires approval per write.
    EfeUpsertCatalog,
    /// Read/write — requires approval.
    EfeIngestEvents,
    /// Read/write — requires approval.
    EfeRetuneCentroids,
    /// Read-only.
    Noop,
}

impl ToolKind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "web_search" => Self::WebSearch,
            "fetch_url" => Self::FetchUrl,
            "efe_classify" => Self::EfeClassify,
            "efe_query_events" => Self::EfeQueryEvents,
            "efe_upsert_catalog" => Self::EfeUpsertCatalog,
            "efe_ingest_events" => Self::EfeIngestEvents,
            "efe_retune_centroids" => Self::EfeRetuneCentroids,
            "noop" => Self::Noop,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::FetchUrl => "fetch_url",
            Self::EfeClassify => "efe_classify",
            Self::EfeQueryEvents => "efe_query_events",
            Self::EfeUpsertCatalog => "efe_upsert_catalog",
            Self::EfeIngestEvents => "efe_ingest_events",
            Self::EfeRetuneCentroids => "efe_retune_centroids",
            Self::Noop => "noop",
        }
    }

    /// True if this tool *mutates state* and therefore needs operator
    /// approval per call.
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Self::EfeUpsertCatalog | Self::EfeIngestEvents | Self::EfeRetuneCentroids
        )
    }
}

/// Resolved whitelist : a typed set built from user-provided strings.
#[derive(Debug, Clone)]
pub struct ToolWhitelist {
    pub allowed: BTreeSet<ToolKind>,
}

impl ToolWhitelist {
    pub fn from_strings(items: &[String]) -> AgentRunnerResult<Self> {
        let mut set = BTreeSet::new();
        for s in items {
            let kind = ToolKind::parse(s)
                .ok_or_else(|| AgentRunnerError::InvalidInput(format!(
                    "unknown tool '{}'", s
                )))?;
            set.insert(kind);
        }
        if set.is_empty() {
            return Err(AgentRunnerError::InvalidInput(
                "tool whitelist must not be empty".into(),
            ));
        }
        Ok(Self { allowed: set })
    }

    pub fn contains(&self, tool: &str) -> bool {
        ToolKind::parse(tool)
            .map(|k| self.allowed.contains(&k))
            .unwrap_or(false)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.allowed.iter().map(|k| k.name()).collect()
    }
}

/// Result of an ACL check. Either Allow / RequireApproval / Deny.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AclDecision {
    Allow,
    RequireApproval,
    Deny(String),
}

/// Enforce the whitelist + write-approval rule. Increments the metric
/// on rejection.
pub fn check_tool_call(
    whitelist: &ToolWhitelist,
    tool_name: &str,
    run_id: Uuid,
    require_approval_for_writes: bool,
    has_valid_approval_token: bool,
    tenant: &str,
) -> AgentRunnerResult<AclDecision> {
    let kind = match ToolKind::parse(tool_name) {
        Some(k) => k,
        None => {
            crate::metrics::AGENT_TOOL_REJECTED_WHITELIST_TOTAL
                .with_label_values(&[tenant, tool_name])
                .inc();
            return Err(AgentRunnerError::ToolNotInWhitelist {
                run_id,
                tool: tool_name.to_string(),
            });
        }
    };
    if !whitelist.allowed.contains(&kind) {
        crate::metrics::AGENT_TOOL_REJECTED_WHITELIST_TOTAL
            .with_label_values(&[tenant, tool_name])
            .inc();
        return Err(AgentRunnerError::ToolNotInWhitelist {
            run_id,
            tool: tool_name.to_string(),
        });
    }
    if kind.is_write() && require_approval_for_writes {
        if has_valid_approval_token {
            return Ok(AclDecision::Allow);
        }
        return Ok(AclDecision::RequireApproval);
    }
    Ok(AclDecision::Allow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wl(items: &[&str]) -> ToolWhitelist {
        ToolWhitelist::from_strings(
            &items.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        ).unwrap()
    }

    #[test]
    fn parse_known_tools() {
        assert_eq!(ToolKind::parse("web_search"), Some(ToolKind::WebSearch));
        assert_eq!(ToolKind::parse("efe_upsert_catalog"), Some(ToolKind::EfeUpsertCatalog));
        assert_eq!(ToolKind::parse("evil_tool"), None);
    }

    #[test]
    fn write_tools_classified() {
        assert!(ToolKind::EfeUpsertCatalog.is_write());
        assert!(!ToolKind::WebSearch.is_write());
    }

    #[test]
    fn whitelist_rejects_unknown() {
        let res = ToolWhitelist::from_strings(&vec!["evil_tool".into()]);
        assert!(res.is_err());
    }

    #[test]
    fn whitelist_rejects_empty() {
        let res = ToolWhitelist::from_strings(&vec![]);
        assert!(res.is_err());
    }

    #[test]
    fn check_allows_listed_read_tool() {
        let w = wl(&["web_search"]);
        let d = check_tool_call(&w, "web_search", Uuid::new_v4(), true, false, "t1").unwrap();
        assert_eq!(d, AclDecision::Allow);
    }

    #[test]
    fn check_rejects_unlisted_tool() {
        let w = wl(&["web_search"]);
        let err = check_tool_call(&w, "efe_upsert_catalog", Uuid::new_v4(), true, true, "t1")
            .unwrap_err();
        assert!(matches!(err, AgentRunnerError::ToolNotInWhitelist { .. }));
    }

    #[test]
    fn check_write_requires_approval() {
        let w = wl(&["efe_upsert_catalog"]);
        let d = check_tool_call(
            &w, "efe_upsert_catalog", Uuid::new_v4(), true, false, "t1",
        )
        .unwrap();
        assert_eq!(d, AclDecision::RequireApproval);
    }

    #[test]
    fn check_write_with_token_allowed() {
        let w = wl(&["efe_upsert_catalog"]);
        let d = check_tool_call(
            &w, "efe_upsert_catalog", Uuid::new_v4(), true, true, "t1",
        )
        .unwrap();
        assert_eq!(d, AclDecision::Allow);
    }

    #[test]
    fn check_write_skipped_when_approval_disabled() {
        let w = wl(&["efe_upsert_catalog"]);
        let d = check_tool_call(
            &w, "efe_upsert_catalog", Uuid::new_v4(), false, false, "t1",
        )
        .unwrap();
        assert_eq!(d, AclDecision::Allow);
    }

    #[test]
    fn unknown_tool_increments_metric() {
        let w = wl(&["web_search"]);
        let _ = check_tool_call(&w, "evil", Uuid::new_v4(), true, false, "tenant_x");
        // Non-zero — exact count is best-effort across tests.
        let v = crate::metrics::AGENT_TOOL_REJECTED_WHITELIST_TOTAL
            .with_label_values(&["tenant_x", "evil"])
            .get();
        assert!(v >= 1);
    }
}
