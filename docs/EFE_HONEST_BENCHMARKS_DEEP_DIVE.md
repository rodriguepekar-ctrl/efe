# EFE — Honest Benchmarks Architecture — Volume II : Deep Dive

| Field | Value |
|---|---|
| **Doc type** | Companion deep-dive to `EFE_HONEST_BENCHMARKS_ARCHITECTURE.md` (Volume I) |
| **Status** | Frozen — awaiting code |
| **Scope** | Implementation + methodology + operations + deep technical foundations that Volume I references but does not detail. Volume I = the *policy* ; Volume II = the *engineering manual*. |
| **Audience** | Bench-framework implementers ; bench reviewers ; auditors ; SRE on-call when bench drift fires |
| **Reading order** | Volume I first (policy, AD/AC, threat model) → Volume II (this doc, drilling each axis to math-level) |
| **Depends on** | Volume I (frozen at same revision) |
| **Threat IDs deepened** | T1-T18 (Volume I) ; T19 chain-of-custody ; T20 stale-baseline ; T21 cross-cloud-bias ; T22 provider-bound-dressed-as-own ; T23 faux-multi-tenancy ; T24 coordinated-cluster-warmup ; T25 window-dressing-throughput ; T26 GPU-warmup-omission |

---

## Table of contents

  0. Reading the two volumes together
  1. HdrHistogram — internals + math + memory footprint
  2. Statistical foundations II (KL, EMD, Wasserstein, multivariate tests)
  3. Multi-tenant interference modeling (queueing theory + fork-join)
  4. Cold-start cascade — full decomposition per component
  5. GPU bench specifics (CUDA streams, warm-up, kernel launch overhead)
  6. Cross-cloud measurement bias — quantified
  7. LLM provider non-stationarity (Mode 2/3 disclosure)
  8. Memory allocator deep-dive (jemalloc / mimalloc / system malloc)
  9. Network stack tuning for bench (TCP_NODELAY, BBRv2, SO_REUSEPORT)
 10. Linux kernel tuning (cset shield, IRQ affinity, NUMA, hugepages)
 11. Bench harness Rust trait contracts (efe-bench-harness crate)
 12. Per-SDK bench interface contracts (Py / TS / Java / Go)
 13. Bench fleet K8s deployment topology
 14. Bench fleet cost analysis (annual budget envelope)
 15. Bench team RACI
 16. On-call for bench drift
 17. Public publication workflow (release-v1-ga.yml integration)
 18. Customer-facing bench portal
 19. Auditor access protocol
 20. Failure mode catalog (50+ per-surface realistic failure modes)
 21. Synthetic corpus generators — per surface algorithms
 22. Replay capture pipeline (anonymization → versioning → access-control)
 23. Adversarial corpus catalog — per surface
 24. Per-surface deep-dive expansions
 25. Cross-region bench setup
 26. A/B comparison workflow (pre-registration template)
 27. Drift bisection automation
 28. Bench-as-SBOM (CycloneDX provenance for bench results)
 29. SLSA-L3 for bench results
 30. Energy measurement deep-dive (RAPL bus, ARM PMU, NVIDIA NVML)
 31. Cost integration deep-dive (AWS / GCP / Azure pricing APIs)
 32. Reproducibility hashes (deterministic outputs ; SOURCE_DATE_EPOCH)
 33. Concrete K8s manifest examples
 34. Concrete Prometheus rule examples
 35. Concrete Grafana dashboard JSON skeletons
 36. Bench output deserialization examples
 37. Per-SDK example bench programs (canonical clients)
 38. Meta-bench — the bench framework's own benchmark
 39. Glossary
 40. Index

---

## 0. Reading the two volumes together

```text
┌─────────────────────────────────────────────────────────────────┐
│ Volume I  : "what we promise + why"     (policy + AD + AC)     │
│             - frozen contract               2 000 LOC          │
│             - threat model T1-T18                              │
│             - per-surface SLO targets                          │
├─────────────────────────────────────────────────────────────────┤
│ Volume II : "how we deliver"            (implementation)        │
│             - math, code, manifests                           │
│             - bench-harness crate + SDK contracts             │
│             - operational : RACI, on-call, public publication │
│             - failure-mode catalog (50+ items)                │
│             - 40 sections / 3500+ LOC                         │
└─────────────────────────────────────────────────────────────────┘
```

Volume I declares "report p99 with bootstrap CI under coordinated-
omission correction". Volume II declares HOW (HdrHistogram with 3
significant digits, value range 1 µs to 1 hour, bucket count derived
mathematically, bootstrap with 10 000 resamples and a specific stratification
strategy, etc.).

Disagreement between Volume I and Volume II is resolved by Volume I.

---

## 1. HdrHistogram — internals + math + memory footprint

### 1.1 Why HdrHistogram

Latency distributions are skewed-right with heavy tails. A naive
linear histogram with N bins of equal width either misses fine
resolution near the median or wastes memory in the tail. HdrHistogram
uses **logarithmic-scale bucket-and-sub-bucket arithmetic** that
guarantees a configured precision (significant figures) across the
full dynamic range.

### 1.2 Bucket-and-sub-bucket math

Configuration :

  - `lowestDiscernibleValue` (LDV) : smallest value with full precision.
    For EFE : 1 µs.
  - `highestTrackableValue` (HTV) : largest representable value.
    For EFE : 3.6e9 µs (1 hour). Anything beyond is clamped + counted.
  - `significantValueDigits` (S) : precision. For EFE : 3 (~ 0.1 % error).

Derived :

  - `subBucketCount` = 2 × ceil(2^S)  
    For S=3 : subBucketCount = 2 × 1024 = 2048.
  - `bucketCount` = ceil(log2(HTV / LDV) - log2(subBucketCount / 2))  
    For EFE : ceil(log2(3.6e9 / 1) - 10) ≈ ceil(31.74 - 10) = 22 buckets.

Total cells : `subBucketCount × bucketCount / 2 + subBucketCount` ≈
**24 576 cells**.

Memory footprint at 8-byte counters : 24 576 × 8 = **192 KiB per
histogram** for a per-tenant histogram. Per-PR bench captures
maintain ~ 50 histograms (per-percentile decomposition, per-tenant
slices, sliding-window snapshots) — total ≈ **10 MiB RAM** for an
entire bench session.

This is order-of-magnitude smaller than alternative approaches :

  - Reservoir sample (size 10 000) : 80 KB per metric × tenant × percentile
    family ≈ 50 MB.
  - Full raw sample retention (1 M samples × 8 B) : 8 MB per metric,
    fast to compute but lossy for resampling.

### 1.3 Precision guarantees

For a sample x recorded at significant digits S=3 :

  - Bucket index : ⌊log2(x / LDV) - log2(subBucketCount / 2)⌋
  - Sub-bucket index : depends on which 2^S "minor scale" x falls in
  - Stored value : the lower edge of the sub-bucket containing x.

The maximum relative error for any percentile-recovered value is
**1 / 2^S = 1 / 1024 ≈ 0.098 %**.

This is sufficient for SLO claims at 0.1 %-level precision. Going to
S=4 (~ 0.006 % error) costs 8× memory and is overkill.

### 1.4 Coordinated-omission correction (formal)

When a request is *intended* at time `t_intended` but completes at
`t_completed`, the *intendedServiceTime* is :

```
intendedServiceTime = t_completed - t_intended
                    = serviceTime + max(0, t_actual_issue - t_intended)
```

When `serviceTime` exceeds the rate-controller's inter-request
interval `Δ = 1 / target_qps`, the controller cannot issue subsequent
requests on schedule. HdrHistogram synthesizes the *missed*
samples :

```
n_missed = floor(serviceTime / Δ)
for k in 1..n_missed :
    record(intendedServiceTime - k × Δ)
```

This recovers what the latency distribution *would have been* if the
server had accepted the queued requests at their intended schedule.

### 1.5 Implementation : `efe-bench-harness::Hdr`

```rust
// crates/efe-bench-harness/src/hdr.rs
use hdrhistogram::Histogram;

pub struct Hdr {
    service:   Histogram<u64>,          // serviceTime µs
    intended:  Histogram<u64>,          // intendedServiceTime µs
    interval_us: u64,                    // rate-controller Δ µs
}

impl Hdr {
    pub fn new(target_qps: f64) -> Self {
        let h = Histogram::<u64>::new_with_bounds(1, 3_600_000_000, 3).unwrap();
        Self {
            service: h.clone(),
            intended: h,
            interval_us: (1_000_000.0 / target_qps).max(1.0) as u64,
        }
    }

    pub fn record(&mut self, service_time_us: u64) {
        self.service.record(service_time_us).unwrap();
        // Coordinated-omission correction.
        let mut t = service_time_us;
        self.intended.record(t).unwrap();
        while t > self.interval_us {
            t -= self.interval_us;
            self.intended.record(t).unwrap();
        }
    }

    pub fn percentile(&self, p: f64, kind: HdrKind) -> u64 {
        let h = match kind { HdrKind::Service => &self.service, HdrKind::Intended => &self.intended };
        h.value_at_percentile(p)
    }
}

pub enum HdrKind { Service, Intended }
```

### 1.6 Serialization

HdrHistogram serializes to a compact binary form (gzip-compressed).
A 1-hour bench's histogram pair (~ 50 MB raw samples) serializes to
~ 200 KB. Stored in `bench_run_<uuid>/result.hdr` for offline analysis.

### 1.7 Cross-language

  - Rust : `hdrhistogram` crate (canonical).
  - Python : `hdrh` (matching reference).
  - Java : `org.hdrhistogram:HdrHistogram` (canonical, by Gil Tene).
  - Go : `github.com/HdrHistogram/hdrhistogram-go`.
  - TypeScript : `hdr-histogram-js` (WASM-backed).

The framework verifies cross-language equivalence : the same input
sample stream produces the same percentile (modulo a 1-µs floor).
Cross-language tests in `crates/efe-bench-harness/tests/cross_lang.rs`.

---

## 2. Statistical foundations II

Volume I §11 covered the basics (bootstrap CI, Welch's t-test,
Mann-Whitney U, power analysis). Volume II adds the **distribution-
shape** tests that the framework uses when comparing two latency
distributions whose shapes — not just central tendency — may have
shifted.

### 2.1 Kolmogorov-Smirnov

KS statistic :

```
D = sup_x |F_a(x) - F_b(x)|
```

where `F_a` and `F_b` are empirical CDFs. KS detects ANY distributional
difference, including bimodality emergence.

Threshold (5 % significance, two-sample) :

```
D_crit ≈ 1.36 × sqrt((n_a + n_b) / (n_a × n_b))
```

Use case : detect "the optimization shifted the *median* by 2 % but
introduced a *new mode* at p95 caused by a path that wasn't tested".

### 2.2 Kullback-Leibler divergence

For two discrete distributions P and Q :

```
D_KL(P || Q) = Σ_x P(x) × log(P(x) / Q(x))
```

Asymmetric. Used to score the "surprise" of distribution Q given
expected distribution P. EFE uses the symmetric **Jensen-Shannon
divergence** :

```
D_JS(P, Q) = 0.5 × D_KL(P || M) + 0.5 × D_KL(Q || M)
where M = 0.5 × (P + Q)
```

Bounded in [0, log 2]. Threshold for "distributional drift detected"
in EFE : `D_JS > 0.05 nats`.

### 2.3 Earth Mover's Distance (Wasserstein-1)

For two 1D distributions :

```
EMD = ∫ |F_a(x) - F_b(x)| dx
```

Geometrically interpretable : "how much probability mass must move,
and how far, to transform one distribution into the other". Robust
to outliers.

The framework reports EMD alongside KS and JS divergence in any A/B
comparison.

### 2.4 Multivariate : MMD with RBF kernel

For benches that report multiple correlated metrics (latency +
throughput + CPU + memory), univariate tests miss the *joint*
shift. Maximum Mean Discrepancy (MMD) with an RBF kernel :

```
MMD²(P, Q) = E[k(x, x')] - 2E[k(x, y)] + E[k(y, y')]
where k(u, v) = exp(-||u - v||² / 2σ²)
```

EFE uses σ chosen via the median heuristic. Permutation test (n=10 000)
for the p-value.

### 2.5 Pre-registration template

To prevent garden-of-forking-paths abuse (Volume I T18 / G.7), every
A/B comparison registers in `bench_ab/<comparison-id>.yaml` BEFORE the
bench runs :

```yaml
apiVersion: efe.bench/v1
kind:       ABComparison
id:         mode1-jemalloc-vs-mimalloc-2026-Q3
metadata:
  hypothesis: "Switching jemalloc -> mimalloc improves p99 by ≥ 3 %."
  registered_at: 2026-08-01T12:00:00Z
spec:
  baseline:
    revision: "mainline-<sha>"
    bench:    mode1
    tier:     staging
  candidate:
    revision: "feature/mimalloc-<sha>"
    bench:    mode1
    tier:     staging
  primary_test:
    statistic:  bootstrap-difference-of-p99
    n_boot:     10000
    alpha:      0.01
    direction:  two-tailed
  secondary_tests:
    - KS
    - JS-divergence
    - Welch-t
    - Mann-Whitney-U
  minimum_detectable_effect:
    p99: 1.23 ms                          # 3 % of 41 ms baseline
  required_runs: 30                       # AD12
  decision_rule: |
    PROMOTE if (primary p-value < 0.01) AND (point estimate >= MDE)
                AND (all secondary tests agree on direction).
    Otherwise : do not promote ; document.
  bias_disclosure:
    - "Same author for baseline + candidate (Goodhart risk T18)"
    - "Mitigated by external rerun on day +7."
```

The framework rejects unregistered comparisons from publication.

### 2.6 Multiple-comparison correction

When N benches are compared in parallel (e.g. nightly drift check),
the family-wise error rate inflates. EFE applies **Benjamini-Hochberg
FDR correction** at q=0.05 :

```
Sort p-values p_1 ≤ p_2 ≤ ... ≤ p_N
Find largest k such that p_k ≤ k/N × q
Reject H_0 for all i ≤ k
```

The drift gate's WARN threshold is the BH-corrected p-value, not the
raw p-value.

---

## 3. Multi-tenant interference modeling

### 3.1 Why model

The single-tenant bench number is not the multi-tenant number. The
delta is **not a constant ratio** ; it depends on workload mix, lock
contention, cache footprint, and queueing dynamics. The framework's
multi-tenant baseline (AD29 / AC20) is not just "run the bench with
8 tenants" ; it's "characterize the *interference function*
I(n_tenants, mix)".

### 3.2 Little's law

For any stable queueing system :

```
L = λ × W
```

where L = average concurrent jobs, λ = arrival rate, W = average
sojourn time. Inverting :

```
W = L / λ                       (latency = concurrency / throughput)
```

For EFE Mode 1 at 1 000 req/s with p99 = 41 ms : average concurrency
at p99 is ~ 41 jobs. If 8 tenants share the resource and each tenant
issues 125 req/s (1 000 / 8) :

  - With perfect fairness : each tenant sees the same p99 = 41 ms.
  - With one tenant bursting to 500 req/s : the cluster sees QPS = 1 375,
    which may exceed the 1 200 req/s knee-of-curve, and p99 across
    all tenants degrades.

The framework's multi-tenant bench measures :

  - **Symmetric** : 8 tenants × 125 req/s each. Each tenant sees the
    "fair share" p99.
  - **Burst** : 1 tenant × 500 req/s + 7 tenants × 125 req/s. The
    bursting tenant's p99 + the non-bursting tenants' p99 reported
    separately.
  - **Pathological** : 1 tenant × 2 000 req/s (above cluster capacity)
    + 7 tenants × 125 req/s. Tests rate-limit enforcement +
    fairness under saturation.

### 3.3 Fork-join interference

Mode 2 (LLM-routed) is fork-join : controller fans out to multiple
back-end calls (LLM + Mode 1 backend). For N parallel sub-requests,
the fan-out latency p99 is :

```
P[max ≤ x] = P[X_1 ≤ x] × P[X_2 ≤ x] × ... × P[X_N ≤ x]
For identical sub-requests : p_max(0.99) ≈ p_sub(0.99^(1/N))
```

For N=3 sub-requests at sub-p99 = 14 ms : fan-out p99 ≈ p_sub(0.997) ≈
17 ms.

Tail amplification : a 1 ‰ slow sub-request causes ~ 0.3 % overall
slowdown at p99. The framework benches fan-out scenarios at N ∈ {1,
3, 5, 10} to quantify.

### 3.4 Lock-contention model

For an exclusively-locked resource (e.g. audit chain HMAC sequence
counter) with hold time h and contention rate λ :

```
W_lock = λ × h² / (2 × (1 - λh))      (M/M/1 with FCFS)
```

For audit chain (h ≈ 200 µs, λ at 8 tenants × 125 req/s = 1 000 req/s) :

```
ρ = λh = 1 000 × 200e-6 = 0.2
W_lock = 0.2 × 200 / (2 × 0.8) = 25 µs
```

At 100 tenants × 125 req/s (= 12 500 req/s arrival but at most 5 000
req/s due to lock saturation : ρ = 1, the system queues without
bound).

The framework's audit-chain bench (Volume I §16.1) explicitly probes
the saturation knee.

### 3.5 Cache footprint interference

L2/L3 cache is shared. Tenant A's working set evicts tenant B's. The
framework measures :

  - **Cold cache p99** : measure on fresh container, no prior tenant.
  - **Warm-single-tenant p99** : warm-up with tenant A only.
  - **Warm-multi-tenant p99** : warm-up with N tenants in rotation.

The delta (warm-multi - warm-single) is the **cache-contention overhead**.
For EFE Mode 1 : ~ 2-4 ms at p99 with 8 tenants on c6i.2xlarge.

---

## 4. Cold-start cascade — full decomposition

The "cold start" SLO (`SLO-COLD-START` < 10 s, baseline 3.2 s) is
not one timer but a *cascade* of phase boundaries. The bench
decomposes each :

```text
T₀ — container exec/syscall (k8s pod creation done)
  Δ_kernel  ≈ 200 ms   : kernel cgroup setup, oci-runtime init
T₁ — process entry (binary entrypoint runs)
  Δ_glibc   ≈ 80 ms    : glibc ld.so + relocation
  Δ_tonic   ≈ 120 ms   : tonic gRPC server init + TLS material
  Δ_rustls  ≈ 60 ms    : rustls roots load + CA chain
T₂ — server framework ready
  Δ_db_pool ≈ 250 ms   : Postgres connection pool warm (5 conns)
  Δ_redis   ≈ 120 ms   : Redis sentinel discovery + auth
T₃ — dependencies connected
  Δ_manifest ≈ 800 ms  : bundle download + signature verify + parse
  Δ_anchor   ≈ 1200 ms : anchor cache rebuild (per-tenant)
T₄ — caches populated
  Δ_health  ≈ 80 ms    : first probe response
T₅ — k8s marks Ready
  Δ_first_req ≈ 290 ms : first real request (cold every cache)
T₆ — first req completed → cold-start total = 3 200 ms
```

The bench reports each `Δ` with its own CI. Drift in any phase is
attributable. A 10× regression in `Δ_anchor` (because the cache
rebuild changed shape) shows up clearly.

Decomposition method : trace spans tagged with phase boundaries,
HDR histograms per phase. The cold-start bench runs **N=30 fresh
container starts** and reports the full distribution per phase.

---

## 5. GPU bench specifics

### 5.1 Warm-up matters MORE on GPU

CUDA kernel launches incur :

  - JIT compilation of fat-binary or PTX (first invocation only).
  - Memory copy host → device (first invocation per tensor shape).
  - Kernel cache population (NVIDIA driver caches per-shape).
  - cuDNN heuristic search (first invocation per shape, op, dtype).

A "cold GPU bench" can be 100× slower than warm. The bench framework
mandates :

  - **3-tier reporting** : ice-cold (first CUDA context), warm-cold
    (cold kernel cache, but CUDA context warm), warm-warm.
  - **Per-shape warm-up declaration** : the shapes warmed-up are in
    the manifest. New shapes encountered during the measurement are
    flagged.

### 5.2 Stream synchronization

CUDA kernels are async. A naïve bench :

```cpp
auto t0 = now();
kernel<<<...>>>(args);
auto t1 = now();        // BUG : kernel may not have completed
```

Reports the kernel-launch latency, not the kernel runtime. The bench
framework wraps :

```cpp
cudaEvent_t start, stop;
cudaEventCreate(&start); cudaEventCreate(&stop);
cudaEventRecord(start);
kernel<<<...>>>(args);
cudaEventRecord(stop);
cudaEventSynchronize(stop);
float ms;
cudaEventElapsedTime(&ms, start, stop);
```

### 5.3 Throughput-latency on GPU

Latency at batch size 1 ≠ latency at batch size 32. Throughput
typically scales sub-linearly with batch (memory bandwidth bound).
The framework benches a sweep : batch ∈ {1, 2, 4, 8, 16, 32, 64,
128}.

For each batch size : (per-batch latency p99) + (per-sample latency
p99) + (throughput samples/s).

### 5.4 Multi-GPU + NCCL

Multimodal (M30) deploys multiple GPUs. NCCL all-reduce latency :

  - p99 < 5 ms for 64 MB at 4 × A100 NVLink.
  - p99 < 50 ms for 64 MB at 4 × T4 PCIe.

The framework benches the underlying NCCL ops separately from the
end-to-end Triton inference latency, so that a Triton regression can
be attributed (network ? kernel ? scheduler ?).

### 5.5 GPU memory bench

Memory pressure on GPU is critical (model weights + activations).
Bench reports :

  - Max resident GPU memory (per device).
  - Allocator slab statistics (cudaMallocAsync vs caching allocator).
  - OOM scenarios : bench triggers a deliberate OOM and measures
    recovery latency.

---

## 6. Cross-cloud measurement bias — quantified

Same SKU spec on different clouds shows latency differences that are
NOT customer-controllable. The framework measures + discloses :

### 6.1 Empirical observations (from prior EFE Q1 2026 cross-cloud bench)

| Surface | AWS c6i.2xl | GCP n2-standard-8 | Azure D8s_v5 |
|---|---|---|---|
| Mode 1 p99 | 41 ms | 38 ms | 45 ms |
| Mode 1 p99.9 | 87 ms | 80 ms | 110 ms |
| TCP RTT same-AZ | 0.4 ms | 0.5 ms | 0.6 ms |
| Disk fsync (NVMe) | 80 µs | 120 µs | 140 µs |

Differences attributable to :

  - CPU SKU (Ice Lake vs Cascade Lake vs newer Sapphire).
  - Hypervisor (Nitro vs Andromeda vs Hyper-V).
  - Storage backend (EBS gp3 vs PD-Balanced vs Premium SSD).
  - Network stack (SRD vs Gigabyte vs Hyper-V Accelerated).

### 6.2 Disclosure requirements

Every cross-cloud comparison report includes :

  - SKU CPU model + generation.
  - Hypervisor version.
  - Storage class + IOPS / throughput tier.
  - Network bandwidth + congestion-control config.
  - Region + AZ.
  - Pricing snapshot (per AD17).

### 6.3 SLO normalization

Published SLOs are **per-cloud**. EFE does NOT publish "p99 < 50 ms
globally" when AWS hits 41 ms and Azure hits 45 ms with no Azure-side
fix planned. The framework rejects single-cloud SLOs as "global"
claims.

---

## 7. LLM provider non-stationarity

### 7.1 The problem

Mode 2 (LLM-routed) and Mode 3 (multi-turn) latency depend on the
external LLM provider (OpenAI, Anthropic, Azure OpenAI, etc.). These
providers exhibit **non-stationary** latency :

  - Diurnal effects (US-evening latency 2× US-morning).
  - Weekly effects (Monday latency higher than Tuesday).
  - Episodic incidents (provider outages, model swaps).
  - Capacity-class effects (paid tier vs free tier).
  - Prompt-shape effects (long context → super-linear latency).

A bench that measures Mode 2 once at 14:00 UTC on a Tuesday does NOT
characterize the customer experience at 22:00 UTC on a Saturday.

### 7.2 EFE disclosure

Mode 2 / Mode 3 benches report :

  - **Provider component p99** : measured separately.
  - **EFE-side overhead p99** : Mode 2 minus provider.
  - **Provider window** : time-of-day + day-of-week of the measurement.
  - **Provider tier** : paid / free / dedicated.
  - **Prompt-shape distribution** : mean tokens in/out + p99 tokens.

### 7.3 Synthetic LLM mock

For CI-time regression gating (where real provider variance would
flood the signal), bench runs against a **deterministic LLM mock** :

```rust
// efe-bench-harness/src/mock_llm.rs
pub struct MockLlm {
    latency_distribution: LogNormal,
    seed: u64,
}

impl MockLlm {
    pub async fn complete(&self, prompt: &str) -> String {
        let target_ms = self.latency_distribution.sample();
        tokio::time::sleep(Duration::from_micros((target_ms * 1000.0) as u64)).await;
        canned_response(prompt)
    }
}
```

The mock's distribution is parameterized from a *snapshot* of the
real provider's latency (captured during a representative 24-hour
window). Snapshot is versioned : `bench_corpus/llm_mock/v0.9.0/`.

### 7.4 Real provider bench

For public publication, EFE runs a **24-hour rolling-window bench**
against the real provider. The published p99 is the worst rolling
1-hour p99 over the 24-hour window (not the mean of the 24 windows ;
this is the customer-worst-experience reporting).

---

## 8. Memory allocator deep-dive

### 8.1 jemalloc (default for efe-server)

`efe-server` links jemalloc via `tikv-jemallocator`. Configuration :

```
MALLOC_CONF=narenas:8,tcache:true,dirty_decay_ms:5000,muzzy_decay_ms:5000
```

  - `narenas=8` : 8 arenas for thread-locality (matches typical
    16-thread NUMA-single-socket).
  - `tcache=true` : per-thread cache (essential for low-latency p99).
  - `dirty_decay_ms` / `muzzy_decay_ms` : 5 s decay (default 10 s
    causes RSS oscillation under bursty traffic).

Bench reports jemalloc stats per run :

```text
allocated:        2 145 MB
active:           2 218 MB
metadata:           42 MB
resident:         2 384 MB
mapped:           2 512 MB
retained:           28 MB
```

### 8.2 mimalloc — candidate alternative

Microsoft's mimalloc has lower fragmentation under certain workloads.
EFE pre-registered A/B (§ 2.5 template) for `mimalloc-vs-jemalloc-2026-Q3`
ran for 30 days ; verdict TBD. The bench framework supports both via
feature flag.

### 8.3 System malloc — baseline

A "system malloc" run (glibc's malloc) is the third leg : measures
the cost of the non-default allocator. Typically 2-3× worse p99
under bursty traffic.

### 8.4 GPU allocator

NVIDIA `cudaMallocAsync` (default in recent CUDA) vs PyTorch's
caching allocator. The framework benches both (M30 multimodal).

---

## 9. Network stack tuning for bench

### 9.1 TCP_NODELAY

Disables Nagle. EFE-server sets it on every gRPC connection
(default in tonic). Bench framework verifies via `getsockopt`.

### 9.2 SO_REUSEPORT

For benches with many client processes, SO_REUSEPORT lets multiple
client procs bind the same port and distribute load across kernel-
side per-cpu accept queues. Reduces single-thread accept bottleneck.

### 9.3 BBRv2 vs CUBIC

BBRv2 has lower latency in non-congested networks, slightly worse
fairness in congested networks. EFE production uses BBRv2 (set via
`sysctl net.ipv4.tcp_congestion_control=bbr2`). The bench host
manifest records the congestion algorithm in use.

### 9.4 io_uring vs epoll

For streaming surfaces (M28), io_uring offers lower per-event
overhead. EFE's `efe-streaming` crate uses io_uring on Linux 5.10+.
Bench reports the active backend (`tokio-uring` vs `tokio` epoll).

### 9.5 GRPC keep-alive

EFE-server gRPC server config :

```
KeepAlive : 30 s (ping interval)
KeepAliveTimeout : 10 s
KeepAlivePermitWithoutCalls : true
MaxConnectionAge : 24 h
MaxConnectionAgeGrace : 10 s
```

Bench long-running clients trigger the keep-alive path ; bench
manifest records.

### 9.6 TLS session resumption

Cold-start measurements include the first TLS handshake (no
resumption). Warm measurements use session-ticket resumption (much
faster). Both reported (AD26).

---

## 10. Linux kernel tuning for bench

### 10.1 CPU isolation via cset shield

```bash
# Reserve cores 0-3 for system, isolate 4-15 for the bench.
sudo cset shield --cpu=4-15 --kthread=on
sudo cset shield --exec --user=bench --command=./efe-server-bench
```

This moves kernel threads off cores 4-15 and pins the bench process
exclusively. Reduces jitter from random kernel work.

### 10.2 IRQ affinity

Route NIC interrupts to dedicated cores (separate from the bench
cores) :

```bash
echo 0-3 > /proc/irq/$(grep eth0 /proc/interrupts | awk -F: '{print $1}')/smp_affinity_list
```

### 10.3 NUMA

EFE-server on a 2-socket host : pin to one socket to avoid cross-
socket memory access latency :

```bash
numactl --cpunodebind=0 --membind=0 ./efe-server-bench
```

### 10.4 Transparent Hugepages

THP can cause unpredictable latency spikes (page-fault stalls during
khugepaged scans). Disable for bench :

```bash
echo never > /sys/kernel/mm/transparent_hugepage/enabled
echo never > /sys/kernel/mm/transparent_hugepage/defrag
```

### 10.5 CPU governor

```bash
sudo cpupower frequency-set -g performance
```

Pins all cores at max frequency. Without this, frequency-scaling
adds 1-3 ms p99 jitter.

### 10.6 sysctl

```text
net.core.somaxconn               = 65535
net.core.rmem_max                = 16777216
net.core.wmem_max                = 16777216
net.ipv4.tcp_rmem                = 4096 87380 16777216
net.ipv4.tcp_wmem                = 4096 65536 16777216
net.ipv4.tcp_congestion_control  = bbr2
net.ipv4.tcp_no_metrics_save     = 1
net.ipv4.tcp_tw_reuse            = 1
vm.swappiness                    = 1
vm.max_map_count                 = 1048576
fs.file-max                      = 1048576
```

### 10.7 ulimit

```bash
ulimit -n 1048576       # file descriptors
ulimit -u 65535         # processes
```

---

## 11. Bench harness Rust trait contracts

The `efe-bench-harness` crate (NEW per Volume I § 45 migration)
provides traits that every bench MUST implement.

### 11.1 `Bench` trait

```rust
pub trait Bench: Send + Sync {
    /// Stable, machine-readable identifier (e.g. "mode1", "vector_query").
    fn id(&self) -> &str;

    /// Human-readable description.
    fn description(&self) -> &str;

    /// Declared axes (see Volume I § 8).
    fn axes(&self) -> Axes;

    /// One bench cycle. The harness invokes this N times per AD12.
    /// The implementation MUST record per-request HDR samples via
    /// `ctx.record_service_time(...)` to enable coordinated-omission
    /// correction.
    async fn run(&self, ctx: &mut BenchContext) -> BenchOutcome;

    /// Optional warm-up. Counted samples here are NOT included in
    /// the headline distribution but ARE disclosed in `BenchOutcome.exclusions`.
    async fn warmup(&self, ctx: &mut BenchContext) -> () { }

    /// Verify host + corpus + container match this bench's declaration.
    /// Default impl checks the axes against the runtime environment.
    fn preflight(&self, env: &Environment) -> Result<(), PreflightError> { Ok(()) }
}
```

### 11.2 `BenchContext`

```rust
pub struct BenchContext {
    pub hdr: Hdr,                       // § 1.5
    pub corpus: Arc<Corpus>,
    pub target_qps: f64,
    pub duration: Duration,
    pub feature_flags: FeatureFlags,
    pub tenant_mix: TenantMix,
    pub random_seed: u64,
}

impl BenchContext {
    pub fn record_service_time(&mut self, t: Duration) { /* HDR record */ }
    pub fn record_error(&mut self, kind: ErrorKind) { /* count */ }
    pub fn record_retry(&mut self, attempt: u32) { /* count */ }
    pub fn record_timeout(&mut self) { /* count */ }
}
```

### 11.3 `Axes`

```rust
pub struct Axes {
    pub scope:        Scope,            // Micro | Component | Service | EndToEnd | System
    pub metric:       MetricKind,       // Latency | Throughput | Cost | Energy | Resilience
    pub regime:       Regime,            // Cold | Warm | Sustained | Burst | Adversarial
    pub tenant_card:  TenantCardinality,// Single | Few(u32) | Many(u32) | CrossCloud(Vec<Cloud>)
    pub host_tier:    HostTier,         // Laptop | Ci | Staging | Prod
    pub arch:         Arch,             // Amd64 | Arm64 | Both
}
```

### 11.4 `BenchOutcome`

```rust
pub struct BenchOutcome {
    pub latency:        LatencyReport,
    pub throughput:     ThroughputReport,
    pub cost_per_req:   Option<CostReport>,
    pub energy_per_req: Option<EnergyReport>,
    pub exclusions:     Vec<Exclusion>,    // AC13
    pub negative_space: Vec<String>,        // AC30
    pub raw_hdr:        Vec<u8>,            // gzip-serialized HDR
}

pub struct LatencyReport {
    pub p50:    MetricWithCI,
    pub p75:    MetricWithCI,
    pub p90:    MetricWithCI,
    pub p95:    MetricWithCI,
    pub p99:    MetricWithCI,
    pub p99_9:  MetricWithCI,
    pub p99_99: MetricWithCI,
    pub min:    Duration,
    pub max:    Duration,
    pub stddev: Duration,
    pub n:      u64,
}

pub struct MetricWithCI {
    pub value:    Duration,
    pub ci_99_lo: Duration,
    pub ci_99_hi: Duration,
}
```

### 11.5 `Corpus`

```rust
pub struct Corpus {
    pub version:      String,           // "v0.9.0"
    pub content_hash: [u8; 32],          // sha256
    pub kind:         CorpusKind,       // Synthetic | Replay | Adversarial
    pub n_requests:   u64,
    pub disclosures:  CorpusDisclosure,
}

pub trait CorpusProvider {
    fn next(&mut self) -> CorpusItem;
}
```

### 11.6 Runner

```rust
pub struct BenchRunner {
    pub n_runs: u32,                     // AD12 ; default 10, override 30 for SLO
    pub workspace: PathBuf,              // bench_run_<uuid>/
}

impl BenchRunner {
    pub async fn run<B: Bench>(&self, bench: B) -> Result<BenchRun, RunnerError> {
        // 1. Pre-flight env check.
        let env = Environment::detect();
        bench.preflight(&env)?;
        // 2. Pin corpus + container ; emit manifest.
        let manifest = Manifest::from_env(&env, &bench);
        // 3. Run N times, collect per-run HDR.
        let mut runs = Vec::with_capacity(self.n_runs as usize);
        for run_idx in 0..self.n_runs {
            let mut ctx = BenchContext::from_manifest(&manifest);
            bench.warmup(&mut ctx).await;
            ctx.hdr.reset();        // exclude warm-up from headline
            let outcome = bench.run(&mut ctx).await;
            runs.push(outcome);
        }
        // 4. Aggregate : per-percentile median + bootstrap CI.
        let aggregated = aggregate(&runs);
        // 5. Emit BenchRun (JSON + Markdown + HDR + Prom).
        Ok(BenchRun { manifest, runs, aggregated })
    }
}
```

---

## 12. Per-SDK bench interface contracts

Each SDK exposes a `bench` module with a uniform contract :

### 12.1 Python

```python
# clients/python/efe/bench.py
class BenchCase(Protocol):
    name: str
    def cold(self, client: EfeClient) -> None: ...
    def warm(self, client: EfeClient, n: int) -> Iterator[float]: ...

def run_bench(case: BenchCase, target_qps: float, duration_s: float) -> BenchResult:
    """Run a bench case, return HDR-backed result."""
```

### 12.2 TypeScript

```typescript
// clients/typescript/@efe/sdk/src/bench.ts
export interface BenchCase {
  name: string;
  cold(client: EfeClient): Promise<void>;
  warm(client: EfeClient, n: number): AsyncIterable<number>;
}
export async function runBench(c: BenchCase, qps: number, durationS: number): Promise<BenchResult>;
```

### 12.3 Java

```java
// clients/java/efe-sdk/efe-sdk-core/src/main/java/io/efe/sdk/bench/BenchCase.java
public interface BenchCase {
    String name();
    void cold(EfeClient client) throws Exception;
    Stream<Long> warm(EfeClient client, int n) throws Exception;
}
public class BenchRunner {
    public BenchResult run(BenchCase c, double qps, Duration dur);
}
```

### 12.4 Go

```go
// clients/go/efe-sdk/bench/bench.go
type BenchCase interface {
    Name() string
    Cold(c *EfeClient) error
    Warm(c *EfeClient, n int) (<-chan time.Duration, error)
}
func RunBench(c BenchCase, qps float64, dur time.Duration) (BenchResult, error)
```

### 12.5 Cross-SDK equivalence

For each canonical bench (e.g. "echo Mode 1"), the 4 SDKs are
expected to produce **same-order-of-magnitude** p99 numbers. Cross-SDK
deltas trace to :

  - Serialization differences (protobuf vs JSON ; gRPC vs REST).
  - Connection pool behaviour (per-SDK default sizes).
  - Async runtime overhead (asyncio vs Node vs Reactor vs goroutine).

Drift between SDKs > 30 % triggers an investigation.

---

## 13. Bench fleet K8s deployment topology

### 13.1 Fleet composition

```text
┌────────────────────────────────────────────────────────────────┐
│ bench-fleet namespace                                          │
├────────────────────────────────────────────────────────────────┤
│ NodePool        Tier      Count   Purpose                      │
├─────────────────┼─────────┼───────┼──────────────────────────────┤
│ bench-staging-x │ staging │ 6     │ c6i.2xlarge AMD64           │
│ bench-staging-a │ staging │ 4     │ c7g.2xlarge ARM64           │
│ bench-staging-g │ staging │ 2     │ g4dn.xlarge with T4 GPU     │
│ bench-prod-x    │ prod    │ 4     │ c6i.4xlarge AMD64           │
│ bench-prod-a    │ prod    │ 2     │ c7g.4xlarge ARM64           │
│ bench-prod-g    │ prod    │ 2     │ p4d.24xlarge with 8× A100   │
│ bench-prod-bm   │ prod    │ 2     │ bare-metal Skylake, RAPL    │
└────────────────────────────────────────────────────────────────┘
```

### 13.2 Bench Job pattern

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  generateName: bench-mode1-
  namespace: bench-fleet
  labels:
    surface: mode1
    tier:    staging
    arch:    amd64
spec:
  parallelism: 1                           # bench runs are serial per tier
  backoffLimit: 0                          # never retry — re-run is explicit
  ttlSecondsAfterFinished: 604800          # 7 days
  template:
    spec:
      nodeSelector:
        node.kubernetes.io/instance-type: c6i.2xlarge
        bench-tier: staging
      tolerations:
        - key: bench-only
          operator: Exists
          effect: NoSchedule
      containers:
        - name: bench
          image: ghcr.io/efe-ai/efe-bench-harness:v1.0.0@sha256:...
          command: ["/usr/local/bin/efe-bench"]
          args:
            - run
            - --bench=mode1
            - --target-qps=1000
            - --duration=120s
            - --n-runs=10
            - --output=/results
          resources:
            requests: {cpu: "8", memory: "16Gi"}
            limits:   {cpu: "8", memory: "16Gi"}      # exclusive
          securityContext:
            capabilities:
              add: [SYS_NICE]                          # for taskset
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
          volumeMounts:
            - name: results
              mountPath: /results
      restartPolicy: Never
      volumes:
        - name: results
          persistentVolumeClaim:
            claimName: bench-results
```

### 13.3 Anti-collocation

A bench Pod NEVER shares its node with another workload. Achieved
via :

  - Dedicated nodepool (`bench-only` taint).
  - `nodeSelector` requiring `bench-tier`.
  - Resource requests = limits = exactly the node capacity (no
    bin-pack room).

### 13.4 Result persistence

A 100 GiB `bench-results` PVC mounted RW. Job writes to
`/results/<surface>/<run-id>/` ; a sidecar uploads to S3 +
Prometheus pushgateway after job completion.

### 13.5 Scheduling

  - Per-PR benches : ad-hoc Jobs triggered by GitHub Actions.
  - Nightly benches : CronJob at 02:00 UTC.
  - Soak benches : CronJob with `parallelism: 1` and matching duration.
  - Release benches : manual trigger via `workflow_dispatch`.

---

## 14. Bench fleet cost analysis

### 14.1 Hourly cost (AWS us-east-1 / eu-west-3 mix)

```text
6× c6i.2xlarge   on-demand   $0.34/h × 24 × 365 = $17 884 / yr
4× c7g.2xlarge   on-demand   $0.29/h × 24 × 365 = $10 162 / yr
2× g4dn.xlarge   on-demand   $0.526/h × 24 × 365 = $9 216 / yr
4× c6i.4xlarge   on-demand   $0.68/h × 24 × 365 = $23 833 / yr
2× c7g.4xlarge   on-demand   $0.58/h × 24 × 365 = $10 162 / yr
2× p4d.24xlarge  on-demand   $32.77/h × 24 × 365 = $574 042 / yr
2× bare-metal    fixed       $2 500/m × 12      = $60 000 / yr
                                                  ─────────────
                                                  $705 299 / yr
```

### 14.2 Cost optimizations

  - Spot instances for nightly + soak benches : -70 % on capacity
    cost. Annual savings ~ $400 k. Risk : occasional spot eviction
    invalidates a soak ; framework reschedules.
  - Reserved instances for the 4× prod hosts that run 24/7 : -40 %.
  - p4d.24xlarge usage : scheduled only during release weeks
    (~ 8 weeks / year), not 24/7. Annual cost drops to ~ $90 k.

After optimization : **~ $250 k / year** annual bench-fleet operating
cost.

### 14.3 Cost per published bench

50 published-quality benches per year (15 surfaces × 3 publication
windows + 5 release-gate runs) → ~ $5 000 per published bench.

Compare to the cost of a single Sev1 outage caused by a dishonest
SLO claim (mean cost in industry surveys : $50 k - $500 k). The bench
fleet pays for itself avoiding ~ 1 Sev1/year.

---

## 15. Bench team RACI

```text
                              R  A  C  I
─────────────────────────────────────────
Bench-framework code        bench bench sre  product
Per-surface bench impl      surface-owner bench sre product
Baseline freeze             surface-owner sre bench product
Drift triage                sre  sre  bench surface-owner
Public bench publication    product sre bench legal+sec
Auditor liaison             legal product bench (auditor)
Cost analysis               finops sre bench product
Energy reporting            sre  sre  bench product
SLO target adjustment       product sre bench (board)
Threat-model maintenance    sec  bench sre product
Corpus PII review           sec  sec  bench legal
Cross-cloud setup           sre  sre  bench product
```

R = Responsible (does the work)  
A = Accountable (signs off)  
C = Consulted (input required)  
I = Informed (notified)

---

## 16. On-call for bench drift

### 16.1 Alert ladder

  - **WARN** (drift > 5 %, single percentile) : Slack notification to
    bench channel ; SRE on-call investigates within 2 business days.
  - **MAJOR** (drift > 20 %, single percentile) : pager + on-call
    rotation engaged within 30 minutes ; investigation due in 24 h.
  - **CRITICAL** (drift > 20 % on a master ★ SLO) : Sev2 page ;
    investigation due in 4 h ; bench team + owning team join.

### 16.2 Triage workflow

```text
1. Confirm drift is real :
   - Re-run the bench. Drift persists across 3 runs ?
   - Re-run on different host (same tier). Drift persists ?
   - If only one host : suspect host-specific issue, not regression.

2. Bisect the responsible PR :
   - Run bench against last-known-good revision.
   - Run bench against first-known-bad revision.
   - Git-bisect with bench-script as the test.

3. Categorize the cause :
   - Infrastructure (host, dependency, kernel) → infrastructure fix.
   - Code regression → revert / fix PR.
   - Corpus drift → corpus version bump (rare).
   - SLO miscalibration → SRE+product convo to adjust the SLO.

4. Document in incident report (template in docs/incident-template.md).
```

### 16.3 Runbook entries per surface

Each per-surface design (Volume I § 14-§ 30) has a runbook entry
in `docs/runbooks/bench/<surface>.md` describing :

  - Common causes of drift on this surface.
  - Specific bisect commands.
  - Owning team contact.
  - Escalation path.

---

## 17. Public publication workflow

### 17.1 Pre-publication review

Before any number leaves the bench-fleet's internal dashboard :

  1. **Bench owner** runs the publication checklist (Volume I § 40.1).
  2. **SRE** independent re-run from cold state ; verifies numbers
     within 99 % CI of the headline.
  3. **Sec** reviews corpus for inadvertent PII (M29 PII scrubber
     re-run).
  4. **Legal** reviews comparison claims for marketing-fairness.
  5. **Product** signs off on customer-facing framing.

### 17.2 Publication artefacts

A published bench produces :

  - **Web page** : on the marketing site, with the dashboard.
  - **JSON** : downloadable at `efe.example.com/bench/<surface>/<version>.json`.
  - **HDR file** : downloadable at `.../<version>.hdr`.
  - **Reproducer** : downloadable shell script that re-runs the bench
    from any laptop.
  - **Manifest** : downloadable host + corpus + container.

### 17.3 Versioning

Each published bench has its own semver. `bench/mode1/v1.0.0`. New
optimizations bump minor ; corpus changes bump major. Old versions
remain accessible.

### 17.4 release-v1-ga.yml integration

```yaml
# .github/workflows/release-v1-ga.yml (excerpt — Volume II owns the bench piece)
jobs:
  release-bench:
    needs: build-binaries
    runs-on: bench-prod
    timeout-minutes: 720                    # 12 h release-tier bench
    steps:
      - uses: actions/checkout@v4
      - name: full release bench matrix
        run: |
          for surface in mode1 mode2 mode3 vector kg multimodal edge sdk-py sdk-ts sdk-java sdk-go ; do
            ./scripts/bench/release_bench.sh --surface $surface --tier prod \
              --n-runs 30 --duration 600s --publish=true
          done
      - name: verify publication checklist
        run: ./scripts/bench/verify_publish_checklist.sh
      - name: upload to public CDN
        run: ./scripts/bench/upload_public.sh
```

---

## 18. Customer-facing bench portal

### 18.1 URL structure

```
efe.example.com/bench/                       index page
efe.example.com/bench/<surface>/             surface dashboard
efe.example.com/bench/<surface>/v<ver>.json  machine readable
efe.example.com/bench/<surface>/v<ver>.hdr   raw HDR file
efe.example.com/bench/repro/<surface>/v<ver> reproducer recipe
efe.example.com/bench/changelog/             baseline updates log
```

### 18.2 Filter / drill-down

The portal supports filtering by :

  - Host tier (staging / prod).
  - Arch (AMD64 / ARM64 / GPU).
  - Tenant cardinality (1 / 8 / 100).
  - Date range (rolling window of measurements).

### 18.3 Negative-space surface

A dedicated page lists, per surface, **what is NOT measured** :

```text
Mode 1 — what we don't measure (yet) :
  - Cross-cloud variance for Mode 1 (Azure tier not in fleet yet)
  - Cold-start under k8s pod-rescheduling chaos
  - Sustained 30-day soak (only weekly 24-h soak today)

Mode 2 — what we don't measure :
  - Provider-side latency (depends on OpenAI ; we measure separately)
  - End-to-end with browser-side latency (out of scope)
```

This is the AC30 "honest absence" disclosure made visible.

---

## 19. Auditor access protocol

### 19.1 Access model

EFE's published benches are auditable by any 3rd party. To support
auditors :

  - All bench scripts in public git.
  - All synthetic corpora public.
  - Replay corpora available under NDA (M29-anonymized).
  - Adversarial corpora public.
  - Container digests signed and accessible via public Sigstore.
  - Reproducer recipe published per bench.

### 19.2 Audit workflow

```text
1. Auditor clones the public repo + bench-corpus snapshot.
2. Auditor pulls the pinned container by digest (verifies signature).
3. Auditor runs `./scripts/bench/repro.sh --surface <X> --tier <Y>` on
   their own infrastructure (matching the manifest tier spec).
4. Auditor compares their result.json to the published headline.
5. Discrepancies > 99 % CI : auditor files an issue on public GitHub.
```

### 19.3 Auditor SLA

EFE commits to :

  - Responding to a public bench-discrepancy issue within 5 business
    days.
  - Publishing a root-cause within 15 business days.
  - Either updating the headline OR providing a documented reason for
    the discrepancy.

---

## 20. Failure mode catalog

Per-surface, the bench framework explicitly tests failure modes.
This section is the master catalog. Each surface has 5-10 failure
modes ; total ~ 120 failure mode benches.

### 20.1 Network

  - `net.partition.full` : ingress dropped 100 % during a window.
  - `net.partition.partial` : 50 % packet loss for 30 s.
  - `net.jitter` : 50 ms ± 30 ms delay injection.
  - `net.bandwidth.cap` : limit bandwidth to 1 Mbps.
  - `net.dns.fail` : DNS resolution fails for `redis` host.

### 20.2 Compute

  - `cpu.throttle` : cgroup limit at 30 %.
  - `cpu.steal` : noisy neighbor consumes 50 % cycles.
  - `cpu.thermal` : forced thermal throttling.
  - `mem.pressure` : cgroup limit at 80 % of working set.
  - `mem.oom` : OOM kill of a worker process.

### 20.3 Storage

  - `disk.full` : ENOSPC on log writes.
  - `disk.slow` : 50 ms p99 IO latency injected via dm-delay.
  - `disk.fault` : EIO on 1 % of reads.

### 20.4 Dependencies

  - `redis.unreachable` : Redis port dropped.
  - `postgres.slow` : statement timeout doubled.
  - `llm.timeout` : LLM provider returns 504 after 30 s.
  - `llm.rate-limit` : LLM provider returns 429 with `Retry-After` header.
  - `nats.lag` : consumer lag grows past 10 000 messages.

### 20.5 Self

  - `process.crash` : segfault on a worker thread.
  - `panic.unwind` : Rust panic with backtrace.
  - `deadlock` : two locks acquired in opposite orders by 2 threads.

### 20.6 Resilience SLOs

For each failure mode, the bench reports :

  - p99 latency under the chaos (vs no-chaos p99).
  - Error rate during chaos.
  - Recovery time (post-chaos-removal).
  - Memory / CPU overhead during chaos.

The thresholds for each are in `bench_baselines/v1.0.0/resilience.yaml`.

---

## 21. Synthetic corpus generators

Algorithm per surface :

### 21.1 Mode 1 synthetic generator

```python
def generate_mode1_corpus(seed: int, n: int, zipf_alpha: float = 1.5):
    rng = np.random.default_rng(seed)
    # Categories Zipf-distributed
    n_categories = 127
    weights = 1.0 / np.power(np.arange(1, n_categories + 1), zipf_alpha)
    weights /= weights.sum()
    for _ in range(n):
        category = rng.choice(n_categories, p=weights)
        body_size = int(rng.lognormal(mean=np.log(1024), sigma=1.0))
        # Build a synthetic request body
        yield {
            "category_id": int(category),
            "body":        rng.bytes(body_size),
            "tenant_id":   pick_tenant_zipf(rng),
            "ts_ms":       int(time.time() * 1000),
        }
```

### 21.2 Mode 2 / Mode 3

Mode 2 synthesizes prompts of realistic token-count distribution.
Mode 3 synthesizes session turns with realistic turn-count
distribution (mean 4.2, p99 17).

### 21.3 Vector search

```python
def generate_vector_corpus(seed: int, n: int, dim: int = 768, n_clusters: int = 100):
    rng = np.random.default_rng(seed)
    # Cluster centers sampled uniformly on the unit sphere
    centers = rng.normal(size=(n_clusters, dim))
    centers /= np.linalg.norm(centers, axis=1, keepdims=True)
    # Per-vector : pick cluster, add Gaussian noise
    for _ in range(n):
        c = rng.choice(n_clusters, p=zipf_weights(n_clusters))
        v = centers[c] + rng.normal(scale=0.1, size=dim)
        v /= np.linalg.norm(v)
        yield v.astype(np.float32)
```

### 21.4 KG graph

Power-law degree distribution. Preferential attachment model
(Barabási-Albert).

### 21.5 Multimodal

Mixed image (RGB 224×224) + caption (text). Caption length log-normal.

### 21.6 Streaming events

Event size log-normal (median 1 KB, p99 50 KB). Inter-arrival times
Poisson at target rate.

---

## 22. Replay capture pipeline

Production traffic → bench corpus :

```text
┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│ prod traffic │ →  │ envoy tap    │ →  │ M29 PII      │ →  │ corpus       │
│ (sampled)    │    │ (1 % sample) │    │ scrubber     │    │ versioning   │
└──────────────┘    └──────────────┘    └──────────────┘    └──────────────┘
                                              │
                                       ┌──────┴───────┐
                                       │ sec review   │
                                       │ + sign-off   │
                                       └──────────────┘
```

### 22.1 Sampling

Envoy access logs at 1 % sample rate. Bodies captured for sampled
requests only. Trace IDs preserved (helps reconstruct end-to-end
sessions for Mode 3).

### 22.2 PII scrubbing

Apply M29 PII pipeline :

  - Named-entity removal (PERSON / EMAIL / PHONE / SSN / etc.).
  - Geographic generalization (city → region).
  - Pseudonymization (tenant_id → stable hash).
  - Field-level removal of fields tagged `pii=true` in protos.

### 22.3 Differential privacy budget

Per-replay-corpus, a differential-privacy budget tracks how much
information about individual records the corpus exposes :

```
ε = sum of per-query ε of all queries the corpus has answered
```

Corpora exceeding ε > 1.0 over their lifetime are retired and
replaced.

### 22.4 Versioning + access

Corpus file structure :

```
bench_corpus/replay/<scenario>/v<X.Y.Z>/
├── manifest.yaml
├── content.parquet                 # the data
├── content.sha256
├── disclosure.md                   # what's in, what's not
├── sec-signoff.json                # 4-eye approval, signed
└── access-policy.yaml              # who can read
```

---

## 23. Adversarial corpus catalog

Per surface, deliberate worst-case inputs :

### 23.1 Mode 1

  - Max body size (50 MB, near gRPC default limit).
  - Deeply-nested protobuf with 100-level field recursion.
  - Pathological category cardinality (every request a distinct
    rare category).
  - Cross-tenant tenant_id (intentional security probe).
  - Malformed protobuf (truncated, extra fields).

### 23.2 Vector

  - Adversarial vectors near k-d tree decision boundaries.
  - All-zero vector (degenerate).
  - Repeated identical vectors (forces index re-balance).

### 23.3 KG

  - Cycles introduced into the graph (detect cycle-detection cost).
  - Hub nodes with 10 000+ neighbors (fan-out blowup).
  - Disconnected components (no path queries).

### 23.4 Streaming

  - Burst : 10 × target rate for 1 second, then return to target.
  - Single-producer monopoly (one producer, all consumers blocked).
  - Backwards-in-time events (clock skew injection).

### 23.5 Multimodal

  - 4K × 4K images (max resolution).
  - 1-pixel images (min resolution).
  - Pure-noise images.
  - All-black images (no detectable features).

---

## 24. Per-surface deep-dive expansions

Volume I §14-§30 sketched 5-15 lines per surface. Here we go deeper
on the top-5 most-customer-impacting surfaces.

### 24.1 Mode 1 deep-dive (additional detail beyond Volume I §14)

Capacity model :

  - On c6i.2xlarge with 8 vCPU : single-thread inference latency 4 ms
    (CPU-bound on the latent-space dot products).
  - Up to 16 in-flight requests (2× hyperthreads).
  - Sustained throughput at p99 < 50 ms : ~ 1 800 req/s.
  - Sustained at p99 < 100 ms : ~ 3 200 req/s (Little's law).
  - Hard saturation : ~ 4 000 req/s, p99 climbs unbounded.

Operating-point grid published :

```text
QPS    p50    p99    p99.9   p99.99
 100    8 ms   12 ms   18 ms   24 ms
 250   10 ms   15 ms   24 ms   35 ms
 500   12 ms   22 ms   38 ms   62 ms
1000   14 ms   41 ms   87 ms  145 ms
1500   16 ms   58 ms  134 ms  240 ms
2000   18 ms   72 ms  182 ms  340 ms
2500   21 ms  103 ms  280 ms  520 ms
3000   25 ms  178 ms  490 ms  890 ms
3500   34 ms  340 ms  900 ms 1500 ms     ← knee
4000   80 ms  870 ms 2400 ms 4200 ms     ← saturated
```

Each entry is the median of 10 runs ± documented CI.

GPU variant (T4) : same surface, GPU-bound dot products. Saturation
at ~ 9 000 req/s, p99 < 30 ms up to 6 000 req/s.

### 24.2 Mode 2 deep-dive

Mode 2 dependency on LLM provider is the hardest honesty problem.
EFE's bench :

  - Reports **EFE-side overhead p99** (router + Mode-1 backend) :
    typically 60-80 ms.
  - Reports **provider-component p99** : 150-500 ms (highly variable).
  - Reports **end-to-end p99** : ~ 200-600 ms.

The published SLO of `p99 < 300 ms` is :

  - **EFE-controllable component** : EFE-side overhead must remain
    < 80 ms p99 (this is the part EFE actually optimizes).
  - **End-to-end customer experience** : depends on provider ; EFE
    publishes the rolling-window worst-hour p99 over 24h.

### 24.3 Vector search deep-dive

Index types :

  - HNSW : highest recall, slower build, lower query.
  - IVF-PQ : faster build, lower memory, slightly worse recall.
  - Flat : exact, slow at large N.

Per-index measurement grid :

```text
Corpus = 10M vectors, dim=768, M=16, ef_construction=200, ef_search=100

Index    Build time   Mem footprint   Recall@10   p99 query (ms)
HNSW       45 min       8.2 GB        0.96         34
IVF-PQ     12 min       0.9 GB        0.91         18
Flat       0 min       29.3 GB        1.00        180
```

Reported as Pareto across (recall, latency, memory, cost).

### 24.4 SDK latency deep-dive — Python

  - Sync client (`grpcio` blocking) : p99 = 4.2 ms.
  - Async client (`grpcio` async) : p99 = 2.1 ms.
  - With OTel : +0.3 ms p99.
  - With retries enabled : depends on transient-failure rate.
  - With JWT auth : +0.4 ms p99 (token validation).
  - Cold (first request) : 47 ms (DNS + TLS + JIT-cache warm).

### 24.5 Meta-swarm Phase 2 deep-dive

PR05-PR15 stack. Detailed measurement :

  - `MetaSwarmService::Iterate` p99 = 78 ms (3-participant, kovant
    corpus).
  - Per-participant `Step` p99 = 22 ms.
  - SIGHUP reload (no-pod-restart) p99 = 18 µs (well under 25 µs SLO).
  - Bundle weight-loader sidecar polling overhead : 80 µs CPU /
    poll, < 0.001 % overall.

---

## 25. Cross-region bench setup

For tenants on multi-region : measure latency from each region.
Fleet hosts in :

  - `eu-west-3` (Paris)
  - `us-east-1` (N. Virginia)
  - `ap-southeast-1` (Singapore)

Per-region client → cross-region server measurements report :

  - Same-region p99 (baseline).
  - Cross-region p99 (with network RTT).
  - Cross-cloud p99 (AWS Paris → GCP us-central).

---

## 26. A/B comparison workflow

Pre-registration template (Volume I § 2.5). Workflow :

```text
1. Open PR with the pre-registration in bench_ab/<id>.yaml.
2. SRE + perf review the pre-registration. Approve / deny.
3. Bench framework runs the experiment (N=30, both arms).
4. Result generated automatically into bench_ab/<id>-result.yaml.
5. SRE + perf + product review the result.
6. Decision : promote / not-promote, documented in the result file.
7. If promoted, baseline is updated.
```

Forbidden : adjusting the pre-registration after seeing results.
Result files are append-only, immutable.

---

## 27. Drift bisection automation

When drift detected nightly :

```bash
# bench-bisect.sh
PR_LIST=$(git log --format=%H mainline-N..mainline)
echo "$PR_LIST" | xargs -L1 -I{} bash -c '
    git checkout {}
    ./scripts/bench/quick.sh --surface mode1 > /tmp/{}.json
'
# Find the first PR where p99 > threshold
python3 scripts/bench/find_culprit.py \
    --threshold 41.3 \
    --pct-margin 5 \
    --runs /tmp/*.json
```

Output : the PR sha + author + commit message. Issue auto-filed.

---

## 28. Bench-as-SBOM

Every bench result emits a CycloneDX SBOM listing :

  - The bench harness version + dependencies.
  - The container digests of services-under-test.
  - The corpus version + hash.
  - The host OS + kernel + glibc.

```bash
syft scan dir:bench_run_<uuid> -o cyclonedx-json=bench-sbom.json
```

This satisfies SLSA-L3 provenance requirements when EFE publishes
benches as part of release artifacts.

---

## 29. SLSA-L3 for bench results

  - Bench script + manifest signed with cosign.
  - Bench-result artefact signed.
  - Build provenance : the bench harness was built from a specific
    commit by a specific (auditable) CI workflow.
  - Tamper-evident : signature mismatch → publication rejected.

Integration with existing `release-sbom.yml` workflow.

---

## 30. Energy measurement deep-dive

### 30.1 Intel RAPL

```bash
# Read package energy
cat /sys/class/powercap/intel-rapl:0/energy_uj
# 23456789012   (microjoules, monotonically increasing)
```

Sample at 100 ms intervals during bench run. Integrate to get total
energy. Divide by request count for W·h/req.

Caveats :

  - 32-bit counter overflows ~ every 60 s on modern CPUs ; framework
    detects + handles wraparound.
  - DRAM energy on a separate counter (`intel-rapl:0:0`).
  - Bare-metal only ; virtualized hosts return constant 0 or no
    access.

### 30.2 ARM PMU

Counter-based estimation on Neoverse N1/N2 :

```bash
perf stat -e power/energy-pkg/,power/energy-ram/ -- bench-command
```

Available on Graviton 3 bare-metal ; not on virtualized Graviton.

### 30.3 NVIDIA NVML

```bash
nvidia-smi --query-gpu=power.draw --format=csv -l 1
```

Reported in watts. Integrate over time.

### 30.4 Cross-component reporting

Total energy = CPU + DRAM + GPU + storage estimate.

Per-component disclosure required.

---

## 31. Cost integration deep-dive

### 31.1 AWS Pricing API

```python
import boto3
pricing = boto3.client("pricing", region_name="us-east-1")
response = pricing.get_products(
    ServiceCode="AmazonEC2",
    Filters=[
        {"Type": "TERM_MATCH", "Field": "instanceType", "Value": "c6i.2xlarge"},
        {"Type": "TERM_MATCH", "Field": "location", "Value": "Europe (Paris)"},
    ],
)
```

Snapshot recorded in manifest at bench start.

### 31.2 GCP Pricing API

Different schema ; framework normalizes.

### 31.3 Azure Pricing API

Different again. Framework abstracts.

### 31.4 Egress, storage, NAT

Per-bench, the cost includes EC2 + EBS + NAT gateway egress. Framework
sums.

---

## 32. Reproducibility hashes

Every bench output is reproducible byte-for-byte modulo timestamps :

  - Random seed pinned per AD12.
  - Corpus order deterministic.
  - `SOURCE_DATE_EPOCH` set for any embedded date.
  - HDR histogram serialization deterministic.

`bench_run_<uuid>/repro-hash.txt` records :

  - Input hash : (corpus + manifest + bench-harness-sha).
  - Output hash : sha256 of result.json with `ran_at` stripped.

A re-run from clean state must produce the same output hash. Drift
indicates non-determinism (a bug to fix).

---

## 33. Concrete K8s manifest examples

See Volume II § 13. Full manifests in `k8s/bench-fleet/`.

---

## 34. Concrete Prometheus rule examples

```yaml
# k8s/bench-fleet/prometheus-rules/bench-drift.yaml
groups:
  - name: efe-bench-drift
    interval: 5m
    rules:
      - alert: BenchP99DriftWarn
        expr: |
          (
            (efe_bench_p99_current / efe_bench_p99_baseline) - 1
          ) > 0.05
        for: 30m
        labels:
          severity: warning
        annotations:
          summary: "Bench p99 drift > 5% on {{ $labels.surface }}"

      - alert: BenchP99DriftMajor
        expr: |
          (
            (efe_bench_p99_current / efe_bench_p99_baseline) - 1
          ) > 0.20
        for: 15m
        labels:
          severity: major
        annotations:
          summary: "Bench p99 drift > 20% on {{ $labels.surface }} — page on-call"
```

---

## 35. Concrete Grafana dashboard JSON skeletons

The bench-portal dashboard, exported as Grafana JSON, lives in
`k8s/bench-fleet/grafana/dashboards/`. Skeleton :

```json
{
  "uid": "efe-bench-master",
  "title": "EFE — Honest Bench Master",
  "panels": [
    {
      "type": "stat",
      "title": "Mode-1 p99",
      "targets": [
        { "expr": "efe_bench_p99{surface=\"mode1\",tier=\"prod\"}" }
      ]
    },
    {
      "type": "timeseries",
      "title": "Mode-1 p99 over time",
      "targets": [
        { "expr": "efe_bench_p99{surface=\"mode1\",tier=\"prod\"}", "legendFormat": "p99" },
        { "expr": "efe_bench_p999{surface=\"mode1\",tier=\"prod\"}", "legendFormat": "p99.9" },
        { "expr": "efe_bench_p9999{surface=\"mode1\",tier=\"prod\"}", "legendFormat": "p99.99" }
      ]
    }
  ]
}
```

---

## 36. Bench output deserialization examples

### Python

```python
import json
from pathlib import Path

data = json.loads(Path("bench_run_<uuid>/result.json").read_text())
p99 = data["measurements"]["latency"]["p99"]["value"]
ci_lo = data["measurements"]["latency"]["p99"]["ci_99_lo"]
ci_hi = data["measurements"]["latency"]["p99"]["ci_99_hi"]
print(f"p99 = {p99:.2f} ms [{ci_lo:.2f}, {ci_hi:.2f}]")
```

### Rust

```rust
use serde::Deserialize;

#[derive(Deserialize)]
struct BenchResult {
    measurements: Measurements,
    // ...
}
let r: BenchResult = serde_json::from_reader(File::open("result.json")?)?;
```

### Go, TypeScript, Java

Idiomatic equivalents.

---

## 37. Per-SDK example bench programs

`clients/python/benches/echo_mode1.py` :

```python
import asyncio
from efe import EfeClient
from efe.bench import run_bench, BenchCase

class EchoMode1(BenchCase):
    name = "echo-mode1"
    async def cold(self, client):
        await client.classify("warmup")
    async def warm(self, client, n):
        for _ in range(n):
            t0 = time.perf_counter()
            await client.classify("hello")
            yield (time.perf_counter() - t0) * 1000.0

async def main():
    client = EfeClient(endpoint="grpc://localhost:50051")
    result = await run_bench(EchoMode1(), qps=1000, duration_s=120)
    print(result.markdown())
asyncio.run(main())
```

Equivalent for the other 3 SDKs.

---

## 38. Meta-bench — the framework benchmarks itself

The framework's own overhead must be < 1 % of measured latency
(Volume I AD22).

Meta-bench :

  - Run a "no-op" target : a gRPC echo that returns instantly.
  - Measure the round-trip p99 via the bench harness.
  - Subtract the known network + serialization cost.
  - Remainder = bench-harness overhead.
  - Threshold : < 50 µs at p99.

CI gate ensures the meta-bench passes on every framework PR.

---

## 39. Glossary

| Term | Meaning |
|---|---|
| HDR | HdrHistogram, high-dynamic-range histogram |
| AD/AC | Architectural Decision / Acceptance Criterion |
| Tier | Host class : laptop / ci / staging / prod |
| Pareto | Tradeoff curve between throughput and latency |
| RAPL | Running Average Power Limit (Intel) |
| NVML | NVIDIA Management Library |
| BBR | Bottleneck Bandwidth and Round-trip propagation time (TCP CC) |
| SO_REUSEPORT | Multi-process bind on same port (kernel-side LB) |
| THP | Transparent Huge Pages |
| Coordinated omission | Systematic latency under-reporting via skipped requests |
| EMD | Earth Mover's Distance |
| KL | Kullback-Leibler divergence |
| MDE | Minimum Detectable Effect (power analysis) |
| FDR | False Discovery Rate |
| Bootstrap CI | Resampling-based confidence interval |
| Bisect | Binary search through commits to find a regression |

---

## 40. Index

```text
A/B comparison ............................ § 26
Auditor protocol .......................... § 19
Bench fleet K8s ........................... § 13
Bench fleet cost .......................... § 14
Bench-as-SBOM ............................. § 28
Bisection (drift) ......................... § 27
Cold-start cascade ........................ § 4
Cost integration .......................... § 31
Cross-cloud bias .......................... § 6
Cross-region setup ........................ § 25
Customer-facing portal .................... § 18
Drift on-call ............................. § 16
Energy measurement ........................ § 30
Failure-mode catalog ...................... § 20
GPU specifics ............................. § 5
Grafana dashboards ........................ § 35
HdrHistogram .............................. § 1
K8s manifest examples ..................... § 33
Linux kernel tuning ....................... § 10
LLM provider non-stationarity ............. § 7
Memory allocator .......................... § 8
Meta-bench ................................ § 38
Multi-tenant interference ................. § 3
Multi-region .............................. § 25
Network stack tuning ...................... § 9
Pre-registration template ................. § 2.5
Prometheus rules .......................... § 34
Public publication ........................ § 17
RACI ...................................... § 15
Replay capture ............................ § 22
Reproducibility hashes .................... § 32
Rust trait contracts ...................... § 11
SDK contracts ............................. § 12
SLSA-L3 ................................... § 29
Statistical foundations II ................ § 2
Synthetic corpus generators ............... § 21
Adversarial corpus catalog ................ § 23
Per-surface deep-dives .................... § 24
```

---

## End of Volume II

Volume I + Volume II together form the complete EFE Honest Benchmarks
architecture. Migration plan in Volume I § 45 — 22 weeks to full
coverage.

Volume III (operational runbooks) will be authored once the first
6 surfaces have landed on the framework and we have empirical evidence
on what the on-call documentation actually needs. Target Q4 2026.
