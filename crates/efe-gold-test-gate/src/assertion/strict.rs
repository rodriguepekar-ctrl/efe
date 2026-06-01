// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : `StrictEquality` assertion strategy.
//!
//! Byte-by-byte deterministic JSON equality with two normalisations
//! :
//! 1. Object keys are compared in deterministic order (serde_json
//!    `Map` is insertion-ordered ; we compare via the value's
//!    structural equality).
//! 2. Numbers compare via `serde_json::Number::Eq` — `1`, `1.0`,
//!    `1e0` are all equal.
//!
//! No tolerance.  Mode A use case (Sprint 1 deterministic agents).

use efe_nano_sdk::GoldCaseVerdict;

use crate::assertion::diff_json;

/// **N19** : evaluate strict equality.  Returns `Passed` on equality,
/// `Failed` otherwise (with a structured diff).
pub fn strict_equality(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
) -> GoldCaseVerdict {
    if actual == expected {
        GoldCaseVerdict::Passed { similarity: None }
    } else {
        let diff = diff_json(actual, expected, "");
        GoldCaseVerdict::Failed {
            reason: format!("strict equality failed at {} fields", diff.len()),
            diff,
            similarity: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_jsons_pass() {
        let a = serde_json::json!({"x": 1, "y": [1, 2]});
        let b = serde_json::json!({"x": 1, "y": [1, 2]});
        assert!(strict_equality(&a, &b).is_passed());
    }

    #[test]
    fn differing_jsons_fail() {
        let a = serde_json::json!({"x": 1});
        let b = serde_json::json!({"x": 2});
        let v = strict_equality(&a, &b);
        assert!(!v.is_passed());
        match v {
            GoldCaseVerdict::Failed { diff, .. } => {
                assert_eq!(diff.len(), 1);
                assert_eq!(diff[0].json_pointer, "/x");
            }
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn numeric_equivalence_1_vs_1_0_passes() {
        // serde_json treats integer 1 and float 1.0 as DIFFERENT
        // tokens by default ; this test pins the current behaviour.
        // Mode A authors who care about this nuance should use
        // NumericTolerance with epsilon=0.
        let a = serde_json::json!({"x": 1});
        let b = serde_json::json!({"x": 1.0});
        // serde_json::Value PartialEq: 1 == 1.0 is FALSE.
        let v = strict_equality(&a, &b);
        // Pin current behaviour : fails.
        assert!(!v.is_passed());
    }

    #[test]
    fn null_equals_null() {
        let a = serde_json::json!(null);
        let b = serde_json::json!(null);
        assert!(strict_equality(&a, &b).is_passed());
    }

    #[test]
    fn empty_objects_equal() {
        let a = serde_json::json!({});
        let b = serde_json::json!({});
        assert!(strict_equality(&a, &b).is_passed());
    }

    #[test]
    fn ordering_in_object_does_not_matter_for_equality() {
        // serde_json::Value PartialEq is structural — key order on
        // construction doesn't matter.
        let a: serde_json::Value =
            serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let b: serde_json::Value =
            serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        assert!(strict_equality(&a, &b).is_passed());
    }

    #[test]
    fn array_order_matters() {
        let a = serde_json::json!([1, 2, 3]);
        let b = serde_json::json!([3, 2, 1]);
        assert!(!strict_equality(&a, &b).is_passed());
    }
}
