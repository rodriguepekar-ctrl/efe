//! AgentRun + AgentRunTemplate validation rules.
//!
//! The webhook server invokes these from inside an axum handler. Each
//! validator returns `ValidationOutcome::Allow` or `Deny(msg)`, plus
//! optionally an internal error on DB issues (which causes the webhook
//! to return 500 + fail-closed).

use serde::Deserialize;
use serde_json::Value as Json;

use super::{ValidationOutcome, WebhookState};
use crate::{AgentRunnerError, AgentRunnerResult};

/// Tools currently allowed system-wide (M25 catalogue ; M26 frozen).
pub const KNOWN_TOOLS: &[&str] = &[
    "web_search",
    "fetch_url",
    "efe_classify",
    "efe_query_events",
    "efe_upsert_catalog",
    "efe_ingest_events",
    "efe_retune_centroids",
    "noop",
];

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // some fields read for the audit trail only
struct AgentRunSpec {
    #[serde(rename = "tenantId")]
    tenant_id: String,
    #[serde(rename = "templateSlug")]
    template_slug: String,
    profile: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    #[serde(rename = "toolWhitelist", default)]
    tool_whitelist: Option<Vec<String>>,
    #[serde(rename = "timeoutSeconds", default)]
    timeout_seconds: Option<i32>,
    #[serde(rename = "maxCostUsd", default)]
    max_cost_usd: Option<f64>,
    #[serde(rename = "minChannelRequired", default)]
    min_channel_required: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // some fields validated for shape only
struct AgentRunTemplateSpec {
    #[serde(rename = "tenantId")]
    tenant_id: String,
    slug: String,
    #[serde(rename = "displayName")]
    display_name: String,
    profile: String,
    #[serde(rename = "defaultProvider")]
    default_provider: String,
    #[serde(rename = "defaultModel")]
    default_model: String,
    #[serde(rename = "systemPrompt")]
    system_prompt: String,
    #[serde(rename = "toolWhitelist")]
    tool_whitelist: Vec<String>,
    #[serde(rename = "maxSteps", default = "default_max_steps")]
    max_steps: i32,
    #[serde(rename = "defaultTimeoutSeconds", default = "default_timeout")]
    default_timeout_seconds: i32,
    #[serde(rename = "defaultMaxCostCents", default = "default_cost")]
    default_max_cost_cents: i64,
    #[serde(rename = "requireApprovalForWrites", default = "default_true")]
    require_approval_for_writes: bool,
}

fn default_max_steps() -> i32 { 25 }
fn default_timeout() -> i32 { 1800 }
fn default_cost() -> i64 { 50 }
fn default_true() -> bool { true }

#[derive(Debug, sqlx::FromRow)]
struct TenantFlags {
    m25_agent_runtime_enabled: bool,
    m25_dpia_signed_at: Option<chrono::DateTime<chrono::Utc>>,
    m25_dpia_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    m25_cost_cap_per_run_usd: Option<f64>,
    m25_timeout_minutes_max: Option<i32>,
    m25_allowed_profiles: Option<Vec<String>>,
    m25_allowed_providers: Option<Vec<String>>,
    m25_max_concurrent_runs: Option<i32>,
    m26_graduation_acknowledged_at: Option<chrono::DateTime<chrono::Utc>>,
    m26_graduation_revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl TenantFlags {
    fn channel(&self) -> &'static str {
        if self.m26_graduation_revoked_at.is_some() {
            "EXPERIMENTAL"
        } else if self.m26_graduation_acknowledged_at.is_some() {
            "STABLE"
        } else if self.m25_agent_runtime_enabled {
            "EXPERIMENTAL"
        } else {
            "DISABLED"
        }
    }

    fn dpia_valid(&self) -> bool {
        match (self.m25_dpia_signed_at, self.m25_dpia_expires_at) {
            (Some(_), Some(exp)) => exp > chrono::Utc::now(),
            _ => false,
        }
    }
}

// ── AgentRun validation ──────────────────────────────────────────────

pub async fn validate_agent_run(
    state: &WebhookState,
    object: &Json,
) -> AgentRunnerResult<ValidationOutcome> {
    let spec_value = match object.get("spec") {
        Some(s) => s,
        None => return Ok(ValidationOutcome::deny("missing spec")),
    };
    let spec: AgentRunSpec = match serde_json::from_value(spec_value.clone()) {
        Ok(s) => s,
        Err(e) => return Ok(ValidationOutcome::deny(format!("spec parse : {}", e))),
    };

    // 1. Tenant flags exist.
    let flags = match load_tenant_flags(&state.pg, &spec.tenant_id).await? {
        Some(f) => f,
        None => return Ok(ValidationOutcome::deny(format!(
            "tenant '{}' not found", spec.tenant_id
        ))),
    };

    // 2. Tenant has agent runtime enabled.
    if matches!(flags.channel(), "DISABLED") {
        return Ok(ValidationOutcome::deny(
            "M25 agent runtime not enabled for tenant",
        ));
    }

    // 3. DPIA valid.
    if !flags.dpia_valid() {
        return Ok(ValidationOutcome::deny("DPIA not signed or expired"));
    }

    // 4. Template exists + active.
    let template: Option<TemplateRow> = sqlx::query_as(
        r#"SELECT id, profile, default_provider, default_model,
                  tool_whitelist_json, max_steps, default_timeout_seconds,
                  default_max_cost_cents, deprecated
           FROM agent_run_template
           WHERE tenant_id = $1 AND slug = $2 AND NOT deprecated
           ORDER BY version DESC LIMIT 1"#,
    )
    .bind(&spec.tenant_id)
    .bind(&spec.template_slug)
    .fetch_optional(&state.pg)
    .await?;

    let template = match template {
        Some(t) => t,
        None => return Ok(ValidationOutcome::deny(format!(
            "template '{}' not found or deprecated", spec.template_slug
        ))),
    };

    // 5. Profile in tenant's allow-list.
    let profile = spec.profile.as_ref().unwrap_or(&template.profile);
    if let Some(allowed) = &flags.m25_allowed_profiles {
        if !allowed.iter().any(|p| p == profile) {
            return Ok(ValidationOutcome::deny(format!(
                "profile '{}' not in tenant allowlist", profile
            )));
        }
    }
    if state.profiles.get(profile).is_err() {
        return Ok(ValidationOutcome::deny(format!(
            "profile '{}' not in registry", profile
        )));
    }

    // 6. Provider in tenant's allow-list.
    let provider = spec
        .provider
        .as_ref()
        .unwrap_or(&template.default_provider);
    if let Some(allowed) = &flags.m25_allowed_providers {
        if !allowed.iter().any(|p| p == provider) {
            return Ok(ValidationOutcome::deny(format!(
                "provider '{}' not in tenant allowlist", provider
            )));
        }
    }

    // 7. Tool whitelist : subset of template's + only known tools.
    let tools = spec
        .tool_whitelist
        .as_ref()
        .cloned()
        .unwrap_or_else(|| {
            serde_json::from_value::<Vec<String>>(template.tool_whitelist_json.clone())
                .unwrap_or_default()
        });
    let template_tools: Vec<String> =
        serde_json::from_value(template.tool_whitelist_json).unwrap_or_default();
    for t in &tools {
        if !KNOWN_TOOLS.contains(&t.as_str()) {
            return Ok(ValidationOutcome::deny(format!(
                "tool '{}' not in known tool catalogue", t
            )));
        }
        if !template_tools.iter().any(|x| x == t) {
            return Ok(ValidationOutcome::deny(format!(
                "tool '{}' not in template's whitelist", t
            )));
        }
    }
    if tools.is_empty() {
        return Ok(ValidationOutcome::deny("tool whitelist must be non-empty"));
    }

    // 8. Cost cap.
    let max_cost_usd = spec
        .max_cost_usd
        .unwrap_or(template.default_max_cost_cents as f64 / 100.0);
    if let Some(cap) = flags.m25_cost_cap_per_run_usd {
        if max_cost_usd > cap {
            return Ok(ValidationOutcome::deny(format!(
                "max_cost_usd {:.2} > tenant cap {:.2}", max_cost_usd, cap
            )));
        }
    }

    // 9. Timeout cap.
    let timeout_s = spec
        .timeout_seconds
        .unwrap_or(template.default_timeout_seconds);
    if let Some(cap_min) = flags.m25_timeout_minutes_max {
        if timeout_s > cap_min * 60 {
            return Ok(ValidationOutcome::deny(format!(
                "timeout_seconds {} > tenant cap {}", timeout_s, cap_min * 60
            )));
        }
    }

    // 10. Concurrent runs.
    if let Some(cap_concurrent) = flags.m25_max_concurrent_runs {
        let active: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)::bigint FROM agent_run
               WHERE tenant_id = $1
                 AND status NOT IN ('COMPLETED','FAILED','TERMINATED','TIMED_OUT','COST_CAPPED')"#,
        )
        .bind(&spec.tenant_id)
        .fetch_one(&state.pg)
        .await?;
        if active >= cap_concurrent as i64 {
            return Ok(ValidationOutcome::deny(format!(
                "tenant has {} active runs (cap {})", active, cap_concurrent
            )));
        }
    }

    // 11. STABLE-only checks.
    if let Some(min) = &spec.min_channel_required {
        let channel = flags.channel();
        if min == "STABLE" && channel != "STABLE" {
            return Ok(ValidationOutcome::deny(format!(
                "template requires STABLE channel ; tenant on {}", channel
            )));
        }
    }

    Ok(ValidationOutcome::Allow)
}

#[derive(sqlx::FromRow)]
struct TemplateRow {
    #[allow(dead_code)] id: uuid::Uuid,
    profile: String,
    default_provider: String,
    #[allow(dead_code)] default_model: String,
    tool_whitelist_json: serde_json::Value,
    #[allow(dead_code)] max_steps: i32,
    default_timeout_seconds: i32,
    default_max_cost_cents: i64,
    #[allow(dead_code)] deprecated: bool,
}

// ── AgentRunTemplate validation ──────────────────────────────────────

pub async fn validate_agent_run_template(
    state: &WebhookState,
    object: &Json,
) -> AgentRunnerResult<ValidationOutcome> {
    let spec_value = match object.get("spec") {
        Some(s) => s,
        None => return Ok(ValidationOutcome::deny("missing spec")),
    };
    let spec: AgentRunTemplateSpec = match serde_json::from_value(spec_value.clone()) {
        Ok(s) => s,
        Err(e) => return Ok(ValidationOutcome::deny(format!("spec parse : {}", e))),
    };

    if spec.slug.is_empty() || spec.slug.len() > 64 {
        return Ok(ValidationOutcome::deny("slug must be 1..=64 chars"));
    }
    if !spec
        .slug
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Ok(ValidationOutcome::deny("slug must match [A-Za-z0-9_-]"));
    }
    if spec.system_prompt.is_empty() || spec.system_prompt.len() > 32768 {
        return Ok(ValidationOutcome::deny(
            "system_prompt must be 1..=32768 chars",
        ));
    }
    if spec.tool_whitelist.is_empty() {
        return Ok(ValidationOutcome::deny("tool_whitelist must be non-empty"));
    }
    for t in &spec.tool_whitelist {
        if !KNOWN_TOOLS.contains(&t.as_str()) {
            return Ok(ValidationOutcome::deny(format!(
                "tool '{}' not in known tool catalogue", t
            )));
        }
    }
    if !(1..=200).contains(&spec.max_steps) {
        return Ok(ValidationOutcome::deny("max_steps must be 1..=200"));
    }
    if !(60..=14400).contains(&spec.default_timeout_seconds) {
        return Ok(ValidationOutcome::deny(
            "default_timeout_seconds must be 60..=14400",
        ));
    }
    if spec.default_max_cost_cents < 1 {
        return Ok(ValidationOutcome::deny(
            "default_max_cost_cents must be ≥ 1",
        ));
    }

    if state.profiles.get(&spec.profile).is_err() {
        return Ok(ValidationOutcome::deny(format!(
            "profile '{}' unknown", spec.profile
        )));
    }
    let _ = spec.display_name;
    let _ = spec.default_provider;
    let _ = spec.default_model;
    let _ = spec.require_approval_for_writes;

    Ok(ValidationOutcome::Allow)
}

// ── DB helpers ───────────────────────────────────────────────────────

async fn load_tenant_flags(
    pg: &sqlx::PgPool,
    tenant: &str,
) -> AgentRunnerResult<Option<TenantFlags>> {
    let f: Option<TenantFlags> = sqlx::query_as(
        r#"SELECT
              m25_agent_runtime_enabled,
              m25_dpia_signed_at,
              m25_dpia_expires_at,
              m25_cost_cap_per_run_usd,
              m25_timeout_minutes_max,
              m25_allowed_profiles,
              m25_allowed_providers,
              m25_max_concurrent_runs,
              m26_graduation_acknowledged_at,
              m26_graduation_revoked_at
           FROM tenant_feature
           WHERE id = $1"#,
    )
    .bind(tenant)
    .fetch_optional(pg)
    .await
    .map_err(|e| AgentRunnerError::Database(e))?;
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tools_includes_m25_catalogue() {
        assert!(KNOWN_TOOLS.contains(&"web_search"));
        assert!(KNOWN_TOOLS.contains(&"efe_upsert_catalog"));
        assert!(KNOWN_TOOLS.contains(&"noop"));
    }
}
