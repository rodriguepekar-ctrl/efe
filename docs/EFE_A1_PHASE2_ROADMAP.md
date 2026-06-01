# EFE Sprint A1 Phase 2 — Detailed Roadmap (Jul-Aug 2026)

> **Document type** : Execution-ready roadmap for Phase 2 (gRPC transport).
> **Status** : DRAFT (frozen at Phase 2 kickoff).
> **Architecture source** : [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md).
> **Parent roadmap** : [`EFE_A1_ROADMAP.md`](EFE_A1_ROADMAP.md) § 5.
> **Window** : 2026-07-06 → 2026-08-31 (8 weeks, 4 sprints, 25 PRs).
> **Gate** : L2 attained ⇔ 10 criteria pass (cf. § 12).

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Phase 2 commitment](#1-phase-2-commitment)
2. [Sprint A1.P2.1 — Protos + bare servers (Jul 06-19)](#2-sprint-a1p21--protos--bare-servers)
3. [Sprint A1.P2.2 — Config + Helm scaffold (Jul 20 - Aug 02)](#3-sprint-a1p22--config--helm-scaffold)
4. [Sprint A1.P2.3 — Observability + start of 7-day soak (Aug 03-16)](#4-sprint-a1p23--observability--start-of-7-day-soak)
5. [Sprint A1.P2.4 — L2 graduation (Aug 17-30)](#5-sprint-a1p24--l2-graduation)
6. [Daily cadence](#6-daily-cadence)
7. [PR catalog (#5-#25)](#7-pr-catalog)
8. [Test gates per sprint](#8-test-gates-per-sprint)
9. [Soak monitoring plan](#9-soak-monitoring-plan)
10. [L2 gate checklist (G1-G10)](#10-l2-gate-checklist)
11. [RACI per deliverable](#11-raci-per-deliverable)
12. [Risk burn-down](#12-risk-burn-down)
13. [Communication](#13-communication)
14. [Rollback plan](#14-rollback-plan)
15. [Open questions per sprint](#15-open-questions-per-sprint)
16. [Definition of Done](#16-definition-of-done)

---

## 0. Executive summary

```
╔════════════════════════════════════════════════════════════════════════╗
║  Phase 2 plan-on-a-page                                                ║
╠════════════════════════════════════════════════════════════════════════╣
║                                                                        ║
║  Sprint 1 (Jul 06-19) — Protos + bare gRPC servers                     ║
║     PRs #5-#10  · 6 PRs · meta-swarm-team                              ║
║     Demo : 1 controller + 3 participants run on dev box,               ║
║            Iterate() bit-identical to Phase 1 reference                ║
║                                                                        ║
║  Sprint 2 (Jul 20-Aug 02) — Config + Helm                              ║
║     PRs #11-#16 · 6 PRs · meta-swarm + platform + sec teams            ║
║     Demo : helm install on staging with flag flipped on/off,           ║
║            SIGHUP rolls participant in/out                             ║
║                                                                        ║
║  Sprint 3 (Aug 03-16) — Observability + soak start                     ║
║     PRs #17-#21 · 5 PRs · meta-swarm + sre teams                       ║
║     Demo : Grafana dashboards live ; 7-day soak begins Mon Aug 10      ║
║                                                                        ║
║  Sprint 4 (Aug 17-30) — L2 graduation                                  ║
║     PRs #22-#25 · 4 PRs · meta-swarm + sre teams                       ║
║     Demo : G1-G10 verified, master gate GREEN, L2 announced            ║
║                                                                        ║
║  Total : 25 PRs · 8 weeks · 47 FTE-weeks                               ║
║                                                                        ║
╚════════════════════════════════════════════════════════════════════════╝
```

**Top-level deliverable** : a transparent, opt-in, fail-closed gRPC
transport for `efe-meta-swarm` that **does not affect a single Phase-1
deployment** unless explicitly enabled, validated by a 7-day internal
soak with zero Sev1/Sev2 and a published head-to-head bench.

---

## 1. Phase 2 commitment

### 1.1 What we will ship (binding)

| # | Deliverable | Sprint | Lead | Verification |
|---|---|---|---|---|
| C1 | `efe-meta-swarm-proto` crate (protos + tonic stubs) | P2.1 | meta-swarm-team | `cargo doc -p efe-meta-swarm-proto` builds clean |
| C2 | `efe-meta-swarm-server` crate (controller + participant bins) | P2.1 | meta-swarm-team | `cargo run --bin controller` succeeds |
| C3 | Multi-process integration test (3 processes, bit-identical result) | P2.1 | meta-swarm-team | `tests/integration_grpc_multiprocess.rs` GREEN |
| C4 | `EFE_META_SWARM_ENABLED` env-var plumbing in `efe-server` | P2.2 | meta-swarm-team | Compare logs/manifests off-vs-on |
| C5 | SIGHUP hot reload (atomic swap, ≤ 200 ms p99) | P2.2 | meta-swarm-team | `tests/integration_sighup_hot_reload.rs` |
| C6 | Helm sub-chart `efe-meta-swarm` default-disabled | P2.2 | platform-team | `helm template` byte-identical when off |
| C7 | mTLS controller ↔ participant + efe-server ↔ controller | P2.2 | sec-team + platform | cert-manager Certificate resource live |
| C8 | 14 Prometheus metrics scraped | P2.3 | meta-swarm-team | `curl :9090/metrics` returns 14 names |
| C9 | 8 OTel span types emitted | P2.3 | meta-swarm-team | Sample trace seen in OTel collector |
| C10 | 2 Grafana dashboards + 4 alert rules | P2.3 | sre-team | Dashboards rendered, alerts firing on test |
| C11 | `tools/loadgen` traffic generator | P2.3 | meta-swarm-team | `loadgen --rps 30 --duration 1m` succeeds |
| C12 | 7-day internal soak passes 10 criteria | P2.3-4 | sre-team | `A1_PHASE2_SOAK_REPORT.md` published |
| C13 | Public bench (5 workloads × 3 modes) | P2.3-4 | meta-swarm-team | `bench_results/meta-swarm-vs-mono-swarm-2026-08.{csv,html}` |
| C14 | 4 new SLOs + master rollup `SLO-OPS-A1-META-GRPC` | P2.4 | meta-swarm-team | `bench_baselines/v0.11.0/meta_swarm_grpc_slos.yaml` |
| C15 | Smoke test 9 pinning rollup #20 | P2.4 | meta-swarm-team | `tests/bench_smoke.rs` GREEN |
| C16 | Runbook `META_SWARM_GRPC.md` + 2 SRE drills | P2.4 | sre-team | Runbook + drill logs committed |
| C17 | Phase 2 end-of-sprint report | P2.4 | meta-swarm-team | `docs/EFE_A1_PHASE2_LESSONS.md` |

### 1.2 What we will NOT ship (deferred to P3+)

- Cross-region replication
- Per-tenant isolation
- Customer-facing SLA
- BLAS / GPU acceleration
- Auto-rotation of mTLS certs (manual rotation OK in Phase 2)
- Raft-based controller HA (K8s lease active-passive is enough)

### 1.3 Success signal at L2

A single sentence the team should be able to say with a straight face
on 2026-08-31 :

> "Any future PR that pushes `iterate()` p99 past 100 ms, breaks
> multi-process determinism, blows the 200 ms reload budget, or
> drops OTel trace coverage below 5 % is blocked at the master gate
> automatically, and we have 7 consecutive days of zero Sev1/Sev2
> production traffic to prove the gate is calibrated correctly."

---

## 2. Sprint A1.P2.1 — Protos + bare servers

**Window** : 2026-07-06 (Mon) → 2026-07-19 (Sun) — 2 weeks
**Theme** : Establish the wire protocol and a minimum-viable distributed runtime.
**Demo date** : Fri 2026-07-17 16:00 UTC (`#meta-swarm` Zoom)
**Capacity** : 6 meta-swarm-team FTE × 2 weeks = 12 FTE-weeks

### 2.1 Week 1 (Jul 06-12) — Protos + scaffolding

#### Goals

- Land the 4 proto files + tonic-build wiring
- Scaffold both binaries (no logic, just network skeleton)
- Threat model draft circulating

#### PRs

##### PR #5 — `feat(efe-meta-swarm-proto): scaffold protos + tonic-build`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 3 FTE-days
**Branch** : `A1.P2/05-meta-swarm-proto-scaffold`

Diff scope :

```
crates/efe-meta-swarm-proto/  (NEW)
├── Cargo.toml
├── build.rs                      # tonic_build::compile_protos
├── proto/
│   └── efe/v1/meta_swarm/
│       ├── service_meta_swarm.proto
│       ├── service_meta_participant.proto
│       ├── messages.proto
│       └── errors.proto
├── src/
│   └── lib.rs                    # re-export tonic-generated
└── README.md                     # versioning policy
Cargo.toml (workspace) +1 member
```

Acceptance :
- `cargo doc -p efe-meta-swarm-proto --no-deps` builds clean
- `cargo build -p efe-meta-swarm-proto` produces a library with 0 warnings
- README.md documents the v1 contract + reserved field numbers ≥ 100
- Smoke test : import a `MetaSwarmServiceClient` from another crate, no compile error

Out-of-scope : no service implementation, no client code beyond what tonic generates.

Dependencies : none (first PR of the sprint).

##### PR #6 — `feat(efe-meta-swarm-server): scaffold server crate`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 4 FTE-days
**Branch** : `A1.P2/06-meta-swarm-server-scaffold`

Diff scope :

```
crates/efe-meta-swarm-server/  (NEW)
├── Cargo.toml                    # panic = "unwind" override
├── src/
│   ├── lib.rs
│   ├── bin/
│   │   ├── controller.rs         # minimal main() : parse args, bind, serve
│   │   └── participant.rs        # idem
│   ├── controller/
│   │   ├── mod.rs
│   │   └── service.rs            # impl MetaSwarmService { todo!() }
│   ├── participant/
│   │   ├── mod.rs
│   │   └── service.rs            # impl MetaParticipantService { todo!() }
│   ├── observability/
│   │   ├── health.rs             # only Health is implemented
│   │   └── metrics.rs            # stub
│   ├── auth/
│   │   ├── mtls.rs               # stub
│   │   └── bearer.rs             # stub
│   └── error.rs
└── README.md
Cargo.toml (workspace) +1 member
```

Acceptance :
- `cargo run --bin controller` binds and answers `Health` with `STATUS_SERVING`
- `cargo run --bin participant` same
- All other RPCs return `UNIMPLEMENTED`
- README documents how to run locally + binary CLI args

Dependencies : PR #5 (uses the generated stubs).

##### PR #7 — `docs(security): Phase 2 threat model draft`

**Owner** : sec-team (lead : R. Kapoor)
**Estimate** : 2 FTE-days

Diff scope :

```
docs/security/PHASE2_THREAT_MODEL.md  (NEW)
```

Acceptance :
- 8 sections : assets, trust boundaries, attackers, threats, mitigations, residual risk, monitoring, references
- ≥ 12 distinct threat scenarios catalogued (STRIDE per surface)
- 0 SEV-1 unmitigated entries
- sec-team peer-review tag on the PR

Dependencies : none ; can run in parallel.

### 2.2 Week 2 (Jul 13-19) — Wire up Iterate

#### Goals

- `MetaSwarmService::Iterate` wired through the in-process Phase-1 `MetaSwarm`
- `MetaParticipantService::Step` wired through `MockParticipant`
- First multi-process integration test passes

#### PRs

##### PR #8 — `feat(efe-meta-swarm-server): wire MetaSwarmService::Iterate`

**Owner** : meta-swarm-team (lead : M. Okafor)
**Estimate** : 5 FTE-days
**Branch** : `A1.P2/08-iterate-wire`

Diff scope :

```
crates/efe-meta-swarm-server/src/
├── controller/
│   ├── service.rs                # Iterate impl
│   ├── manifest.rs               # NEW : YAML manifest reader
│   └── discovery.rs              # NEW : StaticDiscovery only (T2)
├── client/
│   ├── mod.rs                    # NEW
│   └── remote_participant.rs     # NEW : controller's client to participants
└── tests/
    └── integration_iterate_single_proc.rs   # NEW : in-process unit
```

Acceptance :
- Controller, given a static manifest, can resolve participants by id
- `Iterate()` round-trips through 3 participant gRPC clients
- Single-process unit test : `Iterate()` returns same `iterations + residual` as Phase-1 `MetaSwarm::iterate()` (within ±2 iters tolerance for variance)
- Cancellation : if client drops the call, server cancels participant calls within 100 ms

Dependencies : PRs #5, #6.

##### PR #9 — `feat(efe-meta-swarm-server): wire MetaParticipantService::Step`

**Owner** : meta-swarm-team (lead : M. Okafor)
**Estimate** : 4 FTE-days
**Branch** : `A1.P2/09-step-wire`

Diff scope :

```
crates/efe-meta-swarm-server/src/
└── participant/
    ├── service.rs                # Step + Reset + Capabilities + State + Health
    └── adapter.rs                # NEW : adapt to Phase-1 MockParticipant / LocalParticipant
```

Acceptance :
- `Step()` round-trip latency ≤ 5 ms p99 on localhost
- `Capabilities()` returns the static descriptor
- Determinism : 3 consecutive `Step()` with same `pulled_from_meta` produce bit-identical outputs

Dependencies : PR #6.

##### PR #10 — `test(efe-meta-swarm-server): integration_grpc_multiprocess`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 3 FTE-days
**Branch** : `A1.P2/10-multiproc-test`

Diff scope :

```
crates/efe-meta-swarm-server/tests/
├── integration_grpc_multiprocess.rs    # NEW
└── fixtures/
    └── 3-participant-localhost.yaml    # NEW manifest
```

Acceptance :
- Test spawns 1 controller process + 3 participant processes via `std::process::Command`
- Calls `Iterate()` via the client
- Asserts : `iterations <= 60`, `final_residual <= 1e-3`, `stop_reason == Converged`
- Determinism : 3 reruns produce bit-identical residual.to_bits()
- Test finishes in ≤ 30 s (`#[ignore]` if not in CI ; `cargo test -- --ignored` in CI matrix)

Dependencies : PRs #8, #9.

### 2.3 Sprint 1 demo (2026-07-17 Fri 16:00 UTC)

| Time | Topic | Lead |
|---|---|---|
| 00:00 | Intro | meta-swarm-team |
| 00:05 | Proto walkthrough (key RPCs + types) | J. Tan |
| 00:15 | Run `cargo run --bin controller` + 3× `cargo run --bin participant` | M. Okafor |
| 00:25 | Live `Iterate()` call → show bit-identical result vs Phase-1 example | M. Okafor |
| 00:35 | Threat model headlines | R. Kapoor |
| 00:40 | Q&A | all |

### 2.4 Sprint 1 retrospective (2026-07-19 Sun async in #meta-swarm)

3 questions :
- What went well ?
- What blocked us ?
- What we change for Sprint 2 ?

---

## 3. Sprint A1.P2.2 — Config + Helm scaffold

**Window** : 2026-07-20 (Mon) → 2026-08-02 (Sun) — 2 weeks
**Theme** : Make it deployable.  Env var, SIGHUP, Helm chart, mTLS.
**Demo date** : Fri 2026-07-31 16:00 UTC
**Capacity** : meta-swarm-team (4 FTE) + platform-team (2 FTE) + sec-team (1 FTE) × 2 weeks = 14 FTE-weeks

### 3.1 Week 3 (Jul 20-26) — env var + SIGHUP

#### PRs

##### PR #11 — `feat(efe-server): EFE_META_SWARM_ENABLED plumbing`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 4 FTE-days

Diff scope :

```
crates/efe-server/Cargo.toml                     # +1 optional dep efe-meta-swarm-proto behind feature "meta-swarm-grpc"
crates/efe-server/src/bridge/meta_swarm.rs       # NEW : dispatch in-proc vs gRPC
crates/efe-server/src/bridge/mod.rs              # +mod
crates/efe-server/src/config.rs                  # +EFE_META_SWARM_ENABLED reader
```

Acceptance :
- Env var unset OR =false → byte-identical to Phase 1 (verified by hashing the gRPC server response for a fixed request)
- =true + valid `EFE_META_SWARM_PARTICIPANTS` → calls `MetaSwarmServiceClient::iterate`
- =true + invalid config → process refuses to start with explicit error
- Smoke : `efe-server` startup log diff with/without flag is exactly 5 lines (controller URL + auth + 3 participant URLs)

Dependencies : PRs #5, #8.

##### PR #12 — `feat(efe-meta-swarm-server): SIGHUP hot-reload + ReloadConfig RPC`

**Owner** : meta-swarm-team (lead : M. Okafor)
**Estimate** : 5 FTE-days
**Branch** : `A1.P2/12-sighup-reload`

Diff scope :

```
crates/efe-meta-swarm-server/src/
├── controller/
│   ├── reload.rs                # NEW : SIGHUP handler + atomic manifest swap
│   └── service.rs               # +impl ReloadConfig
└── tests/
    └── integration_sighup_hot_reload.rs   # NEW : 1000 random-timed SIGHUPs during Iterate
```

Acceptance :
- Atomic swap : in-flight `Iterate()` never sees partial state
- ≤ 200 ms p99 reload time on the 3-participant scenario
- `drain_inflight=true` waits up to 30 s ; `=false` aborts cleanly with `STOP_REASON_CONFIG_RELOAD`
- Dimension change → `META_ERROR_RELOAD_DIM_CHANGED`
- Manifest re-read from disk uses `O_NOFOLLOW` to prevent symlink swap attacks (sec-team req)

Dependencies : PRs #8, #9.

##### PR #13 — `feat(charts): Helm sub-chart skeleton`

**Owner** : platform-team (lead : K. Müller)
**Estimate** : 4 FTE-days

Diff scope :

```
charts/efe/charts/efe-meta-swarm/  (NEW)
├── Chart.yaml
├── values.yaml                              # default-disabled
├── values.schema.json
├── templates/
│   ├── _helpers.tpl
│   ├── controller-statefulset.yaml
│   ├── controller-service.yaml
│   ├── controller-pdb.yaml
│   ├── participant-deployment.yaml          # template per participant via range
│   ├── participant-service.yaml
│   ├── secret-tls.yaml
│   ├── configmap-manifest.yaml
│   └── tests/connection-test.yaml
└── README.md
```

Acceptance :
- `helm lint` clean
- `helm template ./charts/efe` (defaults) = byte-identical to Phase-1 manifest
- `helm template ./charts/efe --set metaSwarm.enabled=true` produces a manifest covering 1 controller StatefulSet (3 replicas) + 3 participant Deployments + Services
- `helm test` after install passes the `connection-test` pod (calls `Health`)

Dependencies : PRs #6, #11.

### 3.2 Week 4 (Jul 27 - Aug 02) — Helm values + mTLS

#### PRs

##### PR #14 — `feat(charts): full values + templates`

**Owner** : platform-team (lead : K. Müller)
**Estimate** : 4 FTE-days

Diff scope :

```
charts/efe/charts/efe-meta-swarm/templates/
├── controller-servicemonitor.yaml     # NEW (Prometheus Operator)
├── participant-servicemonitor.yaml    # NEW
├── controller-networkpolicy.yaml      # NEW (allow only efe-server ingress)
├── participant-networkpolicy.yaml     # NEW (allow only controller ingress)
└── ...

charts/efe/charts/efe-meta-swarm/values.yaml   # populated
```

Acceptance :
- All 3 participants render with distinct configs (dim, kappa, seed)
- NetworkPolicy correctly scopes the trust boundary
- `--set metaSwarm.participants[0].replicas=2` produces 2 participant pods (multi-replica is a P3 future, but template must already support it)

Dependencies : PR #13.

##### PR #15 — `ci(helm): helm-meta-swarm-lint workflow`

**Owner** : platform-team (lead : K. Müller)
**Estimate** : 2 FTE-days

Diff scope :

```
.github/workflows/helm-meta-swarm-lint.yml   # NEW
charts/efe/golden-phase1-default.yaml        # NEW : checked-in manifest snapshot
charts/efe/golden-phase2-enabled.yaml        # NEW : checked-in manifest snapshot
```

Acceptance :
- Workflow runs on PRs touching `charts/efe/**`
- `helm lint` + `helm template` (default) diff vs `golden-phase1-default.yaml` exact match
- `helm template --set metaSwarm.enabled=true` diff vs `golden-phase2-enabled.yaml` exact match
- Test : intentionally break the chart in a fork branch → workflow RED

Dependencies : PR #14.

##### PR #16 — `feat(security): mTLS wiring + cert-manager Issuer`

**Owner** : sec-team (lead : R. Kapoor) + platform-team (K. Müller)
**Estimate** : 5 FTE-days

Diff scope :

```
charts/efe/charts/efe-meta-swarm/templates/
├── tls-issuer.yaml                # NEW : cert-manager Issuer
├── controller-certificate.yaml    # NEW : cert-manager Certificate
└── participant-certificate.yaml   # NEW (template per participant)

crates/efe-meta-swarm-server/src/auth/
├── mtls.rs                        # full impl : load cert, configure tonic TLS
└── tests/
    └── tls_e2e.rs                  # NEW : in-test cert generation + handshake check
```

Acceptance :
- mTLS handshake enforced on all 3 gRPC links (efe-server ↔ controller ↔ participants)
- SAN-based authorization : controller rejects non-`efe-server.*.svc` callers
- cert-manager issues 90-day leaf certs with auto-renewal at 60 days
- Bearer token fallback works (`EFE_META_SWARM_AUTH_TOKEN`) — for dev only, gated by `EFE_DEV_MODE=true`

Dependencies : PRs #6, #13.

### 3.3 Sprint 2 demo (2026-07-31 Fri 16:00 UTC)

| Time | Topic | Lead |
|---|---|---|
| 00:00 | Intro | meta-swarm-team |
| 00:05 | Env-var flip live : staging deploy → disable → re-enable | J. Tan |
| 00:15 | SIGHUP drill : add a 4th participant in the manifest, send SIGHUP, see it active | M. Okafor |
| 00:25 | Helm install with chart : flag off (compare manifest), then on | K. Müller |
| 00:35 | mTLS handshake walkthrough | R. Kapoor |
| 00:45 | Q&A | all |

### 3.4 Sprint 2 retrospective (2026-08-02 Sun async)

Same 3-question format.

---

## 4. Sprint A1.P2.3 — Observability + start of 7-day soak

**Window** : 2026-08-03 (Mon) → 2026-08-16 (Sun) — 2 weeks
**Theme** : Make it observable, generate traffic, start the soak.
**Demo date** : Fri 2026-08-14 16:00 UTC
**Capacity** : meta-swarm-team (4 FTE) + sre-team (2 FTE) × 2 weeks = 12 FTE-weeks

### 4.1 Week 5 (Aug 03-09) — Metrics + tracing + dashboards

#### PRs

##### PR #17 — `feat(efe-meta-swarm-server): 14 Prometheus metrics`

**Owner** : meta-swarm-team (lead : M. Okafor)
**Estimate** : 4 FTE-days

Diff scope :

```
crates/efe-meta-swarm-server/src/observability/
└── metrics.rs                # full registry, 14 metrics
```

Acceptance :
- All 14 metrics emitted with correct labels (cf. architecture § 9.1)
- `meta_swarm_inflight` gauge tracks concurrent `Iterate` calls
- `meta_swarm_participant_step_duration_seconds` has the right histogram buckets `[1ms, 5ms, …, 500ms]`
- `meta_swarm_build_info{git_sha=…, proto_version=v1}` always 1
- Smoke test : run controller, hit Iterate 10× concurrent, scrape `:9090/metrics`, assert all 14 metric names present

Dependencies : PRs #8, #9.

##### PR #18 — `feat(efe-meta-swarm-server): 8 OpenTelemetry span types`

**Owner** : meta-swarm-team (lead : M. Okafor)
**Estimate** : 4 FTE-days

Diff scope :

```
crates/efe-meta-swarm-server/src/observability/
└── tracing.rs                # OTel setup + span helpers
```

Acceptance :
- 8 distinct span names emitted (cf. architecture § 9.2)
- `traceparent` header propagated client → controller → participant (3-hop trace)
- Sampling controlled by `EFE_META_SWARM_TRACE_SAMPLE_RATIO` (default 0.05)
- Smoke test : enable 100 % sampling, call `Iterate` once, inspect collector → see 1 root span + N child spans (5 inner steps × 3 participants ≈ 15+ child spans)

Dependencies : PR #17.

##### PR #19 — `feat(grafana): dashboards + alert rules`

**Owner** : sre-team (lead : L. Ortiz)
**Estimate** : 3 FTE-days

Diff scope :

```
grafana/dashboards/
├── efe-meta-swarm-ops.json       # operational dashboard
└── efe-meta-swarm-algo.json      # algorithmic dashboard

charts/efe/charts/efe-meta-swarm/templates/
└── prometheus-rules.yaml         # 4 PrometheusRule entries
```

Acceptance :
- Both dashboards render on staging Grafana
- All 14 metrics referenced by at least one panel
- 4 alerts (cf. architecture § 9.4) verified by injecting a fault that triggers each
- Alerts route to test-only PagerDuty service (no real on-call yet)

Dependencies : PRs #17, #18, #14.

### 4.2 Week 6 (Aug 10-16) — Loadgen + soak begins

#### PRs

##### PR #20 — `tools(loadgen): meta-swarm traffic generator`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 3 FTE-days

Diff scope :

```
tools/loadgen/
├── Cargo.toml                # NEW : standalone bin
├── src/
│   ├── main.rs
│   ├── scenarios/
│   │   ├── kovant_steady.rs       # 60 % of soak traffic
│   │   ├── random_initial.rs      # 30 %
│   │   └── sighup_chaos.rs        # 10 %
│   └── reporter.rs           # Prometheus pushgateway client
└── README.md
```

Acceptance :
- `loadgen --rps 30 --duration 168h --scenarios kovant_steady:60,random:30,sighup:10 --target=staging-controller`
- Loadgen emits its own metrics (req sent, errors, observed latency)
- Sighup chaos : every 30 min, sends SIGHUP to a random controller pod
- Graceful shutdown via SIGTERM

Dependencies : PRs #5, #8.

##### PR #21 — `feat(public-bench): harness + 5 workloads`

**Owner** : meta-swarm-team (lead : M. Okafor)
**Estimate** : 4 FTE-days

Diff scope :

```
bench_public/
├── Cargo.toml
├── src/
│   ├── main.rs                # bench driver
│   ├── workloads/
│   │   ├── w1_kovant_3.rs
│   │   ├── w2_kovant_3_2x.rs
│   │   ├── w3_seven_participants.rs
│   │   ├── w4_single_swarm.rs
│   │   └── w5_slow_participant.rs
│   ├── modes/
│   │   ├── mono_swarm.rs
│   │   ├── meta_in_proc.rs
│   │   └── meta_grpc.rs
│   └── report.rs              # CSV + HTML emitters
└── README.md                  # reproducibility recipe
```

Acceptance :
- `cargo run --bin bench_public --release -- --workloads W1,W2,W3,W4,W5 --modes mono,in_proc,grpc`
- Outputs `bench_results/raw/*.json` (criterion-shape per run)
- Outputs `bench_results/meta-swarm-vs-mono-swarm-DRAFT.{csv,html}` (final filename : `…2026-08.{csv,html}` after L2 pass)
- HTML is self-contained (inline CSS+JS, no CDN)
- Harness deterministically reproducible : same hardware + git sha → same CSV bit-for-bit

Dependencies : PR #11 (needs the `EFE_META_SWARM_ENABLED` flag to flip between in-proc and gRPC).

### 4.3 7-day soak begins (Mon 2026-08-10 00:00 UTC)

Cf. § 9 for full monitoring plan.

### 4.4 Sprint 3 demo (2026-08-14 Fri 16:00 UTC)

| Time | Topic | Lead |
|---|---|---|
| 00:00 | Intro | meta-swarm-team |
| 00:05 | Live tour of `ops` dashboard during soak | L. Ortiz |
| 00:15 | Live tour of `algo` dashboard during soak | L. Ortiz |
| 00:25 | Sample OTel trace : 1 Iterate call from efe-server → controller → 3 participants | M. Okafor |
| 00:35 | Soak day-4 status : SLO compliance, incidents, memory trend | L. Ortiz |
| 00:45 | Q&A | all |

---

## 5. Sprint A1.P2.4 — L2 graduation

**Window** : 2026-08-17 (Mon) → 2026-08-30 (Sun) — 2 weeks
**Theme** : Land the SLOs, finish the soak, publish the bench, attain L2.
**Demo date** : Fri 2026-08-28 16:00 UTC (= L2 graduation demo)
**Capacity** : meta-swarm-team (3 FTE) + sre-team (2 FTE) × 2 weeks = 10 FTE-weeks

### 5.1 Week 7 (Aug 17-23) — SLOs + runbook + drills

#### PRs

##### PR #22 — `feat(baselines): 4 new SLOs + master rollup #20`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 2 FTE-days

Diff scope :

```
bench_baselines/v0.11.0/                         (NEW directory)
└── meta_swarm_grpc_slos.yaml                    # 4 SLOs

bench_baselines/v0.9.0/ops_master_slos.yaml      # +1 master rollup #20
scripts/ops/run_suite.sh                         # +1 case "a2_meta_swarm_grpc"
.github/workflows/ops-master-gate.yml            # +1 baseline + 1 step
.github/workflows/bench.yml                      # +1 job bench-meta-swarm-grpc
docs/EFE_A1_BENCH_TRACKING.md                    # extended for A2
```

Acceptance :
- Consolidator ingests v0.11.0 baseline cleanly (`surface SLOs found : 4` for A2 alone)
- Master gate reports 20 master rollups (was 19 after L1)
- Negative test : break one A2 SLO via crafted measures → gate FAIL exit 1

Dependencies : PRs #17 (metrics that feed P99 + reload SLOs), #18 (trace coverage SLO).

##### PR #23 — `test(bench_smoke): smoke test #9 for rollup #20`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 1 FTE-day

Diff scope :

```
crates/efe-meta-swarm/tests/bench_smoke.rs       # +test master_rollup_phase2_inherits_4_slos
```

Acceptance :
- Asserts master rollup `SLO-OPS-A1-META-GRPC` exists, status ACTIVE, inherits exactly the 4 A2 SLO ids
- Mutation test : remove the rollup → smoke RED ; restore → smoke GREEN

Dependencies : PR #22.

##### PR #24 — `bench(public): publish meta-swarm vs mono-swarm (Aug 2026)`

**Owner** : meta-swarm-team (lead : M. Okafor)
**Estimate** : 3 FTE-days

Diff scope :

```
bench_results/
├── meta-swarm-vs-mono-swarm-2026-08.csv          # NEW : 5 workloads × 3 modes
├── meta-swarm-vs-mono-swarm-2026-08.html         # NEW : interactive
└── raw/                                          # NEW : criterion JSON per run
    └── ...
```

Acceptance :
- All 5 workloads × 3 modes recorded
- CSV is machine-parseable (`pandas.read_csv` succeeds)
- HTML has working toggles + residual-trajectory plots
- Reproducibility : `bash bench_runner.sh --public --workload W1` on fresh checkout produces same CSV within ±5 % per cell
- Honest framing block at the top (cf. architecture § 11.4)

Dependencies : PR #21.

##### Runbook + drills (no PR ; tracked as deliverables)

**Owner** : sre-team (lead : L. Ortiz)
**Estimate** : 2 FTE-days

Outputs :

```
docs/runbooks/META_SWARM_GRPC.md                 # NEW : on-call runbook
docs/runbooks/drills/2026-08-19_sighup_drill.md  # NEW : drill log
docs/runbooks/drills/2026-08-21_partdown_drill.md # NEW : drill log
```

Acceptance :
- Runbook follows the template of `ops_master_gate_fail.md`
- Drill 1 : staging SIGHUP at random time → recovery, dashboards green within 30 s
- Drill 2 : `kubectl delete pod participant-supplier-scoring-0` → recovery, dashboards green within 60 s
- Both drill logs include : timestamp, observer, expected/observed behavior, follow-ups

Dependencies : PRs #14, #19.

### 5.2 Week 8 (Aug 24-30) — L2 verification + announcement

#### Activities

##### L2 gate verification (Mon 2026-08-24)

**Owner** : meta-swarm-team + sre-team
**Estimate** : 1 FTE-day (whole team in a Zoom room)

Run the 10-criterion checklist (cf. § 10).  Each criterion verified
by 2 engineers (4-eye policy).  Outcomes tracked in :

```
docs/A1_PHASE2_L2_GATE_VERIFICATION.md           # NEW
```

If any criterion is RED, the gate is **deferred 2 weeks** (cf.
[`EFE_A1_ROADMAP.md`](EFE_A1_ROADMAP.md) § 12).

##### PR #25 — `docs: Phase 2 end-of-sprint lessons + final report`

**Owner** : meta-swarm-team (lead : J. Tan)
**Estimate** : 2 FTE-days

Diff scope :

```
docs/EFE_A1_PHASE2_LESSONS.md                    # NEW : retrospective
docs/A1_PHASE2_SOAK_REPORT.md                    # NEW : 7-day soak report
docs/EFE_A1_ROADMAP.md                           # refresh : P2 → ✅
crates/efe-meta-swarm/PR4_DESCRIPTION.md         # link to PR5-25 chain
```

Acceptance :
- Lessons doc has 4 sections : went well / blocked / surprised / changes for P3
- Soak report has the 10 pass-criterion table + memory time series chart
- Roadmap refresh moves P2 to ✅, refreshes 2026-08-31 as L2 date

Dependencies : all prior PRs.

##### L2 announcement (Fri 2026-08-28 16:00 UTC = demo)

Format : Slack post to `#announcements` using the template from
[`EFE_A1_ROADMAP.md`](EFE_A1_ROADMAP.md) § 16.1.

##### Buffer (Sun 2026-08-30)

Reserved for any straggler PR fix-up.  Hard deadline 23:59 UTC.

---

## 6. Daily cadence

### 6.1 Standups

| Day | Time | Format | Attendees |
|---|---|---|---|
| Mon-Fri | 09:30 UTC | Async post in `#meta-swarm-standup` | All meta-swarm-team |
| Wed | 14:00 UTC | 15-min sync video | Whole team (incl. sre/platform/sec) |

Async standup template :
```
yesterday : <bullet>
today     : <bullet>
blocked   : <bullet | none>
phase 2 PR : <#N>
```

### 6.2 Weekly cadence

| Day | Activity |
|---|---|
| Mon | Sprint goals review (10 min in standup) |
| Wed | Cross-team sync video (15 min) |
| Thu | Mid-sprint risk check (async in `#meta-swarm-risks`) |
| Fri | End-of-week metrics : PR burndown, SLO trend, soak status (if running) |

### 6.3 Soak-week-specific cadence (Aug 10-17)

Every 24 h during the soak :

- 08:00 UTC : SRE on-call posts daily-status to `#meta-swarm-soak`
- 17:00 UTC : meta-swarm-team review the status, decides on any action
- 23:00 UTC : automatic Grafana snapshot saved to `docs/A1_PHASE2_SOAK_DAILY/`

---

## 7. PR catalog

| # | Title | Owner | Sprint | Est | Deps |
|---|---|---|---|---|---|
| 5 | `feat(efe-meta-swarm-proto): scaffold protos` | meta-swarm | P2.1 W1 | 3 d | — |
| 6 | `feat(efe-meta-swarm-server): scaffold` | meta-swarm | P2.1 W1 | 4 d | #5 |
| 7 | `docs(security): Phase 2 threat model draft` | sec | P2.1 W1 | 2 d | — |
| 8 | `feat(server): wire MetaSwarmService::Iterate` | meta-swarm | P2.1 W2 | 5 d | #5, #6 |
| 9 | `feat(server): wire MetaParticipantService::Step` | meta-swarm | P2.1 W2 | 4 d | #6 |
| 10 | `test: integration_grpc_multiprocess` | meta-swarm | P2.1 W2 | 3 d | #8, #9 |
| 11 | `feat(efe-server): EFE_META_SWARM_ENABLED plumbing` | meta-swarm | P2.2 W3 | 4 d | #5, #8 |
| 12 | `feat: SIGHUP hot-reload + ReloadConfig` | meta-swarm | P2.2 W3 | 5 d | #8, #9 |
| 13 | `feat(charts): Helm sub-chart skeleton` | platform | P2.2 W3 | 4 d | #6, #11 |
| 14 | `feat(charts): full values + templates` | platform | P2.2 W4 | 4 d | #13 |
| 15 | `ci(helm): helm-meta-swarm-lint workflow` | platform | P2.2 W4 | 2 d | #14 |
| 16 | `feat(sec): mTLS + cert-manager Issuer` | sec + platform | P2.2 W4 | 5 d | #6, #13 |
| 17 | `feat: 14 Prometheus metrics` | meta-swarm | P2.3 W5 | 4 d | #8, #9 |
| 18 | `feat: 8 OpenTelemetry span types` | meta-swarm | P2.3 W5 | 4 d | #17 |
| 19 | `feat(grafana): dashboards + alert rules` | sre | P2.3 W5 | 3 d | #17, #18, #14 |
| 20 | `tools(loadgen): traffic generator` | meta-swarm | P2.3 W6 | 3 d | #5, #8 |
| 21 | `feat(public-bench): harness + 5 workloads` | meta-swarm | P2.3 W6 | 4 d | #11 |
| 22 | `feat(baselines): 4 SLOs + rollup #20` | meta-swarm | P2.4 W7 | 2 d | #17, #18 |
| 23 | `test(smoke): rollup #20 pin` | meta-swarm | P2.4 W7 | 1 d | #22 |
| 24 | `bench(public): publish results 2026-08` | meta-swarm | P2.4 W7 | 3 d | #21 |
| — | runbook + 2 SRE drills | sre | P2.4 W7 | 2 d | #14, #19 |
| 25 | `docs: Phase 2 lessons + soak report + roadmap refresh` | meta-swarm | P2.4 W8 | 2 d | all |

Total : **25 PRs**, ~**75 FTE-days** ≈ **15 FTE-weeks** of focused PR work
(over 8 calendar weeks, leaves slack for review, drills, soak observation,
unplanned bug-fixes).

---

## 8. Test gates per sprint

Each sprint has a binary test gate.  Sprint cannot close without it.

### 8.1 Sprint A1.P2.1 gate

| # | Check | How |
|---|---|---|
| T1 | `cargo build -p efe-meta-swarm-proto --offline` | CI green |
| T2 | `cargo build -p efe-meta-swarm-server --offline` (both bins) | CI green |
| T3 | `cargo test -p efe-meta-swarm-server --test integration_grpc_multiprocess --release` | CI green ; bit-identical residual to Phase-1 reference |
| T4 | `cargo doc -p efe-meta-swarm-proto --no-deps` | CI green, 0 warnings |
| T5 | `actionlint .github/workflows/*.yml` | CI green |
| T6 | Phase-1 master-gate still PASS | CI green (no Phase-2 work has broken P1) |

### 8.2 Sprint A1.P2.2 gate

| # | Check | How |
|---|---|---|
| T7 | `helm lint charts/efe/charts/efe-meta-swarm` | CI green |
| T8 | `helm template ./charts/efe` byte-diff vs `golden-phase1-default.yaml` | CI green (= empty diff) |
| T9 | `helm template --set metaSwarm.enabled=true` byte-diff vs `golden-phase2-enabled.yaml` | CI green |
| T10 | `cargo test --test integration_sighup_hot_reload` (1000 random-timed SIGHUPs) | CI green |
| T11 | mTLS e2e handshake test | CI green |
| T12 | `EFE_META_SWARM_ENABLED=false` startup hash identical to Phase 1 | CI green |

### 8.3 Sprint A1.P2.3 gate

| # | Check | How |
|---|---|---|
| T13 | All 14 Prometheus metrics scrape returns names | Manual + smoke |
| T14 | OTel sample trace seen in collector | Manual |
| T15 | 4 alert rules each verified by fault injection | SRE drill log |
| T16 | Loadgen completes 1 h smoke at 30 rps with 0 errors | CI |
| T17 | Public bench harness runs 1 workload in 1 mode | CI |
| T18 | Soak day-7 status : pass criteria S1-S10 all green | SRE daily check |

### 8.4 Sprint A1.P2.4 gate (= L2 gate, cf. § 10)

---

## 9. Soak monitoring plan

### 9.1 Setup checklist (must complete by Sun 2026-08-09 23:59 UTC)

- [ ] Staging cluster scaled to soak topology (1 controller StatefulSet 3-replicas + 3 participant Deployments)
- [ ] Loadgen Job deployed, dry-run for 1 h at 30 rps verified clean
- [ ] Grafana dashboards reviewed + annotations enabled
- [ ] PagerDuty test escalation : page test → SRE on-call → ack within 5 min
- [ ] Run-book printed and on the SRE table
- [ ] SRE on-call rotation confirmed for the full 7 days
- [ ] Slack alert channel `#meta-swarm-soak-alerts` created and connected to Alertmanager
- [ ] Daily-status template pinned in `#meta-swarm-soak`
- [ ] Memory baseline measured (RSS at t=0) and recorded

### 9.2 Daily status template

```
== Soak day N / 7  ==  (YYYY-MM-DD HH:MM UTC)

Convergence rate  : ___ %   (target ≥ 99.95 %)
p99 latency       : ___ ms  (target ≤ 100 ms)
RSS growth vs t=0 : ___ %   (target ≤ 5 %)
Incidents        : Sev1=__  Sev2=__  Sev3=__
Reloads success  : ___ / ___ (target ≥ 99.9 %)
Determinism check: PASS / FAIL
Trace coverage   : ___ %   (target ≥ 5 %)

Open items :
  - ...

Next observation : <time>
```

### 9.3 Stop-the-soak triggers

ANY of the following triggers immediate soak abort + reset to day 0
(after RCA) :

- 1 Sev1 incident
- 1 Sev2 incident
- 2 Sev3 incidents
- Convergence rate < 99.9 % for any 10-min window
- p99 latency > 150 ms for any 10-min window
- Determinism check fails
- Memory grows > 10 % over baseline at any point

### 9.4 Final soak report (Sun 2026-08-17 23:59 UTC)

`docs/A1_PHASE2_SOAK_REPORT.md` — full template in [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 10.3.

---

## 10. L2 gate checklist

10 criteria, each binary, each owned, each verifiable.  All must be
GREEN by Mon 2026-08-24 23:59 UTC for L2 to be attained on 2026-08-31.

| # | Criterion | Owner | Verification |
|---|---|---|---|
| G1 | 7 Phase-1 A1 SLOs GREEN on 7/7 last nightly runs | meta-swarm | `target/ops/master_report.json` archives ; grep `"SLO-A1-META-.*"`, all status PASS |
| G2 | 4 Phase-2 A2 SLOs GREEN on 7/7 last nightly runs | meta-swarm | Same source ; grep `"SLO-A2-.*"` |
| G3 | Master rollup `SLO-OPS-A1-META-SWARM` (#19) PASS 7/7 nightly | meta-swarm | Master report |
| G4 | Master rollup `SLO-OPS-A1-META-GRPC` (#20) PASS 7/7 nightly | meta-swarm | Master report |
| G5 | 7-day soak passes all 10 sub-criteria | sre-team | `A1_PHASE2_SOAK_REPORT.md` published |
| G6 | Public bench published with all 5 × 3 cells filled | meta-swarm | `bench_results/meta-swarm-vs-mono-swarm-2026-08.{csv,html}` |
| G7 | Helm sub-chart lint + golden-diff CI all green on main | platform | `helm-meta-swarm-lint.yml` last 7 runs all green |
| G8 | Threat model `PHASE2_THREAT_MODEL.md` signed off | sec-team | Sec-team review tag on PR #7 + sign-off comment |
| G9 | Runbook `META_SWARM_GRPC.md` written + 2 SRE drills logged | sre-team | Runbook MR + 2 drill logs in `docs/runbooks/drills/` |
| G10 | Postmortem template extended with Meta-Swarm section | sre-team | `docs/postmortems/_template.md` diff |

### 10.1 Verification ceremony (Mon 2026-08-24)

90-min Zoom : whole Phase-2 team + sre-team-lead + eng-leadership.
Each criterion verified by reading the evidence aloud, screen-shared,
2 engineers confirm.  Outcomes recorded in
`docs/A1_PHASE2_L2_GATE_VERIFICATION.md`.

If any criterion is RED :
- Issue raised to `#meta-swarm-blockers`
- 2-week deferral planned
- Re-verification ceremony scheduled for Mon 2026-09-07

---

## 11. RACI per deliverable

R = Responsible, A = Accountable, C = Consulted, I = Informed.

| Deliverable | meta-swarm | platform | sre | sec | product | eng-lead |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| Protos + server scaffold | R/A | I | I | C | I | I |
| `EFE_META_SWARM_ENABLED` plumbing | R/A | I | I | I | I | I |
| SIGHUP hot reload | R/A | I | C | C | I | I |
| Helm sub-chart | C | R/A | C | C | I | I |
| mTLS + cert-manager | C | C | I | R/A | I | I |
| 14 Prometheus metrics | R/A | I | C | I | I | I |
| 8 OTel spans | R/A | I | C | I | I | I |
| Grafana dashboards + alerts | C | I | R/A | I | I | I |
| Loadgen | R/A | I | C | I | I | I |
| Public bench harness | R/A | I | I | I | C | I |
| 4 new SLOs + rollup #20 | R/A | I | C | I | I | I |
| Smoke test #9 | R/A | I | I | I | I | I |
| Public bench publication | R/A | I | I | C | C | I |
| Runbook + drills | I | C | R/A | C | I | I |
| Threat model | I | I | C | R/A | I | I |
| 7-day soak | I | I | R/A | I | I | C |
| L2 gate verification | C | C | R/A | I | I | A |
| Phase 2 lessons doc | R/A | I | I | I | I | I |

---

## 12. Risk burn-down

Risks tracked weekly in `#meta-swarm-risks` channel.  Status one of :
🟢 retired · 🟡 active-monitored · 🔴 firing.

### 12.1 Phase 2 risk register (12 entries)

| ID | Risk | Initial L/I | Owner | Mitigation | Burn-down trigger |
|---|---|---|---|---|---|
| RR-1 | gRPC overhead pushes p99 over 100 ms | M/H | meta-swarm | StepStream + 150 ms headroom | PR #21 bench result |
| RR-2 | SIGHUP race condition corrupts state | L/H | meta-swarm | Atomic write-lock swap + 1000-iter property test | PR #12 merged |
| RR-3 | Cross-process determinism breaks | M/H | meta-swarm | Pin rustc 1.88, blas none, determinism check S9 | PR #10 GREEN |
| RR-4 | Helm chart breaks Phase-1 deployments | L/H | platform | Default-disabled + golden manifest diff | PR #15 merged |
| RR-5 | mTLS cert rotation fails | M/M | sec | cert-manager auto-renew 60-day | First successful rotation observed |
| RR-6 | OTel cardinality blow-up | L/M | sre | Bounded labels + 5 % sample | PR #18 merged + 1 day in soak |
| RR-7 | Straggler participant | M/M | meta-swarm | W5 bench measures ; P3 will fix | PR #21 result |
| RR-8 | Public bench shows mono-swarm wins | M/L | meta-swarm | Acceptable ; honest framing | PR #24 publication |
| RR-9 | L2 gate fails Aug 31 | L/H | meta-swarm | Each criterion independent ; 2-week soft deferral | Gate verification ceremony |
| RR-10 | Soak day N fails → restart from day 0 | M/M | sre | Stop-the-soak triggers § 9.3 well-defined | Soak day 7 |
| RR-11 | PR #16 mTLS delayed (CA infra dependency) | M/H | sec + platform | Start mTLS work at Sprint 2 W3 ; have fallback bearer token path | PR #16 merged |
| RR-12 | SRE on-call coverage gap during soak | L/M | sre | Confirm rotation 1 week before soak start | Soak day 0 |

### 12.2 Weekly burn-down post (Friday async)

```
== Phase 2 risk burn-down · Week N ==
🟢 retired   : RR-2 (PR #12 merged), RR-3 (PR #10 GREEN), ...
🟡 active    : RR-1 (PR #21 ETA Aug 12), ...
🔴 firing    : (none)

New risks discovered : <list>
```

---

## 13. Communication

### 13.1 Channels

| Channel | Purpose | Members |
|---|---|---|
| `#meta-swarm` | General Phase 2 chat | All teams |
| `#meta-swarm-standup` | Async daily standups | meta-swarm |
| `#meta-swarm-risks` | Risk burn-down + pivots | meta-swarm-lead + sre-lead + platform-lead + sec-lead |
| `#meta-swarm-soak` | 7-day soak observation | meta-swarm + sre |
| `#meta-swarm-soak-alerts` | Soak Alertmanager pipe | meta-swarm + sre-oncall |
| `#meta-swarm-blockers` | Critical blockers (= use sparingly) | All + eng-leadership |
| `#announcements` | Phase-gate announcements | All EFE |

### 13.2 Documents shipped

| Doc | When | Audience |
|---|---|---|
| Sprint demo deck | End of each sprint (Fri) | All teams |
| Weekly risk burn-down post | End of each week (Fri) | Eng-leadership + product |
| Daily soak status | Daily during soak | sre + meta-swarm |
| Soak final report | Day 7 of soak | All teams |
| L2 verification report | Mon 2026-08-24 | All teams + eng-leadership |
| L2 announcement | Fri 2026-08-28 | All EFE (#announcements) |
| Phase 2 lessons | Sun 2026-08-30 | All teams |

### 13.3 External communication (post-L2)

- Blog post on `efe.io/blog` (target Fri 2026-08-31) : "Shipping
  efe-meta-swarm: a hierarchical multi-swarm coordinator for EFE
  production".
- Public bench page : `efe.io/bench/meta-swarm-vs-mono-swarm-2026-08`.
- No promise of customer-facing SLA yet (that's Phase 5).

---

## 14. Rollback plan

Phase 2 is **single-flag-rollback** by design.  At any point during
Phase 2 (or post-merge), the rollback is :

```bash
kubectl set env deployment/efe-server EFE_META_SWARM_ENABLED=false
```

Then :
1. `efe-server` immediately falls back to the in-process Phase-1 path.
2. Active gRPC connections drain (close on next idle).
3. Meta-swarm pods can be left running idle OR scaled to zero
   (`kubectl scale statefulset/meta-swarm-controller --replicas=0`).
4. No data corruption is possible (Phase 2 is a transport-layer change
   only ; algorithmic surface unchanged).
5. PagerDuty incident opened with severity matching the trigger.
6. Postmortem within 48 h (workspace SLO `SLO-OPS-POSTMORTEM-VELOCITY`).

### 14.1 Rollback decision authority

- During soak : SRE on-call may roll back unilaterally on a Sev1.
- During canary (week 9-12, post-L2) : SRE + meta-swarm-lead joint
  call on Sev2 ; SRE on-call alone on Sev1.
- During GA (post-canary) : same as above.

### 14.2 What does NOT rollback

- The Helm chart : staying installed with the flag flipped off is
  the supported state.  No `helm uninstall` required.
- The 4 A2 SLO definitions : staying defined-and-SKIP-DT is fine
  per the soft-launch contract.
- The master rollup #20 : staying defined as ACTIVE is correct ;
  it will simply report SKIP-DT if no data flows.

---

## 15. Open questions per sprint

Pending decisions tracked by sprint, with a deadline before they
become blockers.

### Sprint A1.P2.1 — decide by Fri 2026-07-10

| # | Question | Owner | Status |
|---|---|---|---|
| OQ1 | f32 vs f64 wire format ? Recommendation : f32-only in P2. | meta-swarm | open |
| OQ2 | StepStream mandatory or optional ? Recommendation : optional (fallback to Step). | meta-swarm | open |

### Sprint A1.P2.2 — decide by Fri 2026-07-24

| # | Question | Owner | Status |
|---|---|---|---|
| OQ3 | TLS-only vs plaintext-dev ? Recommendation : TLS-only, `--insecure` gated by `EFE_DEV_MODE`. | sec | open |
| OQ4 | Helm umbrella chart major vs minor bump ? Recommendation : minor. | platform | open |
| OQ5 | Single controller vs HA active-passive ? Recommendation : single + K8s lease ; Raft in P4. | meta-swarm + sre | open |

### Sprint A1.P2.3 — decide by Fri 2026-08-07

| # | Question | Owner | Status |
|---|---|---|---|
| OQ6 | Soak start date confirmed ? Recommendation : Mon 2026-08-10. | sre | open |
| OQ7 | mTLS rotation : manual vs auto ? Recommendation : manual P2, auto P3. | sec | open |

### Sprint A1.P2.4 — decide by Fri 2026-08-21

| # | Question | Owner | Status |
|---|---|---|---|
| OQ8 | Sigstore signing of bench HTML ? Recommendation : defer to P3. | meta-swarm + sec | open |

Decisions log in `docs/A1_PHASE2_DECISIONS.md`.

---

## 16. Definition of Done

A PR is "done" iff **all** :

1. Code compiles on stable rustc 1.88 with `--features attention-weighted`.
2. `cargo test -p <crate> --offline` GREEN.
3. `cargo clippy -p <crate> --offline --all-targets --features attention-weighted` produces 0 new warnings on the modified files.
4. `cargo fmt -p <crate>` no diff (or alignment-vertical style consistent with crate convention).
5. `cargo doc -p <crate> --no-deps` clean.
6. Phase-1 master gate stays GREEN (`master_gate_check.py` exit 0).
7. The added/modified tests cover the new logic (smoke + 1 happy path + ≥ 1 negative case).
8. The PR description follows the workspace template (Summary / Test plan / Non-regression statement).
9. ≥ 1 reviewer from a different team has approved (cross-team review).
10. CI is all green.
11. If the PR touches a YAML baseline : the smoke test pins the new SLO id.
12. If the PR touches a workflow : `actionlint` clean.

A **sprint** is "done" iff :

- All PRs of that sprint are merged.
- The sprint gate (§ 8) passes.
- The sprint demo happened and was recorded.
- The retrospective is posted in `#meta-swarm`.

Phase 2 is "done" iff :

- All 4 sprints are done.
- L2 gate (§ 10) passes : 10/10 criteria GREEN.
- L2 announcement post is published in `#announcements` + on `efe.io/blog`.

---

## Appendix A — Cross-references

- Phase 2 architecture : [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md)
- Multi-phase roadmap : [`EFE_A1_ROADMAP.md`](EFE_A1_ROADMAP.md)
- Phase 1 bench tracking : [`EFE_A1_BENCH_TRACKING.md`](EFE_A1_BENCH_TRACKING.md)
- Operational doctrine : [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md)
- Master-gate runbook : [`runbooks/ops_master_gate_fail.md`](runbooks/ops_master_gate_fail.md)
- Phase 1 PR #4 : [`../crates/efe-meta-swarm/PR4_DESCRIPTION.md`](../crates/efe-meta-swarm/PR4_DESCRIPTION.md)

## Appendix B — Glossary

- **L1, L2** : maturity levels (current = L1, target end-of-Phase-2 = L2).
- **T1, T2, T3** : deployment topologies (in-process / single-node 3-procs / K8s pod-per-participant).
- **Sprint A1.P2.N** : Phase 2 Sprint N (N ∈ {1, 2, 3, 4}).
- **Soak** : 7-day continuous internal soak with strict no-incident pass criteria.
- **Soft-launch** : 3-run observation window after merge with `continue-on-error: true` on the new bench job.
- **L2 gate** : 10-criterion checklist (§ 10) that gates phase graduation.
- **A2 SLO** : a Phase-2 SLO (id prefixed `SLO-A2-…`), distinct from Phase-1 `SLO-A1-…`.
- **DT-A1-PHASE2-SOFT-LAUNCH-3-RUNS** : deferred ticket attached to A2 SLOs during the 3-main-run soft-launch window.
