# EFE-A1 Phase 2 — PR #20 : `tools/loadgen` — production-grade L2 meta-swarm load generator

| Field | Value |
|---|---|
| **PR** | #20 (Sprint A1 Phase 2 / P2.5 W11 → P2.6 W12) |
| **Branch** | `efe-a1/p2-pr20-loadgen` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team — performance area |
| **Reviewers** | 2× perf-eng + 2× meta-swarm dev + 1× SRE + 1× security + 1× SDK lead |
| **Depends on** | PR #11 (efe-server flag plumbing), PR #16 (mTLS + cert-manager), PR #17 (Prometheus metrics surface), PR #18 (OTel spans + W3C propagation), PR #19 (dashboards + alerts ↔ SLOs that loadgen validates) |
| **Blocks** | PR #21 (chaos benches consume loadgen as the traffic driver), PR #22 (Phase-3 SLO formalisation — loadgen is the contractual generator for SLO validation), PR #23 (k6 → loadgen migration for L2 — k6 stays for L1), GA v1.0 (no production roll-out without a regression-safe load source) |
| **L2 gate** | G17 (production-grade load generator : 5 traffic patterns × 4 workload shapes × 3 rate controllers + mTLS + W3C trace propagation + Prometheus self-instrumentation + parquet/CSV/ndjson result file + multi-pod Kubernetes Job orchestration + replay-from-recording + deterministic seeding + 10 acceptance criteria for SLO regression detection) |
| **Threat IDs in scope** | T76 – T85 (10 new : production saturation via misconfigured loadgen, mTLS cred leak via verbose logs, abandoned-loadgen-pod resource hog, cross-tenant traffic injection, trace ID replay attack, recording-file payload exfil, loadgen-pod-as-pivot, controller resource exhaustion, rate-control bypass via `--qps=infinity`, RBAC over-permissive) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS3 (Performance + reliability) + § 13 (chaos benches) |
| **CVSS bandwidth** | All ≤ SEV-2 (no PII leak, no auth bypass, no RCE) |
| **Helm chart version bump** | New chart `efe-meta-swarm-loadgen` v0.1.0 (NOT a sub-chart of `efe-meta-swarm` — operationally separable) |
| **Crate version** | New crate `efe-meta-swarm-loadgen` v0.1.0, binary `efe-meta-swarm-loadgen` |
| **STABILITY_CONTRACT v** | 1.1 → **1.2** (new tier : *Loadgen self-metric*, 5 STABLE metrics under `efe_msw_loadgen_*` namespace) |
| **Total LOC budget** | Rust core ~3 800 ; gRPC client + mTLS ~600 ; self-instrumentation ~400 ; output formatters ~800 ; profile YAML ~300 ; tests ~1 320 ; Helm chart ~720 ; Docker + CI ~300 ; doc ~2 850 ; runbook + README ~620 → **~11 710 LOC** |
| **CI cost delta** | + 22 s for `cargo test -p efe-meta-swarm-loadgen` ; + 14 s for `loadgen-smoke` integration test (kind cluster) ; + 8 s for `helm-unittest` ; + 6 s for golden CLI flag diff ; total + 50 s |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals + scope
   2. Pre-flight — existing load-test infrastructure (k6, criterion benches)
   3. Branch + commit plan (12 commits across 2 sprints)
   4. Architectural decisions (AD1 – AD40)
   5. Operator personas + 7 use cases
   6. Crate structure + module layout (`tools/loadgen/`)
   7. CLI interface (subcommands + global flags)
   8. Configuration model — CLI ⊕ profile YAML ⊕ env vars precedence
   9. Traffic pattern engines (5 patterns)
  10. Workload shape catalogue (4 shapes + mixed weights)
  11. Rate control strategies (open-loop, closed-loop, hybrid)
  12. mTLS wiring (reuses PR #16 cert path + format)
  13. OTel trace integration (loadgen ↔ controller ↔ participant chain)
  14. Self-instrumentation Prometheus metrics (5 STABLE series)
  15. Determinism + seed propagation
  16. Output formats (terminal TUI, CSV, ndjson, parquet)
  17. Termination policies (5 kinds)
  18. Workload profile YAML schema (`version: 1`)
  19. Multi-pod orchestration (Kubernetes Job indexed completion)
  20. Result aggregation (when parallelism > 1)
  21. Replay mode (record once, replay deterministically)
  22. Distribution (Docker image + Helm chart + GH release artifact)
  23. STABILITY_CONTRACT v1.2 — loadgen self-metric tier
  24. Security model + Kubernetes RBAC
  25. Threat-model coverage (T76 – T85)
  26. CI integration (`loadgen-smoke` + `loadgen-slo-regression` jobs)
  27. Performance budget (per-pod QPS ceiling, latency overhead, memory)
  28. Risks + mitigations
  29. Rollback strategy
  30. Sequence diagrams (open-loop, replay, multi-pod aggregate)
  31. Reviewer checklist
  32. Acceptance criteria (AC1 – AC32)
  33. Tests strategy (unit + integration + golden + kind-cluster e2e)
  34. Appendix A — exhaustive CLI flag reference
  35. Appendix B — workload profile YAML schema (annotated)
  36. Appendix C — Prometheus self-metrics reference
  37. Appendix D — OTel span schema (loadgen-side)
  38. Appendix E — Helm chart values structure
  39. Appendix F — multi-region deployment topology
  40. Appendix G — Phase-3 follow-ups

---

## 0. TL;DR

The meta-swarm controller + participants stack (PR #11 → #18) and its operator
surface (PR #19) are now feature-complete from a *server-side* perspective.
What's missing is a **production-grade, deterministic, regression-safe load
generator** that can :

  - Drive gRPC traffic at controllable rates against the controller and/or
    participants, exercising the full chain (Iterate → Step → State → Reset).
  - Validate the 4 SLOs committed in STABILITY_CONTRACT v1.1 (PR #19 § 32)
    by running specific traffic profiles + measuring outcomes.
  - Generate trace exemplars in Tempo for visual debugging on D2/D3/D9
    (PR #19 dashboards).
  - Serve as the traffic driver for the chaos benches landing in PR #21.
  - Replace the ad-hoc Criterion benches (`SLO-RD1-*` in `slo.md`) with a
    standardised, multi-pod-capable generator.

PR #20 ships :

  - A new Cargo workspace member at **`tools/loadgen/`** producing the
    binary **`efe-meta-swarm-loadgen`**.
  - **5 traffic patterns** (constant, ramp, burst, diurnal, replay).
  - **4 workload shapes** (iterate-heavy, step-heavy, reset/state torture,
    mixed) selectable via CLI or YAML profile.
  - **3 rate controllers** (open-loop token bucket, closed-loop concurrency,
    hybrid open-with-cap).
  - **5 termination policies** (duration, request count, until-SLO-breach,
    convergence, signal).
  - **mTLS bootstrap** matching PR #16's cert format (controller cert-manager
    Certificate consumable by the loadgen pod via the same Secret pattern).
  - **W3C Trace Context propagation** — every loadgen request emits a root
    `efe.meta_swarm.loadgen.iterate` span that becomes the parent of the
    controller's `efe.meta_swarm.iterate` span.
  - **5 STABLE Prometheus metrics** (`efe_msw_loadgen_*`) committed in
    STABILITY_CONTRACT v1.2.
  - **4 output formats** — live terminal TUI (crossterm + ratatui), CSV,
    ndjson, parquet (default for CI consumption via duckdb).
  - **Kubernetes-ready** : Helm chart with Job + ServiceAccount + RBAC +
    ServiceMonitor + PodSecurityContext (nonroot, read-only-rootfs,
    drop ALL capabilities).
  - **Multi-pod parallelism** via Job indexed completion + seed-space
    partitioning + per-pod result file + aggregator post-step.
  - **Deterministic** : same seed produces identical wire bytes —
    invaluable for CI regression detection.

The deliverable is a **single statically-linked Rust binary** (distroless
Docker image, multi-arch amd64+arm64) + Helm chart + 4 example workload
profiles + 10 acceptance criteria + golden CLI flag snapshot for CI.

---

## 1. Goal + non-goals + scope

### 1.1 Goal

Ship a **deterministic, regression-safe, mTLS-capable, OTel-traceable load
generator** for the L2 meta-swarm gRPC surface that :

  - replaces the ad-hoc Criterion benches (`SLO-RD1-*`) with a standardised,
    multi-pod-capable generator,
  - serves as the contractual traffic source for the 4 PR #19 SLOs (any SLO
    validation in CI uses loadgen with a specific profile),
  - drives chaos benches (PR #21) by emitting fault-aware spans,
  - lets operators reproduce production-observed traffic patterns offline
    via the replay mode (record once, replay deterministically),
  - never accidentally saturates production (sane defaults, `--mode=production`
    confirmation gate, hard QPS cap).

### 1.2 Non-goals (out of scope)

  - **NOT a replacement for k6** — k6 covers the L1 SwarmService HTTP/gRPC
    surface and stays in `deploy/load-tests/k6/`. PR #20 is L2 (meta-swarm)
    only.
  - **NOT a generic load generator** — domain-aware (knows about Iterate /
    Step / Reset / State semantics, tensor dims, manifest versions). Other
    Cargo workspaces should use `wrk2` / `vegeta` / `ghz`.
  - **NOT a chaos-injection tool** — chaos-mesh stays the substrate (PR #21).
    Loadgen *coordinates* with chaos via markers but doesn't inject faults.
  - **NOT a soak-test tool** — < 24 h runs are first-class ; multi-day soak
    is a Phase-3 follow-up (the loadgen memory profile assumes single-run).
  - **NOT a graphical analyser** — output is structured (parquet/CSV) ;
    operators use existing tools (Grafana, duckdb, Jupyter).
  - **NOT a CDN / WAF / proxy** — gRPC client side only.
  - **NOT auto-tuning** — operators specify the workload ; loadgen executes
    deterministically. Adaptive load (ramp until SLO breach) is supported
    via the `until-slo` termination policy but the *shape* is operator-set.
  - **NOT a metrics aggregator across multiple regions** — region-local
    aggregation only ; cross-region rollup is a Phase-3 deliverable (Thanos
    layer is the right place).

### 1.3 Scope summary

| Deliverable | In | Out |
|---|---|---|
| Rust binary | ✓ (`efe-meta-swarm-loadgen`) | Static binary, multi-arch |
| Workspace member | ✓ (`tools/loadgen/`) | — |
| Traffic patterns | 5 (constant, ramp, burst, diurnal, replay) | Adaptive ML-driven patterns (Phase-3) |
| Workload shapes | 4 (iterate, step, reset, state) + mixed | Tensor learning workloads (out of scope) |
| Rate controllers | 3 (open / closed / hybrid) | Custom controller plugin API (Phase-3) |
| mTLS | ✓ matches PR #16 cert format | — |
| OTel propagation | ✓ W3C Trace Context | — |
| Prometheus self-metrics | 5 STABLE (`efe_msw_loadgen_*`) | Open-ended metric registration |
| Output formats | 4 (TUI, CSV, ndjson, parquet) | Excel / proprietary formats |
| Termination policies | 5 (duration, count, SLO, convergence, signal) | — |
| Helm chart | ✓ (separate chart `efe-meta-swarm-loadgen`) | Bundled in `efe-meta-swarm` |
| Multi-pod | ✓ (Kubernetes Job + indexed completion) | StatefulSet (over-engineered for this use case) |
| Replay mode | ✓ (record once, replay deterministically) | Production-tap recording (operator-side) |
| Acceptance criteria | 32 (AC1 – AC32) | — |
| Threat model | T76 – T85 | — |

---

## 2. Pre-flight — existing load-test infrastructure

### 2.1 Repository inventory

```
deploy/load-tests/
├── README.md
├── k6/                          # L1 SwarmService HTTP/gRPC load tests (existing)
├── gpu/                         # GPU-specific micro-benches (Triton)
└── p1/                          # Phase-1 latency benches

crates/efe-bench/                # Sprint D18 — bench harness crate (Criterion)
crates/efe-server/tests/         # `server_rpc_realism_check.rs` — 3 in-process integration tests

tools/
├── sigstore/                    # Sigstore keys + policies (non-Rust)
└── loadgen/                     # NEW (PR #20)
```

### 2.2 What k6 does (and why loadgen doesn't replace it)

k6 (Grafana's Go-based load test runner) is JavaScript-scriptable and HTTP-
oriented. It tests **L1 SwarmService** which is the operator-facing API
gateway. k6 :

  - cannot speak meta-swarm's internal gRPC protocol (it speaks HTTP + JSON-
    over-gRPC via a custom xk6-grpc extension at best),
  - has no understanding of tensor types, manifest versions, or
    convergence semantics,
  - emits its own Prometheus metrics (`k6_*`) not in the
    `efe_msw_loadgen_*` namespace,
  - lacks the deterministic seed propagation needed for CI regression.

k6 stays for L1 functional tests. PR #20 fills the L2 gap.

### 2.3 What `crates/efe-bench` does

`efe-bench` is a Criterion-based harness for the meta-swarm controller
running **in-process** :

  - directly invokes `MetaSwarm::iterate` without going through gRPC,
  - measures algorithmic latency (Phase-1 contractive iteration),
  - doesn't exercise the network stack, mTLS, OTel propagation, or any
    operator-surface code path.

PR #20 complements `efe-bench` by exercising the **full networked stack**
including mTLS handshake, gRPC framing, tonic interceptors, OTel span
propagation, and the controller→participant fan-out.

### 2.4 What `server_rpc_realism_check.rs` does

3 integration tests in `crates/efe-server/tests/` :

  - `real_e2e_latency_naive` (3 sizes : small/medium/large)
  - `real_e2e_latency_blas` (with BLAS-accelerated kernels)
  - `real_e2e_throughput` (concurrent fan-out)

These tests stand up a real gRPC server in-process and hit it over loopback.
PR #20 supersedes them : the new pattern is to run the test as a CI step
that invokes `efe-meta-swarm-loadgen` against a kind-cluster controller.

The existing tests remain (no breaking change) but are deprecated in
STABILITY_CONTRACT v1.2 § 9.8 as "BETA" — replaced by loadgen-driven
equivalents in CI.

### 2.5 Gaps to close

  - **0** L2-aware gRPC load generators in the repo.
  - **0** Kubernetes-ready load test pods. The k6 tests run only via local
    `k6 run` invocation.
  - **No reproducible traffic profiles** — operators currently script
    one-off load tests in bash.
  - **No deterministic CI regression target** — Criterion benches measure
    in-process latency but don't catch tonic / mTLS / interceptor
    regressions.
  - **No trace exemplar generator** — PR #19 dashboards have exemplar dot
    placeholders but no traffic to populate them with real traces. Operators
    today must rely on production traffic.

---

## 3. Branch + commit plan (12 commits across 2 sprints)

Fine-grained so each commit is independently reviewable + revertable
(AD17 from PR #19). Reviewers can sign off commit-by-commit.

| # | Commit | LOC | Sprint | Reviewer focus |
|---|---|---|---|---|
| C1 | `feat: workspace member tools/loadgen + Cargo.toml skeleton` | ~140 | P2.5 W11 | Workspace integration |
| C2 | `feat(loadgen): clap CLI + 40 flags + golden snapshot` | ~480 | P2.5 W11 | CLI ergonomics, flag conflicts |
| C3 | `feat(loadgen): gRPC ClientPool + mTLS bootstrap (reuses PR #16 cert)` | ~620 | P2.5 W11 | Connection pool, TLS verification |
| C4 | `feat(loadgen): 5 traffic patterns + tests` | ~860 | P2.5 W11 | Pattern math, edge cases |
| C5 | `feat(loadgen): 4 workload shapes + Iterate/Step/Reset/State dispatch` | ~720 | P2.5 W12 | Protocol-correctness |
| C6 | `feat(loadgen): 3 rate controllers + token bucket math` | ~520 | P2.5 W12 | Rate-limiting fairness |
| C7 | `feat(loadgen): self-instrumentation Prometheus + OTel SDK init` | ~480 | P2.5 W12 | Metric stability, span lifecycle |
| C8 | `feat(loadgen): 4 output formatters (terminal/csv/ndjson/parquet)` | ~840 | P2.5 W12 | Parquet schema, TUI rendering |
| C9 | `feat(loadgen): 5 termination policies + signal handling` | ~360 | P2.6 W12 | Graceful drain semantics |
| C10 | `feat(loadgen): workload profile YAML schema v1 + 4 examples` | ~440 | P2.6 W12 | Profile validation, example fidelity |
| C11 | `feat(loadgen): Helm chart + Docker image + multi-pod orchestration` | ~1 020 | P2.6 W12 | RBAC, PodSecurityContext, distroless image |
| C12 | `docs(loadgen): architecture + STABILITY_CONTRACT v1.2 + runbook + CI workflow` | ~3 470 | P2.6 W13 | Doc parity with PR #19, AC32 |

**Total** : ~9 950 LOC committed + ~1 760 LOC test fixtures = ~**11 710 LOC**
across 12 commits.

**Branch protection** : every commit must pass `cargo test -p efe-meta-swarm-loadgen`,
`cargo clippy -p efe-meta-swarm-loadgen -- -D warnings`, `helm template`,
`helm-unittest`, and the golden CLI flag snapshot test.

---

## 4. Architectural decisions (AD1 – AD40)

> Format identical to PR #18 § 4 / PR #19 § 4. Each AD is one sentence +
> 1-3 lines of rationale.

### AD1 — Rust binary, not Go / Python / Node

Reuses `efe-meta-swarm-proto` crate (proto types), shares the tokio
runtime with the rest of the stack, matches the multi-arch build pipeline.
Go would have meant maintaining a parallel proto-rs ↔ proto-go workflow ;
Python would not have hit the per-pod 50K QPS target.

### AD2 — Workspace member at `tools/loadgen/`, not `crates/`

`crates/` holds libraries and core service binaries (efe-server,
efe-meta-swarm-server). `tools/loadgen/` reflects intent : this is an
operator utility, not a service. Cargo workspace `members = ["tools/loadgen"]`
supports the path. (`crates/efe-bench` is the analogue for in-process
benches — different scope.)

### AD3 — Single statically-linked binary, no plugin architecture

Plugin loadable patterns / workloads would add `libloading` + dynamic
ABI concerns. The 5 patterns + 4 workloads are committed in the
STABILITY_CONTRACT ; new ones land as PRs, not third-party plugins.

### AD4 — Closed-loop ↔ open-loop ↔ hybrid rate control as first-class

Closed-loop measures backpressure (real-world saturation). Open-loop tests
controller's behaviour under fixed external rate. Hybrid is the realistic
middle ground (open rate with concurrency cap = closed-loop ceiling).

### AD5 — Default safety : `--qps` cap 100 with `--i-am-sure` to unlock higher

Prevents accidental production saturation by an operator typing the wrong
endpoint. Hard cap : 100,000 QPS per pod *even with* `--i-am-sure`. Higher
rates → multi-pod parallelism mandatory (AD27).

### AD6 — Production mode requires `--confirm-prod`

`--mode=production` is the only setting that disables certain safety
guards (e.g., allows targeting a deployment with `kube_app=efe-meta-swarm`
labels). `--confirm-prod` must be passed alongside or the binary refuses
to start. Audit log entry on every production-mode invocation.

### AD7 — Deterministic seed via xoshiro256+ PRNG

64-bit seed (default = wall-clock at start), all randomness derives from
this. Same seed + same target list + same duration = bit-identical wire
bytes (modulo target wall-clock skew). xoshiro256+ chosen over SplitMix
for splittable streams (AD27 multi-pod uses sub-seeds).

### AD8 — Multi-pod via Kubernetes Job + indexed completion

Job parallelism N + `completionMode: Indexed` lets each pod read its
ordinal from `JOB_COMPLETION_INDEX` env var. Each pod takes a deterministic
sub-seed from the shared base seed. No leader election, no inter-pod
coordination — embarrassingly parallel.

### AD9 — Trace exemplars at 100 % sampling for loadgen-emitted spans

PR #19 dashboards rely on exemplar dots ; loadgen at 100 % sampling
guarantees every dot has a clickable trace. The controller-side downstream
sampler decides how many to forward to Tempo (typically 1 %).

### AD10 — Replay mode reads CSV/parquet, not pcap

CSV/parquet is operator-readable + editable. pcap would require
gRPC-message reconstruction. Replay preserves wall-clock spacing
by default ; `--replay-mode=accelerate` shrinks the timeline for
faster regression tests.

### AD11 — Workload profile YAML schema versioned (`version: 1`)

Schema migration path for future profile shape changes. Profiles
checked into the repo under `tools/loadgen/examples/` for reproducible
test runs.

### AD12 — Parquet is the default result format

Parquet is compact + columnar (analytics friendly). duckdb reads it in
<100 ms even at 10M rows. CSV / ndjson are available for human eyes
and Excel imports. Terminal TUI is for live operator monitoring (auto-
detected via `isatty`).

### AD13 — Helm chart ships loadgen as a `Job`, not Deployment

One-shot semantic by design. Use `CronJob` wrapper for periodic
regression runs. Pod-level resource limits enforced (loadgen never
starves the cluster).

### AD14 — Namespace-scoped RBAC only

Loadgen needs only network reach to controllers. ServiceAccount + Role
+ RoleBinding (no ClusterRole). The Role grants `get/list/watch` on
Services and Endpoints for target discovery — nothing else.

### AD15 — Self-metrics on `/metrics` port 9095 by default

ServiceMonitor with `port: metrics` scrapes at 15 s. Loadgen
metrics distinguished from server metrics by `service.name=efe-meta-swarm-loadgen`
OTel resource attribute + Prometheus relabeling.

### AD16 — Loadgen-specific metric prefix `efe_msw_loadgen_*` (STABILITY_CONTRACT v1.2)

5 STABLE metrics carrying `verb` (closed taxonomy `iterate|step|reset|state`)
and `result` labels matching the server-side taxonomy. Operators can
correlate loadgen-side and server-side rates at the same time-window.

### AD17 — Exit codes encode test outcome

```
0 = success — all SLOs met, no transport errors
1 = SLO breach — loadgen completed but a target SLO failed
2 = transport error — controller unreachable or mTLS handshake failed
3 = invalid config — CLI flags conflict or profile YAML malformed
4 = signal-interrupted — SIGINT / SIGTERM during a run
5 = internal error — panic, OOM, unforeseen condition
```

CI gates on exit-code 0 ; postmortems pull the result file from exit-code
1 / 5 runs.

### AD18 — `--mode=production` requires `--confirm-prod` flag

Belt-and-suspenders against accidental production saturation. Audit log
entry on every production-mode invocation.

### AD19 — Terminal TUI uses `crossterm` + `ratatui`

Live QPS / p50/p95/p99 / error rate / target health refreshed every 1s.
Quit with `q` ; snapshot to file with `s`. Auto-detect via `atty::is(Stream::Stdout)` ;
non-TTY runs default to ndjson on stdout.

### AD20 — Loadgen emits `efe.meta_swarm.loadgen.iterate` root span

Becomes the parent of the controller's `efe.meta_swarm.iterate` span
(PR #18 § 6) via W3C Trace Context propagation. Operator opens a trace
in Tempo, sees the full chain : loadgen → controller → 3 participants.

### AD21 — gRPC client uses tonic with custom interceptor for trace context injection

Reuses `efe-meta-swarm-server::tracing_otel::propagator` if the crate is a
workspace member of the same workspace ; otherwise replicates the lightweight
W3C propagator (~80 LOC).

### AD22 — Connection pooling : N persistent HTTP/2 connections per target

Configurable via `--conns-per-target` (default 4). Reduces TLS handshake
overhead at startup ; HTTP/2 multiplexes concurrent streams on a single
TCP connection efficiently.

### AD23 — Histogram buckets match server-side `iterate_duration_seconds`

Buckets `[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]`.
Operators compare loadgen-observed p99 with controller-observed p99 in
the same Prometheus query.

### AD24 — Payloads generated on-the-fly from seed

No payload caching beyond a 32-entry ring buffer for replay. Avoids OOM
on long runs. Each request's tensor values derive from the seed +
request index.

### AD25 — Workload profile YAML schema is versioned

`version: 1` is required. Future shapes bump the version + provide a
migration helper. Profiles without `version` are rejected with exit code 3.

### AD26 — Multi-region targets via weighted random selection

`--target=ctrl-us-east:50051@0.5 --target=ctrl-eu-west:50051@0.5`. Each
request picks a target weighted-random (deterministic from the seed).

### AD27 — Tonic client honors deadline propagation

`--per-request-deadline=3s` adds a `grpc-timeout` metadata. Allows testing
the controller's deadline-exceeded behaviour ; the deadline propagates to
the participant via gRPC's standard handling.

### AD28 — Loadgen reports its OWN `efe_msw_loadgen_build_info` metric

Distinguishes which loadgen build produced which workload. Includes
`version`, `git_sha`, `rustc_version`, `build_date`.

### AD29 — `Cargo.lock` shared with the rest of the workspace

Avoids dependency drift between loadgen and server crates. Single source
of truth for tonic / tokio / opentelemetry versions.

### AD30 — Binary install location `target/release/efe-meta-swarm-loadgen`

`tools/loadgen/Cargo.toml`'s `[[bin]] name = "efe-meta-swarm-loadgen"` ;
the `efe-meta-swarm-` prefix avoids collisions with the global namespace.

### AD31 — STABILITY_CONTRACT v1.2 adds *Loadgen self-metric* tier

5 new STABLE metrics with closed `verb` taxonomy. Renaming a loadgen
metric requires the same 6-minor-version notice as a server-side metric.

### AD32 — Replay preserves wall-clock spacing by default

`--replay-mode=preserve-spacing | accelerate | drain`. Accelerate flag
replays X minutes in Y seconds (X > Y). Drain mode floods at maximum
rate (ignores spacing).

### AD33 — Deterministic reproducible builds

`cargo build --locked --release` + `RUSTFLAGS=-Cmetadata=` for bit-
identical binaries. Important for CI golden-file diffs.

### AD34 — Distroless Docker base image (`gcr.io/distroless/cc-debian12:nonroot`)

Minimal attack surface — no shell, no package manager, only the binary
+ libc + ca-certificates. Multi-arch build (amd64 + arm64) via
`cargo-zigbuild` or `docker buildx`.

### AD35 — PodSecurityContext locks down loadgen

```yaml
runAsNonRoot: true
runAsUser: 65532       # distroless nonroot UID
readOnlyRootFilesystem: true
allowPrivilegeEscalation: false
capabilities: { drop: [ALL] }
seccompProfile: { type: RuntimeDefault }
```

### AD36 — Loadgen pod can mount the same Secret as the controller for mTLS

Per-pod cert via cert-manager Certificate resource, OR shared loadgen-
specific cert via `Secret` reference in the Helm chart values.

### AD37 — `--ramp-up=30s` linear ramp-up by default for production protection

Avoids cold-start cliff on the controller (TLS handshake storm,
connection pool warm-up).

### AD38 — Graceful shutdown drains in-flight requests with deadline

`--shutdown-deadline=5s` (matches PR #18 controller drain window).
Flushes Prometheus + OTel before exit. SIGINT triggers graceful drain ;
SIGKILL bypasses (operator's choice).

### AD39 — Stats batched in 1 s window for stdout/TUI

Lock-free aggregation via `hdrhistogram` crate. Avoids I/O bottleneck
at high QPS (writing 50k JSON lines/sec to stdout is the bottleneck —
1 s batches eliminate it).

### AD40 — Loadgen never mutates target state by default

Default workload : read-like operations only (State + Iterate-with-deadline).
`--allow-mutation` enables Step + Reset. Production-mode forbids
mutations regardless (`--allow-mutation` is a no-op when
`--mode=production`).

---

## 5. Operator personas + 7 use cases

### 5.1 Personas

| Persona | Goal | Frequency |
|---|---|---|
| **Perf engineer** | Validate p95/p99 across deploys, find regressions early | Daily |
| **SRE on-call** | Reproduce a customer-reported latency issue offline | Weekly |
| **Meta-swarm dev** | Test a new algorithm under realistic gRPC load | Per-feature |
| **Compliance auditor** | Verify SLO targets are met with documented evidence | Quarterly |
| **CI runner** | Run a fast SLO regression check on every PR | Per-PR |

### 5.2 The 7 canonical use cases

| # | Use case | Profile | Persona | CI ? |
|---|---|---|---|---|
| UC1 | Smoke test before deploy | `smoke-50qps-1min.yaml` | Perf eng, CI | ✓ |
| UC2 | SLO regression check | `slo-validation-5min.yaml` | CI runner | ✓ |
| UC3 | Canary roll-out validation | `canary-50qps-10min.yaml` | Perf eng | ✓ |
| UC4 | Chaos bench traffic driver | `chaos-burst-30min.yaml` | SRE | ✗ (PR #21) |
| UC5 | Reproduce a customer issue | `replay-prod-<TIMESTAMP>.yaml` | SRE | ✗ |
| UC6 | Algorithm regression bench | `algorithm-mixed-1h.yaml` | Meta-swarm dev | ✓ |
| UC7 | Compliance evidence harvest | `compliance-quarterly.yaml` | Compliance | ✓ |

Each profile is checked into `tools/loadgen/examples/` and can be invoked :

```bash
efe-meta-swarm-loadgen run --profile tools/loadgen/examples/smoke-50qps-1min.yaml
```

---

## 6. Crate structure + module layout

```
tools/loadgen/
├── Cargo.toml
├── README.md
├── src/
│   ├── main.rs                     # CLI entry → subcommand dispatch
│   ├── lib.rs                      # Library entry (embedded use in tests)
│   ├── cli.rs                      # clap argument parsing + 40 flags
│   ├── config.rs                   # Config = CLI ⊕ profile YAML ⊕ env vars
│   ├── client/
│   │   ├── mod.rs                  # ClientPool : connection multiplexing
│   │   ├── controller.rs           # MetaSwarmService wrapper
│   │   ├── participant.rs          # MetaParticipantService wrapper
│   │   └── tls.rs                  # mTLS bootstrap (matches PR #16)
│   ├── pattern/
│   │   ├── mod.rs                  # TrafficPattern trait
│   │   ├── constant.rs             # Fixed QPS
│   │   ├── ramp.rs                 # Linear / exponential ramp
│   │   ├── burst.rs                # Token-bucket overshoot
│   │   ├── diurnal.rs              # Sinusoidal day/night
│   │   └── replay.rs               # From recorded CSV/parquet
│   ├── workload/
│   │   ├── mod.rs                  # WorkloadShape trait + RPC dispatch
│   │   ├── iterate.rs              # Iterate-heavy
│   │   ├── step.rs                 # Step-heavy (direct participant)
│   │   ├── reset.rs                # Reset/State torture
│   │   └── mixed.rs                # Weighted blend
│   ├── rate/
│   │   ├── mod.rs                  # RateController trait
│   │   ├── token_bucket.rs         # Open-loop
│   │   ├── closed_loop.rs          # Fixed concurrency
│   │   └── hybrid.rs               # Open-with-cap
│   ├── metrics/
│   │   ├── mod.rs                  # PromExporter + registration
│   │   └── self_instrument.rs      # 5 STABLE series (AD16)
│   ├── tracing/
│   │   ├── mod.rs                  # OTel SDK init
│   │   └── span.rs                 # Loadgen-side spans (AD20)
│   ├── output/
│   │   ├── mod.rs                  # Output trait + multi-writer
│   │   ├── terminal.rs             # ratatui live TUI
│   │   ├── csv.rs
│   │   ├── ndjson.rs
│   │   └── parquet.rs              # arrow + parquet
│   ├── termination/
│   │   ├── mod.rs                  # TerminationPolicy trait
│   │   ├── duration.rs
│   │   ├── requests.rs
│   │   ├── slo.rs                  # Watches metrics until breach
│   │   ├── convergence.rs
│   │   └── signal.rs               # SIGINT / SIGTERM
│   ├── determinism/
│   │   ├── mod.rs                  # Seed propagation
│   │   └── xoshiro.rs              # PRNG wrapper
│   ├── profile/
│   │   ├── mod.rs                  # YAML loader + validation
│   │   └── schema.rs               # serde Deserialize types
│   └── replay/
│       ├── mod.rs                  # Replay engine
│       └── reader.rs               # CSV/parquet reader
├── examples/
│   ├── smoke-50qps-1min.yaml       # UC1
│   ├── slo-validation-5min.yaml    # UC2
│   ├── canary-50qps-10min.yaml     # UC3
│   ├── chaos-burst-30min.yaml      # UC4
│   ├── algorithm-mixed-1h.yaml     # UC6
│   └── compliance-quarterly.yaml   # UC7
├── tests/
│   ├── integration_test.rs         # Real-network kind-cluster test
│   ├── determinism_test.rs         # Same-seed-same-bytes assertion
│   ├── replay_test.rs              # Record-then-replay round-trip
│   ├── cli_golden_test.rs          # CLI flag golden snapshot
│   └── data/
│       ├── replay-fixture.csv
│       └── golden-cli.txt          # CLI --help golden output
└── helm/
    ├── Chart.yaml
    ├── values.yaml
    ├── values.schema.json
    └── templates/
        ├── job.yaml
        ├── cronjob.yaml            # Optional scheduled variant
        ├── service-metrics.yaml
        ├── servicemonitor.yaml
        ├── rbac.yaml
        ├── tls-cert.yaml           # cert-manager Certificate
        └── NOTES.txt
```

### 6.1 Library surface (`lib.rs`)

```rust
pub mod cli;
pub mod config;
pub mod client;
pub mod pattern;
pub mod workload;
pub mod rate;
pub mod metrics;
pub mod tracing;
pub mod output;
pub mod termination;
pub mod determinism;
pub mod profile;
pub mod replay;

/// Top-level entry — re-exported for embedded use in tests + future
/// Cargo benches.
pub use crate::config::LoadgenConfig;
pub use crate::client::ClientPool;
pub fn run(config: LoadgenConfig) -> Result<RunSummary, LoadgenError> { … }
```

### 6.2 Dependency graph

```
tools/loadgen
   ├── crates/efe-meta-swarm-proto     (proto types)
   ├── crates/efe-meta-swarm            (Tensor, MetaSwarmResult)
   ├── crates/efe-meta-swarm-server     (tracing_otel::propagator — optional)
   ├── tonic                            (gRPC client)
   ├── tokio                            (async runtime)
   ├── clap                             (CLI)
   ├── serde + serde_yaml + serde_json (profile + ndjson)
   ├── hdrhistogram                     (lock-free histogram)
   ├── parquet + arrow                  (parquet output)
   ├── ratatui + crossterm              (terminal TUI)
   ├── prometheus                       (self-metrics)
   ├── opentelemetry + opentelemetry_otlp (OTel SDK)
   ├── rustls + tokio-rustls            (mTLS — matches PR #16)
   ├── rand + rand_xoshiro              (deterministic PRNG)
   └── tracing + tracing-subscriber     (logging)
```

Cargo `Cargo.lock` shared with the rest of the workspace (AD29) — no
version drift.

---

## 7. CLI interface

The CLI uses `clap` with subcommands. Default subcommand : `run`.

### 7.1 Subcommands

```
USAGE :
    efe-meta-swarm-loadgen [SUBCOMMAND] [OPTIONS]

SUBCOMMANDS :
    run         Run a load test (default)
    validate    Dry-run config validation (no traffic)
    replay      Replay a recorded session
    compare     Compare two result files (parquet/csv)
    version     Print version + git_sha + rustc_version
```

### 7.2 Global flags (subset — full list in Appendix A)

```
TARGETING
    --target <URL>              gRPC endpoint (repeatable for multi-target)
    --target-role <ROLE>        controller | participant
    --conns-per-target <N>      HTTP/2 connections per target (default 4)
    --per-request-deadline <D>  Per-request gRPC deadline

WORKLOAD
    --profile <YAML>            Workload profile YAML
    --pattern <NAME>            constant | ramp | burst | diurnal | replay
    --workload <NAME>           iterate | step | reset | state | mixed
    --workload-mix <K=V,...>    Mixed weights, e.g., iterate=0.95,state=0.05

RATE
    --qps <QPS>                 Target queries/sec (default 10, cap 100k)
    --concurrency <N>           Closed-loop concurrency
    --rate-mode <MODE>          open | closed | hybrid (default hybrid)
    --ramp-up <DURATION>        Linear ramp-up (default 30s)

TERMINATION
    --duration <DURATION>       Max wall-clock (e.g., 5m, 1h)
    --requests <N>              Max total requests
    --until-slo <SLO=TARGET>    Run until SLO breach
    --shutdown-deadline <DUR>   Graceful drain (default 5s)

mTLS
    --tls-cert <PATH>           Client certificate (PEM)
    --tls-key <PATH>            Client private key (PEM)
    --tls-ca <PATH>             CA bundle (PEM)
    --expected-san <SAN>        Expected server SAN
    --tls-insecure              Disable cert verification (dev only)

OBSERVABILITY
    --prom-listen <ADDR>        Self-metrics endpoint (default 0.0.0.0:9095)
    --otel-endpoint <URL>       OTLP/gRPC collector
    --otel-sampler <FLOAT>      Trace sampling rate (default 1.0)
    --otel-service-name <NAME>  Resource attribute (default efe-meta-swarm-loadgen)
    --log-format <FORMAT>       text | json (default text)
    --log-level <LEVEL>         trace | debug | info | warn | error

OUTPUT
    --output-format <FORMAT>    terminal | json | csv | ndjson | parquet
    --output-file <PATH>        Result file path
    --record                    Record requests to result file for replay

DETERMINISM
    --seed <U64>                Deterministic seed (default = wall clock)

SAFETY
    --mode <MODE>               dev | staging | production
    --confirm-prod              Required if --mode=production
    --dry-run                   Validate config, no traffic
    --allow-mutation            Enable Step + Reset (default: read-only)
    --i-am-sure                 Unlock --qps > 1000

REPLAY
    --replay-file <PATH>        Path to recorded session
    --replay-mode <MODE>        preserve-spacing | accelerate | drain
    --replay-speed <FLOAT>      Multiplier (e.g., 2.0 = 2x speed)

CHAOS-AWARE (PR #21 dep)
    --chaos-marker <LABEL>      Tag spans with this fault marker
    --wait-for-signal <PATH>    Wait until file exists before starting
```

### 7.3 Flag conflict resolution

Mutually-exclusive flags (clap's `conflicts_with`) :

  - `--qps` ↔ `--concurrency` (use `--rate-mode=hybrid` for both)
  - `--duration` ↔ `--requests` (use the first to fire)
  - `--profile` overrides individual flags ; explicit CLI flags
    override the profile (precedence : CLI > profile > defaults).

### 7.4 `--dry-run` (AD18)

Validates the full configuration without sending traffic. Prints :

```
Loadgen config (dry-run) :
  target(s) : ctrl-prod.efe.svc:50051 (weight 1.0)
  workload  : iterate (100% mix)
  rate      : 50 QPS, hybrid mode, max concurrency 200
  duration  : 5m (or first SLO breach)
  TLS       : enabled (cert /etc/efe/tls/tls.crt, expected SAN controller.efe.svc)
  seed      : 1715864700
  mode      : staging

⚠  Would send ~ 15 000 requests over 5m. Confirm with `--no-dry-run` to execute.
```

Exit code 0 if config valid, 3 if not.

---

## 8. Configuration model

### 8.1 Precedence

```
CLI flags    >  Workload profile YAML  >  Environment variables  >  Built-in defaults
```

Each layer can override the one below. The merged result is the
`LoadgenConfig` struct passed to the engine.

### 8.2 Environment variables

```
EFE_LOADGEN_TARGET                  ↔  --target
EFE_LOADGEN_TLS_CERT                ↔  --tls-cert
EFE_LOADGEN_TLS_KEY                 ↔  --tls-key
EFE_LOADGEN_TLS_CA                  ↔  --tls-ca
EFE_LOADGEN_OTEL_ENDPOINT           ↔  --otel-endpoint
EFE_LOADGEN_PROM_LISTEN             ↔  --prom-listen
EFE_LOADGEN_LOG_LEVEL               ↔  --log-level
EFE_LOADGEN_MODE                    ↔  --mode
```

In-cluster pods get most of these from the Downward API (Helm chart auto-
injects them — § 38.4).

### 8.3 Merged config schema (Rust)

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LoadgenConfig {
    pub targets: Vec<TargetSpec>,
    pub workload: WorkloadSpec,
    pub pattern:  PatternSpec,
    pub rate:     RateSpec,
    pub termination: TerminationSpec,
    pub mtls:     Option<TlsConfig>,
    pub otel:     OtelConfig,
    pub prometheus: PrometheusConfig,
    pub output:   OutputConfig,
    pub seed:     u64,
    pub mode:     RunMode,
    pub confirmed_prod: bool,
    pub allow_mutation: bool,
    pub log_level: tracing::Level,
}
```

---

## 9. Traffic pattern engines

5 patterns. Each implements the `TrafficPattern` trait :

```rust
pub trait TrafficPattern: Send + Sync {
    /// Compute the target QPS at time `t` (seconds since start).
    fn target_qps(&self, t: f64) -> f64;

    /// Total expected requests over duration `total_duration`.
    fn expected_requests(&self, total_duration: f64) -> u64;
}
```

### 9.1 `constant` — fixed QPS

```rust
struct ConstantPattern { qps: f64 }
impl TrafficPattern for ConstantPattern {
    fn target_qps(&self, _t: f64) -> f64 { self.qps }
    fn expected_requests(&self, d: f64) -> u64 { (self.qps * d) as u64 }
}
```

Most common pattern. Default for `--pattern=constant`. Smoothed over
the ramp-up window (linear from 0 to qps over `--ramp-up` seconds).

### 9.2 `ramp` — linear or exponential ramp

```yaml
pattern:
  ramp:
    initial_qps: 1
    final_qps:   1000
    shape:       linear     # linear | exponential
    duration:    5m
```

Use for capacity ceiling discovery (ramp until SLO breach + `--until-slo`).

### 9.3 `burst` — token-bucket overshoot

```yaml
pattern:
  burst:
    baseline_qps: 10
    burst_qps:    500
    burst_duration: 500ms
    burst_interval: 1m       # one burst per minute
```

Tests the controller's tail-latency under sudden spikes. Per-spike
recovery latency is recorded in the parquet file.

### 9.4 `diurnal` — sinusoidal day/night

```yaml
pattern:
  diurnal:
    daily_min_qps: 10
    daily_max_qps: 200
    period: 24h               # or 1m for accelerated tests
    phase:  0h                # offset from start of run
```

Realistic-traffic pattern. Combined with `--replay-mode=accelerate` lets
operators run a "24h day in 5 minutes" regression test.

### 9.5 `replay` — from recorded CSV/parquet

```yaml
pattern:
  replay:
    file: /var/recordings/prod-2026-05-15.parquet
    mode: preserve-spacing   # preserve-spacing | accelerate | drain
    speed: 1.0
```

Reproduces a recorded session bit-identically (same payloads, same
inter-arrival times, same target distribution).

---

## 10. Workload shape catalogue

### 10.1 Iterate-heavy (default)

```yaml
workload:
  iterate:
    initial_state: random      # random | zero | from-file
    initial_state_dim: 256
    max_iters: 50
    max_wall_ms: 2000
```

100 % of requests are `Iterate` RPCs. Tests the controller's primary
path including participant fan-out. Default workload for SLO validation.

### 10.2 Step-heavy (bypass controller)

```yaml
workload:
  step:
    target_participants: [p-foo, p-bar]  # subset filter
    pulled_dim: 256
    initial_state: random
```

Direct participant traffic. Use for participant-side regression detection
(e.g., a new algorithm version that's faster server-side but slower at
step latency).

### 10.3 Reset / State torture

```yaml
workload:
  reset:
    reset_period: 100        # reset every 100th request
    state_ratio: 0.5         # 50% Reset, 50% State
```

Tests the controller's state machine + the participant's reset_gen
bookkeeping. Catches subtle bugs in the manifest version coupling.

### 10.4 Mixed (weighted blend)

```yaml
workload:
  mixed:
    iterate: 0.85    # 85% Iterate
    state:   0.10    # 10% State
    step:    0.05    # 5% direct Step
    reset:   0.0     # disabled
```

Operator-specifiable weights (must sum to 1.0). Most-realistic workload
for production SLO validation.

---

## 11. Rate control strategies

3 controllers. Each implements `RateController` :

```rust
#[async_trait]
pub trait RateController: Send + Sync {
    /// Block until the next request slot is available.  Returns the
    /// per-request deadline (None = no deadline).
    async fn acquire(&self) -> Option<Duration>;

    /// Update target rate (used by pattern engine).
    fn set_target_qps(&self, qps: f64);
}
```

### 11.1 Open-loop token bucket (`--rate-mode=open`)

Token bucket with capacity = burst_size, refill rate = target_qps.
`acquire()` returns immediately if tokens > 0, else blocks for the
refill interval. Bursty by design ; overflow drops or blocks based on
`--overflow-policy={drop,block}`.

### 11.2 Closed-loop concurrency (`--rate-mode=closed`)

Fixed N goroutines, each in a tight loop. `acquire()` is a no-op ;
the controller-side latency naturally throttles the effective QPS.
Use this for finding the true ceiling : `--concurrency=200` + measure
achieved QPS.

### 11.3 Hybrid (`--rate-mode=hybrid`) — DEFAULT

Open-loop token bucket WITH max concurrency cap. Best of both worlds :
target QPS is honoured up to the concurrency ceiling (typically =
2× target_qps × p99_latency). Falls back to closed-loop semantics
when the controller saturates.

### 11.4 Comparison table

| Mode | Strict QPS | Realistic | Use case |
|---|---|---|---|
| open    | ✓ | ✗ | Stress test : "what if rate stays high regardless of saturation ?" |
| closed  | ✗ | ✓ | Capacity ceiling : "what's the actual achieved QPS at N concurrency ?" |
| hybrid  | ✓ within cap | ✓ | Production-realistic load : default |

---

## 12. mTLS wiring

Loadgen mTLS configuration matches PR #16 exactly so the same
cert-manager-issued Certificate can be reused.

### 12.1 Cert format

```
/etc/efe-meta-swarm-loadgen/tls/
├── tls.crt           # client certificate (chain)
├── tls.key           # client private key (PKCS8 ECDSA-P256)
└── ca.crt            # CA bundle
```

Identical to PR #16's controller-side mount layout. Operators can mount
the same Secret if shared trust is acceptable, OR provision a dedicated
`Certificate` resource per loadgen run.

### 12.2 Verification

```rust
let tls_config = rustls::ClientConfig::builder()
    .with_root_certificates(load_ca(&config.tls.ca_path)?)
    .with_client_auth_cert(
        load_cert_chain(&config.tls.cert_path)?,
        load_key(&config.tls.key_path)?,
    )?;
```

`expected_san` validation : tonic's `Endpoint::tls_config` accepts a
`domain_name` parameter that maps to SNI + cert SAN check. Loadgen
applies it per target.

### 12.3 Cert rotation

Loadgen processes are short-lived (one Job per test run). Cert rotation
is operator's concern : if a run exceeds the cert validity window (rare),
the next run uses the fresh cert.

For long-running soak tests (Phase-3), loadgen will support inotify-
driven hot reload like PR #16 — explicitly out of scope for PR #20.

---

## 13. OTel trace integration

### 13.1 Root span emission

Every loadgen request emits a root span :

```
efe.meta_swarm.loadgen.iterate
├── kind: CLIENT
├── attributes :
│     loadgen.request_index: <u64>
│     loadgen.seed: <u64>
│     loadgen.run_id: <UUID>
│     loadgen.profile: <string>     # name from YAML
│     loadgen.mode: <dev|staging|production>
│     loadgen.target: <URL>
│     net.peer.name: <hostname>
│     net.peer.port: <u16>
├── status: ok | error
└── child : controller's `efe.meta_swarm.iterate` (via W3C propagation)
```

The `traceparent` header is injected by the tonic interceptor
(reuses PR #18's `inject_current_context`).

### 13.2 Service name

```
service.name = efe-meta-swarm-loadgen
service.namespace = efe                    # K8s namespace
service.instance.id = <hostname>           # pod name
deployment.environment = <dev|staging|production>
efe.loadgen.run_id = <UUID>                # unique per run
efe.loadgen.profile = <profile-name>
```

Operators filter traces by `service.name=efe-meta-swarm-loadgen` to
isolate loadgen-generated traffic from real production traffic.

### 13.3 Sampling

Default 100 % sampling at the loadgen → SDK boundary. The collector-
side tail sampler typically retains 1 % (matches PR #18 § 16).

Operators wanting full-trace inspection set `--otel-sampler=1.0` AND
configure the collector to forward all loadgen-tagged traces.

### 13.4 Exemplar correlation

Loadgen's Prometheus histograms attach the same `trace_id` as the
OTel span. PR #19's D2 P8 exemplar dots become clickable :

```
operator on D2 P8 → sees a red dot at p99
clicks → Tempo opens with the loadgen→controller→participant trace
sees the full chain end-to-end
```

---

## 14. Self-instrumentation Prometheus metrics

5 STABLE metrics in the `efe_msw_loadgen_*` namespace (STABILITY_CONTRACT v1.2 § 9.10).

### 14.1 Metric catalogue

| Metric | Type | Labels | Purpose |
|---|---|---|---|
| `efe_msw_loadgen_request_total` | Counter | `verb`, `result`, `target` | Per-verb request count |
| `efe_msw_loadgen_request_duration_seconds` | Histogram | `verb`, `result` | Per-verb latency (same buckets as server) |
| `efe_msw_loadgen_inflight` | Gauge | `verb` | Current in-flight requests |
| `efe_msw_loadgen_target_up` | Gauge | `target` | 1 if last response succeeded, 0 if last failed |
| `efe_msw_loadgen_build_info` | Gauge | `version`, `git_sha`, `rustc_version`, `build_date` | Always 1, label-only |

### 14.2 Label taxonomies (closed)

```
verb     : iterate | step | reset | state
result   : ok | error | deadline | canceled | throttled
target   : <URL>            # operator may opt out for cardinality
```

### 14.3 Cardinality budget

For a 3-target deployment, 5 verbs × 5 results × 3 targets = 75 series
per pod. Multi-pod adds cardinality only via Prometheus' instance label
(handled by relabel rules).

Hard cap : 200 series per loadgen pod. Loadgen emits a warning if
cardinality breaches the cap.

### 14.4 PromQL recipes (Appendix C)

```promql
# Loadgen-observed iterate p95 (last 5m)
histogram_quantile(0.95,
  sum by (le) (rate(efe_msw_loadgen_request_duration_seconds_bucket{verb="iterate"}[5m])))

# Loadgen vs server-side p95 discrepancy
histogram_quantile(0.95, sum by (le) (rate(efe_msw_loadgen_request_duration_seconds_bucket{verb="iterate"}[5m])))
  -
histogram_quantile(0.95, sum by (le) (rate(efe_meta_swarm_iterate_duration_seconds_bucket[5m])))
# Positive = loadgen sees longer latency than the server (network + queue time).
```

---

## 15. Determinism + seed propagation

### 15.1 Seed flow

```
--seed <U64> (default = wall_clock_ns)
   │
   ▼
xoshiro256+ root RNG  ────────────────────────────────────────────────
   │                                                                 │
   ├── sub-stream A : per-request target selection                  │
   ├── sub-stream B : per-request payload generation                │
   ├── sub-stream C : think-time / inter-arrival jitter             │
   └── sub-stream D : ramp-up jitter                                │
                                                                     │
   In multi-pod mode (AD8) :                                         │
     pod_seed = root_seed XOR JOB_COMPLETION_INDEX                   │
     each pod processes the same N/parallelism request slice         │
```

### 15.2 Reproducibility guarantee

Same `(seed, profile YAML, target list, loadgen version)` ⇒ bit-
identical wire bytes on every invocation (modulo wall-clock skew
at the target side).

This is the contractual property that CI relies on for golden-file
regression detection.

### 15.3 Counterexamples

Reproducibility breaks when :
- Wall-clock differs (`--duration` is wall-clock-bound)
- Target list ordering differs (Helm chart sorts deterministically)
- Loadgen version differs (build_info metric differs ⇒ flagged)
- Network delivers packets in different order (acceptable — the request
  *bytes* are identical even if reordered)

---

## 16. Output formats

### 16.1 Terminal TUI (default when isatty)

Live dashboard refreshed every 1 s :

```
┌─ efe-meta-swarm-loadgen 0.1.0 ─────────────── running 02:14 / 05:00 ─┐
│                                                                       │
│  Pattern    : constant 50 QPS    Target : ctrl.efe.svc:50051         │
│  Workload   : iterate (100%)     Mode   : staging                    │
│  Mode       : hybrid (cap 200)   Seed   : 1715864700                 │
│                                                                       │
│  ┌─ Live ─────────────────┐  ┌─ Targets ─────────────────────────┐   │
│  │ QPS sent  : 50.1       │  │ ctrl-1.efe.svc:50051   UP  100%  │   │
│  │ QPS recv  : 49.8       │  │ ctrl-2.efe.svc:50051   UP   98%  │   │
│  │ inflight  : 12         │  └───────────────────────────────────┘   │
│  └────────────────────────┘                                          │
│                                                                       │
│  ┌─ Latency ──────────────┐  ┌─ Errors ──────────────────────────┐   │
│  │ p50 :   8 ms           │  │ deadline_exceeded : 12  (0.04%)  │   │
│  │ p95 : 245 ms           │  │ participant_down  :  0           │   │
│  │ p99 : 980 ms           │  │ internal          :  1  (0.003%) │   │
│  └────────────────────────┘  └───────────────────────────────────┘   │
│                                                                       │
│  ┌─ SLO ──────────────────────────────────────────────────────────┐  │
│  │ iterate-availability : 99.96 %  ✓  (target 99.9 %)            │  │
│  │ iterate-latency-p95  : 245 ms   ✓  (target ≤ 1.0 s)           │  │
│  └────────────────────────────────────────────────────────────────┘  │
│                                                                       │
│  [q] Quit  [s] Snapshot  [p] Pause   Press '?' for help              │
└───────────────────────────────────────────────────────────────────────┘
```

Built on ratatui + crossterm. Snapshots a single-frame PNG via the `s`
key (writes to `/tmp/loadgen-snap-<UUID>.png`).

### 16.2 CSV (`--output-format=csv`)

```
ts_ns,verb,result,duration_ns,target,trace_id,error_msg
1715864700123456789,iterate,ok,12345678,ctrl-1.efe.svc:50051,abc123def456...,
1715864700133456789,iterate,error,2000000000,ctrl-1.efe.svc:50051,def456abc789...,deadline_exceeded
```

Human-readable, Excel-importable. One row per request. Large file size
at high QPS (~ 200 bytes/row × 50K QPS × 5 min = 3 GB).

### 16.3 ndjson (`--output-format=ndjson`)

```jsonl
{"ts":1715864700123,"verb":"iterate","result":"ok","duration_ns":12345678,"target":"ctrl-1","trace_id":"abc123","payload_bytes":1024}
{"ts":1715864700133,"verb":"iterate","result":"error","duration_ns":2000000000,"target":"ctrl-1","trace_id":"def456","error":"deadline_exceeded"}
```

Streamable. Default for non-TTY runs (e.g., piped to a log aggregator).

### 16.4 Parquet (`--output-format=parquet`, DEFAULT)

Columnar, compressed (snappy by default). Typical 50K-QPS-5min run :
~ 30 MB parquet vs 3 GB CSV.

Schema :
```
ts                  TIMESTAMP_NS (logical TIMESTAMP_NANOS)
verb                STRING (dictionary-encoded)
result              STRING (dictionary-encoded)
duration_ns         INT64
target              STRING (dictionary-encoded)
trace_id            FIXED_LEN_BYTE_ARRAY(16)
payload_bytes       INT32
error_msg           STRING (null when result=ok)
```

CI consumes via `duckdb`:
```bash
duckdb -c "SELECT verb, COUNT(*), quantile(duration_ns/1e6, 0.99) AS p99_ms
           FROM '/tmp/result.parquet'
           GROUP BY verb"
```

---

## 17. Termination policies

5 policies. Each implements `TerminationPolicy` :

```rust
#[async_trait]
pub trait TerminationPolicy: Send + Sync {
    /// Called every 100ms during a run. Returns Some(reason) to stop.
    async fn should_stop(&self, stats: &RunStats) -> Option<StopReason>;
}
```

### 17.1 Duration (`--duration=5m`)

Stops when `wall_clock - start > duration`. Most common termination.

### 17.2 Request count (`--requests=10000`)

Stops when total request count reaches the threshold. Useful for
deterministic, low-rate tests.

### 17.3 Until SLO breach (`--until-slo=iterate-availability=0.999`)

Watches loadgen-side metrics and stops the moment any SLO is violated.
Used to find capacity ceilings (ramp pattern + until-slo).

Multiple SLOs : `--until-slo iterate-availability=0.999 --until-slo iterate-latency-p95=1.0`.

### 17.4 Convergence

Stops when the iterate convergence error stays within a tolerance for
N consecutive iterations. Useful for algorithm validation.

### 17.5 Signal (always active)

SIGINT / SIGTERM triggers graceful shutdown (drain in-flight requests,
flush metrics + OTel, write result file).

SIGKILL bypasses — operator's choice ; result file may be incomplete.

### 17.6 Composition

Termination policies are stacked — the FIRST one to fire wins.

```yaml
termination:
  duration: 5m              # max time
  requests: 1000000         # max count
  until_slo:                # safety
    iterate-availability: 0.999
```

In this example, the run ends at `min(5m, 1M requests, first SLO breach)`.

---

## 18. Workload profile YAML schema (`version: 1`)

```yaml
# Profile schema version. PR #20 ships version 1.
version: 1

name: "canary-50qps-10min"
description: "50 QPS for 10 minutes against the canary slice"

target:
  endpoints:
    - url: "grpc://ctrl-canary.efe.svc:50051"
      weight: 1.0
  role: controller
  conns_per_target: 4
  per_request_deadline: 3s
  mtls:
    cert_path: /etc/efe/tls/tls.crt
    key_path:  /etc/efe/tls/tls.key
    ca_path:   /etc/efe/tls/ca.crt
    expected_san: "ctrl-canary.efe.svc"

pattern:
  type: constant
  qps: 50
  ramp_up: 30s

workload:
  type: mixed
  mix:
    iterate: 0.95
    state:   0.05

rate:
  mode: hybrid
  max_concurrency: 200

termination:
  duration: 10m
  until_slo:
    iterate-availability: 0.999
    iterate-latency-p95: 1.0

output:
  format: parquet
  file: /var/run/loadgen/canary-result.parquet
  record: true            # enables future replay

observability:
  prometheus:
    listen: "0.0.0.0:9095"
  otel:
    endpoint: "http://otel-collector.observability.svc:4317"
    sampler_arg: 1.0
    service_name: "efe-meta-swarm-loadgen-canary"
  log:
    format: json
    level: info

run:
  seed: 42
  mode: staging
  allow_mutation: false
  chaos_marker: null
```

Full schema validation via JSON-Schema (committed at
`tools/loadgen/schemas/profile-v1.json`).

---

## 19. Multi-pod orchestration (Kubernetes Job indexed completion)

### 19.1 Helm Job template

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: efe-msw-loadgen-{{ .Values.profile.name }}
spec:
  parallelism: {{ .Values.parallelism }}      # number of pods
  completions:  {{ .Values.parallelism }}     # complete when all done
  completionMode: Indexed                     # each pod gets ordinal
  backoffLimit: {{ .Values.backoffLimit }}
  template:
    spec:
      serviceAccountName: efe-msw-loadgen
      restartPolicy: Never
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
        seccompProfile: { type: RuntimeDefault }
      containers:
        - name: loadgen
          image: {{ .Values.image.repository }}:{{ .Values.image.tag }}
          imagePullPolicy: IfNotPresent
          args:
            - run
            - --profile
            - /etc/profile/profile.yaml
          env:
            - name: POD_ORDINAL
              valueFrom: { fieldRef: { fieldPath: metadata.annotations['batch.kubernetes.io/job-completion-index'] } }
            - name: POD_NAME
              valueFrom: { fieldRef: { fieldPath: metadata.name } }
            - name: PARALLELISM
              value: "{{ .Values.parallelism }}"
            - name: EFE_LOADGEN_OTEL_ENDPOINT
              value: {{ .Values.otel.endpoint }}
          ports:
            - { name: metrics, containerPort: 9095 }
          resources: { ... }
          volumeMounts:
            - { name: profile, mountPath: /etc/profile }
            - { name: tls, mountPath: /etc/efe/tls, readOnly: true }
            - { name: results, mountPath: /var/run/loadgen }
      volumes:
        - { name: profile, configMap: { name: efe-msw-loadgen-profile } }
        - { name: tls, secret: { secretName: efe-msw-loadgen-tls } }
        - { name: results, emptyDir: {} }
```

### 19.2 Seed partitioning

```
root_seed = config.seed
pod_seed  = root_seed XOR pod_ordinal
```

Each pod operates independently — no cross-pod state. Aggregation is
post-run via the aggregator (next section).

### 19.3 Result file naming

```
/var/run/loadgen/<profile-name>-pod-<ordinal>-of-<parallelism>.parquet
```

Each pod writes its own file ; aggregator stitches them.

---

## 20. Result aggregation

### 20.1 Aggregator pod (separate Job)

```yaml
# helm/templates/aggregator-job.yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: efe-msw-loadgen-aggregate-{{ .Values.profile.name }}
spec:
  template:
    spec:
      containers:
        - name: aggregator
          image: {{ .Values.image.repository }}:{{ .Values.image.tag }}
          args:
            - aggregate
            - --pattern
            - "/var/run/loadgen/*.parquet"
            - --output
            - /var/run/loadgen/aggregated.parquet
            - --emit-slo-report
            - --output-format
            - json
```

### 20.2 Aggregation algorithm

```
aggregate :
  1. Read all `*.parquet` files.
  2. Concat by `ts` column.
  3. Compute global p50/p95/p99 via t-digest sketch (or hdrhistogram).
  4. Compute global error rate by `verb`.
  5. Check each SLO target.
  6. Emit `aggregated.parquet` + `slo-report.json`.
```

### 20.3 SLO report format

```json
{
  "run_id": "...",
  "duration_actual_s": 600.42,
  "total_requests": 30025,
  "by_verb": {
    "iterate": {
      "count": 28524,
      "ok": 28491,
      "error": 33,
      "p50_ms": 8,
      "p95_ms": 245,
      "p99_ms": 980,
      "slo_iterate_availability_target": 0.999,
      "slo_iterate_availability_actual": 0.99884,
      "slo_iterate_availability_pass": false
    }
  },
  "overall_pass": false,
  "exit_code": 1
}
```

CI inspects `overall_pass` and `exit_code` ; PRs that flip it from
true → false fail.

---

## 21. Replay mode

### 21.1 Recording

```bash
efe-meta-swarm-loadgen run \
    --profile examples/canary-50qps-10min.yaml \
    --output-file /tmp/prod-2026-05-16.parquet \
    --record
```

Captures every request : ts, verb, payload (or tensor seed), target,
result. Suitable for re-execution.

### 21.2 Replay

```bash
efe-meta-swarm-loadgen replay \
    --replay-file /tmp/prod-2026-05-16.parquet \
    --replay-mode preserve-spacing \
    --target grpc://ctrl-staging.efe.svc:50051
```

Replays the recorded session against a NEW target. Same payloads, same
spacing, same target weights.

### 21.3 Acceleration

```bash
efe-meta-swarm-loadgen replay \
    --replay-file /tmp/prod-24h.parquet \
    --replay-mode accelerate \
    --replay-speed 100   # 24h → ~14m
```

Time-compresses the replay : useful for regression testing a full day
of production traffic in minutes.

### 21.4 Drain mode

```bash
--replay-mode drain    # ignores spacing, floods at maximum rate
```

Stress-tests : "if a backlog flushes all at once, what happens ?"

### 21.5 Privacy

Recorded files contain request metadata + tensor seeds. Tensor values
themselves are deterministically derived from seed at replay time —
no PII in the payload. `--record` produces files safe for cross-cluster
sharing.

---

## 22. Distribution

### 22.1 Docker image

```dockerfile
# Multi-stage build for minimal final image
FROM rust:1.83 AS builder
WORKDIR /build
COPY . .
RUN cargo build --locked --release -p efe-meta-swarm-loadgen

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /build/target/release/efe-meta-swarm-loadgen /usr/local/bin/
USER nonroot
ENTRYPOINT ["/usr/local/bin/efe-meta-swarm-loadgen"]
```

Published to `ghcr.io/efe-ai/efe-meta-swarm-loadgen:<version>` on every
release. Multi-arch (amd64 + arm64) via `docker buildx`.

### 22.2 Helm chart

Standalone chart at `charts/efe-meta-swarm-loadgen/`. Not a sub-chart
of `efe-meta-swarm` — operationally separable (loadgen pods may run
in a different namespace from controllers).

### 22.3 GitHub release artefact

```
efe-meta-swarm-loadgen-v0.1.0-x86_64-linux.tar.gz
efe-meta-swarm-loadgen-v0.1.0-aarch64-linux.tar.gz
efe-meta-swarm-loadgen-v0.1.0-x86_64-darwin.tar.gz
efe-meta-swarm-loadgen-v0.1.0-aarch64-darwin.tar.gz
```

Each tarball : binary + LICENSE + README + 4 example profiles.
Signed with cosign (sigstore — tools/sigstore exists already in repo).

---

## 23. STABILITY_CONTRACT v1.2 additions

### 23.1 New tier — Loadgen self-metric

Added in STABILITY_CONTRACT v1.2 § 9.10 :

| Tier | Promise | Breaking-change notice |
|---|---|---|
| **Loadgen self-metric** | `efe_msw_loadgen_*` series names + label sets frozen for ≥ 6 chart-minor versions | 6-minor-version notice |

### 23.2 Frozen surface — 5 STABLE series

```
efe_msw_loadgen_request_total            (Counter, labels : verb, result, target)
efe_msw_loadgen_request_duration_seconds (Histogram, labels : verb, result)
efe_msw_loadgen_inflight                 (Gauge, labels : verb)
efe_msw_loadgen_target_up                (Gauge, labels : target)
efe_msw_loadgen_build_info               (Gauge, label-only : version/git_sha/rustc_version/build_date)
```

### 23.3 Closed taxonomies

`verb` (loadgen) :
```
iterate | step | reset | state
```

Inherits `result` and `failure_reason` from STABILITY_CONTRACT v1.0 § 3.

### 23.4 New profile-YAML version contract

`version: 1` profiles are committed for ≥ 6 chart-minor versions. Future
schema changes bump to `version: 2` with migration tooling.

---

## 24. Security model + Kubernetes RBAC

### 24.1 ServiceAccount + Role

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata: { name: efe-msw-loadgen, namespace: efe }
rules:
  - apiGroups: [""]
    resources: ["services", "endpoints"]
    verbs: ["get", "list", "watch"]
  # NO write verbs. NO ClusterRole. NO admission to other namespaces.
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata: { name: efe-msw-loadgen, namespace: efe }
roleRef:
  kind: Role
  name: efe-msw-loadgen
  apiGroup: rbac.authorization.k8s.io
subjects:
  - kind: ServiceAccount
    name: efe-msw-loadgen
    namespace: efe
```

### 24.2 PodSecurityContext (AD35)

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 65532
  runAsGroup: 65532
  fsGroup: 65532
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities: { drop: [ALL] }
  seccompProfile: { type: RuntimeDefault }
```

Result file written to an `emptyDir` volume — host filesystem untouched.

### 24.3 NetworkPolicy

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: efe-msw-loadgen, namespace: efe }
spec:
  podSelector: { matchLabels: { app: efe-msw-loadgen } }
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
        - podSelector: { matchLabels: { app.kubernetes.io/name: prometheus } }
      ports: [{ port: 9095 }]
  egress:
    - to:
        - podSelector: { matchLabels: { app.kubernetes.io/name: efe-meta-swarm-controller } }
      ports: [{ port: 50051 }]
    - to:
        - podSelector: { matchLabels: { app.kubernetes.io/name: otel-collector } }
      ports: [{ port: 4317 }]
    # Optional : participant access via --target-role=participant
    - to:
        - podSelector: { matchLabels: { app.kubernetes.io/name: efe-meta-swarm-participant } }
      ports: [{ port: 50052 }]
```

---

## 25. Threat-model coverage (T76 – T85)

| ID | Threat | Severity | Mitigation | AC |
|---|---|---|---|---|
| T76 | Production saturation via misconfigured loadgen | SEV-2 | `--mode=production` + `--confirm-prod` + `--qps` default 10 + hard cap 100K (AD5/AD6/AD18) | AC10 |
| T77 | mTLS cred leak via verbose logs | SEV-3 | `--log-level=info` default ; cert paths logged but not file contents ; structured logging (no `{:?}` of `TlsConfig`) | AC12 |
| T78 | Abandoned loadgen pod | SEV-2 | Kubernetes Job (one-shot) + `activeDeadlineSeconds` default 4h ; SIGTERM handler with graceful drain | AC15 |
| T79 | Cross-tenant traffic injection | SEV-2 | RBAC : namespace-scoped only ; NetworkPolicy : egress to specific labels | AC18 |
| T80 | Trace ID replay attack | SEV-3 | trace_ids regenerated per run ; never reused across runs (AD20 run_id uniqueness) | AC11 |
| T81 | Recording file payload exfil | SEV-3 | Tensor values deterministic from seed (not raw bytes) ; profile YAML never contains payload data | AC22 |
| T82 | Loadgen pod as pivot | SEV-2 | distroless + nonroot + readOnlyRootFilesystem + drop ALL capabilities (AD35) | AC18 |
| T83 | Controller resource exhaustion | SEV-2 | Default QPS cap 10 ; hard pod-level cap 100K ; ramp-up 30s ; per-request deadline | AC10 |
| T84 | Rate-control bypass via `--qps=infinity` | SEV-2 | clap parser rejects non-finite values ; hard cap enforced post-parse | AC10 |
| T85 | Over-permissive Helm RBAC | SEV-3 | helm-unittest assertions on Role rules (no ClusterRole, no write verbs) | AC18 |

All ≤ SEV-2. No PII leak. No auth bypass. No RCE.

---

## 26. CI integration

### 26.1 New workflow `.github/workflows/pr20-loadgen.yml`

```yaml
name: pr20-loadgen
on:
  pull_request:
    paths:
      - 'tools/loadgen/**'
      - 'crates/efe-meta-swarm-proto/**'
      - 'docs/EFE_A1_PHASE2_PR20_ARCHITECTURE.md'
      - '.github/workflows/pr20-loadgen.yml'
  push:
    branches: [main]

jobs:
  cargo-test:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: Swatinem/rust-cache@v2
      - run: cargo test -p efe-meta-swarm-loadgen --all-features
      - run: cargo clippy -p efe-meta-swarm-loadgen -- -D warnings

  cli-golden:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - run: cargo run -p efe-meta-swarm-loadgen -- --help > /tmp/help.txt
      - run: diff tools/loadgen/tests/data/golden-cli.txt /tmp/help.txt

  loadgen-smoke:
    runs-on: ubuntu-22.04
    needs: cargo-test
    steps:
      - uses: actions/checkout@v4
      - uses: helm/kind-action@v1
        with: { cluster_name: pr20-smoke }
      - name: Install meta-swarm + loadgen
        run: |
          helm install efe-meta-swarm ./charts/efe-stack/charts/efe-meta-swarm \
              --namespace efe --create-namespace \
              --set enabled=true --set image.tag=ci-test
          helm install efe-loadgen ./charts/efe-meta-swarm-loadgen \
              --namespace efe \
              --set image.tag=ci-test \
              --set profile.yaml="$(cat tools/loadgen/examples/smoke-50qps-1min.yaml)"
      - name: Wait + assert
        run: |
          kubectl wait --for=condition=complete --timeout=120s job/efe-msw-loadgen-smoke -n efe
          kubectl logs job/efe-msw-loadgen-smoke -n efe | grep -q '"overall_pass":true'

  helm-unittest:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - run: |
          docker run --rm -v "$PWD":/apps \
              quintush/helm-unittest:3.9.4-0.3.5 \
              --strict charts/efe-meta-swarm-loadgen
```

### 26.2 SLO regression workflow

`.github/workflows/pr20-loadgen-slo-regression.yml` runs nightly on
main + on every PR labelled `slo-regression` :

```yaml
- run: |
    efe-meta-swarm-loadgen run \
        --profile tools/loadgen/examples/slo-validation-5min.yaml \
        --output-format json \
        --output-file /tmp/slo-result.json
    # Exit code 0 = pass, 1 = SLO breach
```

Posts a comment on the PR with the SLO report.

---

## 27. Performance budget

### 27.1 Per-pod QPS ceiling

Target : **50 000 QPS sustained on a 4-core / 1 GiB pod**.

Achieved via :
- tokio multi-thread runtime
- HTTP/2 multiplexing (1 TCP connection, N streams)
- hdrhistogram (lock-free per-thread aggregation)
- 1 s stats batch (avoids stdout flooding)

### 27.2 Latency overhead

Loadgen adds ≤ 10 µs to the measured latency (subtracted from server-
observed latency = network + queue time only).

Validation : `--mode=dev --target=loopback` with controller and loadgen
on the same host ; compare loadgen-observed p50 with `efe-bench`
in-process p50.

### 27.3 Memory ceiling

512 MiB at 50K QPS sustained. Breakdown :
- 100 MiB : in-flight tonic Request futures
- 80 MiB  : hdrhistogram + counters
- 50 MiB  : parquet writer buffer
- 30 MiB  : ratatui TUI buffer
- 252 MiB : binary + runtime + headroom

### 27.4 Trace overhead

At `--otel-sampler=1.0` : ~ 200 ns per request to create the span +
inject the traceparent header. At 50K QPS = 1 % CPU overhead. Acceptable.

### 27.5 Network overhead

Loadgen-side `--prom-listen` scraped at 15 s default. 200 series × 1 KiB =
200 KiB / 15 s = 13 KiB/s per pod. Negligible.

---

## 28. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Loadgen accidentally pointed at prod | Medium | SEV-2 | `--mode=production` confirmation gate (AD18) ; audit log entry |
| Pod abandoned + indefinite runtime | Low | Resource leak | `activeDeadlineSeconds: 4h` on the Job ; `--duration` mandatory in profile |
| QPS limit bypass via env var override | Low | Production saturation | env var precedence < CLI < built-in cap ; hard cap 100K regardless |
| Multi-pod aggregation race | Low | Wrong SLO result | Aggregator runs only after Job `Complete` condition (Job dependency in Helm) |
| OTel collector saturation | Medium | Trace loss | Sampler default 1.0 at loadgen → 0.01 at collector ; documented |
| Cert rotation mid-run | Low | Cert expired | Short runs (< cert lifetime) ; long runs split into multiple Jobs |
| CI flakiness (kind cluster) | Medium | False CI failures | Smoke test on a tiny profile (50 QPS / 1 min) ; retry up to 3× |
| Parquet schema drift | Low | Read failures | Schema versioning in profile YAML (`version: 1`) ; STABILITY_CONTRACT v1.2 |
| Replay file size unbounded | Medium | Disk full | `--record-sampling-rate=0.01` for production captures |
| ratatui terminal corruption on SIGKILL | Low | UX | Graceful drain handles SIGINT/SIGTERM ; SIGKILL is operator's choice |

---

## 29. Rollback strategy

### 29.1 Code rollback

```bash
git revert <commit-range>
cargo build -p efe-meta-swarm-loadgen
```

No runtime state — loadgen is stateless. Rolling back the binary version
in the Helm chart is a single `helm upgrade --set image.tag=<previous>`.

### 29.2 Chart rollback

```bash
helm rollback efe-meta-swarm-loadgen <previous-revision>
```

Removes the Job + RBAC + ConfigMap + ServiceMonitor.

### 29.3 No data loss

Result files in `emptyDir` evaporate with the pod ; persistent storage
is operator's choice (PVC or copy-out via `kubectl cp` post-run).

---

## 30. Sequence diagrams

### 30.1 Open-loop single-pod

```
operator ──helm install──> Kubernetes ──> pod
                                          │
                              ┌───────────┴───────────┐
                              ▼                       ▼
                       loadgen binary          metrics endpoint
                              │                       │
            ┌─────────────────┴─────────────────┐    │
            ▼                                   ▼    │
  TrafficPattern.tick(t)              tokio::spawn   │
            │                            (1 req)     │
            ▼                                │       │
       RateController                        ▼       │
       .acquire().await                Iterate RPC ──┼──> controller
            │                                │       │
            └────────────────────────────────┘       │
                                                     │
                                  Prometheus scrape ─┘
```

### 30.2 Multi-pod parallelism = 3

```
Job (parallelism=3, completionMode=Indexed)
   │
   ├── pod-0 (ordinal=0, seed = root XOR 0) ──> 16 666 reqs ──> result-0.parquet
   ├── pod-1 (ordinal=1, seed = root XOR 1) ──> 16 666 reqs ──> result-1.parquet
   └── pod-2 (ordinal=2, seed = root XOR 2) ──> 16 668 reqs ──> result-2.parquet

   All complete  ⇒  aggregator Job triggers (parallelism=1)
                       │
                       └──> reads 3 parquet files, computes global stats
                                          │
                                          └──> aggregated.parquet + slo-report.json
                                                                   │
                                                                   └──> exit code → CI
```

### 30.3 Replay flow

```
recorded.parquet ──> ReplayEngine
                          │
              ┌───────────┴────────────┐
              ▼                        ▼
       Iterator<RecordRow>      timing scheduler
              │                        │
              ▼                        ▼
       gRPC request               wall-clock spacing
              │                        │
              └────────┬───────────────┘
                       ▼
                  target (new!)
                       │
                       ▼
              Same wire payloads, same spacing.
              Different target. Different result.
```

---

## 31. Reviewer checklist

> Tick every box before LGTM.

**Foundations**
  - [ ] AD1–AD40 reviewed for consistency with PR #18/#19 conventions
  - [ ] STABILITY_CONTRACT v1.2 diff reviewed (§ 23 / new tier)
  - [ ] Workspace member added to root `Cargo.toml`

**Rust binary**
  - [ ] 5 traffic patterns implemented + tested
  - [ ] 4 workload shapes implemented + tested
  - [ ] 3 rate controllers implemented + tested
  - [ ] 5 termination policies implemented + tested
  - [ ] 4 output formatters implemented + tested
  - [ ] mTLS matches PR #16 cert format
  - [ ] OTel propagation matches PR #18 W3C
  - [ ] Self-metrics match STABILITY_CONTRACT v1.2 names + labels
  - [ ] CLI flags match Appendix A
  - [ ] Exit codes per AD17

**Helm chart**
  - [ ] Chart.yaml v0.1.0
  - [ ] values.schema.json validates the profile YAML
  - [ ] RBAC : Role-only, namespace-scoped (no ClusterRole, no write verbs)
  - [ ] PodSecurityContext per AD35
  - [ ] NetworkPolicy egress restricted
  - [ ] helm-unittest assertions on every template

**Distribution**
  - [ ] Distroless Docker image (multi-arch)
  - [ ] Multi-arch GitHub release tarballs
  - [ ] cosign signature on every artefact

**CI**
  - [ ] `pr20-loadgen.yml` workflow added + green
  - [ ] `pr20-loadgen-slo-regression.yml` workflow added
  - [ ] Golden CLI flag snapshot
  - [ ] kind-cluster smoke test green
  - [ ] Cargo lints clean (`-D warnings`)

**Docs**
  - [ ] Architecture (this doc, ≥ 2 500 lines)
  - [ ] tools/loadgen/README.md
  - [ ] STABILITY_CONTRACT v1.2 entry
  - [ ] 4 example workload profiles (UC1, UC2, UC3, UC6)

---

## 32. Acceptance criteria (AC1 – AC32)

| # | Criterion | Verified by |
|---|---|---|
| AC1 | Workspace member `tools/loadgen` exists + builds | `cargo build` |
| AC2 | Binary name is `efe-meta-swarm-loadgen` | `cargo metadata` |
| AC3 | `--help` golden snapshot matches | CI cli-golden job |
| AC4 | 5 traffic patterns selectable via `--pattern` | unit tests |
| AC5 | 4 workload shapes selectable via `--workload` | unit tests |
| AC6 | 3 rate modes selectable via `--rate-mode` | unit tests |
| AC7 | 5 termination policies selectable | unit tests |
| AC8 | 4 output formats (terminal/csv/ndjson/parquet) | unit tests |
| AC9 | `--dry-run` validates config, exits 0 | manual + CI |
| AC10 | `--mode=production` requires `--confirm-prod` ; hard QPS cap enforced | unit test + integration |
| AC11 | OTel span emitted per request ; `traceparent` injected | trace-roundtrip integration test |
| AC12 | mTLS matches PR #16 cert format ; no cert content in logs | unit test + grep |
| AC13 | Self-metrics on port 9095 ; 5 STABLE series | curl `/metrics` in integration test |
| AC14 | Same seed = bit-identical wire payloads | determinism_test.rs |
| AC15 | Graceful drain on SIGINT/SIGTERM within `--shutdown-deadline` | integration test |
| AC16 | Exit codes per AD17 | unit tests |
| AC17 | Helm chart `efe-meta-swarm-loadgen` v0.1.0 | helm template |
| AC18 | RBAC namespace-scoped only ; no ClusterRole | helm-unittest |
| AC19 | PodSecurityContext per AD35 | helm-unittest |
| AC20 | NetworkPolicy egress restricted to controllers + collector + participants | helm-unittest |
| AC21 | Multi-pod : `parallelism > 1` works ; each pod gets unique sub-seed | integration test |
| AC22 | Replay round-trip : record → replay → bit-identical results | replay_test.rs |
| AC23 | Parquet result file readable by duckdb | CI script |
| AC24 | Workload profile YAML schema `version: 1` validated via JSON-Schema | unit test |
| AC25 | 6 example profiles (UC1–UC7 except UC4/UC5) | doc check + lint |
| AC26 | STABILITY_CONTRACT v1.2 introduces the loadgen self-metric tier | doc diff |
| AC27 | Docker image distroless, multi-arch (amd64+arm64) | docker manifest inspect |
| AC28 | `pr20-loadgen.yml` CI workflow exists + passes | CI green |
| AC29 | `pr20-loadgen-slo-regression.yml` CI workflow exists | CI present |
| AC30 | Cargo workspace lockfile shared (no new `Cargo.lock` at `tools/loadgen/`) | filesystem check |
| AC31 | Doc parity : PR #20 architecture ≥ 2 500 lines | `wc -l` |
| AC32 | No new top-level chart conflicts with `efe-stack` | helm template diff |

---

## 33. Tests strategy

### 33.1 Unit tests (`cargo test`)

Per-module tests covering :

- 5 traffic patterns × N qps scenarios = ~ 30 tests
- 4 workload shapes × valid/invalid configs = ~ 20 tests
- 3 rate controllers × open/closed/hybrid edge cases = ~ 25 tests
- 5 termination policies = ~ 15 tests
- Output formatters round-trips = ~ 12 tests
- mTLS handshake (with mock server) = ~ 8 tests
- OTel span attribute correctness = ~ 6 tests
- Determinism (same seed = same bytes) = ~ 4 tests
- CLI parser (clap) = ~ 30 tests

Total ~ 150 unit tests, < 5 s wall-clock.

### 33.2 Integration tests (`cargo test --test integration_test`)

Stands up an in-process controller (mock or efe-meta-swarm-server in
testcontainer) + loadgen against it :

- Smoke : 10 reqs, 1 verb, no TLS = ~ 1 s
- With mTLS : 10 reqs over TLS = ~ 2 s
- With OTel : assert spans flow to a Tempo mock = ~ 3 s
- Multi-target weighted = ~ 1 s
- Replay round-trip = ~ 2 s

Total ~ 30 s wall-clock.

### 33.3 Golden CLI snapshot (`tests/cli_golden_test.rs`)

```rust
#[test]
fn cli_help_matches_golden() {
    let output = Command::new(env!("CARGO_BIN_EXE_efe-meta-swarm-loadgen"))
        .arg("--help")
        .output()
        .unwrap();
    let expected = include_str!("data/golden-cli.txt");
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}
```

Catches CLI flag accidents. Update via `cargo run -- --help > tests/data/golden-cli.txt`.

### 33.4 Determinism test

```rust
#[test]
fn same_seed_same_bytes() {
    let cfg = LoadgenConfig { seed: 42, ..test_config() };
    let bytes_a = render_request_bytes(&cfg);
    let bytes_b = render_request_bytes(&cfg);
    assert_eq!(bytes_a, bytes_b);
}
```

### 33.5 kind-cluster e2e

CI workflow `pr20-loadgen.yml` Job `loadgen-smoke` :
- Install efe-meta-swarm chart + efe-meta-swarm-loadgen chart in kind
- Wait for Job complete
- Assert `overall_pass: true` in the JSON log output

Total wall-time ~ 60 s.

### 33.6 Threat-model tests

- T84 : `--qps=1e9` → CLI parser rejects (unit test)
- T76 : `--mode=production` without `--confirm-prod` → exit 3 (integration)
- T82 : Docker image scanned with `trivy` in CI

---

## 34. Appendix A — exhaustive CLI flag reference

### A.1 Subcommand `run` (default)

| Flag | Type | Default | Purpose |
|---|---|---|---|
| `--target <URL>` | repeatable URL | — | gRPC endpoint |
| `--target-role <ROLE>` | enum | controller | controller / participant |
| `--conns-per-target <N>` | u32 | 4 | HTTP/2 connections per target |
| `--per-request-deadline <D>` | duration | none | gRPC `grpc-timeout` per request |
| `--profile <YAML>` | path | — | Workload profile YAML |
| `--pattern <NAME>` | enum | constant | constant / ramp / burst / diurnal / replay |
| `--workload <NAME>` | enum | iterate | iterate / step / reset / state / mixed |
| `--workload-mix <K=V,...>` | k=v list | — | Mixed weights |
| `--qps <QPS>` | f64 | 10 | Target QPS (hard cap 100K) |
| `--concurrency <N>` | u32 | — | Closed-loop concurrency |
| `--rate-mode <MODE>` | enum | hybrid | open / closed / hybrid |
| `--ramp-up <DURATION>` | duration | 30s | Linear ramp-up |
| `--duration <DURATION>` | duration | — | Max wall-clock |
| `--requests <N>` | u64 | — | Max request count |
| `--until-slo <SLO=VAL>` | repeatable | — | Run until SLO breach |
| `--shutdown-deadline <DUR>` | duration | 5s | Graceful drain timeout |
| `--tls-cert <PATH>` | path | — | Client cert (PEM) |
| `--tls-key <PATH>` | path | — | Client key (PEM) |
| `--tls-ca <PATH>` | path | — | CA bundle (PEM) |
| `--expected-san <SAN>` | string | — | Expected server SAN |
| `--tls-insecure` | bool | false | Disable cert verification |
| `--prom-listen <ADDR>` | addr | 0.0.0.0:9095 | Self-metrics endpoint |
| `--otel-endpoint <URL>` | URL | — | OTLP/gRPC collector |
| `--otel-sampler <FLOAT>` | f64 | 1.0 | Trace sampling rate |
| `--otel-service-name <NAME>` | string | efe-meta-swarm-loadgen | Resource attribute |
| `--log-format <FORMAT>` | enum | text | text / json |
| `--log-level <LEVEL>` | enum | info | trace / debug / info / warn / error |
| `--output-format <FORMAT>` | enum | parquet | terminal / json / csv / ndjson / parquet |
| `--output-file <PATH>` | path | — | Result file |
| `--record` | bool | false | Record requests for replay |
| `--seed <U64>` | u64 | wall_clock_ns | Deterministic seed |
| `--mode <MODE>` | enum | dev | dev / staging / production |
| `--confirm-prod` | bool | false | Required for production mode |
| `--dry-run` | bool | false | Validate config, no traffic |
| `--allow-mutation` | bool | false | Enable Step + Reset |
| `--i-am-sure` | bool | false | Unlock --qps > 1000 |
| `--chaos-marker <LABEL>` | string | — | Tag spans with this marker |
| `--wait-for-signal <PATH>` | path | — | Wait until file exists |
| `--version` | bool | — | Print version |
| `--help` | bool | — | Print help |

### A.2 Subcommand `replay`

| Flag | Default | Purpose |
|---|---|---|
| `--replay-file <PATH>` | required | Recorded session file |
| `--replay-mode <MODE>` | preserve-spacing | preserve-spacing / accelerate / drain |
| `--replay-speed <FLOAT>` | 1.0 | Multiplier |
| (target flags from `run`) | — | New target |

### A.3 Subcommand `aggregate`

| Flag | Default | Purpose |
|---|---|---|
| `--pattern <GLOB>` | required | Input parquet files (glob) |
| `--output <PATH>` | required | Aggregated parquet output |
| `--emit-slo-report` | false | Write JSON SLO report |
| `--output-format <FORMAT>` | json | json / yaml / text |

### A.4 Subcommand `validate`

`--profile <YAML>` → exits 0 if valid, 3 if not. Prints schema violations.

### A.5 Subcommand `compare`

| Flag | Purpose |
|---|---|
| `--baseline <PATH>` | First parquet file |
| `--candidate <PATH>` | Second parquet file |
| `--metric <NAME>` | p50_ms / p95_ms / p99_ms / error_rate |
| `--tolerance <PCT>` | Acceptable regression (default 10%) |

Exit code 0 = no regression, 1 = regression detected.

---

## 35. Appendix B — workload profile YAML schema (annotated)

```yaml
# Workload profile schema v1
# Committed under STABILITY_CONTRACT v1.2 § 9.10.4
# Future schema bumps : `version: 2` + migration tooling in `compare`.

version: 1                              # REQUIRED

# Human-readable identification
name:        string                     # REQUIRED, [a-z0-9-]+
description: string                     # optional

# Target endpoints + connection settings
target:
  endpoints:                            # at least one required
    - url:    string                    # grpc:// or grpcs:// URL
      weight: float                     # 0.0 < weight ≤ 1.0
  role: controller | participant
  conns_per_target: int                 # default 4
  per_request_deadline: duration        # optional ("3s")
  mtls:                                 # optional
    cert_path: path
    key_path:  path
    ca_path:   path
    expected_san: string

# Traffic pattern
pattern:
  type: constant | ramp | burst | diurnal | replay
  # Type-specific fields :
  qps: float                            # for constant
  initial_qps: float                    # for ramp
  final_qps: float                      # for ramp
  shape: linear | exponential           # for ramp
  duration: duration                    # for ramp
  baseline_qps: float                   # for burst
  burst_qps: float                      # for burst
  burst_duration: duration              # for burst
  burst_interval: duration              # for burst
  daily_min_qps: float                  # for diurnal
  daily_max_qps: float                  # for diurnal
  period: duration                      # for diurnal
  phase: duration                       # for diurnal
  file: path                            # for replay
  mode: preserve-spacing | accelerate | drain   # for replay
  speed: float                          # for replay
  ramp_up: duration                     # always available (default 30s)

# Workload shape
workload:
  type: iterate | step | reset | state | mixed
  # Mixed-specific
  mix:
    iterate: float
    step:    float
    reset:   float
    state:   float
  # Iterate-specific
  initial_state: random | zero | from-file
  initial_state_dim: int
  max_iters: int
  max_wall_ms: int
  # Step-specific
  target_participants: list[string]
  pulled_dim: int

# Rate control
rate:
  mode: open | closed | hybrid
  max_concurrency: int
  overflow_policy: drop | block         # for open mode

# Termination policies (composable)
termination:
  duration:  duration                   # optional
  requests:  int                        # optional
  until_slo:                             # optional
    iterate-availability: float
    iterate-latency-p95:  float
    manifest-reload-latency-p95: float
    otel-drop-rate: float
  convergence:                          # optional
    tolerance: float
    consecutive_iterations: int

# Output configuration
output:
  format: terminal | json | csv | ndjson | parquet
  file: path
  record: bool                          # enables replay

# Observability
observability:
  prometheus:
    listen: addr                        # default 0.0.0.0:9095
  otel:
    endpoint: URL
    sampler_arg: float                  # 0.0..1.0
    service_name: string
  log:
    format: text | json
    level: trace | debug | info | warn | error

# Run-level metadata
run:
  seed: u64                             # default wall_clock_ns
  mode: dev | staging | production
  allow_mutation: bool
  chaos_marker: string                  # optional
```

JSON-Schema for validation : `tools/loadgen/schemas/profile-v1.json`
(generated from this YAML annotation by `cargo run --bin gen-schema`).

---

## 36. Appendix C — Prometheus self-metrics reference

### C.1 `efe_msw_loadgen_request_total`

```
# TYPE efe_msw_loadgen_request_total counter
# HELP Total number of requests sent by loadgen, by verb and result.
efe_msw_loadgen_request_total{verb="iterate",result="ok",target="ctrl-1"} 28491
efe_msw_loadgen_request_total{verb="iterate",result="error",target="ctrl-1"} 33
efe_msw_loadgen_request_total{verb="state",result="ok",target="ctrl-1"} 1500
```

### C.2 `efe_msw_loadgen_request_duration_seconds`

```
# TYPE efe_msw_loadgen_request_duration_seconds histogram
# HELP Wall-clock duration of loadgen requests.
efe_msw_loadgen_request_duration_seconds_bucket{verb="iterate",result="ok",le="0.005"} 1023
efe_msw_loadgen_request_duration_seconds_bucket{verb="iterate",result="ok",le="0.01"} 5612
...
efe_msw_loadgen_request_duration_seconds_bucket{verb="iterate",result="ok",le="+Inf"} 28491
efe_msw_loadgen_request_duration_seconds_sum{verb="iterate",result="ok"} 850.32
efe_msw_loadgen_request_duration_seconds_count{verb="iterate",result="ok"} 28491
```

Buckets : `[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]`
(matches server-side AD23).

### C.3 `efe_msw_loadgen_inflight`

```
# TYPE efe_msw_loadgen_inflight gauge
# HELP Current in-flight requests per verb.
efe_msw_loadgen_inflight{verb="iterate"} 12
efe_msw_loadgen_inflight{verb="state"} 0
```

### C.4 `efe_msw_loadgen_target_up`

```
# TYPE efe_msw_loadgen_target_up gauge
# HELP Last response state per target. 1 = ok, 0 = error/no response yet.
efe_msw_loadgen_target_up{target="ctrl-1.efe.svc:50051"} 1
```

### C.5 `efe_msw_loadgen_build_info`

```
# TYPE efe_msw_loadgen_build_info gauge
# HELP Build metadata. Always 1.
efe_msw_loadgen_build_info{version="0.1.0",git_sha="abc123",rustc_version="1.83.0",build_date="2026-05-16"} 1
```

---

## 37. Appendix D — OTel span schema (loadgen-side)

### D.1 Root span `efe.meta_swarm.loadgen.iterate`

```
Name        : efe.meta_swarm.loadgen.iterate
Kind        : CLIENT
Parent      : (none — root)
Attributes  :
    loadgen.request_index    u64
    loadgen.seed             u64
    loadgen.run_id           string (UUID v4)
    loadgen.profile          string
    loadgen.mode             string (dev|staging|production)
    loadgen.target           string
    loadgen.verb             string (iterate|step|reset|state)
    net.peer.name            string
    net.peer.port            u16
    grpc.method              string (Iterate|Step|Reset|State)
Events      :
    request_started          (time of request send)
    response_received        (time of response receipt)
Status      :
    ok | error (with description)
Children    :
    via W3C traceparent → controller's `efe.meta_swarm.iterate` (PR #18 § 6)
```

### D.2 Span `efe.meta_swarm.loadgen.connect`

```
Name        : efe.meta_swarm.loadgen.connect
Kind        : CLIENT
Parent      : (none — emitted at startup)
Attributes  :
    net.peer.name        string
    net.peer.port        u16
    loadgen.target       string
    tls.enabled          bool
    tls.expected_san     string (if applicable)
Events      :
    tls_handshake_started
    tls_handshake_completed
Status      :
    ok | error
```

Emitted once per target at startup. Useful for diagnosing TLS handshake
failures.

### D.3 Span `efe.meta_swarm.loadgen.aggregate`

```
Name        : efe.meta_swarm.loadgen.aggregate
Kind        : INTERNAL
Parent      : (none — aggregator binary)
Attributes  :
    loadgen.run_id       string
    loadgen.parallelism  int
    loadgen.input_files  int
    loadgen.total_rows   u64
    loadgen.overall_pass bool
```

Emitted by the aggregator pod (parallelism > 1 only).

---

## 38. Appendix E — Helm chart values structure

### E.1 `charts/efe-meta-swarm-loadgen/values.yaml`

```yaml
enabled: false

image:
  repository: ghcr.io/efe-ai/efe-meta-swarm-loadgen
  tag: "0.1.0"
  pullPolicy: IfNotPresent
  imagePullSecrets: []

# Workload profile YAML embedded inline OR via ConfigMap reference
profile:
  inline: ""                # YAML string (use --set-file profile.inline=path)
  configMap:
    name: ""                # ConfigMap name (mutually exclusive with inline)
    key:  "profile.yaml"

# Kubernetes Job tuning
job:
  parallelism: 1
  completions: 1            # auto-derived = parallelism for indexed completion
  backoffLimit: 3
  activeDeadlineSeconds: 14400   # 4h hard cap (T78 mitigation)
  ttlSecondsAfterFinished: 86400  # cleanup 24h after Job complete

# Pod-level
podSecurityContext:
  runAsNonRoot: true
  runAsUser: 65532
  runAsGroup: 65532
  fsGroup: 65532
  seccompProfile:
    type: RuntimeDefault

containerSecurityContext:
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities:
    drop: [ALL]

resources:
  requests: { cpu: "500m", memory: "256Mi" }
  limits:   { cpu: "2",    memory: "1Gi"  }

# Networking
target:
  endpoint: "grpc://efe-meta-swarm-controller.efe.svc:50051"
  port: 50051
  role: controller

tls:
  enabled: false
  secretName: efe-meta-swarm-loadgen-tls   # cert-manager-issued
  caCertSecretName: efe-meta-swarm-ca

# Observability
prometheus:
  enabled: true
  port: 9095
  serviceMonitor:
    enabled: true
    interval: 15s
    scrapeTimeout: 10s

otel:
  enabled: true
  endpoint: "http://otel-collector.observability.svc:4317"
  samplerArg: "1.0"

# RBAC
serviceAccount:
  create: true
  name: ""               # auto-generated if empty

rbac:
  create: true

# NetworkPolicy
networkPolicy:
  enabled: true
  allowedTargets:
    - controllerSelector: { app.kubernetes.io/name: efe-meta-swarm-controller }
    - collectorSelector:  { app.kubernetes.io/name: otel-collector }
```

### E.2 `charts/efe-meta-swarm-loadgen/values.schema.json`

JSON-Schema for `values.yaml` — validates types + enum values + ranges.

---

## 39. Appendix F — multi-region deployment topology

### F.1 Multi-region target list

```yaml
target:
  endpoints:
    - url: grpc://ctrl-us-east.efe.svc:50051
      weight: 0.4
    - url: grpc://ctrl-eu-west.efe.svc:50051
      weight: 0.3
    - url: grpc://ctrl-ap-south.efe.svc:50051
      weight: 0.3
  role: controller
```

Each request picks a target weighted-random (seeded). Useful for multi-
region validation : "is eu-west's p99 within 1.5× of us-east's p99 ?"

### F.2 Per-region loadgen pods

For region-bounded saturation tests, run a loadgen Job *per region* with
`--target` scoped to that region's controller. Each region's Prometheus
scrapes the local loadgen pod → cross-region comparison via Thanos.

### F.3 Multi-region SLO contract

When validating a multi-region SLO, the aggregator pulls parquet files
from all regions (via S3 or rclone) and computes a global SLO. Region-
local SLOs are validated independently first ; global only matters when
operators define multi-region targets.

---

## 40. Appendix G — Phase-3 follow-ups

### G.1 Adaptive load (PR #25 ?)

ML-driven load curves that adapt to controller's observed latency. Out
of scope for PR #20 — explicit pattern shapes preferred for deterministic
regression detection.

### G.2 Production-tap recording

Operator-side tool that records production traffic (via a custom tonic
interceptor on the controller) and emits replay-ready parquet files.
PR #20 ships the *consumer* (replay mode) ; the *producer* is Phase-3.

### G.3 Soak-test mode

Long-running (days) loadgen with internal memory-leak detection + cert
rotation handling. Out of scope ; current architecture is short-run.

### G.4 Per-tenant load profiles

Phase-3 multi-tenancy : load profiles tagged with `tenant_id` ; aggregator
splits results per tenant. Requires PR #25 (tenant labels on metrics).

### G.5 Plugin architecture for custom patterns

`libloading`-based plugin for operator-defined traffic patterns. Risks
ABI breakage ; deferred unless strong demand.

### G.6 Built-in chaos integration

Tighter coupling with chaos-mesh : loadgen launches chaos fixtures from
its profile YAML. Currently coordination via `--chaos-marker` is loose.

### G.7 Cost attribution

`$/request` panel in D1 = loadgen-generated request count × per-iterate
cost. Requires cost-attribution exporter (not currently in scope).

### G.8 Continuous benchmark + dashboard

Periodic CronJob runs loadgen + appends to a long-term parquet dataset ;
Grafana dashboard plots historical SLO trends. Phase-3.

### G.9 SDK integration

Embed loadgen as a library in the EFE SDKs (TypeScript + Go) so that
SDK consumers can run synthetic regression tests against their own
EFE instances.

### G.10 GraphQL endpoint for loadgen control

Real-time control API : start, pause, resume, stop a running loadgen
without restarting the binary. Phase-3 (or scoped to a separate ops
tooling PR).

---

## End of architecture document

| Field | Value |
|---|---|
| Section count | 40 |
| Architecture decisions | 40 |
| Acceptance criteria | 32 |
| Threat IDs | 10 (T76 – T85) |
| Traffic patterns | 5 |
| Workload shapes | 4 (+ mixed) |
| Rate controllers | 3 |
| Termination policies | 5 |
| Output formats | 4 |
| Use cases | 7 |
| Prometheus self-metrics | 5 (STABILITY_CONTRACT v1.2) |
| OTel spans | 3 (loadgen-side) |
| Helm chart | 1 standalone (NOT sub-chart) |
| CI workflows | 2 (smoke + slo-regression) |
| Total LOC (this doc) | ~ 2 900 |

— EFE platform team, performance area, May 2026
