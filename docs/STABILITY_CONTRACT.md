# EFE Meta-Swarm — Stability contract

| Field | Value |
|---|---|
| **Version** | v1.1 (chart version 0.6.0+, PR #19) |
| **Owner** | EFE platform team — observability area |
| **Scope** | Metric series + label taxonomies (v1.0) + **Dashboard UIDs, Alert names, Recording rule names, SLO targets** (v1.1, PR #19 § 37) |
| **References** | `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 24 ; `docs/EFE_A1_PHASE2_PR19_ARCHITECTURE.md` § 37 |
| **Effective from** | v1.0 : chart 0.4.0 (PR #17). v1.1 : chart 0.6.0 (PR #19) |

This document is the **public API contract** for the meta-swarm
Prometheus metric surface + the v1.1 operator surface (dashboards,
alerts, SLOs). Dashboards, alerts, SLO calculators, paging
integrations, compliance auditors and external dashboards that
consume these series may rely on the guarantees below.

---

## 1. Stability tiers

| Tier | Promise | Breaking-change notice |
|---|---|---|
| **STABLE** | Series name, label set, and label value taxonomies are frozen across at least 6 chart-minor versions. | 6-minor-version deprecation notice (in `HELP` prefix + chart `artifacthub.io/changes`) ; then removal |
| **BETA** | Series may change in the next chart-minor version. | 1-minor-version deprecation notice |
| **EXPERIMENTAL** | Series may change without notice. | None |

Any change to a STABLE series that breaks the contract (rename, label removal, bucket change, unit change) requires a chart **major** version bump (1.0.0 → 2.0.0).

---

## 2. Series tiering — at chart version 0.4.0 (PR #17)

### 2.1 STABLE

These series will not change incompatibly before chart v1.0 + 6 minors.

| Series | Type | Labels | Source |
|---|---|---|---|
| `efe_meta_swarm_iterate_duration_seconds` | HistogramVec | `result`, `manifest_version` | PR #17 § 6.1 |
| `efe_meta_swarm_iterate_total` | IntCounterVec | `result`, `failure_reason` | PR #17 § 6.2 |
| `efe_meta_swarm_iterate_inflight` | IntGauge | — | PR #17 § 6.3 |
| `efe_meta_swarm_step_duration_seconds` | HistogramVec | `participant_id`, `result` | PR #17 § 7.1 |
| `efe_meta_swarm_step_total` | IntCounterVec | `participant_id`, `result`, `failure_reason` | PR #17 § 7.2 |
| `efe_meta_swarm_step_payload_bytes` | HistogramVec | `direction` | PR #17 § 7.3 |
| `efe_meta_swarm_safety_violations_total` | IntCounterVec | `kind`, `participant_id` | PR #17 § 8.1 |
| `efe_meta_swarm_participants_configured` | IntGauge | — | PR #17 § 9.1 |
| `efe_meta_swarm_participant_up` | IntGaugeVec | `participant_id` | PR #17 § 9.2 |
| `efe_meta_swarm_manifest_reload_duration_seconds` | Histogram | — | PR #17 § 10.1 |
| `efe_meta_swarm_manifest_reload_total` | IntCounterVec | `result`, `trigger` | PR #17 § 10.2 |
| `efe_meta_swarm_build_info` | IntGaugeVec | `version`, `git_sha`, `profile`, `rustc_version`, `build_date` | PR #17 § 11.1 |
| `efe_meta_swarm_reload_attempts_total` | IntCounter | — | PR #12 |
| `efe_meta_swarm_reload_failures_total` | IntCounterVec | `reason` | PR #12 |
| `efe_meta_swarm_reload_duration_microseconds` | Histogram | — | PR #12 |
| `efe_meta_swarm_reload_gen` | Gauge | — | PR #12 |
| `efe_meta_swarm_tls_handshake_duration_seconds` | HistogramVec | `result`, `protocol`, `role` | PR #16 § 20 |
| `efe_meta_swarm_tls_handshake_total` | IntCounterVec | `result`, `role`, `failure_reason` | PR #16 § 20 |
| `efe_meta_swarm_tls_cert_expiry_seconds` | GaugeVec | `cert_name`, `subject_cn` | PR #16 § 20 |
| `efe_meta_swarm_tls_reload_total` | IntCounterVec | `result` | PR #16 § 20 |
| `efe_meta_swarm_tls_reload_at_seconds` | GaugeVec | `role` | PR #16 § 20 |
| `efe_meta_swarm_cardinality_breach_total` | IntCounterVec | `metric` | PR #17 § 18.3 |

### 2.2 BETA

May change in chart v0.5+ with one-version notice.

| Series | Type | Labels | Reason for BETA |
|---|---|---|---|
| `efe_meta_swarm_iterate_iterations` | HistogramVec | `convergence` | Bucket policy may shift as Phase-3 algorithm lands |
| `efe_meta_swarm_iterate_convergence_error` | HistogramVec | `convergence` | Same |

### 2.3 EXPERIMENTAL

None at chart v0.4.0.

---

## 3. Closed-label-value taxonomies

These taxonomies are part of the contract.  Adding a value to any
enum below is **additive** (non-breaking).  Removing a value is
**breaking** and requires a 1-minor-version deprecation notice.

### 3.1 `result` (Outcome)

```
ok | error | deadline | canceled | throttled
```

### 3.2 `failure_reason`

```
timeout | dim_mismatch | safety_violation | participant_down |
internal | deadline_exceeded | cancelled | other
```

### 3.3 `convergence`

```
converged | max_iters | diverged | canceled
```

### 3.4 `kind` (safety)

```
box | poly | hull
```

### 3.5 `direction` (payload)

```
in | out
```

### 3.6 `trigger` (manifest reload)

```
sighup | grpc | file_change
```

### 3.7 `role` (TLS / metrics)

```
server | client
```

---

## 4. Breaking-change matrix

| Change | STABLE | BETA | EXPERIMENTAL |
|---|---|---|---|
| Rename metric | ❌ requires major bump | ⚠️ 1-minor notice | ✅ anytime |
| Add label | ✅ additive | ✅ | ✅ |
| Remove label | ❌ requires major bump | ⚠️ 1-minor notice | ✅ anytime |
| Add bucket | ❌ (changes p99 estimation contract) | ✅ | ✅ |
| Remove bucket | ❌ requires major bump | ⚠️ 1-minor notice | ✅ anytime |
| Change HELP | ✅ cosmetic | ✅ | ✅ |
| Change unit | ❌ requires major bump | ❌ requires minor bump | ✅ anytime |
| Tier change STABLE → BETA | ❌ requires 6-minor notice | n/a | n/a |
| Tier change BETA → STABLE | n/a | ✅ promotion | n/a |

---

## 5. Deprecation procedure

When deprecating a STABLE series :

1. **Chart version N** — the new replacement series is added (additive).
2. **Chart version N + 0** — the deprecated series's HELP string is prefixed with `[DEPRECATED in vX.Y — replaced by `<new-series>`. Will be removed in v<N+6>]`.
3. The deprecation lands in `artifacthub.io/changes` of the same Chart.yaml release with `kind: deprecated`.
4. A migration runbook entry is added under `docs/runbooks/` named `MIGRATION_<old-series>_to_<new-series>.md`.
5. **Chart version N + 6** — the deprecated series is removed.
6. The removal lands in `artifacthub.io/changes` with `kind: removed`.

---

## 6. Pre-touch policy guarantee

Every series with a closed-taxonomy label set is **pre-touched at boot**
so Prometheus's `gather()` emits the (label-value)-tuple with value `0`
before any observation occurs.  Dashboards that filter on a known
label value will render `0` instead of "no data".

The pre-touch list per series is :

| Series | Pre-touch cardinality |
|---|---|
| `iterate_total` | 5 × 8 = 40 |
| `iterate_iterations` | 4 |
| `iterate_convergence_error` | 4 |
| `step_total` | N × 5 × 8 (per participant) + 5 × 8 sentinel |
| `step_duration_seconds` | N × 5 + 5 sentinel |
| `step_payload_bytes` | 2 |
| `safety_violations_total` | 3 × N + 3 sentinel |
| `manifest_reload_total` | 2 × 3 = 6 |
| `participant_up` | N + 1 sentinel |
| `cardinality_breach_total` | 1 (`"(none)"` sentinel) |

`N` = the manifest's `participants.len()` (bounded by schema at ≤ 10).

---

## 7. SLO derivation contract

Forward-pointing : PR #22 derives 4 SLOs from this surface.  Any
change to the series in § 2.1 that breaks one of those SLOs must be
coordinated with the SRE team via the platform-SLO review.

| SLO | Reads from | Promise |
|---|---|---|
| SLO-ITER-LATENCY | `iterate_duration_seconds` | p99 < 5 s |
| SLO-ITER-ERROR-RATE | `iterate_total` | < 0.1 % error rate over 30 days |
| SLO-STEP-LATENCY | `step_duration_seconds` | p99 < 100 ms per participant |
| SLO-PARTICIPANT-UP | `participant_up` | ≥ 99.9 % minutes over 30 days |

---

## 8. PR #18 — OpenTelemetry span stability contract (chart 0.6.0)

PR #18 adds **8 OpenTelemetry span types** to the public observability
surface.  Span names, span attributes, and attribute value taxonomies
are subject to the same stability tier rules as the Prometheus metric
surface in §§ 1-3 above.

### 8.1 STABLE span types

The following span names are **STABLE** at chart 0.6.0 :

| Span name | Kind | Tier | First emitted by |
|---|---|---|---|
| `efe.meta_swarm.iterate` | SERVER | STABLE | Controller `Iterate` RPC handler (root span) |
| `efe.meta_swarm.connect_participants` | INTERNAL | STABLE | Controller `iterate.rs::connect_participants` |
| `efe.meta_swarm.participant.connect` | CLIENT | STABLE | Controller per-participant `RemoteParticipant::connect` |
| `efe.meta_swarm.inner_loop` | INTERNAL | STABLE | Controller `iterate.rs::inner_loop` spawn_blocking wrapper |
| `efe.meta_swarm.safety_check` | INTERNAL | STABLE | Controller `iterate.rs` post-stop attribution |
| `efe.meta_swarm.step` | SERVER | STABLE | Participant `Step` RPC handler (root span) |
| `efe.meta_swarm.step.local` | INTERNAL | STABLE | Participant `service.rs::step` spawn_blocking wrapper |
| `efe.meta_swarm.manifest.reload` | INTERNAL | STABLE | Controller + participant manifest reload handler |
| `efe.meta_swarm.shutdown.drain` | INTERNAL | STABLE | Controller graceful shutdown drain window |

### 8.2 Span attribute stability

| Span name | STABLE attribute (creation-time) | STABLE attribute (close-time) |
|---|---|---|
| `efe.meta_swarm.iterate` | `rpc.system`, `rpc.service`, `rpc.method`, `meta_swarm.manifest.version`, `meta_swarm.iterate.max_iters`, `meta_swarm.iterate.tolerance`, `meta_swarm.iterate.alpha_default`, `meta_swarm.iterate.coupling_gamma`, `meta_swarm.iterate.deterministic`, `meta_swarm.iterate.participants_count`, `meta_swarm.discovery_mode`, `meta_swarm.tls.enabled` | `meta_swarm.iterate.iterations`, `meta_swarm.iterate.final_residual`, `meta_swarm.iterate.stop_reason`, `meta_swarm.iterate.wall_us`, `meta_swarm.iterate.outcome`, `meta_swarm.iterate.failure_reason` |
| `efe.meta_swarm.connect_participants` | `meta_swarm.participants.count`, `meta_swarm.discovery_mode`, `meta_swarm.tls.enabled`, `meta_swarm.tls.mutual` | — |
| `efe.meta_swarm.participant.connect` | `rpc.system`, `rpc.service`, `rpc.method`, `meta_swarm.participant_id`, `meta_swarm.participant.endpoint`, `meta_swarm.tls.expected_san`, `meta_swarm.connect.attempt_count` | `meta_swarm.connect.elapsed_us` |
| `efe.meta_swarm.inner_loop` | `meta_swarm.inner_loop.max_iters`, `meta_swarm.inner_loop.tolerance` | `meta_swarm.inner_loop.iteration_count`, `meta_swarm.inner_loop.final_residual`, `meta_swarm.inner_loop.convergence`, `meta_swarm.inner_loop.elapsed_us` |
| `efe.meta_swarm.safety_check` | `meta_swarm.safety.kind`, `meta_swarm.safety.participant_id`, `meta_swarm.safety.violated` | `meta_swarm.safety.adjustment_norm` |
| `efe.meta_swarm.step` | `rpc.system`, `rpc.service`, `rpc.method`, `meta_swarm.participant_id`, `meta_swarm.step.step_index`, `meta_swarm.step.dim`, `meta_swarm.step.payload_in_bytes` | `meta_swarm.step.payload_out_bytes`, `meta_swarm.step.local_residual`, `meta_swarm.step.elapsed_us`, `meta_swarm.step.outcome`, `meta_swarm.step.failure_reason` |
| `efe.meta_swarm.step.local` | `meta_swarm.step.local.gamma`, `meta_swarm.step.local.alpha`, `meta_swarm.step.local.dim`, `meta_swarm.step.local.kappa` | `meta_swarm.step.local.elapsed_us`, `meta_swarm.step.local.outcome`, `meta_swarm.step.local.failure_reason` |
| `efe.meta_swarm.manifest.reload` | `meta_swarm.reload.trigger`, `meta_swarm.reload.gen_before`, `meta_swarm.reload.participants_before` | `meta_swarm.reload.gen_after`, `meta_swarm.reload.participants_after`, `meta_swarm.reload.outcome`, `meta_swarm.reload.failure_reason`, `meta_swarm.reload.changes_count`, `meta_swarm.reload.duration_seconds` |
| `efe.meta_swarm.shutdown.drain` | `meta_swarm.shutdown.signal`, `meta_swarm.shutdown.drain_deadline_seconds`, `meta_swarm.shutdown.inflight_at_start` | `meta_swarm.shutdown.inflight_at_end`, `meta_swarm.shutdown.elapsed_seconds`, `meta_swarm.shutdown.outcome` |

### 8.3 Span attribute value taxonomies (closed)

These string-valued attributes carry **closed** taxonomies ; adding a
new value is a STABLE-tier change requiring 6-minor-version notice.

| Attribute | Closed values |
|---|---|
| `meta_swarm.iterate.outcome` | `ok`, `error`, `deadline`, `throttled` |
| `meta_swarm.iterate.failure_reason` | `dim_mismatch`, `non_contraction`, `cancelled`, `internal`, `unhealthy`, `participant`, `other` |
| `meta_swarm.iterate.stop_reason` | `converged`, `max_iters_reached`, `error`, `cancelled` |
| `meta_swarm.inner_loop.convergence` | `converged`, `max_iters_reached`, `error` |
| `meta_swarm.step.outcome` | `ok`, `error` |
| `meta_swarm.step.failure_reason` | `dim_mismatch`, `internal`, `panicked`, `other` |
| `meta_swarm.step.local.outcome` | `ok`, `error` |
| `meta_swarm.step.local.failure_reason` | `internal`, `panicked` |
| `meta_swarm.safety.kind` | `box`, `cosine`, `disabled` |
| `meta_swarm.reload.trigger` | `sighup`, `grpc`, `file_change` |
| `meta_swarm.reload.outcome` | `ok`, `error` |
| `meta_swarm.shutdown.signal` | `sigterm`, `sigint` |
| `meta_swarm.shutdown.outcome` | `clean`, `deadline` |
| `otel.kind` | `SERVER`, `CLIENT`, `INTERNAL` |
| `rpc.system` | `grpc` |

### 8.4 Cardinality budget

Per-span attribute cardinality is hard-capped at **200** attributes
(AD12 / AC19 / T64).  Attributes beyond this budget are dropped by
the OTel-SDK + `dropped_attributes_count` is non-zero on the SpanData.
The 200-attribute ceiling is wired via
`SdkTracerProvider::builder().with_max_attributes_per_span(200)` in
`crate::tracing_otel::init_otel`.

### 8.5 PII redaction (AC18 / T57)

Every user-controlled string attribute is routed through the PR #17
PII redactor (`crate::observability::metrics::cardinality::redact_pii`)
before recording.  Matched patterns (emails, JWTs, AWS access keys,
hex-32, UUIDs) are replaced by `<redacted>`.  Manifest-validated
DNS-1123 identifiers pass through unchanged.

### 8.6 Span propagation

Every cross-process span boundary uses **W3C Trace Context**
propagation (`traceparent` + `tracestate` gRPC metadata headers).  The
canonical extractor / injector pair is :

* Server-side : `ServerTraceInterceptor` (tonic `Interceptor`) —
  extracts `traceparent` from inbound `MetadataMap` and binds the
  parent context to the current tracing span.
* Client-side : per-request `inject_current_context(&propagator,
  request.metadata_mut())` — adds `traceparent` to outbound metadata.

Both are STABLE at chart 0.6.0.

### 8.7 PR #18 — Prometheus metrics complement

The 3 OTLP exporter health metrics introduced by PR #18 carry a
**STABLE** tier at chart 0.6.0 :

| Series | Type | Labels | Tier |
|---|---|---|---|
| `efe_meta_swarm_otel_exported_spans_total` | Counter | — | STABLE |
| `efe_meta_swarm_otel_dropped_spans_total` | CounterVec | `reason` ∈ {`queue_full`, `export_error`, `shutdown`, `timeout`} | STABLE |
| `efe_meta_swarm_otel_export_duration_seconds` | Histogram | 11 buckets, 1 ms → 5 s | STABLE |

Total `/metrics` surface after PR #18 : **26** = 4 (PR #12) + 5 (PR #16) + 14 (PR #17) + 3 (PR #18).

### 8.8 BatchSpanProcessor configuration (AD9 / AC15)

The OTel SDK's `BatchSpanProcessor` is configured with explicit knobs
that supersede the SDK defaults :

| Knob | PR #18 value | SDK default |
|---|---|---|
| `max_export_batch_size` | 1024 | 512 |
| `max_queue_size` | 8192 | 2048 |
| `scheduled_delay` | 5 s | 5 s |
| `max_export_timeout` | 5 s | 30 s |

These are **STABLE** at chart 0.6.0 ; operators can override per-pod
via the standard `OTEL_BSP_*` env vars (the SDK respects those after
the chart's defaults).

---

## 9. PR #19 — Operator surface stability contract v1.1 (chart 0.6.0+)

PR #19 adds 4 new stability tiers covering the *operator-facing surface* :
dashboards, alerts, recording rules, SLO targets. These are not metric
series — they are the **contracted artifacts** that consumers of the
metric surface depend on (PagerDuty rule mappings, Slack triage filters,
external dashboards that link to ours by UID, audit trails that reference
alert names).

### 9.1 New stability tiers — Dashboard UID / Alert name / Recording rule name / SLO target

| Tier | Promise | Breaking-change notice |
|---|---|---|
| **Dashboard UID** | UID is stable for at least 6 chart-minor versions. | 6-minor-version notice + chart `artifacthub.io/changes` |
| **Alert name** | Alert name + the 4 critical labels (`service`, `team`, `severity`, `runbook_id`) are frozen. | 6-minor-version notice |
| **Recording rule name** | Series name + sum-by labels are frozen. | 6-minor-version notice |
| **SLO target** | Target value never drops (relaxes). Raising a target is non-breaking. | 6-minor-version notice for relaxation only |

### 9.2 Frozen surface — Dashboard UIDs

These 9 UIDs are stable across at least 6 chart-minor versions :

| UID | Title | Source |
|---|---|---|
| `efe-msw-overview`    | EFE Meta-Swarm — Overview & SLO        | PR #19 § 9  |
| `efe-msw-iterate`     | EFE Meta-Swarm — Iterate Performance   | PR #19 § 10 |
| `efe-msw-participant` | EFE Meta-Swarm — Participant Drill-down| PR #19 § 11 |
| `efe-msw-reload`      | EFE Meta-Swarm — Manifest Reload Audit | PR #19 § 12 |
| `efe-msw-otel`        | EFE Meta-Swarm — OTel Pipeline         | PR #19 § 13 |
| `efe-msw-tls`         | EFE Meta-Swarm — TLS & Cert Rotation   | PR #19 § 14 |
| `efe-msw-safety`      | EFE Meta-Swarm — Safety Violations     | PR #19 § 15 |
| `efe-msw-runtime`     | EFE Meta-Swarm — Runtime / Process     | PR #19 § 16 |
| `efe-msw-traces`      | EFE Meta-Swarm — Trace Explorer        | PR #19 § 17 |

External dashboards may permalink to these by UID without fear of breakage.

### 9.3 Frozen surface — Alert names

These 10 alert names are stable. PagerDuty service-key rules, Slack triage
filters, JIRA filter URLs, and compliance audit lookups all reference these
names ; renaming requires a chart major bump.

| Alert | Severity | SLO | Runbook | Source |
|---|---|---|---|---|
| `IterateFailureRateHigh`    | critical | — (symptom)              | MSW1  | PR #19 § 22 |
| `IterateLatencyP99Critical` | critical | iterate-latency-p95       | MSW2  | PR #19 § 23 |
| `ParticipantDown`           | warning  | —                        | MSW3  | PR #19 § 24 |
| `TLSCertExpiringSoon`       | warning  | —                        | MSW4  | PR #19 § 25 |
| `ControllerNotServing`      | critical | — (platform)             | MSW5  | PR #19 § 26 |
| `OTelExporterFailing`       | warning  | otel-drop-rate           | MSW6  | PR #19 § 27 |
| `SafetyViolationSpike`      | warning  | —                        | MSW7  | PR #19 § 28 |
| `ManifestReloadFailureSpike`| warning  | manifest-reload-latency  | MSW8  | PR #19 § 29 |
| `SLOBudgetBurnFast14x`      | critical | iterate-availability     | MSW9  | PR #19 § 30 |
| `SLOBudgetBurnSlow6x`       | warning  | iterate-availability     | MSW10 | PR #19 § 31 |

### 9.4 Frozen surface — Recording rule names

These 6 recording rules are stable. External dashboards consuming them
get the same 6-minor-version guarantee as the underlying metrics.

| Series | Cardinality | Source query |
|---|---|---|
| `efe_msw:iterate_total:rate5m`        | ≤ 6 (sum by result)             | `sum by (result) (rate(efe_meta_swarm_iterate_total[5m]))` |
| `efe_msw:iterate_total:rate30m`       | ≤ 6                             | `sum by (result) (rate(efe_meta_swarm_iterate_total[30m]))` |
| `efe_msw:iterate_total:rate1h`        | ≤ 6                             | `sum by (result) (rate(efe_meta_swarm_iterate_total[1h]))` |
| `efe_msw:iterate_total:rate6h`        | ≤ 6                             | `sum by (result) (rate(efe_meta_swarm_iterate_total[6h]))` |
| `efe_msw:iterate_duration:p95_5m`     | ≤ 5 (per manifest_version)      | `histogram_quantile(0.95, sum by (manifest_version, le) (rate(...)))` |
| `efe_msw:iterate_duration:p99_5m`     | ≤ 5                             | `histogram_quantile(0.99, sum by (manifest_version, le) (rate(...)))` |

Total cardinality : ≤ 36 series — caps AD16 + T73 cardinality budget.

### 9.5 Frozen surface — SLO targets

The 4 default SLO targets below are committed. **Lowering a target
(relaxation) is a contract break ; raising a target is non-breaking.**

| SLO id | Target | Window | Source |
|---|---|---|---|
| `iterate-availability`         | **99.9 %**            | 30d | PR #19 § 32.1 |
| `iterate-latency-p95`          | **95 % ≤ 1 s**        | 30d | PR #19 § 32.2 |
| `manifest-reload-latency-p95`  | **95 % ≤ 500 ms**     | 30d | PR #19 § 32.3 |
| `otel-drop-rate`               | **≥ 99 % delivered**  | 30d | PR #19 § 32.4 |

Operators may override per-deployment via `.Values.alerts.pr19.slo.*` —
chart-level overrides count as local tuning, NOT contract relaxation
(the chart default is what's committed).

### 9.6 Alert label taxonomy (new in v1.1)

The 4 alert labels below have closed taxonomies. Renaming is a contract
break ; adding a new value is additive (non-breaking).

#### 9.6.1 `severity`

```
critical | warning | info
```

#### 9.6.2 `service`

```
meta-swarm
```

Single value. Reserved for future fan-out (e.g. `meta-swarm-control-plane`
vs `meta-swarm-data-plane`) — additive in v1.x.

#### 9.6.3 `team`

```
meta-swarm-sre
```

Single value. Owns paging integration.

#### 9.6.4 `runbook_id`

```
MSW1 | MSW2 | MSW3 | MSW4 | MSW5 | MSW6 | MSW7 | MSW8 | MSW9 | MSW10
```

Closed for PR #19 ; new entries follow `MSW<N+1>` convention.

#### 9.6.5 `burn_rate` (SLO burn alerts only)

```
fast | slow
```

#### 9.6.6 `slo_id`

```
iterate-availability | iterate-latency-p95 | manifest-reload-latency-p95 | otel-drop-rate
```

---

## 10. PR #20 — Loadgen self-metric tier (v1.2)

PR #20 introduces a new STABLE tier covering the `efe-meta-swarm-loadgen`
binary's self-instrumentation surface.  External dashboards / alerts
that consume loadgen metrics get the same 6-minor-version guarantee
as the server-side metrics.

### 10.1 New tier — Loadgen self-metric

| Tier | Promise | Breaking-change notice |
|---|---|---|
| **Loadgen self-metric** | `efe_msw_loadgen_*` series name + label set + closed taxonomies are frozen for ≥ 6 chart-minor versions. | 6-minor-version notice |
| **Profile YAML schema** | The workload-profile YAML `version` discriminator + structural shape per version are frozen. | New `version` value for breaking changes |

### 10.2 Frozen surface — 5 STABLE loadgen metrics

These series are emitted on the `:9095/metrics` endpoint of every
loadgen pod.

| Series | Type | Labels | Source |
|---|---|---|---|
| `efe_msw_loadgen_request_total`            | Counter   | `verb`, `result`, `target`             | PR #20 § 14 |
| `efe_msw_loadgen_request_duration_seconds` | Histogram | `verb`, `result`                       | PR #20 § 14 |
| `efe_msw_loadgen_inflight`                 | Gauge     | `verb`                                 | PR #20 § 14 |
| `efe_msw_loadgen_target_up`                | Gauge     | `target`                               | PR #20 § 14 |
| `efe_msw_loadgen_build_info`               | Gauge     | `version`, `git_sha`, `rustc_version`, `build_date` | PR #20 § 14 |

### 10.3 Closed label-value taxonomies

#### `verb` (loadgen-side workload classifier)

```
iterate | step | reset | state
```

Inherits the `result` and `failure_reason` taxonomies from
STABILITY_CONTRACT v1.0 § 3.

#### `target` (URL or hostname)

Free-form string — operators may opt out of `target` cardinality
by passing `--prom-target-label=false` (Phase-3).

### 10.4 Frozen surface — Workload profile YAML schema

`version: 1` is committed for ≥ 6 chart-minor versions.  Future
schema bumps emit `version: 2` with a migration helper in the
`compare` subcommand.

### 10.5 Frozen surface — CLI exit codes

| Exit code | Meaning | Source |
|---|---|---|
| 0 | success — all SLOs met | PR #20 AD17 |
| 1 | SLO breach — loadgen completed but a target SLO failed | PR #20 AD17 |
| 2 | transport error — controller unreachable, mTLS handshake | PR #20 AD17 |
| 3 | invalid config — CLI flag conflict, profile YAML malformed | PR #20 AD17 |
| 4 | signal-interrupted — SIGINT/SIGTERM during a run | PR #20 AD17 |
| 5 | internal error — panic, OOM, unforeseen | PR #20 AD17 |

CI scripts and PagerDuty integrations rely on these values being stable.

### 10.6 Frozen surface — `--mode` taxonomy

```
dev | staging | production
```

Adding a new mode is non-breaking (additive).  Renaming is a contract
break.

---

## 11. PR #21 — Public-bench harness tier (v1.3)

PR #21 introduces 4 new STABLE tiers covering the public benchmark
surface :

### 11.1 New tiers

| Tier | Promise | Notice |
|---|---|---|
| **Scenario ID** | `Sxx_<name>` is frozen for ≥ 6 chart-minor versions | 6-minor |
| **Baseline tolerance** | Default per-metric tolerance per scenario frozen | 6-minor |
| **Result schema version** | `summary.json` / baseline JSON schema v1 frozen | 6-minor |
| **Comparison metric set** | Closed taxonomy of 6 metrics | 6-minor (adding additive) |

### 11.2 Frozen scenario IDs (10)

```
S01_baseline_100qps_5min
S02_canary_1000qps_15min
S03_ramp_to_breakdown
S04_chaos_participant_loss
S05_chaos_network_partition
S06_chaos_cert_expiry
S07_chaos_reload_storm
S08_multi_region_replay
S09_soak_2h_stability
S10_slo_validation_30min
```

### 11.3 Frozen comparison metric set

```
p50_ms | p95_ms | p99_ms | error_rate | throughput_qps | recovery_time_s
```

Direction-aware (latency / error-rate lower-is-better ; throughput
higher-is-better).

### 11.4 Frozen result schema (`bench/schemas/baseline-v1.json`)

Top-level required fields : `schema_version`, `scenario_id`, `metrics`.
Closed metric keys above.  Adding new optional top-level fields is
additive.

### 11.5 Frozen verdict taxonomy

```
pass | warn | fail | no-baseline
```

### 11.6 Frozen exit codes (harness binary)

| Code | Meaning |
|---|---|
| 0 | success — all scenarios pass |
| 1 | bench fail — at least one verdict=fail |
| 2 | cluster error |
| 3 | config error |
| 4 | signal-interrupted |
| 5 | internal |

### 11.7 Frozen default tolerances

Closed defaults committed in [`bench/harness/src/lib.rs`](../bench/harness/src/lib.rs)
(`tolerance_defaults` module).  Per-scenario override allowed via
`tolerance:` YAML block ; relaxing a default is a chart-major break.

---

## 12. Audit trail

| Date | Change | Chart version | Contract version |
|---|---|---|---|
| 2026-07-27 | Initial contract (PR #17 merge) — 14 STABLE + 2 BETA series | 0.4.0 | v1.0 |
| 2026-05-15 | PR #18 — 8 STABLE OTel span types + 3 STABLE OTel exporter health metrics | 0.5.0 | v1.0 |
| 2026-05-16 | PR #19 — 4 new operator-surface tiers (Dashboard UID, Alert name, Recording rule name, SLO target) + 9 UIDs + 10 alert names + 6 recording rules + 4 SLO targets committed | 0.6.0 | **v1.1** |
| 2026-05-16 | PR #20 — Loadgen self-metric tier + Profile YAML schema v1 + CLI exit codes 0..5 + `--mode` taxonomy committed.  5 STABLE `efe_msw_loadgen_*` series. | 0.6.0 (loadgen chart 0.1.0) | **v1.2** |
| 2026-05-16 | PR #21 — Public-bench tier : 10 scenario IDs + 6-metric closed taxonomy + baseline JSON schema v1 + verdict taxonomy + harness exit codes 0..5 + default tolerances + 6 chaos faults committed. | 0.6.0 (harness 0.1.0) | **v1.3** |


---

## 22. SLO surface — frozen v1.4 (PR #22)

Frozen contracts for the EFE 4-SLO rollup surface ('efe-slo-rollup'
crate at 'slos/rollup/').  Reference :
[`docs/EFE_A1_PHASE2_PR22_ARCHITECTURE.md`](EFE_A1_PHASE2_PR22_ARCHITECTURE.md).

### 22.1  4 SLO IDs (closed taxonomy)

```
L2_AVAILABILITY
L2_LATENCY_P95
SWARM_RECOVERY_TIME
REPLICATION_LAG_P99
```

### 22.2  Frozen numerical thresholds

| Constant | Value |
|---|---|
| `L2_AVAILABILITY_TARGET` | 0.995 |
| `L2_LATENCY_P95_TARGET_S` | 0.050 |
| `L2_LATENCY_P95_COMPLIANCE` | 0.99 |
| `SWARM_RECOVERY_TIME_TARGET_S` | 60.0 |
| `SWARM_RECOVERY_TIME_COMPLIANCE` | 0.99 |
| `REPLICATION_LAG_TARGET` | 100 |
| `REPLICATION_LAG_COMPLIANCE` | 0.99 |
| `BURN_RATE_FAST` | 14.4 |
| `BURN_RATE_SLOW` | 6.0 |

Locked by 'slos/rollup/tests/stability_contract_v14.rs' (frozen-test suite).

### 22.3  8-tier window taxonomy

`1m / 5m / 30m / 1h / 6h / 24h / 7d / 30d` — all UTC-aligned.

### 22.4  Frozen file schemas

- `slos/schemas/daily-parquet-v1.json`
- `slos/schemas/monthly-json-v1.json`
- `slos/schemas/quarterly-evidence-v1.json`

### 22.5  Frozen CLI subcommand surface

`compute | rollup | publish | evidence | list | validate | version`

### 22.6  Frozen Helm values surface

`chartVersion`, `region`, `mode`, `storage.*`, `cronjobs.*`,
`cosign.identity`, `cosign.disabled`, `prometheus.url`, `image.*`,
`securityContext.*`, `containerSecurityContext.*`, `rbac.*`,
`serviceAccount.*`.

### 22.7  Frozen public-page URLs

```
/current.json
/history/{yyyy-mm}.json
/quarterly/{Q1..4}-{yyyy}.{json,pdf,pdf.bundle}
/badges/slo-1-availability.svg
/badges/slo-2-latency-p95.svg
/badges/slo-3-recovery-time.svg
/badges/slo-4-replication-lag.svg
/version.json
```

### 22.8  Cosign signing identity regex (frozen)

```
^https://token\.actions\.githubusercontent\.com$
```

Workflow identity must match :
```
https://github\.com/efe-ai/efe/\.github/workflows/slo-quarterly\.yml@refs/.+
```

### 22.9  Audit trail addendum

| Date | Change | Chart | Contract |
|---|---|---|---|
| 2026-05-16 | PR #22 — 4 SLO IDs + 9 thresholds + 8 windows + 3 schemas + 7 CLI + signing identity regex frozen | 0.6.0 (rollup 0.1.0) | **v1.4** |


---

## 23. Smoke surface — frozen v1.5 (PR #23)

Frozen contracts for the EFE 9-smoke-check merge gate (`efe-smoke`
crate at `smoke/runner/`).  Reference :
[`docs/EFE_A1_PHASE2_PR23_ARCHITECTURE.md`](EFE_A1_PHASE2_PR23_ARCHITECTURE.md).

### 23.1  9 smoke check IDs (closed taxonomy)

```
S1_boot
S2_l1_swarm
S3_l2_meta_swarm
S4_mtls_rotation
S5_reload_sighup
S6_metrics
S7_otel
S8_restart
S9_shutdown
```

### 23.2  Per-check wall-clock budgets (seconds)

| S1 | S2 | S3 | S4 | S5 | S6 | S7 | S8 | S9 | Σ |
|---|---|---|---|---|---|---|---|---|---|
| 30 | 15 | 20 | 25 | 20 | 10 | 15 | 30 | 15 | **180** |

Plus ~ 60 s cluster boot = total ≤ 240 s (AD3 global hard cap).

### 23.3  14 STABLE metric families validated by S6

Frozen by PR #17 STABILITY_CONTRACT v1.0 § 4 ; re-locked here :
```
efe_l2_request_total
efe_l2_request_duration_seconds_bucket
efe_swarm_recovery_event_total
efe_swarm_recovery_seconds
efe_replication_lag_bucket
efe_replication_lag
efe_controller_participants_total
efe_controller_uptime_seconds
efe_participant_join_total
efe_participant_uptime_seconds
efe_l2_active_connections
efe_l2_tls_handshakes_total
efe_l2_tls_handshakes_failed_total
efe_swarm_size_current
```

### 23.4  CLI subcommand surface (frozen)

`run | list | validate | gate | version`

### 23.5  Smoke result JSON schema v1

See [`smoke/schemas/smoke-result-v1.json`](../smoke/schemas/smoke-result-v1.json).

### 23.6  ClusterProvisioner trait surface (frozen)

```rust
#[async_trait]
pub trait ClusterProvisioner: Send + Sync {
    async fn provision(&self) -> ProvisionResult<PathBuf>;
    async fn teardown(&self) -> ProvisionResult<()>;
    fn name(&self) -> &'static str;
    fn kind(&self) -> ProvisionerKind;
}
pub enum ProvisionerKind { Kind, K3s, External }
```

Located in `crates/efe-cluster-provisioner/src/traits.rs` (extracted
from PR #21 `bench/harness/src/cluster/`).  PR #21 callers receive a
byte-identical compat shim (AC6).

### 23.7  ExitCode taxonomy (6, frozen)

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Smoke fail (≥ 1 check failed) |
| 2 | Cluster provisioner failure |
| 3 | Config / flag / production-refused |
| 4 | Signal-interrupted |
| 5 | Internal |

### 23.8  Audit trail addendum

| Date | Change | Chart | Contract |
|---|---|---|---|
| 2026-05-16 | PR #23 — 9 smoke check IDs + 9 budgets + 14 metric families + 5 CLI + result JSON schema + ClusterProvisioner trait + 6 ExitCodes frozen | 0.6.0 (smoke 0.1.0) | **v1.5** |

---

## 24. Bench public-page surface — frozen v1.6 (PR #24)

Frozen contracts for the EFE public bench portal at
`https://efe.github.io/bench/` (publisher : `bench/harness/src/publish/`).
Reference : [`docs/EFE_A1_PHASE2_PR24_ARCHITECTURE.md`](EFE_A1_PHASE2_PR24_ARCHITECTURE.md).

### 24.1  11 badge file names (closed taxonomy)

```
/badges/scenario-S01.svg
/badges/scenario-S02.svg
/badges/scenario-S03.svg
/badges/scenario-S04.svg
/badges/scenario-S05.svg
/badges/scenario-S06.svg
/badges/scenario-S07.svg
/badges/scenario-S08.svg
/badges/scenario-S09.svg
/badges/scenario-S10.svg
/badges/overall.svg
```

Adding a 12th badge requires a major-version bump.

### 24.2  10 scenario page URLs

```
/scenarios/s01-baseline.html
/scenarios/s02-canary.html
/scenarios/s03-ramp.html
/scenarios/s04-chaos-participant-loss.html
/scenarios/s05-chaos-network-partition.html
/scenarios/s06-chaos-cert-expiry.html
/scenarios/s07-chaos-reload-storm.html
/scenarios/s08-multi-region.html
/scenarios/s09-soak.html
/scenarios/s10-slo-validation.html
```

### 24.3  5 JSON API endpoints (each `schema_version: 1`)

```
/api/current.json
/api/history.json
/api/scenarios.json
/api/baselines.json
/api/version.json
```

Forward-compatible : unknown fields are ignored on read (AD16).

### 24.4  Quarterly evidence path layout

```
/quarterly/Q{1..4}-{yyyy}.json
/quarterly/Q{1..4}-{yyyy}.pdf
/quarterly/Q{1..4}-{yyyy}.pdf.bundle
```

### 24.5  CLI surface (7 new `publish` flags + `publish-quarterly` subcommand + `evidence verify` subcommand)

`publish` flags (frozen) :
- `--bench-results <PATH>`
- `--bench-baselines <PATH>`
- `--chart-version <VERSION>`
- `--embed-slos`
- `--embed-smoke` (accepts `--embed-smoke`, `--embed-smoke true`, `--embed-smoke false` — default `true`)
- `--dry-run`
- `--history-window-days <N>`

`publish-quarterly` flags :
- `--period <QUARTER>`
- `--chart-version <VERSION>`
- `--output <PATH>`
- `--cosign-identity <URI>`
- `--cosign-disabled`
- `--dry-run`

`evidence verify <PDF>` (frozen — AC22, cross-chain verifier) :
- Single positional `<PDF>` path argument.
- Accepts **both** PR #22 SLO quarterly PDFs **and** PR #24 bench quarterly PDFs
  via the shared cosign keyless OIDC identity regex defined in § 24.8.
- Verifies : (a) embedded SHA-256 (PDF carries the sha256 of the sibling
  JSON sidecar via the `/EFE_BENCH_EVIDENCE_JSON_SHA256` PDF dictionary
  entry), (b) sibling `.bundle` cosign signature against the shared
  identity regex, (c) Rekor transparency-log inclusion is verified by
  cosign-2.x's `verify-blob --bundle` implicitly.
- Honest unsigned-artefact path : if no `.bundle` is present next to the
  PDF, the verifier prints a warning and exits with code `0` (SHA-only).
  If a `.bundle` IS present but cosign is not on PATH, the verifier
  exits with code `5` and a diagnostic — never silently downgrades.
- Stable exit codes (mapped to the global harness taxonomy at § 22 / § 24) :
  - `0` = verified (or SHA-only when no `.bundle` is present)
  - `5` = any verification failure : missing file, embedded-SHA mismatch,
    cosign sig invalid, cosign not available when `.bundle` present.

  Finer-grained exit codes are deliberately collapsed into the existing
  v1.6 taxonomy to avoid widening the contract surface.  Operators
  needing distinction parse the structured diagnostic message
  (e.g. `embedded SHA mismatch`, `cosign verify-blob failed`,
  `cosign required to verify .bundle`).

### 24.6  Site-URL invariants

- kebab-case lowercase
- no query strings
- explicit file extensions (`.html`, `.json`, `.svg`, `.pdf`)
- no 301/302 redirects at this level

### 24.7  gh-pages branch protection (operator-applied repo settings)

- Force-push : DISABLED
- Signed commits : required
- CODEOWNERS for `bench/**` : `@efe/release-eng`
- 2 approvals required
- Linear history : enforced

### 24.8  Shared cosign keyless OIDC identity regex (extended from PR #22 v1.4 § 22.8)

```
^https://github\.com/efe-ai/efe/\.github/workflows/(slo-quarterly|bench-publish-quarterly)\.yml@refs/.+$
```

A single verifier command accepts either PDF chain.

### 24.9  Audit trail addendum

| Date | Change | Chart | Contract |
|---|---|---|---|
| 2026-05-16 | PR #24 — 11 badges + 10 scenario pages + 5 JSON endpoints + quarterly evidence path + CLI 7 flags + cosign identity regex extension frozen | 0.6.0 (harness 0.2.0) | **v1.6** |
| 2026-05-16 | PR #25 — process invariants I.1 (audit prompt mandatory) + I.2 (single source of truth) + I.6 (audit budget) + drill cadence frozen — see § 25 | 0.6.0 (harness 0.2.0) | **v1.7** |

---

## § 25 — Process invariants (v1.7 amendment)

The following PROCESS invariants are frozen at v1.7 (chart-minor 0.7.0).
Unlike the v1.6 surface contracts (file names, schemas, exit codes),
these invariants govern the **engineering workflow** : how PRs are
audited, how parallel const lists are maintained, how audit-budget
estimates are recorded, and the cadence of operational drills.

Violation requires a STABILITY_CONTRACT MAJOR bump (not minor).  The
v1.7 process invariants are frozen for **≥ 6 chart-minor versions**
starting at 0.7.0 (same cadence as the v1.6 surface invariants).

This section is enforced primarily through CI (see PR #25 architecture)
and audited by release-eng quarterly.

### 25.1  I.1 — audit prompt mandatory

PRs ≥ 1000 LOC touching the harness `src/` tree MUST commit a
`docs/lessons/PR-<N>-lessons.md` file conforming to the schema
in [`docs/lessons/_template.md`](../docs/lessons/_template.md).

**Trigger conditions (all must hold)** :
- PR net diff (additions + deletions) ≥ 1000 LOC, AND
- PR touches ≥ 1 file under `bench/harness/src/`, `crates/*/src/`,
  or `tools/loadgen/src/`, AND
- PR title does NOT carry `[I.1-bypass]` or `[I.1-refactor]` tag.

**Required artefact** :
- File path : `docs/lessons/PR-<N>-lessons.md`.
- Schema : 7 mandatory sections per `docs/lessons/_template.md` :
  1. Headline
  2. Timeline summary
  3. Defect taxonomy
  4. What worked / What did not
  5. Process changes proposed
  6. Defect index (≥ 1 row)
  7. Known residual gaps (non-empty body)

**Enforcement** :
- CI status check `pr25-i1-audit-prompt` (`.github/workflows/pr25-process-checks.yml`).
- Validator : `bash scripts/audit-prompt-validator.sh <path>`.
- Exit codes : 0 ok ; 2 file missing ; 3 section missing ; 4 empty defect table ; 5 empty gaps section ; 6 zero timeline rows.

**Bypass tags (audited via `docs/process/i1-bypass-log.md`)** :
- `[I.1-bypass]` — security hotfix.  Follow-up lessons file PR within 7 days.
- `[I.1-refactor]` — diff ≥ 95 % whitespace + reorder.  Non-author reviewer confirms.

**Frozen attributes** :
- The 7 section titles (exact match required by validator).
- The LOC threshold (1000) — cannot tighten or loosen without a major bump.
- The bypass tag spelling.
- The validator exit-code mapping (0/2/3/4/5/6).

### 25.2  I.2 — single source of truth (SOT)

New const lists of paths / filenames / flags with ≥ 5 elements MUST
be declared in a SINGLE canonical location.  Consumers reference by
`use`, never duplicate.

**Trigger** : a `const X: &[T]` declaration with ≥ 5 elements is added
in a PR, AND any other site in the workspace either iterates the same
content or could plausibly need to.

**Required pattern** :
```rust
// Canonical location (most general module).
pub const X: &[T] = &[...];

// Consumer site.
use crate::path::X;
for item in X { ... }
```

**Enforcement** :
- Rust test : `bench/harness/tests/process_invariants.rs` (PR #25 day 3+).
- Shell scanner : `bash scripts/sot-audit.sh` (informational) ;
  `bash scripts/sot-audit.sh --strict` (gate mode for local pre-PR).
- Overlap threshold : 80 % — catches genuine drift, allows test fixtures
  that intentionally mirror to differ by ≥ 1 entry.

**Whitelisted patterns** (enumerated in `process_invariants.rs`) :
- Stability tests that mirror a canonical const to detect drift.
- Cross-crate stability mirrors (e.g. PR #22 regex mirror — D-AB pattern).
- Generated code that happens to match by content.

**Frozen attributes** :
- The 5-element threshold.
- The 80 % overlap detection threshold.
- The whitelist mechanism's existence (the specific entries are
  unfrozen — they can be added/removed per PR).

### 25.3  I.6 — audit budget recorded

PR descriptions MUST contain a section :

```markdown
## Audit budget

- Implementation : <N> days
- Audit (per I.1) : <M> days (<P> % of impl)
- Buffer for unforeseen depth : <K> days
- Total estimate : <N+M+K> days
```

All 4 numeric values MUST be present (not the underscore placeholders
from the PR template).

**Trigger** : every PR opened against `main`.

**Enforcement** :
- CI status check `pr25-i6-audit-budget` (same workflow file as I.1).
- Pattern match : the workflow greps the PR body for the section
  header AND the 4 numeric line items.

**Cumulative tracking** : actuals at merge time are recorded in
[`docs/process/audit-budget-tracker.md`](../docs/process/audit-budget-tracker.md).
Release-eng reviews aggregates and recalibrates the steady-state
target after every 3 Phase-3 PRs.

**Calibration anchor** :
- PR #24 : 150 % audit overhead (outlier — first audit-process PR).
- Phase-3 steady-state target : **30 %** (median Actual %).

**Frozen attributes** :
- The 4 line-item structure of the audit-budget section.
- The presence of the recalibration mechanism (the specific target
  number is unfrozen).
- The cumulative tracker's existence.

### 25.4  Drill cadence

Operational drills exercised against staging (NOT production) on a
fixed cadence :

| Drill | Cadence | Sub-runbook | First scheduled |
|---|---|---|---|
| Quarterly defacement drill | Once per calendar quarter | [`docs/drills/drill-defacement-response.md`](../docs/drills/drill-defacement-response.md) | 2026-Q2 |
| Annual revocation drill | Once per Q2 (SOC 2 audit alignment) | [`docs/drills/drill-quarterly-revocation.md`](../docs/drills/drill-quarterly-revocation.md) | 2027-Q2 |
| Master runbook review | Once per calendar quarter | [`docs/runbooks/bench-publish-incident-response.md`](../docs/runbooks/bench-publish-incident-response.md) § 0 (last reviewed line) | 2026-Q2 |

**Enforcement** : the drill verdicts (`pass` / `partial` / `fail`)
are filed in `docs/drills/reports/`.  A missed cadence triggers a
release-eng action item to schedule within 30 days.

**Frozen attributes** :
- The cadence (quarterly / annual).
- The named drills (defacement, revocation).
- The drill verdict taxonomy.
- The Q2 alignment for the annual revocation drill (SOC 2 cycle).

### 25.5  Audit trail addendum

Process-invariant changes are recorded here.  Frozen for ≥ 6
chart-minor versions starting v1.7 (i.e. v1.7 → 0.7.0 ... 0.12.x
inclusive ; revision allowed at chart-minor 0.13).

| Date | Change | Chart | Contract |
|---|---|---|---|
| 2026-05-16 | PR #25 — I.1 audit prompt + I.2 SOT + I.6 audit budget + drill cadence frozen ; § 25 introduced | 0.6.0 (harness 0.2.0) | **v1.7** |
