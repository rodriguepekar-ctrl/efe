# EFE-A1 Phase 2 — PR #18 : 8 OpenTelemetry span types

| Field | Value |
|---|---|
| **PR** | #18 (Sprint A1 Phase 2 / P2.3 W7 → P2.4 W8) |
| **Branch** | `efe-a1/p2-pr18-otel-spans` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× SRE + 2× meta-swarm + 1× observability + 1× security + 1× compliance |
| **Depends on** | PR #6 (controller + participant scaffold), PR #8 (Iterate wire), PR #9 (Step wire), PR #11 (efe-server flag plumbing), PR #12 (SIGHUP / ReloadConfig), PR #14 (Helm production-grade), PR #16 (mTLS + 3 TLS spans), PR #17 (14 Prometheus metrics) |
| **Blocks** | PR #19 (Grafana dashboards — depends on trace-metric correlation via `trace_id` exemplars), PR #22 (4 SLOs — depends on trace-based error attribution), PR #23 (chaos benches — depend on cross-process trace propagation), GA v1.0 (no production debugging without distributed traces) |
| **L2 gate** | G15 (production-grade distributed tracing : 8 span types + W3C Trace Context propagation + OTLP/gRPC exporter + tail-sampling-ready + 0.1 % runtime overhead) |
| **Threat IDs in scope** | T56-T65 (10 new threats : trace-ID exfil via /metrics, span PII leak, traceparent spoof, sampling-rate DoS, clock skew, tail-sampling drop, exporter OOM, exporter TLS misconfig, span attribute cardinality explosion, tracestate injection) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS2 + § 11 |
| **CVSS bandwidth** | All ≤ SEV-2 (no PII leak, no auth bypass, no RCE) |
| **Helm chart version bump** | `efe-meta-swarm` Chart.yaml.version 0.4.0 → **0.5.0** (minor — new env vars + ConfigMap) ; appVersion 0.5.0 → **0.6.0** (binary new tracing module) |
| **Total LOC budget** | Rust ~2 800 ; Helm ~400 ; tests ~1 200 ; doc ~2 500 → **~6 900 LOC** |
| **CI cost delta** | + 8 s wall-time for the new helm-unittest suite ; + 14 s for the trace-roundtrip integration test ; total + 22 s |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals
   2. Pre-flight — current `tracing` + OTel state
   3. Branch + commit plan (10 commits across 2 sprints)
   4. Architectural decisions (AD1–AD30)
   5. The 8 span types — name, attributes, parent-child, status mapping
   6. Span 1 — `efe.meta_swarm.iterate` (controller root)
   7. Span 2 — `efe.meta_swarm.connect_participants` (child)
   8. Span 3 — `efe.meta_swarm.inner_loop` (child)
   9. Span 4 — `efe.meta_swarm.safety_check` (child)
  10. Span 5 — `efe.meta_swarm.step` (participant root)
  11. Span 6 — `efe.meta_swarm.step.local` (child)
  12. Span 7 — `efe.meta_swarm.manifest.reload`
  13. Span 8 — `efe.meta_swarm.shutdown.drain`
  14. W3C Trace Context propagation (controller ↔ participant)
  15. Resource attributes (service.name, semconv compliance)
  16. Sampling strategy (head + tail hand-off to collector)
  17. OTLP/gRPC exporter configuration
  18. Span attribute cardinality budget
  19. PII redaction in span attributes (T57 mitigation)
  20. Status mapping — gRPC `Code` → OTel `StatusCode`
  21. Clock-skew handling (T60)
  22. Trace-metric correlation via exemplars (forward-ref PR #19)
  23. Tracing module — `crates/efe-meta-swarm-server/src/tracing_otel/`
  24. The tonic interceptor — extract + inject metadata
  25. Helm chart additions — values + schema + ConfigMap
  26. Operator-driven workflows : enable, change endpoint, disable
  27. Backward compat + migration
  28. Acceptance criteria (AC1–AC32)
  29. Threat-model coverage (T56-T65)
  30. Tests strategy — unit + integration + golden trace
  31. Performance budget
  32. Risks + mitigations
  33. Rollback strategy
  34. Sequence diagrams (cross-process trace, sampling, shutdown drain)
  35. Reviewer checklist
  36. Appendix A — exhaustive span schema (semconv-style)
  37. Appendix B — OTLP wire-format example
  38. Appendix C — Jaeger/Tempo query cookbook
  39. Appendix D — span attribute cardinality benchmarking
  40. Appendix E — OpenTelemetry semconv v1.27 compliance
  41. Appendix F — comparison vs Ray / vLLM / Istio
  42. Appendix G — operator runbook : `OtelExporterDown`

---

## 0. TL;DR

PR #16 shipped 3 TLS-specific OpenTelemetry spans (`tls.handshake`, `tls.reload`,
`tls.cert_check`) but they ride on the existing `tracing` crate scaffold without
a real OTLP exporter — they emit to stdout-flavoured `tracing_subscriber` only.
PR #17 shipped 14 Prometheus metrics — quantitative observability, no causal
chain across processes.

PR #18 fills the remaining observability gap : **8 new OpenTelemetry spans**
covering the controller's Iterate call path (4 spans) and the participant's
Step call path (2 spans) plus 2 cross-cutting spans (manifest reload + shutdown
drain), wired into a real **OTLP/gRPC exporter** with **W3C Trace Context
propagation** through the controller ↔ participant gRPC channel.

The deliverable is :

  - 8 new span types (§ 5–13), bringing the total to 11 (3 TLS + 8 PR #18).
  - W3C Trace Context propagation via a tonic interceptor (server + client
    interceptors install on both controller and participant).
  - OTLP/gRPC exporter with batched export (5s interval, max 1024 spans per
    batch, dropped-span counter on overflow).
  - Head-based parent-respecting sampling, configurable per environment :
    dev 100 %, staging 10 %, prod 1 % + 100 % of error spans (`always_on_error`).
  - Resource attributes auto-detected (`service.name`, `service.version`,
    `service.instance.id`, `deployment.environment`) per OTel semconv v1.27.
  - Span attribute cardinality budget (200 per pod) + circuit breaker.
  - PII redaction on attribute values (reuses PR #17's `redact_pii`).
  - 4 new env vars (`OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SDK_DISABLED`,
    `OTEL_TRACES_SAMPLER_ARG`, `OTEL_RESOURCE_ATTRIBUTES`) — all OTel-spec.
  - 3 new PrometheusRule alerts (exporter down, drop-rate high, sampling drift).

32 acceptance criteria (§ 28). 30 architectural decisions (§ 4). 10 new threats
named (T56-T65). Zero new dependencies beyond the OpenTelemetry-rs core
(`opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry`).

---

## 1. Goal + non-goals

### Goal

Ship a production-grade distributed-tracing surface for the EFE meta-swarm
controller + participant binaries that :

  - Captures the **causal chain** of every Iterate / Step / Reload / Shutdown
    operation as a tree of OTel spans.
  - **Propagates the trace** across the controller ↔ participant gRPC boundary
    via W3C Trace Context (`traceparent` + `tracestate` headers), so a single
    user-facing Iterate call produces a coherent trace spanning all
    participants the controller fans out to.
  - Exports to **OTLP/gRPC** to a configurable collector (default
    `http://otel-collector.observability.svc:4317` ; operator-overridable via
    `OTEL_EXPORTER_OTLP_ENDPOINT`).
  - Implements **head-based parent-respecting sampling** : if the incoming
    request carries a sampled traceparent, the span is sampled ; otherwise the
    binary's own sampler decides (rate-based, env-tunable).
  - Honours **OpenTelemetry semantic conventions v1.27** for attribute naming
    (`service.name`, `rpc.system="grpc"`, `code.namespace`, etc.).
  - Annotates spans with **trace-metric correlation hints** (the Prometheus
    `efe_meta_swarm_iterate_duration_seconds` histogram exposes
    `trace_id`/`span_id` exemplars at scrape time — PR #19 dashboard
    consumes them).
  - Adds **3 new PrometheusRule alerts** : exporter down (TCP unhealthy ≥ 5 m),
    drop rate > 1 % over 5 m, sampling rate drift > 50 % vs target.
  - Maintains **strict backward compat** : the existing `tracing` scaffold + 3
    TLS spans from PR #16 stay unchanged. Operators with no OTel collector
    deployed get a no-op exporter (zero runtime overhead).

Customer-facing outcome :

  - SRE pulls up a Jaeger / Tempo / Honeycomb UI, types an Iterate trace ID,
    sees the full tree : Iterate → connect_participants → 3× participant
    connects → inner_loop → 100× step (3 participants × 33 iters) →
    safety_check.
  - Compliance auditor traces a failed Iterate end-to-end : the gRPC error code
    is mapped to OTel `StatusCode::Error`, the cause attribute is set to the
    `failure_reason` taxonomy (`dim_mismatch`, `safety_violation`, etc.).
  - Customer reports a slow Iterate ; SRE compares two trace IDs side-by-side
    and sees which participant's Step span took 10 × longer than baseline.

### Non-goals

  - **Not** OTel logs. Logs stay in the `tracing` crate's
    `tracing_subscriber::fmt` formatter. The OTel log signal (`logs.exporter`)
    is a Phase-3 concern.
  - **Not** OTel metrics. Prometheus stays canonical (PR #12 + PR #17).
    Cross-system metric duplication is explicitly avoided.
  - **Not** custom instrumentation libraries. We instrument only the surfaces
    we own (`efe-meta-swarm-server`). gRPC transport-level spans (auto-
    created by `tonic-otel`) are out of scope ; PR #19 may add them.
  - **Not** trace storage. Span retention, ingestion, query lives in the
    operator's OTel backend (Jaeger / Tempo / Honeycomb / Lightstep) — out of
    chart scope.
  - **Not** a replacement for the existing 3 TLS spans from PR #16. Those
    coexist ; PR #18 adds 8 NEW spans for a total of 11.
  - **Not** OTel baggage. Cross-cutting context propagation via `baggage`
    header is Phase-3 (used for tenant-id propagation in M28+).
  - **Not** trace-based metrics (`Span` → `Metric` projection at the OTel
    collector). That's a collector-side concern.
  - **Not** auto-instrumentation. Every span in this PR is hand-instrumented
    with explicit `#[instrument]` macros or `tracing::info_span!` calls. We
    avoid binary auto-instrumentation (eBPF / runtime patching) for
    determinism.
  - **Not** trace sampling control via gRPC RPC. Sampling is set at boot via
    env vars ; runtime change requires a manifest reload + Pod restart.
    Dynamic sampling control is a Phase-3 enhancement.
  - **Not** an OTel collector chart. The chart we ship configures the binary
    to PUSH to an existing collector ; spinning up the collector itself is
    the operator's responsibility (typically via the `opentelemetry-collector`
    Helm chart).

---

## 2. Pre-flight — current `tracing` + OTel state

### 2.1 What PR #6 / PR #16 shipped (`tracing` scaffold)

```text
crates/efe-meta-swarm-server/src/observability/tracing.rs   ← initialises tracing_subscriber
```

The current scaffold installs `tracing_subscriber::fmt` formatter that writes
to stdout. Every `tracing::info_span!`, `#[instrument]`, `info!(...)` call
goes there. **No OTLP exporter is wired.**

PR #16 added 3 TLS-specific spans :

```rust
// crates/efe-meta-swarm-server/src/tls/reload.rs
let span = tracing::info_span!("tls.reload", path = %path.display());

// tls/reload.rs
let span = tracing::info_span!("tls.cert_check", ...);

// (future, not yet wired in PR #16 — handshake interceptor blocked
//  on tonic 0.12 API limitation, documented as a TODO)
// let span = tracing::info_span!("tls.handshake", ...);
```

`tls.handshake` is documented as "PR #18 to add tonic interceptor" — picked up
by this PR.

### 2.2 Current Prometheus metric correlation

PR #17 metric `efe_meta_swarm_iterate_duration_seconds` is observed but has no
exemplar field — no way for Grafana to jump from a metric point to the
underlying trace. PR #18 wires exemplars.

### 2.3 What's MISSING (PR #18 scope)

  - **OTLP exporter** : no `opentelemetry-otlp` integration. Spans created by
    the `tracing` macros never leave the pod.
  - **W3C Trace Context propagation** : controller's Iterate doesn't
    propagate its trace to participants. Each Step appears as its own root
    trace.
  - **Sampling configuration** : tracing_subscriber defaults to record-all
    which works for stdout but would crush an OTLP collector.
  - **The 8 spans themselves** : 5 of 8 (iterate, connect_participants,
    inner_loop, safety_check, step) don't exist ; 2 (step.local,
    manifest.reload) exist as plain `#[instrument]` macros without OTel
    semantic attributes ; 1 (shutdown.drain) doesn't exist.
  - **Resource attributes** : `service.name` not set ; the OTel collector
    can't route the spans to the right service tier.
  - **Sampler at boot** : no `OTEL_TRACES_SAMPLER_*` env var handling.
  - **Exporter health metrics** : no `otel_export_*` counter / drop gauge.
  - **PrometheusRule alerts** : no exporter-down / drop-rate / sampling-drift
    alerts.
  - **Helm values** : no `tracing.otel.*` block ; no `OTEL_*` env vars
    propagated.

This is the work of PR #18.

---

## 3. Branch + commit plan (10 commits across 2 sprints)

```
Sprint S 0 (Aug 24 - Sep 4 2026) — exporter + interceptor + 2 root spans
─────────────────────────────────────────────────────────────────────────
C1  pr18: scaffold tracing_otel module + opentelemetry-otlp Cargo deps
        (~ 250 LOC, no new spans yet — sets the registration shape)
C2  pr18: OTLP/gRPC exporter + batch processor + drop counter
        (~ 350 LOC + tests)
C3  pr18: W3C Trace Context propagator — tonic server + client interceptor
        (~ 400 LOC + tests)
C4  pr18: efe.meta_swarm.iterate — root span on controller-side Iterate
        (~ 250 LOC + tests)
C5  pr18: efe.meta_swarm.step — root span on participant-side Step
        (~ 200 LOC + tests)

Sprint S 1 (Sep 7 - Sep 18) — child spans + reload + shutdown + chart
──────────────────────────────────────────────────────────────────────
C6  pr18: child spans — connect_participants + inner_loop + safety_check
        (~ 350 LOC + tests)
C7  pr18: efe.meta_swarm.step.local + efe.meta_swarm.manifest.reload spans
        (~ 200 LOC + tests)
C8  pr18: efe.meta_swarm.shutdown.drain span + signal handler integration
        (~ 200 LOC + tests)
C9  pr18: chart — values.yaml + schema + ConfigMap + 3 PrometheusRule alerts
        + helm-unittest
        (~ 400 LOC + tests)
C10 pr18: integration test (cross-process trace propagation roundtrip)
        + golden-trace verification + audit pass + sub-chart version bump
        (~ 300 LOC + integration tests + goldens)
```

Each commit ships with :

  - Code + unit tests + integration tests.
  - Updated `values.schema.json` if applicable.
  - Updated helm-unittest assertions (PR #14 / PR #16 / PR #17 audit-pass pattern).
  - Updated goldens (`tests/golden/kovant-prod.yaml`).
  - Conventional commit message.
  - GitHub PR description referencing this architecture doc § N.

---

## 4. Architectural decisions (AD1–AD30)

| AD | Decision | Status |
|---|---|---|
| AD1 | The `tracing` crate is the canonical instrumentation surface. OTel is an EXPORT path bridged via `tracing-opentelemetry`. We do NOT replace `tracing` with raw `opentelemetry::Tracer`. | accepted |
| AD2 | OTel exporter : `opentelemetry-otlp` over gRPC (`tonic`-based) by default. HTTP/protobuf fallback opt-in via `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf`. | accepted |
| AD3 | Span naming convention : `efe.meta_swarm.<verb>` or `efe.meta_swarm.<noun>.<verb>` (e.g. `efe.meta_swarm.iterate`, `efe.meta_swarm.step.local`). All lowercase, dot-separated. | accepted |
| AD4 | Attribute naming convention : `meta_swarm.<facet>.<name>` (e.g. `meta_swarm.iterate.iterations`, `meta_swarm.participant_id`). Snake-case, OTel-semconv compatible. | accepted |
| AD5 | W3C Trace Context propagation is the wire format ; `B3` / `Jaeger` formats are not supported in Phase 2. Operators with B3-only backends use an OTel collector pipeline that does the conversion. | accepted |
| AD6 | Head-based parent-respecting sampler. `parentbased_traceidratio` per OTel SDK convention. Ratio configurable per env via `OTEL_TRACES_SAMPLER_ARG`. | accepted |
| AD7 | Default sampling rates : dev 1.0 ; staging 0.1 ; prod 0.01. Override per-deployment via the chart's `tracing.otel.samplerArg` knob. | accepted |
| AD8 | `always_on_error` overlay : every span whose `Status::Error` is set is forced-sampled regardless of the rate. Wired via a custom `Sampler` that wraps the rate-based one. | accepted |
| AD9 | Batch span processor with `max_export_batch_size=1024`, `max_export_timeout=5s`, `scheduled_delay=5s`. Buffer overflow drops oldest span + increments `otel_dropped_spans_total`. | accepted |
| AD10 | Resource attributes set at boot from env : `service.name`, `service.version`, `service.instance.id`, `deployment.environment`, `host.arch`, `host.os.type`, `process.pid`. | accepted |
| AD11 | The 8 spans use OpenTelemetry semantic-conventions-v1.27-aligned attributes where applicable : `rpc.system="grpc"`, `rpc.service`, `rpc.method`, `network.protocol.name="grpc"`. | accepted |
| AD12 | Span attribute cardinality budget : 200 attributes per span max. The custom `tracing-opentelemetry` bridge layer drops attributes past the budget + emits a warning. | accepted |
| AD13 | PII redaction reuses PR #17's `redact_pii` filter on every string-typed attribute value before export. | accepted |
| AD14 | Span events (logs attached to spans) are NOT emitted in PR #18. The `tracing` events stay on stdout. Phase-3 PR #19.1 may bridge events to OTel log signal. | accepted |
| AD15 | The exporter MUST NOT block the gRPC handler. Batch processor runs on a dedicated tokio task with a bounded queue ; queue overflow drops the oldest span. | accepted |
| AD16 | Clock skew : timestamps come from `std::time::SystemTime` (wall-clock). Span duration computed from `Instant::now()` (monotonic). Cross-process duration display in the OTel UI uses wall-clock ; mismatches < 1s are auto-corrected by the collector. | accepted |
| AD17 | gRPC status mapping : `Code::Ok → StatusCode::Ok` ; everything else → `StatusCode::Error` with `description` set to the error message. | accepted |
| AD18 | Span links : a participant's Step span carries a `Link` pointing back to the controller's Iterate span ID. Allows query "find all Step spans linked to this Iterate". | accepted |
| AD19 | Backward compat : the existing `tracing_subscriber::fmt` stdout formatter stays installed alongside OTLP. Operators with no collector get stdout-only traces (the OTel exporter is a no-op). | accepted |
| AD20 | The `tracing` crate's `Level::ERROR` events automatically set the span's status to `Error` regardless of the gRPC return code. | accepted |
| AD21 | Trace-metric correlation : Prometheus exemplars are wired in PR #19. PR #18 ensures the underlying span IDs are available in `tracing::Span::current().context().span().span_context().trace_id()`. | accepted |
| AD22 | Resource attribute `service.instance.id` = `${HOSTNAME}` (the K8s Pod name). Falls back to a random UUID if HOSTNAME unset (dev / unit-test). | accepted |
| AD23 | OTel SDK can be disabled entirely via `OTEL_SDK_DISABLED=true` (the OTel-spec env var). This is the no-op fallback when no collector is deployed. | accepted |
| AD24 | Custom sampler decisions are recorded in the `meta_swarm.sampling.decision` attribute (`sampled` / `dropped` / `forced_on_error`). | accepted |
| AD25 | The 8 spans are stably tiered (PR #17 § 24 / stability contract) — STABLE at GA. Their names, attributes, and parent-child relationships are public API. | accepted |
| AD26 | Chart sub-version bump : 0.4.0 → 0.5.0 (minor — new env vars + ConfigMap + 3 alerts). appVersion 0.5.0 → 0.6.0 (binary new module). | accepted |
| AD27 | Exporter TLS : OTLP/gRPC defaults to plain-text in-cluster (T63 mitigation — collector lives in same trust zone). Operator opt-in to mTLS via `OTEL_EXPORTER_OTLP_CERTIFICATE` etc. | accepted |
| AD28 | Process-wide single tracer provider. The provider is set up once at binary boot in `tracing_otel::init` ; no per-handler re-init. | accepted |
| AD29 | A test-only `InMemoryExporter` is exposed for integration tests that need to capture + assert on the emitted spans without spinning up a real OTLP collector. | accepted |
| AD30 | Span sampling decision is the SAME for every replica of the same `traceparent`. Parent-respecting deterministic sampling — required for cross-process trace assembly. | accepted |

---

## 5. The 8 span types — names + attributes + parent-child + status

```text
═══════════════════════════════════════════════════════════════════════════════
                              CONTROLLER PROCESS
═══════════════════════════════════════════════════════════════════════════════
                                                                              
  efe.meta_swarm.iterate          ← root span on MetaSwarmService::Iterate
    ┃                                                                          
    ┣━ efe.meta_swarm.connect_participants                                     
    ┃    ┃                                                                     
    ┃    ┣━ efe.meta_swarm.participant.connect (per participant ; client-side)
    ┃    ┃                                                                     
    ┃    └━ (cross-process — propagates trace to participant Step span)        
    ┃                                                                          
    ┣━ efe.meta_swarm.inner_loop                                              
    ┃    ┃                                                                     
    ┃    └━ efe.meta_swarm.safety_check (per Phase-1 inner iteration)         
    ┃                                                                          
    └━ (Iterate response built + returned to caller)                          

═══════════════════════════════════════════════════════════════════════════════
                            PARTICIPANT PROCESS (each)
═══════════════════════════════════════════════════════════════════════════════

  efe.meta_swarm.step             ← root span on MetaParticipantService::Step
    ┃                                                                          
    └━ efe.meta_swarm.step.local  ← child wrapping spawn_blocking call        
                                                                              
  (link → controller's `efe.meta_swarm.iterate` span ID)                      

═══════════════════════════════════════════════════════════════════════════════
                                CROSS-CUTTING
═══════════════════════════════════════════════════════════════════════════════

  efe.meta_swarm.manifest.reload  ← on apply_reload (SIGHUP / gRPC / file)    
                                                                              
  efe.meta_swarm.shutdown.drain   ← on SIGTERM / SIGINT — captures graceful   
                                    drain window + in-flight count            
```

§ 6–13 detail each span with the exact instrumentation site, attribute schema,
status mapping, and operational rationale.

---

## 6. Span 1 — `efe.meta_swarm.iterate` (controller root span)

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.iterate` |
| **Span kind** | `SERVER` (we're the gRPC server side) |
| **Parent** | Inherited from the inbound `traceparent` header (if any) ; otherwise a new root span |
| **Created at** | Start of `controller/service.rs::iterate` handler |
| **Closed at** | Return path (both Ok + Err) |
| **Stability tier** | STABLE at GA |

### 6.1 Attributes (set at span creation)

| Attribute | Type | Source | Required? |
|---|---|---|---|
| `rpc.system` | string | `"grpc"` (constant) | ✓ |
| `rpc.service` | string | `"efe.meta_swarm.v1.MetaSwarmService"` | ✓ |
| `rpc.method` | string | `"Iterate"` | ✓ |
| `meta_swarm.manifest.version` | string | `state.manifest.read().version` | ✓ |
| `meta_swarm.iterate.max_iters` | int | `req.iteration_config.max_iters` | ✓ |
| `meta_swarm.iterate.tolerance` | double | `req.iteration_config.tolerance` | ✓ |
| `meta_swarm.iterate.alpha_default` | double | `req.iteration_config.alpha_default` | ✓ |
| `meta_swarm.iterate.coupling_gamma` | double | `req.iteration_config.coupling_gamma` | ✓ |
| `meta_swarm.iterate.deterministic` | bool | `req.iteration_config.deterministic` | ✓ |
| `meta_swarm.iterate.participants_count` | int | `manifest.participants.len()` | ✓ |
| `meta_swarm.discovery_mode` | string | `state.config.discovery.to_string()` | ✓ |
| `meta_swarm.tls.enabled` | bool | `state.tls_reloader.is_some()` | ✓ |
| `meta_swarm.sampling.decision` | string | from custom sampler (§ 16) | ✓ |

### 6.2 Attributes (set at span close)

| Attribute | Type | Source | Required? |
|---|---|---|---|
| `meta_swarm.iterate.iterations` | int | `result.iterations` | when Ok |
| `meta_swarm.iterate.final_residual` | double | `result.final_residual` | when Ok |
| `meta_swarm.iterate.stop_reason` | string | `result.stop_reason` (`converged` / `max_iters_hit` / `error`) | when Ok |
| `meta_swarm.iterate.wall_us` | int | `start.elapsed().as_micros()` | always |
| `meta_swarm.iterate.outcome` | string | from `Outcome` (PR #17 taxonomy) | always |
| `meta_swarm.iterate.failure_reason` | string | from `FailureReason` (PR #17) | when Err |

### 6.3 Status mapping

| gRPC return | OTel `StatusCode` | Status description |
|---|---|---|
| `Ok` | `StatusCode::Ok` | — |
| `Err(Status::deadline_exceeded(...))` | `StatusCode::Error` | "deadline exceeded after Nms" |
| `Err(Status::cancelled(...))` | `StatusCode::Error` | "cancelled by client" |
| `Err(Status::resource_exhausted(...))` | `StatusCode::Error` | "MAX_INFLIGHT throttled" |
| `Err(...)` (other) | `StatusCode::Error` | from `Status.message()` |

### 6.4 Instrumentation site

`crates/efe-meta-swarm-server/src/controller/service.rs::iterate` — the
handler wraps the call in `crate::tracing_otel::span!(...)` (a thin macro
around `tracing::info_span!` + OTel-semconv-correct attribute names).

### 6.5 Rationale

This is the trace's **root span** for any user-driven Iterate. Every other
controller-side and participant-side span in the same call sits below this
one. The attributes are dense (12 mandatory) because Iterate is the
high-value debugging target — operators tracing a slow Iterate should not
need to chase down the manifest version, tolerance, or discovery mode from
logs.

---

## 7. Span 2 — `efe.meta_swarm.connect_participants` (child of iterate)

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.connect_participants` |
| **Span kind** | `INTERNAL` |
| **Parent** | `efe.meta_swarm.iterate` |
| **Created at** | `controller/iterate.rs::connect_participants` entry |
| **Closed at** | `connect_participants` return |
| **Stability tier** | STABLE at GA |

### 7.1 Attributes (entry)

| Attribute | Type | Source |
|---|---|---|
| `meta_swarm.participants.count` | int | `manifest.participants.len()` |
| `meta_swarm.discovery_mode` | string | `state.config.discovery` |
| `meta_swarm.tls.enabled` | bool | reloader present |
| `meta_swarm.tls.mutual` | bool | reloader's `require_mutual` |

### 7.2 Child sub-span : `efe.meta_swarm.participant.connect`

One per participant, span kind `CLIENT` (we're the gRPC client side).

| Attribute | Type | Source |
|---|---|---|
| `rpc.system` | string | `"grpc"` |
| `rpc.service` | string | `"efe.meta_swarm.v1.MetaParticipantService"` |
| `rpc.method` | string | `"Connect"` (synthetic — there's no real `Connect` RPC) |
| `meta_swarm.participant_id` | string | `p.id` |
| `meta_swarm.participant.endpoint` | string | resolved URL (host:port) |
| `meta_swarm.tls.expected_san` | string | when TLS active |
| `meta_swarm.connect.attempt_count` | int | always 1 in Phase 2 (no retry yet) |
| `meta_swarm.connect.elapsed_us` | int | at close |

### 7.3 Status mapping

| Outcome | OTel `StatusCode` |
|---|---|
| Connect succeeded | `StatusCode::Ok` |
| TCP timeout | `StatusCode::Error` + "connect timeout to <endpoint>" |
| DNS failure | `StatusCode::Error` + "discovery error : <reason>" |
| TLS handshake failure | `StatusCode::Error` + "tls handshake failed" |

### 7.4 Rationale

Connecting 3 participants in series takes ~150ms cumulative in Phase 2 ; a
flat span hides which connect step is the slow one. Per-participant
sub-spans let SRE pinpoint the regressor.

---

## 8. Span 3 — `efe.meta_swarm.inner_loop` (child of iterate)

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.inner_loop` |
| **Span kind** | `INTERNAL` |
| **Parent** | `efe.meta_swarm.iterate` |
| **Created at** | Just before `MetaSwarm::iterate()` (the Phase-1 algorithm) |
| **Closed at** | After Phase-1 returns |
| **Stability tier** | BETA at GA (Phase-3 may add inner-step sub-spans) |

### 8.1 Attributes

| Attribute | Type | Source |
|---|---|---|
| `meta_swarm.inner_loop.max_iters` | int | from `IterationConfig` |
| `meta_swarm.inner_loop.tolerance` | double | same |
| `meta_swarm.inner_loop.iteration_count` | int | `result.iterations` at close |
| `meta_swarm.inner_loop.final_residual` | double | `result.final_residual` at close |
| `meta_swarm.inner_loop.convergence` | string | `converged` / `max_iters` / `diverged` / `canceled` |
| `meta_swarm.inner_loop.elapsed_us` | int | `start.elapsed().as_micros()` |

### 8.2 Status mapping

| Outcome | OTel `StatusCode` |
|---|---|
| Converged | `StatusCode::Ok` |
| MaxItersHit | `StatusCode::Ok` (not an error — operator's tolerance is just tight) |
| Error (Phase-1 internal) | `StatusCode::Error` |
| Canceled (via `tokio::select!`) | `StatusCode::Error` + "cancelled" |

### 8.3 Rationale

This span isolates the **algorithm's compute time** from the surrounding
plumbing. Operators tuning `tolerance` look at this span's duration vs
iteration count distribution.

---

## 9. Span 4 — `efe.meta_swarm.safety_check` (child of inner_loop)

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.safety_check` |
| **Span kind** | `INTERNAL` |
| **Parent** | `efe.meta_swarm.inner_loop` |
| **Created at** | Inside the Phase-1 inner iteration's safety projection step |
| **Closed at** | After projection returns |
| **Stability tier** | BETA at GA (instrumented when Phase-3 surfaces the call site) |

### 9.1 Attributes

| Attribute | Type | Source |
|---|---|---|
| `meta_swarm.safety.kind` | string | `box` / `poly` / `hull` |
| `meta_swarm.safety.participant_id` | string | offending participant id |
| `meta_swarm.safety.violated` | bool | true if projection adjusted the tensor |
| `meta_swarm.safety.adjustment_norm` | double | L2 norm of the adjustment when violated |

### 9.2 Status mapping

| Outcome | OTel `StatusCode` |
|---|---|
| Not violated | `StatusCode::Ok` |
| Violated + adjusted | `StatusCode::Ok` (the projection is a recovery mechanism) |
| Violated + un-recoverable | `StatusCode::Error` |

### 9.3 Phase-2 limitation

Phase-1 `MetaSwarm::iterate` doesn't currently expose the safety check call
site to the surrounding crate. PR #18 wires a `safety_check` callback hook
that the Phase-3 algorithm refactor will populate. In Phase 2, this span
emits 0 times — the surface is registered for forward compatibility.

---

## 10. Span 5 — `efe.meta_swarm.step` (participant root)

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.step` |
| **Span kind** | `SERVER` |
| **Parent** | Extracted from inbound `traceparent` (the controller's `iterate` span) |
| **Link** | Back to the controller's `connect_participants` span via the gRPC client-side span context |
| **Created at** | `participant/service.rs::step` handler entry |
| **Closed at** | Handler return |
| **Stability tier** | STABLE at GA |

### 10.1 Attributes (entry)

| Attribute | Type | Source |
|---|---|---|
| `rpc.system` | string | `"grpc"` |
| `rpc.service` | string | `"efe.meta_swarm.v1.MetaParticipantService"` |
| `rpc.method` | string | `"Step"` |
| `meta_swarm.participant_id` | string | `self.state.local.id()` |
| `meta_swarm.step.step_index` | int | `req.step_index` |
| `meta_swarm.step.dim` | int | `pulled.dim` |
| `meta_swarm.step.payload_in_bytes` | int | pulled tensor bytes |

### 10.2 Attributes (close)

| Attribute | Type | Source |
|---|---|---|
| `meta_swarm.step.payload_out_bytes` | int | out tensor bytes |
| `meta_swarm.step.local_residual` | double | computed residual |
| `meta_swarm.step.elapsed_us` | int | wall time |
| `meta_swarm.step.outcome` | string | from `Outcome` taxonomy |
| `meta_swarm.step.failure_reason` | string | from `FailureReason` (when Err) |

### 10.3 Cross-process propagation

The parent context comes from the gRPC `metadata.traceparent` value injected
by the controller's client-side interceptor. The tonic server-side
interceptor extracts it on entry and sets the OTel current context. Result :
Jaeger / Tempo display the participant Step span as a child of the
controller's connect_participants sub-span.

---

## 11. Span 6 — `efe.meta_swarm.step.local` (child of step)

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.step.local` |
| **Span kind** | `INTERNAL` |
| **Parent** | `efe.meta_swarm.step` |
| **Created at** | Just before `spawn_blocking(move || local.step(...))` |
| **Closed at** | After spawn_blocking returns |
| **Stability tier** | STABLE at GA |

### 11.1 Attributes

| Attribute | Type | Source |
|---|---|---|
| `meta_swarm.step.local.gamma` | double | from manifest |
| `meta_swarm.step.local.alpha` | double | from manifest |
| `meta_swarm.step.local.dim` | int | local.dim() |
| `meta_swarm.step.local.kappa` | double | local.kappa() |
| `meta_swarm.step.local.elapsed_us` | int | wall time |

### 11.2 Span context propagation across `spawn_blocking`

`tokio::task::spawn_blocking` runs on a non-tokio worker pool. The
`tracing` crate's span context is thread-local ; without explicit
propagation it's lost. We use `tracing::Span::current().in_scope(...)`
inside the closure to re-enter the span before running the Phase-1 step.

---

## 12. Span 7 — `efe.meta_swarm.manifest.reload`

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.manifest.reload` |
| **Span kind** | `INTERNAL` (no RPC layer) |
| **Parent** | New root span (manifest reload is independent of any caller) |
| **Created at** | Entry of `apply_reload_*` (SIGHUP / gRPC / file_change paths) |
| **Closed at** | Reload completion |
| **Stability tier** | STABLE at GA |

### 12.1 Attributes

| Attribute | Type | Source |
|---|---|---|
| `meta_swarm.reload.trigger` | string | `sighup` / `grpc` / `file_change` |
| `meta_swarm.reload.gen_before` | int | `state.health.snapshot().reload_gen` |
| `meta_swarm.reload.gen_after` | int | post-bump value (at close) |
| `meta_swarm.reload.participants_before` | int | `state.manifest.read().participants.len()` |
| `meta_swarm.reload.participants_after` | int | new manifest's participant count |
| `meta_swarm.reload.outcome` | string | `ok` / `error` |
| `meta_swarm.reload.failure_reason` | string | when error (validation/io/drift/incompatible) |
| `meta_swarm.reload.changes_count` | int | `outcome.applied_changes.len()` |
| `meta_swarm.reload.duration_seconds` | double | wall time |

---

## 13. Span 8 — `efe.meta_swarm.shutdown.drain`

| Field | Value |
|---|---|
| **Span name** | `efe.meta_swarm.shutdown.drain` |
| **Span kind** | `INTERNAL` |
| **Parent** | None (root span) |
| **Created at** | Signal handler in `bin/controller.rs::async_main` after `await_termination()` |
| **Closed at** | After grpc + metrics tasks complete (or drain timeout) |
| **Stability tier** | STABLE at GA |

### 13.1 Attributes

| Attribute | Type | Source |
|---|---|---|
| `meta_swarm.shutdown.signal` | string | `sigterm` / `sigint` |
| `meta_swarm.shutdown.drain_deadline_seconds` | double | `config.drain_timeout.as_secs_f64()` |
| `meta_swarm.shutdown.inflight_at_start` | int | `state.inflight.inflight()` |
| `meta_swarm.shutdown.inflight_at_end` | int | same after drain |
| `meta_swarm.shutdown.elapsed_seconds` | double | wall time |
| `meta_swarm.shutdown.outcome` | string | `clean` (drained on time) / `deadline` (hit drain deadline) / `forced` |

### 13.2 Status mapping

| Outcome | OTel `StatusCode` |
|---|---|
| `clean` | `Ok` |
| `deadline` | `Error` + "in-flight requests still pending at drain deadline" |
| `forced` | `Error` + "forced shutdown" |

---

## 14. W3C Trace Context propagation (controller ↔ participant)

### 14.1 Wire format

W3C `traceparent` header (HTTP request header / gRPC metadata) :

```
traceparent: 00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01
              ^^   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ ^^^^^^^^^^^^^^^^^^^^^ ^^^
              version  trace_id (16 bytes hex)              parent_id (8 bytes)   flags
```

`tracestate` carries vendor-specific extras (we don't use it in PR #18 ; pass-through only).

### 14.2 Controller-side propagation (client interceptor)

```rust
// crates/efe-meta-swarm-server/src/tracing_otel/interceptor.rs

use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tonic::metadata::MetadataMap;

pub struct ClientTraceInterceptor {
    propagator: TraceContextPropagator,
}

impl tonic::service::Interceptor for ClientTraceInterceptor {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        let cx = tracing::Span::current().context();
        let mut injector = TonicMetadataInjector(req.metadata_mut());
        self.propagator.inject_context(&cx, &mut injector);
        Ok(req)
    }
}
```

`TonicMetadataInjector` adapts `tonic::metadata::MetadataMap` to OTel's
`Injector` trait : writes `traceparent` + `tracestate` to the metadata.

### 14.3 Participant-side extraction (server interceptor)

```rust
pub struct ServerTraceInterceptor {
    propagator: TraceContextPropagator,
}

impl tonic::service::Interceptor for ServerTraceInterceptor {
    fn call(&mut self, req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        let extractor = TonicMetadataExtractor(req.metadata());
        let parent_cx = self.propagator.extract(&extractor);
        // Bind parent context to the current tracing scope so any
        // `info_span!` in the handler picks it up as parent.
        let _entered = tracing::Span::current().set_parent(parent_cx);
        Ok(req)
    }
}
```

### 14.4 Wire-up

Both interceptors are added to the tonic Server / Channel construction in
`bin/controller.rs` and `bin/participant.rs`. They run before/after the
existing application-layer logic.

---

## 15. Resource attributes (semconv compliance)

| Attribute | Source | Example value |
|---|---|---|
| `service.name` | `OTEL_SERVICE_NAME` env or `"efe-meta-swarm-controller"` / `-participant` | `"efe-meta-swarm-controller"` |
| `service.version` | `env!("CARGO_PKG_VERSION")` | `"0.2.0"` |
| `service.instance.id` | `$HOSTNAME` (K8s Pod name) | `"tst-efe-meta-swarm-controller-7d8b9f4-x4q2j"` |
| `deployment.environment` | `OTEL_DEPLOYMENT_ENVIRONMENT` env | `"prod"` / `"staging"` / `"dev"` |
| `host.arch` | runtime detect | `"arm64"` / `"amd64"` |
| `host.os.type` | runtime detect | `"linux"` |
| `process.pid` | `std::process::id()` | `1` (typically 1 in K8s) |
| `process.executable.name` | `argv[0]` | `"controller"` / `"participant"` |
| `k8s.pod.name` | `OTEL_RESOURCE_ATTRIBUTES` env (via Downward API) | same as `service.instance.id` |
| `k8s.namespace.name` | same source | `"efe"` |
| `k8s.deployment.name` | same | `"tst-efe-meta-swarm-controller"` |
| `efe.meta_swarm.role` | hard-coded | `"controller"` / `"participant"` |

---

## 16. Sampling strategy (head + tail hand-off)

### 16.1 Head sampler

`opentelemetry::sdk::trace::Sampler::parent_based(rate_based(ratio))`. The
parent-respecting outer wrapper means : if the inbound `traceparent` is
sampled, we sample ; otherwise the inner rate-based sampler decides.

### 16.2 `always_on_error` overlay

A custom sampler that wraps the rate-based one + force-samples any span
whose `tracing::Level::ERROR` event fires. Implementation : the
`tracing-opentelemetry` layer intercepts span close events and rewrites the
`sampled` flag before export. This means an error span is always exported
regardless of the head sampler's decision.

### 16.3 Default ratios per environment

| Env | Default | Justification |
|---|---|---|
| dev | 1.0 (100 %) | Single-developer, low traffic, full visibility |
| staging | 0.1 (10 %) | Moderate traffic, statistical sampling sufficient |
| prod | 0.01 (1 %) | High traffic ; error overlay catches incidents |

### 16.4 Tail sampling

Performed at the OTel collector (not in the binary). The collector applies a
`tailsampling` processor with policies like :

  - Always sample if `status.code = ERROR`.
  - Always sample if any span attribute matches `*.outcome = "throttled"`.
  - Probabilistic 10 % of remaining.

This is configured in the operator's collector YAML, not in EFE's chart.

---

## 17. OTLP/gRPC exporter configuration

### 17.1 Defaults

```rust
let exporter = opentelemetry_otlp::SpanExporter::builder()
    .with_tonic()
    .with_endpoint("http://otel-collector.observability.svc:4317")
    .with_timeout(Duration::from_secs(10))
    .build()?;

let processor = BatchSpanProcessor::builder(exporter, opentelemetry_sdk::runtime::Tokio)
    .with_max_export_batch_size(1024)
    .with_max_export_timeout(Duration::from_secs(5))
    .with_scheduled_delay(Duration::from_secs(5))
    .with_max_queue_size(8192)
    .build();
```

### 17.2 Env-var overrides

| Env | Default | Meaning |
|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `http://otel-collector.observability.svc:4317` | gRPC endpoint |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `grpc` | `grpc` or `http/protobuf` |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | `10000` (ms) | per-export timeout |
| `OTEL_EXPORTER_OTLP_HEADERS` | empty | comma-sep `k=v,k=v` ; for API-key-auth backends |
| `OTEL_EXPORTER_OTLP_CERTIFICATE` | unset | TLS server cert path |
| `OTEL_EXPORTER_OTLP_CLIENT_KEY` / `_CERTIFICATE` | unset | mTLS client identity |
| `OTEL_SDK_DISABLED` | `false` | when `true`, exporter is a no-op |
| `OTEL_TRACES_SAMPLER` | `parentbased_traceidratio` | sampler family |
| `OTEL_TRACES_SAMPLER_ARG` | env-specific (1.0 / 0.1 / 0.01) | ratio for rate-based |
| `OTEL_SERVICE_NAME` | `efe-meta-swarm-controller` (or `-participant`) | service.name resource attr |
| `OTEL_RESOURCE_ATTRIBUTES` | empty | semicolon-sep `k=v;k=v` ; merged with auto-detected |
| `OTEL_DEPLOYMENT_ENVIRONMENT` | unset | populates `deployment.environment` resource attr |

### 17.3 Exporter health metrics

The exporter increments three Prometheus counters (added to PR #17's surface) :

```
efe_meta_swarm_otel_exported_spans_total           (Counter)
efe_meta_swarm_otel_dropped_spans_total{reason}    (CounterVec)
efe_meta_swarm_otel_export_duration_seconds        (Histogram)
```

`reason` ∈ {`queue_full`, `export_error`, `shutdown`, `timeout`}.

These are PR #18-owned and registered alongside the 14 PR #17 metrics.

---

## 18. Span attribute cardinality budget

Per AD12, each span has at most 200 attributes. Operators tuning custom
attribute injection should stay well below.

In Phase 2, the actual max attribute count per span :

| Span | Attribute count |
|---|---|
| `efe.meta_swarm.iterate` | 19 |
| `efe.meta_swarm.connect_participants` | 4 + 7 × N (per participant sub) |
| `efe.meta_swarm.inner_loop` | 6 |
| `efe.meta_swarm.safety_check` | 4 |
| `efe.meta_swarm.step` | 11 |
| `efe.meta_swarm.step.local` | 5 |
| `efe.meta_swarm.manifest.reload` | 9 |
| `efe.meta_swarm.shutdown.drain` | 6 |

All well under the 200-budget. The budget is a defence against future
instrumentation drift.

---

## 19. PII redaction in span attributes (T57 mitigation)

Reuses PR #17's `redact_pii` filter. Every string-typed attribute value is
passed through the filter before being added to the span. Capitalised
bigrams, US SSN patterns, and email-like strings are replaced with
`<redacted>`.

Implementation : a wrapper around `tracing::Span::record` that intercepts
string values.

The `meta_swarm.participant_id` attribute is the highest-PII-risk surface ;
the chart's `values.schema.json` enforces `^[a-z][a-z0-9-]{0,62}$` so values
are clean by construction (defence layer 1). Layer 2 is the runtime
`redact_pii` filter. Layer 3 is the chart-side Rego rule
(`metric_label_lowercase.rego` from PR #17 reused).

---

## 20. Status mapping — gRPC `Code` → OTel `StatusCode`

Symmetric to PR #17's `classify_grpc_status` helper. The mapping :

| gRPC `Code` | OTel `StatusCode` | Reason in span status description |
|---|---|---|
| `Ok` | `Ok` | — |
| `Cancelled` | `Error` | "cancelled by client" |
| `Unknown` | `Error` | "internal error" |
| `InvalidArgument` | `Error` | "dim_mismatch or proto validation" |
| `DeadlineExceeded` | `Error` | "deadline exceeded" |
| `NotFound` | `Error` | "participant not found in manifest" |
| `AlreadyExists` | `Error` | "duplicate reload generation" |
| `PermissionDenied` | `Error` | "unauthorized" |
| `ResourceExhausted` | `Error` | "MAX_INFLIGHT throttled" |
| `FailedPrecondition` | `Error` | "safety_violation" |
| `Aborted` | `Error` | "operation aborted" |
| `OutOfRange` | `Error` | "tensor index out of range" |
| `Unimplemented` | `Error` | "RPC not implemented" |
| `Internal` | `Error` | "internal error" |
| `Unavailable` | `Error` | "participant_down" |
| `DataLoss` | `Error` | "data loss" |
| `Unauthenticated` | `Error` | "authentication failed" |

`StatusCode::Unset` is never emitted — every span gets either `Ok` or `Error`
at close.

---

## 21. Clock-skew handling (T60)

OTel span start/end timestamps come from `std::time::SystemTime` (wall-clock).
Cross-process trace assembly in Jaeger/Tempo expects all spans in a trace to
share a common monotonic-ish wall-clock baseline.

Issues :
  - K8s pods on different nodes may have drifted clocks (~ ms).
  - NTP sync is typical but not guaranteed.

Mitigations :
  - **In-process spans** : duration computed from `Instant::now()` (monotonic),
    not wall-clock subtraction.
  - **Cross-process traces** : the OTel collector's `clockskew` processor
    auto-corrects span timestamps within ± 1s of the parent span's
    end-of-creation. Operators with > 1s skew should fix NTP first.
  - **Negative duration spans** : the OTel SDK clamps duration ≥ 0 and emits
    a `tracing::warn!` log to surface the skew.

---

## 22. Trace-metric correlation via exemplars (forward-ref PR #19)

OTel-native exemplars on Prometheus histograms expose `{trace_id, span_id,
timestamp}` per bucket sample. PR #19 wires Grafana panels that link from a
histogram bucket directly to the trace in Jaeger/Tempo.

PR #18 ensures the underlying span context is available at observe() time :

```rust
let span = tracing::Span::current();
let trace_id = span.context().span().span_context().trace_id().to_string();
state.metrics.iterate().duration_seconds
    .with_label_values(&["ok", &manifest_version])
    .observe_with_exemplar(duration_seconds, hashmap! { "trace_id" => trace_id });
```

The exemplar emission requires the `prometheus-client` crate (not the
current `prometheus` crate which lacks exemplar support). PR #19 may do the
crate migration ; PR #18 only ensures the trace ID is available at the
metric observation site.

---

## 23. Tracing module — `crates/efe-meta-swarm-server/src/tracing_otel/`

```text
src/tracing_otel/
├── mod.rs              # init() + public surface
├── exporter.rs         # OTLP exporter wiring
├── sampler.rs          # always_on_error sampler wrapper
├── interceptor.rs      # tonic Server / Client interceptors
├── spans.rs            # span! macros + attribute helpers
├── resources.rs        # service.name / k8s.* resource detection
├── propagator.rs       # W3C TraceContext extractor / injector
├── in_memory.rs        # test-only InMemoryExporter (AD29)
└── metrics.rs          # 3 exporter health metrics
```

### 23.1 Public API

```rust
pub fn init_otel(role: Role, registry: &MetricsRegistry) -> Result<TracerGuard, TracingError>;
pub struct TracerGuard;  // RAII : drops the tracer provider on shutdown

pub fn current_trace_id() -> Option<String>;
pub fn current_span_id() -> Option<String>;

// Macro re-exported : EFE-flavoured info_span! with semconv attribute names.
pub use spans::span;
```

`init_otel` is called from `bin/controller.rs::async_main` + `bin/participant.rs::async_main`
right after `obs_tracing::init()`. The returned `TracerGuard` lives for the
binary's lifetime ; its Drop runs `TracerProvider::shutdown` to flush pending
spans.

---

## 24. The tonic interceptor — extract + inject metadata

Both interceptors are plugged into the tonic Server / Channel via the
`.interceptor` / `.with_interceptor` builders. Wire-up :

```rust
// controller bin
let server = Server::builder()
    .add_service(MetaSwarmServiceServer::with_interceptor(
        svc,
        tracing_otel::ServerTraceInterceptor::new(),
    ));

// controller client channel (in connect_with_tls)
let channel = Endpoint::from_shared(endpoint_str)?
    .connect_with_connector(/* ... */).await?;
let intercepted = tonic::transport::Channel::from_shared(...)
    .interceptor(tracing_otel::ClientTraceInterceptor::new());
```

---

## 25. Helm chart additions

### 25.1 values.yaml extensions

```yaml
tracing:
  otel:
    enabled: false                          # NEW — flips the OTel exporter on
    serviceName: ""                         # NEW — defaults to <fullname>-controller / -participant
    endpoint: "http://otel-collector.observability.svc:4317"  # NEW
    protocol: "grpc"                        # grpc | http/protobuf
    timeout: "10s"
    headers: ""                             # API-key auth ; comma-sep k=v
    samplerArg: "0.01"                      # rate ; 0.01 = 1% prod default
    deploymentEnvironment: "prod"           # populates deployment.environment
    tls:
      enabled: false                        # OTLP-side TLS opt-in
      caBundle: ""                          # path to CA cert
    batch:
      maxExportBatchSize: 1024
      maxExportTimeoutSeconds: 5
      scheduledDelaySeconds: 5
      maxQueueSize: 8192
```

### 25.2 ConfigMap with OTel env vars

A new template `templates/otel-configmap.yaml` emits a ConfigMap with the
above values mapped to `OTEL_*` env vars. The Deployments mount it as
`envFrom`.

### 25.3 values.schema.json gates

```json
"tracing": {
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "otel": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "enabled":     { "type": "boolean" },
        "serviceName": { "type": "string", "pattern": "^[a-z0-9][a-z0-9.-]{0,127}$" },
        "endpoint":    { "type": "string", "format": "uri" },
        "protocol":    { "enum": ["grpc", "http/protobuf"] },
        "timeout":     { "type": "string", "pattern": "^[0-9]+(s|ms)$" },
        "samplerArg":  { "type": "string", "pattern": "^(0(\\.[0-9]+)?|1(\\.0+)?)$" },
        "deploymentEnvironment": { "enum": ["dev", "staging", "prod", "canary"] }
      }
    }
  }
}
```

### 25.4 3 new PrometheusRule alerts

```yaml
- alert: MetaSwarmOtelExporterDown
  expr: |
    rate(efe_meta_swarm_otel_exported_spans_total[5m]) == 0
      and on() efe_meta_swarm_iterate_total > 0
  for: 5m
  severity: warning
  # Means : Iterate calls are happening but no spans are being exported.

- alert: MetaSwarmOtelDropRateHigh
  expr: |
    rate(efe_meta_swarm_otel_dropped_spans_total[5m])
      / rate(efe_meta_swarm_otel_exported_spans_total[5m])
      > 0.01
  for: 5m
  severity: warning

- alert: MetaSwarmOtelSamplingDrift
  expr: |
    abs(
      rate(efe_meta_swarm_otel_exported_spans_total[5m])
      / rate(efe_meta_swarm_iterate_total[5m])
      - {{ .Values.tracing.otel.samplerArg }}
    ) > {{ .Values.tracing.otel.samplerArg }} * 0.5
  for: 30m
  severity: info
```

---

## 26. Operator-driven workflows : enable, change endpoint, disable

### 26.1 Enable OTel

```bash
helm upgrade tst ./charts/efe-stack -n efe \
  --set efe-meta-swarm.tracing.otel.enabled=true \
  --set efe-meta-swarm.tracing.otel.endpoint=http://otel-collector.observability.svc:4317 \
  --set efe-meta-swarm.tracing.otel.samplerArg=0.1   # 10% sampling for canary
```

### 26.2 Change endpoint mid-flight (no Pod restart)

Currently NOT supported in Phase 2 — the OTel SDK reads the endpoint at boot
and creates the gRPC channel. Operators must do `helm upgrade` + rolling
restart. Phase 3 may add hot-reload of the OTel exporter via SIGHUP.

### 26.3 Disable OTel temporarily

```bash
helm upgrade tst ./charts/efe-stack -n efe \
  --set efe-meta-swarm.tracing.otel.enabled=false
# OR (without redeploy) :
kubectl set env deploy/tst-efe-meta-swarm-controller OTEL_SDK_DISABLED=true
```

---

## 27. Backward compat + migration

### 27.1 Default state (post-PR #18)

  - `tracing.otel.enabled: false` (default) → no OTel exporter, stdout-only.
  - All PR #6, PR #16, PR #17 tests pass unchanged.
  - The 3 TLS spans from PR #16 still emit to stdout via tracing_subscriber.
  - The 8 new spans are created in the `tracing` runtime but the OTel layer
    is a no-op — they're emitted to stdout too. Zero perf overhead.

### 27.2 Opt-in migration

Operator flips `tracing.otel.enabled: true` + sets endpoint. The chart
restarts the Deployments via a `prometheus.io/scrape-config-checksum`
annotation change. Spans start appearing in the OTel collector within ~5s
(batch interval).

---

## 28. Acceptance criteria (AC1–AC32)

| AC | Statement | Status |
|---|---|---|
| AC1 | `tracing.otel.enabled: false` (default) ; chart renders no OTLP env vars. Backward-compat. | gate |
| AC2 | `tracing.otel.enabled: true` renders the `otel-configmap.yaml` ConfigMap + propagates `OTEL_*` env vars to controller + participant Deployments. | gate |
| AC3 | Controller binary creates the 4 controller-side spans (iterate, connect_participants, inner_loop, safety_check) when `OTEL_SDK_DISABLED != true`. | gate |
| AC4 | Participant binary creates the 2 participant-side spans (step, step.local). | gate |
| AC5 | Manifest reload + shutdown drain spans emit on every reload / shutdown event. | gate |
| AC6 | W3C `traceparent` from inbound gRPC metadata is extracted and used as parent for `efe.meta_swarm.iterate`. | gate |
| AC7 | W3C `traceparent` is injected into outbound gRPC metadata for every controller→participant Step call. | gate |
| AC8 | Cross-process trace : Jaeger / in-memory exporter shows the participant Step span as a child of the controller iterate span. | gate |
| AC9 | gRPC `Code::Ok` → `StatusCode::Ok` ; everything else → `StatusCode::Error` with description set. | gate |
| AC10 | `service.name` resource attribute set to `efe-meta-swarm-controller` (or `-participant`). | gate |
| AC11 | `service.instance.id` set from `$HOSTNAME` ; falls back to UUID if unset. | gate |
| AC12 | `deployment.environment` populated from `OTEL_DEPLOYMENT_ENVIRONMENT` env. | gate |
| AC13 | Head sampler honours `OTEL_TRACES_SAMPLER_ARG` ; default per env (dev 1.0 / staging 0.1 / prod 0.01). | gate |
| AC14 | `always_on_error` overlay : an Iterate that returns `Err` is always sampled regardless of the rate. | gate |
| AC15 | Batch processor uses 1024-batch / 5s-interval / 8192-queue defaults. | gate |
| AC16 | Queue overflow drops oldest + increments `efe_meta_swarm_otel_dropped_spans_total{reason="queue_full"}`. | gate |
| AC17 | `OTEL_SDK_DISABLED=true` disables the exporter ; spans still emit to stdout via tracing_subscriber. | gate |
| AC18 | PII redaction filter applied to every string-typed attribute value. | gate |
| AC19 | Span attribute cardinality > 200 emits a warning + drops the overflow. | gate |
| AC20 | The 3 exporter-health metrics (`exported_spans_total`, `dropped_spans_total`, `export_duration_seconds`) appear in `/metrics`. | gate |
| AC21 | 3 PrometheusRule alerts render when `alerts.enabled + tracing.otel.enabled`. | gate |
| AC22 | `service.name` regex gate in values.schema.json rejects empty / uppercase. | gate |
| AC23 | `samplerArg` regex gate accepts only `^(0(\.[0-9]+)?\|1(\.0+)?)$`. | gate |
| AC24 | Integration test : a synthetic Iterate produces a complete 4-span tree (iterate → connect_participants → inner_loop → safety_check). | gate |
| AC25 | Integration test : cross-process Iterate produces 4 + 2 × N spans (controller + 1 step per participant + 1 step.local per). | gate |
| AC26 | InMemoryExporter (AD29) captures all spans for assertion in tests. | gate |
| AC27 | Manifest reload span fired on every reload (gRPC, SIGHUP, file_change). | gate |
| AC28 | Shutdown drain span fires on SIGTERM with `outcome=clean` or `outcome=deadline`. | gate |
| AC29 | All 8 spans honour semantic conventions v1.27 for shared attributes (`rpc.system`, etc.). | gate |
| AC30 | The 3 TLS spans from PR #16 still emit (backward compat). | gate |
| AC31 | Stability contract entry exists in `docs/STABILITY_CONTRACT.md` for the 8 new spans. | gate |
| AC32 | Performance overhead < 0.1 % on a steady-state Iterate (measured via cargo bench). | gate |

---

## 29. Threat-model coverage (T56-T65)

| Threat | Description | Mitigation |
|---|---|---|
| **T56** | Trace ID exfiltration via /metrics | Trace IDs appear in exemplars (PR #19) ; NetworkPolicy already isolates /metrics from outside (PR #14) |
| **T57** | Span attribute PII leakage | `redact_pii` filter on every string attribute (§ 19) + schema regex (§ 19 layer 1) + Rego (§ 19 layer 3) |
| **T58** | traceparent header spoofing | Accept any client traceparent (no validation) — by W3C spec ; downstream OTel backends are trust-zone-aware |
| **T59** | Sampling rate misconfiguration → OTLP collector overload | Schema regex on `samplerArg` (§ 25.3) + alert `MetaSwarmOtelSamplingDrift` |
| **T60** | Clock skew between controller and participant | Monotonic Instant for duration ; collector's clockskew processor (§ 21) |
| **T61** | Tail sampling discards critical error spans | Head-sampling `always_on_error` overlay forces error spans through (AD8) |
| **T62** | OTLP exporter OOM | Bounded queue (8192) + drop-oldest on overflow + dedicated tokio task + drop counter |
| **T63** | Exporter TLS misconfig | Plain-text in-cluster default (AD27) ; opt-in mTLS via `OTEL_EXPORTER_OTLP_CERTIFICATE` + Rego rule validates |
| **T64** | Span attribute cardinality explosion | 200-attribute budget per span (AD12) + warning when exceeded |
| **T65** | tracestate bag-of-bytes injection | Pass-through only ; we don't parse or validate tracestate ; downstream OTel collector sanitises |

---

## 30. Tests strategy

### 30.1 Rust unit tests (~ 25 tests)

  - `tracing_otel::exporter::tests` — exporter construction, default endpoint, env override.
  - `tracing_otel::sampler::tests` — rate sampler + always_on_error overlay.
  - `tracing_otel::interceptor::tests` — extract/inject traceparent.
  - `tracing_otel::resources::tests` — service.name + service.instance.id detection.
  - `tracing_otel::in_memory::tests` — InMemoryExporter records + emits.

### 30.2 Integration tests

  - `integration_pr18_otel_iterate_trace.rs` — synthetic Iterate produces 4-span tree.
  - `integration_pr18_otel_cross_process.rs` — controller → participant trace propagation.
  - `integration_pr18_otel_reload_trace.rs` — reload span on SIGHUP.
  - `integration_pr18_otel_shutdown_trace.rs` — shutdown drain span on SIGTERM.
  - `integration_pr18_otel_sampling.rs` — rate sampler honours `OTEL_TRACES_SAMPLER_ARG`.

### 30.3 Helm-unittest

  - `pr18_otel_test.yaml` — ConfigMap renders correctly with all 4 env vars.
  - `pr18_alerts_test.yaml` — 3 alerts render at correct thresholds.
  - Extends `schema_rejection_test.yaml` with invalid sampler / endpoint cases.

### 30.4 Golden trace

A golden trace file `tests/golden/iterate_trace.json` containing the
expected OTLP JSON for a steady-state Iterate. The integration test parses
the InMemoryExporter's captured spans + compares against the golden,
filtering volatile fields (timestamps, instance IDs).

### 30.5 Cargo bench

`benches/span_emission.rs` — measure overhead of one `tracing::info_span!`
call : target < 1 μs.

---

## 31. Performance budget

| Operation | Budget | Measured (target) |
|---|---|---|
| `info_span!("iterate", ...)` creation | < 1 μs | ~ 500 ns on Apple M1 |
| Attribute set (`span.record("k", v)`) | < 100 ns | ~ 50 ns |
| Span close + OTLP batch enqueue | < 2 μs | ~ 1 μs |
| Full `iterate` span lifecycle (create + 19 attrs + close + enqueue) | < 5 μs | ~ 3 μs |

At 100 RPS, the overhead is ~ 300 μs/s = 0.03 % CPU. Well under the 0.1 %
budget.

Memory : the queue at full capacity (8192 spans) holds ~ 8 MB (1 KB per
span average). Negligible.

---

## 32. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| OTel collector not reachable | Medium | Low | Exporter drops + alert ; in-flight workload unaffected (drop is best-effort) |
| Cardinality explosion via custom attributes | Low | High | Attribute budget + warning (AD12) |
| Sampling rate too high → collector overload | Medium | High | Schema regex on rate + alert `OtelSamplingDrift` |
| Cross-process trace parent mismatch (clock skew) | Low | Low | Collector auto-corrects (§ 21) |
| `tracing-opentelemetry` crate API instability | Medium | Medium | Pin to specific version ; subscribe to upstream announcements |
| Stability contract violated by future PR | Medium | High | `docs/STABILITY_CONTRACT.md` lists 8 spans as STABLE |
| Performance regression from OTel overhead | Low | Low | Cargo bench (§ 30.5) ; budget enforcement |
| TLS misconfig on exporter | Low | Medium | Plain-text in-cluster default ; opt-in mTLS via well-known OTEL_* vars |

---

## 33. Rollback strategy

  1. Set `tracing.otel.enabled: false` ⇒ exporter disappears, spans become
     no-op.
  2. Set `OTEL_SDK_DISABLED=true` env override (no Helm upgrade needed).
  3. Helm rollback to chart version 0.4.0 (PR #17) — removes the entire
     module.

The application's `tracing` events keep flowing to stdout regardless ; no
visible disruption.

---

## 34. Sequence diagrams

### 34.1 Cross-process Iterate trace

```text
SDK client      Controller                Participant 1         OTel collector
   │              │                          │                       │
   │ Iterate ───>│                          │                       │
   │              ├ extract traceparent     │                       │
   │              ├ create iterate span     │                       │
   │              ├ enqueue                ─────────────────────────>│
   │              │                          │                       │
   │              ├ create connect_         │                       │
   │              │  participants span     │                       │
   │              │                          │                       │
   │              ├ inject traceparent ────>│                       │
   │              │  via gRPC metadata     ├ extract                │
   │              │                          ├ create step span      │
   │              │                          ├ enqueue ──────────────>│
   │              │                          │                       │
   │              │                          ├ create step.local span│
   │              │                          ├ enqueue ──────────────>│
   │              │<─ Step response ────────│                       │
   │              │                          │                       │
   │              ├ create inner_loop span  │                       │
   │              ├ enqueue                ─────────────────────────>│
   │              │                          │                       │
   │              ├ close iterate span     │                       │
   │              ├ enqueue                ─────────────────────────>│
   │              │                          │                       │
   │<─ Iterate ───│                          │                       │
   │              │                          │                       │
   │              │                          │  (batch flushed every 5s)
```

---

## 35. Reviewer checklist

  - [ ] All 8 spans appear in `tracing_otel::spans` module.
  - [ ] All 4 controller-side spans emit on Iterate.
  - [ ] All 2 participant-side spans emit on Step + are children of controller iterate.
  - [ ] `manifest.reload` + `shutdown.drain` spans emit at the right times.
  - [ ] W3C Trace Context propagation tested end-to-end.
  - [ ] OTLP exporter env vars all parsed correctly.
  - [ ] PII redaction filter applied to span attributes.
  - [ ] Attribute cardinality budget enforced.
  - [ ] All 3 alerts render correctly with thresholds.
  - [ ] Stability contract entry added.
  - [ ] Goldens regenerated.
  - [ ] Cargo bench shows < 1 μs span overhead.
  - [ ] verify_meta_swarm_code.py updated with PR #18 checks.

---

## 36. Appendix A — exhaustive span schema

```yaml
# Phase-2 OpenTelemetry span schema, semconv v1.27-aligned.
spans:
  - name: efe.meta_swarm.iterate
    kind: SERVER
    stability: STABLE
    parent: from-traceparent
    attributes:
      required:
        - { name: rpc.system, type: string, value: "grpc" }
        - { name: rpc.service, type: string, value: "efe.meta_swarm.v1.MetaSwarmService" }
        - { name: rpc.method, type: string, value: "Iterate" }
        - { name: meta_swarm.manifest.version, type: string }
        - { name: meta_swarm.iterate.max_iters, type: int }
        - { name: meta_swarm.iterate.tolerance, type: double }
        - { name: meta_swarm.iterate.participants_count, type: int }
      on_close:
        - { name: meta_swarm.iterate.iterations, type: int }
        - { name: meta_swarm.iterate.final_residual, type: double }
        - { name: meta_swarm.iterate.stop_reason, type: string }
        - { name: meta_swarm.iterate.wall_us, type: int }
        - { name: meta_swarm.iterate.outcome, type: string }
        - { name: meta_swarm.iterate.failure_reason, type: string, optional: true }

  # ... (7 more spans, schema repeated) ...
```

---

## 37. Appendix B — OTLP wire-format example

```protobuf
ResourceSpans {
  resource: Resource {
    attributes: [
      { "service.name", "efe-meta-swarm-controller" },
      { "service.version", "0.2.0" },
      { "service.instance.id", "tst-efe-meta-swarm-controller-7d8b9f4-x4q2j" },
      { "deployment.environment", "prod" },
      { "k8s.namespace.name", "efe" }
    ]
  }
  scope_spans: [{
    scope: { name: "efe-meta-swarm", version: "0.2.0" }
    spans: [{
      trace_id: "0af7651916cd43dd8448eb211c80319c"
      span_id: "b7ad6b7169203331"
      parent_span_id: ""
      name: "efe.meta_swarm.iterate"
      kind: SPAN_KIND_SERVER
      start_time_unix_nano: 1733740800000000000
      end_time_unix_nano: 1733740800012345678
      attributes: [
        { "rpc.system", "grpc" },
        { "rpc.method", "Iterate" },
        { "meta_swarm.iterate.iterations", 47 },
        { "meta_swarm.iterate.final_residual", 1.2e-5 },
        { "meta_swarm.iterate.stop_reason", "converged" }
      ]
      status: { code: STATUS_CODE_OK }
    }]
  }]
}
```

---

## 38. Appendix C — Jaeger / Tempo query cookbook

  - **All traces for a slow Iterate** :
    `service.name="efe-meta-swarm-controller" AND name="efe.meta_swarm.iterate" AND duration > 5s`

  - **All Iterates that hit a deadline** :
    `tag:"meta_swarm.iterate.outcome=deadline"`

  - **Per-participant Step latency p99** :
    `service.name="efe-meta-swarm-participant" AND name="efe.meta_swarm.step" — quantile_over_time(0.99, duration)`

  - **Iterate → Step parent-child correlation** :
    `name="efe.meta_swarm.iterate" → name="efe.meta_swarm.step"`

---

## 39. Appendix D — span attribute cardinality benchmarking

`benches/span_attribute_cardinality.rs` :

  - 1 span × N attributes (N=1…200) ; measure observe() latency.
  - Pass criterion : p99 latency stays linear ; no cliff past 200.

---

## 40. Appendix E — OpenTelemetry semconv v1.27 compliance

| Semconv attribute | We emit | Optional? |
|---|---|---|
| `rpc.system` | ✓ | required |
| `rpc.service` | ✓ | required |
| `rpc.method` | ✓ | required |
| `rpc.grpc.status_code` | ✓ (on error) | recommended |
| `network.protocol.name` | ✓ (`"grpc"`) | recommended |
| `server.address` / `server.port` | ✗ (Phase 3) | recommended |
| `service.name` | ✓ resource | required |
| `service.version` | ✓ resource | required |
| `service.instance.id` | ✓ resource | recommended |
| `deployment.environment` | ✓ resource | recommended |
| `k8s.pod.name`, `k8s.namespace.name` | ✓ resource | recommended (when in K8s) |
| `host.arch`, `host.os.type` | ✓ resource | optional |
| `process.pid` | ✓ resource | optional |

---

## 41. Appendix F — comparison vs Ray / vLLM / Istio

| Project | Equivalent span | Notes |
|---|---|---|
| Ray Serve | `serve.handle_request` | One root span per gRPC ; no child for inner replica fan-out |
| vLLM | `vllm.generate.iteration` | One span per token-gen iteration ; finer-grained than ours |
| Istio | `istio.ingress.request` | Sidecar-level, transport-only ; no application semantics |

EFE's contribution beyond these : explicit `safety_check` + `shutdown.drain`
spans for the iteration-engine domain.

---

## 42. Appendix G — operator runbook : `OtelExporterDown`

### When

`MetaSwarmOtelExporterDown` alert fires :
`rate(efe_meta_swarm_otel_exported_spans_total[5m]) == 0 AND iterate_total > 0`.

### Triage

  1. Check the OTel collector pod : `kubectl get pod -n observability -l app=otel-collector`.
  2. Check the exporter health metric :
     `kubectl exec deploy/tst-efe-meta-swarm-controller -- curl -s http://localhost:9091/metrics | grep otel_export`.
  3. Look for `efe_meta_swarm_otel_dropped_spans_total{reason="..."}` ; the
     dominant reason is the culprit.

### Resolution

  - `reason=export_error` ⇒ network issue ; restart the OTel collector ; verify
    NetworkPolicy allows controller → collector port 4317.
  - `reason=queue_full` ⇒ sampling rate too high OR collector too slow ; reduce
    `samplerArg` or scale up the collector.
  - `reason=timeout` ⇒ collector overloaded ; check collector CPU/mem.

### Recovery

  - Once the collector is healthy, the exporter resumes within ~ 5s (next batch).
  - The dropped spans are NOT replayed — lost forever. Acceptable given
    sampling.

---

## End of architecture document — PR #18

Architecture frozen. 10-commit code plan in § 3. Migration path § 27. Coexists
with PR #16 (3 TLS spans) + PR #17 (14 Prometheus metrics) by adding 8 NEW
spans. 32 acceptance criteria. 10 new threats named (T56-T65). Stability
contract entry for the 8 spans. Cardinality budget enforced at 200
attributes per span.
