//! HTTP client for the Microsoft Presidio sidecar.
//!
//! Presidio exposes two HTTP endpoints :
//! - `POST /analyze` : input text → list of detected entities.
//! - `POST /anonymize` : input text + analyzer results → redacted text.
//!
//! We use only the analyzer ; redaction is done in-process via
//! [`crate::redactor::redact`] for tighter control over the placeholder
//! format.

use crate::canonicalize::canonicalize_for_pii;
use crate::circuit_breaker::CircuitBreaker;
use crate::error::{PiiError, PiiResult};
use crate::metrics;
use crate::policy::{AnalysisOutcome, DetectedSpan, PolicyKind};
use crate::redactor;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

/// Configuration for [`PresidioClient`].
#[derive(Debug, Clone)]
pub struct PresidioConfig {
    /// Base URL of the Presidio analyzer service. Typically
    /// `http://efe-stack-pii.efe-system.svc.cluster.local:3000`.
    pub base_url: String,
    /// Per-request timeout. Default 100ms.
    pub timeout: Duration,
    /// Default analyzer language for inputs we can't otherwise detect.
    /// `fr` and `en` are the two we model-load by default.
    pub default_language: String,
    /// Recognizers to run. Empty means Presidio defaults.
    pub recognizers: Vec<String>,
}

impl Default for PresidioConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:3000".into(),
            timeout: Duration::from_millis(100),
            default_language: "en".into(),
            recognizers: vec![],
        }
    }
}

impl PresidioConfig {
    /// Build from env vars : `EFE_PRESIDIO_URL` (base URL),
    /// `EFE_PII_TIMEOUT_MS` (per-request timeout in ms),
    /// `EFE_PII_DEFAULT_LANG` (default analyzer language).
    /// Missing vars fall back to [`Self::default`].
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(url) = std::env::var("EFE_PRESIDIO_URL") {
            cfg.base_url = url;
        }
        if let Ok(ms) = std::env::var("EFE_PII_TIMEOUT_MS") {
            if let Ok(n) = ms.parse::<u64>() {
                cfg.timeout = Duration::from_millis(n);
            }
        }
        if let Ok(lang) = std::env::var("EFE_PII_DEFAULT_LANG") {
            cfg.default_language = lang;
        }
        cfg
    }
}

/// Async trait abstracting analysis for testing.
#[async_trait]
pub trait PiiAnalyzer: Send + Sync {
    /// Run the full pipeline : canonicalize → analyze → apply policy.
    async fn analyze(&self, text: &str, policy: PolicyKind) -> AnalysisOutcome;
}

/// Presidio HTTP client.
#[derive(Clone)]
pub struct PresidioClient {
    http: reqwest::Client,
    cfg: PresidioConfig,
    breaker: CircuitBreaker,
}

impl std::fmt::Debug for PresidioClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresidioClient")
            .field("base_url", &self.cfg.base_url)
            .field("timeout", &self.cfg.timeout)
            .field("breaker", &self.breaker.current_state())
            .finish()
    }
}

impl PresidioClient {
    /// Build a new client. Returns an error if the reqwest HTTP client
    /// can't be constructed (very rare ; only on TLS bootstrap issues).
    pub fn new(cfg: PresidioConfig) -> PiiResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(cfg.timeout)
            .user_agent("efe-pii/2.4.0")
            .build()
            .map_err(|e| PiiError::Http(format!("client build : {}", e)))?;
        Ok(Self {
            http,
            cfg,
            breaker: CircuitBreaker::new(),
        })
    }

    /// Low-level analyze call. Returns raw detections (not yet
    /// canonicalised). Use [`Self::analyze`] for the full pipeline.
    async fn presidio_analyze(&self, text: &str) -> PiiResult<Vec<DetectedSpan>> {
        if !self.breaker.allow() {
            return Err(PiiError::CircuitOpen);
        }
        let req = PresidioAnalyzeRequest {
            text: text.into(),
            language: self.cfg.default_language.clone(),
            entities: if self.cfg.recognizers.is_empty() {
                None
            } else {
                Some(self.cfg.recognizers.clone())
            },
            score_threshold: 0.3,
        };
        let url = format!("{}/analyze", self.cfg.base_url.trim_end_matches('/'));
        let started = std::time::Instant::now();
        let resp = self.http.post(&url).json(&req).send().await;
        let elapsed = started.elapsed();
        let outcome_label = if resp.is_ok() { "ok" } else { "fail" };
        metrics::ANALYZE_DURATION
            .with_label_values(&[outcome_label])
            .observe(elapsed.as_secs_f64());
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                self.breaker.record_failure();
                metrics::ANALYZE_FAILURES_TOTAL
                    .with_label_values(&["http"])
                    .inc();
                return Err(if e.is_timeout() {
                    PiiError::Timeout(self.cfg.timeout)
                } else {
                    PiiError::Http(e.to_string())
                });
            }
        };
        if !resp.status().is_success() {
            self.breaker.record_failure();
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            metrics::ANALYZE_FAILURES_TOTAL
                .with_label_values(&["status"])
                .inc();
            return Err(PiiError::BackendStatus { status, body });
        }
        let parsed: Vec<PresidioAnalyzeResultRow> = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                self.breaker.record_failure();
                metrics::ANALYZE_FAILURES_TOTAL
                    .with_label_values(&["decode"])
                    .inc();
                return Err(PiiError::Http(format!("response json : {}", e)));
            }
        };
        self.breaker.record_success();
        let spans: Vec<DetectedSpan> = parsed
            .into_iter()
            .map(|r| DetectedSpan {
                start: r.start,
                end: r.end,
                entity: r.entity_type,
                score: r.score,
            })
            .collect();
        Ok(spans)
    }
}

#[async_trait]
impl PiiAnalyzer for PresidioClient {
    async fn analyze(&self, raw_text: &str, policy: PolicyKind) -> AnalysisOutcome {
        if policy == PolicyKind::Off {
            return AnalysisOutcome::Clean(raw_text.to_string());
        }
        // Step 1 : canonicalize.
        let canonical = canonicalize_for_pii(raw_text);
        // Step 2 : analyze.
        let spans = match self.presidio_analyze(&canonical).await {
            Ok(s) => s,
            Err(PiiError::CircuitOpen) => {
                metrics::ANALYZE_CIRCUIT_BREAKS_TOTAL.inc();
                return AnalysisOutcome::Unavailable {
                    reason: "circuit_open".into(),
                };
            }
            Err(e) => {
                debug!(?e, "presidio analyze failed");
                return AnalysisOutcome::Unavailable {
                    reason: format!("{}", e),
                };
            }
        };
        if spans.is_empty() {
            return AnalysisOutcome::Clean(canonical);
        }
        // Update metrics per detection type.
        for sp in &spans {
            metrics::DETECTIONS_TOTAL
                .with_label_values(&[&sp.entity])
                .inc();
        }
        // Step 3 : apply policy.
        match policy {
            PolicyKind::Off => unreachable!(),
            PolicyKind::Detect => AnalysisOutcome::Detected {
                text: canonical,
                spans,
            },
            PolicyKind::Redact => {
                let redacted = redactor::redact(&canonical, &spans);
                AnalysisOutcome::Detected {
                    text: redacted,
                    spans,
                }
            }
            PolicyKind::Strict => {
                if spans.iter().any(|s| s.is_high_confidence()) {
                    for sp in &spans {
                        metrics::REJECTIONS_TOTAL
                            .with_label_values(&[&sp.entity])
                            .inc();
                    }
                    warn!(n = spans.len(), "strict policy rejected input");
                    AnalysisOutcome::Rejected { spans }
                } else {
                    AnalysisOutcome::Detected {
                        text: canonical,
                        spans,
                    }
                }
            }
        }
    }
}

// ── Wire types matching Presidio's HTTP shape ─────────────────────────

#[derive(Debug, Serialize)]
struct PresidioAnalyzeRequest {
    text: String,
    language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    entities: Option<Vec<String>>,
    #[serde(rename = "score_threshold")]
    score_threshold: f32,
}

#[derive(Debug, Deserialize)]
struct PresidioAnalyzeResultRow {
    start: usize,
    end: usize,
    entity_type: String,
    score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn off_policy_short_circuits() {
        let client = PresidioClient::new(PresidioConfig::default()).unwrap();
        let out = client.analyze("anything", PolicyKind::Off).await;
        assert!(matches!(out, AnalysisOutcome::Clean(s) if s == "anything"));
    }

    #[tokio::test]
    async fn presidio_unreachable_returns_unavailable() {
        // Default base_url = localhost:3000 (nothing listening).
        let cfg = PresidioConfig {
            base_url: "http://127.0.0.1:1/non-existent".into(),
            timeout: Duration::from_millis(50),
            ..Default::default()
        };
        let client = PresidioClient::new(cfg).unwrap();
        let out = client.analyze("user@example.com", PolicyKind::Detect).await;
        assert!(matches!(out, AnalysisOutcome::Unavailable { .. }));
    }

    // Φ-D-OPS post-audit fix B7 : `config_from_env_overrides` mutates
    // process-global env vars in #[test]. Cargo runs unit tests in
    // parallel — a sibling test reading `EFE_PRESIDIO_URL` (or any
    // env-derived state) races. Serialise via a module-local mutex,
    // mirroring `efe-server/src/v1/batch/worker.rs:134` pattern.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn config_from_env_overrides() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("EFE_PRESIDIO_URL", "http://test:9999");
        std::env::set_var("EFE_PII_TIMEOUT_MS", "200");
        let cfg = PresidioConfig::from_env();
        assert_eq!(cfg.base_url, "http://test:9999");
        assert_eq!(cfg.timeout, Duration::from_millis(200));
        std::env::remove_var("EFE_PRESIDIO_URL");
        std::env::remove_var("EFE_PII_TIMEOUT_MS");
    }
}
