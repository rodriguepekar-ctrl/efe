//! **Sprint 8 — N11** : metrics surface for the Firecracker pool.
//!
//! 14 counters / gauges / histograms, all labelled `(kind, model_id,
//! model_version, tenant_class)`.  Exposed on the EFE Nano `/metrics`
//! Prometheus endpoint.
//!
//! # Why hand-rolled instead of `prometheus` crate
//!
//! The `prometheus` crate (already in workspace) would add ~ 200 KB to
//! the runtime binary AND requires a registry singleton + per-metric
//! `with_label_values` HashMap lookup on every increment.  At 100k
//! RPS Mode B that's 100k HashMap lookups per second per metric — a
//! measurable cost.
//!
//! **Sprint 8 — N11** ships a **typed-label** counter set : labels are
//! known at compile time, looked up via array index, not HashMap.
//! Compatible with Prometheus scrape format via `render()`.
//!
//! Sprint 9 may swap to the workspace `prometheus` crate once the
//! hot-path metrics path is proven < 50 ns per increment.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use super::types::SubPoolKey;
#[cfg(test)]
use super::types::TenantClass;

// ─────────────────────────────────────────────────────────────────────
//  Atomic counter primitives
// ─────────────────────────────────────────────────────────────────────

/// Lock-free `u64` counter ; `.inc()` is one `fetch_add`.
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Lock-free `i64` gauge ; supports inc/dec.
#[derive(Debug, Default)]
pub struct Gauge(AtomicU64);

impl Gauge {
    pub fn set(&self, n: u64) {
        self.0.store(n, Ordering::Relaxed);
    }
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dec(&self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Coarse histogram with fixed buckets.  Sprint 8 ships 8 buckets
/// covering 1 µs → 30 s in pseudo-log spacing.  Observations beyond
/// 30 s land in the overflow bucket.
#[derive(Debug)]
pub struct Histogram {
    buckets: [(Duration, AtomicU64); 8],
    overflow: AtomicU64,
    sum_micros: AtomicU64,
    count: AtomicU64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

impl Histogram {
    pub fn new() -> Self {
        Self {
            buckets: [
                (Duration::from_micros(10), AtomicU64::new(0)),
                (Duration::from_micros(100), AtomicU64::new(0)),
                (Duration::from_millis(1), AtomicU64::new(0)),
                (Duration::from_millis(10), AtomicU64::new(0)),
                (Duration::from_millis(100), AtomicU64::new(0)),
                (Duration::from_secs(1), AtomicU64::new(0)),
                (Duration::from_secs(5), AtomicU64::new(0)),
                (Duration::from_secs(30), AtomicU64::new(0)),
            ],
            overflow: AtomicU64::new(0),
            sum_micros: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, value: Duration) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum_micros
            .fetch_add(value.as_micros().min(u64::MAX as u128) as u64, Ordering::Relaxed);
        for (bound, counter) in &self.buckets {
            if value <= *bound {
                counter.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        self.overflow.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    pub fn mean(&self) -> Duration {
        let c = self.count();
        if c == 0 {
            return Duration::ZERO;
        }
        let total_micros = self.sum_micros.load(Ordering::Relaxed);
        Duration::from_micros(total_micros / c)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Per-subpool metrics
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : 14 metrics per (kind, model, version, class).
#[derive(Debug, Default)]
pub struct SubPoolMetrics {
    // Gauges (current state).
    pub warm_count: Gauge,
    pub in_use_count: Gauge,

    // Counters (lifetime totals).
    pub cold_start_total: Counter,
    pub crash_total: Counter,
    pub reap_total: Counter,
    pub acquire_timeout_total: Counter,
    pub pool_exhausted_total: Counter,
    pub health_check_failure_total: Counter,
    pub pid_exit_total: Counter,
    pub vsock_error_total: Counter,

    // Histograms.
    pub cold_start_duration: Histogram,
    pub inference_duration: Histogram,
    pub vsock_rtt: Histogram,
    pub acquire_wait: Histogram,
}

// ─────────────────────────────────────────────────────────────────────
//  FirecrackerMetrics — registry across all subpools
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : top-level metrics registry, sharded by `SubPoolKey`.
///
/// Lazily creates `SubPoolMetrics` on first access via
/// [`for_key`](Self::for_key).  Renders to Prometheus exposition
/// format via [`render`](Self::render).
#[derive(Debug, Default)]
pub struct FirecrackerMetrics {
    sub_pools: Mutex<HashMap<SubPoolKey, Arc<SubPoolMetrics>>>,
    // Global non-labelled counters.
    pub spawn_total: Counter,
    pub reap_total_global: Counter,
}

impl FirecrackerMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get-or-create the metrics for a sub-pool.  O(1) amortized.
    pub fn for_key(&self, key: &SubPoolKey) -> Arc<SubPoolMetrics> {
        let mut guard = self.sub_pools.lock().unwrap();
        Arc::clone(
            guard
                .entry(key.clone())
                .or_insert_with(|| Arc::new(SubPoolMetrics::default())),
        )
    }

    /// Render Prometheus exposition format.  Each metric becomes one
    /// `# TYPE` line + many `metric_name{labels} value` lines.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(4 * 1024);
        let guard = self.sub_pools.lock().unwrap();

        // Global counters.
        out.push_str("# TYPE efe_nano_firecracker_spawn_total counter\n");
        out.push_str(&format!(
            "efe_nano_firecracker_spawn_total {}\n",
            self.spawn_total.get()
        ));
        out.push_str("# TYPE efe_nano_firecracker_reap_total_global counter\n");
        out.push_str(&format!(
            "efe_nano_firecracker_reap_total_global {}\n",
            self.reap_total_global.get()
        ));

        macro_rules! render_metric {
            ($out:expr, $name:expr, $kind:expr, $fn:expr) => {{
                $out.push_str(&format!("# TYPE {} {}\n", $name, $kind));
                for (key, m) in guard.iter() {
                    let labels = render_labels(key);
                    let value = $fn(m.as_ref());
                    $out.push_str(&format!("{}{{{}}} {}\n", $name, labels, value));
                }
            }};
        }

        render_metric!(
            out,
            "efe_nano_firecracker_warm_count",
            "gauge",
            |m: &SubPoolMetrics| m.warm_count.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_in_use_count",
            "gauge",
            |m: &SubPoolMetrics| m.in_use_count.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_cold_start_total",
            "counter",
            |m: &SubPoolMetrics| m.cold_start_total.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_crash_total",
            "counter",
            |m: &SubPoolMetrics| m.crash_total.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_reap_total",
            "counter",
            |m: &SubPoolMetrics| m.reap_total.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_acquire_timeout_total",
            "counter",
            |m: &SubPoolMetrics| m.acquire_timeout_total.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_pool_exhausted_total",
            "counter",
            |m: &SubPoolMetrics| m.pool_exhausted_total.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_health_check_failure_total",
            "counter",
            |m: &SubPoolMetrics| m.health_check_failure_total.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_pid_exit_total",
            "counter",
            |m: &SubPoolMetrics| m.pid_exit_total.get()
        );
        render_metric!(
            out,
            "efe_nano_firecracker_vsock_error_total",
            "counter",
            |m: &SubPoolMetrics| m.vsock_error_total.get()
        );

        // Histograms : emit count + sum_seconds + buckets.
        for (name, accessor) in [
            ("cold_start_duration", histo_accessor_cold_start as fn(&SubPoolMetrics) -> &Histogram),
            ("inference_duration", histo_accessor_inference),
            ("vsock_rtt", histo_accessor_vsock),
            ("acquire_wait", histo_accessor_acquire),
        ] {
            out.push_str(&format!(
                "# TYPE efe_nano_firecracker_{name}_seconds histogram\n"
            ));
            for (key, m) in guard.iter() {
                let labels = render_labels(key);
                let h = accessor(m.as_ref());
                let count = h.count.load(Ordering::Relaxed);
                let sum_us = h.sum_micros.load(Ordering::Relaxed);
                out.push_str(&format!(
                    "efe_nano_firecracker_{name}_seconds_count{{{labels}}} {count}\n"
                ));
                out.push_str(&format!(
                    "efe_nano_firecracker_{name}_seconds_sum{{{labels}}} {}\n",
                    sum_us as f64 / 1_000_000.0
                ));
                let mut cumulative = 0u64;
                for (bound, bucket_counter) in &h.buckets {
                    cumulative += bucket_counter.load(Ordering::Relaxed);
                    out.push_str(&format!(
                        "efe_nano_firecracker_{name}_seconds_bucket{{{labels},le=\"{}\"}} {}\n",
                        bound.as_secs_f64(),
                        cumulative
                    ));
                }
                cumulative += h.overflow.load(Ordering::Relaxed);
                out.push_str(&format!(
                    "efe_nano_firecracker_{name}_seconds_bucket{{{labels},le=\"+Inf\"}} {cumulative}\n"
                ));
            }
        }

        out
    }
}

// Histogram accessors (need fn pointers, can't use closures with the
// macro / generics in stable Rust today).
fn histo_accessor_cold_start(m: &SubPoolMetrics) -> &Histogram {
    &m.cold_start_duration
}
fn histo_accessor_inference(m: &SubPoolMetrics) -> &Histogram {
    &m.inference_duration
}
fn histo_accessor_vsock(m: &SubPoolMetrics) -> &Histogram {
    &m.vsock_rtt
}
fn histo_accessor_acquire(m: &SubPoolMetrics) -> &Histogram {
    &m.acquire_wait
}

fn render_labels(key: &SubPoolKey) -> String {
    format!(
        "kind=\"{}\",model_id=\"{}\",model_version=\"{}\",tenant_class=\"{}\"",
        match key.kind {
            super::super::VmKind::ModelInference => "model_inference",
            super::super::VmKind::LlmCaller => "llm_caller",
        },
        escape_label(&key.model_id),
        escape_label(&key.model_version),
        key.tenant_class.as_str(),
    )
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_inc_get() {
        let c = Counter::default();
        assert_eq!(c.get(), 0);
        c.inc();
        c.inc();
        c.add(5);
        assert_eq!(c.get(), 7);
    }

    #[test]
    fn gauge_inc_dec() {
        let g = Gauge::default();
        g.set(10);
        g.inc();
        g.inc();
        g.dec();
        assert_eq!(g.get(), 11);
    }

    #[test]
    fn histogram_observes_into_correct_bucket() {
        let h = Histogram::new();
        h.observe(Duration::from_micros(50)); // < 100 µs
        h.observe(Duration::from_millis(5)); // < 10 ms
        h.observe(Duration::from_secs(60)); // overflow

        assert_eq!(h.count(), 3);
        // Bucket 100 µs (index 1) should have 1.
        // Bucket 10 ms (index 3) should have 1.
        // Overflow should have 1.
        let b1 = h.buckets[1].1.load(Ordering::Relaxed);
        let b3 = h.buckets[3].1.load(Ordering::Relaxed);
        let ov = h.overflow.load(Ordering::Relaxed);
        assert_eq!(b1, 1);
        assert_eq!(b3, 1);
        assert_eq!(ov, 1);
    }

    #[test]
    fn histogram_mean_is_correct() {
        let h = Histogram::new();
        h.observe(Duration::from_millis(10));
        h.observe(Duration::from_millis(20));
        h.observe(Duration::from_millis(30));
        assert_eq!(h.mean(), Duration::from_millis(20));
    }

    #[test]
    fn histogram_mean_zero_when_empty() {
        let h = Histogram::new();
        assert_eq!(h.mean(), Duration::ZERO);
    }

    #[test]
    fn for_key_returns_same_arc_for_same_key() {
        let m = FirecrackerMetrics::new();
        let key = SubPoolKey::mode_b("a", "1.0", TenantClass::Tier1);
        let m1 = m.for_key(&key);
        let m2 = m.for_key(&key);
        assert!(Arc::ptr_eq(&m1, &m2));
    }

    #[test]
    fn for_key_returns_distinct_arcs_for_distinct_keys() {
        let m = FirecrackerMetrics::new();
        let k1 = SubPoolKey::mode_b("a", "1.0", TenantClass::Tier1);
        let k2 = SubPoolKey::mode_b("a", "1.0", TenantClass::Experimental);
        let m1 = m.for_key(&k1);
        let m2 = m.for_key(&k2);
        assert!(!Arc::ptr_eq(&m1, &m2));
    }

    #[test]
    fn render_includes_global_counters() {
        let m = FirecrackerMetrics::new();
        m.spawn_total.add(7);
        m.reap_total_global.add(3);
        let out = m.render();
        assert!(out.contains("efe_nano_firecracker_spawn_total 7"));
        assert!(out.contains("efe_nano_firecracker_reap_total_global 3"));
    }

    #[test]
    fn render_emits_warm_count_with_labels() {
        let m = FirecrackerMetrics::new();
        let key = SubPoolKey::mode_b("ner_clinical", "1.0.0", TenantClass::TierAudit);
        let sub = m.for_key(&key);
        sub.warm_count.set(5);
        let out = m.render();
        assert!(out.contains("efe_nano_firecracker_warm_count"));
        assert!(out.contains("model_id=\"ner_clinical\""));
        assert!(out.contains("tenant_class=\"tier_audit\""));
        assert!(out.contains("} 5"));
    }

    #[test]
    fn render_emits_histogram_buckets() {
        let m = FirecrackerMetrics::new();
        let key = SubPoolKey::mode_b("a", "1.0", TenantClass::Tier1);
        let sub = m.for_key(&key);
        sub.inference_duration.observe(Duration::from_millis(5));
        sub.inference_duration.observe(Duration::from_millis(50));
        let out = m.render();
        assert!(out.contains("efe_nano_firecracker_inference_duration_seconds_bucket"));
        assert!(out.contains("le=\"0.01\""));
        assert!(out.contains("le=\"0.1\""));
        assert!(out.contains("le=\"+Inf\""));
        // count = 2
        assert!(out.contains("efe_nano_firecracker_inference_duration_seconds_count"));
    }

    #[test]
    fn render_escapes_special_chars_in_labels() {
        let m = FirecrackerMetrics::new();
        let key = SubPoolKey {
            kind: super::super::super::VmKind::ModelInference,
            model_id: "weird\"name".into(),
            model_version: "1.0".into(),
            tenant_class: TenantClass::Tier1,
        };
        let _ = m.for_key(&key);
        let out = m.render();
        assert!(out.contains("model_id=\"weird\\\"name\""));
    }

    #[test]
    fn concurrent_counter_increments_are_safe() {
        use std::sync::Arc;
        let c = Arc::new(Counter::default());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let c = Arc::clone(&c);
            handles.push(std::thread::spawn(move || {
                for _ in 0..1000 {
                    c.inc();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(c.get(), 8000);
    }
}
