//! Backpressure / load-shedding middleware.
//!
//! Two complementary mechanisms :
//!
//! 1. **Concurrency cap** — atomic counter of in-flight requests.
//!    Above `max_inflight`, new requests get HTTP 503 +
//!    `Retry-After: 1..5s` (random). Backed by a single
//!    `AtomicU64` per service instance.
//!
//! 2. **Latency-aware shedding** — sliding-window p99 latency observer.
//!    Above `p99_budget`, sheds `shed_percent`% of new requests
//!    randomly (returns 503 with the same shape).
//!
//! Together they implement graceful degradation : the service stays
//! responsive for the fraction of traffic it can serve well, instead
//! of accepting everything and tearing apart under load.
//!
//! Metrics :
//! - `efe_backpressure_dropped_total{reason}` — counter.
//! - `efe_backpressure_inflight` — gauge.
//! - `efe_backpressure_p99_ms` — gauge (last-sampled).
//!
//! Configuration via env (see [`BackpressureConfig::from_env`]) :
//! - `EFE_BACKPRESSURE_ENABLED` (default `true`).
//! - `EFE_BACKPRESSURE_MAX_INFLIGHT` (default `1000`).
//! - `EFE_BACKPRESSURE_P99_BUDGET_MS` (default `500`).
//! - `EFE_BACKPRESSURE_P99_WINDOW_S` (default `30`).
//! - `EFE_BACKPRESSURE_SHED_PERCENT` (default `10`).

use http::{Request, Response, StatusCode};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tower::{Layer, Service};
use tracing::{debug, warn};

/// Runtime configuration for backpressure.
#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    pub enabled: bool,
    pub max_inflight: u64,
    pub p99_budget_ms: u64,
    pub p99_window: Duration,
    pub shed_percent: u8,
    pub retry_after_min_s: u8,
    pub retry_after_max_s: u8,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_inflight: 1000,
            p99_budget_ms: 500,
            p99_window: Duration::from_secs(30),
            shed_percent: 10,
            retry_after_min_s: 1,
            retry_after_max_s: 5,
        }
    }
}

impl BackpressureConfig {
    /// Build from process environment, falling back to defaults.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("EFE_BACKPRESSURE_ENABLED") {
            cfg.enabled = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes");
        }
        if let Ok(v) = std::env::var("EFE_BACKPRESSURE_MAX_INFLIGHT") {
            if let Ok(n) = v.parse() {
                cfg.max_inflight = n;
            }
        }
        if let Ok(v) = std::env::var("EFE_BACKPRESSURE_P99_BUDGET_MS") {
            if let Ok(n) = v.parse() {
                cfg.p99_budget_ms = n;
            }
        }
        if let Ok(v) = std::env::var("EFE_BACKPRESSURE_P99_WINDOW_S") {
            if let Ok(n) = v.parse::<u64>() {
                cfg.p99_window = Duration::from_secs(n);
            }
        }
        if let Ok(v) = std::env::var("EFE_BACKPRESSURE_SHED_PERCENT") {
            if let Ok(n) = v.parse::<u8>() {
                cfg.shed_percent = n.min(100);
            }
        }
        cfg
    }
}

/// Sliding-window observer using a bounded VecDeque of (timestamp,
/// latency_ms) tuples. We compute p99 lazily on each shed decision by
/// sorting the window snapshot. The window is small (< 1000 samples
/// typically), so the cost is acceptable.
///
/// For high-RPS production, swap to HDRHistogram or t-digest. Marked
/// as a follow-on M28 optimization in the architecture doc.
struct P99Observer {
    window: VecDeque<(Instant, u32)>,
    capacity: usize,
    window_dur: Duration,
}

impl P99Observer {
    fn new(window: Duration) -> Self {
        Self {
            window: VecDeque::with_capacity(4096),
            capacity: 4096,
            window_dur: window,
        }
    }

    fn record(&mut self, now: Instant, latency_ms: u32) {
        self.window.push_back((now, latency_ms));
        // Trim from the front (old samples).
        let cutoff = now - self.window_dur;
        while let Some(&(t, _)) = self.window.front() {
            if t < cutoff {
                self.window.pop_front();
            } else {
                break;
            }
        }
        // Hard cap on capacity.
        while self.window.len() > self.capacity {
            self.window.pop_front();
        }
    }

    fn p99_ms(&self) -> u32 {
        if self.window.is_empty() {
            return 0;
        }
        let mut samples: Vec<u32> = self.window.iter().map(|(_, ms)| *ms).collect();
        samples.sort_unstable();
        let idx = ((samples.len() as f64) * 0.99).ceil() as usize - 1;
        let idx = idx.min(samples.len() - 1);
        samples[idx]
    }
}

struct BackpressureState {
    cfg: BackpressureConfig,
    inflight: AtomicU64,
    p99_obs: RwLock<P99Observer>,
}

impl BackpressureState {
    fn new(cfg: BackpressureConfig) -> Self {
        let win = cfg.p99_window;
        Self {
            cfg,
            inflight: AtomicU64::new(0),
            p99_obs: RwLock::new(P99Observer::new(win)),
        }
    }

    fn sample_random(&self) -> u8 {
        // Fast PRNG via process clock low bits. Good enough for shed
        // decisions ; not cryptographic.
        let nanos = Instant::now().elapsed().as_nanos() as u64;
        let mix = nanos.wrapping_mul(0x9E3779B97F4A7C15);
        ((mix >> 32) as u32 % 100) as u8
    }

    fn retry_after_seconds(&self) -> u8 {
        let lo = self.cfg.retry_after_min_s as u32;
        let hi = self.cfg.retry_after_max_s as u32;
        if hi <= lo {
            return lo as u8;
        }
        let range = hi - lo + 1;
        let nanos = Instant::now().elapsed().as_nanos() as u64;
        let pick = (nanos % range as u64) as u32 + lo;
        pick as u8
    }
}

/// Tower layer.
#[derive(Clone)]
pub struct BackpressureLayer {
    state: Arc<BackpressureState>,
}

impl BackpressureLayer {
    pub fn new(cfg: BackpressureConfig) -> Self {
        Self {
            state: Arc::new(BackpressureState::new(cfg)),
        }
    }

    /// Expose the inflight count for /metrics surfaces.
    pub fn inflight(&self) -> u64 {
        self.state.inflight.load(Ordering::Relaxed)
    }
}

impl<S> Layer<S> for BackpressureLayer {
    type Service = BackpressureService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        BackpressureService {
            inner,
            state: self.state.clone(),
        }
    }
}

#[derive(Clone)]
pub struct BackpressureService<S> {
    inner: S,
    state: Arc<BackpressureState>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for BackpressureService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let state = self.state.clone();
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        Box::pin(async move {
            if !state.cfg.enabled {
                return inner.call(req).await;
            }

            // ── 1. Concurrency cap ───────────────────────────────────
            // Increment inflight ; RAII guard decrements on drop.
            let current = state.inflight.fetch_add(1, Ordering::AcqRel) + 1;
            let _dec = InflightDecrementGuard {
                state: state.clone(),
            };
            backpressure_metrics::INFLIGHT
                .with_label_values(&[])
                .set(current as i64);

            if current > state.cfg.max_inflight {
                backpressure_metrics::DROPPED
                    .with_label_values(&["inflight"])
                    .inc();
                debug!(
                    current,
                    max = state.cfg.max_inflight,
                    "backpressure : inflight cap reached, 503"
                );
                return Ok(build_503(state.retry_after_seconds()));
            }

            // ── 2. Latency-aware shedding ────────────────────────────
            let p99 = state.p99_obs.read().await.p99_ms();
            backpressure_metrics::P99_MS
                .with_label_values(&[])
                .set(p99 as i64);
            if p99 > state.cfg.p99_budget_ms as u32 {
                let r = state.sample_random();
                if r < state.cfg.shed_percent {
                    backpressure_metrics::DROPPED
                        .with_label_values(&["p99_shed"])
                        .inc();
                    debug!(
                        p99,
                        budget = state.cfg.p99_budget_ms,
                        "backpressure : p99 above budget, shedding"
                    );
                    return Ok(build_503(state.retry_after_seconds()));
                }
            }

            // ── 3. Delegate, observe latency ─────────────────────────
            let started = Instant::now();
            let resp = inner.call(req).await;
            let elapsed = started.elapsed();
            let ms = elapsed.as_millis().min(u32::MAX as u128) as u32;
            state.p99_obs.write().await.record(Instant::now(), ms);
            resp
        })
    }
}

/// RAII guard that decrements `inflight` when dropped (request done
/// regardless of success or panic in the inner service).
struct InflightDecrementGuard {
    state: Arc<BackpressureState>,
}

impl Drop for InflightDecrementGuard {
    fn drop(&mut self) {
        self.state.inflight.fetch_sub(1, Ordering::AcqRel);
    }
}

fn build_503<B: Default>(retry_after: u8) -> Response<B> {
    let mut resp = Response::new(B::default());
    *resp.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    resp.headers_mut().insert(
        http::header::RETRY_AFTER,
        http::HeaderValue::from(retry_after as u32),
    );
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    resp
}

// ── Metrics submodule (scoped under crate::middleware) ──────────────

/// Backpressure-layer Prometheus metrics. Public so external callers
/// (e.g. main.rs at boot) can force lazy initialization.
pub mod backpressure_metrics {
    use once_cell::sync::Lazy;
    use prometheus::{
        register_int_counter_vec, register_int_gauge_vec, IntCounterVec, IntGaugeVec,
    };

    pub static DROPPED: Lazy<IntCounterVec> = Lazy::new(|| {
        register_int_counter_vec!(
            "efe_backpressure_dropped_total",
            "Requests dropped by backpressure middleware.",
            &["reason"],
        )
        .expect("register efe_backpressure_dropped_total")
    });

    pub static INFLIGHT: Lazy<IntGaugeVec> = Lazy::new(|| {
        register_int_gauge_vec!(
            "efe_backpressure_inflight",
            "Current in-flight requests through the backpressure layer.",
            &[],
        )
        .expect("register efe_backpressure_inflight")
    });

    pub static P99_MS: Lazy<IntGaugeVec> = Lazy::new(|| {
        register_int_gauge_vec!(
            "efe_backpressure_p99_ms",
            "Last-sampled p99 latency (ms) observed by backpressure layer.",
            &[],
        )
        .expect("register efe_backpressure_p99_ms")
    });

    /// Force lazy init at boot.
    pub fn init() {
        let _ = &*DROPPED;
        let _ = &*INFLIGHT;
        let _ = &*P99_MS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_sensible() {
        let c = BackpressureConfig::default();
        assert!(c.enabled);
        assert_eq!(c.max_inflight, 1000);
        assert_eq!(c.p99_budget_ms, 500);
        assert_eq!(c.shed_percent, 10);
    }

    #[test]
    fn p99_observer_empty_returns_zero() {
        let o = P99Observer::new(Duration::from_secs(30));
        assert_eq!(o.p99_ms(), 0);
    }

    #[test]
    fn p99_observer_basic() {
        let mut o = P99Observer::new(Duration::from_secs(30));
        let t0 = Instant::now();
        // 90 samples at 10ms, 10 samples at 1000ms = 100 total.
        // Sorted : indices 0..=89 = 10, 90..=99 = 1000.
        // p99 index = ceil(100 * 0.99) - 1 = 99 - 1 = 98. samples[98] = 1000.
        for _ in 0..90 {
            o.record(t0, 10);
        }
        for _ in 0..10 {
            o.record(t0, 1000);
        }
        let p99 = o.p99_ms();
        assert_eq!(
            p99, 1000,
            "with 10% outliers, p99 should pick the high tail ; got {}",
            p99
        );
    }

    #[test]
    fn p99_observer_low_outlier_stays_low() {
        // 99 at 10ms + 1 at 1000ms → p99 (90th index) is still 10.
        // This is the strict definition : "the value such that 99% of
        // samples are ≤ it".
        let mut o = P99Observer::new(Duration::from_secs(30));
        let t0 = Instant::now();
        for _ in 0..99 {
            o.record(t0, 10);
        }
        o.record(t0, 1000);
        let p99 = o.p99_ms();
        // Conservative percentile : 1 outlier in 100 is below the p99 cutoff.
        assert_eq!(p99, 10);
    }

    #[test]
    fn p99_observer_trims_old_samples() {
        let mut o = P99Observer::new(Duration::from_millis(50));
        let t0 = Instant::now();
        o.record(t0, 999);
        std::thread::sleep(Duration::from_millis(60));
        let t1 = Instant::now();
        o.record(t1, 10);
        // Old sample should be evicted.
        assert!(o.p99_ms() < 100, "old high sample should have been trimmed");
    }

    #[test]
    fn retry_after_within_range() {
        let cfg = BackpressureConfig::default();
        let state = BackpressureState::new(cfg.clone());
        for _ in 0..50 {
            let r = state.retry_after_seconds();
            assert!(r >= cfg.retry_after_min_s && r <= cfg.retry_after_max_s);
        }
    }
}
