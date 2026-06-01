# EFE — Honest Benchmarks Architecture

| Field | Value |
|---|---|
| **Doc type** | Foundational architecture (cross-cutting, not PR-scoped) |
| **Scope** | The benchmark framework + policy + per-surface designs for the entire EFE project (Mode 1/2/3, M5-M33, Phase-1/Phase-2 meta-swarm, 4 SDKs, edge, billing, vector, KG, multimodal, agent runtime, FFI, backup, streaming, compliance) |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× sre + 1× sec + 1× perf + 1× finops + 1× data-science + 1× compliance + 1× ops + 1× legal |
| **Depends on** | `bench_baselines/v0.9.0/ops_master_slos.yaml` (existing), `docs/EFE_A1_BENCH_TRACKING.md`, `docs/EFE_BENCH_EXTENSION_ARCHITECTURE.md` (extension proposal — this doc supersedes the *foundational* parts of it) |
| **Supersedes** | Implicit bench conventions accreted across M5-M33. Inconsistent percentile reporting. Mixed time-units (ms/µs/ns) across reports. |
| **Blocks** | Public v1 GA bench publication (`release-v1-ga.yml`) without honest-bench compliance. |
| **L-gate** | L2 (Phase 2 graduation) requires every SLO in `ops_master_slos.yaml` to be measured against this framework. |
| **Threat IDs in scope** | T1–T18 (ways benchmarks lie — § 6) ; T19–T22 (chain-of-custody for public claims) ; T23–T26 (Goodhart's law specifically for self-benchmarking) |
| **CVSS bandwidth** | All ≤ SEV-3 (this is a measurement framework, not a runtime path) — but **dishonest benchmarks** can cause SEV-1 customer outages when shipped SLOs don't match production reality |

---

## Table of contents

  0. TL;DR
  1. Mission + ground-truth definitions
  2. Non-goals (out of scope)
  3. The two definitions : "real" vs "honest"
  4. Pre-flight : current state of EFE benches (audit)
  5. Pre-flight findings (what's broken, what's not measured)
  6. Threat model — 18 ways benchmarks lie (T1–T18)
  7. Architectural decisions (AD1–AD30)
  8. Bench taxonomy — 5 orthogonal axes
  9. Workload corpus design (synthetic + replay + adversarial)
 10. Host environment standardization — 4 tiers
 11. Statistical foundations
 12. Coordinated omission — Gil Tene's correction
 13. Anti-cherry-pick reporting policy
 14. Per-surface bench design — A. Core inference (Mode 1/2/3)
 15. Per-surface bench design — B. Bundle / manifest lifecycle
 16. Per-surface bench design — C. Audit + compliance
 17. Per-surface bench design — D. Multi-tenancy + lifecycle
 18. Per-surface bench design — E. Streaming + events (M28)
 19. Per-surface bench design — F. Active learning (M24)
 20. Per-surface bench design — G. Vector search (M32)
 21. Per-surface bench design — H. Knowledge graph (M33)
 22. Per-surface bench design — I. Multimodal (M30)
 23. Per-surface bench design — J. Edge (M27)
 24. Per-surface bench design — K. Billing + FinOps (M31)
 25. Per-surface bench design — L. Agent runtime (M25/M26)
 26. Per-surface bench design — M. Meta-swarm Phase 2 (PR05-PR15)
 27. Per-surface bench design — N. SDK clients (Py / TS / Java / Go)
 28. Per-surface bench design — O. FFI / Python in-process
 29. Per-surface bench design — P. Backup / DR
 30. Per-surface bench design — Q. Operational (cold start, probes, k8s-dns)
 31. Cross-cutting — soak + chaos
 32. Cross-cutting — hardware matrix (ARM64 / AMD64 / GPU)
 33. Cross-cutting — baseline tracking + drift detection
 34. Cross-cutting — comparison policy (vs prior version, vs competitors)
 35. Reporting — dashboards, schemas, percentile reporting
 36. CI integration — master gate + nightly + soak
 37. Cost integration — $/req with FinOps
 38. Energy integration — W·h/req with RAPL
 39. Reproducibility — one-command reproducer
 40. Public bench publication policy
 41. Acceptance criteria (AC1–AC30)
 42. Tests strategy — the bench framework tests itself
 43. Risks + mitigations
 44. Rollback strategy
 45. Migration path from current bench infra
 46. Sequence diagrams
 47. Reviewer checklist
 48. Appendix A — environment disclosure manifest schema
 49. Appendix B — statistical formulas
 50. Appendix C — coordinated-omission worked example
 51. Appendix D — per-surface SLO target catalog
 52. Appendix E — corpus versioning policy
 53. Appendix F — bench-output JSON schema (v1)
 54. Appendix G — anti-patterns gallery (annotated)

---

## 0. TL;DR

EFE has 18 master SLOs (consolidated in `bench_baselines/v0.9.0/ops_master_slos.yaml`).
It has bench infrastructure scattered across 5 crates (`efe-metering`,
`efe-vector`, `efe-core`, `efe-kg`, `efe-meta-swarm`), per-mode bench
files (`scripts/bench_runner.sh`), per-sprint baselines (v0.6.0 AL,
v0.7.0 stream, v0.8.0 compliance + LATACT, v0.9.0 master), and ad-hoc
`cargo bench` targets. The state is **not dishonest, but not yet
*provably* honest** : we don't enforce coordinated-omission correction,
we don't always report p99.9, we don't always publish the corpus
version with the measurement, the multi-tenant interference baseline
is uneven, and cold/warm cache deltas are reported on some surfaces
and not others.

This doc defines the **EFE Honest Bench Framework** : a foundational
contract for *every* measurement claim that EFE makes — internal SLOs,
public marketing benches, regression-gating CI checks, vendor RFP
responses, and on-call observability calibration.

The principle, in one sentence :

> **"If we can't reproduce it from a one-command script + a publicly
> verifiable corpus + a fully disclosed host profile + a full
> percentile distribution including p99.9, we don't publish the
> number — even internally."**

30 architectural decisions (AD1–AD30) operationalize that principle.
30 acceptance criteria (AC1–AC30) gate the bench framework itself.
18 threat models (T1–T18) name the specific lies that this
framework refuses.  17 per-surface designs (§ 14–§ 30) describe how
to measure each EFE module honestly.  Cross-cutting sections cover
soak (§ 31), hardware (§ 32), drift (§ 33), comparison policy (§ 34),
reporting (§ 35), CI (§ 36), cost (§ 37), energy (§ 38),
reproducibility (§ 39), and public publication (§ 40).

The migration path (§ 45) accepts existing benches *as-is* into a
"legacy" tier ; new benches added after this doc's frozen-date must
pass the AC1-AC30 gates.  Legacy benches are migrated by
sprint-A4-W2 deadline (target Q3 2026 per the ROADMAP).

---

## 1. Mission + ground-truth definitions

### Mission

Ship a benchmark framework + policy + per-surface designs such that :

  1. Every EFE SLO published anywhere (docs/ , marketing site, RFP
     responses, regulatory filings) is **traceable** to a frozen
     workload corpus + a fully disclosed host profile + a re-runnable
     reproducer + a full percentile distribution.
  2. Every EFE on-call paging threshold is **calibrated** against a
     bench measurement (no thresholds set "by feel").
  3. Every EFE pull-request that touches a perf-sensitive crate
     surfaces a delta-from-baseline measurement in the PR review
     (positive or negative — both reported).
  4. Every EFE customer-facing latency claim can be re-derived by
     any auditor in under 1 hour using only the public bench scripts
     + the public corpus + a documented hardware profile.
  5. Every regression detected in nightly bench surfaces as a GitHub
     issue with the exact commit-hash window + the responsible PR
     bisected automatically.
  6. The framework **itself** is benchmarked (meta-bench : how long
     does the bench framework's overhead add to a measured request ?
     answer must be < 1 % of measured latency, AC22).

### Ground-truth definitions

A bench measurement is **trustworthy iff** the following are simultaneously
true (Daubert-test-style standard, borrowed from forensic science) :

  - **Reproducible** : a third party with identical hardware + identical
    corpus + identical container digest gets numbers within the
    documented confidence interval. Documented as AC10.
  - **Falsifiable** : the measurement's claim is specific enough that
    one can construct a counter-example. ("p99 < 50 ms on Mode-1 with
    the kovant-prod corpus on the c6i.2xlarge tier" is falsifiable.
    "fast" is not.)
  - **Disclosed** : every parameter that *could* influence the number
    is in the bench manifest. Includes CPU model, RAM size + speed,
    kernel version, glibc version, jemalloc version (if used), the
    container digest, the corpus version, the random seed, the
    feature flags, the rate limit settings.
  - **Calibrated** : the bench framework itself has been measured
    against a known-good external reference (e.g. tonic-bench, k6
    standard benchmarks) and the delta is documented.
  - **Statistically rigorous** : sample size satisfies power analysis
    (§ 11.4), bootstrap CI computed (§ 11.5), variance reported
    alongside the central tendency (§ 11.6).

A bench measurement is **honest iff** it is *trustworthy* AND :

  - **Full disclosure** : the full distribution is published, not just
    the favoured percentiles (AC14).
  - **Excluded data declared** : any warm-up samples, retries,
    timeouts, or outliers EXCLUDED from the headline number are
    explicitly listed with their count and rationale (AC13).
  - **Negative space declared** : the surfaces *not* benchmarked, and
    the operating points *not* explored, are explicitly listed (AC30).
  - **Comparison fairness** : when comparing against a prior version
    or a competitor, the comparison uses *identical* host + corpus +
    parameters, or the differences are explicitly listed (AC16).

This doc is the contract that makes both definitions *checkable* —
not just aspirational.

---

## 2. Non-goals (out of scope)

  - **Not** a replacement for `cargo bench` / Criterion. Criterion
    remains the micro-benchmark primitive for in-process measurements
    (AD2). This framework wraps Criterion for service-level + multi-
    process + multi-host scenarios where Criterion alone is
    insufficient.
  - **Not** a load testing tool. We integrate k6 + Locust (AD4) for
    sustained-traffic generation but the framework is about *honest
    measurement*, not stress.
  - **Not** a security testing framework. CVE scans, fuzzing, OSS-Fuzz
    integration live in their own workflow (`master-gate.yml` security
    job). This framework does, however, bench *resilience under
    attack* (chaos benches, § 31).
  - **Not** the place where new SLOs are decided. SLO definition is
    a product+SRE conversation (`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`
    § 8). This framework is how SLOs are *measured*, not how they're
    chosen.
  - **Not** a benchmark of theoretical maximum throughput. EFE
    publishes *realistic-load* numbers (§ 9 corpus is mixed
    skew-typical, not max-batch). Theoretical numbers (e.g.
    "10M reqs/sec on `dev/null`") are not published.
  - **Not** a place for marketing comparisons against competitors
    without disclosure. § 34 comparison policy is explicit : any
    competitor mention requires a side-by-side parameter table with
    every divergence flagged.
  - **Not** a replacement for production observability (Prometheus /
    Grafana / Loki). Production observability measures the *actual*
    customer workload ; this framework measures *standardized*
    workloads on standardized hosts. The two are reconciled via the
    "drift" check (§ 33).
  - **Not** a place to bench third-party dependencies as a proxy for
    EFE (e.g. "we benchmark Postgres" — no, we benchmark EFE *using*
    Postgres, the difference matters).

---

## 3. The two definitions : "real" vs "honest"

These are commonly conflated. Distinguishing them is fundamental.

### 3.1 "Real" benchmarks

A bench is *real* iff its measurement reflects production-like
conditions :

  - **Production-like workload** : not single-record max-batch, not
    perfectly-shaped uniform requests. Mixed skew (per § 9.2), real
    request bodies (per § 9.3), real tenant cardinality (per § 9.4).
  - **Production-like topology** : not in-process, not single-server.
    The bench harness drives the system *as a client would* — over
    real network, via real TLS, with real auth (or auth-disabled with
    a disclosure flag).
  - **Production-like concurrency** : not single-client warm-pool,
    but multi-client variable QPS hitting a multi-tenant deployment
    with realistic noisy-neighbour interference.
  - **Production-like environment** : not the developer's M-series
    Mac with thermal headroom, but a host tier that matches the
    customer's actual deployment surface (commodity cloud VM, or
    bare-metal class-A — per § 10).
  - **Production-like failure modes** : retries, timeouts, partial
    failures, slow disks, throttled CPU, network jitter — all
    encountered during the measurement window, not artificially
    excluded.

### 3.2 "Honest" benchmarks

A bench is *honest* iff its *report* :

  - **Discloses** every choice that could move the number (§ 1
    "Disclosed").
  - **Reports** the full distribution, not just preferred percentiles
    (AC14).
  - **Names** what was excluded and why (warm-up, outliers, errors)
    (AC13).
  - **Names** what was NOT measured (AC30).
  - **Provides** a reproducer that re-derives the number (AC10).
  - **Uses** statistical methods that are not gameable (HdrHistogram
    + bootstrap CI : AD1 + AD11).
  - **Refuses** the comparison-anti-patterns enumerated in § 6 (T1-T18).

The two concepts are *orthogonal* :

  - A *real* but *dishonest* bench : run in production-like conditions
    but reported as "p50 = 12 ms" without the p99.9 = 280 ms tail.
    Worse than useless because it gives false confidence.
  - An *unreal* but *honest* bench : "in-process micro-bench of the
    `embed()` function shows 8 µs ± 0.1 µs @ p99 with bootstrap
    99 % CI [7.9, 8.2] on commodity cloud VM, corpus_v1.2, with
    explicit disclosure that this excludes serialization, network,
    auth, and tenant lookup which together add an estimated 4-12 ms".
    Useful for diagnosing where time goes, useless for capacity
    planning.

The framework demands **both** real AND honest for any number
published outside of internal-perf-investigation contexts.

---

## 4. Pre-flight : current state of EFE benches (audit)

This section is the literal current state, derived from the 14-May-2026
audit + the prior bench tracking doc.

### 4.1 What exists

```text
bench_baselines/
└── v0.9.0/
    └── ops_master_slos.yaml    18 master SLOs frozen 2026-05-12

crates/
├── efe-metering/               [[bench]] target
├── efe-vector/                 [[bench]] target
├── efe-core/                   [[bench]] target
├── efe-kg/                     [[bench]] target
├── efe-meta-swarm/             benches/three_swarms_kovant_latency.rs
├── efe-server/                 [[bench]] : real_strategies_e2e
│                                          real_memory
│                                          server_rpc
│   src/bin/                    (controllers + binaries)
│
└── (other crates)

scripts/
├── bench_runner.sh             17 870 bytes — multi-mode dispatcher
└── (no other bench infra)

docs/
├── EFE_A1_BENCH_TRACKING.md    bench tracking history
├── EFE_BENCH_EXTENSION_ARCHITECTURE.md   bench extension proposal
└── EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md § 8  SLO origin
```

### 4.2 What's measured per surface (current)

| Surface | Have bench? | Per-percentile? | Multi-tenant? | Reproducer? | Public corpus? |
|---|---|---|---|---|---|
| Mode 1 (classify p99) | ✓ | p99 only | partial | yes | partial |
| Mode 2 (LLM p99) | ✓ | p99 only | no | no | no |
| Mode 3 (multi-turn) | ✓ | p99 only | no | no | no |
| Bundle reload | ✓ | median only | no | yes | partial |
| Cold start | ✓ | median only | n/a | yes | n/a |
| SDK Python p99 | ✓ | p99 only | no | yes | yes |
| SDK TS / Java / Go | partial | varies | no | no | no |
| Audit chain write | partial | no | no | no | no |
| Vector search | ✓ | p99 only | no | yes | partial |
| Knowledge graph | partial | varies | no | no | no |
| Streaming | partial | varies | no | no | no |
| Active learning | partial | varies | no | no | no |
| Multimodal | no | no | no | no | no |
| Edge (M27) | no | no | no | no | no |
| Billing (M31) | no | no | no | no | no |
| Agent runtime (M25/M26) | partial | varies | no | no | no |
| FFI (M24 in-proc) | partial | varies | no | yes | no |
| Backup / DR | no | no | n/a | no | n/a |
| Meta-swarm (Phase 2) | ✓ | varies | partial | yes | yes |

Roughly half the M-series surfaces are unmeasured, and almost every
existing measurement reports only one percentile (typically p99)
without bootstrap CI, without coordinated-omission correction, without
the full distribution.

### 4.3 What works well today

  - Mode-1 latency : long-running, well-calibrated, p99 baseline at
    41 ms with documented 50 ms target. Reproducible.
  - SDK Python latency : 2.1 ms median, < 5 ms target. Reproducible.
  - The 18-SLO consolidated bundle (`ops_master_slos.yaml`) is a
    well-structured master document with explicit burn-rate matrix.

### 4.4 What's broken or absent today

  - **No coordinated-omission correction anywhere**. Latency-under-load
    bench numbers are systematically optimistic (T1).
  - **No multi-tenant interference baseline**. Tenant burst → tenant
    p99 impact is unmeasured (T3).
  - **No public corpus versioning**. Corpus may have drifted since
    v0.6.0 measurement ; we can't tell (T19 — chain of custody).
  - **Cold-cache deltas inconsistent**. Some surfaces report cold ;
    some only warm (T2).
  - **Soak duration uneven**. M-series surfaces typically not
    soak-tested beyond 1 h ; memory leak detection limited (T16).
  - **Chaos integration absent**. Resilience SLOs (e.g. "mean time
    to recovery") declared but not bench-measured (T17).
  - **Energy not measured**. W·h/req not even in the SLO bundle.
    Cost ($/req) is in K1-K4 but not derived from energy.
  - **Hardware matrix narrow**. Most benches run on AMD64 only ; ARM64
    + GPU coverage uneven (T6).
  - **No drift detection runs nightly**. Drift caught only at PR-time
    against the v0.9.0 frozen baseline. New regressions land for
    days before being noticed.
  - **No "negative space" disclosure**. The SLO bundle doesn't list
    surfaces *not* covered (AC30 violation).
  - **Per-PR bench impact not surfaced**. Bench numbers move silently
    between PRs.

This framework addresses each of these (§ 14-§ 30 + § 31-§ 40 + AC1-30).

---

## 5. Pre-flight findings — summary table

| Finding | Severity | Coverage in this doc |
|---|---|---|
| Coordinated omission not corrected | **HIGH** | AD1 (HdrHistogram) ; § 12 ; AC22 |
| No multi-tenant baseline | **HIGH** | AD29 ; § 17 ; AC20 |
| Cold/warm cache uneven | **MEDIUM** | AD26 ; § 14-30 per surface ; AC8 |
| Public corpus not versioned | **HIGH** | AD6 ; § 9 ; AC2 |
| Single-percentile reporting | **HIGH** | AD13 ; § 13 ; AC3 |
| No bootstrap CI | **MEDIUM** | AD11 ; § 11.5 ; AC4 |
| Soak uneven | **MEDIUM** | AD15 ; § 31 ; AC15 |
| Chaos absent | **MEDIUM** | AD16 ; § 31 ; AC19 |
| Energy unmeasured | **LOW** | AD18 ; § 38 ; AC17 |
| Hardware matrix narrow | **MEDIUM** | AD25 ; § 32 ; AC21 |
| Drift caught only at PR time | **MEDIUM** | AD19, AD20 ; § 33 ; AC12 |
| Negative space undisclosed | **MEDIUM** | AD30 ; § 40 ; AC30 |
| Per-PR delta invisible | **MEDIUM** | AD19 ; § 33, § 36 ; AC23 |

---

## 6. Threat model — 18 ways benchmarks lie

This is the heart of the "honest" half. Each threat is named, defined,
and matched to its mitigation. The framework refuses these threats by
construction.

### T1 — Coordinated omission

**Definition** : When a benchmark client measures latency by issuing
a request, waiting for response, then issuing the next request, slow
samples that exceed the request interval cause the *client* to skip
issuing the queued-but-undispatched requests. The result : the
measured latency distribution systematically under-reports the tail
because the slow samples that *would have been delayed by queueing*
are silently dropped.

**Concrete example** : Client wants 1 000 req/s, so issues a request
every 1 ms. If a single response takes 50 ms, the client did not
issue requests during those 50 ms. The actual p99 in production
would be ~50 ms because those queued requests would have stacked up.
The measured p99 is ~1 ms because the client never sent them.

Reference : Gil Tene's talk *"How NOT to Measure Latency"*.

**Mitigation** : AD1 (HdrHistogram or equivalent with explicit
coordinated-omission correction). Every latency measurement in EFE
records both `serviceTime` (uncorrected) and `intendedServiceTime`
(corrected for the requested rate).

### T2 — Hot-cache hiding cold-cache truth

**Definition** : Warm-up phase fills caches (CPU L1/L2/L3, allocator
slabs, database query plan cache, file system page cache, network
connection pool, gRPC HTTP/2 streams). After warm-up, latency is
artificially low. Customers hitting EFE for the first time after a
deployment experience cold-cache latency that the bench doesn't
report.

**Concrete example** : EFE-server cold-start latency is 3.2 s
(documented baseline). First request after cold start incurs an
additional ~80 ms because the embed-anchor cache is empty. If the
bench measures requests 1001-2000 only, the first-1000-after-cold-start
penalty is invisible.

**Mitigation** : AD26. Every cacheable surface reports cold AND warm
separately, plus the cold/warm ratio. SLOs are stated for both regimes.

### T3 — Single-tenant hiding multi-tenant contention

**Definition** : Bench runs with a single tenant. Production has 100+
tenants on the same instance. Tenant lookups, audit chain locks,
rate-limit counters, connection pool checkouts all serialize across
tenants. Single-tenant bench shows 12 ms ; same surface at 50 tenants
with skewed traffic shows 47 ms.

**Mitigation** : AD29. Every service-level bench mandates at least one
multi-tenant variant : 8-tenant standard skew, plus a 100-tenant
adversarial variant for top-5 hot paths.

### T4 — Average latency hiding tail

**Definition** : Reports the mean. The mean hides the long tail
that customers actually experience on a non-trivial fraction of
requests. "Average response time : 23 ms" can be consistent with
50 % of requests at 5 ms and 50 % at 41 ms — a bimodal distribution
that *no* customer would describe as "averaging 23 ms".

**Mitigation** : AD13. Reports {p50, p75, p90, p95, p99, p99.9,
p99.99}. Means are reported only as a supplementary curiosity, never
as the headline.

### T5 — Throughput-latency point hiding the curve

**Definition** : Reports throughput at saturation but latency at a
single under-loaded operating point. The classic Litlle's-law
trade-off is invisible : "We do 12 000 req/s with p99 = 14 ms"
omits that 12 000 req/s comes with p99 = 230 ms, while p99 = 14 ms
is achievable only up to ~3 000 req/s.

**Mitigation** : AD14. Every bench reports the throughput-latency
Pareto curve as a function (typically 5-10 operating points from
10 % to 100 % of capacity).

### T6 — Cherry-picked SKU / hardware tier

**Definition** : Bench runs on the fastest available hardware (or
the customer's specific hardware that happens to be unusually fast).
Customers on lower tiers get worse numbers than the bench advertised.

**Mitigation** : AD7 + AD25. Standardized 4 host tiers (laptop / CI /
staging / prod), each fully disclosed. Public bench numbers are
reported per tier ; comparison across tiers is explicit.

### T7 — Apples-to-oranges (different feature flags)

**Definition** : The bench runs with `EFE_AUTH_ENABLED=false`,
`EFE_AUDIT_ENABLED=false`, `EFE_RATE_LIMIT_ENABLED=false`, and
publishes a number that customers will never see in production
because production has all three enabled.

**Mitigation** : AD9 + AD23. Container digest pinned ; feature flags
in the bench manifest ; comparison gates check feature-flag identity.

### T8 — Marketing percentile

**Definition** : Publishes only the percentile that flatters the
product. p50 looks great, p99 is hidden. Or worse, p10 / p25
("most users") is published.

**Mitigation** : AD13 + AC14. Full distribution OR no publication.

### T9 — Best-of-N (cherry-pick the fastest run)

**Definition** : Runs the bench 10 times, reports the fastest. The
variance across runs is invisible. Cherry-picking the fastest run
is *especially* misleading when the bench is short and the system
has high variance.

**Mitigation** : AD12. Minimum N=10 runs ; reports the *median run's*
distribution + variance across runs. The "fastest run" is reported
in passing if at all.

### T10 — JIT warm-up hiding real first-request latency

**Definition** : SDK or runtime has a JIT (Java SDK on HotSpot, Go
SDK with method inlining, even Rust binaries with branch predictor
training). The first 1 000 requests are slow ; subsequent ones are
fast. The bench warms up for 1 000 requests then measures, hiding
the cold-start reality.

**Mitigation** : AD26 (cold AND warm). The cold regime explicitly
includes the JIT warm-up phase if the SDK uses one.

### T11 — Outlier removal hiding actual tail

**Definition** : "Outliers removed via 3-sigma" is sometimes used to
clean up noisy data. For latency distributions, this is *always
wrong* — the tail IS the data, and "outliers" are exactly the
customer experiences we care about.

**Mitigation** : AD13 + AC13. Outlier removal forbidden for latency
distributions. If excluded, the count + rationale are mandatory.

### T12 — Geomean abuse mixing units

**Definition** : Reports a "geomean improvement of 1.4×" computed
across benches that measure milliseconds, megabytes-per-second, and
errors-per-million. Geomean is dimensionally meaningless across
mixed units. The number is uninterpretable.

**Mitigation** : AC4 + AC14. Each metric reported in its native unit.
Cross-bench summary requires per-metric breakdown.

### T13 — Closed-source bench harness

**Definition** : "Trust us, our bench harness measures X". No
reproducer ; no auditability.

**Mitigation** : AD23 + AD24. Bench-as-code in repo ; public
reproducer ; independently verifiable.

### T14 — SLO target inflation

**Definition** : Bench publishes "we beat our SLO of 100 ms p99" —
but the SLO was set at 100 ms specifically because the team knew
the actual measurement was ~80 ms. The SLO is a marketing artefact,
not a constraint.

**Mitigation** : AD20. SLO targets are set by SRE/product based on
customer-facing capacity planning ; bench publications report the
current measurement *plus* the drift from the target *plus* the
historical baseline. No "we beat our SLO" claims without showing
the SLO's own history.

### T15 — Cost hidden

**Definition** : Compare "EFE is 2× faster than competitor" without
disclosing that EFE runs on a $0.50/h SKU while the competitor runs
on a $0.05/h SKU. The cost-per-request comparison is the inverse
of the latency comparison.

**Mitigation** : AD17 + AC18. Cost ($/req) reported alongside
latency for every public claim.

### T16 — Soak too short (memory leaks emerge later)

**Definition** : Bench runs for 1 hour, measures p99 = 14 ms. After
24 hours of production traffic, p99 climbs to 230 ms due to a slow
memory leak that triggers GC pressure / paging. The 1-hour bench
was honest *for 1 hour* but cannot extrapolate.

**Mitigation** : AD15. Soak duration matrix : 24 h short, 7 d
standard, 30 d extended.

### T17 — Latency under no load

**Definition** : Bench measures latency at 1 req/s, with no
contention, perfect cache, single client. Publishes "p99 = 4 ms".
Customer in production sees p99 = 47 ms at their actual 200 req/s
load.

**Mitigation** : AD14 + AD29. Bench reports the full throughput-
latency Pareto curve, not just one corner.

### T18 — Self-bench bias (Goodhart's law)

**Definition** : The team that ships the SLO is also the team that
runs the bench. Optimizations specifically target the bench, not
realistic customer workloads. "When a measure becomes a target, it
ceases to be a good measure."

**Mitigation** : AD30 + § 40. (a) Bench corpus is co-designed with
customer-facing teams (success / support). (b) Periodically, a
randomly-chosen team member outside perf runs the bench against a
fresh corpus and reports a "blind audit" diff. (c) Public benches
encourage independent verification.

---

## 7. Architectural decisions (AD1–AD30)

| AD  | Decision                                                                                                                                          | Status      |
|-----|---------------------------------------------------------------------------------------------------------------------------------------------------|-------------|
| AD1 | HdrHistogram for every latency capture. Records both `serviceTime` and `intendedServiceTime` (coordinated-omission correction).                  | accepted    |
| AD2 | Criterion remains the in-process micro-bench primitive. Wrap, don't replace.                                                                       | accepted    |
| AD3 | Custom gRPC bench harness (Rust + tonic) for service-level multi-client measurements. Single binary, multi-target.                                | accepted    |
| AD4 | k6 + Locust integrated for HTTP/REST surface + edge proxy + portal-admin perf benches.                                                            | accepted    |
| AD5 | Workload corpus design : synthetic + replay (anonymized prod) + adversarial. § 9.                                                                  | accepted    |
| AD6 | Corpus is versioned with semver in git (`bench_corpus/v<MAJOR.MINOR.PATCH>/`). Every measurement records its corpus version.                       | accepted    |
| AD7 | Standardized 4 host tiers : `laptop` / `ci` / `staging` / `prod`. Full disclosure manifest per tier (§ 10).                                       | accepted    |
| AD8 | CPU pinning + isolated cores via `cset shield` (Linux) for repeatability. `taskset` fallback. CPU governor pinned to `performance`.               | accepted    |
| AD9 | Pinned container images by SHA256 digest. Image signed with cosign (matches SLSA-L3 release surface).                                              | accepted    |
| AD10| Kernel + glibc + jemalloc + tonic + OS distro versions recorded in bench manifest. CI gate fails if any drifts.                                   | accepted    |
| AD11| Bootstrap 99 % CI computed for every reported number. Welch's t-test for A/B comparisons. Mann-Whitney U for non-parametric distributions.       | accepted    |
| AD12| Minimum N=10 runs per published measurement. N=30 for SLO-validation. N=100 for soak window during nightly. Per-run delta tracked.                | accepted    |
| AD13| Reports the full canonical percentile set : {p50, p75, p90, p95, p99, p99.9, p99.99}. Plus min, max, stddev. Means optional.                     | accepted    |
| AD14| Throughput-latency Pareto reported as a *curve* not a *point*. Minimum 5 operating points from 10 % to 100 % of capacity.                         | accepted    |
| AD15| Soak duration matrix : 24 h short, 7 d standard, 30 d extended (memory leak). Continuous nightly soak on top-5 surfaces.                         | accepted    |
| AD16| Chaos integration via `tc netem` (jitter / loss), CPU throttling (cgroup), OOM kill, network partition. Resilience SLOs measured under chaos.    | accepted    |
| AD17| Cost ($/req) computed via cloud pricing API snapshot at bench time. Cost in the manifest. Region disclosed.                                       | accepted    |
| AD18| Energy (W·h/req) measured via Intel RAPL on bare-metal hosts. AMD64 + ARM64 supported. Virtualized hosts mark this "N/A : virtualized".          | accepted    |
| AD19| Baseline tracking : git-versioned `bench_baselines/v<X.Y.Z>/<surface>.yaml`. Diff-on-PR via `master_gate_check.py --bench-diff`.                  | accepted    |
| AD20| Drift gates : ±5 % WARN, ±20 % FAIL. Per-percentile bands (p99 drift treated stricter than p50).                                                   | accepted    |
| AD21| Comparison policy : only same-revision-pair comparisons allowed in published outputs. Marketing comparisons require full parameter table.        | accepted    |
| AD22| Bench framework's own overhead < 1 % of measured latency. Meta-bench measures this. AC22.                                                         | accepted    |
| AD23| One-command reproducer per benchmark : `bash scripts/bench/<surface>.sh`. Docker compose for multi-process scenarios.                              | accepted    |
| AD24| Bench-as-code : every bench script + corpus + reproducer in `git`. No "internal-only" bench scripts for SLOs that ship publicly.                  | accepted    |
| AD25| Multi-arch matrix : every public bench runs ARM64 + AMD64. GPU benches run NVIDIA-T4/A100/H100 + MPS (Apple Silicon dev).                          | accepted    |
| AD26| Cold/warm cache : every cacheable surface reports both. Cold = first request after container restart. Warm = after 1000-request warm-up.          | accepted    |
| AD27| SLO error-budget burn rate computed weekly. Per-SLO burn rate visible on the bench dashboard. Sev1 when burn > 14×.                              | accepted    |
| AD28| Failure-mode benches : include timeouts, retries, partial failures. Reported as separate scenarios.                                                | accepted    |
| AD29| Multi-tenant bench is mandatory for any service touching tenants. 8-tenant standard + 100-tenant adversarial.                                     | accepted    |
| AD30| Negative space disclosed : every public bench publication lists *surfaces NOT measured* and *operating points NOT explored*.                      | accepted    |

---

## 8. Bench taxonomy — 5 orthogonal axes

Each benchmark in EFE is classified along 5 axes. The classification
drives the harness, the corpus, the host, and the report.

### Axis 1 — Scope

  - **Micro** (in-process function-level, `cargo bench` / Criterion)
  - **Component** (single crate, with its dependencies stubbed)
  - **Service** (single gRPC service, real wire)
  - **End-to-end** (client SDK → gateway → server → storage → response)
  - **System** (multi-service, multi-tenant, multi-node)

### Axis 2 — Metric

  - **Latency** (HdrHistogram, full percentile set)
  - **Throughput** (req/s under a documented latency budget)
  - **Cost** ($/req with pricing snapshot)
  - **Energy** (W·h/req via RAPL)
  - **Resilience** (recovery time, error rate under chaos)

### Axis 3 — Workload regime

  - **Cold** (post-restart, empty caches)
  - **Warm** (post-warm-up, populated caches)
  - **Sustained** (1+ hour continuous load)
  - **Burst** (1-second 10× spike)
  - **Adversarial** (degenerate / worst-case inputs)

### Axis 4 — Tenant cardinality

  - **Single** (one tenant, isolated)
  - **Few** (8 tenants, standard skew)
  - **Many** (100 tenants, adversarial skew)
  - **Cross-cloud** (tenants across AWS/GCP/Azure regions)

### Axis 5 — Hardware tier

  - **Laptop** (Apple M3 Pro or equivalent — for dev verification only)
  - **CI** (GitHub Actions ubuntu-22.04 — for CI-time regression gates)
  - **Staging** (c6i.2xlarge or equivalent — pre-prod parity)
  - **Production** (c6i.4xlarge + GPU + 10 GbE — customer-facing)

Each bench's manifest declares its position on all 5 axes. The bench
runner refuses to run a bench whose declared axes don't match the
runtime environment (e.g. running a `production` bench on a laptop
fails fast with a clear error).

---

## 9. Workload corpus design

The corpus is the most-disputed honesty risk. A "real" corpus that
nobody can reproduce isn't reproducible ; a synthetic corpus that
doesn't resemble production isn't real. The framework's approach :
co-existing synthetic + replay + adversarial corpora, each
versioned.

### 9.1 Three corpus types

**Synthetic corpus** :

  - Generated procedurally from a documented seed.
  - Mixed skew across categories (Zipf-distributed query frequencies).
  - Realistic body shapes (drawn from production size histogram).
  - Realistic tenant cardinality (Zipfian tenant access pattern).
  - Realistic field cardinalities (e.g. "purchase amounts" follow a
    log-normal, not uniform).
  - Reproducible : `seed=42`, `version=v1.2.0`, same input → same
    output bytes.
  - Public : ships with the repo at `bench_corpus/synthetic/v1.2.0/`.

**Replay corpus** :

  - Anonymized capture of production traffic.
  - Anonymization : per-field PII scrubbing + structural anonymization
    (per `efe-pii` M29).
  - Multiple production scenarios : kovant-prod / kovant-staging /
    canary / cross-cloud.
  - Versioned : `bench_corpus/replay/<scenario>/v<X.Y.Z>/`.
  - Controlled distribution : signed manifest + access-control list.
    Public *summary* (size, distribution stats) + private *raw bodies*.

**Adversarial corpus** :

  - Worst-case inputs : malformed, max-length, deeply-nested,
    duplicate-key, cross-tenant boundary-violation attempts.
  - Designed to surface tail-latency outliers + security failure
    modes.
  - Versioned : `bench_corpus/adversarial/v<X.Y.Z>/`.
  - Public.

### 9.2 Distribution shape

Every corpus declares its distribution explicitly in `corpus.yaml` :

```yaml
# bench_corpus/synthetic/v1.2.0/corpus.yaml
version:  v1.2.0
frozen_at: 2026-05-15T00:00:00Z
seed:     42
profile:
  request_body_size_bytes:
    distribution: log-normal
    median:        1024
    p99:           12000
    p99.99:        85000
  tenant_access:
    distribution: zipf
    alpha:        1.2
    cardinality:  8000     # 8000 distinct tenants
  query_categories:
    distribution: zipf
    alpha:        1.5
    cardinality:  127
  embedding_dim:
    fixed:        768
  anchor_count_per_tenant:
    distribution: power-law
    p50:          12
    p99:          487
disclosure:
  exclusions :
    - cross-tenant deep-dive queries (deferred to v1.3.0)
    - multimodal pairs (deferred to v1.3.0)
```

### 9.3 Corpus immutability

A corpus version is **frozen** : its content + manifest + content
hash are immutable after release. New data triggers a new version
(`v1.2.0 → v1.3.0`). This is essential for chain-of-custody (T19).

The framework refuses to run with a corpus whose computed hash
diverges from the manifest's declared hash.

### 9.4 Tenant cardinality matrix

| Cardinality | Use case | Corpus subset |
|---|---|---|
| 1 | Diagnostic micro-bench | `synthetic/v1.2.0/tenants-1/` |
| 8 | Standard service-level bench (AD29) | `synthetic/v1.2.0/tenants-8/` |
| 100 | Adversarial multi-tenant interference | `synthetic/v1.2.0/tenants-100/` |
| 1 000 | Long-soak realism | `replay/kovant-prod/v0.9.0/tenants-1k/` |
| 10 000 | Capacity-stress | `synthetic/v1.2.0/tenants-10k/` |

### 9.5 Negative-space disclosure (corpus side)

The corpus manifest's `disclosure.exclusions` field is mandatory and
machine-verifiable. The bench framework refuses to publish if the
exclusions list is empty (because every corpus has limits ; an
empty exclusion list signals dishonesty by omission).

---

## 10. Host environment standardization — 4 tiers

A measurement on a host whose specs differ across runs is unreliable.
EFE defines 4 standard host profiles. Each bench manifest declares
which tier(s) it runs on.

### 10.1 Tier definitions

```text
┌──────────────┬─────────────────────────────────────────────────────────┐
│ Tier         │ Spec                                                    │
├──────────────┼─────────────────────────────────────────────────────────┤
│ laptop       │ Apple M3 Pro 12C/18C GPU 36GB RAM                       │
│              │ Used for : dev sanity checks ; never for published nums │
│              │ Energy : N/A (virtualized BSD-derived OS, no RAPL)      │
├──────────────┼─────────────────────────────────────────────────────────┤
│ ci           │ GitHub Actions ubuntu-22.04 (4 vCPU AMD EPYC 7763, 16GB)│
│              │ Used for : per-PR regression gates ; CI-time soaks      │
│              │ Energy : N/A (shared VM)                                │
├──────────────┼─────────────────────────────────────────────────────────┤
│ staging      │ AWS c6i.2xlarge x86_64 (Ice Lake 3.5 GHz, 16GB)         │
│              │ + 1× NVIDIA T4 16GB for GPU benches                     │
│              │ Used for : pre-prod parity, published SLO measurement    │
│              │ Energy : RAPL on bare-metal staging-bm class             │
├──────────────┼─────────────────────────────────────────────────────────┤
│ prod         │ AWS c6i.4xlarge x86_64 + 10 GbE + 1× NVIDIA A100 40GB   │
│              │ Used for : customer-facing SLO publication               │
│              │ Energy : RAPL (bare-metal prod hosts also covered)       │
└──────────────┴─────────────────────────────────────────────────────────┘
```

### 10.2 ARM64 alternates

For multi-arch coverage (AD25), each tier has an ARM64 alternate :

```text
laptop    → laptop-arm     (M3 Pro)
ci        → ci-arm         (GitHub Actions ubuntu-22.04-arm64)
staging   → staging-arm    (AWS c7g.2xlarge Graviton3 — Neoverse N1)
prod      → prod-arm       (AWS c7g.4xlarge Graviton3)
```

### 10.3 Host manifest

Every bench run records a host manifest (Appendix A schema) :

```yaml
host:
  tier:          prod
  arch:          amd64
  cpu:
    model:       Intel Xeon Platinum 8375C
    sockets:     1
    cores:       16
    threads:     32
    base_ghz:    2.9
    boost_ghz:   3.5
    flags:       [avx2, avx512f, sha_ni, rdrand]
  ram:
    total_gb:    32
    speed:       DDR4-3200
  storage:
    type:        NVMe
    iops_4k_r:   400000
  network:
    bandwidth:   10 GbE
    rtt_target:  0.8 ms
  os:
    distro:      ubuntu
    version:     "22.04.4"
    kernel:      "6.5.0-26-generic"
    glibc:       "2.35"
  runtime:
    rustc:       "1.88.0"
    jemalloc:    "5.3.0"
  container:
    digest:      "sha256:abc123def456..."
    tag:         "efe-server:r3-clean@sha256:..."
  cpu_governor:  performance
  thp:           never               # transparent huge pages disabled
  numa:          interleave
  shielded_cpus: "4-15"              # cset shield with cpu pinning
  ulimit_nofile: 1048576
```

### 10.4 Tier-mismatch refusal

The bench runner verifies the runtime host matches the declared tier.
If not, the run aborts with a clear error :

```
ERROR: bench declared tier='prod' but runtime CPU is 'Apple M3 Pro'.
       Set BENCH_TIER=laptop explicitly to acknowledge the downgrade,
       OR run on a `prod`-tier host.
```

---

## 11. Statistical foundations

### 11.1 What we measure

For latency : the full distribution. Centroids (p50, p99, etc.) are
*summaries* of that distribution, not replacements for it.

For throughput : sustained req/s at a documented latency budget.
"12 000 req/s at p99 < 50 ms" is the contract.

For cost : $/req at the measured throughput AND latency. A faster
SKU at higher cost may or may not be a win — the framework reports
both.

For energy : W·h/req. RAPL-derived on bare-metal ; "N/A" on shared
hosts.

### 11.2 Sample size

Power analysis : to detect a 5 % shift in p99 with 90 % power at α=0.01
under the typical EFE latency variance (σ ≈ 0.3 × p99), the sample
size per run is :

```
n = 2 × (z_{α/2} + z_{β})² × σ² / Δ²
  = 2 × (2.576 + 1.282)² × (0.3p99)² / (0.05p99)²
  = 2 × 14.886 × 0.09 / 0.0025
  ≈ 1071 samples per run
```

EFE bench standard : minimum 5 000 samples per run, minimum 10 runs
per published number (AD12). 50 000 samples gives the bootstrap CI
(§ 11.5) a comfortable margin.

### 11.3 Percentile estimation

p99 from a sample of size 5 000 has a binomial-derived confidence
interval. The 99 % CI on p99 is approximately :

```
CI = [p99 - 2.576 × √(0.99 × 0.01 / 5000), p99 + 2.576 × √(0.99 × 0.01 / 5000)]
   = [p99 - 0.0036, p99 + 0.0036]   (in percentile-rank units)
```

At higher percentiles, the CI widens. p99.99 needs ~50 000 samples
to be tight. EFE bench standard reports the CI for each percentile.

### 11.4 Bootstrap

For non-parametric distributions (most latency distributions are
*not* normal), bootstrap is the gold standard. EFE uses **stratified
bootstrap with 10 000 resamples** for the 99 % CI on each reported
percentile.

```python
def bootstrap_ci(samples, percentile, n_boot=10_000, alpha=0.01):
    boot_percentiles = []
    n = len(samples)
    for _ in range(n_boot):
        resample = np.random.choice(samples, size=n, replace=True)
        boot_percentiles.append(np.percentile(resample, percentile))
    lo = np.percentile(boot_percentiles, 100 * alpha / 2)
    hi = np.percentile(boot_percentiles, 100 * (1 - alpha / 2))
    return lo, hi
```

### 11.5 A/B comparison

When comparing two versions :

  - **Welch's t-test** for the means (when distributions are
    approximately normal — rare for latency).
  - **Mann-Whitney U** for non-parametric distributions (most cases).
  - **Bootstrap difference of percentiles** for the p99 / p99.9
    comparison. The most useful test for EFE.

The bench framework runs all three. If they disagree (typical when
distributions are skewed), it reports all three with a discussion of
which is most appropriate for the data shape.

### 11.6 Variance reporting

Beyond the central tendency :

  - **Within-run variance** : standard deviation across the samples
    of a single run. Reported alongside each percentile.
  - **Between-run variance** : standard deviation of the per-run
    medians across the N=10 runs. Reports framework-level noise.

If between-run variance > 5 % of the central tendency, the framework
flags the bench as "noisy" and refuses to publish without an
investigation comment.

---

## 12. Coordinated omission — Gil Tene's correction

This deserves its own section because it's the most-misunderstood
honesty mechanic and the single biggest source of latency-claim
falsehoods in the industry.

### 12.1 The problem

Suppose a client wants 1 000 req/s. It schedules a request every
1 ms. If a single request takes 50 ms, the client's loop falls
behind. It cannot "catch up" by issuing 50 requests in the next
instant — the network and the server would queue them, and the next
50 ms-worth of measurements would be inflated by queueing latency.

So the client typically does one of two things :

  (a) Skip the queued requests entirely. The bench reports the 50-ms
      outlier and continues. **Coordinated omission** : the bench
      omits exactly the measurements that production users would
      experience as "request stalled in queue".

  (b) Catch up by issuing the queued requests immediately. The bench
      reports the queueing-inflated latencies as if they were
      response-time measurements. **Coordinated amplification** : the
      bench inflates the next few percentiles in a way that doesn't
      match production reality either.

Neither is honest.

### 12.2 The HdrHistogram correction

The HdrHistogram library (Gil Tene) records, for each measurement,
both :

  - `serviceTime` : the actual time between request issue and response.
  - `intendedServiceTime` : the time between when the request *should
    have been issued* (per the rate schedule) and the response.

If `serviceTime > requestInterval`, the histogram synthesizes
additional samples representing the "intended" measurements that
were never issued :

```
synthetic_samples = floor(serviceTime / requestInterval)
for k in 0 .. synthetic_samples-1 :
    record(serviceTime - k * requestInterval)
```

This recovers the *intended* latency distribution. The bench framework
records both `serviceTime` and `intendedServiceTime` and reports both.

### 12.3 EFE implementation

The `efe-bench-harness` crate (NEW per § 45 migration) wraps
HdrHistogram-rs and exposes a `RateLimitedClient` that records both
metrics. Every service-level bench uses this client.

```rust
// pseudo-code
let harness = BenchHarness::new(target_qps=1000.0, hdr=true);
let result = harness.run_for(duration=120.s, async {
    client.iterate(req).await
});
println!("p99 serviceTime         : {} ms", result.service_time.p99());
println!("p99 intendedServiceTime : {} ms", result.intended.p99());
println!("coordinated-omission delta : {:.1}%",
    100.0 * (result.intended.p99() - result.service_time.p99()) / result.service_time.p99());
```

### 12.4 What to report

The headline number for any latency claim is the
**intendedServiceTime p99** — the one customers actually experience.

The serviceTime p99 is reported as a supplementary diagnostic
("server-side response time, excluding queueing : p99 = 14 ms").

If the delta between intended and service is > 10 %, the bench
report flags this and the bench team investigates whether the server
is at capacity, the queue depth is unreasonable, or there's a long-
tail GC pause.

### 12.5 Worked example (Appendix C)

A worked numerical example is in Appendix C. The TL;DR : for a system
saturated at 90 % of capacity, coordinated-omission *uncorrected*
reports p99 = 14 ms ; *corrected*, p99 = 87 ms.

---

## 13. Anti-cherry-pick reporting policy

Reports must include the *full canonical percentile set* (AD13). Reports
cannot be filtered to highlight favourable percentiles.

### 13.1 The canonical set

```
{ p50, p75, p90, p95, p99, p99.9, p99.99 }
+ min, max, stddev
+ sample count (n)
+ within-run variance
+ between-run variance (across the 10+ runs)
+ corpus version
+ host tier
+ all environment knobs (Appendix A)
```

### 13.2 What's banned

  - "Average latency" as a headline (T4).
  - p50 as the only percentile (T8).
  - "We meet our 100 ms p99 SLO" without showing the historical
    baseline of *prior* p99 (T14).
  - Removing the top 1 % of samples to "clean up outliers" (T11).
  - Comparing this version's p99 against a competitor's p50 (T7).
  - Cross-bench geomean of dimensionally-different metrics (T12).

### 13.3 What's mandatory

  - Full distribution as JSON (Appendix F schema).
  - Bootstrap CI for every percentile.
  - The exclusion list : warm-up sample count + count of removed
    timeouts + count of removed errors + the rationale per exclusion.
  - The corpus version.
  - The bench commit SHA.
  - The host manifest.
  - The reproducer command.

A bench report that lacks any of these is rejected by the framework
and won't appear on the dashboard or in published outputs.

---

## 14. Per-surface bench design — A. Core inference (Mode 1/2/3)

### A1 — Mode 1 (latent classification, single-pass)

**SLO target** : `SLO-CLASSIFY-LATENCY` — p99 < 50 ms (current baseline
41 ms on `staging` tier at 1 000 req/s with 8 tenants, kovant-prod
corpus v0.9.0).

**Workload axes** :

  - Scope : service (gRPC `MetaSwarmService::Classify` equivalent on
    Mode 1 + anchor-aware MVP path).
  - Metric : intendedServiceTime p99 (primary), throughput, cost,
    energy.
  - Regime : cold (post-restart, empty embed-anchor cache) AND warm.
  - Tenants : 8 (standard).
  - Host : staging tier, AMD64. ARM64 + GPU as secondary matrix.

**Corpus** : `bench_corpus/replay/kovant-prod/v0.9.0/mode1/` — 100 000
anonymized classification requests, Zipf category distribution,
log-normal body sizes.

**Reproducer** :

```bash
bash scripts/bench/mode1.sh \
    --tier staging \
    --tenants 8 \
    --target-qps 1000 \
    --duration 120s \
    --corpus bench_corpus/replay/kovant-prod/v0.9.0/mode1/
```

**Operating points** : 100, 250, 500, 1 000, 2 000, 5 000 req/s.

**Report** : the per-qps Pareto curve, plus the canonical percentile set
at each qps, plus cold-vs-warm delta, plus cost ($/req) at 1 000 qps,
plus energy (W·h/req) at 1 000 qps.

### A2 — Mode 2 (LLM-routed anchors)

**SLO target** : `SLO-MODE2-LATENCY` — p99 < 300 ms (current baseline
247 ms).

**Workload axes** :

  - Scope : service (LLM router → anchor selection → Mode 1 backend).
  - Metric : intendedServiceTime p99 (includes the LLM provider's
    latency contribution — disclosed).
  - Regime : cold AND warm. (The LLM provider's cache state is
    *out of EFE's control* ; this is disclosed.)
  - Tenants : 8.
  - Host : staging tier.

**LLM provider** : configurable via `LLM_ROUTER_URL`. Bench runs
against (a) a local mock LLM router that returns canned anchor
suggestions (deterministic, fast — for regression gating), AND
(b) the real provider (slow, variable — for honest customer-facing
publication). Both runs are reported with explicit labels.

**Corpus** : `bench_corpus/replay/kovant-prod/v0.9.0/mode2/` — 50 000
anonymized routed-anchor requests.

**Honesty disclosures** :

  - The LLM provider latency contribution is reported as a separate
    column (Mode 2 latency = Mode 1 latency + LLM router overhead +
    provider RTT).
  - The provider's own SLO is documented in the report.
  - Provider rate-limits + retries are visible : retried-request
    samples are reported separately, not folded into the headline
    p99.

### A3 — Mode 3 (multi-turn dialogue)

**SLO target** : `SLO-MODE3-TURN-LATENCY` — p99 < 800 ms (current
baseline 612 ms).

**Workload axes** :

  - Scope : end-to-end (client SDK → gateway → server → Redis session
    state → response).
  - Metric : intendedServiceTime p99 *per turn*. Session-level p99
    reported separately.
  - Regime : cold AND warm. Sessions persist across warm runs ; cold
    runs invalidate Redis.
  - Tenants : 8.
  - Host : staging tier.

**Session shape** : drawn from production distribution. Mean 4.2 turns
per session, p99 = 17 turns.

**Honesty disclosures** :

  - Per-turn latency vs session-level latency : both reported.
  - Redis backpressure : if Redis is at capacity, the bench reports
    Redis latency contribution explicitly.
  - Token consumption is tracked per turn and reported as `tokens/turn`.

### A4 — Mode 1+ anchor-aware MVP

Variant of A1 with `controller.config.anchor_aware = true` enabled.
Reported as a separate bench because the optimization changes the
distribution shape and the cold/warm ratio.

---

## 15. Per-surface bench design — B. Bundle / manifest lifecycle

### B1 — Bundle cold load

**SLO** : `SLO-BUNDLE-COLD-LOAD` — p99 < 8 s.

**Procedure** :

  1. Spin a fresh `efe-server` container with a documented `BUNDLE_URL`.
  2. Time from process start to "bundle loaded, ready for traffic"
     (Prometheus `efe_bundle_loaded_total` increments).
  3. Repeat N=30.

**Variables disclosed** :

  - Bundle size (10 MB / 100 MB / 1 GB tiers).
  - Bundle source (S3 / local FS / OCI registry).
  - Bundle signature verification time (sigstore — disclosed).
  - Cold cache state (no DNS cache, no TLS session resumption).

### B2 — Bundle reload (warm cache, SIGHUP)

**SLO** : `SLO-BUNDLE-RELOAD` — p99 < 5 s (current baseline 1.7 s).

PR #12 from Phase 2 introduced the SIGHUP-driven hot-reload path. This
bench measures :

  - Time from `kill -HUP 1` to "new bundle live, old bundle drained".
  - Inflight-request impact during reload (must remain ≤ p99 + 10 %).
  - Connection-pool churn (TLS handshake count during reload : should
    be 0).

### B3 — Bundle integrity verification

**SLO** : `SLO-BUNDLE-VERIFY` — p99 < 200 ms.

  - sigstore signature verification.
  - HMAC chain verification.
  - SBOM cross-check (CycloneDX from PR #15 lint workflow).

### B4 — Bundle deploy via Helm hook

**SLO** : `SLO-HELM-HOOK-DEPLOY` — p99 < 30 s.

PR #14's `pre-upgrade-snapshot` + `post-upgrade-sighup` hooks. Bench
measures :

  - Time from `helm upgrade` to "all pods report SIGHUP-reloaded".
  - Per-pod SIGHUP delivery latency.
  - Failure modes : pod restart fallback ; partial reload.

### B5 — Bundle weight-loader sidecar polling (PR #15)

**SLO** : `SLO-SIDECAR-POLL` — depends on `bundle.pollIntervalSeconds`
(default 60 s).

Bench measures sidecar's request latency, throughput, and its
contribution to the main container's memory pressure.

---

## 16. Per-surface bench design — C. Audit + compliance

### C1 — Audit chain write latency

**SLO** : `SLO-AUDIT-WRITE` — p99 < 50 ms.

  - HMAC-SHA256 chain continuity.
  - Postgres INSERT into `audit_events`.
  - Per-tenant chain state isolation.

**Honesty** : measured under multi-tenant skew (8 tenants writing
concurrently). HMAC key rotation handled separately.

### C2 — Audit chain verify (full chain replay)

**SLO** : `SLO-AUDIT-VERIFY` — < 30 s for 1 M events / tenant.

Continuous-verify cron : per `crates/efe-active-learning/src/audit.rs`
`verify_chain` function. Bench measures :

  - Time to verify a 1 M-event chain.
  - Memory footprint during verification.
  - Failure mode : detection of injected tampering (verify-time
    accuracy = 100 %).

### C3 — Compliance control freshness (M11 / M29)

**SLO** : `SLO-COMP-FRESHNESS` — control freshness ≥ 99 % within 24 h.

Bench triggers controls + measures time-to-evidence.

### C4 — GDPR Article-30 export

**SLO** : `SLO-GDPR-ART30-EXPORT` — < 60 s per tenant.

Bench runs the export for a tenant with 1 M+ records. Reports time,
output size, completeness check.

### C5 — Data erasure complete

**SLO** : `SLO-DATA-ERASURE` — 100 % verification post-deletion.

  - `scripts/m29/verify_erasure_complete.sql` runs post-deletion.
  - Bench measures time + verifies that 0 PII rows remain.

### C6 — Conformity-assessment chain validation

**SLO** : `SLO-CONFORM-CHAIN` — chain valid 100 %.

EU AI Act conformity chain : bench validates an end-to-end chain
including risk classification, technical documentation, evidence
linkage.

---

## 17. Per-surface bench design — D. Multi-tenancy + lifecycle

### D1 — Tenant provisioning saga

**SLO** : `SLO-TENANT-PROVISION` — p95 < 30 s.

End-to-end saga : DB row insert → secrets provisioning → Redis namespace →
audit chain init → first-request readiness.

### D2 — Tenant FSM transitions

**SLO** : `SLO-TENANT-FSM` — 0 invalid transitions per 1 M.

Bench drives the tenant FSM through every transition path. Reports
the count of invalid-transition rejections (must be exactly the
expected per-test count).

### D3 — Tenant deletion (cascade)

**SLO** : `SLO-TENANT-DELETE` — p95 < 60 s including cascade.

Bench drives deletion + verifies that ALL related resources are
removed : Postgres rows, Redis keys, audit events archived, etc.

### D4 — Cross-tenant isolation

**SLO** : `SLO-TENANT-ISOLATION` — 0 cross-tenant data leakage.

Bench creates 8 tenants ; runs queries that *could* leak (e.g.
deliberately mis-formatted tenant_id) ; verifies that no query
returns rows from a non-targeted tenant.

### D5 — Tenant-aware rate limiting (D18 multi-replica)

**SLO** : `SLO-RATELIMIT-D18` — fairness ratio ≥ 0.95.

Bench fires 1 000 req/s/tenant against an 8-tenant cluster ; verifies
that each tenant gets ≥ 95 % of its budget (no tenant starves).

### D6 — Tenant LLM budget enforcement (M21)

**SLO** : `SLO-LLM-BUDGET` — budget overrun < 5 % per tenant per day.

Bench burns tenant's LLM budget ; verifies that subsequent requests
are rejected at the documented threshold.

---

## 18. Per-surface bench design — E. Streaming + events (M28)

### E1 — NATS JetStream consumer lag

**SLO** : `SLO-NATS-LAG` — p99 consumer lag < 1 s.

  - Producer rate : 10 000 events/s sustained.
  - Consumer measures the timestamp delta between event emission and
    event processing.
  - Per-stream + per-subject breakdown.

### E2 — Event loss zero in 24 h

**SLO** : `SLO-EVENT-LOSS` — 0 events lost in 24 h sustained traffic.

End-to-end : producer sequences events with monotonic IDs ; consumer
verifies no gaps over 24-hour soak.

### E3 — Streaming RPC throughput

**SLO** : `SLO-STREAM-THROUGHPUT` — 100 000 events/s sustained.

`efe-streaming` crate. Reports throughput + tail latency under load.

### E4 — Backpressure under load

**SLO** : `SLO-STREAM-BACKPRESSURE` — graceful degrade, no OOM.

Producer floods at 5× target rate. Bench verifies :

  - Consumers slow down gracefully (no crash, no OOM).
  - Acks honor backpressure semantics.
  - Producer sees increased latency, not silent drops.

---

## 19. Per-surface bench design — F. Active learning (M24)

### F1 — AL retune freshness window

**SLO** : `SLO-AL-RETUNE-FRESHNESS` — retune triggered within < 6 h of
drift detection.

Bench injects controlled drift ; measures time-to-retune.

### F2 — AL cost per labelled sample

**SLO** : `SLO-AL-COST-PER-SAMPLE` — < $0.005 (per Φ-D-OPS Master SLO #13).

End-to-end : sample selection + human label routing + LLM auto-label
+ retune. Cost computed via the FinOps integration (§ 37).

### F3 — AL automatic re-confirmation

**SLO** : `SLO-AL-AUTO-CONFIRM` — < 100 ms / sample for the auto-confirm
decision.

Bench measures per-sample auto-confirm decision latency.

### F4 — AL drift detection trigger latency

**SLO** : `SLO-AL-DRIFT-DETECT` — drift detected within 30 min of onset.

Bench injects synthetic drift ; measures detection latency.

---

## 20. Per-surface bench design — G. Vector search (M32)

### G1 — Vector index build

**SLO** : `SLO-VEC-INDEX-BUILD` — < 30 min for 10 M-vector corpus.

Build times for : 10 k, 100 k, 1 M, 10 M corpora.

### G2 — ANN query latency vs recall@k

**SLO** : `SLO-VEC-QUERY` — p99 < 50 ms @ recall@10 ≥ 0.95.

Tradeoff curve : p99 latency as a function of recall target. Plot
the Pareto front.

### G3 — Vector snapshot cron

**SLO** : `SLO-VEC-SNAPSHOT` — incremental snapshot < 5 min for daily delta.

### G4 — Replay (PITR for vectors)

**SLO** : `SLO-VEC-REPLAY` — restore a 1 M-vector corpus < 15 min.

### G5 — pgvector init container startup

**SLO** : `SLO-PGVECTOR-INIT` — init container starts < 30 s.

---

## 21. Per-surface bench design — H. Knowledge graph (M33)

### H1 — NER extractor sidecar latency

**SLO** : `SLO-NER-LATENCY` — p99 < 200 ms / document.

### H2 — KG snapshot cron

**SLO** : `SLO-KG-SNAPSHOT` — delta snapshot < 10 min ; full < 60 min.

### H3 — GraphRAG multi-hop query

**SLO** : `SLO-KG-MULTIHOP` — p99 < 500 ms @ depth = 3.

Bench varies depth (1, 2, 3, 5, 7 hops) and fan-out (1-100 neighbours).

### H4 — Graph build throughput

**SLO** : `SLO-KG-BUILD` — 10 000 nodes/s sustained.

---

## 22. Per-surface bench design — I. Multimodal (M30)

### I1 — Triton inference latency

**SLO** : `SLO-TRITON-INFER` — p99 < 100 ms for vision+text fusion.

Bench against NVIDIA Triton on the prod GPU tier (A100).

### I2 — Pre/post-processing overhead

**SLO** : `SLO-MULTIMODAL-PRE-POST` — pre+post < 20 % of total latency.

### I3 — Multi-modal embedding fusion

**SLO** : `SLO-MULTIMODAL-FUSION` — p99 < 30 ms / pair.

---

## 23. Per-surface bench design — J. Edge (M27)

### J1 — Envoy proxy overhead

**SLO** : `SLO-ENVOY-OVERHEAD` — Envoy adds < 2 ms p99 to passthrough.

### J2 — Rate limit service

**SLO** : `SLO-RATELIMIT-SERVICE` — p99 < 5 ms.

### J3 — PgBouncer pool saturation

**SLO** : `SLO-PGBOUNCER-POOL` — graceful queue at saturation, < 1 % connection refusal.

### J4 — Edge SLO under DDoS burst

**SLO** : `SLO-EDGE-DDOS` — under 10× burst, legit traffic p99 increase < 50 %.

Chaos bench (§ 31).

---

## 24. Per-surface bench design — K. Billing + FinOps (M31)

### K1 — Stripe webhook receiver latency

**SLO** : `SLO-STRIPE-WEBHOOK` — p99 < 1 s.

### K2 — Rollup CronJob throughput

**SLO** : `SLO-BILLING-ROLLUP` — daily rollup < 10 min for 1 M tenants.

### K3 — Invoicer batch generation

**SLO** : `SLO-INVOICER` — monthly invoice batch < 30 min for 10 k customers.

### K4 — Per-tenant cost accuracy

**SLO** : `SLO-COST-ACCURACY` — reported cost vs ground-truth Stripe charges drift < 0.5 %.

---

## 25. Per-surface bench design — L. Agent runtime (M25/M26)

### L1 — Agent population latency (D34 nano-agent)

**SLO** : `SLO-NANO-AGENT-POP` — `efe_nano_agent_population_total` increments < 50 ms p99.

### L2 — Consensus weight gauge stability

**SLO** : `SLO-CONSENSUS-WEIGHT` — gauge stddev / mean < 5 % over 1 h.

### L3 — Agent tools dispatch (M17 — 8 tool types)

**SLO** : `SLO-AGENT-TOOL-DISPATCH` — p99 < 100 ms / tool invocation.

### L4 — M20 BYO-LLM agent SDK overhead

**SLO** : `SLO-BYO-LLM-OVERHEAD` — overhead vs native SDK call < 10 % p99.

### L5 — Agent runtime production (M26) E2E

**SLO** : `SLO-M26-E2E` — full agent workflow (M26 assembly verification) < 5 s p99.

---

## 26. Per-surface bench design — M. Meta-swarm Phase 2 (PR05-PR15)

### M1 — MetaSwarmService::Iterate (controller fan-out)

**SLO** : `SLO-METASWARM-ITERATE` — p99 < 100 ms for 3-participant fan-out.

Cross-process gRPC ; from controller to N participants and back.

### M2 — MetaParticipantService::Step (per-participant compute)

**SLO** : `SLO-METAPARTICIPANT-STEP` — p99 < 25 ms per step.

### M3 — integration_grpc_multiprocess (worst case)

End-to-end multi-process scenario from PR #10. Measures the
controller-participant gRPC roundtrip latency under sustained iteration.

### M4 — SIGHUP reload + ReloadConfig RPC

**SLO** : `SLO-RELOAD-LATENCY` — p99 < 25 µs from SIGHUP to first
post-reload iteration (PR #12 alert threshold).

### M5 — Helm hook lifecycle

PR #14 hooks : pre-upgrade-snapshot + post-upgrade-sighup. Bench
measures hook execution time + impact on running pods.

### M6 — Bundle weight-loader sidecar polling

PR #14 placeholder. Bench measures sidecar polling overhead.

### M7 — mTLS Issuer + Certificate provisioning (PR #16)

When PR #16 lands : measure cert-manager Issuer + Certificate creation
times + binary handshake latency.

### M8 — Controller HPA stub

Phase-2 lock min=max=1. Bench verifies HPA doesn't scale during a
controlled load test.

### M9 — PrometheusRule alerts coverage

Bench validates that the 6 alerts in `prometheusrule.yaml` fire
correctly under controlled fault injection.

---

## 27. Per-surface bench design — N. SDK clients (4 langages)

For each SDK, the same set of measurements :

| Metric | Python | TypeScript | Java | Go |
|---|---|---|---|---|
| Cold (first-request) latency p99 | < 50 ms | < 100 ms | < 80 ms | < 30 ms |
| Warm latency p99 | < 5 ms | < 8 ms | < 10 ms | < 4 ms |
| Throughput per client | 200 req/s | 300 req/s | 500 req/s | 1000 req/s |
| Auth handshake (cold) | < 200 ms | < 300 ms | < 400 ms | < 150 ms |
| Retry overhead p99 | < 10 ms | < 15 ms | < 12 ms | < 8 ms |
| OTel propagation | < 1 ms | < 2 ms | < 2 ms | < 0.5 ms |

### N1-N4 — Per-language detailed reports

Per-SDK :

  - JIT warm-up phase explicit (Java HotSpot, Go method inlining).
  - Memory usage per session.
  - Connection pool behaviour under multi-client.

### N5 — SDK auth handshake (OAuth + JWT)

Includes the cold-start + JWT validation + token refresh paths.

### N6 — SDK retry (exponential backoff)

Bench injects transient failures ; measures retry success rate + tail
latency contribution.

### N7 — SDK OTel propagation overhead

Trace context propagation overhead.

---

## 28. Per-surface bench design — O. FFI / Python in-process

### O1 — PyO3 FFI roundtrip

**SLO** : `SLO-FFI-ROUNDTRIP` — p99 < 1 ms for a no-op call.

### O2 — Python in-process Mode 1 (vs gRPC out-of-process)

**SLO** : `SLO-PYO3-MODE1` — p99 within 10 % of in-server Mode 1
latency (FFI overhead is the only delta).

### O3 — NumPy array zero-copy overhead

**SLO** : `SLO-NUMPY-ZEROCOPY` — < 100 µs for a 768-dim embedding
roundtrip.

---

## 29. Per-surface bench design — P. Backup / DR

### P1 — Cold backup snapshot

**SLO** : `SLO-BACKUP-SNAPSHOT` — daily snapshot < 30 min.

### P2 — Backup restore (full)

**SLO** : `SLO-BACKUP-RESTORE-FULL` — full restore < 60 min.

### P3 — Backup restore (PITR)

**SLO** : `SLO-BACKUP-PITR` — point-in-time restore < 90 min.

### P4 — 4-eyes approval workflow

**SLO** : `SLO-BACKUP-4-EYES` — approval-to-restore < 15 min.

### P5 — Cross-env refusal

**SLO** : `SLO-BACKUP-CROSS-ENV-REFUSE` — 100 % refusal of restore
across environments.

### P6 — RPO/RTO SLO validation

**SLO** : `SLO-RPO` = 1 h, `SLO-RTO` = 30 min. Bench runs a full DR
drill quarterly (per `ops_master_slos.yaml` master SLO #17).

---

## 30. Per-surface bench design — Q. Operational

### Q1 — Master gate workflow runtime (CI)

**SLO** : `SLO-MASTER-GATE` — full run < 30 min.

CI workflow execution time. Per-job breakdown.

### Q2 — Helm install end-to-end

**SLO** : `SLO-HELM-INSTALL` — full install < 5 min.

### Q3 — Helm rolling upgrade (canary slice)

**SLO** : `SLO-HELM-UPGRADE` — rolling upgrade with canary slice
< 10 min.

### Q4 — Cold start of efe-server binary

**SLO** : `SLO-COLD-START` — < 10 s (current baseline 3.2 s).

### Q5 — Liveness/readiness probe latency

**SLO** : `SLO-PROBE-LATENCY` — probe response < 100 ms.

### Q6 — Service discovery latency

**SLO** : `SLO-SVC-DISCOVERY` — k8s-dns lookup p99 < 50 ms.

### Q7 — Connection pool warm-up

**SLO** : `SLO-POOL-WARMUP` — pool warm < 2 s post-start.

---

## 31. Cross-cutting — soak + chaos

### 31.1 Soak

Continuous load for extended duration. Catches memory leaks, slow
allocator fragmentation, GC pressure buildup, file-descriptor leaks,
cache eviction patterns invisible in short tests.

Tiers :

  - **24 h short** : every Sunday on top-5 surfaces.
  - **7 d standard** : every 4 weeks on top-3 surfaces.
  - **30 d extended** : every 6 months as a release gate.

Metrics captured during soak :

  - p99 over time (sliding 1-h window) — flat or increasing ?
  - Memory RSS over time — flat or growing ?
  - Page-cache + slab cache size — flat or growing ?
  - File-descriptor count.
  - GC pauses (per SDK, if applicable).
  - Connection-pool churn.

Soak report : the time-series plot + the regression test (linear
regression on RSS over time : slope < 1 MB / hour OR fail).

### 31.2 Chaos

Inject controlled faults during a bench :

  - **Network jitter** : `tc netem delay 50ms 30ms 25%` (50 ms jitter,
    25 % variance).
  - **Network loss** : `tc netem loss 1%`.
  - **CPU throttle** : cgroup `cpu.max 30000 100000` (30 % cap).
  - **Memory pressure** : cgroup `memory.high 80 %`.
  - **OOM kill** : `kill -9 <pid>` on a random worker.
  - **Network partition** : `iptables -A INPUT -p tcp --dport 5432 -j DROP` (DB unreachable).
  - **Slow disk** : `dm-delay` to inject I/O latency.

Each chaos scenario has a resilience SLO :

  - p99 latency under chaos < N × p99 baseline.
  - Error rate < documented threshold.
  - Recovery time (post-chaos-removal) < N seconds.

Bench reports the chaos scenarios + the resilience metrics in the
same dashboard.

---

## 32. Cross-cutting — hardware matrix

| Surface | AMD64 staging | ARM64 staging | AMD64 prod | ARM64 prod | GPU T4 | GPU A100 |
|---|---|---|---|---|---|---|
| Mode 1 | ✓ | ✓ | ✓ | ✓ | — | — |
| Mode 2 | ✓ | ✓ | ✓ | ✓ | — | — |
| Mode 3 | ✓ | ✓ | ✓ | ✓ | — | — |
| Vector | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| KG | ✓ | — | ✓ | — | ✓ | ✓ |
| Multimodal | — | — | — | — | ✓ | ✓ |
| Streaming | ✓ | ✓ | ✓ | ✓ | — | — |
| Meta-swarm | ✓ | ✓ | ✓ | ✓ | — | — |
| Agent runtime | ✓ | ✓ | ✓ | ✓ | — | — |
| SDK Py / TS / Java / Go | ✓ | ✓ | — | — | — | — |
| Bundle reload | ✓ | ✓ | ✓ | ✓ | — | — |
| Cold start | ✓ | ✓ | ✓ | ✓ | — | — |

ARM64 has structural cost+power advantage. GPU benches measure cost-
adjusted latency : "$/req at p99 = X ms" across SKUs.

---

## 33. Cross-cutting — baseline tracking + drift detection

### 33.1 Baseline structure

```yaml
# bench_baselines/v1.0.0/mode1.yaml
apiVersion: efe.bench/v1
kind:       Baseline
metadata:
  surface:   mode1
  version:   v1.0.0
  frozen_at: 2026-06-01T00:00:00Z
spec:
  host_tier: staging
  arch:      amd64
  corpus:    bench_corpus/replay/kovant-prod/v0.9.0/mode1/
  target_qps: 1000
  duration_s: 120

  measurements:
    p50:
      value:    12.4
      unit:     ms
      ci_99:    [12.1, 12.7]
    p99:
      value:    41.3
      unit:     ms
      ci_99:    [40.2, 42.5]
    p99_9:
      value:    87.2
      unit:     ms
      ci_99:    [82.1, 92.8]

  thresholds:
    warn_delta_pct: 5
    fail_delta_pct: 20
```

### 33.2 Drift detection

Nightly CI : run the bench, compare against baseline, post a comment
to a tracking issue.

  - **WARN** if any percentile drifts > 5 %.
  - **FAIL** if any percentile drifts > 20 %.

Drift triggers a bisect : find the PR responsible.

### 33.3 Baseline updates

  - **Drift accepted as new baseline** : explicit PR that updates the
    baseline YAML. Requires SRE+perf+product review.
  - **Drift rejected** : revert the responsible PR or optimize.

---

## 34. Cross-cutting — comparison policy

When comparing :

  - **vs prior EFE version** : same host tier + corpus. Report
    full table of all 7 percentiles for both versions side-by-side.
    No omissions.
  - **vs competitor** : full parameter-table disclosure. Every
    setting that diverges is flagged. Cost-adjusted comparison
    mandatory (T15).
  - **vs theoretical maximum** : marked as "ideal" not "achieved".
    Not the headline number.

---

## 35. Reporting — dashboards, schemas, percentile reporting

### 35.1 Dashboard layout

  - **Front panel** : 5 top SLOs (availability, mode-1 p99, mode-2 p99,
    mode-3 p99, cost). Each shows current value + 7-day trend +
    burn rate.
  - **Surface panels** : per-surface drill-down (full percentile set
    + Pareto curve + cold/warm + multi-tenant variants).
  - **Soak panel** : 24 h / 7 d / 30 d trend lines.
  - **Chaos panel** : resilience SLOs under each chaos scenario.
  - **Cost panel** : $/req per surface per region per SKU.
  - **Energy panel** : W·h/req per surface (bare metal only).

### 35.2 Output schemas

JSON (Appendix F) for machine consumption ; Markdown for human
consumption ; Prometheus for time-series ingestion.

### 35.3 Percentile reporting standard

Every latency number reports {p50, p75, p90, p95, p99, p99.9, p99.99}.
Mean as optional supplementary. Min/max always.

---

## 36. CI integration — master gate + nightly + soak

### 36.1 Master gate (per PR)

Triggered on every PR :

  - Micro-benches (Criterion) run quickly (< 5 min total).
  - Service-level benches with small N=3 runs (< 15 min).
  - Comparison against baseline.
  - Annotate PR with delta-from-baseline table.

If any FAIL threshold breached : block merge.

### 36.2 Nightly bench

Every night :

  - Full bench suite on `staging` tier.
  - Drift detection vs baselines.
  - Auto-issue if drift detected.

### 36.3 Soak

  - 24 h short soak weekly.
  - 7 d standard soak monthly.
  - 30 d extended soak before each major release.

### 36.4 Release gate

Before `release-v1-ga.yml` runs the full bench matrix on `prod` tier.
Public bench publication blocked until full matrix passes.

---

## 37. Cost integration — $/req with FinOps

### 37.1 Pricing snapshot

Bench manifests record the cloud SKU + region + pricing snapshot at
bench time :

```yaml
pricing:
  sku:          c6i.2xlarge
  region:       eu-west-3
  on_demand:    $0.34 / h
  spot:         $0.10 / h (variable)
  snapshot_at:  2026-06-01T00:00:00Z
```

### 37.2 Cost computation

`$/req` = (SKU $/h) / (req/h) at the measured throughput.

For mixed SKU stacks (e.g. CPU + GPU + S3) : sum across, report
breakdown.

### 37.3 Tenant-attributed cost

Cross-checked against the per-tenant cost from M31 billing :

  - Bench cost / req at known throughput.
  - Production cost / req (from billing) at production throughput.
  - Drift = (bench - prod) / prod.

---

## 38. Energy integration — W·h/req with RAPL

### 38.1 RAPL on Linux

Intel RAPL exposes per-package + per-DRAM energy counters via
`/sys/class/powercap/intel-rapl:*/energy_uj`. Sample at 100 ms.

```python
def rapl_energy_uj() -> int:
    """Return cumulative RAPL energy in microjoules."""
    total = 0
    for p in pathlib.Path("/sys/class/powercap/intel-rapl:0").glob("energy_uj"):
        total += int(p.read_text().strip())
    return total
```

### 38.2 W·h/req computation

```
energy_per_req_J = (rapl_after - rapl_before) / 1e6 / total_requests
energy_per_req_Wh = energy_per_req_J / 3600
```

Typical EFE Mode 1 measurement : 0.012 W·h/req on `staging-bm`
(Skylake 2-socket).

### 38.3 ARM64 + AMD

ARM64 RAPL : Linux supports `amd_energy` (AMD) + `arm_pcc_perf` (ARM
servers). The framework abstracts via `rapl_or_skip()` — falls back
to "N/A : energy meter unavailable" on virtualized or unsupported
hosts.

### 38.4 Co-reporting

Every public bench reports : (latency p99) + (throughput) + ($/req) +
(W·h/req).

---

## 39. Reproducibility — one-command reproducer

```bash
# Reproduces the Mode-1 latency baseline.
bash scripts/bench/repro.sh \
    --surface mode1 \
    --tier staging \
    --target-qps 1000 \
    --duration 120s \
    --baseline bench_baselines/v1.0.0/mode1.yaml
```

The script :

  1. Verifies host tier matches (cset shield + cpu governor).
  2. Pulls the pinned corpus version.
  3. Pulls the pinned container digest.
  4. Runs the bench harness.
  5. Compares against baseline.
  6. Emits the report (JSON + Markdown + Prom metrics).

Output : a single `bench_run_<surface>_<timestamp>/` directory with :

  - `result.json` (machine-readable)
  - `result.md` (human-readable)
  - `result.hdr` (HdrHistogram binary)
  - `manifest.yaml` (host + corpus + container)
  - `repro.sh` (the exact command + env to re-run)

---

## 40. Public bench publication policy

### 40.1 Pre-publication checklist

Before any EFE bench number is published outside the team :

  - [ ] Corpus version + hash recorded.
  - [ ] Host manifest complete (Appendix A).
  - [ ] Container digest pinned.
  - [ ] Full percentile distribution available (not just headline).
  - [ ] Bootstrap CI computed.
  - [ ] Coordinated-omission corrected.
  - [ ] Multi-tenant variant measured (or "single-tenant" explicit).
  - [ ] Cold AND warm variants (if cacheable).
  - [ ] Cost ($/req) + Energy (W·h/req or "N/A").
  - [ ] Negative space listed (what's NOT measured).
  - [ ] Reproducer script in repo + verified runnable.
  - [ ] Exclusions list (warm-up, timeouts, errors) populated.
  - [ ] Comparison fairness verified (if comparing).
  - [ ] Independent re-run by a non-perf team member passed.

### 40.2 Comparison-bench publication

Additional requirements when comparing :

  - [ ] Parameter table : every config differing between configurations
        listed.
  - [ ] Cost-adjusted view (the "per-dollar latency" metric).
  - [ ] Hardware match (same SKU on both sides) OR explicit downgrade
        flag.
  - [ ] Feature-flag identity verified.

### 40.3 Marketing site policy

The marketing-site bench numbers MUST link to the underlying
`bench_run_*` directory in the repo. Numbers that lack a backing
reproducer are removed.

---

## 41. Acceptance criteria (AC1–AC30)

| AC   | Statement                                                                                                                                     | Status   |
|------|-----------------------------------------------------------------------------------------------------------------------------------------------|----------|
| AC1  | Every public bench has full environment disclosure (CPU, RAM, kernel, glibc, container digest).                                                 | gate     |
| AC2  | Every public bench pins corpus version + content hash.                                                                                          | gate     |
| AC3  | {p50, p75, p90, p95, p99, p99.9, p99.99} reported for every latency bench.                                                                       | gate     |
| AC4  | Bootstrap 99 % CI computed for every reported number.                                                                                            | gate     |
| AC5  | Throughput-latency Pareto reported as a function (≥ 5 operating points).                                                                          | gate     |
| AC6  | Multi-tenant interference baseline measured (8-tenant default).                                                                                  | gate     |
| AC7  | Cold start measured (binary + container + JIT).                                                                                                  | gate     |
| AC8  | Hot/cold cache ratio reported for cacheable surfaces.                                                                                            | gate     |
| AC9  | SLO targets reproduced from baseline run with identical setup.                                                                                   | gate     |
| AC10 | One-command reproducer per bench in repo.                                                                                                        | gate     |
| AC11 | Output schema versioned + machine-readable (Appendix F).                                                                                          | gate     |
| AC12 | Drift detection runs nightly + gates PRs.                                                                                                        | gate     |
| AC13 | Publications disclose what was excluded (warm-up, outliers, retries).                                                                            | gate     |
| AC14 | Publications never cherry-pick percentiles ; full set or none.                                                                                   | gate     |
| AC15 | ≥ 24 h soak weekly for top-5 surfaces ; ≥ 7 d monthly for top-3.                                                                                  | gate     |
| AC16 | Competitor comparisons require parameter-table disclosure.                                                                                       | gate     |
| AC17 | Energy (W·h/req) on bare-metal benches (or "N/A : virtualized").                                                                                  | informational |
| AC18 | Cost ($/req) with live pricing snapshot + region disclosed.                                                                                       | gate     |
| AC19 | Failure-mode benches include retries, timeouts, partial failures.                                                                                | gate     |
| AC20 | Service-level benches include 8+ tenant interference scenarios.                                                                                  | gate     |
| AC21 | Hardware matrix : ≥ 3 tiers benched for top-5 paths.                                                                                              | gate     |
| AC22 | Coordinated-omission correction (HdrHistogram or equivalent) used.                                                                               | gate     |
| AC23 | Bench-as-code : every bench in repo, no manual runs.                                                                                              | gate     |
| AC24 | Reviewer checklist on bench PRs enforces all gates.                                                                                              | gate     |
| AC25 | Bench documentation auto-generated from corpus + manifest metadata.                                                                              | gate     |
| AC26 | Statistical tests for A/B comparisons : pre-registered + reported.                                                                               | gate     |
| AC27 | Sample size determination : power analysis + minimum N=10.                                                                                       | gate     |
| AC28 | CI gate : regression > 5 % blocks merge ; > 20 % blocks even with override.                                                                       | gate     |
| AC29 | Bench manifest reproducibility hashes published + verifiable.                                                                                    | gate     |
| AC30 | Honest absence : surfaces NOT benchmarked listed explicitly in every publication.                                                                 | gate     |

---

## 42. Tests strategy — the bench framework tests itself

The framework's *own* code is tested :

  - **Unit tests** of HdrHistogram wrappers ; coordinated-omission
    correction reproduces the canonical test case from Gil Tene's
    published examples.
  - **Bootstrap test** : reproduces a known textbook result (e.g. the
    bootstrap CI on the bench's own internal timer).
  - **Statistical tests test** : Mann-Whitney + Welch implementations
    reproduce SciPy reference values within 1e-9.
  - **Corpus integrity test** : every published corpus hash reproduces.
  - **Manifest schema test** : every published manifest validates
    against the v1 schema.
  - **Sanity meta-bench** : the bench harness's own overhead is
    measured. Must be < 1 % of measured latency (AD22).
  - **Drift gate test** : a synthetic drift > 5 % triggers WARN ;
    > 20 % triggers FAIL.
  - **Cost computation test** : reference SKU pricing + req/h →
    cost. Reproduces a textbook example.

---

## 43. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Tooling drift : HdrHistogram-rs upstream breaking change | Medium | Medium | Pin version + meta-bench self-test catches |
| Corpus drift : v1.2.0 corpus contains data that should be excluded | Low | High | Per-corpus disclosure + co-design with success / support |
| Host drift : staging tier hw changes silently | Medium | High | Manifest fail-fast on mismatch |
| Goodhart's law on our own bench corpus | Medium | High | Quarterly external re-run + corpus rotation |
| Energy meter access drift on cloud hosts | Medium | Low | Fallback to "N/A : virtualized" |
| Cost API rate-limit | Low | Low | Daily snapshot cached |
| Bench framework's own overhead drift > 1 % | Low | Medium | Meta-bench in CI |
| False FAIL : drift gate triggers on noisy bench | Medium | Medium | Multi-run consensus (median of N=10 runs) |
| Stale baseline : optimization landed but baseline not updated | Medium | Low | Quarterly baseline refresh + PR-time delta |
| Public corpus exposure of accidentally retained PII | Low | High | M29 PII scrubbing + corpus-publish-time scan |

---

## 44. Rollback strategy

If the framework lands and breaks CI on day 1 :

  1. Revert the framework PR.
  2. Existing benches (legacy tier) keep running on the old infra
     (compat shim).
  3. Per-bench migration is rolled back individually.
  4. Drift gate temporarily disabled until framework is re-stabilized.

---

## 45. Migration path from current bench infra

### 45.1 Phase 1 — Foundation (6 weeks)

  1. Land the `efe-bench-harness` crate (Rust).
  2. Land HdrHistogram-rs wrapper.
  3. Land bootstrap-CI library.
  4. Land manifest schema + validator.
  5. Land 4 host-tier definitions.
  6. Update `scripts/bench_runner.sh` to call the new harness.

### 45.2 Phase 2 — Per-surface migration (12 weeks)

Per the per-surface designs (§ 14-§ 30) :

  - Week 1-2 : Mode 1.
  - Week 3-4 : Mode 2, Mode 3.
  - Week 5-6 : Vector, KG.
  - Week 7-8 : Streaming, Active Learning.
  - Week 9-10 : Multimodal, Edge, Billing.
  - Week 11-12 : Agent runtime, FFI, Backup/DR, Operational.

Each surface migration : write the new bench harness invocation,
run it against the v0.9.0 baseline, verify reproducibility, freeze a
v1.0.0 baseline.

### 45.3 Phase 3 — Public publication (4 weeks)

  - Publish v1 marketing-bench numbers from the new framework.
  - Sunset the legacy bench infra (compat shim removed).

Total migration : ~ 22 weeks (5 months). Aligns with Q3 2026.

---

## 46. Sequence diagrams

### 46.1 PR-time bench gate flow

```text
PR author          GitHub Actions          bench-harness          baseline
   │                       │                       │                  │
   ├── push commit ──>     │                       │                  │
   │                       ├─ trigger bench job    │                  │
   │                       ├─ checkout repo        │                  │
   │                       ├─ pin container        │                  │
   │                       ├─ pin corpus version   │                  │
   │                       ├─ invoke harness ──>   │                  │
   │                       │                       ├─ run N=3 runs    │
   │                       │                       │  with HdrHist    │
   │                       │                       │  + corp-omiss    │
   │                       │                       │  correction      │
   │                       │                       ├─ compute boot CI │
   │                       │                       ├─ compare vs ──>  │
   │                       │                       │                  ├─ verdict
   │                       │                       │  <── diff──────  │
   │                       │ <── result ──         │                  │
   │                       ├─ post PR comment      │                  │
   │ <── comment ──        │                       │                  │
   │     "Mode-1 p99       │                       │                  │
   │      drift: +2.1%"    │                       │                  │
   │                       │                       │                  │
```

### 46.2 Honest-claim verification flow

```text
External auditor       Public repo         bench-harness        host
   │                         │                       │              │
   ├── git clone ──>         │                       │              │
   │                         │                       │              │
   ├── docker pull           │                       │              │
   │  @sha256:abc... ────>   │                       │              │
   │                         │                       │              │
   ├── bench/repro.sh ─────────────────────>         │              │
   │     --surface mode1                             │              │
   │                                                  ├─ verify host │
   │                                                  ├─ pull corpus │
   │                                                  ├─ run bench   │
   │                                                  │  on host────>│
   │                                                  │              │
   │                                                  │  <── samples │
   │                                                  ├─ compute pct │
   │                                                  ├─ verify vs   │
   │                                                  │  baseline    │
   │ <── result.json ────────────────────────         │              │
   │                                                  │              │
   ├── compare to ───────────────────────>             │              │
   │     marketing claim                                              │
   │  result matches → claim verified                                  │
   │  result diverges → claim rejected ────>                          │
   │                       (auditor publishes counter-claim)         │
```

---

## 47. Reviewer checklist

  - [ ] AD coverage : every AD has a corresponding implementation
        stanza, framework crate function, or per-surface bench design
        that implements it.
  - [ ] AC coverage : every AC has a CI gate, a workflow assertion,
        or an artifact-presence check.
  - [ ] Threat coverage : every T1-T18 has a named mitigation.
  - [ ] Corpus designed with success / support input (Goodhart's law).
  - [ ] External team member ran the bench from cold state and got
        within CI of the headline number.
  - [ ] Negative space list non-empty.
  - [ ] Reproducer command works from a fresh checkout.
  - [ ] Manifest schema validates.
  - [ ] Meta-bench overhead < 1 % verified.
  - [ ] Cost ($/req) cross-checked against M31 billing.
  - [ ] Energy (W·h/req) reported on at least one bare-metal tier
        OR "N/A : virtualized" disclosed.
  - [ ] Drift gate triggers on synthetic 5 % shift.
  - [ ] Drift gate triggers on synthetic 20 % shift with hard-fail.

---

## 48. Appendix A — environment disclosure manifest schema

```yaml
# bench_run_<surface>_<timestamp>/manifest.yaml
apiVersion: efe.bench/v1
kind:       RunManifest
metadata:
  surface:    mode1
  run_id:     <uuid4>
  ran_at:     <ISO-8601 UTC>
spec:
  host:
    tier:     prod                 # AD7
    arch:     amd64                # AD25
    cpu:
      model:    Intel Xeon Platinum 8375C
      sockets:  1
      cores:    16
      threads:  32
      base_ghz: 2.9
      boost_ghz: 3.5
      flags:    [avx2, avx512f, sha_ni, rdrand]
    ram:
      total_gb: 32
      speed:    DDR4-3200
    storage:
      type:        NVMe
      iops_4k_r:   400000
      iops_4k_w:   200000
    network:
      bandwidth:   10 GbE
      rtt_target:  0.8 ms
    os:
      distro:      ubuntu
      version:     "22.04.4"
      kernel:      "6.5.0-26-generic"
      glibc:       "2.35"
      libcrypto:   "openssl 3.0.13"
    runtime:
      rustc:       "1.88.0"
      jemalloc:    "5.3.0"
      libtcmalloc: ""               # not used
    container:
      digest:      sha256:<digest>
      tag:         "efe-server:r3-clean@sha256:<digest>"
      sbom_digest: sha256:<sbom>
    cpu_governor: performance
    thp:          never
    numa:         interleave
    shielded_cpus: "4-15"
    ulimit_nofile: 1048576

  corpus:
    name:        bench_corpus/replay/kovant-prod/v0.9.0/mode1/
    version:     v0.9.0
    content_hash: sha256:<hash>
    n_requests:   100000

  feature_flags:
    EFE_AUTH_ENABLED:        true
    EFE_AUDIT_ENABLED:       true
    EFE_RATE_LIMIT_ENABLED:  true
    EFE_AGENT_REST_ENABLED:  false

  bench:
    framework_commit: <git SHA>
    framework_version: v1.0.0
    target_qps: 1000
    duration_s: 120
    warmup_s:   30                   # disclosed in AC13 exclusions
    n_runs:     10                   # AD12

  pricing:
    sku:          c6i.2xlarge
    region:       eu-west-3
    on_demand:    0.34               # USD / h
    snapshot_at:  <ISO-8601 UTC>

  rapl:
    available:    true
    pkg_path:     /sys/class/powercap/intel-rapl:0
    dram_path:    /sys/class/powercap/intel-rapl:0:0
```

---

## 49. Appendix B — statistical formulas

### B.1 Bootstrap 99 % CI on a percentile

```
Input  : samples = [x_1, ..., x_n], percentile p (e.g. p99)
Output : CI (lo, hi)

for i in 1..10000:
    boot_i = random_sample_with_replacement(samples, size=n)
    boot_percentiles[i] = percentile(boot_i, p)

CI = (percentile(boot_percentiles, 0.5),
      percentile(boot_percentiles, 99.5))
```

### B.2 Welch's t-test for two means

```
t = (mean_a - mean_b) / sqrt(var_a/n_a + var_b/n_b)
df = (var_a/n_a + var_b/n_b)^2 / (
       (var_a/n_a)^2 / (n_a - 1) + (var_b/n_b)^2 / (n_b - 1))
p = 2 * (1 - student_t.cdf(|t|, df))
```

### B.3 Mann-Whitney U (rank-sum) for two non-parametric distributions

```
U = R_a - n_a * (n_a + 1) / 2
z = (U - mean_U) / sigma_U  with continuity correction
p = 2 * (1 - normal.cdf(|z|))
```

### B.4 Power analysis (required sample size)

```
n = 2 * (z_{α/2} + z_{β})^2 * σ^2 / Δ^2

where :
  z_{α/2} = 2.576    (α = 0.01, two-tailed)
  z_{β}   = 1.282    (power = 0.90)
  σ       = stddev estimate
  Δ       = minimum detectable difference
```

For EFE Mode 1 (σ ≈ 0.3 × p99, Δ = 0.05 × p99) : n ≈ 1071. Standard
is 5 000.

---

## 50. Appendix C — coordinated-omission worked example

System under bench :

  - Target rate : 1 000 req/s (interval = 1 ms).
  - Server is at 90 % of capacity.
  - Single GC pause of 50 ms occurs at t=10 s.

Uncorrected client behaviour (omission) :

  - 49 requests not issued during the pause.
  - First post-pause request : recorded at ~ 1 ms (low because the
    server's queue is empty by t=10.05 s).
  - p99 uncorrected : ~ 14 ms (because most requests are sub-ms,
    the lone 50 ms outlier registers at p99.999 only).

Corrected (HdrHistogram with intendedServiceTime) :

  - The 49 "lost" intended requests are synthesized as samples with
    latencies of (50, 49, 48, ..., 2, 1) ms — each represents the
    "intended" service time if the request had been issued on
    schedule.
  - p99 corrected : ~ 87 ms (because of the 49 high-latency intended
    samples).

The correction surfaces the real customer experience : in production,
those queued requests *would* have been issued by other clients, and
they *would* have been delayed by the GC pause. The uncorrected p99
is a lie ; the corrected p99 is honest.

---

## 51. Appendix D — per-surface SLO target catalog

Single source-of-truth catalog of all 60+ SLO targets across the
project. Cross-referenced from § 14-§ 30.

Master 18 SLOs (`ops_master_slos.yaml` v0.9.0) are denoted `★`.
Surface-specific SLOs derived from per-section designs are denoted
without marker.

```text
Mode 1 :
  ★ SLO-CLASSIFY-LATENCY        p99 < 50 ms
    SLO-MODE1-COLD-LATENCY      cold p99 < 200 ms
    SLO-MODE1-WARM-RATIO        warm/cold ≥ 4×
    SLO-MODE1-THROUGHPUT        2 500 req/s sustained @ p99 < 100 ms

Mode 2 :
  ★ SLO-MODE2-LATENCY           p99 < 300 ms
    SLO-MODE2-ROUTER-OVERHEAD   router < 20 ms p99
    SLO-MODE2-PROVIDER-RTT      (informational, provider-dependent)

Mode 3 :
  ★ SLO-MODE3-TURN-LATENCY      p99 < 800 ms
    SLO-MODE3-SESSION-LATENCY   session p99 < 5 s
    SLO-MODE3-REDIS-OVERHEAD    Redis state < 10 ms p99

Bundle :
  ★ SLO-BUNDLE-RELOAD           p99 < 5 s
    SLO-BUNDLE-COLD-LOAD        p99 < 8 s
    SLO-BUNDLE-VERIFY           p99 < 200 ms
    SLO-HELM-HOOK-DEPLOY        p99 < 30 s
    SLO-SIDECAR-POLL            < 100 ms p99 / poll

Audit + compliance :
    SLO-AUDIT-WRITE             p99 < 50 ms
  ★ SLO-COMP-FRESHNESS          ≥ 99 % within 24 h
    SLO-AUDIT-VERIFY            < 30 s / 1 M events / tenant
    SLO-GDPR-ART30-EXPORT       < 60 s / tenant
    SLO-DATA-ERASURE            100 % verification
    SLO-CONFORM-CHAIN           100 % valid

Multi-tenancy + lifecycle :
  ★ SLO-TENANT-PROVISION        p95 < 30 s
  ★ SLO-TENANT-FSM              0 invalid / 1 M
    SLO-TENANT-DELETE           p95 < 60 s
    SLO-TENANT-ISOLATION        0 cross-tenant leakage
    SLO-RATELIMIT-D18           fairness ≥ 0.95
    SLO-LLM-BUDGET              overrun < 5 % / tenant / day

Streaming :
  ★ SLO-NATS-LAG                p99 < 1 s
  ★ SLO-EVENT-LOSS              0 events / 24 h
    SLO-STREAM-THROUGHPUT       100 k events/s sustained
    SLO-STREAM-BACKPRESSURE     graceful degrade

Active learning :
  ★ SLO-AL-RETUNE-FRESHNESS     < 6 h drift-to-retune
  ★ SLO-AL-COST-PER-SAMPLE      < $0.005
    SLO-AL-AUTO-CONFIRM         < 100 ms / sample
    SLO-AL-DRIFT-DETECT         < 30 min onset-to-detect

Vector search :
    SLO-VEC-INDEX-BUILD         < 30 min / 10 M-vector
    SLO-VEC-QUERY               p99 < 50 ms @ recall@10 ≥ 0.95
    SLO-VEC-SNAPSHOT            < 5 min incremental
    SLO-VEC-REPLAY              < 15 min / 1 M-vector
    SLO-PGVECTOR-INIT           < 30 s

Knowledge graph :
    SLO-NER-LATENCY             p99 < 200 ms / document
    SLO-KG-SNAPSHOT-DELTA       < 10 min
    SLO-KG-SNAPSHOT-FULL        < 60 min
    SLO-KG-MULTIHOP             p99 < 500 ms @ depth=3
    SLO-KG-BUILD                10 k nodes/s sustained

Multimodal :
    SLO-TRITON-INFER            p99 < 100 ms (A100)
    SLO-MULTIMODAL-PRE-POST     pre+post < 20 % of total
    SLO-MULTIMODAL-FUSION       p99 < 30 ms / pair

Edge :
    SLO-ENVOY-OVERHEAD          < 2 ms p99 passthrough
    SLO-RATELIMIT-SERVICE       p99 < 5 ms
    SLO-PGBOUNCER-POOL          < 1 % connection refusal
    SLO-EDGE-DDOS               legit p99 < 1.5× under 10× burst

Billing + FinOps :
    SLO-STRIPE-WEBHOOK          p99 < 1 s
    SLO-BILLING-ROLLUP          < 10 min / 1 M tenants
    SLO-INVOICER                < 30 min / 10 k customers
    SLO-COST-ACCURACY           drift < 0.5 %

Agent runtime :
    SLO-NANO-AGENT-POP          < 50 ms p99
    SLO-CONSENSUS-WEIGHT        stddev/mean < 5 %
    SLO-AGENT-TOOL-DISPATCH     p99 < 100 ms / invocation
    SLO-BYO-LLM-OVERHEAD        < 10 % p99 vs native
    SLO-M26-E2E                 < 5 s p99

Meta-swarm Phase 2 :
    SLO-METASWARM-ITERATE       p99 < 100 ms / 3-participant
    SLO-METAPARTICIPANT-STEP    p99 < 25 ms / step
    SLO-RELOAD-LATENCY          p99 < 25 µs SIGHUP-to-first-iter

SDK clients :
  ★ SLO-SDK-LATENCY-PY          p99 < 5 ms
    SLO-SDK-LATENCY-TS          p99 < 8 ms
    SLO-SDK-LATENCY-JAVA        p99 < 10 ms
    SLO-SDK-LATENCY-GO          p99 < 4 ms
    SLO-SDK-COLD-LATENCY-PY     p99 < 50 ms

FFI :
    SLO-FFI-ROUNDTRIP           p99 < 1 ms (no-op)
    SLO-PYO3-MODE1              within 10 % of native
    SLO-NUMPY-ZEROCOPY          < 100 µs / 768-dim

Backup / DR :
  ★ SLO-MTTR                    < 30 min (Sev1+Sev2)
  ★ SLO-DR-DRILL                quarterly with RTO < 30 min
    SLO-BACKUP-SNAPSHOT         < 30 min daily
    SLO-BACKUP-RESTORE-FULL     < 60 min
    SLO-BACKUP-PITR             < 90 min
    SLO-BACKUP-4-EYES           approval < 15 min
    SLO-BACKUP-CROSS-ENV        100 % refusal

Operational :
  ★ SLO-OPS-AVAILABILITY        ≥ 99.95 % monthly
    SLO-MASTER-GATE             < 30 min CI runtime
    SLO-HELM-INSTALL            < 5 min
    SLO-HELM-UPGRADE            < 10 min
    SLO-COLD-START              < 10 s
    SLO-PROBE-LATENCY           < 100 ms
    SLO-SVC-DISCOVERY           p99 < 50 ms
    SLO-POOL-WARMUP             < 2 s

Cross-cutting :
  ★ SLO-CANARY-ROLLBACK         < 5 % rollback-rate per release
  ★ SLO-CROSS-CLOUD-VARIANCE    bounded ≤ documented threshold
  ★ SLO-POSTMORTEM-48H          ≤ 48 h Sev1
  ★ SLO-A1-P1-BENCH-GATE        Sprint A1 P1 multi-swarm bench gate
```

= 60 SLOs across 18 surfaces.  18 master SLOs (`★`) consolidated in
`ops_master_slos.yaml v0.9.0`.

---

## 52. Appendix E — corpus versioning policy

  - **Major bump (X)** : breaking distribution change (e.g. corpus
    structure changes ; non-comparable across versions).
  - **Minor bump (Y)** : additive (new scenarios added ; old still
    runnable).
  - **Patch bump (Z)** : bugfix (e.g. PII discovered and scrubbed).
    Baseline MUST be re-frozen.

Corpus deprecation : v0.x.x corpora retain a 12-month grace period
after a major bump. After that, the framework refuses to run
against deprecated corpora.

---

## 53. Appendix F — bench-output JSON schema v1

```yaml
$schema:    https://json-schema.org/draft/2020-12/schema
$id:        https://efe.example.com/schemas/bench-result-v1.json
type:       object
required:   [apiVersion, kind, metadata, spec, measurements]
properties:
  apiVersion: {type: string, const: "efe.bench/v1"}
  kind:       {type: string, const: "BenchResult"}
  metadata:
    type: object
    required: [surface, run_id, ran_at, framework_version, framework_commit]
    properties:
      surface:           {type: string}
      run_id:            {type: string, format: uuid}
      ran_at:            {type: string, format: date-time}
      framework_version: {type: string}
      framework_commit:  {type: string}
  spec:
    type: object
    required: [host, corpus, feature_flags, bench]
    properties:
      host:          {$ref: "host-manifest-v1"}
      corpus:        {$ref: "corpus-manifest-v1"}
      feature_flags: {type: object}
      bench:         {$ref: "bench-params-v1"}
      pricing:       {$ref: "pricing-v1"}
      rapl:          {$ref: "rapl-v1"}
  measurements:
    type: object
    required: [latency, throughput, exclusions]
    properties:
      latency:
        type: object
        required: [p50, p75, p90, p95, p99, p99_9, p99_99, min, max, stddev]
        properties:
          p50:    {$ref: "metric-with-ci"}
          p75:    {$ref: "metric-with-ci"}
          p90:    {$ref: "metric-with-ci"}
          p95:    {$ref: "metric-with-ci"}
          p99:    {$ref: "metric-with-ci"}
          p99_9:  {$ref: "metric-with-ci"}
          p99_99: {$ref: "metric-with-ci"}
          min:    {type: number}
          max:    {type: number}
          stddev: {type: number}
          unit:   {type: string, enum: [ns, us, ms, s]}
      throughput:
        type: object
        required: [sustained_qps, latency_budget_p99]
        properties:
          sustained_qps:        {type: number}
          latency_budget_p99:   {type: number}
      cost_per_req:
        type: object
        required: [value, currency, region]
        properties:
          value:    {type: number}
          currency: {type: string}
          region:   {type: string}
      energy_per_req:
        type: object
        oneOf:
          - {required: [value, unit]}
          - {required: [na_reason]}
        properties:
          value:     {type: number}
          unit:      {type: string, const: Wh}
          na_reason: {type: string}
      exclusions:
        type: array
        items:
          type: object
          required: [kind, count, rationale]
          properties:
            kind:      {type: string, enum: [warmup, timeout, retry, error]}
            count:     {type: integer}
            rationale: {type: string}
      negative_space:
        type: array
        description: "Surfaces NOT measured ; mandatory if non-empty corpus disclosure exclusions exist."
        items: {type: string}

definitions:
  metric-with-ci:
    type: object
    required: [value, ci_99_lo, ci_99_hi]
    properties:
      value:     {type: number}
      ci_99_lo:  {type: number}
      ci_99_hi:  {type: number}
```

---

## 54. Appendix G — anti-patterns gallery (annotated)

Each anti-pattern below is a real (or canonical) example that this
framework rejects.

### G.1 The "average response time" headline

> "Our average response time is 17 ms."

Hides : the bimodal distribution where 50 % of requests are 5 ms and
50 % are 29 ms. The framework reports the full distribution.

### G.2 The cherry-picked window

> "Our p99 over the last hour was 14 ms."

Hides : the *busy hour* p99 was 230 ms. The framework reports the
soak's 24 h sliding p99.

### G.3 The downplayed cost

> "We're 2× faster than the competitor."

Hides : "we run on an SKU 5× more expensive". The framework reports
the cost-adjusted ratio.

### G.4 The single-tenant idyll

> "p99 = 8 ms."

Hides : that was 1 tenant, not 100. Production has 100. The framework
reports multi-tenant variants.

### G.5 The marketing geomean

> "Across 12 benches we are 1.4× faster."

Hides : we're 0.7× on the 3 most-customer-relevant benches and 4×
on the 3 we optimized for. The framework rejects geomeans across
dimensionally-mixed metrics.

### G.6 The hidden warm-up

> "p99 of our 1 hour run : 14 ms."

Hides : the first 5 minutes (warm-up) were excluded. AC13 disclosure
required.

### G.7 The pre-registered statistical lie

> "p < 0.05 confirmed our optimization works."

Hides : we ran 20 different tests, kept the one with p < 0.05
(garden of forking paths). AC26 pre-registration + correction for
multiple comparisons.

### G.8 The proxy bench

> "We benchmark Postgres and Postgres is fast."

Hides : the application sitting on top of Postgres has serialization
overhead. Framework benches EFE-using-Postgres, not just Postgres.

### G.9 The cold-cache magic

> "Our cache hit rate is 99 % so the p99 latency is dominated by cold misses."

Hides : the cold-miss percentile is 800 ms. The framework reports
cold AND warm separately.

### G.10 The SLO-aspirational publish

> "We meet our 100 ms p99 SLO."

Hides : the SLO was set generously based on the team knowing their
measurement is ~60 ms. The framework reports the *baseline* alongside.

---

## End of architecture document

This document is the EFE Honest Bench Framework reference. Migration
to this framework runs through Q3 2026 per § 45. All ACs (§ 41)
become CI gates as their corresponding code lands per the migration
schedule.
