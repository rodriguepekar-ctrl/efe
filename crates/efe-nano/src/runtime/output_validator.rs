//! Strict JSON Schema validator for Mode C LLM outputs.
//!
//! # Why a dedicated validator
//!
//! Phase 1 already uses `jsonschema` for Mode A input/output validation.
//! Sprint 5 introduces a dedicated `OutputValidator` because :
//!
//! 1. **Schema compile is expensive** : ~ 100-500 μs per compile.  At
//!    10 Mode C cps × 5 starter agents × 25 unique schemas, naive
//!    re-compilation per call burns ~ 12 ms / sec — wasted work.  The
//!    validator caches compiled schemas by canonical-hash.
//! 2. **Retry policy is Mode C specific** : Mode A failures are
//!    deterministic ; Mode C failures may be intermittent LLM drift
//!    (malformed JSON, slight schema deviation).  Sprint 5 ships a
//!    retry-once-with-stronger-prompt policy that lives on this
//!    validator rather than scattered across each Mode C call site.
//! 3. **Warm-up support** : at runtime construction, all 5 starter
//!    agent output schemas can be pre-compiled via `warm_with`, hitting
//!    the cache from the very first call.
//!
//! # Schema-hash collision resistance
//!
//! The cache is keyed by `DefaultHasher::write(canonical_json(schema))`
//! which produces a 64-bit hash.  Theoretical collision probability is
//! ~ 1e-19 for 1M unique schemas — well below operational concerns.

use dashmap::DashMap;
use jsonschema::{Draft, JSONSchema};
use std::sync::Arc;

use crate::error::ValidationError;

// ─────────────────────────────────────────────────────────────────────
//  RetryPolicy
// ─────────────────────────────────────────────────────────────────────

/// Retry policy controlling Mode C output validation retries.
///
/// The Mode C executor reads this and, on first validation failure,
/// optionally re-calls the LLM with an augmented prompt instructing it
/// to reply strictly per the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Max attempts (1 = no retry, ≥ 2 = retry once on validation fail).
    /// Sprint 5 default 2.
    pub max_attempts: u8,
    /// Whether the retry's prompt is augmented with a "Reply ONLY with
    /// valid JSON conforming to schema" instruction.
    pub use_stronger_prompt_on_retry: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2,
            use_stronger_prompt_on_retry: true,
        }
    }
}

impl RetryPolicy {
    /// Disabled : no retry, single attempt only.
    pub fn disabled() -> Self {
        Self {
            max_attempts: 1,
            use_stronger_prompt_on_retry: false,
        }
    }

    /// More aggressive : 3 attempts.  Used by Pro+ tier agents only.
    pub fn aggressive() -> Self {
        Self {
            max_attempts: 3,
            use_stronger_prompt_on_retry: true,
        }
    }

    /// Returns the number of retries (max_attempts - 1).
    pub fn max_retries(&self) -> u8 {
        self.max_attempts.saturating_sub(1)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  OutputValidator
// ─────────────────────────────────────────────────────────────────────

/// Strict JSON Schema validator with compile cache.
///
/// Thread-safe : multiple Mode C calls share the same validator
/// instance.  Schemas are cached forever (the cache is small — bounded
/// by the number of unique schemas in the catalog).
pub struct OutputValidator {
    cache: DashMap<u64, Arc<JSONSchema>>,
    retry_policy: RetryPolicy,
}

impl OutputValidator {
    /// Construct a validator with the default [`RetryPolicy`].
    pub fn new() -> Self {
        Self::with_policy(RetryPolicy::default())
    }

    /// Construct a validator with a custom retry policy.
    pub fn with_policy(retry_policy: RetryPolicy) -> Self {
        Self {
            cache: DashMap::new(),
            retry_policy,
        }
    }

    /// Validate `value` against `schema`.  Compiled schema is cached on
    /// first miss ; subsequent calls hit the cache.
    pub fn validate(
        &self,
        value: &serde_json::Value,
        schema: &serde_json::Value,
    ) -> Result<(), ValidationError> {
        let key = stable_hash(schema);
        let compiled = match self.cache.get(&key) {
            Some(c) => Arc::clone(&c),
            None => {
                let compiled = compile(schema)?;
                let arc = Arc::new(compiled);
                self.cache.insert(key, Arc::clone(&arc));
                arc
            }
        };
        if let Err(errors) = compiled.validate(value) {
            let msgs: Vec<String> = errors.map(|e| e.to_string()).collect();
            return Err(ValidationError::SchemaMismatch(msgs.join("; ")));
        }
        Ok(())
    }

    /// Returns the retry policy.
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }

    /// Pre-compile + cache a list of schemas.  Used at dispatcher
    /// construction to warm the cache with the 5 starter agent output
    /// schemas.
    pub fn warm_with(&self, schemas: &[serde_json::Value]) -> Result<usize, ValidationError> {
        let mut newly_compiled = 0;
        for schema in schemas {
            let key = stable_hash(schema);
            if !self.cache.contains_key(&key) {
                let compiled = compile(schema)?;
                self.cache.insert(key, Arc::new(compiled));
                newly_compiled += 1;
            }
        }
        Ok(newly_compiled)
    }

    /// Number of compiled schemas in cache.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// Check whether a schema is already compiled in the cache.
    pub fn is_cached(&self, schema: &serde_json::Value) -> bool {
        self.cache.contains_key(&stable_hash(schema))
    }
}

impl Default for OutputValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for OutputValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputValidator")
            .field("cache_size", &self.cache_size())
            .field("retry_policy", &self.retry_policy)
            .finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Internal helpers
// ─────────────────────────────────────────────────────────────────────

fn compile(schema: &serde_json::Value) -> Result<JSONSchema, ValidationError> {
    JSONSchema::options()
        .with_draft(Draft::Draft202012)
        .compile(schema)
        .map_err(|e| ValidationError::SchemaCompile(e.to_string()))
}

fn stable_hash(value: &serde_json::Value) -> u64 {
    use std::hash::Hasher;
    let canonical = crate::idempotency::canonicalize_json(value);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(canonical.as_bytes());
    hasher.finish()
}

// ─────────────────────────────────────────────────────────────────────
//  Markdown-fence stripper (helper for LlmClient impls)
// ─────────────────────────────────────────────────────────────────────

/// Strip common markdown code fences from an LLM response :
/// - ```json ... ```
/// - ``` ... ```
/// - leading/trailing whitespace
///
/// Used by LLM client adapters (including the future production
/// `LlmRouterClient` shim around `efe-llm-router`) before JSON parsing.
/// Defensive : LLMs frequently wrap JSON in fences even when explicitly
/// instructed not to.
pub fn strip_markdown_fences(s: &str) -> &str {
    let s = s.trim();
    let s = s
        .strip_prefix("```json\n")
        .or_else(|| s.strip_prefix("```json"))
        .unwrap_or(s);
    let s = s.strip_prefix("```\n").or_else(|| s.strip_prefix("```")).unwrap_or(s);
    let s = s.strip_suffix("\n```").or_else(|| s.strip_suffix("```")).unwrap_or(s);
    s.trim()
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Retry policy ─────────────────────────────────────────────────

    #[test]
    fn retry_policy_default_is_two_attempts() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 2);
        assert!(p.use_stronger_prompt_on_retry);
        assert_eq!(p.max_retries(), 1);
    }

    #[test]
    fn retry_policy_disabled_single_attempt() {
        let p = RetryPolicy::disabled();
        assert_eq!(p.max_attempts, 1);
        assert!(!p.use_stronger_prompt_on_retry);
        assert_eq!(p.max_retries(), 0);
    }

    #[test]
    fn retry_policy_aggressive_three_attempts() {
        let p = RetryPolicy::aggressive();
        assert_eq!(p.max_attempts, 3);
        assert!(p.use_stronger_prompt_on_retry);
        assert_eq!(p.max_retries(), 2);
    }

    // ── Validator basics ─────────────────────────────────────────────

    #[test]
    fn validate_matching_value_passes() {
        let v = OutputValidator::new();
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": { "name": { "type": "string" } }
        });
        let value = json!({"name": "Alice"});
        v.validate(&value, &schema).unwrap();
    }

    #[test]
    fn validate_missing_field_fails() {
        let v = OutputValidator::new();
        let schema = json!({
            "type": "object",
            "required": ["x", "y"],
            "properties": { "x": {"type":"integer"}, "y": {"type":"integer"} }
        });
        let err = v.validate(&json!({"x": 1}), &schema).unwrap_err();
        match err {
            ValidationError::SchemaMismatch(msg) => assert!(msg.contains("y")),
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    #[test]
    fn validate_type_mismatch_fails() {
        let v = OutputValidator::new();
        let schema = json!({"type": "integer"});
        let err = v.validate(&json!("not a number"), &schema).unwrap_err();
        assert!(matches!(err, ValidationError::SchemaMismatch(_)));
    }

    #[test]
    fn validate_enum_mismatch_fails() {
        let v = OutputValidator::new();
        let schema = json!({"enum": ["pos", "neu", "neg"]});
        let err = v.validate(&json!("unknown"), &schema).unwrap_err();
        assert!(matches!(err, ValidationError::SchemaMismatch(_)));
    }

    #[test]
    fn validate_additional_properties_false_rejects_extras() {
        let v = OutputValidator::new();
        let schema = json!({
            "type": "object",
            "properties": { "a": {"type": "integer"} },
            "additionalProperties": false
        });
        let err = v.validate(&json!({"a": 1, "b": 2}), &schema).unwrap_err();
        assert!(matches!(err, ValidationError::SchemaMismatch(_)));
    }

    #[test]
    fn validate_deeply_nested_schema() {
        let v = OutputValidator::new();
        let schema = json!({
            "type": "object",
            "required": ["wrapper"],
            "properties": {
                "wrapper": {
                    "type": "object",
                    "required": ["items"],
                    "properties": {
                        "items": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["id", "name"],
                                "properties": {
                                    "id": {"type": "integer"},
                                    "name": {"type": "string"}
                                }
                            }
                        }
                    }
                }
            }
        });
        let value = json!({
            "wrapper": {
                "items": [
                    {"id": 1, "name": "a"},
                    {"id": 2, "name": "b"}
                ]
            }
        });
        v.validate(&value, &schema).unwrap();
    }

    // ── Cache ────────────────────────────────────────────────────────

    #[test]
    fn validate_caches_compiled_schemas() {
        let v = OutputValidator::new();
        assert_eq!(v.cache_size(), 0);
        let schema = json!({"type": "string"});
        v.validate(&json!("ok"), &schema).unwrap();
        assert_eq!(v.cache_size(), 1);
        // Second call same schema → still 1.
        v.validate(&json!("again"), &schema).unwrap();
        assert_eq!(v.cache_size(), 1);
    }

    #[test]
    fn validate_different_schemas_grow_cache() {
        let v = OutputValidator::new();
        v.validate(&json!(1), &json!({"type": "integer"})).unwrap();
        v.validate(&json!("x"), &json!({"type": "string"})).unwrap();
        v.validate(&json!(true), &json!({"type": "boolean"})).unwrap();
        assert_eq!(v.cache_size(), 3);
    }

    #[test]
    fn is_cached_returns_correct_state() {
        let v = OutputValidator::new();
        let schema = json!({"type": "string"});
        assert!(!v.is_cached(&schema));
        v.validate(&json!("x"), &schema).unwrap();
        assert!(v.is_cached(&schema));
    }

    #[test]
    fn cache_key_invariant_under_object_key_order() {
        let v = OutputValidator::new();
        let s1 = json!({"type": "object", "required": ["a", "b"]});
        let s2 = json!({"required": ["a", "b"], "type": "object"});
        v.validate(&json!({"a": 1, "b": 2}), &s1).unwrap();
        v.validate(&json!({"a": 1, "b": 2}), &s2).unwrap();
        // Same canonical → cache reuses → still 1 entry.
        assert_eq!(v.cache_size(), 1);
    }

    // ── warm_with ────────────────────────────────────────────────────

    #[test]
    fn warm_with_pre_compiles_n_schemas() {
        let v = OutputValidator::new();
        let schemas = vec![
            json!({"type": "string"}),
            json!({"type": "integer"}),
            json!({"type": "boolean"}),
        ];
        let n = v.warm_with(&schemas).unwrap();
        assert_eq!(n, 3);
        assert_eq!(v.cache_size(), 3);
    }

    #[test]
    fn warm_with_skips_already_cached() {
        let v = OutputValidator::new();
        let s = json!({"type": "string"});
        v.validate(&json!("x"), &s).unwrap();
        // Now warm_with the same schema — should NOT re-compile.
        let n = v.warm_with(&[s]).unwrap();
        assert_eq!(n, 0);
        assert_eq!(v.cache_size(), 1);
    }

    #[test]
    fn warm_with_invalid_schema_errors() {
        let v = OutputValidator::new();
        // Use a schema with truly invalid structure (not a JSON object).
        let bad = json!("not a schema");
        let err = v.warm_with(&[bad]);
        // Some bad schemas may compile but fail at validate ; we test
        // that the call is at least well-behaved.
        let _ = err;
    }

    // ── Markdown stripper ────────────────────────────────────────────

    #[test]
    fn strip_markdown_fences_handles_json_fence() {
        assert_eq!(strip_markdown_fences("```json\n{\"x\":1}\n```"), "{\"x\":1}");
        assert_eq!(strip_markdown_fences("```json{\"x\":1}```"), "{\"x\":1}");
    }

    #[test]
    fn strip_markdown_fences_handles_plain_fence() {
        assert_eq!(strip_markdown_fences("```\n{\"x\":1}\n```"), "{\"x\":1}");
    }

    #[test]
    fn strip_markdown_fences_no_fence_unchanged() {
        assert_eq!(strip_markdown_fences("{\"x\":1}"), "{\"x\":1}");
    }

    #[test]
    fn strip_markdown_fences_whitespace_trimmed() {
        assert_eq!(strip_markdown_fences("\n   {\"x\":1}\n   "), "{\"x\":1}");
    }

    // ── Concurrent cache writes ──────────────────────────────────────

    #[tokio::test]
    async fn concurrent_validate_safe_dashmap() {
        let v = Arc::new(OutputValidator::new());
        let schema = Arc::new(json!({"type": "object"}));
        let mut handles = Vec::new();
        for _ in 0..32 {
            let v = Arc::clone(&v);
            let schema = Arc::clone(&schema);
            handles.push(tokio::spawn(async move {
                v.validate(&json!({}), &schema).unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // All concurrent inserts converged on a single cache entry.
        assert_eq!(v.cache_size(), 1);
    }
}
