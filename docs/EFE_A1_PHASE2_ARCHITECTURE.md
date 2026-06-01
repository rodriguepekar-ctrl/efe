# EFE Sprint A1 — Phase 2 : Transport gRPC

> **Status** : DRAFT (architecture frozen 2026-05-13).
> **Owners** : meta-swarm-team (lead), platform-team (Helm), sre-team (observability).
> **Timeline** : 2026-07-01 → 2026-08-31 (9 weeks, 4 sprints).
> **Gate**  : L1 (PR #4 master-gate hook merged) → **L2 (multi-process production)**.
> **Predecessor** : [`EFE_A1_PHASE1_ARCHITECTURE.md`](EFE_A1_PHASE1_ARCHITECTURE.md).
> **Successor**   : Phase 3 (operational hardening, cross-tenant, scale-out).

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Objectives & non-objectives](#1-objectives--non-objectives)
2. [Constraints inherited from Phase 1](#2-constraints-inherited-from-phase-1)
3. [Topology overview](#3-topology-overview)
4. [Protocol Buffers — service design](#4-protocol-buffers--service-design)
5. [Rust API ↔ gRPC mapping](#5-rust-api--grpc-mapping)
6. [Multi-process orchestration](#6-multi-process-orchestration)
7. [Configuration : env vars + SIGHUP hot-reload](#7-configuration--env-vars--sighup-hot-reload)
8. [Helm sub-chart `efe-meta-swarm`](#8-helm-sub-chart-efe-meta-swarm)
9. [Observability : Prometheus + OpenTelemetry](#9-observability--prometheus--opentelemetry)
10. [Internal 7-day soak (EFE-team-only)](#10-internal-7-day-soak)
11. [Public bench : meta-swarm vs mono-swarm](#11-public-bench--meta-swarm-vs-mono-swarm)
12. [Gate 2 → 3 (L2 attained) criteria](#12-gate-2--3-l2-attained-criteria)
13. [Code structure additions](#13-code-structure-additions)
14. [SLO budget additions (Phase 2)](#14-slo-budget-additions-phase-2)
15. [Security model : mTLS + scope guards](#15-security-model)
16. [Migration path Phase 1 → Phase 2](#16-migration-path-phase-1--phase-2)
17. [Risk matrix + mitigations](#17-risk-matrix--mitigations)
18. [Detailed timeline (week-by-week)](#18-detailed-timeline)
19. [CI / CD pipeline additions](#19-cicd-pipeline-additions)
20. [Documentation deliverables](#20-documentation-deliverables)
21. [Open questions for Phase 2 kickoff](#21-open-questions)

---

## 0. Executive summary

Phase 2 lifts `efe-meta-swarm` from an **in-process Rust library** (Phase 1
deliverable, PR #1-#4) to a **multi-process gRPC-distributed coordinator**
suitable for production deployment alongside `efe-server`.  The
deliverables in priority order :

1. **Two new gRPC services** (`MetaSwarmService`, `MetaParticipantService`),
   declared as **additive** protos — no breaking change to existing
   `efe.v1.*` services.
2. **Multi-process binary** : a new `efe-meta-swarm-server` crate that
   exposes the Phase-1 in-process API behind a gRPC façade.  Supports
   running 3 local instances on dev box and N pod-per-participant on K8s.
3. **Configuration** : a single env var `EFE_META_SWARM_ENABLED` enables
   the feature with no other change ; SIGHUP triggers a hot reload of
   participant topology + safety polytopes (no process restart).
4. **Helm sub-chart** `charts/efe-meta-swarm/` packaged inside the
   workspace umbrella chart, **default-disabled** — opted-in via
   `--set metaSwarm.enabled=true`.
5. **Observability** : 14 Prometheus metrics + 8 OpenTelemetry span
   types, two dashboards (operational + algorithmic), four alert rules.
6. **Soak validation** : 7 consecutive days of EFE-team-only traffic
   with zero Sev1/Sev2.
7. **Public bench** : a reproducible head-to-head between meta-swarm
   (Phase 2) and mono-swarm (the legacy `efe-server`-only path) under
   five workloads, published as a transparent CSV + interactive HTML.
8. **L2 gate** : achieved iff (a) soak passes, (b) bench is published,
   (c) the 7 surface A1 SLOs + 4 new Phase-2 SLOs are ACTIVE.

Phase 2 does **not** introduce :

- Cross-region replication (Phase 3).
- Tenant-aware sharding (Phase 3).
- Customer-facing SLA tracker (Phase 5).
- BLAS / GPU acceleration (deferred ; tracked under `DT-A1-BLAS-PHASE3`).

The architecture is intentionally **additive over Phase 1** — the
in-process path stays valid for unit tests and embedded use cases ;
only the deployment surface gains a network transport.

---

## 1. Objectives & non-objectives

### 1.1 Objectives (must-have)

| # | Objective | Verification |
|---|---|---|
| O1 | Run `MetaSwarm::iterate()` across 3 OS processes with bit-identical results vs in-process Phase 1 reference. | `tests/integration_grpc_multiprocess.rs` ; bit-identical residuals to `examples/three_swarms_kovant.rs` reference. |
| O2 | New gRPC services are **additive** : zero modification to `efe.v1.classify`, `efe.v1.admin`, `efe.v1.mode2`, `efe.v1.mode3`, `efe.v1.catalog`. | `git diff --stat crates/efe-server/proto/efe/v1/*` empty for these 5 files. |
| O3 | Feature flag `EFE_META_SWARM_ENABLED` defaults to `false` ; existing deployments are byte-identical without it. | `efe-server` startup smoke test compares `/var/log/efe-server.log` with/without the env var unset. |
| O4 | SIGHUP triggers a hot reload of (i) participant manifest, (ii) safety polytopes, (iii) coupling-gamma, in ≤ 200 ms p99, without dropping in-flight iterations. | `tests/integration_sighup_hot_reload.rs` + new SLO `SLO-A1-META-RELOAD-P99`. |
| O5 | Helm sub-chart `efe-meta-swarm` is **default-disabled** ; production parity stays at Phase-1 until ops explicitly enables it. | `helm template --set metaSwarm.enabled=false` produces a manifest set strictly identical to Phase 1. |
| O6 | Observability : 14 Prometheus metrics + 8 OTel span types, scraped by the existing Prometheus instance, sent via existing OTel collector. | Manual smoke + `tests/integration_obs_phase2.rs`. |
| O7 | 7-day internal soak passes with zero Sev1/Sev2 and ≤ 1 Sev3. | Soak report `docs/A1_PHASE2_SOAK_REPORT.md`. |
| O8 | Public bench published as `bench_results/meta-swarm-vs-mono-swarm-2026-08.{csv,html}`. | Artifact attached to the L2 graduation issue. |

### 1.2 Non-objectives (explicitly out-of-scope, deferred)

| # | Out-of-scope | Phase that owns it |
|---|---|---|
| N1 | Cross-region replication of meta-swarm state. | Phase 3 (Q4 2026). |
| N2 | Per-tenant meta-swarm instances + isolation. | Phase 3. |
| N3 | Customer-facing SLA (99.95 % availability). | Phase 5 (2027). |
| N4 | BLAS / cuBLAS / Metal acceleration of `Tensor::multiply`. | Tracked as `DT-A1-BLAS-PHASE3` ; not gated on Phase 2. |
| N5 | Distributed consensus on a quorum > 3 participants. | Phase 4 (raft-based, ETA Q1 2027). |
| N6 | mTLS rotation automation. Phase 2 ships with manual certificate rotation. | Phase 3. |
| N7 | Web UI for meta-swarm health. | Phase 5. |

### 1.3 Success metrics (quantified)

- **Functional parity** : `examples/three_swarms_kovant.rs` produces the
  same `(iterations, final_residual)` tuple bit-identically in Phase 1
  vs Phase 2 multi-process (residual.to_bits() equality across 100
  reruns).
- **Latency overhead** : Phase 2 `iterate()` p99 ≤ Phase 1 p99 × 2.5
  (gRPC round-trip budget : ~30 ms p99 added on top of the Phase-1
  ~12 ms p99 → target ≤ 100 ms p99 in Phase 2, which matches
  `SLO-A1-META-LATENCY-P99` budget).
- **Throughput** : Phase 2 server sustains ≥ 100 `iterate()` requests/sec
  on a 4-vCPU pod (≥ 80 % of in-process Phase 1 throughput).
- **Hot-reload safety** : SIGHUP during in-flight iteration → either
  (a) iteration completes under old config, or (b) cleanly aborts with
  `StopReason::ConfigReloaded` and never produces a partial result.

---

## 2. Constraints inherited from Phase 1

These invariants from Phase 1 (PR #1-#4) must remain true post-Phase 2 :

| # | Phase-1 invariant | Phase-2 obligation |
|---|---|---|
| C1 | `MetaSwarm` public API surface is additive-only (no breaking changes since `v0.1.0`). | Phase 2 wraps the in-process API ; does not modify its signature. |
| C2 | `examples/three_swarms_kovant.rs` converges in 47 iters with residual ≤ 1e-3, deterministic across reruns. | Phase 2 keeps the contractive operator semantics ; gRPC adds ordering guarantees but not new fixed-point dynamics. |
| C3 | The 7 surface SLOs `SLO-A1-META-*` and the master rollup `SLO-OPS-A1-META-SWARM` (added PR #4) remain ACTIVE. | Phase 2 adds 4 new SLOs (latency overhead, reload p99, OTel-trace-coverage, multi-proc determinism) ; pre-existing rollup unchanged. |
| C4 | `efe-meta-swarm` crate stays `UNLICENSED` (carve-out R5-B1 of `EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`). | Phase 2 adds `efe-meta-swarm-server` and `efe-meta-swarm-proto` crates under the same carve-out. |
| C5 | The `core-integration` feature gate is the only opt-in for the `LocalParticipant<S: efe_core::Swarm>` adapter (PR #2). | Phase 2 introduces a parallel `RemoteParticipant` gated by `--features remote-grpc`. |
| C6 | Doctrine anti-Φ-D.A1 : every SLO is ACTIVE or SKIP-DT. No SILENT-SKIP. | Phase 2 SLOs carry `deferred_ticket: DT-A1-PHASE2-SOFT-LAUNCH-3-RUNS` during the 3-run observation window. |
| C7 | `[profile.release].panic = "abort"`. Sidecars must persist incrementally. | The Phase-2 server crate **overrides** to `panic = "unwind"` because crashing one participant must not abort the whole process (graceful restart). Documented in `crates/efe-meta-swarm-server/Cargo.toml`. |
| C8 | Master gate `bench_baselines/v0.9.0/ops_master_slos.yaml` has 19 master rollups (PR #4 added #19). | Phase 2 adds rollup #20 `SLO-OPS-A1-META-GRPC` and rolls 4 new Phase-2 surface SLOs into it. |

---

## 3. Topology overview

Phase 2 introduces **three** deployment topologies that share a single
binary and protocol :

### 3.1 Topology T1 — In-process (Phase-1 legacy, unchanged)

```
┌──────────────────────────────────┐
│  efe-server (single process)     │
│  ├── efe-server::services::*     │
│  └── efe-meta-swarm (linked lib) │
│      └── MetaSwarm::iterate()    │
└──────────────────────────────────┘
```

- No gRPC, no extra processes.
- The `EFE_META_SWARM_ENABLED` env var defaults to false → this
  topology is byte-identical to Phase 1 prod.
- Useful for tests, embedded use, and any deployment that opts out
  of multi-process.

### 3.2 Topology T2 — Single-node, 3 local instances (dev / staging)

```
┌─────────────────────┐    ┌────────────────────────┐
│  efe-server (1)     │◀──▶│ efe-meta-swarm-server  │
│  routes RPC to MS   │    │ (controller, port 50080)│
└─────────────────────┘    └──┬───────┬───────┬─────┘
                              │       │       │
                              ▼       ▼       ▼
                  participant-1  participant-2  participant-3
                  (port 50091)   (port 50092)   (port 50093)
                  supplier-      contract-      spend-
                  scoring        risk           categorization
```

- 1 controller + 3 participant processes on the same host.
- Each participant is a separate OS process implementing
  `MetaParticipantService`.
- Controller multiplexes `MetaSwarmService::Iterate` across them.
- Used for the 7-day soak (§ 10) and CI integration tests
  (`bench-meta-swarm-grpc-multiproc`).

### 3.3 Topology T3 — Kubernetes, pod-per-participant (production)

```
                            ┌────────────────────────────┐
                            │   efe-server pod (existing) │
                            └────────────┬───────────────┘
                                         │ gRPC
                                         ▼
                ┌────────────────────────────────────────┐
                │  meta-swarm-controller-{0,1,2}         │
                │  StatefulSet, 3 replicas (HA quorum)   │
                └────────┬───────────┬───────────────────┘
                         │           │
                         ▼           ▼
                  ┌─────────┐  ┌─────────┐  ┌─────────┐
                  │ part-0  │  │ part-1  │  │ part-2  │
                  │ (sup.)  │  │ (con.)  │  │ (spn.)  │
                  │ Deploym │  │ Deploym │  │ Deploym │
                  └─────────┘  └─────────┘  └─────────┘
```

- `efe-meta-swarm-controller` : StatefulSet, 3 replicas for HA,
  Raft-light election (Phase 4 will upgrade to proper Raft).
  For Phase 2 we run an **active-passive** model : 1 leader, 2
  hot standby, ZooKeeper / etcd not required (uses K8s lease objects).
- `efe-meta-swarm-participant-*` : Deployment, 1 replica each in
  Phase 2 (one pod per logical participant).  HPA disabled — scaling
  out a single participant is Phase 4.
- All wired via headless Services (`controller-headless`,
  `participant-supplier-scoring`, …) + ClusterIP for the
  controller's primary endpoint.

The Helm sub-chart (§ 8) defaults T3 to **disabled** (`metaSwarm.enabled
= false`), so production stays on T1 until the platform team flips the
flag during the L2 graduation rollout.

### 3.4 Transport selection logic

```
EFE_META_SWARM_ENABLED unset OR =false  →  T1 (in-process)
EFE_META_SWARM_ENABLED=true, EFE_META_SWARM_DISCOVERY=static-localhost  →  T2
EFE_META_SWARM_ENABLED=true, EFE_META_SWARM_DISCOVERY=k8s-dns  →  T3
```

`EFE_META_SWARM_DISCOVERY` defaults to `static-localhost` (= T2).
The Helm chart sets it to `k8s-dns` automatically.

---

## 4. Protocol Buffers — service design

Two new protos, sibling to the existing `efe/v1/*.proto` set :

```
crates/efe-meta-swarm-proto/proto/
├── efe/
│   └── v1/
│       └── meta_swarm/
│           ├── service_meta_swarm.proto       (MetaSwarmService)
│           ├── service_meta_participant.proto (MetaParticipantService)
│           ├── messages.proto                 (shared message types)
│           └── errors.proto                   (Phase-2-specific errors)
```

### 4.1 `MetaSwarmService` (controller-facing)

```protobuf
syntax = "proto3";
package efe.v1.meta_swarm;
option go_package = "github.com/efe/proto/v1/meta_swarm;metaswarm";

import "efe/v1/meta_swarm/messages.proto";

// Coordinator-facing service.  `efe-server` calls these RPCs ;
// `efe-meta-swarm-controller` implements them.
service MetaSwarmService {

  // 1. Resolve a fixed-point of the joint contractive system.
  //   Maps directly to `MetaSwarm::iterate()`.  Blocking — caller is
  //   expected to wait up to `max_wall_ms`.  Server-side limit is
  //   capped at 30 s ; longer requests are rejected with
  //   `DEADLINE_EXCEEDED`.
  rpc Iterate(IterateRequest) returns (IterateResponse);

  // 2. Streaming version — used by the public bench (§ 11) to
  //   capture the residual trajectory.  Each chunk corresponds to
  //   one inner step.  Closes with the final result.
  rpc IterateStream(IterateRequest) returns (stream IterateChunk);

  // 3. One-shot evaluation of the meta-operator at a frozen state.
  //   Maps to `MetaSwarm::step_once()`.  Used by Phase-3 latency-
  //   sensitive paths.
  rpc StepOnce(StepOnceRequest) returns (StepOnceResponse);

  // 4. Reset participant states to zero (or to a provided init).
  //   Maps to `MetaSwarm::reset()`.
  rpc Reset(ResetRequest) returns (ResetResponse);

  // 5. Hot-reload the participant manifest + safety polytopes.
  //   Triggered by SIGHUP on the controller (§ 7.2) or by an admin
  //   RPC.  Returns the diff that was applied.
  rpc ReloadConfig(ReloadConfigRequest) returns (ReloadConfigResponse);

  // 6. Health + readiness probe.  Cheap (<1 ms).  Cf. § 9.4.
  rpc Health(HealthRequest) returns (HealthResponse);

  // 7. Server info (build sha, configured participants, current
  //   leader id).  Used by the Phase-2 dashboard.
  rpc Info(InfoRequest) returns (InfoResponse);
}
```

### 4.2 `MetaParticipantService` (participant-facing)

```protobuf
syntax = "proto3";
package efe.v1.meta_swarm;

import "efe/v1/meta_swarm/messages.proto";

// Participant-facing service.  The controller calls these RPCs ;
// each `efe-meta-swarm-participant-*` implements them.
service MetaParticipantService {

  // 1. Receive a pulled vector from the meta-space and produce the
  //   next contractive step.  Maps to `SwarmParticipant::step_with_pull()`.
  //   The most-frequent RPC : N participants × iters ~50 round-trips
  //   per outer Iterate().
  rpc Step(StepRequest) returns (StepResponse);

  // 2. Reset to the per-participant seed state.
  rpc Reset(ResetParticipantRequest) returns (ResetParticipantResponse);

  // 3. Return the current state tensor (for inspection / debug).
  //   NOT called on the hot path ; reserved for the Phase-2
  //   dashboard and the consistency probe.
  rpc State(StateRequest) returns (StateResponse);

  // 4. Server-streaming variant for the bench : emit one Step per
  //   request, replying with a stream of step results.  Saves a
  //   round-trip per step.  Optional — controllers MAY use Step.
  rpc StepStream(stream StepRequest) returns (stream StepResponse);

  // 5. Capability query.  Returns the participant's dimension,
  //   contractive constant, RNG seed, build version, and the list
  //   of features it supports (e.g. `BLAS_BACKEND=mkl`).  Called
  //   once at controller startup + on every SIGHUP.
  rpc Capabilities(CapabilitiesRequest) returns (CapabilitiesResponse);

  // 6. Health + readiness.
  rpc Health(HealthRequest) returns (HealthResponse);
}
```

### 4.3 `messages.proto` (shared types)

```protobuf
syntax = "proto3";
package efe.v1.meta_swarm;

// A dense f32 tensor of dimension `dim`, row-major.  We use f32
// (not f64) on the wire to halve bandwidth — the controller-side
// math runs in f32 in Phase-1, matching the bench scenario.
//
// Note (Phase 3) : we may add `repeated double dim_d` + a `precision`
// enum field for tenants that require f64.  Phase 2 stays f32-only.
message TensorF32 {
  uint32 dim = 1;
  repeated float data = 2 [packed = true];
}

// A meta-space configuration : dimension + symmetric box bounds.
// Other safety geometries (polyhedron, learned) are sent as
// `geometry_oneof` — extensible.
message MetaSpaceConfig {
  uint32 dim = 1;
  oneof safety_geometry {
    BoxSafety box   = 2;
    Polyhedron poly = 3;
    LearnedHull hull = 4;
  }
}

message BoxSafety {
  float radius = 1;   // symmetric, ±radius per axis
}

message Polyhedron {
  // A·x ≤ b ; rows packed as flat f32 dim_a × dim
  repeated float a_flat = 1 [packed = true];
  repeated float b      = 2 [packed = true];
  uint32 dim_a = 3;
}

message LearnedHull {
  bytes serialized_model = 1;  // bincode (Phase 2 placeholder)
  string version         = 2;
}

// A participant's static descriptor — sent at Capabilities query.
message ParticipantDescriptor {
  string id              = 1;
  uint32 dim             = 2;
  float  kappa           = 3;
  uint64 seed            = 4;
  string build_sha       = 5;
  string blas_backend    = 6;   // "none" | "mkl" | "openblas" | "metal"
  repeated string features = 7;  // ["determinism", "metrics", "trace"]
}

// Iteration knobs — mirrors `MetaSwarmBuilder` config.
message IterationConfig {
  uint32 max_iters       = 1;   // ≤ 200
  double tolerance       = 2;   // joint residual
  double alpha_default   = 3;
  double coupling_gamma  = 4;
  bool   deterministic   = 5;   // bit-identical reruns guaranteed
  uint32 max_wall_ms     = 6;   // server caps at 30000
}

// Iterate request / response.
message IterateRequest {
  // Optional : if absent, controller uses its persisted manifest.
  repeated ParticipantDescriptor expected_participants = 1;
  MetaSpaceConfig meta = 2;
  IterationConfig iteration = 3;
  // Optional : if absent, participants use their last state
  // (or zero if just reset).
  repeated TensorF32 initial_states = 4;
  // Optional : tracing context (W3C TraceParent).
  string traceparent = 5;
}

message IterateResponse {
  uint32 iterations          = 1;
  double final_residual      = 2;
  TensorF32 meta_consensus   = 3;
  repeated TensorF32 final_states = 4;  // per participant, in order
  StopReason stop_reason     = 5;
  uint64 wall_us             = 6;
  // Trace id propagated back for cross-process correlation.
  string trace_id            = 7;
}

enum StopReason {
  STOP_REASON_UNSPECIFIED   = 0;
  STOP_REASON_CONVERGED     = 1;
  STOP_REASON_MAX_ITERS     = 2;
  STOP_REASON_SAFETY_FAIL   = 3;
  STOP_REASON_PARTICIPANT_DOWN = 4;
  STOP_REASON_DEADLINE      = 5;
  STOP_REASON_CONFIG_RELOAD = 6;
}

// Streaming chunk — one inner step per chunk.
message IterateChunk {
  uint32 step          = 1;
  double residual      = 2;
  // Per-participant inner state norms — useful for the bench HTML.
  repeated float participant_norms = 3 [packed = true];
}

// Step (participant-facing) request / response.
message StepRequest {
  TensorF32 pulled_from_meta = 1;
  // Step index for the participant — used for deterministic seeding
  // of the participant's local RNG (Phase 3 sharded variance models).
  uint32 step_index = 2;
}

message StepResponse {
  TensorF32 new_state = 1;
  double    local_residual = 2;
  // Latency self-report for SLO-A1-META-PARTICIPANT-LATENCY-P99.
  uint64    step_us = 3;
}

// ReloadConfig — server-side.
message ReloadConfigRequest {
  // If empty : the controller re-reads its on-disk manifest.
  // If non-empty : the new manifest is provided inline (admin RPC).
  optional MetaSpaceConfig new_meta = 1;
  repeated ParticipantDescriptor new_participants = 2;
  bool drain_inflight = 3;  // if true : finish current Iterate(),
                            // else : abort with CONFIG_RELOAD.
}

message ReloadConfigResponse {
  uint32 reload_gen      = 1;  // monotonically-increasing counter
  repeated string applied_changes = 2;
  uint64 reload_wall_us  = 3;
  bool   inflight_aborted = 4;
}

// Health probe.
message HealthRequest {}
message HealthResponse {
  enum Status {
    STATUS_UNSPECIFIED = 0;
    STATUS_SERVING     = 1;
    STATUS_NOT_SERVING = 2;
    STATUS_RELOADING   = 3;   // SIGHUP in flight
  }
  Status status       = 1;
  uint32 inflight     = 2;  // current concurrent Iterate calls
  uint32 reload_gen   = 3;
}

// Info — for the dashboard.
message InfoRequest {}
message InfoResponse {
  string build_sha           = 1;
  string proto_version       = 2;  // "v1"
  repeated ParticipantDescriptor configured_participants = 3;
  string leader_pod_name     = 4;  // for T3 K8s topology
  uint64 uptime_s            = 5;
  uint32 last_reload_gen     = 6;
}
```

### 4.4 `errors.proto`

```protobuf
syntax = "proto3";
package efe.v1.meta_swarm;

// Phase-2 error codes.  Mapped to gRPC standard codes (FAILED_PRECONDITION,
// UNAVAILABLE, …) at the transport layer ; the detailed code is the
// `error_code` field on the trailer.
enum MetaSwarmErrorCode {
  META_ERROR_UNSPECIFIED        = 0;
  // Configuration errors (FAILED_PRECONDITION).
  META_ERROR_DIM_MISMATCH       = 1;
  META_ERROR_DUPLICATE_ID       = 2;
  META_ERROR_INVALID_SAFETY     = 3;
  META_ERROR_KAPPA_OUT_OF_RANGE = 4;
  // Runtime errors (INTERNAL).
  META_ERROR_PARTICIPANT_DOWN   = 10;
  META_ERROR_DETERMINISM_BROKEN = 11;
  META_ERROR_SAFETY_VIOLATION   = 12;
  // Reload errors (FAILED_PRECONDITION).
  META_ERROR_RELOAD_BUSY        = 20;
  META_ERROR_RELOAD_DIM_CHANGED = 21;
}
```

### 4.5 Versioning & backwards compatibility

- Package name `efe.v1.meta_swarm` — the `v1` suffix is the API
  version, **frozen** at Phase 2 GA.
- Field numbers are reserved past `100` for Phase-3 additions.
- Phase 3 will add `efe.v2.meta_swarm` if a breaking change is
  needed ; `v1` will be deprecated but not removed for at least
  6 months.
- Wire format : proto3 with explicit `optional` keyword on fields
  that may be absent (e.g. `new_meta` in `ReloadConfigRequest`).

### 4.6 Code generation

```bash
# Generated by build.rs of efe-meta-swarm-proto :
crates/efe-meta-swarm-proto/src/lib.rs              # tonic-built
crates/efe-meta-swarm-proto/src/efe.v1.meta_swarm.rs
```

Build script uses `tonic-build 0.11+` with the existing workspace
`prost` version (pinned in workspace `[dependencies]`).

---

## 5. Rust API ↔ gRPC mapping

The bijection between the Phase-1 in-process API and the Phase-2
network surface :

| Phase 1 (in-process) | Phase 2 (gRPC) | Notes |
|---|---|---|
| `MetaSwarm::iterate() -> IterateResult` | `MetaSwarmService::Iterate(IterateRequest) -> IterateResponse` | Wall-time budget served by `max_wall_ms`. |
| `MetaSwarm::step_once() -> StepOnceResult` | `MetaSwarmService::StepOnce` | Hotpath optimisation for Phase 3. |
| `MetaSwarm::reset() -> ()` | `MetaSwarmService::Reset(ResetRequest)` | Reset propagates to all participants. |
| `SwarmParticipant::step_with_pull(pulled) -> Tensor` | `MetaParticipantService::Step(StepRequest) -> StepResponse` | The most-frequent RPC ; subject to streaming optimisation. |
| `SwarmParticipant::reset()` | `MetaParticipantService::Reset` | |
| `SwarmParticipant::current_state() -> Tensor` | `MetaParticipantService::State` | Debug / inspection. |
| `MetaSwarmBuilder::build()` | Server-side : controller builds from manifest on startup or SIGHUP. | Client never builds. |
| `MockParticipant::with_contraction(id, dim, kappa, seed)` | `MetaParticipantService::Capabilities -> ParticipantDescriptor` | Server announces. |
| `BoxSafety::symmetric(dim, r)` | `MetaSpaceConfig.safety_geometry.box.radius` | Wire format. |
| (none — Phase 2 only) | `MetaSwarmService::ReloadConfig` | New, Phase 2. |
| (none) | `MetaSwarmService::Health`, `Info` | New, Phase 2. |

### 5.1 Client adapter

A new type `RemoteMetaSwarm` (in `efe-meta-swarm-server::client`)
implements a `Drive`-like API matching `MetaSwarm` :

```rust
pub struct RemoteMetaSwarm {
    client: MetaSwarmServiceClient<tonic::transport::Channel>,
    config: IterationConfig,
}

impl RemoteMetaSwarm {
    pub async fn iterate(&self) -> Result<IterateResult, MetaSwarmError> {
        let resp = self.client
            .clone()
            .iterate(IterateRequest { /* ... */ })
            .await?;
        Ok(resp.into())
    }
}
```

The client side **does not** depend on `efe-meta-swarm` (the in-process
crate) ; it only depends on `efe-meta-swarm-proto`.  This keeps the
client surface minimal and avoids accidentally pulling the contractive
math into client binaries.

### 5.2 Server adapter

The server side wraps the Phase-1 `MetaSwarm` directly :

```rust
pub struct MetaSwarmGrpcServer {
    inner: Arc<RwLock<MetaSwarm>>,   // Phase-1 in-process state
    manifest_path: PathBuf,
    reload_gen: AtomicU32,
    metrics: MetricsRegistry,
}

#[tonic::async_trait]
impl MetaSwarmService for MetaSwarmGrpcServer {
    async fn iterate(
        &self,
        req: Request<IterateRequest>,
    ) -> Result<Response<IterateResponse>, Status> {
        // 1. Tracing : propagate W3C traceparent.
        let span = tracing::Span::current();
        if let Some(tp) = req.metadata().get("traceparent") { ... }
        // 2. Acquire read lock on the inner MetaSwarm.
        let inner = self.inner.read().await;
        // 3. Apply optional initial_states.
        if let Some(states) = req.into_inner().initial_states { ... }
        // 4. Run iterate() with deadline = max_wall_ms.
        let result = tokio::time::timeout(
            Duration::from_millis(req.iteration.max_wall_ms.into()),
            tokio::task::spawn_blocking({
                let inner = inner.clone();
                move || inner.iterate()
            }),
        ).await????;
        // 5. Convert + return.
        Ok(Response::new(result.into_proto()))
    }
    ...
}
```

Key points :

- `spawn_blocking` because Phase-1 `iterate()` is synchronous.
  Phase-3 may move to an async core.
- `RwLock` because SIGHUP needs write access to swap the manifest ;
  in-flight `iterate()` holds a read lock and is safe.
- Metrics are recorded at every step boundary (§ 9.1).

---

## 6. Multi-process orchestration

### 6.1 Process lifecycle

Each binary type follows the standard EFE pattern :

1. Parse CLI args + env vars.
2. Init tracing + metrics (Prometheus exporter on `:9090/metrics`).
3. Build inner state from manifest.
4. Bind to its gRPC port.
5. Install signal handlers : SIGTERM (graceful shutdown), SIGHUP
   (reload), SIGUSR1 (dump debug info to stderr).
6. Enter the tonic server loop.
7. On SIGTERM : `drain_inflight=true`, refuse new RPCs, wait up to
   30 s for in-flight to drain, then exit.

### 6.2 Discovery

T2 (single-node) uses static addresses :

```bash
EFE_META_SWARM_PARTICIPANTS=supplier-scoring=localhost:50091,contract-risk=localhost:50092,spend-categorization=localhost:50093
```

T3 (K8s) uses DNS :

```bash
EFE_META_SWARM_DISCOVERY=k8s-dns
EFE_META_SWARM_PARTICIPANT_SERVICES=participant-supplier-scoring.efe.svc.cluster.local:50090,...
```

The discovery layer is a `trait Discovery` with two impls : `StaticDiscovery`
and `K8sDnsDiscovery` (no client-go ; pure DNS lookup, refreshed every
10 s).

### 6.3 Failure modes (Phase 2 acceptable)

| Failure | Behaviour | Recovery |
|---|---|---|
| Participant pod crashes mid-iterate | `Iterate` returns `STOP_REASON_PARTICIPANT_DOWN` ; controller increments `meta_swarm_iterate_failures_total{reason="participant_down"}`. | K8s restarts pod ; next `Iterate` succeeds once the pod is `Ready`. |
| Controller pod crashes | New leader elected via K8s lease ; in-flight `Iterate` calls return UNAVAILABLE. | Client retries (`efe-server` has standard exponential backoff). |
| Network partition (controller ↔ participant) | gRPC keepalive (every 10 s) detects within 30 s ; controller marks participant DOWN. | Auto-rejoin on partition heal. |
| SIGHUP during iterate | `drain_inflight=true` (default) waits up to 30 s ; `=false` aborts with CONFIG_RELOAD. | Either way, no partial results leak. |
| Determinism broken (residual.to_bits diverges) | `Iterate` returns `META_ERROR_DETERMINISM_BROKEN` ; controller logs at WARN ; alert fires. | Manual investigation. |

### 6.4 Drain protocol

1. Controller receives SIGTERM.
2. Health endpoint flips to `NOT_SERVING`.
3. K8s `terminationGracePeriodSeconds: 60` → `preStop` hook adds
   a 10 s drain delay so the load balancer dedups.
4. New `Iterate` calls return UNAVAILABLE immediately.
5. Existing in-flight calls complete normally (≤ 30 s budget).
6. Process exits with code 0.

---

## 7. Configuration : env vars + SIGHUP hot-reload

### 7.1 Environment variables

```bash
# === Core toggle ===
EFE_META_SWARM_ENABLED=false                 # default ; true to opt in

# === Discovery ===
EFE_META_SWARM_DISCOVERY=static-localhost    # | k8s-dns
EFE_META_SWARM_PARTICIPANTS=…                # static : <id>=<host:port>,…
EFE_META_SWARM_PARTICIPANT_SERVICES=…        # k8s-dns : full service FQDNs

# === Controller binding ===
EFE_META_SWARM_CONTROLLER_ADDR=0.0.0.0:50080 # the controller's gRPC
EFE_META_SWARM_CONTROLLER_METRICS=:9090      # Prometheus scrape addr

# === Participant binding (only on participant pods) ===
EFE_META_SWARM_PARTICIPANT_ID=supplier-scoring  # which participant am I
EFE_META_SWARM_PARTICIPANT_ADDR=0.0.0.0:50090
EFE_META_SWARM_PARTICIPANT_METRICS=:9091

# === Manifest ===
EFE_META_SWARM_MANIFEST=/etc/efe-meta-swarm/manifest.yaml

# === TLS / auth ===
EFE_META_SWARM_TLS_CERT=/etc/efe/tls/meta-swarm.crt
EFE_META_SWARM_TLS_KEY=/etc/efe/tls/meta-swarm.key
EFE_META_SWARM_TLS_CA=/etc/efe/tls/ca.crt
EFE_META_SWARM_AUTH_TOKEN=…                  # static bearer (Phase-3 mTLS-only)

# === Hot-reload ===
EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S=30     # cf. § 6.4
EFE_META_SWARM_RELOAD_ABORT_INFLIGHT=false   # default : drain

# === Limits ===
EFE_META_SWARM_MAX_INFLIGHT=10               # concurrent Iterate cap
EFE_META_SWARM_DEFAULT_MAX_WALL_MS=15000     # if client omits

# === Observability ===
EFE_META_SWARM_OTEL_ENDPOINT=http://otel:4317
EFE_META_SWARM_TRACE_SAMPLE_RATIO=0.05       # 5 % default

# === Carryover Phase 1 ===
EFE_META_SWARM_DETERMINISTIC=true            # SLO-A1-META-DETERMINISM
EFE_META_SWARM_LAST_P99_OBSERVED_MS=…        # consumed by NON-REGRESSION
```

### 7.2 SIGHUP hot-reload semantics

The reload protocol :

```
1. Operator kills -HUP <controller_pid>     (or `kubectl rollout restart` per pod).
2. Signal handler triggers ReloadConfig RPC (loopback or in-proc).
3. Controller :
   a. Increment reload_gen.
   b. Acquire write lock on inner MetaSwarm.
   c. Wait up to `drain_timeout_s` for in-flight Iterate to finish
      (if drain_inflight=true) OR abort them (if =false).
   d. Re-read manifest from EFE_META_SWARM_MANIFEST.
   e. Diff new manifest vs current :
      - Dimension change ?    → REJECT (META_ERROR_RELOAD_DIM_CHANGED).
      - Participant id added  → call Capabilities on new participant,
                                fail-closed if it cannot handshake.
      - Participant id removed → wait for in-flight, then drop.
      - Safety geometry changed → swap atomically.
      - kappa / seed changed  → applied at next Iterate.
   f. Release write lock.
   g. Emit metric `meta_swarm_reload_total{outcome="ok|fail"}`.
4. Health endpoint flips to RELOADING during step c-f.
5. Return diff to caller in ReloadConfigResponse.applied_changes.
```

**Invariant** : either the whole reload commits, or none of it does
(no partial state).  Implementation : build the new MetaSwarm in a
temp variable, swap atomically inside the write lock, drop the old
one outside.

### 7.3 SLO `SLO-A1-META-RELOAD-P99`

```yaml
- id: SLO-A1-META-RELOAD-P99
  description: "p99 wall time for SIGHUP-triggered ReloadConfig on a 7-participant manifest"
  target: 200       # ms
  operator: "<="
  comparator: lte
  threshold_ms: 200
  severity: major
  owner: meta-swarm-team
  deferred_ticket: DT-A1-PHASE2-SOFT-LAUNCH-3-RUNS
```

Phase-2 PR adds this row to `bench_baselines/v0.11.0/meta_swarm_slos.yaml`
(new minor version of the baseline) + ports it to the master rollup
`SLO-OPS-A1-META-SWARM.inherits`.

---

## 8. Helm sub-chart `efe-meta-swarm`

### 8.1 Directory structure

```
charts/efe/                              # existing umbrella
└── charts/                              # sub-charts
    ├── efe-server/                      # existing
    └── efe-meta-swarm/                  # NEW (Phase 2)
        ├── Chart.yaml
        ├── values.yaml
        ├── values.schema.json
        ├── templates/
        │   ├── _helpers.tpl
        │   ├── controller-statefulset.yaml
        │   ├── controller-service.yaml
        │   ├── controller-pdb.yaml
        │   ├── controller-servicemonitor.yaml
        │   ├── participant-deployment.yaml      (template per participant)
        │   ├── participant-service.yaml
        │   ├── participant-servicemonitor.yaml
        │   ├── secret-tls.yaml
        │   ├── configmap-manifest.yaml
        │   └── tests/
        │       └── connection-test.yaml         (`helm test`)
        └── README.md
```

### 8.2 `values.yaml` (default-disabled)

```yaml
# === values.yaml ===
metaSwarm:
  # FLAGSHIP DEFAULT : false. Production parity stays at Phase 1.
  enabled: false

  # Container image — shared by controller + participants.
  image:
    repository: efe/meta-swarm-server
    tag: "0.2.0"          # Phase-2 GA
    pullPolicy: IfNotPresent

  # === Controller ===
  controller:
    replicaCount: 3
    resources:
      requests:
        cpu: 500m
        memory: 512Mi
      limits:
        cpu: 2000m
        memory: 2Gi
    podDisruptionBudget:
      minAvailable: 2
    service:
      type: ClusterIP
      port: 50080
    metrics:
      port: 9090
      enabled: true

  # === Participants ===
  participants:
    - id: supplier-scoring
      dim: 384
      kappa: 0.6
      seed: 11
      replicas: 1
      resources:
        requests: { cpu: 250m, memory: 256Mi }
        limits:   { cpu: 1000m, memory: 1Gi }
    - id: contract-risk
      dim: 512
      kappa: 0.7
      seed: 22
      replicas: 1
      resources:
        requests: { cpu: 250m, memory: 256Mi }
        limits:   { cpu: 1000m, memory: 1Gi }
    - id: spend-categorization
      dim: 384
      kappa: 0.5
      seed: 33
      replicas: 1
      resources:
        requests: { cpu: 250m, memory: 256Mi }
        limits:   { cpu: 1000m, memory: 1Gi }

  # === Meta-space ===
  metaSpace:
    dim: 256
    safety:
      kind: box
      radius: 10.0

  # === Iteration knobs ===
  iteration:
    maxIters: 100
    tolerance: 1.0e-4
    alphaDefault: 0.5
    couplingGamma: 0.2
    deterministic: true

  # === TLS ===
  tls:
    enabled: true
    certSecretName: efe-meta-swarm-tls   # opaque secret w/ tls.crt, tls.key, ca.crt

  # === Observability ===
  observability:
    serviceMonitor:
      enabled: true                # Prometheus Operator integration
      interval: 30s
    otel:
      enabled: true
      endpoint: http://otel-collector.observability.svc:4317
      traceSampleRatio: 0.05

  # === Soft-launch (Phase 2 first 3 runs in prod) ===
  softLaunch:
    enabled: true
    expiresAt: "2026-09-15"      # auto-flips to false after this date
```

### 8.3 Default-disabled enforcement

The umbrella `charts/efe/values.yaml` does **not** override
`metaSwarm.enabled`.  Thus :

```bash
helm install efe ./charts/efe   # metaSwarm.enabled = false ⇒ no MS resources
helm install efe ./charts/efe --set metaSwarm.enabled=true  # opt-in
```

A `helm template` of the chart with default values produces a
manifest set strictly identical to Phase 1 deployments — verified by
the new CI job `helm-template-diff-phase1-vs-phase2-defaults` (§ 19.3).

### 8.4 `helm test` smoke

A pod `efe-meta-swarm-connection-test` is shipped that, when
`metaSwarm.enabled=true`, calls `MetaSwarmService::Health` and
`MetaSwarmService::Info` against the controller Service and asserts
both return `STATUS_SERVING` within 10 s.

---

## 9. Observability : Prometheus + OpenTelemetry

### 9.1 Prometheus metrics (14)

| # | Metric name | Type | Labels | Description |
|---|---|---|---|---|
| 1 | `meta_swarm_iterate_total` | counter | `outcome={converged,max_iters,safety_fail,participant_down,deadline,reload}` | Number of `Iterate` RPCs by outcome. |
| 2 | `meta_swarm_iterate_duration_seconds` | histogram | (none) | Wall time of `Iterate` ; buckets `[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 30]`. |
| 3 | `meta_swarm_iterate_iterations` | histogram | (none) | Inner-step count per `Iterate` ; buckets `[1, 5, 10, 20, 30, 50, 80, 120, 200]`. |
| 4 | `meta_swarm_iterate_final_residual` | summary | (none) | Final residual at convergence (median + p99). |
| 5 | `meta_swarm_participant_step_total` | counter | `participant_id`, `outcome={ok,fail}` | Step RPCs per participant. |
| 6 | `meta_swarm_participant_step_duration_seconds` | histogram | `participant_id` | Per-participant Step latency. |
| 7 | `meta_swarm_participant_up` | gauge | `participant_id` | 1 = controller can reach participant, 0 = DOWN. |
| 8 | `meta_swarm_inflight` | gauge | (none) | Current concurrent `Iterate` calls. |
| 9 | `meta_swarm_reload_total` | counter | `outcome={ok,fail,rejected}` | SIGHUP reloads. |
| 10 | `meta_swarm_reload_duration_seconds` | histogram | (none) | Reload wall time. |
| 11 | `meta_swarm_reload_gen` | gauge | (none) | Current reload generation. |
| 12 | `meta_swarm_safety_violation_total` | counter | `level={warn,error}`, `axis` | Safety projections that hit the boundary. |
| 13 | `meta_swarm_build_info` | gauge | `git_sha`, `proto_version`, `rustc_version` | Always 1 ; reports the build fingerprint. |
| 14 | `meta_swarm_consensus_norm` | summary | (none) | Norm of the meta-consensus tensor at convergence. |

All metrics are scraped via the existing ServiceMonitor pattern.
A single Grafana dashboard `efe-meta-swarm.json` (under
`grafana/dashboards/`) renders them.

### 9.2 OpenTelemetry spans (8 types)

| # | Span name | Attributes | When emitted |
|---|---|---|---|
| 1 | `meta_swarm.iterate` | `iterations`, `final_residual`, `stop_reason`, `participant_count` | One per `Iterate` RPC ; root of the trace for that request. |
| 2 | `meta_swarm.step` | `step_index`, `residual_after_step` | One per inner step inside `iterate`. |
| 3 | `meta_swarm.participant_step` | `participant_id`, `step_us`, `local_residual` | One per participant per step (child of `meta_swarm.step`). |
| 4 | `meta_swarm.lift` | `participant_id`, `pre_dim`, `meta_dim` | Per-step lift call. |
| 5 | `meta_swarm.pull` | `participant_id`, `pre_dim`, `meta_dim` | Per-step pull call. |
| 6 | `meta_swarm.meta_op` | `dim`, `kind` | Per-step meta-operator application. |
| 7 | `meta_swarm.safety_project` | `level`, `distance` | Only when projection is non-trivial (axis hit). |
| 8 | `meta_swarm.reload` | `reload_gen`, `applied_changes_count` | One per SIGHUP. |

Trace context : controller propagates `traceparent` to participants via
the gRPC metadata.  Sampling : 5 % default (`traceSampleRatio`),
configurable.

### 9.3 Dashboards (2)

**Dashboard 1 — Operational** (`grafana/dashboards/efe-meta-swarm-ops.json`) :

- Panel: `iterate_total` rate by outcome (stacked area)
- Panel: `iterate_duration_seconds` p50 / p95 / p99 (time series)
- Panel: `inflight` (gauge)
- Panel: `participant_up` per participant (table, red/green)
- Panel: `safety_violation_total` rate (heatmap by axis)
- Panel: `reload_total` count + last reload time

**Dashboard 2 — Algorithmic** (`grafana/dashboards/efe-meta-swarm-algo.json`) :

- Panel: `iterate_iterations` histogram + budget line (60)
- Panel: `iterate_final_residual` median + p99 (time series)
- Panel: `consensus_norm` median + p99
- Panel: `participant_step_duration_seconds` per participant
- Panel: residual-trajectory chart (uses the `IterateStream` data from
  a sampled call every 1 min)

### 9.4 Alerting rules (4 PrometheusRule)

| Rule | Expression | Severity |
|---|---|---|
| `MetaSwarmHighFailureRate` | `sum(rate(meta_swarm_iterate_total{outcome!="converged"}[5m])) / sum(rate(meta_swarm_iterate_total[5m])) > 0.02` | critical |
| `MetaSwarmParticipantDown` | `meta_swarm_participant_up == 0 for 1m` | critical |
| `MetaSwarmReloadFail` | `increase(meta_swarm_reload_total{outcome="fail"}[10m]) > 0` | major |
| `MetaSwarmLatencyP99` | `histogram_quantile(0.99, sum by (le)(rate(meta_swarm_iterate_duration_seconds_bucket[5m]))) > 0.1` | major |

All routed via existing Alertmanager → PagerDuty integration.

---

## 10. Internal 7-day soak

### 10.1 Setup

- Environment : `efe-staging` cluster, 1 controller StatefulSet (3
  replicas) + 3 participant Deployments.
- Traffic : an `efe-meta-swarm-loadgen` Job (new, in `tools/loadgen/`)
  issues `Iterate` RPCs at 30 req/s sustained, 100 req/s burst.
- Workload mix :
  - 60 % standard 3-swarm Kovant scenario (default manifest)
  - 30 % varied initial states (random Tensor::pseudo_random seeds)
  - 10 % SIGHUP-driven manifest swaps (every 30 min)
- Duration : 168 h (7 days) starting Monday 00:00 UTC.
- Observers : EFE-team only ; no external traffic ; no customer-facing
  endpoint.

### 10.2 Pass criteria (must all be true)

| # | Criterion | Threshold |
|---|---|---|
| S1 | Zero Sev1 incidents | 0 |
| S2 | Zero Sev2 incidents | 0 |
| S3 | ≤ 1 Sev3 incident | ≤ 1 |
| S4 | Iterate convergence rate | ≥ 99.95 % |
| S5 | Master gate stays GREEN every nightly run | 7/7 nightly PASS |
| S6 | p99 latency | ≤ 100 ms throughout |
| S7 | Memory leak (RSS growth) | ≤ 5 % over 7 days |
| S8 | SIGHUP success rate | ≥ 99.9 % |
| S9 | Determinism preserved (sampled every 1 h) | 100 % |
| S10 | OTel trace coverage | ≥ 5 % of Iterate calls have a complete trace |

A single failure of any criterion **resets the 7-day clock**.  This is
intentionally strict — the L2 gate is the L1 of customer trust.

### 10.3 Soak report

At day 7, the soak driver emits `docs/A1_PHASE2_SOAK_REPORT.md` with :

- Daily summary (convergence rate, p99, incidents)
- Reload event log (one row per SIGHUP)
- Determinism sample log
- Memory time series
- Open follow-up items (deferred to Phase 3 if needed)

---

## 11. Public bench : meta-swarm vs mono-swarm

### 11.1 Methodology

Five workloads, run on identical hardware (4-vCPU / 16 GB Ubuntu
22.04 GCP n2-standard-4) :

| # | Workload | Description |
|---|---|---|
| W1 | Kovant-3 (Phase-1 scenario) | supplier-scoring(384) + contract-risk(512) + spend-categorization(384) → meta(256). |
| W2 | Kovant-3 with 2× larger dims | 768 + 1024 + 768 → 512.  Stress the bandwidth side. |
| W3 | 7-participant scenario | 7 swarms of dim 256, meta 128. Tests scaling to more participants. |
| W4 | Single-swarm baseline | The legacy mono-swarm `efe-server` path : no meta-coordination. Both meta-swarm and mono-swarm produce the same `accuracy_top1` score on a held-out golden set ; we compare latency. |
| W5 | Slow-participant scenario | One participant has artificial 50 ms latency injected per step. Measures meta-swarm graceful degradation. |

For each workload, three measurement modes :

- **Mono-swarm** (`efe-server` only, Phase-1 path).
- **Meta-swarm in-process** (`efe-meta-swarm` linked, no gRPC).
- **Meta-swarm gRPC** (Phase-2 multi-process T2 topology).

### 11.2 Metrics published

| Metric | Mono-swarm | MS in-proc | MS gRPC | Unit |
|---|---:|---:|---:|---|
| Convergence iterations | N/A | 47 | 47 | count |
| Final residual | N/A | 9.7e-5 | 9.7e-5 | scalar |
| Wall p50 | TBD | TBD | TBD | ms |
| Wall p99 | TBD | TBD | TBD | ms |
| Throughput (req/s, 4 vCPU) | TBD | TBD | TBD | req/s |
| Memory peak | TBD | TBD | TBD | MB |
| Accuracy top-1 (on golden set) | TBD | TBD | TBD | % |

The values are **measured live during the run**, not estimated.  The CSV
+ HTML are published under `bench_results/meta-swarm-vs-mono-swarm-2026-08.{csv,html}`
and linked from the L2 graduation issue.

### 11.3 Publication format

- CSV : the matrix above, machine-readable.
- HTML : an interactive page with toggles, residual-trajectory plots,
  hardware fingerprint, reproducibility recipe (`bash bench_runner.sh
  --public --workload W1`).
- All raw criterion JSON included in `bench_results/raw/`.

### 11.4 Honest framing

The bench description prominently states :

> Meta-swarm is **not** a strict replacement for mono-swarm.  It adds
> coordination guarantees (deterministic consensus, contractive fixed
> point, audit-able safety projection) that mono-swarm cannot provide.
> The latency overhead reflects this added work.  We publish all data
> so customers can decide based on their requirements.

No cherry-picking ; no marketing.  All five workloads are reported
even if mono-swarm wins.

---

## 12. Gate 2 → 3 (L2 attained) criteria

L2 is attained iff **all** of the following are true on
2026-08-31 :

| # | Criterion | Owner | Evidence |
|---|---|---|---|
| G1 | The 7 Phase-1 surface A1 SLOs are GREEN on the master gate, 7/7 of the last 7 nightly runs. | meta-swarm-team | `target/ops/master_report.md` archives. |
| G2 | The 4 Phase-2 surface SLOs (§ 14) are GREEN on the master gate, 7/7 of the last 7 nightly runs. | meta-swarm-team | id. |
| G3 | Master rollup `SLO-OPS-A1-META-SWARM` (#19) is PASS on the last 7 nightly runs. | meta-swarm-team | id. |
| G4 | Master rollup `SLO-OPS-A1-META-GRPC` (#20, added Phase 2) is PASS on the last 7 nightly runs. | meta-swarm-team | id. |
| G5 | 7-day internal soak passes all 10 criteria (§ 10.2). | sre-team | `A1_PHASE2_SOAK_REPORT.md`. |
| G6 | Public bench published with all 5 workloads × 3 modes. | meta-swarm-team | `bench_results/meta-swarm-vs-mono-swarm-2026-08.{csv,html}`. |
| G7 | Helm sub-chart Lint + `helm template` regression test pass. | platform-team | CI job `helm-meta-swarm-lint`. |
| G8 | Security review : threat model document approved by `sec-team`. | sec-team | `docs/security/PHASE2_THREAT_MODEL.md` signed off. |
| G9 | Runbook `docs/runbooks/META_SWARM_GRPC.md` written + 2 SRE rehearsals (1 SIGHUP drill, 1 participant-down drill). | sre-team | Runbook + rehearsal logs in `docs/runbooks/drills/`. |
| G10 | Postmortem template `docs/postmortems/_template.md` extended with a Meta-Swarm section. | sre-team | Template diff. |

If any criterion is RED, L2 graduation is **deferred** by one sprint
(2 weeks) and re-evaluated.  No "soft pass" — Φ-D.A1 doctrine
applies.

---

## 13. Code structure additions

### 13.1 New crates

```
crates/
├── efe-meta-swarm/                   # Phase-1 (existing, unchanged)
├── efe-meta-swarm-proto/             # NEW — proto + tonic-generated client/server stubs
│   ├── Cargo.toml
│   ├── build.rs
│   ├── proto/
│   │   └── efe/v1/meta_swarm/
│   │       ├── service_meta_swarm.proto
│   │       ├── service_meta_participant.proto
│   │       ├── messages.proto
│   │       └── errors.proto
│   └── src/lib.rs                    # re-exports tonic-generated code
│
└── efe-meta-swarm-server/            # NEW — binary + library
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs
    │   ├── main.rs                   # entry-point — choose controller vs participant role via CLI
    │   ├── bin/
    │   │   ├── controller.rs
    │   │   └── participant.rs
    │   ├── controller/
    │   │   ├── mod.rs
    │   │   ├── service.rs            # impl MetaSwarmService
    │   │   ├── manifest.rs           # YAML manifest reader
    │   │   ├── discovery.rs          # trait + impls (static, k8s-dns)
    │   │   └── reload.rs             # SIGHUP handler
    │   ├── participant/
    │   │   ├── mod.rs
    │   │   ├── service.rs            # impl MetaParticipantService
    │   │   └── adapter.rs            # wraps Phase-1 MockParticipant / LocalParticipant
    │   ├── client/
    │   │   ├── mod.rs
    │   │   └── remote_meta_swarm.rs  # RemoteMetaSwarm — gRPC client adapter
    │   ├── observability/
    │   │   ├── metrics.rs            # Prometheus registry
    │   │   ├── tracing.rs            # OTel setup
    │   │   └── health.rs
    │   ├── auth/
    │   │   ├── mtls.rs
    │   │   └── bearer.rs
    │   └── error.rs
    ├── tests/
    │   ├── integration_grpc_multiprocess.rs
    │   ├── integration_sighup_hot_reload.rs
    │   ├── integration_failure_modes.rs
    │   └── integration_obs_phase2.rs
    └── benches/
        └── grpc_round_trip.rs        # roundtrip latency bench
```

### 13.2 Workspace `Cargo.toml`

Add to `[workspace.members]`:

```toml
"crates/efe-meta-swarm-proto",
"crates/efe-meta-swarm-server",
```

Add to `[workspace.dependencies]`:

```toml
# Phase-2 additions (only used by efe-meta-swarm-server*)
tonic            = { version = "0.12", features = ["tls", "gzip"] }
tonic-build      = "0.12"
prost            = "0.13"
prost-types      = "0.13"
opentelemetry    = { version = "0.24", features = ["metrics", "trace"] }
opentelemetry-otlp = "0.17"
prometheus       = "0.13"
signal-hook      = "0.3"
signal-hook-tokio = "0.3"
```

`efe-meta-swarm-server/Cargo.toml` overrides `panic = "unwind"`
(see C7 in § 2).

### 13.3 Modifications to existing files

| File | Change |
|---|---|
| `Cargo.toml` (workspace) | +2 members, +9 deps. |
| `crates/efe-server/Cargo.toml` | +1 optional dep `efe-meta-swarm-proto` behind feature `meta-swarm-grpc`. |
| `crates/efe-server/src/bridge/mod.rs` | New module `meta_swarm.rs` that dispatches `iterate()` to the gRPC client when `EFE_META_SWARM_ENABLED=true`. |
| `bench_baselines/v0.10.0/meta_swarm_slos.yaml` | Either bumped to `v0.11.0` (recommended) with +4 new Phase-2 SLOs, or kept as-is and supplemented by `v0.11.0/meta_swarm_grpc_slos.yaml`. |
| `bench_baselines/v0.9.0/ops_master_slos.yaml` | +1 master rollup `SLO-OPS-A1-META-GRPC`. |
| `.github/workflows/bench.yml` | +1 job `bench-meta-swarm-grpc` (Phase-2 specific). |
| `.github/workflows/ops-master-gate.yml` | +1 step "A2 meta-swarm gRPC master gate". |
| `scripts/ops/run_suite.sh` | +1 case `a2_meta_swarm_grpc`. |

No file currently in the project is **deleted** or has a signature
changed.

---

## 14. SLO budget additions (Phase 2)

Four new surface SLOs, frozen at `bench_baselines/v0.11.0/meta_swarm_grpc_slos.yaml` :

| SLO id | Type | Comparator | Threshold | Source |
|---|---|---:|---:|---|
| `SLO-A2-GRPC-OVERHEAD-P99` | latency | `<=` | 30 ms | gRPC round-trip overhead = Phase-2 p99 − Phase-1 p99 |
| `SLO-A2-GRPC-MULTIPROC-DETERMINISM` | algorithmic | `==` | 1 | 3 reruns across 3 processes produce bit-identical residual.to_bits |
| `SLO-A2-RELOAD-P99` | operational | `<=` | 200 ms | SIGHUP-triggered ReloadConfig wall time |
| `SLO-A2-TRACE-COVERAGE` | observability | `>=` | 0.05 | Fraction of `Iterate` RPCs with a complete OTel trace |

New master rollup #20 :

```yaml
- id:          SLO-OPS-A1-META-GRPC
  name:        Sprint A1 Phase 2 gRPC transport gate
  description: |
    Rolls up the 4 `SLO-A2-*` surface SLOs produced by the
    `bench-meta-swarm-grpc` job and the `a2_meta_swarm_grpc` suite
    runner.  Activated in PR #5 (Sprint A1 P2, week N).
  metric:      efe_meta_swarm_a2_gate_status
  target:      1
  operator:    "=="
  severity:    major
  status:      ACTIVE
  inherits:
    - SLO-A2-GRPC-OVERHEAD-P99
    - SLO-A2-GRPC-MULTIPROC-DETERMINISM
    - SLO-A2-RELOAD-P99
    - SLO-A2-TRACE-COVERAGE
  blast_radius: A1-PHASE2-OPT-IN-TENANTS
```

Smoke test 9 added to `tests/bench_smoke.rs` to pin its existence
(same pattern as P5 audit fix from Phase 1).

---

## 15. Security model

### 15.1 Trust boundaries

```
         ▲ External (untrusted)
         │
  ┌──────┴──────────┐
  │ ingress / LB    │
  └──────┬──────────┘
         │ TLS 1.3
  ┌──────▼──────────┐
  │ efe-server pod  │ ← public auth (existing : OAuth2 / JWT)
  └──────┬──────────┘
         │ mTLS (NEW Phase 2)   ← internal trust boundary
  ┌──────▼──────────────┐
  │ meta-swarm-controller│
  └──────┬──────────────┘
         │ mTLS
  ┌──────▼──────────┐
  │ participant pods │
  └─────────────────┘
```

- **mTLS mandatory** between efe-server ↔ controller and controller
  ↔ participant.  No plaintext gRPC in any environment.
- Certificates : issued by the existing internal CA (cert-manager),
  rotated every 30 days, leaf cert validity 90 days.
- Bearer token (`EFE_META_SWARM_AUTH_TOKEN`) is a **transitional** auth
  method — used only when mTLS bootstrap fails, removed in Phase 3.

### 15.2 Authorization

- The controller authorises callers by the SAN field of their leaf
  cert :  `efe-server.efe.svc.cluster.local` → MetaSwarmService
  full access ; anything else → DENY.
- Participants authorise callers by SAN : only the controller may call.
- Operator RPC `ReloadConfig` requires an additional `admin` scope on
  the auth token.

### 15.3 Threat model summary

| Threat | Mitigation |
|---|---|
| Malicious participant returns crafted state to bias consensus | Safety projection (Phase 1) ; the meta-operator is contractive — bias is bounded by `(1-κ_max) × max_step_size`.  Phase-3 will add cryptographic attestation of step outputs. |
| Replay of `Iterate` response | gRPC's per-call deadlines + the `Iterate` is idempotent w.r.t. determinism. |
| MITM on controller ↔ participant | mTLS. |
| SIGHUP injection by unauthorized operator | SIGHUP requires admin scope + the kernel enforces PID-level permission. K8s RBAC restricts pod exec. |
| Manifest tampering (ConfigMap) | K8s RBAC + sealed-secrets for the participant seeds. |

Full threat model lives in `docs/security/PHASE2_THREAT_MODEL.md`
(deliverable G8).

---

## 16. Migration path Phase 1 → Phase 2

### 16.1 Rollout strategy

Phased, gated, reversible :

1. **Week 1 of Aug 2026** : staging deployment with
   `metaSwarm.enabled=false`.  No behavioural change vs Phase 1.
2. **Week 2** : staging `--set metaSwarm.enabled=true`.  Internal
   shadow traffic only.  Compare outputs to in-process Phase 1
   on the same requests.
3. **Week 3** : production canary on 10 % of replicas with the
   flag on ; remaining 90 % stay Phase 1.
4. **Week 4** : if canary clean for 7 days → 50 % roll-out.  Then
   100 % the following week if still clean.

### 16.2 Rollback

Single env-var flip :

```bash
kubectl set env deployment/efe-server EFE_META_SWARM_ENABLED=false
```

The `efe-server` falls back to the in-process Phase-1 path
immediately.  Meta-swarm pods can be left running idle (no traffic)
or scaled to zero.  No data corruption is possible because Phase 2
is a transport-layer change only — the algorithmic surface is
identical.

### 16.3 Compatibility matrix

| Client | Server | Outcome |
|---|---|---|
| efe-server Phase 1 | (no MS) | In-process, Phase-1 unchanged. |
| efe-server Phase 2 with flag OFF | (no MS) | In-process, Phase-1 path. |
| efe-server Phase 2 with flag ON | MS controller v0.2.0 | gRPC, Phase-2 happy path. |
| efe-server Phase 2 with flag ON | MS controller v0.1.x | DENIED at `Info` RPC (proto_version mismatch). |
| efe-server Phase 1 | MS controller v0.2.0 | Never happens — the flag is gated by Phase-2 efe-server. |

---

## 17. Risk matrix + mitigations

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | gRPC overhead pushes p99 over the 100 ms budget | M | H | Streaming `StepStream` (§ 4.2) saves a round-trip per step ; if still too slow, bump budget to 150 ms with explicit doc note. |
| R2 | SIGHUP race condition corrupts state | L | H | Atomic swap inside write-lock (§ 7.2) ; integration test `tests/integration_sighup_hot_reload.rs` runs 1000 random-timed SIGHUPs concurrently with Iterate. |
| R3 | Cross-process determinism breaks (different rustc, BLAS, etc.) | M | H | Pin rustc 1.88 in Cargo + container ; `blas_backend: "none"` for Phase 2 ; determinism check in soak (S9). |
| R4 | Helm sub-chart breaks existing efe-server deployments | L | H | `enabled=false` default + CI job `helm-template-diff-phase1-vs-phase2-defaults` verifies byte-identical output. |
| R5 | mTLS cert rotation fails in production | M | M | cert-manager auto-renewal at 60 days ; alert `MetaSwarmCertExpiringSoon` fires at 7 days. |
| R6 | OTel cardinality explodes Grafana | L | M | Bounded labels (no free-form `participant_id` — fixed set of 3) ; OTel sampling 5 %. |
| R7 | One participant becomes the bottleneck (stragglers) | M | M | W5 bench measures this ; if real, Phase 3 will introduce per-participant SLO + auto-scale. |
| R8 | Public bench shows mono-swarm wins on W4 | M | L | Acceptable — bench is honest, decision is per-customer. Marketing materials state this clearly. |
| R9 | L2 gate fails 2026-08-31 | L | H | Each criterion is independently met or unmet ; failing G5 only triggers a 2-week deferral, not a Phase-2 cancellation. |

---

## 18. Detailed timeline

### Sprint A1.P2.1 — 2026-07-06 → 2026-07-19 (S1-S2 July)

**Goal** : land the protos + the bare-bones servers.

Week 1 (S1) :
- [ ] PR #5  scaffold `efe-meta-swarm-proto` (protos + tonic-build) [meta-swarm-team]
- [ ] PR #6  scaffold `efe-meta-swarm-server` (controller + participant binaries, no logic yet) [meta-swarm-team]
- [ ] PR #7  threat model doc draft [sec-team]

Week 2 (S2) :
- [ ] PR #8  controller-side `MetaSwarmService::Iterate` wiring through Phase-1 `MetaSwarm` [meta-swarm-team]
- [ ] PR #9  participant-side `MetaParticipantService::Step` wiring [meta-swarm-team]
- [ ] PR #10 integration test `tests/integration_grpc_multiprocess.rs` [meta-swarm-team]

End-of-sprint demo : `cargo run --bin controller` + 3× `cargo run --bin participant` on dev box, Iterate succeeds with bit-identical Phase-1 result.

### Sprint A1.P2.2 — 2026-07-20 → 2026-08-02 (S3-S4 July)

**Goal** : env-var + SIGHUP + Helm scaffold.

Week 3 (S3) :
- [ ] PR #11 `EFE_META_SWARM_ENABLED` plumbing in efe-server [meta-swarm-team]
- [ ] PR #12 SIGHUP hot-reload + `ReloadConfig` RPC [meta-swarm-team]
- [ ] PR #13 Helm sub-chart skeleton `charts/efe/charts/efe-meta-swarm/` [platform-team]

Week 4 (S4) :
- [ ] PR #14 Helm sub-chart values + templates (controller + participants) [platform-team]
- [ ] PR #15 `helm template` regression test (default-disabled produces same manifest as Phase 1) [platform-team]
- [ ] PR #16 mTLS wiring + cert-manager Issuer/Certificate [sec-team + platform-team]

End-of-sprint demo : `helm install` on staging with flag flipped on/off ; SIGHUP rolls participant in/out.

### Sprint A1.P2.3 — 2026-08-03 → 2026-08-16 (S1-S2 August)

**Goal** : observability + start of soak.

Week 5 (S1) :
- [ ] PR #17 Prometheus metrics (14) [meta-swarm-team]
- [ ] PR #18 OpenTelemetry spans (8) [meta-swarm-team]
- [ ] PR #19 Grafana dashboards + alert rules [sre-team]

Week 6 (S2) :
- [ ] PR #20 `tools/loadgen` traffic generator [meta-swarm-team]
- [ ] Begin 7-day soak (Mon 2026-08-10 00:00 UTC) [sre-team]
- [ ] PR #21 Public bench harness + W1-W5 workloads [meta-swarm-team]

End-of-sprint demo : Grafana dashboards live ; soak day 7 report posted.

### Sprint A1.P2.4 — 2026-08-17 → 2026-08-30 (S3-S4 August)

**Goal** : L2 graduation.

Week 7 (S3) :
- [ ] PR #22 4 new SLOs + master rollup `SLO-OPS-A1-META-GRPC` [meta-swarm-team]
- [ ] PR #23 Smoke test 9 (rollup #20 pin) [meta-swarm-team]
- [ ] PR #24 Public bench publication (CSV + HTML) [meta-swarm-team]
- [ ] Runbook + 2 SRE rehearsals [sre-team]

Week 8 (S4) :
- [ ] L2 graduation issue : verify G1-G10 (§ 12) [meta-swarm-team + sre-team]
- [ ] Final master gate run + sign-off [sre-team]
- [ ] PR #25 Phase 2 end-of-sprint progress report [meta-swarm-team]
- [ ] **Gate 2 → 3 ✅** (target : 2026-08-31)

---

## 19. CI / CD pipeline additions

### 19.1 New `bench.yml` job

```yaml
bench-meta-swarm-grpc:
  name: Run grpc_round_trip + multi-proc integration (Sprint A1 P2)
  needs: bench-build
  runs-on: ubuntu-latest
  timeout-minutes: 25
  continue-on-error: true   # ← flip after 3 main runs
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@1.88
      with: { toolchain: "1.88" }
    - uses: Swatinem/rust-cache@v2
      with: { shared-key: ${{ env.RUST_CACHE_KEY }} }
    - name: Run bench
      run: |
        MODE="${{ github.event.inputs.mode || 'quick' }}" \
        BENCHES=grpc_round_trip \
        BENCH_TIMEOUT=900 \
        bash scripts/bench_runner.sh
    - name: Run multi-proc integration test
      run: |
        cargo test -p efe-meta-swarm-server \
          --test integration_grpc_multiprocess \
          --release
    - name: Upload partial artefact
      if: always()
      uses: actions/upload-artifact@v4
      with:
        name: bench-partial-meta-swarm-grpc-${{ github.sha }}
        path: |
          target/criterion/**/new/estimates.json
          target/criterion/grpc_round_trip/meta-swarm-grpc-extras.json
          target/bench-grpc-round-trip.txt
        retention-days: 30
```

### 19.2 New `ops-master-gate.yml` step

```yaml
- name: A2 meta-swarm gRPC master gate (Sprint A1 P2 PR #18)
  continue-on-error: true
  run: bash scripts/ops/run_suite.sh a2_meta_swarm_grpc
```

### 19.3 New `helm-meta-swarm-lint.yml` workflow

```yaml
name: helm-meta-swarm-lint
on:
  pull_request:
    paths: ['charts/efe/charts/efe-meta-swarm/**']
  push:
    branches: [main]
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
      - name: helm lint
        run: helm lint charts/efe/charts/efe-meta-swarm
      - name: helm template (default = disabled)
        run: |
          helm template ./charts/efe > /tmp/manifest-default.yaml
          # Compare against Phase-1 golden manifest.
          diff -u charts/efe/golden-phase1-default.yaml /tmp/manifest-default.yaml
      - name: helm template (enabled)
        run: |
          helm template ./charts/efe --set metaSwarm.enabled=true > /tmp/manifest-enabled.yaml
          # Diff snapshot (sandbox tracked).
          diff -u charts/efe/golden-phase2-enabled.yaml /tmp/manifest-enabled.yaml
```

### 19.4 Existing `bench-aggregate` updates

The `needs:` list gets `bench-meta-swarm-grpc` appended.  No other
modification — the new SLO keys (`SLO-A2-*`) are unique and consumed
only by `run_suite.sh a2_meta_swarm_grpc`.

---

## 20. Documentation deliverables

| # | Doc | Owner | Deadline |
|---|---|---|---|
| D1 | `docs/EFE_A1_PHASE2_ARCHITECTURE.md` (this document) | meta-swarm-team | 2026-07-01 |
| D2 | `crates/efe-meta-swarm-server/README.md` | meta-swarm-team | end of Sprint A1.P2.1 |
| D3 | `charts/efe/charts/efe-meta-swarm/README.md` | platform-team | end of Sprint A1.P2.2 |
| D4 | `docs/runbooks/META_SWARM_GRPC.md` | sre-team | mid-Sprint A1.P2.4 |
| D5 | `docs/security/PHASE2_THREAT_MODEL.md` | sec-team | end of Sprint A1.P2.1 |
| D6 | `docs/A1_PHASE2_SOAK_REPORT.md` | sre-team | day 7 of soak |
| D7 | Update `docs/runbooks/ops_master_gate_fail.md` § 8 for A2 SLOs | sre-team | Sprint A1.P2.4 S3 |
| D8 | `docs/EFE_A1_PHASE2_LESSONS.md` (post-mortem of Phase 2 ; even if all PASS) | meta-swarm-team | 2026-09-15 |
| D9 | Public bench HTML + CSV + reproducibility recipe | meta-swarm-team | Sprint A1.P2.4 S3 |
| D10 | `crates/efe-meta-swarm-proto/README.md` with versioning policy | meta-swarm-team | end of Sprint A1.P2.1 |

---

## 21. Open questions for Phase 2 kickoff

| # | Question | Decision deadline |
|---|---|---|
| Q1 | f32 on the wire vs f64 — do we have a customer that needs f64 in Phase 2 ? | Sprint A1.P2.1 S1 |
| Q2 | Streaming `StepStream` mandatory or optional ? Server complexity vs round-trip savings. | Sprint A1.P2.1 S2 |
| Q3 | Do we ship Phase 2 with TLS-only, or allow plaintext for dev ? Recommendation : TLS-only, with a `--insecure` flag gated by `EFE_DEV_MODE=true`. | Sprint A1.P2.2 S3 |
| Q4 | Helm umbrella chart : do we ship Phase 2 in the existing chart version, or bump major ? Recommendation : minor bump (`0.x.0 → 0.(x+1).0`). | Sprint A1.P2.2 S4 |
| Q5 | Soak start date : 2026-08-10 needs SRE on-call coverage. Confirm with @sre. | Sprint A1.P2.3 S1 |
| Q6 | Public bench : do we sign the HTML report (sigstore) for tamper-evidence ? Phase 3 default. | post-L2 |
| Q7 | mTLS rotation : keep manual for Phase 2 or invest in auto-rotation? Recommendation : manual + 7-day expiry alert. | Sprint A1.P2.2 S4 |
| Q8 | Three controllers HA vs single controller : worth the complexity for Phase 2 ? Recommendation : ship single controller with a K8s lease (active-passive), upgrade to Raft in Phase 4. | Sprint A1.P2.2 S3 |

---

## Appendix A — Cross-references

- Phase 1 architecture : [`EFE_A1_PHASE1_ARCHITECTURE.md`](EFE_A1_PHASE1_ARCHITECTURE.md)
- Phase 1 bench tracking : [`EFE_A1_BENCH_TRACKING.md`](EFE_A1_BENCH_TRACKING.md)
- Operational doctrine : [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md)
- Phase 1 PR #4 description : [`../crates/efe-meta-swarm/PR4_DESCRIPTION.md`](../crates/efe-meta-swarm/PR4_DESCRIPTION.md)
- Master SLO bundle : [`../bench_baselines/v0.9.0/ops_master_slos.yaml`](../bench_baselines/v0.9.0/ops_master_slos.yaml)
- Master gate check : [`../scripts/ops/master_gate_check.py`](../scripts/ops/master_gate_check.py)

## Appendix B — Glossary

- **L1, L2, L3** : maturity levels (Phase 1 = L1 attained ; Phase 2 = L2 ; Phase 3 = L3).
- **Topology T1/T2/T3** : in-process / single-node 3-instances / K8s.
- **Master rollup** : a meta-SLO that aggregates ≥ 2 surface SLOs via the `inherits` field.
- **Soft-launch** : 3-run observation window during which `continue-on-error: true` is set on the new bench job.
- **Soak** : continuous load over a defined duration (here 7 days) with strict no-incident criteria.
- **L2 gate** : the 10-criterion checklist (§ 12) ; passing it graduates the Phase from "preview" to "GA-equivalent for internal users".
