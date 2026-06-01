// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21 + G11** : Schema validator — Mode A.
//!
//! Validates a JSON value against a schema constraint set (subset
//! of JSON Schema 2020-12).  Sprint 13 supports :
//! - `type` : `"object"`, `"array"`, `"string"`, `"number"`,
//!   `"boolean"`, `"null"`
//! - `required` : `Vec<String>`
//! - `properties` : `Map<String, Schema>` (recursive)
//! - `minLength` / `maxLength` for strings
//! - `minimum` / `maximum` for numbers
//!
//! Sprint 14 plugs the full `jsonschema` crate (network-aware
//! refs, format validation).
//!
//! **Determinism** : same `(value, schema)` → same verdict.

use async_trait::async_trait;
use efe_nano_sdk::AgentId;
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SchemaValidator {
    schema: serde_json::Value,
}

impl SchemaValidator {
    pub const AGENT_ID: &'static str = "schema-validator";

    pub fn new(schema: serde_json::Value) -> Self {
        Self { schema }
    }
}

/// **N21** : the verdict written to `/schema_valid`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaVerdict {
    pub valid: bool,
    pub errors: Vec<String>,
}

/// **N21** : pure validation.  Returns `Ok(())` if `value` matches
/// `schema` ; `Err(Vec<String>)` with diagnostic messages otherwise.
pub fn validate(
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<(), Vec<String>> {
    let mut errors: Vec<String> = Vec::new();
    validate_inner(value, schema, "", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_inner(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    // type check
    if let Some(t) = schema.get("type").and_then(|t| t.as_str()) {
        if !matches_type(value, t) {
            errors.push(format!(
                "{} : expected type {}, got {}",
                if path.is_empty() { "$" } else { path },
                t,
                json_type_label(value)
            ));
            return;
        }
    }

    // string constraints
    if let Some(s) = value.as_str() {
        if let Some(min) = schema.get("minLength").and_then(|m| m.as_u64()) {
            if (s.chars().count() as u64) < min {
                errors.push(format!(
                    "{} : string shorter than minLength {}",
                    if path.is_empty() { "$" } else { path },
                    min
                ));
            }
        }
        if let Some(max) = schema.get("maxLength").and_then(|m| m.as_u64()) {
            if (s.chars().count() as u64) > max {
                errors.push(format!(
                    "{} : string longer than maxLength {}",
                    if path.is_empty() { "$" } else { path },
                    max
                ));
            }
        }
    }

    // number constraints
    if let Some(n) = value.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(|m| m.as_f64()) {
            if n < min {
                errors.push(format!(
                    "{} : value {n} < minimum {min}",
                    if path.is_empty() { "$" } else { path }
                ));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(|m| m.as_f64()) {
            if n > max {
                errors.push(format!(
                    "{} : value {n} > maximum {max}",
                    if path.is_empty() { "$" } else { path }
                ));
            }
        }
    }

    // object : required + properties
    if let Some(obj) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for r in required {
                if let Some(field) = r.as_str() {
                    if !obj.contains_key(field) {
                        errors.push(format!(
                            "{} : missing required field {field:?}",
                            if path.is_empty() { "$" } else { path }
                        ));
                    }
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
            for (k, sub_schema) in properties {
                if let Some(sub_value) = obj.get(k) {
                    let new_path = format!("{path}/{k}");
                    validate_inner(sub_value, sub_schema, &new_path, errors);
                }
            }
        }
    }
}

fn matches_type(value: &serde_json::Value, t: &str) -> bool {
    match (value, t) {
        (serde_json::Value::Null, "null") => true,
        (serde_json::Value::Bool(_), "boolean") => true,
        (serde_json::Value::Number(_), "number") => true,
        (serde_json::Value::Number(n), "integer") => n.is_i64() || n.is_u64(),
        (serde_json::Value::String(_), "string") => true,
        (serde_json::Value::Array(_), "array") => true,
        (serde_json::Value::Object(_), "object") => true,
        _ => false,
    }
}

fn json_type_label(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[async_trait]
impl SwarmAgent for SchemaValidator {
    fn agent_id(&self) -> AgentId {
        AgentId::new(Self::AGENT_ID).expect("static agent id")
    }

    async fn run(
        &self,
        ctx: SwarmAgentContext,
    ) -> Result<AgentOutcome, SwarmAgentError> {
        let started = std::time::Instant::now();
        // Read the input root.
        let input = ctx
            .blackboard
            .read("/input")
            .map(|v| v.value)
            .unwrap_or_else(|| (*ctx.input).clone());
        let verdict = match validate(&input, &self.schema) {
            Ok(()) => SchemaVerdict {
                valid: true,
                errors: vec![],
            },
            Err(errs) => SchemaVerdict {
                valid: false,
                errors: errs,
            },
        };
        ctx.blackboard
            .write(
                "/schema_valid",
                serde_json::to_value(&verdict).unwrap_or(serde_json::Value::Null),
                ctx.agent_id.clone(),
            )
            .map_err(|e| SwarmAgentError::Blackboard {
                message: e.to_string(),
            })?;
        Ok(AgentOutcome::Completed {
            cost_microcents: 0,
            duration_ms: (started.elapsed().as_millis() as u32).max(1),
            blackboard_writes: 1,
            events_published: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_blackboard::{Blackboard, EventBus};
    use efe_nano_sdk::SwarmId;
    use std::sync::Arc;

    fn ctx_for(input: serde_json::Value, schema: serde_json::Value) -> (SwarmAgentContext, SchemaValidator) {
        let blackboard = Arc::new(Blackboard::new(SwarmId::new()));
        blackboard
            .write(
                "/input",
                input.clone(),
                AgentId::new("test-fixture").unwrap(),
            )
            .unwrap();
        let ctx = SwarmAgentContext {
            swarm_id: blackboard.swarm_id(),
            agent_id: AgentId::new(SchemaValidator::AGENT_ID).unwrap(),
            blackboard,
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(input),
            rng_seed: 42,
        };
        (ctx, SchemaValidator::new(schema))
    }

    #[test]
    fn valid_object_matches_schema() {
        let v = serde_json::json!({"name": "Alice", "age": 30});
        let s = serde_json::json!({
            "type": "object",
            "required": ["name", "age"],
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "number"}
            }
        });
        assert!(validate(&v, &s).is_ok());
    }

    #[test]
    fn missing_required_field_reported() {
        let v = serde_json::json!({"name": "Alice"});
        let s = serde_json::json!({
            "type": "object",
            "required": ["name", "age"]
        });
        let errs = validate(&v, &s).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("age")));
    }

    #[test]
    fn wrong_type_reported() {
        let v = serde_json::json!("hello");
        let s = serde_json::json!({"type": "number"});
        let errs = validate(&v, &s).unwrap_err();
        assert!(errs[0].contains("expected type number"));
    }

    #[test]
    fn string_min_length_enforced() {
        let v = serde_json::json!("ab");
        let s = serde_json::json!({"type": "string", "minLength": 3});
        assert!(validate(&v, &s).is_err());
    }

    #[test]
    fn string_max_length_enforced() {
        let v = serde_json::json!("hello");
        let s = serde_json::json!({"type": "string", "maxLength": 3});
        assert!(validate(&v, &s).is_err());
    }

    #[test]
    fn number_minimum_enforced() {
        let v = serde_json::json!(5);
        let s = serde_json::json!({"type": "number", "minimum": 10});
        assert!(validate(&v, &s).is_err());
    }

    #[test]
    fn number_maximum_enforced() {
        let v = serde_json::json!(100);
        let s = serde_json::json!({"type": "number", "maximum": 50});
        assert!(validate(&v, &s).is_err());
    }

    #[test]
    fn nested_object_validated_recursively() {
        let v = serde_json::json!({"user": {"name": "Bob"}});
        let s = serde_json::json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "required": ["name", "age"]
                }
            }
        });
        let errs = validate(&v, &s).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("age")));
    }

    #[test]
    fn deterministic_validation() {
        let v = serde_json::json!({"x": 5});
        let s = serde_json::json!({"type": "object", "required": ["x"]});
        assert_eq!(validate(&v, &s), validate(&v, &s));
    }

    #[tokio::test]
    async fn agent_writes_verdict_valid() {
        let (ctx, agent) = ctx_for(
            serde_json::json!({"x": 5}),
            serde_json::json!({"type": "object", "required": ["x"]}),
        );
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let v = bb.read("/schema_valid").unwrap();
        let parsed: SchemaVerdict = serde_json::from_value(v.value).unwrap();
        assert!(parsed.valid);
        assert!(parsed.errors.is_empty());
    }

    #[tokio::test]
    async fn agent_writes_verdict_invalid() {
        let (ctx, agent) = ctx_for(
            serde_json::json!({"x": 5}),
            serde_json::json!({"type": "object", "required": ["missing"]}),
        );
        let bb = ctx.blackboard.clone();
        let _ = agent.run(ctx).await.unwrap();
        let v = bb.read("/schema_valid").unwrap();
        let parsed: SchemaVerdict = serde_json::from_value(v.value).unwrap();
        assert!(!parsed.valid);
        assert!(!parsed.errors.is_empty());
    }

    #[tokio::test]
    async fn agent_id_stable() {
        let v = SchemaValidator::new(serde_json::json!({}));
        assert_eq!(v.agent_id().as_str(), "schema-validator");
    }
}
