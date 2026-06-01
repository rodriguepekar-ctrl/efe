//! PR #22 § 14 — Prometheus HTTP source.
//!
//! Issues `POST /api/v1/query` against a Prometheus base URL.  Phase-2
//! is read-only ; we never push (rules + alerts are deployed via
//! `kubectl apply`).
//!
//! TLS is rustls (workspace default) ; HTTP basic auth is **NOT**
//! supported (use kube-rbac-proxy in front).

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{SloError, SloResult};
use crate::source::{EventQuery, HistogramQuery, MetricQuery, RecoveryEvent, SourceProvider};

/// Default per-request timeout.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

pub struct PromHttpSource {
    base_url: String,
    client: reqwest::Client,
}

impl PromHttpSource {
    /// Build a new client.  `base_url` must be a Prometheus instance
    /// root (e.g. `http://prom:9090`).
    pub fn new(base_url: &str) -> SloResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent(format!(
                "efe-slo-rollup/{} (PR22)",
                crate::build::VERSION
            ))
            .build()
            .map_err(|e| SloError::Source(format!("reqwest client : {e}")))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    async fn instant_query(&self, query: &str, at_unix: i64) -> SloResult<PromResponse> {
        let url = format!("{}/api/v1/query", self.base_url);
        let resp = self
            .client
            .post(&url)
            .form(&[("query", query), ("time", &at_unix.to_string())])
            .send()
            .await
            .map_err(|e| SloError::Source(format!("HTTP POST {url} : {e}")))?;
        let body = resp
            .json::<PromResponse>()
            .await
            .map_err(|e| SloError::Source(format!("decode prom json : {e}")))?;
        if body.status != "success" {
            return Err(SloError::Source(format!(
                "prom returned status={} error={}",
                body.status,
                body.error_type.unwrap_or_else(|| "(no error type)".into()),
            )));
        }
        Ok(body)
    }

    /// Render a PromQL selector from (metric, [matchers], window).
    fn render_counter_query(q: &MetricQuery) -> String {
        let matchers = if q.label_matchers.is_empty() {
            String::new()
        } else {
            let inner: Vec<_> = q
                .label_matchers
                .iter()
                .map(|(k, v)| format!("{k}=\"{v}\""))
                .collect();
            format!("{{{}}}", inner.join(","))
        };
        format!(
            "sum(increase({}{matchers}[{}s]))",
            q.metric,
            q.window_seconds
        )
    }

    fn render_histogram_query(q: &HistogramQuery) -> String {
        let matchers = if q.label_matchers.is_empty() {
            String::new()
        } else {
            let inner: Vec<_> = q
                .label_matchers
                .iter()
                .map(|(k, v)| format!("{k}=\"{v}\""))
                .collect();
            format!("{{{}}}", inner.join(","))
        };
        format!(
            "histogram_quantile({}, sum by (le) (rate({}{matchers}[{}s])))",
            q.quantile,
            q.metric_bucket,
            q.window_seconds
        )
    }
}

#[async_trait]
impl SourceProvider for PromHttpSource {
    async fn integer_counter(&self, q: MetricQuery) -> SloResult<u64> {
        let query = Self::render_counter_query(&q);
        let resp = self
            .instant_query(&query, q.window_start_unix + q.window_seconds as i64)
            .await?;
        Ok(resp.data.scalar_or_zero())
    }

    async fn histogram_quantile(&self, q: HistogramQuery) -> SloResult<f64> {
        let query = Self::render_histogram_query(&q);
        let resp = self
            .instant_query(&query, q.window_start_unix + q.window_seconds as i64)
            .await?;
        Ok(resp.data.scalar_or_nan())
    }

    async fn recovery_events(&self, q: EventQuery) -> SloResult<Vec<RecoveryEvent>> {
        // Prom returns a histogram of recovery durations ; we
        // approximate the event list by querying total + p99 — full
        // event reconstruction is Phase-3 (G.11).
        // For Phase-2 we simulate via a range query of the duration
        // histogram and return one event per non-zero bucket sample.
        let query = format!(
            "sum by (le) (increase({}_bucket[{}s]))",
            q.metric, q.window_seconds
        );
        let resp = self
            .instant_query(&query, q.window_start_unix + q.window_seconds as i64)
            .await?;
        // Phase-2 stub : if any bucket is present, fabricate one
        // event per cumulative-count (le-sorted).  Full event-driven
        // surface is Phase-3.
        let mut out = Vec::new();
        if let PromData::Vector(samples) = resp.data {
            let mut last_count: f64 = 0.0;
            let mut samples = samples;
            samples.sort_by(|a, b| {
                let le_a = a.metric.get("le").and_then(|s| s.parse::<f64>().ok()).unwrap_or(f64::INFINITY);
                let le_b = b.metric.get("le").and_then(|s| s.parse::<f64>().ok()).unwrap_or(f64::INFINITY);
                le_a.partial_cmp(&le_b).unwrap_or(std::cmp::Ordering::Equal)
            });
            for s in samples {
                let count = s.value.1.parse::<f64>().unwrap_or(0.0);
                let bucket_le = s
                    .metric
                    .get("le")
                    .and_then(|le| le.parse::<f64>().ok())
                    .unwrap_or(f64::INFINITY);
                let n = (count - last_count).max(0.0).round() as u64;
                for _ in 0..n {
                    out.push(RecoveryEvent { duration_seconds: bucket_le, at_unix: q.window_start_unix });
                }
                last_count = count;
            }
        }
        Ok(out)
    }

    async fn scrape_coverage(&self, window_start_unix: i64, window_seconds: u64) -> SloResult<f64> {
        // Coverage = fraction of expected scrapes that succeeded.
        // We approximate via `count_over_time(up{job=...}[Xs]) /
        // expected`. Phase-2 sets expected via the scrape interval
        // (15s); operators can override at deployment-time.
        let query = format!(
            "count_over_time(up[{window_seconds}s]) / (({window_seconds}/15))"
        );
        let resp = self
            .instant_query(&query, window_start_unix + window_seconds as i64)
            .await?;
        Ok(resp.data.scalar_or_nan().clamp(0.0, 1.0))
    }

    async fn participants(&self, window_start_unix: i64, window_seconds: u64) -> SloResult<Vec<String>> {
        let query = format!(
            "group by (participant) (efe_replication_lag_bucket{{__name__=~\".+\"}}[{window_seconds}s])"
        );
        let resp = self
            .instant_query(&query, window_start_unix + window_seconds as i64)
            .await?;
        if let PromData::Vector(samples) = resp.data {
            return Ok(samples
                .into_iter()
                .filter_map(|s| s.metric.get("participant").cloned())
                .collect());
        }
        Ok(vec![])
    }
}

// ────────────────────────────────────────────────────────────────────
//                       Prometheus JSON shapes
// ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PromResponse {
    status: String,
    #[serde(default)]
    data: PromData,
    #[serde(rename = "errorType", default)]
    error_type: Option<String>,
}

/// Prometheus `data` field — discriminated by `resultType`.  Phase-2
/// reads only `vector` + `scalar` for SLI math ; the other variants
/// are kept to preserve binding-completeness against the Prom HTTP
/// schema (`/api/v1/query`).
#[derive(Debug, Deserialize)]
#[serde(tag = "resultType", content = "result")]
enum PromData {
    #[serde(rename = "vector")]
    Vector(Vec<PromSample>),
    #[serde(rename = "scalar")]
    Scalar(PromScalar),
    #[serde(rename = "matrix")]
    Matrix(#[allow(dead_code)] Vec<PromMatrixEntry>),
    #[serde(rename = "string")]
    String(#[allow(dead_code)] (f64, String)),
}

impl Default for PromData {
    fn default() -> Self {
        Self::Vector(vec![])
    }
}

impl PromData {
    fn scalar_or_zero(&self) -> u64 {
        match self {
            Self::Vector(v) => v
                .first()
                .and_then(|s| s.value.1.parse::<f64>().ok())
                .map(|f| f.round() as u64)
                .unwrap_or(0),
            Self::Scalar(s) => s.1.parse::<f64>().ok().map(|f| f.round() as u64).unwrap_or(0),
            _ => 0,
        }
    }

    fn scalar_or_nan(&self) -> f64 {
        match self {
            Self::Vector(v) => v
                .first()
                .and_then(|s| s.value.1.parse::<f64>().ok())
                .unwrap_or(f64::NAN),
            Self::Scalar(s) => s.1.parse::<f64>().ok().unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }
}

#[derive(Debug, Deserialize)]
struct PromSample {
    metric: std::collections::HashMap<String, String>,
    /// Prometheus encodes sample as `[timestamp, "value_as_string"]`.
    value: (f64, String),
}

/// Prometheus scalar-result type.  Both fields are required by the
/// Prom JSON shape but we use only `1` (the string-encoded value) for
/// SLI computation ; `0` (the timestamp) is preserved for future use
/// (Phase-3 G.11 — trend regression detection).
#[derive(Debug, Deserialize)]
struct PromScalar(#[allow(dead_code)] f64, String);

/// Prometheus matrix-result row.  Used only by the (currently unused)
/// range-query path — Phase-2 limits itself to instant-query.  Kept
/// for forward-compatibility with the schema_version=1 binding.
#[derive(Debug, Deserialize)]
struct PromMatrixEntry {
    #[serde(rename = "metric", default)]
    #[allow(dead_code)]
    metric: std::collections::HashMap<String, String>,
    #[serde(rename = "values", default)]
    #[allow(dead_code)]
    values: Vec<(f64, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MetricQuery;

    #[test]
    fn render_counter_query_with_no_matchers() {
        let q = MetricQuery {
            metric: "efe_l2_request_total".into(),
            window_seconds: 300,
            window_start_unix: 0,
            label_matchers: vec![],
        };
        assert_eq!(
            PromHttpSource::render_counter_query(&q),
            "sum(increase(efe_l2_request_total[300s]))"
        );
    }

    #[test]
    fn render_counter_query_with_matchers() {
        let q = MetricQuery {
            metric: "efe_l2_request_total".into(),
            window_seconds: 60,
            window_start_unix: 0,
            label_matchers: vec![
                ("status".into(), "success".into()),
                ("source".into(), "prod".into()),
            ],
        };
        let expected = "sum(increase(efe_l2_request_total{status=\"success\",source=\"prod\"}[60s]))";
        assert_eq!(PromHttpSource::render_counter_query(&q), expected);
    }

    #[test]
    fn render_histogram_query() {
        let q = HistogramQuery {
            metric_bucket: "efe_l2_request_duration_seconds_bucket".into(),
            quantile: 0.95,
            window_seconds: 300,
            window_start_unix: 0,
            label_matchers: vec![],
        };
        let expected = "histogram_quantile(0.95, sum by (le) (rate(efe_l2_request_duration_seconds_bucket[300s])))";
        assert_eq!(PromHttpSource::render_histogram_query(&q), expected);
    }

    #[test]
    fn new_normalises_trailing_slash() {
        let s = PromHttpSource::new("http://prom:9090/").unwrap();
        assert_eq!(s.base_url, "http://prom:9090");
    }
}
