//! Drift detection engine.
//!
//! Detects degradation in classification quality per (tenant, catalog, class)
//! using KL divergence on confidence histograms (windowed comparison).
//!
//! See § 9 of `EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`.

use crate::{CurationError, CurationResult, TenantCurationFlags, TenantId, CatalogId, ClassId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

const HISTOGRAM_BINS: usize = 10;
const MIN_SAMPLES: usize = 100;
const KL_LAPLACE_EPSILON: f64 = 1e-9;

/// Snapshot of a class's confidence distribution over a time window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftSnapshot {
    pub class_id: ClassId,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub n_samples: usize,
    pub confidence_histogram: [u32; HISTOGRAM_BINS],
    pub mean_confidence: f64,
    pub support: usize,
}

/// A detected drift event ready for persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftEvent {
    pub id: Uuid,
    pub tenant_id: TenantId,
    pub catalog_id: CatalogId,
    pub class_id: ClassId,
    pub kl_divergence: f64,
    pub mean_confidence_old: f64,
    pub mean_confidence_new: f64,
    pub support_change_pct: f64,
    pub window_old_start: DateTime<Utc>,
    pub window_new_end: DateTime<Utc>,
    pub severity: DriftSeverity,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DriftSeverity { Low, Medium, High }

impl std::fmt::Display for DriftSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Low => write!(f, "LOW"), Self::Medium => write!(f, "MEDIUM"), Self::High => write!(f, "HIGH") }
    }
}

/// Compute KL divergence KL(p_new || p_old) on smoothed histograms.
///
/// KL is non-negative ; returns `f64::NAN` only if either histogram is empty.
/// Uses Laplace smoothing (`KL_LAPLACE_EPSILON`) to avoid `log(0)`.
pub fn compute_kl_divergence(old: &DriftSnapshot, new: &DriftSnapshot) -> f64 {
    let p_old = normalize_with_smoothing(&old.confidence_histogram);
    let p_new = normalize_with_smoothing(&new.confidence_histogram);
    p_new.iter().zip(p_old.iter())
        .map(|(p, q)| if *p > KL_LAPLACE_EPSILON { p * (p / q).ln() } else { 0.0 })
        .sum()
}

fn normalize_with_smoothing(hist: &[u32; HISTOGRAM_BINS]) -> [f64; HISTOGRAM_BINS] {
    let total: f64 = hist.iter().map(|&v| v as f64).sum::<f64>() + (HISTOGRAM_BINS as f64) * KL_LAPLACE_EPSILON;
    let mut out = [0.0; HISTOGRAM_BINS];
    for (i, &v) in hist.iter().enumerate() {
        out[i] = (v as f64 + KL_LAPLACE_EPSILON) / total;
    }
    out
}

/// Classify drift severity based on KL divergence and confidence drop.
pub fn classify_severity(kl: f64, conf_drop: f64) -> DriftSeverity {
    if kl > 0.40 || conf_drop.abs() > 0.20 { DriftSeverity::High }
    else if kl > 0.25 && conf_drop > 0.10 { DriftSeverity::Medium }
    else { DriftSeverity::Low }
}

/// Query a drift snapshot for a (tenant, catalog, class) over the last `window_days`.
pub async fn query_snapshot(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId, class: &ClassId, window_days: i64,
) -> CurationResult<DriftSnapshot> {
    let window_end = Utc::now();
    let window_start = window_end - Duration::days(window_days);
    let rows = sqlx::query(
        r#"
        SELECT confidence
        FROM llm_call_audit
        WHERE tenant_id = $1
          AND catalog_id = $2
          AND verdict_class_id = $3
          AND created_at >= $4
          AND created_at < $5
        "#,
    )
    .bind(&tenant.0).bind(&catalog.0).bind(&class.0)
    .bind(window_start).bind(window_end)
    .fetch_all(pg).await?;

    let n_samples = rows.len();
    let mut hist = [0u32; HISTOGRAM_BINS];
    let mut sum = 0.0_f64;
    for row in &rows {
        let conf: f64 = row.try_get::<f64, _>("confidence").unwrap_or(0.0);
        let bin = ((conf * HISTOGRAM_BINS as f64) as usize).min(HISTOGRAM_BINS - 1);
        hist[bin] += 1;
        sum += conf;
    }
    let mean_confidence = if n_samples > 0 { sum / n_samples as f64 } else { 0.0 };

    Ok(DriftSnapshot {
        class_id: class.clone(),
        window_start, window_end, n_samples,
        confidence_histogram: hist, mean_confidence, support: n_samples,
    })
}

/// Detect drift on a (tenant, catalog, class). Returns `Ok(None)` if either
/// window has < `MIN_SAMPLES` (insufficient signal to claim drift).
pub async fn detect_drift(
    pg: &PgPool, tenant: &TenantId, catalog: &CatalogId, class: &ClassId,
    flags: &TenantCurationFlags,
) -> CurationResult<Option<DriftEvent>> {
    let new = query_snapshot(pg, tenant, catalog, class, 7).await?;
    let old_window_days = 30; // includes the recent 7 days (overlap acceptable for stability)
    let old = query_snapshot(pg, tenant, catalog, class, old_window_days).await?;

    if new.n_samples < MIN_SAMPLES || old.n_samples < MIN_SAMPLES {
        return Ok(None);
    }

    let kl = compute_kl_divergence(&old, &new);
    let conf_drop = old.mean_confidence - new.mean_confidence;
    let support_change_pct = if old.support > 0 {
        (new.support as f64 - old.support as f64) / old.support as f64
    } else { 0.0 };

    let triggered = kl > flags.drift_kl_threshold
        || conf_drop > flags.drift_conf_drop_threshold
        || support_change_pct.abs() > flags.drift_support_threshold_pct;

    if !triggered { return Ok(None); }

    Ok(Some(DriftEvent {
        id: Uuid::new_v4(),
        tenant_id: tenant.clone(), catalog_id: catalog.clone(), class_id: class.clone(),
        kl_divergence: kl,
        mean_confidence_old: old.mean_confidence, mean_confidence_new: new.mean_confidence,
        support_change_pct,
        window_old_start: old.window_start, window_new_end: new.window_end,
        severity: classify_severity(kl, conf_drop),
        detected_at: Utc::now(),
    }))
}

/// Persist a drift event to the database.
pub async fn persist_drift_event(pg: &PgPool, event: &DriftEvent) -> CurationResult<()> {
    sqlx::query(
        r#"
        INSERT INTO taxonomy_drift_event (
            id, tenant_id, catalog_id, class_id, detected_at,
            window_old_start, window_new_end, kl_divergence,
            mean_confidence_old, mean_confidence_new, support_change_pct, severity
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::drift_severity)
        "#,
    )
    .bind(event.id).bind(&event.tenant_id.0).bind(&event.catalog_id.0)
    .bind(&event.class_id.0).bind(event.detected_at)
    .bind(event.window_old_start).bind(event.window_new_end)
    .bind(event.kl_divergence).bind(event.mean_confidence_old)
    .bind(event.mean_confidence_new).bind(event.support_change_pct)
    .bind(event.severity.to_string())
    .execute(pg).await?;
    Ok(())
}

/// Scheduled drift detection loop. Runs every `interval_secs` seconds (default 6h).
pub async fn drift_scheduler_loop(pg: Arc<PgPool>, tenants: Arc<dyn TenantRegistry>, interval_secs: u64) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        if let Err(e) = scan_all_tenants(&pg, tenants.as_ref()).await {
            tracing::error!(?e, "drift scheduler scan failed");
        }
    }
}

async fn scan_all_tenants(pg: &PgPool, tenants: &dyn TenantRegistry) -> CurationResult<()> {
    for tenant_ctx in tenants.iter_active_with_flags().await? {
        if !tenant_ctx.flags.drift_detection_enabled { continue; }
        // Set per-session tenant context for RLS.
        sqlx::query("SELECT set_config('app.current_tenant_id', $1, true)")
            .bind(&tenant_ctx.id.0).execute(pg).await?;
        for cat in tenant_ctx.catalogs.iter() {
            for class in cat.classes.iter() {
                match detect_drift(pg, &tenant_ctx.id, &cat.id, class, &tenant_ctx.flags).await {
                    Ok(Some(event)) => {
                        if let Err(e) = persist_drift_event(pg, &event).await {
                            tracing::error!(?e, ?event.id, "failed to persist drift event");
                            continue;
                        }
                        tracing::info!(
                            tenant = %tenant_ctx.id, catalog = %cat.id.0, class = %class.0,
                            kl = event.kl_divergence, severity = %event.severity,
                            "drift event persisted"
                        );
                        crate::metrics::DRIFT_EVENTS_TOTAL
                            .with_label_values(&[&tenant_ctx.id.0, &event.severity.to_string()])
                            .inc();
                    }
                    Ok(None) => {
                        crate::metrics::DRIFT_NO_EVENT_TOTAL
                            .with_label_values(&[&tenant_ctx.id.0]).inc();
                    }
                    Err(e) => {
                        tracing::error!(?e, "drift detection failed for class");
                        crate::metrics::DRIFT_DETECTION_ERRORS_TOTAL.inc();
                    }
                }
            }
        }
    }
    Ok(())
}

/// Trait for the tenant registry — abstracted to support mocking in tests.
#[async_trait::async_trait]
pub trait TenantRegistry: Send + Sync {
    async fn iter_active_with_flags(&self) -> CurationResult<Vec<TenantContext>>;
}

pub struct TenantContext {
    pub id: TenantId,
    pub flags: TenantCurationFlags,
    pub catalogs: Vec<CatalogContext>,
}

pub struct CatalogContext { pub id: CatalogId, pub classes: Vec<ClassId> }

// Metrics live in the top-level `crate::metrics` module ; the `drift_scheduler_loop`
// references them via `crate::metrics::DRIFT_*`. Removed the orphan
// `metrics_compat` shim that duplicated those definitions.

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn snap(hist: [u32; HISTOGRAM_BINS], n: usize, mean: f64) -> DriftSnapshot {
        DriftSnapshot {
            class_id: ClassId("c".into()),
            window_start: Utc::now() - Duration::days(7),
            window_end: Utc::now(),
            n_samples: n,
            confidence_histogram: hist,
            mean_confidence: mean,
            support: n,
        }
    }

    #[test]
    fn test_drift_kl_zero_for_identical_distributions() {
        let h = [10, 10, 10, 10, 10, 10, 10, 10, 10, 10];
        let s = snap(h, 100, 0.5);
        let kl = compute_kl_divergence(&s, &s);
        assert!(kl.abs() < 1e-9, "KL should be ~0 for identical distributions, got {}", kl);
    }

    #[test]
    fn test_drift_kl_positive_for_different_distributions() {
        let h_low = [50, 30, 10, 5, 2, 1, 1, 0, 0, 1]; // mostly low confidence
        let h_high = [1, 0, 0, 1, 2, 5, 10, 30, 50, 1]; // mostly high
        let s_old = snap(h_low, 100, 0.2);
        let s_new = snap(h_high, 100, 0.8);
        let kl = compute_kl_divergence(&s_old, &s_new);
        assert!(kl > 1.0, "KL should be large for divergent distributions, got {}", kl);
    }

    #[test]
    fn test_drift_severity_classification() {
        assert_eq!(classify_severity(0.10, 0.05), DriftSeverity::Low);
        assert_eq!(classify_severity(0.30, 0.15), DriftSeverity::Medium);
        assert_eq!(classify_severity(0.50, 0.05), DriftSeverity::High);
        assert_eq!(classify_severity(0.10, 0.30), DriftSeverity::High);
    }

    #[test]
    fn test_drift_kl_handles_zero_bin_with_smoothing() {
        let mut h_a = [10u32; HISTOGRAM_BINS]; h_a[5] = 0;
        let mut h_b = [10u32; HISTOGRAM_BINS]; h_b[5] = 100;
        let s_a = snap(h_a, 90, 0.5);
        let s_b = snap(h_b, 190, 0.55);
        let kl = compute_kl_divergence(&s_a, &s_b);
        assert!(kl.is_finite(), "KL should be finite even with zero bins (Laplace smoothing)");
        assert!(kl > 0.0);
    }

    #[test]
    fn test_drift_severity_low_default() {
        assert_eq!(classify_severity(0.0, 0.0), DriftSeverity::Low);
    }
}
