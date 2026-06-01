//! LLMRouter — main orchestrator (M21).

use crate::audit::AuditEmitter;
use crate::cache::Cache;
use crate::circuit_breaker::CircuitBreaker;
use crate::config::{OptimizeFor, RedactionLevel, TenantLLMConfig};
use crate::cost_governor::CostGovernor;
use crate::error::RouterError;
use crate::provider::{
    CompleteRequest, LLMProvider, Message, ProviderError, ProviderHandle,
};
use efe_prompts::PromptRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterRequest {
    pub tenant_id: String,
    pub catalog: String,
    pub bundle_version: String,
    pub sentence_redacted: String,
    pub candidates: Vec<String>,
    pub prompt_template_id: String,
    pub max_tokens: u32,
    pub timeout_ms: u32,
}

impl RouterRequest {
    pub fn estimated_tokens(&self) -> u32 {
        let candidates_len: usize = self.candidates.iter().map(|c| c.len()).sum();
        let total = self.sentence_redacted.len() + candidates_len + 500;
        (total / 4) as u32
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterResponse {
    pub text: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: f64,
    pub provider_name: String,
    pub model_name: String,
    pub cache_hit: bool,
    pub duration_ms: u32,
}

impl RouterResponse {
    fn cached(c: crate::cache::CachedResponse, duration_ms: u32) -> Self {
        Self {
            text: c.text,
            tokens_in: c.tokens_in,
            tokens_out: c.tokens_out,
            cost_usd: 0.0,
            provider_name: c.provider_name,
            model_name: c.model_name,
            cache_hit: true,
            duration_ms,
        }
    }
}

fn quality_rank(name: &str) -> u32 {
    match name {
        n if n.starts_with("anthropic:claude-opus") => 1,
        n if n.starts_with("anthropic:claude-sonnet") => 2,
        n if n.starts_with("openai:gpt-5") && !n.contains("mini") => 3,
        n if n.starts_with("vertex:gemini-2.5-pro") => 4,
        n if n.starts_with("openai:gpt-5-mini") => 5,
        n if n.starts_with("vertex:gemini-2.5-flash") => 6,
        n if n.starts_with("mistral:") => 7,
        n if n.starts_with("vllm:") => 8,
        n if n.starts_with("hf-tgi:") => 9,
        _ => 100,
    }
}

fn latency_p50_ms(name: &str) -> u32 {
    match name {
        n if n.starts_with("vllm:") => 100,
        n if n.starts_with("vertex:gemini-2.5-flash") => 250,
        n if n.starts_with("openai:gpt-5-mini") => 350,
        n if n.starts_with("hf-tgi:") => 200,
        n if n.starts_with("anthropic:") => 500,
        n if n.starts_with("openai:gpt-5") => 400,
        n if n.starts_with("bedrock:") => 600,
        n if n.starts_with("azure-openai:") => 450,
        n if n.starts_with("mistral:") => 400,
        _ => 1000,
    }
}

#[derive(Clone)]
pub struct LLMRouter {
    providers: Arc<HashMap<String, ProviderHandle>>,
    cost: CostGovernor,
    cache: Cache,
    circuit: CircuitBreaker,
    prompts: Arc<PromptRegistry>,
    audit: AuditEmitter,
    pg_pool: Arc<sqlx::PgPool>,
}

impl LLMRouter {
    pub fn new(
        providers: HashMap<String, ProviderHandle>,
        cost: CostGovernor,
        cache: Cache,
        circuit: CircuitBreaker,
        prompts: Arc<PromptRegistry>,
        audit: AuditEmitter,
        pg_pool: Arc<sqlx::PgPool>,
    ) -> Self {
        Self {
            providers: Arc::new(providers),
            cost,
            cache,
            circuit,
            prompts,
            audit,
            pg_pool,
        }
    }

    pub fn provider_names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Get a snapshot of all circuit breaker states (for /admin endpoint).
    pub fn circuit_snapshot(&self) -> HashMap<String, String> {
        self.circuit
            .states_snapshot()
            .into_iter()
            .map(|(k, v)| (k, v.as_str().to_string()))
            .collect()
    }

    /// Main entry point : run a Mode-2 LLM call end-to-end.
    #[tracing::instrument(skip_all, fields(tenant_id = %request.tenant_id, catalog = %request.catalog))]
    pub async fn complete(
        &self,
        request: RouterRequest,
    ) -> Result<RouterResponse, RouterError> {
        let t0 = Instant::now();

        let tenant_config = self.tenant_config(&request.tenant_id).await?;
        if !tenant_config.enabled {
            return Err(RouterError::FeatureDisabled(request.tenant_id.clone()));
        }

        let estimated_cost = self.estimate_cost(&request, &tenant_config);
        let _budget = self
            .cost
            .check_budget(&request.tenant_id, estimated_cost)
            .await?;

        let cache_key = Cache::compute_exact_key(
            &request.tenant_id,
            &request.catalog,
            &request.bundle_version,
            &request.prompt_template_id,
            1,
            &request.sentence_redacted,
        );
        if let Some(cached) = self.cache.get_exact(&cache_key).await? {
            let dur = t0.elapsed().as_millis() as u32;
            return Ok(RouterResponse::cached(cached, dur));
        }

        let provider_chain = self.select_providers(&request, &tenant_config);
        if provider_chain.is_empty() {
            return Err(RouterError::NoEligibleProvider);
        }

        let template = self
            .prompts
            .lookup(&request.prompt_template_id, &request.tenant_id)
            .map_err(|e| RouterError::Infrastructure(anyhow::anyhow!(e.to_string())))?;
        let mut vars = HashMap::new();
        vars.insert("catalog".into(), request.catalog.clone());
        vars.insert("sentence".into(), request.sentence_redacted.clone());
        vars.insert("top_k".into(), request.candidates.len().to_string());
        vars.insert(
            "candidates_json".into(),
            serde_json::to_string(&request.candidates).unwrap_or_default(),
        );
        let (system_prompt, user_prompt) = template.render(&vars);

        let mut last_err: Option<ProviderError> = None;
        let mut errors_count = 0usize;
        for provider in &provider_chain {
            if !self.circuit.is_closed(provider.name()) {
                continue;
            }
            let provider_request = CompleteRequest {
                system_prompt: Some(system_prompt.clone()),
                messages: vec![Message::user(user_prompt.clone())],
                max_tokens: request.max_tokens,
                temperature: 0.0,
                seed: None,
                timeout_ms: request.timeout_ms,
                tools: None,
            };
            match provider.complete(provider_request).await {
                Ok(resp) => {
                    self.circuit.record_success(provider.name());
                    let _ = self.cache.put_exact(&cache_key, &resp, 3_600).await;
                    let _ = self
                        .cache
                        .add_to_catalog_index(
                            &request.tenant_id,
                            &request.catalog,
                            &cache_key,
                        )
                        .await;
                    let _ = self
                        .cost
                        .commit_actual_cost(&request.tenant_id, resp.cost_usd)
                        .await;
                    let dur = t0.elapsed().as_millis() as u32;
                    info!(
                        provider = provider.name(),
                        tokens_in = resp.tokens_in,
                        tokens_out = resp.tokens_out,
                        cost_usd = resp.cost_usd,
                        duration_ms = dur,
                        "router complete success"
                    );
                    return Ok(RouterResponse {
                        text: resp.text,
                        tokens_in: resp.tokens_in,
                        tokens_out: resp.tokens_out,
                        cost_usd: resp.cost_usd,
                        provider_name: resp.provider_name,
                        model_name: resp.model_name,
                        cache_hit: false,
                        duration_ms: dur,
                    });
                }
                Err(e) => {
                    self.circuit.record_failure(provider.name());
                    warn!(provider = provider.name(), error = %e, "provider failed");
                    errors_count += 1;
                    last_err = Some(e);
                }
            }
        }

        let summary = last_err
            .as_ref()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no provider attempted".into());
        Err(RouterError::AllProvidersFailed {
            count: errors_count,
            summary,
            last_error: last_err,
        })
    }

    /// Provider selection algorithm (cf. § 6.3 architecture doc).
    pub fn select_providers(
        &self,
        request: &RouterRequest,
        config: &TenantLLMConfig,
    ) -> Vec<ProviderHandle> {
        let mut providers: Vec<ProviderHandle> = self
            .providers
            .values()
            .filter(|p: &&Arc<dyn LLMProvider>| {
                let n = p.name();
                config.allowed_providers.iter().any(|a| a == n)
                    && p.max_context_tokens() >= request.estimated_tokens()
            })
            .cloned()
            .collect();

        match config.optimize_for {
            OptimizeFor::Cost => {
                providers.sort_by(|a, b| {
                    a.cost_model()
                        .input_per_1m_tokens_usd
                        .partial_cmp(&b.cost_model().input_per_1m_tokens_usd)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            OptimizeFor::Quality => {
                providers.sort_by_key(|p| quality_rank(p.name()));
            }
            OptimizeFor::Latency => {
                providers.sort_by_key(|p| latency_p50_ms(p.name()));
            }
            OptimizeFor::Balanced => {
                providers.sort_by(|a, b| {
                    let score_a = balanced_score(a.name(), &a.cost_model());
                    let score_b = balanced_score(b.name(), &b.cost_model());
                    score_a
                        .partial_cmp(&score_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }

        if let Some(primary) = config.primary_provider.as_deref() {
            providers.sort_by_key(|p| if p.name() == primary { 0 } else { 1 });
        }

        providers
    }

    fn estimate_cost(&self, request: &RouterRequest, config: &TenantLLMConfig) -> f64 {
        let provider_name = config
            .primary_provider
            .as_deref()
            .or_else(|| config.allowed_providers.first().map(|s| s.as_str()))
            .unwrap_or("anthropic:claude-sonnet-4-5");
        let cost = if let Some(p) = self.providers.get(provider_name) {
            p.cost_model()
        } else {
            crate::provider::CostModel {
                input_per_1m_tokens_usd: 3.0,
                output_per_1m_tokens_usd: 15.0,
            }
        };
        let ti = request.estimated_tokens();
        let to = request.max_tokens.min(500);
        (ti as f64) * cost.input_per_1m_tokens_usd / 1_000_000.0
            + (to as f64) * cost.output_per_1m_tokens_usd / 1_000_000.0
    }

    async fn tenant_config(&self, tenant_id: &str) -> Result<TenantLLMConfig, RouterError> {
        type Row = (
            bool,         // enabled
            Vec<String>,  // allowed_providers
            Option<String>, // primary_provider
            Vec<String>,  // fallback_providers
            String,       // optimize_for
            i32,          // max_context_tokens
            bool,         // require_baa
            bool,         // require_eu_residency
            bool,         // require_no_training
            String,       // prompt_redaction_level
            bool,         // audit_full_prompts
        );
        let row: Option<Row> = sqlx::query_as(
            r#"SELECT enabled,
                      allowed_providers,
                      primary_provider,
                      fallback_providers,
                      optimize_for,
                      max_context_tokens,
                      require_baa,
                      require_eu_residency,
                      require_no_training,
                      prompt_redaction_level,
                      audit_full_prompts
               FROM tenant_llm_config
               WHERE tenant_id = $1"#,
        )
        .bind(tenant_id)
        .fetch_optional(&*self.pg_pool)
        .await
        .map_err(|e| RouterError::TenantConfig(e.to_string()))?;

        let row = row.ok_or_else(|| {
            RouterError::TenantConfig(format!("tenant {} not configured", tenant_id))
        })?;

        let optimize_for = match row.4.as_str() {
            "cost" => OptimizeFor::Cost,
            "quality" => OptimizeFor::Quality,
            "latency" => OptimizeFor::Latency,
            _ => OptimizeFor::Balanced,
        };
        let prompt_redaction_level = match row.9.as_str() {
            "none" => RedactionLevel::None,
            "mild" => RedactionLevel::Mild,
            _ => RedactionLevel::Aggressive,
        };

        Ok(TenantLLMConfig {
            tenant_id: tenant_id.into(),
            enabled: row.0,
            allowed_providers: row.1,
            primary_provider: row.2,
            fallback_providers: row.3,
            optimize_for,
            max_context_tokens: row.5 as u32,
            require_baa: row.6,
            require_eu_residency: row.7,
            require_no_training: row.8,
            prompt_redaction_level,
            audit_full_prompts: row.10,
            semantic_cache_enabled: false,
        })
    }
}

fn balanced_score(name: &str, cost: &crate::provider::CostModel) -> f64 {
    let cost_norm = cost.input_per_1m_tokens_usd / 15.0;
    let lat_norm = (latency_p50_ms(name) as f64) / 1000.0;
    0.5 * cost_norm + 0.5 * lat_norm
}

// Audit FromStr import for Decimal resolved via String parsing in
// cost_governor.rs ; keep this here for future expansion if needed.
#[allow(dead_code)]
fn _ensure_str_used() {
    let _ = String::from_str("0").unwrap();
}
