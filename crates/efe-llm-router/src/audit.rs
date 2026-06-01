//! Audit emission (M21) — extends D14-bis HMAC chain to llm_call_audit.

use crate::provider::CompleteResponse;
use crate::router::RouterRequest;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallAuditEvent {
    pub tenant_id: String,
    pub catalog: String,
    pub bundle_version: String,
    pub sentence_hash: String,
    pub mode: String,
    pub provider_used: String,
    pub model_used: String,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: f64,
    pub cache_hit: bool,
    pub duration_ms: u32,
    pub label: Option<String>,
    pub confidence: Option<f32>,
    pub anchors_used: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("postgres error: {0}")]
    Pg(#[from] sqlx::Error),
    #[error("invalid HMAC key")]
    HmacKey,
}

#[derive(Clone)]
pub struct AuditEmitter {
    pool: Arc<PgPool>,
    hmac_key: Arc<Vec<u8>>,
    state: Arc<dashmap::DashMap<String, Arc<Mutex<TenantChainState>>>>,
}

#[derive(Debug, Clone, Default)]
struct TenantChainState {
    last_seq: i64,
    last_hmac: String,
    initialised: bool,
}

impl AuditEmitter {
    pub fn new(pool: Arc<PgPool>, hmac_key: Vec<u8>) -> Self {
        Self {
            pool,
            hmac_key: Arc::new(hmac_key),
            state: Arc::new(dashmap::DashMap::new()),
        }
    }

    /// Append an LLM call event. Computes seq + HMAC via M14-bis chain.
    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip(self, request, response))]
    pub async fn append_llm_call(
        &self,
        request: &RouterRequest,
        response: &CompleteResponse,
        sentence_hash: &str,
        mode: &str,
        cache_hit: bool,
        duration_ms: u32,
        label: Option<&str>,
        confidence: Option<f32>,
        anchors_used: &[String],
    ) -> Result<(), AuditError> {
        let tenant_lock = self
            .state
            .entry(request.tenant_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(TenantChainState::default())))
            .clone();

        let mut chain = tenant_lock.lock().await;
        if !chain.initialised {
            self.bootstrap_chain(&request.tenant_id, &mut chain).await?;
        }

        let next_seq = chain.last_seq + 1;
        let prev_hmac = chain.last_hmac.clone();

        let payload = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}|{}|{:.6}|{}|{}|{}|{:.4}|{}|{}|{}",
            request.tenant_id,
            request.catalog,
            request.bundle_version,
            sentence_hash,
            mode,
            response.provider_name,
            response.model_name,
            response.tokens_in,
            response.tokens_out,
            response.cost_usd,
            cache_hit,
            duration_ms,
            label.unwrap_or(""),
            confidence.unwrap_or(0.0),
            anchors_used.join(","),
            next_seq,
            prev_hmac,
        );

        let mut mac =
            HmacSha256::new_from_slice(&self.hmac_key).map_err(|_| AuditError::HmacKey)?;
        mac.update(payload.as_bytes());
        let hmac_hex = hex::encode(mac.finalize().into_bytes());

        let cost_dec = Decimal::from_str(&format!("{:.6}", response.cost_usd))
            .unwrap_or_default();

        sqlx::query(
            r#"INSERT INTO llm_call_audit
               (tenant_id, catalog, bundle_version, sentence_hash, mode,
                provider_used, model_used, tokens_in, tokens_out, cost_usd,
                cache_hit, duration_ms, label, confidence, anchors_used,
                seq, prev_hmac, hmac)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
        )
        .bind(&request.tenant_id)
        .bind(&request.catalog)
        .bind(&request.bundle_version)
        .bind(sentence_hash)
        .bind(mode)
        .bind(&response.provider_name)
        .bind(&response.model_name)
        .bind(response.tokens_in as i32)
        .bind(response.tokens_out as i32)
        .bind(cost_dec)
        .bind(cache_hit)
        .bind(duration_ms as i32)
        .bind(label)
        .bind(confidence)
        .bind(anchors_used)
        .bind(next_seq)
        .bind(&prev_hmac)
        .bind(&hmac_hex)
        .execute(&*self.pool)
        .await?;

        chain.last_seq = next_seq;
        chain.last_hmac = hmac_hex;
        Ok(())
    }

    async fn bootstrap_chain(
        &self,
        tenant_id: &str,
        state: &mut TenantChainState,
    ) -> Result<(), AuditError> {
        let row: Option<(i64, String)> = sqlx::query_as(
            r#"SELECT seq, hmac FROM llm_call_audit
               WHERE tenant_id = $1
               ORDER BY seq DESC
               LIMIT 1"#,
        )
        .bind(tenant_id)
        .fetch_optional(&*self.pool)
        .await?;
        if let Some((seq, hmac)) = row {
            state.last_seq = seq;
            state.last_hmac = hmac;
        } else {
            state.last_seq = 0;
            state.last_hmac = "0".repeat(64);
        }
        state.initialised = true;
        Ok(())
    }
}
