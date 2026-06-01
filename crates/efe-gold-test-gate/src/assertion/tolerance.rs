// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : `NumericTolerance` assertion strategy.
//!
//! ADR-1 : per-JSON-pointer ε comparison.  Fields not listed in
//! `fields` MUST match exactly (strict equality).  Listed fields
//! pass if `|actual - expected| ≤ ε`.

use std::collections::BTreeMap;

use efe_nano_sdk::{GoldCaseVerdict, GoldFieldDiff};

/// **N19** : evaluate numeric-tolerance.
///
/// Algorithm :
/// 1. For each field in `fields`, look up the value at the JSON
///    pointer in both `actual` and `expected`.
/// 2. If both are numbers, compare with the ε bound.
/// 3. If pointer missing on either side OR the value is not numeric,
///    mark as a failure (with `epsilon_used` set to the value, for
///    operator triage).
/// 4. For all other fields, fall back to strict equality.
pub fn numeric_tolerance(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    fields: &BTreeMap<String, f64>,
) -> GoldCaseVerdict {
    let mut diffs: Vec<GoldFieldDiff> = Vec::new();

    // First check the ε-bounded fields explicitly.
    for (pointer, epsilon) in fields {
        let a = actual.pointer(pointer);
        let e = expected.pointer(pointer);
        match (a, e) {
            (Some(a), Some(e)) => {
                let (Some(af), Some(ef)) = (json_as_f64(a), json_as_f64(e)) else {
                    diffs.push(GoldFieldDiff {
                        json_pointer: pointer.clone(),
                        actual: a.clone(),
                        expected: e.clone(),
                        epsilon_used: Some(*epsilon),
                    });
                    continue;
                };
                let delta = (af - ef).abs();
                if !delta.is_finite() || delta > *epsilon {
                    diffs.push(GoldFieldDiff {
                        json_pointer: pointer.clone(),
                        actual: a.clone(),
                        expected: e.clone(),
                        epsilon_used: Some(*epsilon),
                    });
                }
            }
            _ => diffs.push(GoldFieldDiff {
                json_pointer: pointer.clone(),
                actual: a.cloned().unwrap_or(serde_json::Value::Null),
                expected: e.cloned().unwrap_or(serde_json::Value::Null),
                epsilon_used: Some(*epsilon),
            }),
        }
    }

    // Strict equality on remaining fields : produce a JSON Value
    // copy with the tolerance-bounded pointers zeroed out so the
    // structural diff doesn't double-report them.
    let actual_stripped = strip_pointers(actual.clone(), fields.keys());
    let expected_stripped = strip_pointers(expected.clone(), fields.keys());
    if actual_stripped != expected_stripped {
        let extra = crate::assertion::diff_json(&actual_stripped, &expected_stripped, "");
        diffs.extend(extra);
    }

    if diffs.is_empty() {
        GoldCaseVerdict::Passed { similarity: None }
    } else {
        GoldCaseVerdict::Failed {
            reason: format!(
                "numeric tolerance failed at {} field{}",
                diffs.len(),
                if diffs.len() == 1 { "" } else { "s" }
            ),
            diff: diffs,
            similarity: None,
        }
    }
}

fn json_as_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// **N19 helper** : Replace the value at each `pointer` with `Null`
/// (so the structural diff path ignores it).
fn strip_pointers<'a>(
    mut value: serde_json::Value,
    pointers: impl Iterator<Item = &'a String>,
) -> serde_json::Value {
    for p in pointers {
        if let Some(v) = value.pointer_mut(p) {
            *v = serde_json::Value::Null;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(p: &str, eps: f64) -> BTreeMap<String, f64> {
        let mut m = BTreeMap::new();
        m.insert(p.into(), eps);
        m
    }

    #[test]
    fn within_epsilon_passes() {
        let a = serde_json::json!({"score": 0.51});
        let e = serde_json::json!({"score": 0.50});
        let v = numeric_tolerance(&a, &e, &fields("/score", 0.05));
        assert!(v.is_passed());
    }

    #[test]
    fn outside_epsilon_fails() {
        let a = serde_json::json!({"score": 0.7});
        let e = serde_json::json!({"score": 0.5});
        let v = numeric_tolerance(&a, &e, &fields("/score", 0.01));
        assert!(!v.is_passed());
    }

    #[test]
    fn missing_pointer_fails() {
        let a = serde_json::json!({});
        let e = serde_json::json!({"score": 0.5});
        let v = numeric_tolerance(&a, &e, &fields("/score", 0.05));
        assert!(!v.is_passed());
    }

    #[test]
    fn non_numeric_value_fails() {
        let a = serde_json::json!({"score": "high"});
        let e = serde_json::json!({"score": 0.5});
        let v = numeric_tolerance(&a, &e, &fields("/score", 0.05));
        assert!(!v.is_passed());
    }

    #[test]
    fn non_listed_fields_must_match_exactly() {
        let a = serde_json::json!({"score": 0.5, "label": "A"});
        let e = serde_json::json!({"score": 0.5, "label": "B"});
        let v = numeric_tolerance(&a, &e, &fields("/score", 0.05));
        assert!(!v.is_passed());
    }

    #[test]
    fn epsilon_zero_demands_exact_match_on_floats() {
        let a = serde_json::json!({"score": 0.5000001});
        let e = serde_json::json!({"score": 0.5});
        let v = numeric_tolerance(&a, &e, &fields("/score", 0.0));
        assert!(!v.is_passed());
    }

    #[test]
    fn nested_pointer_supported() {
        let a = serde_json::json!({"nested": {"score": 0.51}});
        let e = serde_json::json!({"nested": {"score": 0.50}});
        let v = numeric_tolerance(&a, &e, &fields("/nested/score", 0.05));
        assert!(v.is_passed());
    }

    #[test]
    fn multi_field_with_one_failure_reports_only_failure() {
        let mut m = BTreeMap::new();
        m.insert("/a".into(), 0.01);
        m.insert("/b".into(), 0.5);
        let actual = serde_json::json!({"a": 0.5, "b": 0.0});
        let expected = serde_json::json!({"a": 0.5, "b": 1.0});
        let v = numeric_tolerance(&actual, &expected, &m);
        match v {
            GoldCaseVerdict::Failed { diff, .. } => {
                assert_eq!(diff.len(), 1);
                assert_eq!(diff[0].json_pointer, "/b");
            }
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn diff_carries_epsilon_used() {
        let a = serde_json::json!({"score": 0.7});
        let e = serde_json::json!({"score": 0.5});
        let v = numeric_tolerance(&a, &e, &fields("/score", 0.01));
        match v {
            GoldCaseVerdict::Failed { diff, .. } => {
                assert_eq!(diff[0].epsilon_used, Some(0.01));
            }
            _ => panic!("expected Failed"),
        }
    }
}
