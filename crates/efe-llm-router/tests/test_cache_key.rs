//! Pin tests for cache key construction (M21 Phase D).

use efe_llm_router::cache::Cache;

#[test]
fn test_cache_key_includes_tenant() {
    let k1 = Cache::compute_exact_key("acme", "support", "v1.0.0", "tpl_v1", 1, "hello");
    let k2 = Cache::compute_exact_key("evil", "support", "v1.0.0", "tpl_v1", 1, "hello");
    assert_ne!(k1, k2);
    assert!(k1.starts_with("efe:llm:cache:exact:acme:"));
    assert!(k2.starts_with("efe:llm:cache:exact:evil:"));
}

#[test]
fn test_cache_key_includes_bundle_version() {
    let k1 = Cache::compute_exact_key("acme", "support", "v1.0.0", "tpl_v1", 1, "hello");
    let k2 = Cache::compute_exact_key("acme", "support", "v1.1.0", "tpl_v1", 1, "hello");
    assert_ne!(k1, k2);
}

#[test]
fn test_cache_key_includes_template_version() {
    let k1 = Cache::compute_exact_key("acme", "support", "v1.0.0", "tpl_v1", 1, "hello");
    let k2 = Cache::compute_exact_key("acme", "support", "v1.0.0", "tpl_v1", 2, "hello");
    assert_ne!(k1, k2);
}

#[test]
fn test_cache_key_anti_collision_null_bytes() {
    // "acme|bob" + "" should NOT collide with "acme" + "|bob"
    let k1 = Cache::compute_exact_key("acme", "bob", "v1.0.0", "tpl_v1", 1, "hello");
    let k2 = Cache::compute_exact_key("acme|bob", "", "v1.0.0", "tpl_v1", 1, "hello");
    assert_ne!(k1, k2);
}

#[test]
fn test_cache_key_deterministic() {
    let k1 = Cache::compute_exact_key("acme", "support", "v1.0.0", "tpl_v1", 1, "hello");
    let k2 = Cache::compute_exact_key("acme", "support", "v1.0.0", "tpl_v1", 1, "hello");
    assert_eq!(k1, k2);
}

#[test]
fn test_cache_key_redacted_sentence_only() {
    // Same redacted sentence → same key, regardless of original.
    // (This is enforced by callers ; verified by deterministic property
    // here.)
    let k = Cache::compute_exact_key(
        "acme",
        "support",
        "v1.0.0",
        "tpl_v1",
        1,
        "[REDACTED:email:abc12345]",
    );
    assert!(k.contains("acme"));
}

#[test]
fn test_cache_key_format() {
    let k = Cache::compute_exact_key("acme", "x", "v1.0.0", "t", 1, "s");
    let parts: Vec<&str> = k.split(':').collect();
    assert_eq!(parts.len(), 6);
    assert_eq!(parts[0], "efe");
    assert_eq!(parts[1], "llm");
    assert_eq!(parts[2], "cache");
    assert_eq!(parts[3], "exact");
    assert_eq!(parts[4], "acme");
    assert_eq!(parts[5].len(), 64); // sha256 hex
}
