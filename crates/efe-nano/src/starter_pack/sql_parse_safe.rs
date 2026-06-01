//! `sql_parse_safe` — parse SQL and verify it's a safe SELECT-only query.
//!
//! Mode A — pure Rust (sqlparser).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;
use sqlparser::ast::Statement;
use sqlparser::dialect::{GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("sql_parse_safe").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Parse a SQL query, validate syntax, and verify it's a safe SELECT-only query.".to_string(),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "sql_parse_safe::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["sql"],
            "properties": {
                "sql": { "type": "string" },
                "dialect": {
                    "type": "string",
                    "enum": ["generic", "postgres", "mysql", "sqlite"],
                    "default": "generic"
                }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["parseable", "is_safe_select_only", "statement_count"],
            "properties": {
                "parseable": { "type": "boolean" },
                "is_safe_select_only": { "type": "boolean" },
                "statement_count": { "type": "integer", "minimum": 0 },
                "statement_kinds": { "type": "array", "items": { "type": "string" } },
                "errors": { "type": "array", "items": { "type": "string" } }
            }
        }),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.005,
        latency_p99_ms_target: 30,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: Utc::now(),
    }
}

pub fn handler(
) -> impl Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync + 'static {
    |input: serde_json::Value| -> Result<serde_json::Value, String> {
        let sql = input
            .get("sql")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'sql'".to_string())?;
        let dialect_str = input
            .get("dialect")
            .and_then(|v| v.as_str())
            .unwrap_or("generic");

        let dialect: Box<dyn sqlparser::dialect::Dialect> = match dialect_str {
            "postgres" => Box::new(PostgreSqlDialect {}),
            "mysql" => Box::new(MySqlDialect {}),
            "sqlite" => Box::new(SQLiteDialect {}),
            _ => Box::new(GenericDialect {}),
        };

        match Parser::parse_sql(&*dialect, sql) {
            Ok(statements) => {
                let mut is_safe = true;
                let mut kinds: Vec<&str> = Vec::new();
                for stmt in &statements {
                    let kind = statement_kind(stmt);
                    kinds.push(kind);
                    if !is_safe_kind(kind) {
                        is_safe = false;
                    }
                }
                Ok(json!({
                    "parseable": true,
                    "is_safe_select_only": is_safe,
                    "statement_count": statements.len(),
                    "statement_kinds": kinds,
                    "errors": [],
                }))
            }
            Err(e) => Ok(json!({
                "parseable": false,
                "is_safe_select_only": false,
                "statement_count": 0,
                "statement_kinds": [],
                "errors": [e.to_string()],
            })),
        }
    }
}

fn statement_kind(stmt: &Statement) -> &'static str {
    match stmt {
        Statement::Query(_) => "Query",
        Statement::Insert { .. } => "Insert",
        Statement::Update { .. } => "Update",
        Statement::Delete { .. } => "Delete",
        Statement::CreateTable { .. } => "CreateTable",
        Statement::Drop { .. } => "Drop",
        Statement::Truncate { .. } => "Truncate",
        Statement::AlterTable { .. } => "AlterTable",
        Statement::CreateIndex { .. } => "CreateIndex",
        Statement::CreateView { .. } => "CreateView",
        Statement::Grant { .. } => "Grant",
        Statement::Revoke { .. } => "Revoke",
        Statement::Commit { .. } => "Commit",
        Statement::Rollback { .. } => "Rollback",
        _ => "Other",
    }
}

fn is_safe_kind(kind: &str) -> bool {
    matches!(kind, "Query")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_validation_category() {
        assert_eq!(manifest().category, Category::Validation);
    }

    #[test]
    fn parses_simple_select() {
        let h = handler();
        let out = h(json!({"sql": "SELECT id, name FROM users WHERE age > 18"})).unwrap();
        assert_eq!(out["parseable"], true);
        assert_eq!(out["is_safe_select_only"], true);
        assert_eq!(out["statement_count"], 1);
        assert_eq!(out["statement_kinds"][0], "Query");
    }

    #[test]
    fn flags_insert_as_unsafe() {
        let h = handler();
        let out = h(json!({"sql": "INSERT INTO users (name) VALUES ('Alice')"})).unwrap();
        assert_eq!(out["parseable"], true);
        assert_eq!(out["is_safe_select_only"], false);
        assert_eq!(out["statement_kinds"][0], "Insert");
    }

    #[test]
    fn flags_delete_as_unsafe() {
        let h = handler();
        let out = h(json!({"sql": "DELETE FROM users WHERE id = 1"})).unwrap();
        assert_eq!(out["is_safe_select_only"], false);
        assert_eq!(out["statement_kinds"][0], "Delete");
    }

    #[test]
    fn flags_update_as_unsafe() {
        let h = handler();
        let out = h(json!({"sql": "UPDATE users SET name = 'x' WHERE id = 1"})).unwrap();
        assert_eq!(out["is_safe_select_only"], false);
        assert_eq!(out["statement_kinds"][0], "Update");
    }

    #[test]
    fn flags_drop_as_unsafe() {
        let h = handler();
        let out = h(json!({"sql": "DROP TABLE users"})).unwrap();
        assert_eq!(out["is_safe_select_only"], false);
        assert_eq!(out["statement_kinds"][0], "Drop");
    }

    #[test]
    fn invalid_sql_returns_unparseable() {
        let h = handler();
        let out = h(json!({"sql": "SELET * FROM"})).unwrap();
        assert_eq!(out["parseable"], false);
        assert_eq!(out["is_safe_select_only"], false);
        assert!(!out["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn multi_statement_with_mixed_safety() {
        let h = handler();
        // Two SELECTs separated by ; — both safe.
        let out = h(json!({"sql": "SELECT 1; SELECT 2"})).unwrap();
        assert_eq!(out["statement_count"], 2);
        assert_eq!(out["is_safe_select_only"], true);

        // SELECT + DELETE → mixed → flagged unsafe.
        let out = h(json!({"sql": "SELECT 1; DELETE FROM t"})).unwrap();
        assert_eq!(out["statement_count"], 2);
        assert_eq!(out["is_safe_select_only"], false);
    }
}
