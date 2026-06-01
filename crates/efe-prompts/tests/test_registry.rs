//! Pin tests for PromptRegistry (M21 Phase A).

use efe_prompts::{AbAssignmentTable, PromptRegistry, RegistryError};

const TPL_V1: &str = r#"---
name: foo_v1
version: 1
provider_compat: [anthropic]
eval_pass: true
---

# System
sys v1 with $catalog
# User
usr v1 with $sentence
"#;

const TPL_V2: &str = r#"---
name: foo_v2
version: 2
provider_compat: [anthropic, openai]
eval_pass: true
---

# System
sys v2
# User
usr v2 with $sentence
"#;

#[test]
fn test_empty_registry_returns_not_found() {
    let r = PromptRegistry::empty();
    let err = r.lookup("foo_v1", "tenant1").unwrap_err();
    assert!(matches!(err, RegistryError::NotFound(_)));
}

#[test]
fn test_from_static_loads_templates() {
    let r = PromptRegistry::from_static(vec![("foo_v1", TPL_V1)]).unwrap();
    let t = r.lookup("foo_v1", "tenant1").unwrap();
    assert_eq!(t.name, "foo_v1");
    assert_eq!(t.version, 1);
}

#[test]
fn test_lookup_with_ab_assignment() {
    let r = PromptRegistry::from_static(vec![("foo_v1", TPL_V1), ("foo_v2", TPL_V2)]).unwrap();
    // Set A/B : 100% of tenants → v2
    r.set_ab_table(AbAssignmentTable::default().add_cohort("foo_v1", "foo_v2", 1.0));
    let t = r.lookup("foo_v1", "tenant1").unwrap();
    assert_eq!(t.name, "foo_v2");
    assert_eq!(t.version, 2);
}

#[test]
fn test_lookup_falls_back_to_base_if_assigned_missing() {
    // Assign to "foo_v3" (not in registry) → fallback to foo_v1.
    let r = PromptRegistry::from_static(vec![("foo_v1", TPL_V1)]).unwrap();
    r.set_ab_table(AbAssignmentTable::default().add_cohort("foo_v1", "foo_v3_unknown", 1.0));
    let t = r.lookup("foo_v1", "tenant1").unwrap();
    assert_eq!(t.name, "foo_v1");
}

#[test]
fn test_list_returns_sorted_names() {
    let r = PromptRegistry::from_static(vec![("foo_v1", TPL_V1), ("foo_v2", TPL_V2)]).unwrap();
    let names = r.list();
    assert_eq!(names, vec!["foo_v1", "foo_v2"]);
}

#[test]
fn test_from_static_invalid_template_errors_clearly() {
    let bad = "not a template";
    let err = PromptRegistry::from_static(vec![("bad", bad)]).unwrap_err();
    assert!(matches!(err, RegistryError::ParseError { .. }));
}

#[test]
fn test_from_dir_loads_md_files() {
    use std::fs;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("foo_v1.md"), TPL_V1).unwrap();
    fs::write(dir.path().join("foo_v2.md"), TPL_V2).unwrap();
    fs::write(dir.path().join("readme.txt"), "ignored").unwrap();

    let r = PromptRegistry::from_dir(dir.path()).unwrap();
    let names = r.list();
    assert_eq!(names, vec!["foo_v1", "foo_v2"]);
}
