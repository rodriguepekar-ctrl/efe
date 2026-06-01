//! Pin tests for AbAssignmentTable (M21 Phase A).

use efe_prompts::{AbAssignmentTable, AbCohort};

#[test]
fn test_empty_table_returns_base_id() {
    let table = AbAssignmentTable::default();
    assert_eq!(table.resolve("foo_v1", "tenant1"), "foo_v1");
    assert_eq!(table.resolve("foo_v1", "tenant2"), "foo_v1");
}

#[test]
fn test_assignment_deterministic_per_tenant() {
    let table = AbAssignmentTable::default().add_cohort("foo_v1", "foo_v2", 0.5);
    // Multiple invocations for same (base, tenant) → same answer.
    let r1 = table.resolve("foo_v1", "tenantA");
    let r2 = table.resolve("foo_v1", "tenantA");
    let r3 = table.resolve("foo_v1", "tenantA");
    assert_eq!(r1, r2);
    assert_eq!(r2, r3);
    assert!(r1 == "foo_v1" || r1 == "foo_v2");
}

#[test]
fn test_50_50_split_within_5pp() {
    // With 1 cohort weight=0.5, ~50% of tenants should land on v2.
    let table = AbAssignmentTable::default().add_cohort("foo_v1", "foo_v2", 0.5);
    let mut v2_count = 0;
    let n = 1_000;
    for i in 0..n {
        let tenant = format!("tenant_{}", i);
        if table.resolve("foo_v1", &tenant) == "foo_v2" {
            v2_count += 1;
        }
    }
    let pct = (v2_count as f64) / (n as f64);
    assert!(
        (0.45..=0.55).contains(&pct),
        "expected ~50% v2 cohort, got {pct}"
    );
}

#[test]
fn test_cohort_zero_weight_never_assigned() {
    let table = AbAssignmentTable::default().add_cohort("foo_v1", "foo_v2", 0.0);
    for i in 0..100 {
        let tenant = format!("t_{}", i);
        assert_eq!(table.resolve("foo_v1", &tenant), "foo_v1");
    }
}

#[test]
fn test_cohort_full_weight_always_assigned() {
    let table = AbAssignmentTable::default().add_cohort("foo_v1", "foo_v2", 1.0);
    for i in 0..100 {
        let tenant = format!("t_{}", i);
        assert_eq!(table.resolve("foo_v1", &tenant), "foo_v2");
    }
}

#[test]
fn test_unrelated_base_id_passthrough() {
    let table = AbAssignmentTable::default().add_cohort("foo_v1", "foo_v2", 1.0);
    // Base id "bar_v1" has no assignment → passthrough.
    assert_eq!(table.resolve("bar_v1", "tenantA"), "bar_v1");
}

#[test]
fn test_serializable_roundtrip() {
    let table = AbAssignmentTable::default()
        .add_cohort("foo_v1", "foo_v2", 0.3)
        .add_cohort("bar_v1", "bar_v2", 0.7);
    let json = serde_json::to_string(&table).unwrap();
    let restored: AbAssignmentTable = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.resolve("foo_v1", "fixed_tenant"),
        table.resolve("foo_v1", "fixed_tenant")
    );
}

#[test]
fn test_multiple_cohorts_cumulative_weight() {
    // 30% v2 + 30% v3 = 60% non-v1 ; 40% stay on v1.
    let mut t = AbAssignmentTable::default();
    t.assignments.insert(
        "foo_v1".to_string(),
        vec![
            AbCohort {
                variant: "foo_v2".into(),
                weight: 0.3,
            },
            AbCohort {
                variant: "foo_v3".into(),
                weight: 0.3,
            },
        ],
    );
    let mut v1 = 0;
    let mut v2 = 0;
    let mut v3 = 0;
    let n = 1_000;
    for i in 0..n {
        let tenant = format!("t_{}", i);
        match t.resolve("foo_v1", &tenant).as_str() {
            "foo_v1" => v1 += 1,
            "foo_v2" => v2 += 1,
            "foo_v3" => v3 += 1,
            _ => panic!("unexpected variant"),
        }
    }
    let v1_pct = v1 as f64 / n as f64;
    let v2_pct = v2 as f64 / n as f64;
    let v3_pct = v3 as f64 / n as f64;
    assert!((0.35..=0.45).contains(&v1_pct), "v1 pct {v1_pct}");
    assert!((0.25..=0.35).contains(&v2_pct), "v2 pct {v2_pct}");
    assert!((0.25..=0.35).contains(&v3_pct), "v3 pct {v3_pct}");
}
