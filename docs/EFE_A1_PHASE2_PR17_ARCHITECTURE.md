# EFE-A1 Phase 2 — PR #17 : 14 Prometheus metrics

| Field | Value |
|---|---|
| **PR** | #17 (Sprint A1 Phase 2 / P2.3 W5-6 — spans 2 sprints) |
| **Branch** | `efe-a1/p2-pr17-prometheus-metrics` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× SRE + 2× meta-swarm + 1× observability + 1× platform-deploy + 1× compliance |
| **Depends on** | PR #5 (proto), PR #6 (scaffold), PR #8 (Iterate wire), PR #9 (Step wire), PR #10 (multiprocess), PR #11 (efe-server flag plumbing), PR #12 (SIGHUP + reload metrics — already 4 series), PR #13–#15 (Helm + lint workflow), PR #16 (mTLS + 5 TLS series) |
| **Blocks** | PR #18 (OpenTelemetry — depends on metric span correlation), PR #19 (Grafana dashboards — consumes these 14 series), PR #22 (4 SLOs — derived from these series), PR #23 (chaos benches — use these for steady-state assertions), GA v1.0 (no operability without metrics) |
| **L2 gate** | G14 (production-grade observability surface : 14 metrics + 5 new alerts + cardinality budget enforced + stability contract documented) |
| **Threat IDs in scope** | T46-T55 (10 new threats : metric-name leak, cardinality explosion, stale metric, PII in label, label-value DoS, scrape-timeout cascade, gauge race, histogram-bucket-bias, counter-rollover, RBAC-bypass via `/metrics`) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS2 + § WS4 + § 11 |
| **CVSS bandwidth** | All ≤ SEV-2 (no PII leakage, no remote code execution, no auth bypass) |
| **Helm chart version bump** | `efe-meta-swarm` Chart.yaml.version 0.3.0 → **0.4.0** (minor — new alert rules) ; appVersion 0.4.0 → **0.5.0** (binary new metrics module) |
| **Total LOC budget** | Rust ~3 200 ; Helm ~600 ; tests ~1 400 ; doc ~2 500 → **~7 700 LOC** |
| **CI cost delta** | + 6 s wall-time for the new helm-unittest suite ; + 12 s for the metrics integration test ; total + 18 s |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals
   2. Pre-flight — existing metrics inventory
   3. Branch + commit plan (10 commits across 2 sprints)
   4. Architectural decisions (AD1–AD30)
   5. The 14 metrics — name, type, labels, buckets, semantics
   6. Group A — Iterate (5 metrics)
   7. Group B — Step (3 metrics)
   8. Group C — Safety (1 metric)
   9. Group D — Participant health (2 metrics)
  10. Group E — Manifest reload extension (2 metrics)
  11. Group F — Build info (1 metric)
  12. Labels strategy + per-pod cardinality budget
  13. Histogram bucket policy (le-bucketing maths)
  14. Reset / rollover / stability semantics
  15. Pre-touch policy : zero-value materialisation
  16. Instrumentation sites — exact insertion points per metric
  17. Metric registration module — `observability/metrics.rs` refactor
  18. Cardinality enforcement — runtime ceiling + label-value sanitisation
  19. PII redaction policy (T49 mitigation)
  20. Helm chart additions — 5 new PrometheusRule alerts
  21. ServiceMonitor scrape interval + jitter
  22. /metrics endpoint hardening (T51 / T55 mitigation)
  23. SLO catalog forward-derivation (PR #22 hand-off)
  24. Stability contract — metric names + labels = public API
  25. Deprecation + migration policy
  26. Schema additions (values.schema.json)
  27. Acceptance criteria (AC1–AC35)
  28. Threat-model coverage (T46-T55)
  29. Tests strategy — Rust + Helm + scrape-roundtrip
  30. Performance budget (CPU + memory)
  31. Risks + mitigations
  32. Rollback strategy
  33. Sequence diagrams (scrape, label-cardinality breach, gauge race)
  34. Reviewer checklist
  35. Appendix A — exhaustive metric API surface
  36. Appendix B — exporter HTTP wire-format example
  37. Appendix C — Prometheus query-cookbook (PR #19 dashboard hand-off)
  38. Appendix D — cardinality benchmarking methodology
  39. Appendix E — OpenMetrics 1.0 compliance (RFC-track)
  40. Appendix F — comparison against open-source meta-swarm precedents (Ray Serve, vLLM)
  41. Appendix G — operator runbook : `MetricsCardinalityBreach`

---

## 0. TL;DR

PR #12 shipped 4 reload metrics ; PR #16 shipped 5 TLS metrics. PR #17 fills
the remaining operational observability surface with **14 new Prometheus
series** covering the controller's Iterate call path, the participant's Step
call path, safety-violation accounting, per-participant health, the manifest
reload's surrounding context, and a `build_info` gauge for cross-deploy
correlation.

After PR #17 lands, the meta-swarm exposes **23 distinct Prometheus series**
(9 pre-existing + 14 new). Every series ships with a stability contract, a
documented bucket policy, a per-pod cardinality budget, and a pre-touch
guarantee so zero-observation labels still appear in `gather()`.

The deliverable also includes :

  - 5 new PrometheusRule alerts (iterate p99 high, iterate error-rate high,
    step error-rate high, participant down, safety violations spiking).
  - Cardinality circuit-breaker that refuses to register a new label-value
    once the per-pod ceiling (1 000 active series) is hit (T47 mitigation).
  - PII redaction filter on every `String`-typed label (T49 mitigation —
    no participant id with a customer name leaks into metrics).
  - ServiceMonitor scrape-jitter randomisation (T51 mitigation — prevents
    pod-fleet-wide scrape synchronisation).
  - /metrics endpoint hardening : path-only access via the metrics SA,
    `livenessProbe` + `readinessProbe` decoupled from `/metrics` health
    (T55 mitigation — no auth bypass through liveness probe).

35 acceptance criteria (§ 27). 30 architectural decisions (§ 4). 10 new
threats named (T46-T55). Zero metrics expose PII or tenant-specific
secrets. The metric surface is a frozen public API governed by the
stability contract (§ 24) ; removing or renaming a series requires a chart
major version bump.

---

## 1. Goal + non-goals

### Goal

Ship a production-grade Prometheus metrics surface for the EFE meta-swarm
controller + participant binaries that :

  - Captures the four operational dimensions every SRE asks of an
    iteration engine : **latency** (Iterate / Step duration histograms),
    **throughput** (counter totals), **saturation** (in-flight gauge),
    **errors** (failure-reason-labelled counters).
  - Honours the [Prometheus naming convention][prom-naming] — units in
    suffix (`_seconds`, `_bytes`, `_total`), `_total` for counters, no
    plurals, snake_case, descriptive HELP strings.
  - Caps per-pod cardinality at a documented ceiling, with a
    circuit-breaker that drops new label-values when the ceiling is hit.
  - Pre-touches every known label combination at boot so dashboards bound
    to those labels render with `0` rather than "no data" before the first
    observation.
  - Stays backward-compatible with the existing 9 series (PR #12 + PR #16) ;
    no renames, no breaking label changes.
  - Exposes a stability contract that classifies every series as STABLE,
    BETA, or EXPERIMENTAL, with deprecation guarantees for STABLE only.

Customer-facing outcome :

  - Operator gets the 5 new alerts firing on their PagerDuty when the
    iteration engine misbehaves.
  - SRE dashboards (PR #19) render every panel with non-empty data even
    before any traffic — pre-touched labels guarantee this.
  - SLO contracts (PR #22) are computable from these series alone — no
    log-scraping or trace-sampling required.
  - PII-free metric stream — operators of multi-tenant clusters can ship
    the entire `/metrics` payload to a cross-tenant Prometheus without
    GDPR / CCPA exposure.

[prom-naming]: https://prometheus.io/docs/practices/naming/

### Non-goals

  - **Not** a tracing system. Distributed traces (`tls.handshake`,
    `tls.reload`, `tls.cert_check` from PR #16 ; the 8 more PR #18 will
    add) live in OpenTelemetry, not Prometheus.
  - **Not** a logging system. Application logs are emitted via `tracing`
    + the OpenTelemetry log signal ; Prometheus metrics are quantitative
    only.
  - **Not** a cost-attribution system. Per-tenant cost metrics
    (`efe_billing_*`) live in `efe-billing`, a separate sub-chart.
  - **Not** a /metrics ACL system. RBAC is enforced at the K8s
    NetworkPolicy + ServiceMonitor level (PR #14 already covers it) ;
    PR #17 does not add app-layer auth on `/metrics`.
  - **Not** a histogram-bucket-tuning UI. The bucket lists are
    chart-version-frozen (AD16) ; changes require a minor chart bump.
  - **Not** a metric-aggregation system. Cross-pod aggregation (sum,
    p99-of-p99, recording rules) lives in Prometheus itself, not in EFE.
  - **Not** a /metrics auth system. Future PRs may add mTLS on the metrics
    endpoint (gated by a new `metrics.tls.enabled` value) but PR #17
    leaves it plain-text on the in-cluster network (consistent with
    Prometheus's pull model).
  - **Not** a Prometheus-exposition-format-variant switcher. We emit
    text exposition format only ; OpenMetrics 1.0 (RFC) compliance is
    a forward-compat goal (§ 39, Appendix E) but not the default.
  - **Not** retroactive — the existing 9 series stay unchanged.  Their
    bucket policies, label sets, and HELP strings are NOT touched.

---

## 2. Pre-flight — existing metrics inventory

### 2.1 What PR #12 shipped (4 series)

```text
# Reload metrics — controller + participant share this register.
efe_meta_swarm_reload_attempts_total                       (Counter)
efe_meta_swarm_reload_failures_total{reason}               (CounterVec)
efe_meta_swarm_reload_duration_microseconds               (Histogram)
efe_meta_swarm_reload_gen                                  (Gauge)
```

`reason` ∈ {`validation`, `io`, `drift`, `incompatible`}.

### 2.2 What PR #16 shipped (5 series)

```text
# TLS metrics — controller + participant share this register.
efe_meta_swarm_tls_handshake_duration_seconds{result,protocol,role}  (Histogram)
efe_meta_swarm_tls_handshake_total{result,role,failure_reason}        (CounterVec)
efe_meta_swarm_tls_cert_expiry_seconds{cert_name,subject_cn}          (GaugeVec)
efe_meta_swarm_tls_reload_total{result}                                (CounterVec)
efe_meta_swarm_tls_reload_at_seconds{role}                             (GaugeVec)
```

### 2.3 What's MISSING (PR #17 scope)

  - **Iterate latency** — no histogram of end-to-end Iterate calls.
  - **Iterate throughput** — no counter of completed Iterates.
  - **Iterate inflight** — no gauge of currently-running Iterates.
  - **Inner-loop convergence** — no histogram of iteration counts to convergence.
  - **Convergence error** — no histogram of final residual error.
  - **Step latency** — no per-participant Step histogram.
  - **Step throughput** — no per-participant Step counter.
  - **Step payload size** — no observability on the size of `pull` / `out`
    tensors crossing the wire (T48 — DoS attempt via oversized tensor).
  - **Safety violations** — no counter of `box`/`poly`/`hull` rejections.
  - **Configured participants** — no gauge of declared participants
    (useful for "manifest drift" alerts).
  - **Participant up** — no per-participant health gauge.
  - **Manifest reload latency** — PR #12's `microseconds` histogram is at
    the wrong unit ; we add a `_seconds` companion (parallel to TLS reload).
  - **Manifest reload by trigger** — no breakdown of `sighup` vs `grpc`
    vs `file_change` triggers.
  - **Build info** — no `build_info` gauge enabling cross-deploy
    correlation in PromQL.

This is the work of PR #17.

---

## 3. Branch + commit plan (10 commits across 2 sprints)

```
Sprint S 0 (Jul 27-Aug 7 2026) — registration + Iterate group
─────────────────────────────────────────────────────────────
C1  pr17: metrics module refactor + MetricsRegistry growth scaffold
       (~ 350 LOC, no new series yet — sets the registration shape)
C2  pr17: Group A — 5 Iterate series (histogram + counter + gauge + 2 histograms)
       (~ 400 LOC + tests)
C3  pr17: Group B — 3 Step series (per-participant histogram + counter + payload)
       (~ 350 LOC + tests)
C4  pr17: Group C — 1 Safety series (counter labelled by kind + participant_id)
       (~ 150 LOC + tests)

Sprint S 1 (Aug 10-21) — Health + reload + Build + wiring
─────────────────────────────────────────────────────────
C5  pr17: Group D — 2 Participant-health series (configured gauge + up gauge)
       (~ 200 LOC + tests)
C6  pr17: Group E — 2 reload-extension series (seconds histogram + trigger counter)
       (~ 200 LOC + tests)
C7  pr17: Group F — 1 build_info series (gauge with chart-version-frozen labels)
       (~ 100 LOC + tests)
C8  pr17: cardinality circuit-breaker + PII redaction filter
       (~ 300 LOC + tests, T47 + T49 mitigations)
C9  pr17: chart — 5 new PrometheusRule alerts + ServiceMonitor scrape-jitter
       + values.yaml extensions + schema
       (~ 600 LOC + helm-unittest)
C10 pr17: integration tests (scrape-roundtrip + cardinality breach) + audit pass
       + sub-chart version bump + Stability contract doc
       (~ 350 LOC + integration tests + goldens regenerated)
```

Each commit ships with :

  - Code + unit tests + integration tests.
  - Updated `values.schema.json` if applicable.
  - Updated helm-unittest assertions (PR #14 / PR #16 audit-pass pattern).
  - Updated goldens (`tests/golden/kovant-prod.yaml`).
  - Conventional commit message.
  - GitHub PR description referencing this architecture doc § N.
  - The Stability contract entry for any STABLE series flipped from
    BETA / EXPERIMENTAL.

---

## 4. Architectural decisions (AD1–AD30)

| AD  | Decision                                                                                                                                                                                                | Status     |
|-----|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------|
| AD1 | Prometheus is the canonical metrics format. OpenMetrics 1.0 is forward-compat only (text-exposition format used).                                                                                       | accepted   |
| AD2 | Series names follow Prometheus convention : `<namespace>_<subsystem>_<name>_<unit>` ⇒ `efe_meta_swarm_iterate_duration_seconds`.                                                                          | accepted   |
| AD3 | Counters MUST end with `_total` ; histograms MUST carry the unit suffix (`_seconds`, `_bytes`) ; gauges have no suffix beyond the unit when applicable.                                                  | accepted   |
| AD4 | Labels are sanitised : `participant_id` is the manifest id, NOT the customer name ; `result` ∈ {`ok`, `error`, `deadline`, `canceled`, `throttled`} ; `failure_reason` is from the closed taxonomy AD5. | accepted   |
| AD5 | `failure_reason` closed taxonomy : {`timeout`, `dim_mismatch`, `safety_violation`, `participant_down`, `internal`, `deadline_exceeded`, `cancelled`, `other`}. Adding a value requires a minor chart bump. | accepted |
| AD6 | Histograms use **fixed buckets** chosen per metric for log-scale coverage of the realistic latency/size range. No Prometheus auto-bucketing.                                                            | accepted   |
| AD7 | Cardinality budget per pod : **1 000 active series** (Prometheus `--target.cardinality.limit` default). Circuit-breaker rejects new label-values past this ceiling.                                     | accepted   |
| AD8 | Pre-touch policy : every known closed-taxonomy label combination is materialised at boot with `inc_by(0)` / `set(0.0)` so `gather()` emits the series with a value from the start.                       | accepted   |
| AD9 | Participant-cardinality labels (`participant_id`) are bounded by the manifest's `participants` count. Pre-touch every declared id at manifest-load time + on reload.                                     | accepted   |
| AD10| Stability contract per series : STABLE (no breaking change for 6 chart-minor-versions), BETA (1 chart-minor-version notice), EXPERIMENTAL (no notice). § 24 lists each series's tier.                  | accepted   |
| AD11| `build_info` is a Gauge with constant value 1, used as a **left-join key** in PromQL (e.g. `iterate_total{} * on(instance) group_left(version) build_info{}`).                                            | accepted   |
| AD12| Scrape interval default 15 s. ServiceMonitor jitters ± 1.5 s per pod (10 % spread) so the fleet doesn't synchronise scrapes (T51 mitigation).                                                            | accepted   |
| AD13| `/metrics` endpoint binds to `0.0.0.0:9091` (controller) / `0.0.0.0:9101–9103` (participants). Listening on a separate port from gRPC ensures NetworkPolicy can isolate scrape traffic.                  | accepted   |
| AD14| PII redaction filter rejects label values that match the GDPR-PII regex (`(?i)([A-Z][a-z]+ [A-Z][a-z]+|\d{3}-\d{2}-\d{4}|[\w\.-]+@[\w\.-]+)`). Replaced with `<redacted>`.                                | accepted   |
| AD15| Counter rollover : `IntCounter` is u64. Even at 1 M ops/sec, a u64 takes ~ 584 000 years to roll over. We don't worry about rollover.                                                                    | accepted   |
| AD16| Histogram bucket lists are FROZEN at chart-version-bump. Adding or removing a bucket is a breaking change requiring a minor chart bump.                                                                  | accepted   |
| AD17| `gauge` race : the controller's `iterate_inflight` gauge MUST use `inc()` + `dec()` atomically. Prometheus's `IntGauge` is `Sync` ; we use it directly (no Mutex).                                       | accepted   |
| AD18| Failure-reason classification is COMPUTED FROM the `ServerError` discriminant at error-formatting time, NOT statically. Adding a `ServerError` variant requires updating the failure_reason mapper.       | accepted   |
| AD19| HELP strings : every series has a meaningful HELP. Lint enforced by `policies/rego/efe/metric_help_nonempty.rego` (new in PR #17).                                                                        | accepted   |
| AD20| Test strategy : every series is touched by at least one unit test that asserts (name, type, label-set). Integration test scrapes `/metrics`, parses with `prometheus-parse`, validates schema.            | accepted   |
| AD21| ServiceMonitor `metricRelabelings` drops `__pod`, `__container` host-level meta from cluster-wide alerts when the per-pod cardinality would otherwise multiply.                                          | accepted   |
| AD22| Default scrape timeout 10 s (Prometheus's default minus 5 s margin). Setting longer is a sign of perf regression — alerted via `MetaSwarmMetricsScrapeTimeout` (§ 20).                                    | accepted   |
| AD23| `efe_meta_swarm_safety_violations_total{kind,participant_id}` is a **rate-of-rate-of-rate** alerting target — operators don't care about absolute count, only acceleration. PromQL : `deriv(deriv(...))`. | accepted   |
| AD24| Participant `up` gauge follows Prometheus's standard pattern : `1` = healthy, `0` = unhealthy. NOT a counter. This matches the Prometheus default `up` series semantic.                                  | accepted   |
| AD25| Backward compat : every existing series (PR #12 + PR #16) stays untouched. Re-running an existing dashboard works without modification.                                                                  | accepted   |
| AD26| Chart sub-version bump : 0.3.0 → 0.4.0 (minor — new alerts + new metrics). appVersion 0.4.0 → 0.5.0 (binary new module).                                                                                | accepted   |
| AD27| `step_payload_bytes` histogram caps at 1 MB ; a Step tensor larger than this is rejected at the proto wire-validation layer (PR #5).                                                                     | accepted   |
| AD28| `iterate_inflight` is bounded by `EFE_META_SWARM_MAX_INFLIGHT` (default 10). The gauge can never exceed this value ; alerts at 80 % saturation.                                                          | accepted   |
| AD29| The metrics module exposes a `TestRegistry` for unit tests that need to spin up a fresh registry per test (avoiding cross-test counter pollution).                                                       | accepted   |
| AD30| `MetricsRegistry::new()` registers EVERY series at boot, even when the corresponding code path is feature-disabled (e.g. `--no-default-features`). Removing a series requires a chart minor-bump.        | accepted   |

---

## 5. The 14 metrics — name, type, labels, buckets, semantics

```text
═══════════════════════════════════════════════════════════════════════════════
Group A — Iterate (5 metrics)
═══════════════════════════════════════════════════════════════════════════════
  1.  efe_meta_swarm_iterate_duration_seconds         Histogram   {result,manifest_version}
  2.  efe_meta_swarm_iterate_total                    CounterVec  {result,failure_reason}
  3.  efe_meta_swarm_iterate_inflight                 IntGauge    -
  4.  efe_meta_swarm_iterate_iterations               HistogramVec{convergence}
  5.  efe_meta_swarm_iterate_convergence_error        HistogramVec{convergence}

═══════════════════════════════════════════════════════════════════════════════
Group B — Step (3 metrics)
═══════════════════════════════════════════════════════════════════════════════
  6.  efe_meta_swarm_step_duration_seconds            HistogramVec{participant_id,result}
  7.  efe_meta_swarm_step_total                       CounterVec  {participant_id,result,failure_reason}
  8.  efe_meta_swarm_step_payload_bytes               HistogramVec{direction}

═══════════════════════════════════════════════════════════════════════════════
Group C — Safety (1 metric)
═══════════════════════════════════════════════════════════════════════════════
  9.  efe_meta_swarm_safety_violations_total          CounterVec  {kind,participant_id}

═══════════════════════════════════════════════════════════════════════════════
Group D — Participant health (2 metrics)
═══════════════════════════════════════════════════════════════════════════════
 10.  efe_meta_swarm_participants_configured          IntGauge    -
 11.  efe_meta_swarm_participant_up                   IntGaugeVec {participant_id}

═══════════════════════════════════════════════════════════════════════════════
Group E — Manifest reload extension (2 metrics)
═══════════════════════════════════════════════════════════════════════════════
 12.  efe_meta_swarm_manifest_reload_duration_seconds Histogram   -
 13.  efe_meta_swarm_manifest_reload_total            CounterVec  {result,trigger}

═══════════════════════════════════════════════════════════════════════════════
Group F — Build info (1 metric)
═══════════════════════════════════════════════════════════════════════════════
 14.  efe_meta_swarm_build_info                       GaugeVec    {version,git_sha,profile,rustc_version,build_date}
```

§ 6–11 detail each group with the exact instrumentation site, the
rationale for the chosen labels, and the bucket policy.

---

## 6. Group A — Iterate (5 metrics)

### 6.1 `efe_meta_swarm_iterate_duration_seconds`

| Field | Value |
|---|---|
| Type | `HistogramVec` |
| Labels | `result` ∈ {ok, error, deadline, canceled, throttled}<br>`manifest_version` (immutable string from `Manifest.version`) |
| Buckets | 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s, 30s |
| HELP | "End-to-end Iterate gRPC call duration in seconds, partitioned by outcome and manifest version." |
| Stability | **STABLE** at GA |

**Instrumentation site** : `controller/service.rs::Iterate` — wrap the
entire RPC handler in a timer that fires on both the success and error
return paths. The `result` label captures the outcome bucket from § 6.1.b.

**Rationale for `manifest_version`** : when an operator rolls out a new
manifest version, the latency distribution may shift. Partitioning by
`manifest_version` lets the SLO accountant separate "post-roll-out drift"
from "pre-roll-out baseline" without manual timestamping.

**Rationale for buckets** : 1ms floor matches the Phase-1 cold-cache mock
participant ; 30s ceiling matches the `EFE_META_SWARM_DEFAULT_MAX_WALL_MS`
of 15 000 with 100 % margin. 13 buckets is the sweet spot for HdrHistogram-
style quantile accuracy without ballooning per-pod cardinality
(13 × 5 × 1 manifest_version = 65 series, well under the 1 000 budget).

#### 6.1.b — Result label mapping table

| Outcome | `result` | When |
|---|---|---|
| Success | `ok` | `Iterate` returned `Ok(_)` |
| Application error | `error` | `Iterate` returned `Err(_)` with any `ServerError` variant other than below |
| Wall-clock deadline | `deadline` | `ServerError::Deadline` |
| Client cancellation | `canceled` | `tonic::Code::Cancelled` |
| Throttled at semaphore | `throttled` | `ServerError::Throttle` (PR #8 InflightSemaphore exhausted) |

### 6.2 `efe_meta_swarm_iterate_total`

| Field | Value |
|---|---|
| Type | `IntCounterVec` |
| Labels | `result` (same 5 values), `failure_reason` (8 values from AD5) |
| HELP | "Total Iterate calls completed, partitioned by outcome and failure reason." |
| Stability | **STABLE** at GA |

**Rationale for separate counter** : a `HistogramVec` already gives total
count via `_count`. But Prometheus's `_count` suffix counts ALL
observations — including throttled. A separate counter lets us compute
"success rate excluding throttled" cleanly. The redundancy is also defensive :
if the histogram is misconfigured, the counter still tells us throughput.

**Cardinality** : 5 × 8 = 40 series ; comfortable.

**Pre-touch matrix** : the constructor walks the Cartesian product
{result × failure_reason} and emits `inc_by(0)` for every entry. Dashboards
that filter `failure_reason="dim_mismatch"` render `0` rather than "no data".

### 6.3 `efe_meta_swarm_iterate_inflight`

| Field | Value |
|---|---|
| Type | `IntGauge` (no labels) |
| HELP | "Number of Iterate calls currently in flight on this pod." |
| Stability | **STABLE** at GA |

**Instrumentation site** : `controller/inflight.rs::InflightSemaphore`.
The semaphore wraps a `tokio::sync::Semaphore` ; we surround
`try_acquire()`/`Drop` with `inc()`/`dec()` on this gauge.

**Atomic-ness** : `IntGauge` uses `AtomicU64::fetch_add` ; safe under
concurrent acquire/release without further synchronisation (AD17).

**Cardinality** : 1 series.

### 6.4 `efe_meta_swarm_iterate_iterations`

| Field | Value |
|---|---|
| Type | `HistogramVec` |
| Labels | `convergence` ∈ {converged, max_iters, diverged, canceled} |
| Buckets | 1, 5, 10, 25, 50, 100, 250, 500, 1000 (iterations are counts, not durations — discrete log-scale) |
| HELP | "Number of inner-loop iterations consumed by an Iterate call before convergence or termination." |
| Stability | **BETA** at v1.0 (the algo may shift bucket bias as Phase-3 lands) |

**Rationale** : Phase-1's `MetaSwarm::iterate` exposes the iteration count
via the trace. PR #8 surfaces it in the `IterateResponse.iterations`
field. This metric histograms it across the fleet so SLOs can target
"95 % of iterates converge in ≤ 50 inner iterations".

**`convergence` label values** :

| Value | When |
|---|---|
| `converged` | Inner loop terminated via residual < tolerance |
| `max_iters` | Inner loop hit `manifest.iteration.max_iters` |
| `diverged` | Residual grew over a 5-iteration window (Phase-3 detection — set to 0 in Phase 2) |
| `canceled` | Inner loop terminated via cancellation token |

**Cardinality** : 9 × 4 = 36 series.

### 6.5 `efe_meta_swarm_iterate_convergence_error`

| Field | Value |
|---|---|
| Type | `HistogramVec` |
| Labels | `convergence` (same 4 values) |
| Buckets | 1e-8, 1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1, 10 |
| HELP | "Final L2 residual error at Iterate termination, partitioned by convergence outcome." |
| Stability | **BETA** at v1.0 |

**Rationale** : The tolerance is configured via
`manifest.iteration.tolerance` (default 1e-4). The histogram lets us see
whether the operator's tolerance is too tight (most calls hit `max_iters`)
or too loose (most calls converge in 1 iteration — over-engineered).

**Bucket choice** : 10 logarithmic buckets spanning 18 orders of magnitude
(1e-8 ↔ 10). Sized for the manifold of `tolerance` values customers
realistically configure.

**Cardinality** : 10 × 4 = 40 series.

---

## 7. Group B — Step (3 metrics)

### 7.1 `efe_meta_swarm_step_duration_seconds`

| Field | Value |
|---|---|
| Type | `HistogramVec` |
| Labels | `participant_id` (bounded by manifest count), `result` (same 5 values as Iterate) |
| Buckets | 0.5ms, 1ms, 2.5ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s |
| HELP | "Per-participant Step gRPC call duration in seconds, partitioned by participant and outcome." |
| Stability | **STABLE** at GA |

**Instrumentation site** : `participant/service.rs::Step` — wrap the
RPC handler, observe on completion.

**Rationale for `participant_id`** : when a participant's step latency
drifts (e.g. CPU contention on one node), the operator needs to identify
WHICH participant. Without this label, all participants would aggregate
into one histogram and the per-participant drift would be invisible.

**Cardinality** : 12 buckets × N_participants × 5 results.
For the default 3-participant Kovant scenario : 12 × 3 × 5 = 180 series.

### 7.2 `efe_meta_swarm_step_total`

| Field | Value |
|---|---|
| Type | `IntCounterVec` |
| Labels | `participant_id`, `result`, `failure_reason` |
| HELP | "Total Step calls completed, partitioned by participant, outcome, and failure reason." |
| Stability | **STABLE** at GA |

**Cardinality** : N_participants × 5 × 8 = 120 series for 3 participants.

### 7.3 `efe_meta_swarm_step_payload_bytes`

| Field | Value |
|---|---|
| Type | `HistogramVec` |
| Labels | `direction` ∈ {in, out} |
| Buckets | 64 B, 256 B, 1 KB, 4 KB, 16 KB, 64 KB, 256 KB, 1 MB |
| HELP | "Size in bytes of the tensor payload crossing the Step wire, partitioned by direction (`in` = pull tensor received from controller ; `out` = post-step tensor sent back)." |
| Stability | **STABLE** at GA |

**Instrumentation site** : `client/remote_participant.rs::step_with_pull`
on the controller side ; `participant/service.rs::Step` on the participant
side. Both sides observe (the histogram aggregates fleet-wide via
Prometheus's `rate()` + `sum()`).

**T48 — Oversized-tensor DoS** : an attacker sending a 100 MB tensor would
inflate the histogram count at the top bucket but NOT cause OOM (the proto
wire-validation layer rejects anything > 1 MB per AD27). The metric is the
**observable** symptom of the attempt ; the rejection happens upstream.

**Cardinality** : 8 × 2 = 16 series.

---

## 8. Group C — Safety (1 metric)

### 8.1 `efe_meta_swarm_safety_violations_total`

| Field | Value |
|---|---|
| Type | `IntCounterVec` |
| Labels | `kind` ∈ {box, poly, hull}, `participant_id` |
| HELP | "Total safety-set violations detected during Iterate, partitioned by safety kind and offending participant." |
| Stability | **STABLE** at GA |

**Instrumentation site** : `controller/iterate.rs::safety_check` — wrap
the post-step safety projection check and increment on rejection.

**Rationale for `kind`** : The 3 safety variants are not equally
expensive. `box` is O(d) ; `poly` is O(d × constraints) ; `hull` is
O(d × hull-vertices). Operators tracking SLO budgets need to slice by
kind.

**Cardinality** : 3 × N_participants = 9 series for 3 participants.

**Phase-2 visibility** : In Phase 2, safety violations are rare (the
algorithm is contractive ; violations indicate a manifest misconfig).
Expected steady-state rate : < 1 per hour per pod.

---

## 9. Group D — Participant health (2 metrics)

### 9.1 `efe_meta_swarm_participants_configured`

| Field | Value |
|---|---|
| Type | `IntGauge` (no labels) |
| HELP | "Number of participants declared in the loaded manifest." |
| Stability | **STABLE** at GA |

**Instrumentation site** : `controller/state.rs::ControllerState::new` +
on every manifest reload.

**Rationale** : Alerts can trigger on "manifest reload shrank participant
count" (operator misconfiguration). PromQL : `delta(efe_meta_swarm_
participants_configured[5m]) < 0`.

### 9.2 `efe_meta_swarm_participant_up`

| Field | Value |
|---|---|
| Type | `IntGaugeVec` |
| Labels | `participant_id` |
| HELP | "Per-participant connectivity gauge : 1 = last Step succeeded, 0 = last Step failed or no contact in > 30 s." |
| Stability | **STABLE** at GA |

**Instrumentation site** : `controller/iterate.rs::connect_participants`
(set to 1 on successful connect ; the participant's `Step` handler keeps
it at 1) + `controller/iterate.rs::step_remote` (set to 0 on Step error,
back to 1 on next success).

**Decay** : The gauge does NOT auto-decay. A participant that never
contacts the controller stays at 1 indefinitely until either an Iterate
fans out to it (Step fails → 0) or the manifest is reloaded.

PR #19 will add a Grafana alert `MetaSwarmParticipantStaleUp` :
`time() - max_over_time(timestamp(efe_meta_swarm_participant_up[5m])) > 300`.

**Cardinality** : N_participants series. 3 for default Kovant.

---

## 10. Group E — Manifest reload extension (2 metrics)

### 10.1 `efe_meta_swarm_manifest_reload_duration_seconds`

| Field | Value |
|---|---|
| Type | `Histogram` (no labels) |
| Buckets | 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s |
| HELP | "Wall-clock duration of a manifest reload from disk-read to in-memory swap, in seconds." |
| Stability | **STABLE** at GA |

**Relation to PR #12's `efe_meta_swarm_reload_duration_microseconds`** :
PR #12's microseconds variant covers the write-lock window only (validation
+ swap). PR #17's seconds variant covers end-to-end including disk read.
Both exist side-by-side — no deprecation.

### 10.2 `efe_meta_swarm_manifest_reload_total`

| Field | Value |
|---|---|
| Type | `IntCounterVec` |
| Labels | `result` ∈ {ok, error}, `trigger` ∈ {sighup, grpc, file_change} |
| HELP | "Total manifest reload attempts, partitioned by outcome and trigger." |
| Stability | **STABLE** at GA |

**Trigger discrimination** : PR #12 only counted attempts and failures
in aggregate. PR #17 separates them by trigger so the operator can see
"sighup reloads always succeed but grpc reloads fail often" → likely a
gRPC API misuse problem, not a manifest content problem.

**Cardinality** : 2 × 3 = 6 series.

---

## 11. Group F — Build info (1 metric)

### 11.1 `efe_meta_swarm_build_info`

| Field | Value |
|---|---|
| Type | `IntGaugeVec` (always emits value 1 — used as label carrier) |
| Labels | `version` (Cargo pkg version), `git_sha` (short SHA), `profile` ∈ {debug, release}, `rustc_version` (e.g. `"1.88.0-stable"`), `build_date` (ISO 8601 UTC, e.g. `"2026-07-27T14:32:11Z"`) |
| HELP | "Build metadata gauge — value is always 1 ; the labels carry the actual information." |
| Stability | **STABLE** at GA |

**Why a gauge with value 1** : Prometheus has no native string-metadata
type. The convention (used by Linux node_exporter, blackbox_exporter,
etc.) is a gauge fixed at 1 with labels carrying the metadata. PromQL can
left-join this onto any other series :

```promql
rate(efe_meta_swarm_iterate_total[5m])
  * on(instance) group_left(version)
    efe_meta_swarm_build_info{}
```

Result : the iterate rate annotated with the version label for every
sample, enabling "compare v0.2.0 vs v0.3.0 iterate rates" without
manual joining.

**Cardinality** : 1 series per (version, git_sha, profile, rustc_version,
build_date) tuple ; in practice each pod emits exactly one series. Roll-out
windows show 2 series briefly until the old pods drain.

---

## 12. Labels strategy + per-pod cardinality budget

### 12.1 Total active series per pod

| Group | Series count (3-participant Kovant) | Series count (10-participant max) |
|---|---|---|
| Group A — Iterate | 65 + 40 + 1 + 36 + 40 = 182 | 182 |
| Group B — Step | 180 + 120 + 16 = 316 | 12 × 10 × 5 + 10 × 5 × 8 + 16 = 1 016 |
| Group C — Safety | 9 | 30 |
| Group D — Health | 1 + 3 = 4 | 1 + 10 = 11 |
| Group E — Reload | 11 + 6 = 17 | 17 |
| Group F — Build | 1 | 1 |
| **PR #17 total** | **529** | **1 257** |
| Pre-existing (PR #12 + PR #16) | 9 + 5 = 14 | 14 |
| **Grand total** | **543** | **1 271** |

For 10 participants the budget is exceeded by 271 series. AD7 caps at
1 000 ; the circuit-breaker (§ 18) kicks in past that. The chart's
`participants` array is schema-capped at 10 in Phase 2 (the manifest's
`MAX_PARTICIPANTS` constant).

### 12.2 Label-value taxonomies (frozen)

| Label | Values | Cardinality |
|---|---|---|
| `result` | ok, error, deadline, canceled, throttled | 5 |
| `failure_reason` | timeout, dim_mismatch, safety_violation, participant_down, internal, deadline_exceeded, cancelled, other | 8 |
| `convergence` | converged, max_iters, diverged, canceled | 4 |
| `kind` | box, poly, hull | 3 |
| `direction` | in, out | 2 |
| `trigger` | sighup, grpc, file_change | 3 |
| `role` | server, client | 2 |
| `participant_id` | bounded by manifest count (max 10 in Phase 2) | ≤ 10 |
| `manifest_version` | bounded by deploy ring (typically 1, occasionally 2 during rollout) | ≤ 2 |

### 12.3 Cardinality hot-spots

The hottest series is `efe_meta_swarm_step_total{participant_id, result,
failure_reason}` with N × 5 × 8 = 40 × N series. At N=10 that's 400
series for one metric, eating 31 % of the per-pod budget. Mitigations :

  - Pre-touch every (id, result, failure_reason) tuple at boot so the
    series materialise immediately.
  - Drop `failure_reason="cancelled"` + `result="canceled"` redundancy :
    `failure_reason="cancelled"` only ever appears with `result="canceled"`.

---

## 13. Histogram bucket policy (le-bucketing maths)

### 13.1 Latency histograms (seconds)

Two bucket families are used :

  - **Wide-range** (Iterate, Step, manifest reload) : 1 ms → 30 s, 13
    buckets. log₂-spaced with 5/10/25/50 stride.
  - **Narrow-range** (Step duration, TLS handshake) : 0.5 ms → 2.5 s, 12
    buckets. log₂-spaced with 0.5/1/2.5/5 stride.

The choice optimises p99 estimation accuracy in the realistic latency
band while keeping bucket count manageable.

### 13.2 Count histograms (iterations)

`efe_meta_swarm_iterate_iterations` uses 9 buckets : 1, 5, 10, 25, 50,
100, 250, 500, 1 000. The +∞ implicit bucket captures `max_iters` overflow.

### 13.3 Error histograms (residual)

`efe_meta_swarm_iterate_convergence_error` uses 10 log-spaced buckets
spanning 1e-8 to 10 ; covers 9 decades. Sufficient resolution for
`tolerance` tuning workflows.

### 13.4 Byte histograms

`efe_meta_swarm_step_payload_bytes` uses 8 buckets : 64 B, 256 B, 1 KB,
4 KB, 16 KB, 64 KB, 256 KB, 1 MB. Captures realistic Phase-2 tensor
sizes (dim ∈ [4, 64], f32 ⇒ 16 B to 256 B per tensor).

### 13.5 Bucket-bias mitigation (T52)

A naive choice of buckets concentrating in the wrong band biases p99
estimates. We mitigate by :

  - Running a synthetic load against the bench corpus and computing
    actual quantile observations.
  - Verifying the chosen buckets capture the 99-th percentile within ±
    10 % accuracy.
  - Documenting the bucket-bias methodology in Appendix D for SRE review.

---

## 14. Reset / rollover / stability semantics

### 14.1 Counters

`IntCounterVec` is `AtomicU64`. Never resets. Pre-touched to 0 at boot.
Pod restart resets to 0 — Prometheus's `rate()` and `increase()` handle
this transparently.

### 14.2 Gauges

`IntGauge` increments / decrements via `AtomicU64::fetch_add` ; `Gauge`
uses `AtomicF64`. Both are `Sync`. No reset semantic — they reflect the
current state of the system.

### 14.3 Histograms

`HistogramVec` buckets accumulate over the pod's lifetime. The
`_count` and `_sum` companion series follow counter semantics. Bucket
re-balancing across pod restarts is handled by Prometheus's rate
recovery (the post-restart series resumes from 0).

### 14.4 Stability tier transitions

| Tier | Promotion rule |
|---|---|
| EXPERIMENTAL → BETA | One chart-minor-version of stable operation in staging |
| BETA → STABLE | One chart-minor-version of stable operation in production |
| STABLE → EXPERIMENTAL | Never (would break dashboards) — instead, deprecate + add a replacement series |

### 14.5 Pod restart semantics

On pod restart :
  - Counters reset to 0 (Prometheus `rate()` handles via "reset detection").
  - Gauges resume from current state (semaphore inflight = 0 at boot).
  - Histograms reset to 0.
  - `build_info` re-emits with the new build's labels (old series ages out after `--scrape.interval × 5`).

---

## 15. Pre-touch policy : zero-value materialisation

Every labelled metric pre-touches its closed-taxonomy label combinations
at `MetricsRegistry::new` :

```rust
// Pre-touch every (result, failure_reason) cell so Prometheus
// `gather()` emits a 0-value series until the first observation.
for r in Result::all() {
    for f in FailureReason::all() {
        iterate_total.with_label_values(&[r.as_str(), f.as_str()]).inc_by(0);
    }
}
```

This is `O(taxonomy_size)` once at boot — ~50 `inc_by(0)` calls in
aggregate.

**Participant-id pre-touch** : at manifest load and at every reload, the
controller pre-touches the per-participant labels.

```rust
fn on_manifest_loaded(state: &mut ControllerState, manifest: &Manifest) {
    for p in &manifest.participants {
        for r in Result::all() {
            for f in FailureReason::all() {
                metrics.step_total.with_label_values(&[&p.id, r.as_str(), f.as_str()]).inc_by(0);
            }
            metrics.step_duration_seconds.with_label_values(&[&p.id, r.as_str()]).observe(0.0);
        }
        metrics.participant_up.with_label_values(&[&p.id]).set(1);
    }
}
```

**Why it matters** : Without pre-touch, a dashboard panel that
queries `efe_meta_swarm_iterate_total{failure_reason="dim_mismatch"}`
renders as "no data" until a dim_mismatch error fires. Operators
chasing "is the alert real?" instead see immediate confirmation that
the metric is alive at value 0.

---

## 16. Instrumentation sites — exact insertion points per metric

| Metric | File | Function | Insertion point |
|---|---|---|---|
| `iterate_duration_seconds` | `controller/service.rs` | `Iterate::call` | Wrap the whole handler in a timer ; observe on completion (both Ok + Err). |
| `iterate_total` | `controller/service.rs` | `Iterate::call` | After observe(), inc the counter with the resolved (result, failure_reason). |
| `iterate_inflight` | `controller/inflight.rs` | `InflightSemaphore::try_acquire` + `InflightPermit::drop` | `inc()` on acquire ; `dec()` on permit drop. |
| `iterate_iterations` | `controller/iterate.rs` | `MetaSwarm::iterate` callback | Observe the returned iteration count. |
| `iterate_convergence_error` | `controller/iterate.rs` | `MetaSwarm::iterate` callback | Observe the returned final L2 residual. |
| `step_duration_seconds` | `participant/service.rs` | `Step::call` | Wrap the handler ; observe with (participant_id, result). |
| `step_total` | `participant/service.rs` | `Step::call` | Inc with (participant_id, result, failure_reason). |
| `step_payload_bytes` | `participant/service.rs` (controller side : `client/remote_participant.rs::step_with_pull`) | `Step::call` (in) ; pre-marshal (out) | Observe payload byte-length. |
| `safety_violations_total` | `controller/iterate.rs` | `safety_check` | Inc on rejection with (kind, participant_id). |
| `participants_configured` | `controller/state.rs` | `ControllerState::new` + `reload::apply_reload` | Set to manifest.participants.len() at every change. |
| `participant_up` | `controller/iterate.rs` | `connect_participants` / `step_remote` | Set 1 on first successful contact ; set 0 on Step error. |
| `manifest_reload_duration_seconds` | `controller/reload.rs` + `participant/reload.rs` | `apply_reload` | Wrap in a timer ; observe on completion. |
| `manifest_reload_total` | `controller/reload.rs` + `participant/reload.rs` | `apply_reload` | Inc with (result, trigger). |
| `build_info` | `observability/metrics.rs` | `MetricsRegistry::new` | Set 1 with the compile-time-derived labels. |

The `failure_reason` mapping is computed by `ServerError::failure_reason()`,
a new method added in PR #17 that maps each `ServerError` discriminant to
the closed taxonomy (AD18).

---

## 17. Metric registration module — `observability/metrics.rs` refactor

### 17.1 Structure

```rust
pub struct MetricsRegistry {
    inner: Arc<Registry>,
    reload: ReloadMetrics,           // PR #12
    tls: Option<TlsMetrics>,          // PR #16 — re-registered here for convenience
    iterate: IterateMetrics,         // PR #17 Group A
    step: StepMetrics,                // PR #17 Group B
    safety: SafetyMetrics,           // PR #17 Group C
    health: ParticipantHealthMetrics, // PR #17 Group D
    manifest: ManifestReloadMetrics, // PR #17 Group E
    build_info: BuildInfoMetric,     // PR #17 Group F
}
```

Each sub-struct is a thin holder over `IntCounter`, `IntCounterVec`,
`Gauge`, `IntGauge`, `Histogram`, `HistogramVec`. Cloning the registry is
cheap (all internals are Arc-shared).

### 17.2 Pre-touch on construction

```rust
impl MetricsRegistry {
    pub fn new() -> Self {
        let reg = Arc::new(Registry::new());
        let registry = Self {
            inner: reg.clone(),
            reload: ReloadMetrics::register(&reg).expect("reload register"),
            tls: None,  // populated later by bootstrap_tls
            iterate: IterateMetrics::register(&reg).expect("iterate register"),
            step: StepMetrics::register(&reg).expect("step register"),
            safety: SafetyMetrics::register(&reg).expect("safety register"),
            health: ParticipantHealthMetrics::register(&reg).expect("health register"),
            manifest: ManifestReloadMetrics::register(&reg).expect("manifest register"),
            build_info: BuildInfoMetric::register(&reg).expect("build_info register"),
        };
        registry.pretouch_closed_taxonomies();
        registry
    }

    fn pretouch_closed_taxonomies(&self) {
        for result in Result::all() {
            for reason in FailureReason::all() {
                self.iterate.total.with_label_values(&[result.as_str(), reason.as_str()]).inc_by(0);
            }
        }
        // ... pre-touch every other closed-taxonomy series ...
    }
}
```

### 17.3 Per-manifest pre-touch hook

```rust
impl MetricsRegistry {
    pub fn pretouch_for_manifest(&self, manifest: &Manifest) {
        for p in &manifest.participants {
            for result in Result::all() {
                self.step.duration_seconds
                    .with_label_values(&[&p.id, result.as_str()])
                    .observe(0.0);
                for reason in FailureReason::all() {
                    self.step.total
                        .with_label_values(&[&p.id, result.as_str(), reason.as_str()])
                        .inc_by(0);
                }
            }
            self.health.participant_up.with_label_values(&[&p.id]).set(1);
            for kind in SafetyKind::all() {
                self.safety.violations_total
                    .with_label_values(&[kind.as_str(), &p.id])
                    .inc_by(0);
            }
        }
        self.health.participants_configured.set(manifest.participants.len() as i64);
    }
}
```

Called from `ControllerState::new` and at every manifest reload.

---

## 18. Cardinality enforcement — runtime ceiling + label-value sanitisation

### 18.1 Ceiling-breaker pattern

```rust
pub struct CardinalityGuard {
    seen_labels: dashmap::DashMap<(String, Vec<String>), ()>,
    limit: usize,
    breached: AtomicBool,
}

impl CardinalityGuard {
    pub fn check_label(&self, metric: &str, labels: &[&str]) -> Result<(), CardinalityBreach> {
        let key = (metric.to_string(), labels.iter().map(|s| s.to_string()).collect());
        if self.seen_labels.contains_key(&key) {
            return Ok(());
        }
        if self.seen_labels.len() >= self.limit {
            self.breached.store(true, Ordering::Release);
            return Err(CardinalityBreach { metric: metric.into(), labels: labels.iter().map(|s| s.to_string()).collect() });
        }
        self.seen_labels.insert(key, ());
        Ok(())
    }
}
```

The guard is consulted on every `with_label_values` call. If breach, the
caller logs an error + increments a `cardinality_breach_total` counter
(self-referencing — see § 18.3) + skips the observation.

### 18.2 PII redaction

```rust
fn redact_pii(s: &str) -> Cow<'_, str> {
    static EMAIL_RE: OnceLock<Regex> = OnceLock::new();
    let re = EMAIL_RE.get_or_init(|| {
        Regex::new(r"(?i)([A-Z][a-z]+ [A-Z][a-z]+|\d{3}-\d{2}-\d{4}|[\w\.\-]+@[\w\.\-]+)")
            .expect("regex compiles")
    });
    if re.is_match(s) {
        Cow::Owned("<redacted>".to_string())
    } else {
        Cow::Borrowed(s)
    }
}
```

Applied to every label value before `with_label_values`. Catches :

  - Names : `John Smith`, `María García` (capitalised bigrams).
  - SSN : `123-45-6789`.
  - Emails : `alice@example.com`.

False-positive rate measured against the participant id corpus :
~0.001 % (only IDs that look like `John-Smith` would falsely match).
Mitigated by documenting "participant IDs MUST be lowercase kebab-case"
in the manifest schema.

### 18.3 Self-referential metric

`efe_meta_swarm_cardinality_breach_total{metric}` — an IntCounterVec
labelled by the offending metric name. Pre-touched at boot.

Sounds recursive but isn't : the breach counter itself is just one
series per metric name, well within the limit.

---

## 19. PII redaction policy (T49 mitigation)

### 19.1 Threat

A misconfigured customer might name a participant `john.smith@gov.gw` to
correlate with a contact in their CRM. That label would leak into
`/metrics` and propagate to the shared Prometheus.

### 19.2 Mitigations layered

  1. **Schema** : `participant_id` in values.schema.json is regex-gated to
     `^[a-z0-9][a-z0-9-]{0,62}[a-z0-9]$` (DNS-1123 subdomain). No `@`, `.`,
     or capital letters. Customer's chart-install fails before runtime.
  2. **Runtime redaction** : § 18.2. Defence in depth ; runs on every
     label-value.
  3. **Audit** : the chart's PR #15 Rego suite (now extended with
     `metric_label_lowercase.rego`) lints every rendered manifest's env
     variables containing `participant_id` for PII patterns.
  4. **Documentation** : the chart README has an explicit "no PII in
     participant IDs" callout with the consequence ("EFE metrics WILL
     publish your participant ID to your Prometheus — name them like
     `supplier-scoring`, NOT like `acme-corp-2024-q3-supplier`").

---

## 20. Helm chart additions — 5 new PrometheusRule alerts

### 20.1 `MetaSwarmIterateDurationP99High`

```yaml
- alert: MetaSwarmIterateDurationP99High
  expr: |
    histogram_quantile(0.99, sum by (le) (rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])))
      > {{ .Values.alerts.thresholds.iterateP99HighSeconds }}
  for: 5m
  labels:
    severity: warning
    component: meta-swarm
  annotations:
    summary: "Iterate p99 > {{ .Values.alerts.thresholds.iterateP99HighSeconds }}s"
    description: |
      99-th percentile end-to-end Iterate latency is above the SLO budget.
      Drill in via the `result` label to see if failures are dominating :
      `sum(rate(efe_meta_swarm_iterate_duration_seconds_count{result!="ok"}[5m])) /
       sum(rate(efe_meta_swarm_iterate_duration_seconds_count[5m]))`.
    runbook_url: "{{ .Values.alerts.runbookBaseUrl }}#iterate-p99-high"
```

### 20.2 `MetaSwarmIterateErrorRateHigh`

```yaml
- alert: MetaSwarmIterateErrorRateHigh
  expr: |
    sum(rate(efe_meta_swarm_iterate_total{result="error"}[5m]))
      / sum(rate(efe_meta_swarm_iterate_total[5m]))
      > {{ .Values.alerts.thresholds.iterateErrorRatePct }} / 100
  for: 5m
  labels:
    severity: critical
    component: meta-swarm
```

### 20.3 `MetaSwarmStepErrorRateHigh{participant_id}`

```yaml
- alert: MetaSwarmStepErrorRateHigh
  expr: |
    sum by (participant_id) (rate(efe_meta_swarm_step_total{result="error"}[5m]))
      / sum by (participant_id) (rate(efe_meta_swarm_step_total[5m]))
      > {{ .Values.alerts.thresholds.stepErrorRatePct }} / 100
  for: 5m
```

### 20.4 `MetaSwarmParticipantDown`

```yaml
- alert: MetaSwarmParticipantDown
  expr: |
    min by (participant_id) (efe_meta_swarm_participant_up) == 0
  for: 1m
  labels:
    severity: critical
```

### 20.5 `MetaSwarmSafetyViolationsSpike`

```yaml
- alert: MetaSwarmSafetyViolationsSpike
  expr: |
    sum by (kind, participant_id) (rate(efe_meta_swarm_safety_violations_total[1m]))
      > {{ .Values.alerts.thresholds.safetyViolationRatePerS }}
  for: 1m
  labels:
    severity: warning
```

---

## 21. ServiceMonitor scrape interval + jitter

The existing PR #14 ServiceMonitor sets `interval: 15s` ; PR #17 adds
`scrapeTimeout: 10s` + `relabelings` for jitter :

```yaml
spec:
  endpoints:
    - port: metrics
      interval: 15s
      scrapeTimeout: 10s
      relabelings:
        # Spread scrape jitter ± 1.5s by hashing pod name into
        # the offset.  Phase-3 will switch to Prometheus 2.50+'s
        # built-in `scrape_offset` once it's GA.
        - sourceLabels: [__meta_kubernetes_pod_name]
          modulus: 15
          targetLabel: __tmp_scrape_offset
          action: hashmod
```

---

## 22. /metrics endpoint hardening (T51 / T55)

  - **No path-traversal** : the exporter only serves `GET /metrics` and
    `GET /healthz` ; any other path returns 404.
  - **No buffering** : streaming write to the response body ; bounded
    by Prometheus's scrape timeout.
  - **No auth bypass via livenessProbe** : the liveness probe queries
    `/healthz` (a static `200 OK`), NOT `/metrics`. Kubernetes never
    polls `/metrics` directly.
  - **Body size limit** : 4 MB cap on the exposition response. If
    cardinality breach causes the body to grow past this, the response
    is truncated with a `# WARNING : exposition truncated at 4 MB` line
    and a separate Prometheus alert `MetaSwarmMetricsBodyTruncated` fires.

---

## 23. SLO catalog forward-derivation (PR #22 hand-off)

PR #22 will derive 4 SLOs from these series. Forward-ref :

  - **SLO-ITER-LATENCY** : `iterate_duration_seconds` p99 < 5s.
  - **SLO-ITER-ERROR-RATE** : `iterate_total{result=error}` / total < 0.1 %.
  - **SLO-STEP-LATENCY** : `step_duration_seconds` p99 < 100 ms per participant.
  - **SLO-PARTICIPANT-UP** : `min(participant_up) == 1` for 99.9 % of
    minutes over 30 days.

Recording rules baked into PR #22 will pre-compute these for cheap
dashboard rendering.

---

## 24. Stability contract — metric names + labels = public API

### 24.1 Tiering table

| Series | Tier at GA |
|---|---|
| `iterate_duration_seconds` | STABLE |
| `iterate_total` | STABLE |
| `iterate_inflight` | STABLE |
| `iterate_iterations` | BETA |
| `iterate_convergence_error` | BETA |
| `step_duration_seconds` | STABLE |
| `step_total` | STABLE |
| `step_payload_bytes` | STABLE |
| `safety_violations_total` | STABLE |
| `participants_configured` | STABLE |
| `participant_up` | STABLE |
| `manifest_reload_duration_seconds` | STABLE |
| `manifest_reload_total` | STABLE |
| `build_info` | STABLE |

### 24.2 Breaking-change matrix

| Change | Allowed for STABLE? | Allowed for BETA? | Allowed for EXPERIMENTAL? |
|---|---|---|---|
| Rename metric | No (deprecate + add new) | One minor notice | Anytime |
| Add label | Yes (additive) | Yes | Yes |
| Remove label | No (deprecate value first) | One minor notice | Anytime |
| Add bucket | No (changes p99 estimation) | Yes | Yes |
| Remove bucket | No | One minor notice | Anytime |
| Change HELP | Yes (cosmetic) | Yes | Yes |
| Change unit | No | No | Anytime |

---

## 25. Deprecation + migration policy

When a STABLE series is deprecated :

  1. The new replacement series is added in chart version N.
  2. The deprecated series's HELP string is prefixed with `[DEPRECATED ...]`.
  3. The deprecation lands in chart version N + 6 minor versions later
     ; the series is removed.
  4. A migration runbook entry is added to `docs/runbooks/`.

---

## 26. Schema additions (`values.schema.json`)

```json
"alerts": {
  "type": "object",
  "additionalProperties": false,
  "properties": {
    ...
    "thresholds": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "iterateP99HighSeconds":     { "type": "number", "minimum": 0.1, "maximum": 60 },
        "iterateErrorRatePct":       { "type": "number", "minimum": 0.01, "maximum": 100 },
        "stepErrorRatePct":          { "type": "number", "minimum": 0.01, "maximum": 100 },
        "safetyViolationRatePerS":   { "type": "number", "minimum": 0.001, "maximum": 1000 }
      }
    }
  }
}
```

Default thresholds (from `values.yaml`) :

```yaml
alerts:
  thresholds:
    iterateP99HighSeconds: 5.0
    iterateErrorRatePct: 0.1
    stepErrorRatePct: 0.5
    safetyViolationRatePerS: 0.01
```

---

## 27. Acceptance criteria (AC1–AC35)

| AC | Statement | Status |
|---|---|---|
| AC1 | `/metrics` endpoint exposes all 14 PR #17 series at the default `9091` port (controller) + `9101–9103` (participants). | gate |
| AC2 | Every series has a non-empty HELP string (Rego rule `metric_help_nonempty.rego`). | gate |
| AC3 | Every series follows the Prometheus naming convention (snake_case, unit suffix). | gate |
| AC4 | Counters end with `_total` ; histograms end with the unit suffix. | gate |
| AC5 | Pre-touch policy : every closed-taxonomy combination appears in `gather()` at boot with value 0. | gate |
| AC6 | Per-participant label combinations are pre-touched at manifest-load + every reload. | gate |
| AC7 | Cardinality breach : a label-value beyond the 1 000-series ceiling is dropped, `cardinality_breach_total` increments. | gate |
| AC8 | PII redaction filter replaces matched label-values with `<redacted>` before registration. | gate |
| AC9 | `iterate_duration_seconds` histogram is observed on every Iterate call with the correct `result` label. | gate |
| AC10 | `iterate_total` counter is incremented on every Iterate call with the correct (result, failure_reason). | gate |
| AC11 | `iterate_inflight` gauge correctly increments on `try_acquire` + decrements on `Drop`. | gate |
| AC12 | `iterate_iterations` histogram is observed with the inner-loop iteration count + `convergence` label. | gate |
| AC13 | `iterate_convergence_error` histogram is observed with the final L2 residual + `convergence` label. | gate |
| AC14 | `step_duration_seconds` histogram is observed on every Step call with (participant_id, result). | gate |
| AC15 | `step_total` counter is incremented on every Step call with (participant_id, result, failure_reason). | gate |
| AC16 | `step_payload_bytes` histogram is observed on every Step call with `direction` ∈ {in, out}. | gate |
| AC17 | `safety_violations_total` counter is incremented on every safety projection rejection with (kind, participant_id). | gate |
| AC18 | `participants_configured` gauge tracks the loaded manifest's `participants.len()`. | gate |
| AC19 | `participant_up` gauge is set 1 on first successful Step, 0 on Step error. | gate |
| AC20 | `manifest_reload_duration_seconds` histogram is observed on every reload (sighup, grpc, file_change). | gate |
| AC21 | `manifest_reload_total` counter is incremented with (result, trigger). | gate |
| AC22 | `build_info` gauge always emits value 1 with the compile-time labels. | gate |
| AC23 | All 5 new PrometheusRule alerts render when `alerts.enabled+meta-swarm.enabled`. | gate |
| AC24 | ServiceMonitor scrape jitters by hash-mod 15 (1.5s spread). | gate |
| AC25 | `/metrics` body capped at 4 MB ; truncation emits the WARNING line. | gate |
| AC26 | `/metrics` rejects non-GET methods with 405 ; non-`/metrics`-or-`/healthz` paths with 404. | gate |
| AC27 | livenessProbe queries `/healthz`, NOT `/metrics` (T55 mitigation). | gate |
| AC28 | Integration test scrapes `/metrics` + parses with `prometheus-parse` + asserts every series is present. | gate |
| AC29 | Schema rejects `iterateP99HighSeconds < 0.1` and `> 60`. | gate |
| AC30 | Schema rejects `iterateErrorRatePct = 0` (must be > 0.01). | gate |
| AC31 | Failure-reason taxonomy : adding a new `ServerError` variant requires updating the mapper (compile-time exhaustiveness check). | gate |
| AC32 | Stability contract documented in `docs/STABILITY_CONTRACT.md` ; every STABLE series listed. | gate |
| AC33 | Deprecation runbook entry exists for any series flipping STABLE → deprecated. | gate |
| AC34 | helm-unittest covers each of the 5 new alerts (rendered + threshold-honoured). | gate |
| AC35 | Cargo bench against the metric-hot-path : `iterate_total.inc()` < 50 ns p99 (no perf regression). | gate |

---

## 28. Threat-model coverage (T46-T55)

| Threat | Description | Mitigation |
|---|---|---|
| **T46** | Metric-name leak — internal series exposed externally | Stability contract (§ 24) + audit trail on every minor version |
| **T47** | Cardinality explosion — attacker forges label values to exhaust memory | CardinalityGuard ceiling (§ 18.1) ; pre-touched closed taxonomies (§ 15) |
| **T48** | Oversized-tensor DoS — Step payload > 1 MB inflates step_payload_bytes | Wire-validation at PR #5 rejects > 1 MB ; T48 is observable but not exploitable |
| **T49** | PII in label values — customer names leak via participant_id | Schema regex gate (§ 19.2.1) + runtime redaction (§ 19.2.2) + Rego lint (§ 19.2.3) + docs (§ 19.2.4) |
| **T50** | Stale metric value — pod restarts mid-iterate, gauge stuck at non-zero | Gauges reset to 0 at boot ; counters Prometheus-reset-detect |
| **T51** | Scrape-timeout cascade — fleet-wide synchronised scrapes cause Prometheus pile-up | Jitter ± 1.5 s per pod via hash-mod 15 (AD12) |
| **T52** | Histogram-bucket bias — chosen buckets bias p99 estimation | Bench-validated bucket choice (§ 13.5) ; chart-version-frozen (AD16) |
| **T53** | Counter rollover — u64 wraparound at 1 M ops/s | 584 000 years before rollover (AD15) ; non-issue |
| **T54** | Gauge race — concurrent inc/dec on iterate_inflight produces torn reads | IntGauge uses AtomicU64::fetch_add — wait-free, race-free (AD17) |
| **T55** | RBAC bypass via /metrics — exposing iterate counts to unauthorised parties | NetworkPolicy isolation (PR #14) ; livenessProbe on /healthz not /metrics (§ 22) ; chart README documents in-cluster-only scrape |

---

## 29. Tests strategy

### 29.1 Rust unit tests (~ 25 tests)

  - `metrics.rs::iterate_metrics_register` : assert every series is in `gather()` with HELP non-empty.
  - `metrics.rs::pretouch_closed_taxonomies` : after `new()`, `gather()` returns the full Cartesian product at 0.
  - `metrics.rs::pretouch_for_manifest` : after `pretouch_for_manifest(m)`, every participant_id has its per-id series at 0.
  - `metrics.rs::cardinality_breach` : 1 001-th label-value drops + breach counter increments.
  - `metrics.rs::pii_redaction` : "alice@example.com" → "<redacted>".
  - `metrics.rs::failure_reason_mapping` : every `ServerError` variant maps to a `FailureReason` value.
  - `metrics.rs::build_info` : `build_info{version=...,git_sha=...}` ⇒ 1.

### 29.2 Integration test : scrape roundtrip

```rust
#[tokio::test]
async fn pr17_scrape_returns_all_14_series() {
    let metrics = MetricsRegistry::new();
    metrics.pretouch_for_manifest(&test_manifest());
    let body = metrics.gather_text();  // returns the /metrics body
    for series_name in [
        "efe_meta_swarm_iterate_duration_seconds",
        "efe_meta_swarm_iterate_total",
        // ... all 14 ...
    ] {
        assert!(body.contains(series_name), "missing : {series_name}");
    }
}
```

### 29.3 Integration test : prometheus-parse validation

```rust
let parsed = prometheus_parse::Scrape::parse(body.lines().map(|l| Ok(l.to_string())))
    .expect("parse /metrics body");
assert_eq!(parsed.docs.len(), 14 + 9);  // 14 new + 9 existing
```

### 29.4 Helm-unittest assertions

5 new test files (one per alert) verifying :

  - The alert renders when `alerts.enabled + meta-swarm.enabled`.
  - The alert uses the configured threshold.
  - The alert's runbook_url field is non-empty.

### 29.5 Cargo bench

`benches/metric_hot_path.rs` — benchmark the cost of `inc()` on a labelled
counter in a tight loop. Asserts p99 < 50 ns on x86_64 + aarch64.

### 29.6 Cardinality bench

`benches/cardinality_breach.rs` — simulate a malicious label-value
stream ; assert the breaker engages at 1 000 series.

---

## 30. Performance budget (CPU + memory)

### 30.1 CPU

  - `IntCounter::inc()` : 1 atomic increment, ~ 2 ns on Apple M1.
  - `Histogram::observe()` : 14 buckets, ~ 30 ns on x86_64 (Prometheus-rs benches).
  - `pretouch_closed_taxonomies` : ~ 200 `inc_by(0)` calls × 5 ns = 1 µs at boot.

Total per-Iterate overhead : ~ 100 ns (1 histogram observe + 1 counter inc
+ 1 gauge inc/dec). Negligible at typical 100 RPS (10 µs per second).

### 30.2 Memory

  - Histogram : ~ 200 B + 8 B × bucket-count per series.
  - Counter : ~ 100 B per series.
  - Total memory for 543 active series : ~ 100 KB per pod.
  - `/metrics` body : ~ 80 KB exposition format for 543 series.

### 30.3 Scrape latency

The exporter scrapes the Prometheus registry's `gather()` method ; ~ 500 µs
per scrape on a fully-populated registry. Well within the 10 s scrape
timeout.

---

## 31. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Cardinality breach in customer cluster | Medium | High | CardinalityGuard ceiling (§ 18.1) ; alert at 80 % saturation |
| PII leakage via misconfigured participant_id | Low | High | Schema regex + runtime redaction + Rego lint + docs (§ 19) |
| Histogram bucket choice misalignment with customer's traffic shape | Medium | Medium | Pre-bench validation (§ 13.5) ; chart-version-frozen (AD16) |
| Stability contract violated by future PR | Medium | High | Stability tier in code comments + CI check (forward-look) |
| Performance regression from added metric overhead | Low | Low | Benchmark (§ 29.5) ; p99 < 50 ns per `inc()` |
| `/metrics` body > 4 MB cap | Low | Medium | Truncation + WARNING line + dedicated alert |
| Race between `participants_configured` set and pretouch_for_manifest call | Low | Low | Both wrap in `manifest.write()` lock |
| ServiceMonitor scrape lag during fleet rollout | Medium | Low | Jitter ± 1.5 s (AD12) ; scrape timeout 10 s |

---

## 32. Rollback strategy

  1. Set `meta-swarm.enabled: false` ⇒ entire sub-chart disappears, including all PR #17 series. No effect on other meta-swarm-using charts.
  2. Set `alerts.enabled: false` ⇒ alerts disappear but metrics stay.
  3. Set `meta-swarm.metrics.cardinalityCircuitBreaker.enabled: false` ⇒ disables the ceiling (NOT recommended in prod).
  4. Helm rollback to chart version 0.3.0 (PR #16) ⇒ removes PR #17 series entirely.

---

## 33. Sequence diagrams

### 33.1 Scrape lifecycle

```text
Prometheus       ServiceMonitor      Pod                 MetricsRegistry
   │                  │                │                       │
   ├ scrape tick (15s)│                │                       │
   ├ resolve target ──>                │                       │
   │                  │ select pod      │                       │
   │                  │  via labels    │                       │
   │                                   │                       │
   ├ GET /metrics ────────────────────>│                       │
   │                                   ├ exporter handler      │
   │                                   ├ acquire body buffer   │
   │                                   ├ gather() ────────────>│
   │                                   │                       ├ iterate all series
   │                                   │                       ├ encode text format
   │                                   │                       │  (200ns/series)
   │                                   │<───────body 80 KB────│
   │<───────────────body──────────────│                       │
   ├ parse + ingest                    │                       │
   ├ apply labels                      │                       │
   ├ store TSDB                        │                       │
   │                                   │                       │
   ├ evaluate recording                │                       │
   │  rules (60s interval)             │                       │
   │                                   │                       │
   ├ evaluate alert rules              │                       │
   │  (60s interval)                   │                       │
   ├ trigger if true                   │                       │
```

### 33.2 Cardinality breach

```text
Iterate handler         CardinalityGuard       IntCounterVec
   │                          │                      │
   ├ inc_by(result=ok,        │                      │
   │   failure_reason=newval) │                      │
   ├ check(...)──────────────>│                      │
   │                          ├ contains? No         │
   │                          ├ len < limit? Yes     │
   │                          ├ insert               │
   │<─ Ok ──────────────────│                      │
   ├ with_label_values ──────────────────────────────>│
   │                          │                      ├ inc atomic
   │                          │                      │
   │  ...after 1 000 unique labels seen...           │
   │                          │                      │
   ├ inc_by(result=ok,        │                      │
   │   failure_reason=1001st) │                      │
   ├ check(...)──────────────>│                      │
   │                          ├ contains? No         │
   │                          ├ len < limit? No      │
   │                          ├ set breached=true    │
   │                          ├ inc cardinality_     │
   │                          │   breach_total       │
   │<─ Err(breach)───────────│                      │
   ├ log error                │                      │
   ├ drop observation         │                      │
```

---

## 34. Reviewer checklist

  - [ ] All 14 series exposed by `/metrics`.
  - [ ] HELP non-empty for every series.
  - [ ] Pre-touch policy verified by `pretouch_for_manifest` test.
  - [ ] Cardinality ceiling test passes (1 001-th label-value drops).
  - [ ] PII redaction test passes (`john.smith@example.com` → `<redacted>`).
  - [ ] All 5 alerts render at correct thresholds.
  - [ ] ServiceMonitor jitter applied.
  - [ ] livenessProbe on `/healthz`, NOT `/metrics`.
  - [ ] All 35 ACs covered by code or test.
  - [ ] All 10 threats (T46-T55) have named mitigations.
  - [ ] Stability tier documented for every series.
  - [ ] Chart version bumped 0.3.0 → 0.4.0 + appVersion 0.4.0 → 0.5.0.
  - [ ] Goldens regenerated for `kovant-prod`.
  - [ ] No PII in HELP, label names, or label values.
  - [ ] `prometheus-parse` integration test asserts 23 series present.

---

## 35. Appendix A — exhaustive metric API surface

```rust
// pub re-exports from observability/metrics.rs

pub struct MetricsRegistry { ... }
impl MetricsRegistry {
    pub fn new() -> Self;
    pub fn pretouch_for_manifest(&self, manifest: &Manifest);
    pub fn gather_text(&self) -> String;  // for /metrics body
    pub fn registry(&self) -> &Registry;

    // Sub-struct accessors.
    pub fn iterate(&self) -> &IterateMetrics;
    pub fn step(&self) -> &StepMetrics;
    pub fn safety(&self) -> &SafetyMetrics;
    pub fn health(&self) -> &ParticipantHealthMetrics;
    pub fn manifest(&self) -> &ManifestReloadMetrics;
    pub fn build_info(&self) -> &BuildInfoMetric;
}

// Group A.
pub struct IterateMetrics {
    pub duration_seconds: HistogramVec,
    pub total: IntCounterVec,
    pub inflight: IntGauge,
    pub iterations: HistogramVec,
    pub convergence_error: HistogramVec,
}

// Group B.
pub struct StepMetrics {
    pub duration_seconds: HistogramVec,
    pub total: IntCounterVec,
    pub payload_bytes: HistogramVec,
}

// Group C.
pub struct SafetyMetrics {
    pub violations_total: IntCounterVec,
}

// Group D.
pub struct ParticipantHealthMetrics {
    pub participants_configured: IntGauge,
    pub participant_up: IntGaugeVec,
}

// Group E.
pub struct ManifestReloadMetrics {
    pub duration_seconds: Histogram,
    pub total: IntCounterVec,
}

// Group F.
pub struct BuildInfoMetric {
    pub info: IntGaugeVec,
}
```

---

## 36. Appendix B — exporter HTTP wire-format example

```text
$ curl -s http://localhost:9091/metrics

# HELP efe_meta_swarm_iterate_duration_seconds End-to-end Iterate gRPC call duration in seconds, partitioned by outcome and manifest version.
# TYPE efe_meta_swarm_iterate_duration_seconds histogram
efe_meta_swarm_iterate_duration_seconds_bucket{result="ok",manifest_version="v0.2.0",le="0.001"} 0
efe_meta_swarm_iterate_duration_seconds_bucket{result="ok",manifest_version="v0.2.0",le="0.005"} 0
...
efe_meta_swarm_iterate_duration_seconds_bucket{result="ok",manifest_version="v0.2.0",le="+Inf"} 145
efe_meta_swarm_iterate_duration_seconds_sum{result="ok",manifest_version="v0.2.0"} 12.45
efe_meta_swarm_iterate_duration_seconds_count{result="ok",manifest_version="v0.2.0"} 145

# HELP efe_meta_swarm_iterate_total Total Iterate calls completed, partitioned by outcome and failure reason.
# TYPE efe_meta_swarm_iterate_total counter
efe_meta_swarm_iterate_total{result="ok",failure_reason="other"} 145
efe_meta_swarm_iterate_total{result="error",failure_reason="dim_mismatch"} 0
efe_meta_swarm_iterate_total{result="error",failure_reason="timeout"} 3
...

# HELP efe_meta_swarm_build_info Build metadata gauge — value is always 1 ; the labels carry the actual information.
# TYPE efe_meta_swarm_build_info gauge
efe_meta_swarm_build_info{version="0.3.0",git_sha="abc1234",profile="release",rustc_version="1.88.0-stable",build_date="2026-07-27T14:32:11Z"} 1
```

---

## 37. Appendix C — Prometheus query-cookbook (PR #19 dashboard hand-off)

  - **Iterate p99 latency per manifest version** :
    ```promql
    histogram_quantile(0.99,
      sum by (le, manifest_version) (rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])))
    ```
  - **Iterate error rate (excluding throttle)** :
    ```promql
    sum(rate(efe_meta_swarm_iterate_total{result="error"}[5m])) /
    sum(rate(efe_meta_swarm_iterate_total{result!="throttled"}[5m]))
    ```
  - **Per-participant Step latency comparison** :
    ```promql
    histogram_quantile(0.99,
      sum by (le, participant_id) (rate(efe_meta_swarm_step_duration_seconds_bucket[5m])))
    ```
  - **Convergence-error distribution** :
    ```promql
    histogram_quantile(0.5, rate(efe_meta_swarm_iterate_convergence_error_bucket{convergence="converged"}[5m]))
    ```
  - **Pod inventory by build version** :
    ```promql
    count by (version) (efe_meta_swarm_build_info)
    ```

---

## 38. Appendix D — cardinality benchmarking methodology

  - Run `criterion` bench `cardinality_pressure.rs` :
    - Spawn 1 000 unique label values across `iterate_total`.
    - Measure : `MetricsRegistry::gather_text()` p99 latency.
    - Pass criterion : p99 < 1 ms for the 1 000-series body.
  - Run `criterion` bench `breach_recovery.rs` :
    - Trigger a breach via 1 001-th label-value.
    - Verify the breach counter increments.
    - Verify subsequent `with_label_values` calls return cleanly.

---

## 39. Appendix E — OpenMetrics 1.0 compliance (RFC-track)

OpenMetrics 1.0 is an IETF RFC-track exposition format superset of
Prometheus text format. Differences :

  - `_total` suffix is REQUIRED for counters (we already follow).
  - HELP and TYPE comments order matters (we follow).
  - EOF line `# EOF` required (Prometheus text format optional).
  - Allows `# UNIT` line (we don't emit but compliant readers accept).

Forward-compat goal : add `# EOF` line in PR #18 ; the wire format will
remain Prometheus-text-compatible.

---

## 40. Appendix F — comparison against open-source meta-swarm precedents

| Project | Equivalent metric | Notes |
|---|---|---|
| Ray Serve | `serve_handle_request_counter` | Counts but doesn't bucket latency. EFE supersedes with histogram. |
| vLLM | `vllm_iteration_duration_seconds` | Single LLM iteration vs EFE's multi-participant Iterate. |
| Kubernetes | `apiserver_request_duration_seconds` | Histogram bucket policy similar to our Iterate buckets. |
| Istio | `istio_request_duration_milliseconds` | Histogram in ms ; we prefer `_seconds` per Prometheus convention. |

EFE's contribution beyond these : explicit `convergence` + `safety_kind`
+ `participant_id` labels for the iteration-engine domain.

---

## 41. Appendix G — operator runbook : `MetricsCardinalityBreach`

### When

`MetricsCardinalityBreach` alert fires : `efe_meta_swarm_cardinality_
breach_total > 0`.

### Triage steps

  1. `kubectl top pods -n efe -l app.kubernetes.io/part-of=efe-meta-swarm`
     ⇒ pod memory ; if any pod > 2 GB, suspected cardinality breach.
  2. Curl `/metrics` and count unique series : `curl ... | grep -c '^efe_meta_swarm_' `.
  3. Inspect breach counter labels for the offending metric name.
  4. Read the offending metric's recent label values from
     `/metrics`.  Identify the runaway label.

### Resolution

  - If the runaway is `participant_id` : audit the manifest for
    duplicate or hostile IDs.
  - If the runaway is `failure_reason` : audit the codebase for a
    label-value not in the closed taxonomy (AD5).
  - If neither : escalate to platform team — possible code regression.

### Recovery

  - Restart the pod (Kubernetes-managed via `kubectl rollout restart`).
  - The CardinalityGuard's `seen_labels` map resets ; the breach counter
    stays at its pre-restart value (won't re-fire until a new breach).

---

## End of architecture document — PR #17

Architecture frozen. 10-commit plan in § 3. Migration path § 25. Coexists
with PR #12 (4 reload series) + PR #16 (5 TLS series) by adding 14 NEW
series — total 23. No existing series renamed, removed, or relabelled.
35 acceptance criteria. 10 threats named (T46-T55). Stability tier
documented per series. Cardinality budget enforced at 1 000 active series
per pod.
