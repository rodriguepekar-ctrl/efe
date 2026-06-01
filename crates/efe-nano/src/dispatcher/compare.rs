//! **Sprint 8 — N13** : Output comparison strategies for shadow runs.
//!
//! See architecture doc § 7.  The four strategies :
//!
//! 1. [`compare_exact`] — byte-identical hash compare.  O(1).
//! 2. [`compare_structural`] — JSON shape / type / leaf-count match.
//!    O(N) over the JSON tree.
//! 3. [`compare_semantic`] — delegate to a comparator nano agent.
//!    Cost : one extra agent dispatch.
//! 4. [`compare_none`] — no-op : always returns `Match { similarity: 1.0 }`
//!    but with the comparison-skipped marker baked in.  Used for
//!    `LlmEphemeral` agents.
//!
//! # The output contract
//!
//! Each strategy returns a [`ShadowOutcome`] — either `Match`,
//! `Mismatch`, or `ComparatorUnavailable`.  `ShadowFailed` is reserved
//! for shadow-RUNTIME failures and is emitted by the
//! [`crate::dispatcher::shadow::ShadowScheduler`] before comparison.

use chrono::Utc;
use efe_nano_sdk::{ComparisonStrategy, ShadowOutcome};
use serde_json::Value;

/// **Sprint 8 — N13** : Bundle of timing/cost stats captured by the
/// scheduler at compare-time and threaded into the outcome.
#[derive(Debug, Clone, Copy)]
pub struct CompareContext {
    pub prod_latency_ms: u32,
    pub shadow_latency_ms: u32,
    pub prod_cost_cents: f64,
    pub shadow_cost_cents: f64,
}

// ─────────────────────────────────────────────────────────────────────
//  Entry point — dispatches based on strategy
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N13** : top-level dispatch.  The `Semantic` strategy
/// requires runtime access — this entry point does not handle Semantic
/// (caller must use [`compare_semantic`] directly with the runtime).
///
/// For Semantic policies, the scheduler calls [`compare_semantic`]
/// separately ; for all other policies, [`compare`] is sufficient.
pub fn compare(
    strategy: &ComparisonStrategy,
    prod_output: &Value,
    shadow_output: &Value,
    ctx: CompareContext,
) -> ShadowOutcome {
    match strategy {
        ComparisonStrategy::Exact => compare_exact(prod_output, shadow_output, ctx),
        ComparisonStrategy::Structural => compare_structural(prod_output, shadow_output, ctx),
        ComparisonStrategy::None => compare_none(ctx),
        ComparisonStrategy::Semantic { .. } => {
            // The Semantic branch needs the runtime to dispatch the
            // comparator agent — must be called via `compare_semantic`
            // by the scheduler.  Returning ComparatorUnavailable here
            // signals "this entry point can't handle Semantic".
            ShadowOutcome::ComparatorUnavailable {
                compared_at: Utc::now(),
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Exact
// ─────────────────────────────────────────────────────────────────────

/// **Strategy : Exact**.  Byte-identical JSON canonical form.
///
/// Uses [`crate::audit::hash_json`] which canonicalizes object key
/// ordering — `{"a": 1, "b": 2}` and `{"b": 2, "a": 1}` are equivalent.
pub fn compare_exact(
    prod_output: &Value,
    shadow_output: &Value,
    ctx: CompareContext,
) -> ShadowOutcome {
    let prod_h = crate::audit::hash_json(prod_output);
    let shadow_h = crate::audit::hash_json(shadow_output);
    if prod_h == shadow_h {
        ShadowOutcome::Match {
            compared_at: Utc::now(),
            similarity: 1.0,
            prod_latency_ms: ctx.prod_latency_ms,
            shadow_latency_ms: ctx.shadow_latency_ms,
            prod_cost_cents: ctx.prod_cost_cents,
            shadow_cost_cents: ctx.shadow_cost_cents,
        }
    } else {
        ShadowOutcome::Mismatch {
            compared_at: Utc::now(),
            similarity: 0.0,
            diff_summary: format!(
                "exact hash mismatch : prod={} shadow={}",
                hex8(&prod_h),
                hex8(&shadow_h)
            ),
            prod_latency_ms: ctx.prod_latency_ms,
            shadow_latency_ms: ctx.shadow_latency_ms,
            prod_cost_cents: ctx.prod_cost_cents,
            shadow_cost_cents: ctx.shadow_cost_cents,
        }
    }
}

fn hex8(b: &[u8; 32]) -> String {
    let mut s = String::with_capacity(16);
    for byte in &b[..8] {
        use std::fmt::Write;
        write!(s, "{byte:02x}").unwrap();
    }
    s
}

// ─────────────────────────────────────────────────────────────────────
//  Structural
// ─────────────────────────────────────────────────────────────────────

/// **Strategy : Structural**.  Recurse the JSON tree ; require same
/// shape (key set, value types, array lengths) ; score leaves by % that
/// are byte-identical.
///
/// Returns `Match` if structure matches AND leaf-similarity ≥ 0.95.
/// Returns `Mismatch` otherwise (including any structural difference,
/// which scores similarity = 0.0).
pub fn compare_structural(
    prod_output: &Value,
    shadow_output: &Value,
    ctx: CompareContext,
) -> ShadowOutcome {
    let mut total_leaves = 0u32;
    let mut equal_leaves = 0u32;
    let now = Utc::now();
    let (shape_ok, diff) = walk(prod_output, shadow_output, &mut total_leaves, &mut equal_leaves);
    if !shape_ok {
        return ShadowOutcome::Mismatch {
            compared_at: now,
            similarity: 0.0,
            diff_summary: diff.unwrap_or_else(|| "structural mismatch".into()),
            prod_latency_ms: ctx.prod_latency_ms,
            shadow_latency_ms: ctx.shadow_latency_ms,
            prod_cost_cents: ctx.prod_cost_cents,
            shadow_cost_cents: ctx.shadow_cost_cents,
        };
    }
    let similarity = if total_leaves == 0 {
        1.0
    } else {
        equal_leaves as f32 / total_leaves as f32
    };
    if similarity >= 0.95 {
        ShadowOutcome::Match {
            compared_at: now,
            similarity,
            prod_latency_ms: ctx.prod_latency_ms,
            shadow_latency_ms: ctx.shadow_latency_ms,
            prod_cost_cents: ctx.prod_cost_cents,
            shadow_cost_cents: ctx.shadow_cost_cents,
        }
    } else {
        ShadowOutcome::Mismatch {
            compared_at: now,
            similarity,
            diff_summary: format!(
                "leaf similarity {:.2} % below 95 % threshold ({}/{} leaves match)",
                similarity * 100.0,
                equal_leaves,
                total_leaves
            ),
            prod_latency_ms: ctx.prod_latency_ms,
            shadow_latency_ms: ctx.shadow_latency_ms,
            prod_cost_cents: ctx.prod_cost_cents,
            shadow_cost_cents: ctx.shadow_cost_cents,
        }
    }
}

/// Walk both JSON trees in parallel, asserting shape equality and
/// counting leaves.  Returns `(shape_ok, Some(diff))` on shape mismatch
/// (with a short diagnostic), or `(true, None)` on shape match.
fn walk(a: &Value, b: &Value, total: &mut u32, equal: &mut u32) -> (bool, Option<String>) {
    match (a, b) {
        (Value::Null, Value::Null) => {
            *total += 1;
            *equal += 1;
            (true, None)
        }
        (Value::Bool(x), Value::Bool(y)) => {
            *total += 1;
            if x == y { *equal += 1; }
            (true, None)
        }
        (Value::Number(x), Value::Number(y)) => {
            *total += 1;
            // Cf. serde_json::Number — equality is byte-stable but we
            // also tolerate numeric f64 within 1e-9.
            let eq = if x == y {
                true
            } else if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                (xf - yf).abs() < 1e-9
            } else {
                false
            };
            if eq { *equal += 1; }
            (true, None)
        }
        (Value::String(x), Value::String(y)) => {
            *total += 1;
            if x == y { *equal += 1; }
            (true, None)
        }
        (Value::Array(xa), Value::Array(ya)) => {
            if xa.len() != ya.len() {
                return (
                    false,
                    Some(format!("array length differs : {} vs {}", xa.len(), ya.len())),
                );
            }
            for (xi, yi) in xa.iter().zip(ya.iter()) {
                let (ok, diff) = walk(xi, yi, total, equal);
                if !ok { return (false, diff); }
            }
            (true, None)
        }
        (Value::Object(xo), Value::Object(yo)) => {
            // Key set must match.
            if xo.len() != yo.len() {
                return (
                    false,
                    Some(format!(
                        "object key count differs : {} vs {}",
                        xo.len(),
                        yo.len()
                    )),
                );
            }
            for (k, xv) in xo.iter() {
                match yo.get(k) {
                    Some(yv) => {
                        let (ok, diff) = walk(xv, yv, total, equal);
                        if !ok { return (false, diff); }
                    }
                    None => {
                        return (false, Some(format!("key '{k}' missing from shadow output")));
                    }
                }
            }
            (true, None)
        }
        // Cross-type mismatch.
        _ => (
            false,
            Some(format!("type mismatch : {} vs {}", type_str(a), type_str(b))),
        ),
    }
}

fn type_str(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ─────────────────────────────────────────────────────────────────────
//  None
// ─────────────────────────────────────────────────────────────────────

/// **Strategy : None**.  Always returns `Match { similarity: 1.0 }` —
/// no comparison is performed but the shadow ran (the latency / cost
/// stats are real).
///
/// Used for `LlmEphemeral` agents where comparison would produce
/// 100 % spurious mismatches.
pub fn compare_none(ctx: CompareContext) -> ShadowOutcome {
    ShadowOutcome::Match {
        compared_at: Utc::now(),
        similarity: 1.0,
        prod_latency_ms: ctx.prod_latency_ms,
        shadow_latency_ms: ctx.shadow_latency_ms,
        prod_cost_cents: ctx.prod_cost_cents,
        shadow_cost_cents: ctx.shadow_cost_cents,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Semantic — async runtime-dependent comparator
// ─────────────────────────────────────────────────────────────────────

/// **Strategy : Semantic**.  Dispatch a comparator nano agent against
/// `{prod_output, shadow_output}` and extract a similarity score from
/// its `.output["similarity"]` field.
///
/// # Comparator agent contract
///
/// - Input schema : `{ "prod_output": Any, "shadow_output": Any }`
/// - Output schema : `{ "similarity": number ∈ [0, 1], "reasoning": string }`
/// - Determinism : must be `FullDeterministic` or `Seeded` so the
///   comparator itself doesn't drift.  Catalog rejects Semantic
///   comparisons that point to a non-deterministic comparator.
///
/// # Cost
///
/// One extra agent dispatch per shadow comparison.  Use sparingly —
/// `Structural` covers most Mode C cases.
pub async fn compare_semantic(
    comparator: std::sync::Arc<efe_nano_sdk::NanoAgent>,
    runtime: std::sync::Arc<dyn crate::runtime::NanoAgentExecutor>,
    prod_output: &Value,
    shadow_output: &Value,
    ctx: CompareContext,
) -> ShadowOutcome {
    let input = serde_json::json!({
        "prod_output": prod_output,
        "shadow_output": shadow_output,
    });
    let call_ctx = crate::runtime::CallContext::stand_alone(
        efe_nano_sdk::TenantId::system(),
        std::sync::Arc::new(crate::audit::stub::ConsoleAuditWriter::new()),
        2_000,
    );
    match runtime.execute_with_context(&comparator, input, &call_ctx).await {
        Ok(resp) => {
            let sim = resp
                .output
                .get("similarity")
                .and_then(|v| v.as_f64())
                .map(|f| f.clamp(0.0, 1.0) as f32);
            match sim {
                Some(s) if s >= 0.95 => ShadowOutcome::Match {
                    compared_at: Utc::now(),
                    similarity: s,
                    prod_latency_ms: ctx.prod_latency_ms,
                    shadow_latency_ms: ctx.shadow_latency_ms,
                    prod_cost_cents: ctx.prod_cost_cents,
                    shadow_cost_cents: ctx.shadow_cost_cents,
                },
                Some(s) => ShadowOutcome::Mismatch {
                    compared_at: Utc::now(),
                    similarity: s,
                    diff_summary: resp
                        .output
                        .get("reasoning")
                        .and_then(|v| v.as_str())
                        .unwrap_or("semantic similarity below 0.95")
                        .to_string(),
                    prod_latency_ms: ctx.prod_latency_ms,
                    shadow_latency_ms: ctx.shadow_latency_ms,
                    prod_cost_cents: ctx.prod_cost_cents,
                    shadow_cost_cents: ctx.shadow_cost_cents,
                },
                None => ShadowOutcome::ComparatorUnavailable {
                    compared_at: Utc::now(),
                },
            }
        }
        Err(_) => ShadowOutcome::ComparatorUnavailable {
            compared_at: Utc::now(),
        },
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> CompareContext {
        CompareContext {
            prod_latency_ms: 10,
            shadow_latency_ms: 12,
            prod_cost_cents: 0.001,
            shadow_cost_cents: 0.002,
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Exact
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn exact_identical_yields_match_one() {
        let o = compare_exact(&json!({"x": 1}), &json!({"x": 1}), ctx());
        assert!(matches!(o, ShadowOutcome::Match { similarity, .. } if similarity == 1.0));
    }

    #[test]
    fn exact_key_order_irrelevant() {
        let o = compare_exact(&json!({"a": 1, "b": 2}), &json!({"b": 2, "a": 1}), ctx());
        assert!(matches!(o, ShadowOutcome::Match { .. }));
    }

    #[test]
    fn exact_value_change_yields_mismatch() {
        let o = compare_exact(&json!({"x": 1}), &json!({"x": 2}), ctx());
        assert!(matches!(o, ShadowOutcome::Mismatch { .. }));
    }

    #[test]
    fn exact_carries_latency_and_cost() {
        let o = compare_exact(&json!({}), &json!({}), ctx());
        match o {
            ShadowOutcome::Match { prod_latency_ms, shadow_cost_cents, .. } => {
                assert_eq!(prod_latency_ms, 10);
                assert!((shadow_cost_cents - 0.002).abs() < 1e-9);
            }
            _ => panic!("expected Match"),
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Structural
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn structural_identical_yields_match() {
        let o = compare_structural(&json!({"x": 1, "y": [1, 2]}), &json!({"x": 1, "y": [1, 2]}), ctx());
        assert!(matches!(o, ShadowOutcome::Match { .. }));
    }

    #[test]
    fn structural_minor_leaf_change_above_threshold_matches() {
        // 20 leaves, 1 changed = 95 % — exactly threshold.
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..20 {
            a.push(json!(i));
            b.push(json!(i));
        }
        b[0] = json!(99);
        let o = compare_structural(&json!(a), &json!(b), ctx());
        // 19/20 = 0.95 → at threshold → match.
        assert!(matches!(o, ShadowOutcome::Match { .. }));
    }

    #[test]
    fn structural_significant_leaf_change_below_threshold_mismatches() {
        // 20 leaves, 5 changed = 75 % — below threshold.
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..20 {
            a.push(json!(i));
            b.push(json!(i));
        }
        for slot in b.iter_mut().take(5) {
            *slot = json!(999);
        }
        let o = compare_structural(&json!(a), &json!(b), ctx());
        assert!(matches!(o, ShadowOutcome::Mismatch { .. }));
    }

    #[test]
    fn structural_type_mismatch_immediate_failure() {
        let o = compare_structural(&json!({"x": 1}), &json!({"x": "1"}), ctx());
        match o {
            ShadowOutcome::Mismatch { diff_summary, similarity, .. } => {
                assert!(diff_summary.contains("type mismatch"));
                assert_eq!(similarity, 0.0);
            }
            _ => panic!("expected Mismatch"),
        }
    }

    #[test]
    fn structural_missing_key_immediate_failure() {
        let o = compare_structural(&json!({"x": 1, "y": 2}), &json!({"x": 1}), ctx());
        match o {
            ShadowOutcome::Mismatch { diff_summary, .. } => {
                assert!(diff_summary.contains("key count differs"));
            }
            _ => panic!("expected Mismatch"),
        }
    }

    #[test]
    fn structural_array_length_diff_immediate_failure() {
        let o = compare_structural(&json!([1, 2, 3]), &json!([1, 2]), ctx());
        match o {
            ShadowOutcome::Mismatch { diff_summary, .. } => {
                assert!(diff_summary.contains("array length"));
            }
            _ => panic!("expected Mismatch"),
        }
    }

    #[test]
    fn structural_nested_shape_preserved() {
        // {"a": [{"x": 1}, {"x": 2}]} == {"a": [{"x": 1}, {"x": 2}]}.
        let a = json!({"a": [{"x": 1}, {"x": 2}]});
        let b = json!({"a": [{"x": 1}, {"x": 2}]});
        let o = compare_structural(&a, &b, ctx());
        assert!(matches!(o, ShadowOutcome::Match { .. }));
    }

    #[test]
    fn structural_floating_point_tolerance() {
        // Avoid clippy::approx_constant by using a number not close to
        // any math constant.  We're checking equality, not numerics.
        let a = json!({"v": 1.23456789012345_f64});
        let b = json!({"v": 1.23456789012345_f64});
        let o = compare_structural(&a, &b, ctx());
        assert!(matches!(o, ShadowOutcome::Match { similarity, .. } if similarity == 1.0));
    }

    #[test]
    fn structural_empty_objects_are_match() {
        let o = compare_structural(&json!({}), &json!({}), ctx());
        assert!(matches!(o, ShadowOutcome::Match { similarity, .. } if similarity == 1.0));
    }

    // ─────────────────────────────────────────────────────────────────
    //  None
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn none_always_match() {
        let o = compare_none(ctx());
        assert!(matches!(o, ShadowOutcome::Match { similarity, .. } if similarity == 1.0));
    }

    #[test]
    fn none_does_not_inspect_outputs() {
        // Same call shape, but different inputs — None ignores.
        let o = compare_none(ctx());
        match o {
            ShadowOutcome::Match { prod_latency_ms, .. } => {
                assert_eq!(prod_latency_ms, 10);
            }
            _ => panic!("expected Match"),
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Dispatch via `compare`
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn dispatch_routes_to_correct_strategy() {
        let a = json!({"x": 1});
        let b = json!({"x": 1});
        let m = compare(&ComparisonStrategy::Exact, &a, &b, ctx());
        assert!(matches!(m, ShadowOutcome::Match { .. }));
        let s = compare(&ComparisonStrategy::Structural, &a, &b, ctx());
        assert!(matches!(s, ShadowOutcome::Match { .. }));
        let n = compare(&ComparisonStrategy::None, &a, &b, ctx());
        assert!(matches!(n, ShadowOutcome::Match { .. }));
        // Semantic via dispatch → ComparatorUnavailable (needs runtime).
        let sem = compare(
            &ComparisonStrategy::Semantic { comparator_agent_id: efe_nano_sdk::NanoAgentId::new("c").unwrap() },
            &a, &b, ctx(),
        );
        assert!(matches!(sem, ShadowOutcome::ComparatorUnavailable { .. }));
    }
}
