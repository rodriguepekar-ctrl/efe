//! Admin CLI hook for replay (Sprint 6 § 13 Day 9).
//!
//! Provides the `efe-nano-admin audit replay` command surface as a
//! library function.  Production wires this to a `clap`-based CLI in a
//! separate `efe-nano-admin` binary crate ; Sprint 6 ships only the
//! library API.

use efe_nano_sdk::TenantId;
use serde::Serialize;
use uuid::Uuid;

use crate::error::ReplayError;
#[cfg(test)]
use crate::replay::types::ReplayOpStatus;
use crate::replay::types::{PerOpReplay, ReplayReport};
use crate::replay::ReplayEngine;

// ─────────────────────────────────────────────────────────────────────
//  ReplayCliArgs
// ─────────────────────────────────────────────────────────────────────

/// Parsed args for the `audit replay` command.
#[derive(Debug, Clone)]
pub struct ReplayCliArgs {
    pub tenant: TenantId,
    pub request_id: Uuid,
    /// If `Some`, write the JSON report to this file ; otherwise stdout.
    pub output_file: Option<std::path::PathBuf>,
    /// Pretty-print JSON (default true for human readability).
    pub pretty: bool,
}

// ─────────────────────────────────────────────────────────────────────
//  Run the command
// ─────────────────────────────────────────────────────────────────────

/// Execute the replay command + serialize the report.
///
/// Returns the JSON representation as a String.  Tests use this to
/// assert on the report shape ; production code writes the String to
/// stdout or `args.output_file`.
pub async fn run(
    engine: &ReplayEngine,
    args: ReplayCliArgs,
) -> Result<String, ReplayError> {
    let report = engine.replay(args.tenant, args.request_id).await?;
    let json = serialize_report(&report, args.pretty);

    if let Some(path) = &args.output_file {
        tokio::fs::write(path, &json)
            .await
            .map_err(|e| ReplayError::Internal(format!("write {path:?} : {e}")))?;
    }

    Ok(json)
}

// ─────────────────────────────────────────────────────────────────────
//  Serialization
// ─────────────────────────────────────────────────────────────────────

/// JSON-serializable mirror of [`ReplayReport`].  Uses string IDs +
/// hex hashes to avoid binary-in-JSON issues.
#[derive(Debug, Serialize)]
struct ReplayReportJson {
    request_id: String,
    tenant: String,
    chain_verified: bool,
    total_ops: u32,
    matched_ops: u32,
    mismatched_ops: u32,
    skipped_non_deterministic_ops: u32,
    failed_ops: u32,
    fully_consistent: bool,
    duration_ms: u32,
    dispatch_error: Option<String>,
    per_op: Vec<PerOpReplayJson>,
}

#[derive(Debug, Serialize)]
struct PerOpReplayJson {
    op_id: String,
    nano_agent_id: String,
    execution_mode: String,
    status: String,
    original_output_hash_hex: Option<String>,
    replay_output_hash_hex: Option<String>,
    explanation: Option<String>,
}

fn serialize_report(r: &ReplayReport, pretty: bool) -> String {
    let json = ReplayReportJson {
        request_id: r.request_id.to_string(),
        tenant: r.tenant.to_string(),
        chain_verified: r.chain_verified,
        total_ops: r.total_ops,
        matched_ops: r.matched_ops,
        mismatched_ops: r.mismatched_ops,
        skipped_non_deterministic_ops: r.skipped_non_deterministic_ops,
        failed_ops: r.failed_ops,
        fully_consistent: r.is_fully_consistent(),
        duration_ms: r.duration_ms,
        dispatch_error: r.dispatch_error.clone(),
        per_op: r.per_op.iter().map(per_op_json).collect(),
    };
    if pretty {
        serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_string())
    } else {
        serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string())
    }
}

fn per_op_json(p: &PerOpReplay) -> PerOpReplayJson {
    PerOpReplayJson {
        op_id: p.op_id.clone(),
        nano_agent_id: p.nano_agent_id.clone(),
        execution_mode: p.execution_mode.clone(),
        status: p.status.as_str().to_string(),
        original_output_hash_hex: p.original_output_hash.map(hex::encode),
        replay_output_hash_hex: p.replay_output_hash.map(hex::encode),
        explanation: p.explanation.clone(),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helper : convenience builder for `ReplayCliArgs`
// ─────────────────────────────────────────────────────────────────────

impl ReplayCliArgs {
    pub fn new(tenant: TenantId, request_id: Uuid) -> Self {
        Self {
            tenant,
            request_id,
            output_file: None,
            pretty: true,
        }
    }

    pub fn with_output_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.output_file = Some(path.into());
        self
    }

    pub fn compact(mut self) -> Self {
        self.pretty = false;
        self
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_report() -> ReplayReport {
        ReplayReport {
            request_id: Uuid::from_u128(0x1111),
            tenant: TenantId::system(),
            chain_verified: true,
            total_ops: 3,
            matched_ops: 2,
            mismatched_ops: 0,
            skipped_non_deterministic_ops: 1,
            replay_window_expired_ops: 0,
            failed_ops: 0,
            per_op: vec![
                PerOpReplay {
                    op_id: "a".into(),
                    nano_agent_id: "json_validate".into(),
                    execution_mode: "pure_rust".into(),
                    original_output_hash: Some([1u8; 32]),
                    replay_output_hash: Some([1u8; 32]),
                    status: ReplayOpStatus::Match,
                    explanation: None,
                },
                PerOpReplay {
                    op_id: "b".into(),
                    nano_agent_id: "summarize_strict".into(),
                    execution_mode: "llm_augmented".into(),
                    original_output_hash: Some([2u8; 32]),
                    replay_output_hash: Some([3u8; 32]),
                    status: ReplayOpStatus::SkippedNonDeterministic,
                    explanation: Some("LLM stochastic".into()),
                },
            ],
            new_response: None,
            dispatch_error: None,
            started_at: Utc::now(),
            completed_at: Utc::now(),
            duration_ms: 42,
        }
    }

    #[test]
    fn serialize_report_compact_is_single_line() {
        let r = sample_report();
        let s = serialize_report(&r, false);
        assert!(!s.contains('\n'), "compact should not contain newlines");
        // Sanity check JSON structure.
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["total_ops"], 3);
        assert_eq!(parsed["matched_ops"], 2);
    }

    #[test]
    fn serialize_report_pretty_has_newlines() {
        let r = sample_report();
        let s = serialize_report(&r, true);
        assert!(s.contains('\n'));
    }

    #[test]
    fn serialize_includes_hex_encoded_hashes() {
        let r = sample_report();
        let s = serialize_report(&r, false);
        // Hex of [1u8; 32] = "0101...01" (64 chars).
        assert!(s.contains(&"01".repeat(32)));
    }

    #[test]
    fn serialize_includes_status_strings() {
        let r = sample_report();
        let s = serialize_report(&r, false);
        assert!(s.contains("\"match\""));
        assert!(s.contains("\"skipped_non_deterministic\""));
    }

    #[test]
    fn args_builder_compact() {
        let args = ReplayCliArgs::new(TenantId::system(), Uuid::nil()).compact();
        assert!(!args.pretty);
        assert!(args.output_file.is_none());
    }

    #[test]
    fn args_builder_with_output_file() {
        let args = ReplayCliArgs::new(TenantId::system(), Uuid::nil())
            .with_output_file("/tmp/x.json");
        assert_eq!(args.output_file.as_ref().unwrap().to_str().unwrap(), "/tmp/x.json");
    }
}
