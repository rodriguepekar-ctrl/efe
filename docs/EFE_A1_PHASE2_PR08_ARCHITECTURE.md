# PR #8 — `MetaSwarmService::Iterate` wiring — Architecture ultra-détaillée

> **Document type** : Execution-ready implementation blueprint for PR #8.
> **Scope** : 4th PR of Phase 2 (Sprint A1.P2.1, Week 2, day 1-5).
> **Owner** : meta-swarm-team — assigned lead : M. Okafor (back-up : J. Tan).
> **Reviewers (required)** : 2× meta-swarm-team + 1× sec-team + 1× sre-team.
> **Estimate** : 5 FTE-days (engineering) + 1 FTE-day (review) = 6 FTE-days.
> **Branch** : `A1.P2/08-iterate-wire`.
> **Deadline** : Sun 2026-07-19 23:59 UTC (last PR of Sprint A1.P2.1).
> **Hard dependencies** :
>   - PR #5 (proto crate) merged
>   - PR #6 (server scaffold) merged
> **Parent docs** :
>   - [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 5 (Rust API ↔ gRPC mapping)
>   - [`EFE_A1_PHASE2_ROADMAP.md`](EFE_A1_PHASE2_ROADMAP.md) § 2.2 (PR #8 entry)
>   - [`EFE_A1_PHASE2_PR06_ARCHITECTURE.md`](EFE_A1_PHASE2_PR06_ARCHITECTURE.md) (server scaffold this PR wires)
>   - [`docs/security/PHASE2_THREAT_MODEL.md`](security/PHASE2_THREAT_MODEL.md) (threats T1, T7, T8, T13, T19 mitigated here)

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Goal + non-goals](#1-goal--non-goals)
2. [Pre-flight checks](#2-pre-flight-checks)
3. [Branch + commit plan](#3-branch--commit-plan)
4. [Architectural decisions](#4-architectural-decisions)
5. [The sync ↔ async bridge problem](#5-the-sync--async-bridge-problem)
6. [Wire diagram : RPC → MetaSwarm → participants](#6-wire-diagram)
7. [Crate layout additions](#7-crate-layout-additions)
8. [`client/remote_participant.rs` — full design](#8-clientremote_participantrs--full-design)
9. [`controller/iterate.rs` — orchestration](#9-controllreriteraters--orchestration)
10. [`controller/inflight.rs` — MAX_INFLIGHT enforcement](#10-controllrerinflightrs--max_inflight)
11. [`controller/state.rs` — inner `MetaSwarm` shared state](#11-controllerstaters)
12. [`controller/service.rs` — replace stubs with real bodies](#12-controllerservicers)
13. [`manifest/mod.rs` — extended validation](#13-manifestmodrs--validation)
14. [`Cargo.toml` updates](#14-cargotoml-updates)
15. [Determinism, cancellation, timeout semantics](#15-determinism-cancellation-timeout)
16. [Error mapping : Phase-1 `MetaSwarmError` → gRPC `Status`](#16-error-mapping)
17. [Concurrency model (`MAX_INFLIGHT` + `RwLock`)](#17-concurrency-model)
18. [Threat-model coverage : T1, T7, T8, T13, T19](#18-threat-model-coverage)
19. [Tests strategy](#19-tests-strategy)
20. [Acceptance criteria](#20-acceptance-criteria)
21. [PR description template](#21-pr-description-template)
22. [Risk analysis](#22-risk-analysis)
23. [Reviewer checklist](#23-reviewer-checklist)
24. [Definition of Done](#24-definition-of-done)
25. [Post-merge follow-ups](#25-post-merge-follow-ups)

---

## 0. TL;DR

PR #8 replaces the `Err(Status::unimplemented(...))` stubs in
`controller/service.rs` for `Iterate`, `StepOnce`, and `Reset` with
real bodies that :

1. **Validate** the request against the loaded manifest (dim, kappa,
   determinism).
2. **Acquire** an inflight slot (`MAX_INFLIGHT` semaphore).
3. **Build** a `MetaSwarm` (Phase-1 type) from the manifest + the
   configured participants — but each participant is a `RemoteParticipant`
   (gRPC client wrapper), NOT a `MockParticipant`.
4. **Run** `MetaSwarm::iterate()` (Phase-1 sync algorithm) inside
   `tokio::task::spawn_blocking`, with cancellation propagation.
5. **Convert** the `IterateResult` (Phase-1 type) into the proto
   `IterateResponse`.
6. **Release** the inflight slot + log + emit metrics.

What it adds :

- 1 new module `client/remote_participant.rs` (~ 250 LOC) — a gRPC
  client adapter implementing the Phase-1 `SwarmParticipant` trait.
- 1 new module `controller/iterate.rs` (~ 400 LOC) — the orchestration
  glue between the gRPC handler and the Phase-1 algorithm.
- 1 new module `controller/inflight.rs` (~ 80 LOC) — a `Semaphore`
  wrapper that enforces `MAX_INFLIGHT`.
- Modified `controller/service.rs` (~ 200 LOC, replaces stubs).
- Modified `controller/state.rs` (~ 50 LOC, adds inner `MetaSwarm` Arc).
- Modified `manifest/mod.rs` (~ 100 LOC, extends with validation).
- Modified `Cargo.toml` (+1 dep : `efe-meta-swarm` Phase-1 crate).

What it does NOT add :

- No mTLS — plaintext channels (PR #16 wires mTLS).
- No metrics emission — PR #17 wires the 14 metrics.
- No OTel traces — PR #18 wires the 8 span types.
- No `ReloadConfig` — PR #12 wires SIGHUP + manifest reload.
- No `IterateStream` — PR #21 wires server-streaming for bench.
- No multi-process integration test — PR #10 owns that.

Critical-path significance : PR #8 is the FIRST PR where the server
**actually does something** distributed.  Until PR #8, the server is
a stubbed scaffold.  PR #9 (Step on participant side) lands in
parallel ; together they enable the multi-process integration test
of PR #10.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Wire the controller's `MetaSwarmService::Iterate` (and `StepOnce`,
> `Reset`) to invoke the Phase-1 `MetaSwarm::iterate()` algorithm
> with participants connected over gRPC, preserving the Phase-1
> determinism guarantee, the contractive math, and the safety
> projection, while adding Phase-2 specific controls (manifest dim
> validation, `MAX_INFLIGHT` backpressure, cancellation, 30 s server
> cap on `max_wall_ms`).

### 1.2 Non-goals

| # | Excluded | Lives in | Why |
|---|---|---|---|
| NG1 | `Step` body on participant side | **PR #9** | Symmetric work ; parallel sprint slot |
| NG2 | mTLS handshake | PR #16 | Sec PR ; comes later in sprint 2 |
| NG3 | Prometheus metrics emission | PR #17 | Observability sprint |
| NG4 | OTel span emission | PR #18 | Same |
| NG5 | `ReloadConfig` body | PR #12 | SIGHUP sprint |
| NG6 | `IterateStream` (server streaming) | PR #21 | Bench-only path |
| NG7 | Multi-process integration test | PR #10 | Needs PR #8 + #9 first |
| NG8 | NetworkPolicy / Helm | PRs #13, #14 | Platform sprint |
| NG9 | f64 wire precision opt-in | Phase 3 | OQ1 = f32-only at Phase 2 |
| NG10 | Cryptographic attestation of step outputs | Phase 3 | `DT-A2-ATTESTATION` |

### 1.3 Single binary unit of merge-readiness

PR #8 is mergeable iff **all** :

1. `Iterate(IterateRequest)` runs end-to-end in a unit test using a
   `MockRemoteParticipant` (in-process fake gRPC).
2. The returned `IterateResponse` has bit-identical
   `(iterations, final_residual)` to the Phase-1 example
   `examples/three_swarms_kovant.rs`.
3. `StepOnce` and `Reset` are wired (same pattern).
4. Cancellation (client drop) propagates to participants within
   100 ms.
5. `MAX_INFLIGHT` enforcement : the 11th concurrent call returns
   `RESOURCE_EXHAUSTED`.
6. Dim mismatch in `IterateRequest` returns `FAILED_PRECONDITION`.
7. All Phase 1 + PR #5 + PR #6 tests still pass.

---

## 2. Pre-flight checks

| # | Check | How | Expected |
|---|---|---|---|
| PF1 | PR #5 merged | `gh pr view 5 --json state` | `MERGED` |
| PF2 | PR #6 merged | `gh pr view 6 --json state` | `MERGED` |
| PF3 | PR #6 smoke test passes on main | CI | green |
| PF4 | Phase-1 `MetaSwarm::iterate()` API stable | grep `pub fn iterate` in `crates/efe-meta-swarm/src/meta_swarm.rs` | unchanged since Phase-1 GA |
| PF5 | Phase-1 `examples/three_swarms_kovant.rs` reproducible (47 iters, residual 9.7e-5) | `cargo run --example three_swarms_kovant` | as documented |
| PF6 | OQ1 decided : f32-only on the wire | `docs/A1_PHASE2_DECISIONS.md` | f32 |
| PF7 | OQ2 decided : `StepStream` optional | id. | optional |
| PF8 | Threat model PR #7 ships : we cross-ref T1, T7, T8, T13, T19 | `docs/security/PHASE2_THREAT_MODEL.md` | exists |
| PF9 | tokio-util crate available offline (for `CancellationToken`) | `ls ~/.cargo/registry/cache/*/tokio-util*.crate` | present |

---

## 3. Branch + commit plan

### 3.1 Branch

```
A1.P2/08-iterate-wire
```

### 3.2 Commit-by-commit plan (5 commits, each CI-green)

| # | Commit | Files | LOC ± | Verifies |
|---|---|---|---|---|
| C1 | `chore: add efe-meta-swarm dep + InflightSemaphore module` | `crates/efe-meta-swarm-server/Cargo.toml`, `+ src/controller/inflight.rs` | +100 | dep resolves ; inflight unit test passes |
| C2 | `feat(server): RemoteParticipant client (impls SwarmParticipant)` | `+ src/client/mod.rs`, `+ src/client/remote_participant.rs` | +320 | unit test : mock channel, RemoteParticipant returns expected Tensor |
| C3 | `feat(server): manifest validation + tests` | `src/manifest/mod.rs`, `src/manifest/schema.rs` | +180 | unit tests : valid manifest accepted, invalid rejected (dim/kappa/seed checks) |
| C4 | `feat(server): controller/iterate.rs orchestration + state.rs MetaSwarm Arc` | `+ src/controller/iterate.rs`, `src/controller/state.rs` | +500 | orchestration unit tests with MockRemoteParticipant |
| C5 | `feat(server): wire Iterate, StepOnce, Reset in service.rs + smoke` | `src/controller/service.rs`, `+ tests/integration_iterate_inproc.rs` | +300 | smoke : in-process gRPC call → bit-identical Phase-1 result |

Total : ~ **1 400 LOC** (production + tests).

### 3.3 Drafting strategy

Open as **Draft** after C1.  Push C2-C5 incrementally.  Reviewers
walk through commits in order ; PR moves to "Ready" after C5.

---

## 4. Architectural decisions

Locked in this PR ; downstream PRs (#9, #10, #11, #12) honour them.

| ID | Decision | Rationale | Cost if wrong |
|---|---|---|---|
| **AD1** | Reuse the Phase-1 `SwarmParticipant` trait verbatim ; `RemoteParticipant` implements it. | Zero changes to `efe-meta-swarm` Phase-1 crate ; algorithm is identical sync code paths. | If the trait's sync signature blocks PR #8's perf, we'd need to async-ify the trait (workspace-wide change). |
| **AD2** | `MetaSwarm::iterate()` runs in `tokio::task::spawn_blocking`. | Phase-1 is sync ; tonic handler is async ; this is the standard bridge. | Worker thread budget consumed ; mitigated by `MAX_INFLIGHT`. |
| **AD3** | `RemoteParticipant::step_with_pull` (sync trait method) calls async gRPC via `Handle::current().block_on()`. | The sync trait is called from inside `spawn_blocking` ; we're on a non-async thread, so `block_on` doesn't double-block. | Subtle ; documented thoroughly in `remote_participant.rs`. |
| **AD4** | Cancellation : `tokio_util::sync::CancellationToken` propagated from RPC handler to participant calls. | Standard pattern ; integrates with tonic's `Request::extensions`. | If client drops, participants still finish their current Step — bounded by 30 s server timeout. |
| **AD5** | `MAX_INFLIGHT` enforced via `tokio::sync::Semaphore` (NOT a fixed-size pool). | Semaphore-acquired permit is dropped on iterate return ; backpressure is automatic. | None ; standard pattern. |
| **AD6** | `Arc<RwLock<MetaSwarm>>` for shared state. | SIGHUP (PR #12) needs write access ; in-flight iterates hold read locks. | RwLock contention if SIGHUP frequent ; not expected at Phase-2 rates. |
| **AD7** | Manifest is loaded ONCE at controller startup (PR #6) ; PR #8 reads it but does not re-load. | Reload is PR #12's job. | None. |
| **AD8** | Determinism preserved : no random thread-pool scheduling on participant order ; manifest order is canonical. | Phase-1 SLO-A1-META-DETERMINISM requires bit-identical reruns. | If we ever fan-out participants in parallel (Phase 3), order must be stable. |
| **AD9** | Server cap on `max_wall_ms` = 30 000 (30 s). Client can request lower ; client cannot request higher. | DoS prevention (T7). | Any tenant needing > 30 s must wait for Phase-3 timeout reform. |
| **AD10** | `RemoteParticipant` uses its own internal `tonic::transport::Channel` (not shared globally). | Per-participant connection isolation ; simplifies retries. | More TCP sockets ; mitigated by HTTP/2 multiplexing within each channel. |

---

## 5. The sync ↔ async bridge problem

This PR's hardest design question.

### 5.1 The problem statement

- Phase-1 `MetaSwarm::iterate()` is **synchronous** Rust : it returns
  a `Result<IterateResult, MetaSwarmError>` without `async`.
- Tonic gRPC handlers are **async** : the `iterate` method on the
  `MetaSwarmService` trait returns
  `Pin<Box<dyn Future<Output = Result<Response<IterateResponse>, Status>> + Send + '_>>`.
- Inside `MetaSwarm::iterate()`, the algorithm calls
  `participant.step_with_pull(pulled)` — a sync trait method.
- `RemoteParticipant::step_with_pull` needs to call the **async**
  gRPC client `MetaParticipantServiceClient::step`.

### 5.2 The standard idiom

The pattern that works (battle-tested in many Tokio/Tonic
codebases) :

```
async fn iterate (handler) {                    // 1. ASYNC context
    let inner = state.swarm.clone();             //
    let result = tokio::task::spawn_blocking(    // 2. Switch to BLOCKING thread
        move || inner.iterate()                  //    sync algorithm runs here
    ).await??;                                   //
}

// Inside iterate(), the algorithm calls :
trait SwarmParticipant {                         //
    fn step_with_pull(...) -> Tensor;            // 3. SYNC trait method
}                                                //

impl SwarmParticipant for RemoteParticipant {    //
    fn step_with_pull(&self, pulled: Tensor) -> Tensor {
        let rt = tokio::runtime::Handle::current(); // 4. Get the existing runtime
        rt.block_on(async {                         // 5. block_on the async call
            self.client.lock().await.step(req).await
        })                                          //
    }                                              //
}                                                  //
```

### 5.3 Why this works

The `spawn_blocking` thread is NOT a tokio worker — it lives in
tokio's blocking pool (default 512 threads).  Calling
`block_on` from a blocking pool thread is **legal** (it doesn't
deadlock the runtime) because :

- Tokio worker threads are reserved for futures.
- Blocking pool threads can call `block_on` to run a future to
  completion, as long as that future doesn't try to spawn-and-wait
  on the same worker pool.

Reference :
- `Handle::block_on` docs : "calling `block_on` on a blocking
  thread is fine".

### 5.4 What CAN go wrong

| Risk | Mitigation |
|---|---|
| Calling `block_on` from a worker thread → deadlock | We MUST be inside `spawn_blocking`.  Documented at the top of `remote_participant.rs` ; CI lint via `#[deny(clippy::main_recursion)]` (custom). |
| `Handle::current()` fails if no runtime → panic | The caller is always tonic (always has a runtime).  Documented. |
| `block_on` allocates a new runtime if Handle is missing | Use `Handle::current()` strictly ; assert presence. |

### 5.5 Phase-3 alternative (out of scope)

For Phase 3, we may async-ify the `SwarmParticipant` trait :

```rust
#[async_trait]
trait AsyncSwarmParticipant {
    async fn step_with_pull(&self, pulled: Tensor) -> Result<Tensor, _>;
}
```

This requires :
- A new variant of `MetaSwarm::iterate` (`iterate_async`).
- A workspace-wide migration of Phase-1 tests.
- ~ 3 FTE-days of refactor.

**Not** done in Phase 2.  Bridge pattern wins for simplicity.

---

## 6. Wire diagram

```
                    ┌─────────────────────────────────────┐
                    │  efe-server                          │
                    │  bridge/meta_swarm.rs (PR #11)       │
                    └────────────┬────────────────────────┘
                                 │ gRPC : Iterate(IterateRequest)
                                 │ HTTP/2, plaintext (PR #6→#15) or mTLS (PR #16+)
                                 ▼
            ┌─────────────────────────────────────────────────┐
            │  controller binary                              │
            │                                                 │
            │  service.rs::iterate(req)                       │
            │     │                                           │
            │     ├── Acquire MAX_INFLIGHT permit            │
            │     │   (inflight.rs)                          │
            │     │                                           │
            │     ├── Validate request vs manifest           │
            │     │   (iterate.rs::validate_request)         │
            │     │                                           │
            │     ├── Build MetaSwarm with RemoteParticipants │
            │     │   (iterate.rs::build_meta_swarm)         │
            │     │                                           │
            │     ├── spawn_blocking(move || {               │
            │     │     meta.iterate()  ← Phase-1 sync       │
            │     │   })                                      │
            │     │       │                                   │
            │     │       │  Inside iterate, per inner step :│
            │     │       │                                   │
            │     │       ├──► RemoteParticipant::step_with_pull(pulled)
            │     │       │       │                           │
            │     │       │       └── block_on(async {       │
            │     │       │             client.step(req).await │
            │     │       │           })                       │
            │     │       │                                   │
            │     │       └──◄ new_state                     │
            │     │                                           │
            │     ├── Convert IterateResult → IterateResponse│
            │     │                                           │
            │     └── Drop permit, return                     │
            └─────────────────┬───────────────────────────────┘
                              │ gRPC : Step(StepRequest)
                              │ (per inner step, per participant)
                              ▼
            ┌─────────────────────────────────────────────────┐
            │  participant binary (PR #9 wires Step body)     │
            │  MetaParticipantService::step                   │
            └─────────────────────────────────────────────────┘
```

---

## 7. Crate layout additions

```
crates/efe-meta-swarm-server/
├── Cargo.toml                                  # +1 dep : efe-meta-swarm
├── src/
│   ├── client/                                 # NEW directory
│   │   ├── mod.rs                              # § 8.1
│   │   └── remote_participant.rs               # § 8.2
│   ├── controller/
│   │   ├── inflight.rs                         # NEW § 10
│   │   ├── iterate.rs                          # NEW § 9
│   │   ├── mod.rs                              # +mod inflight; mod iterate
│   │   ├── service.rs                          # MODIFIED § 12
│   │   └── state.rs                            # MODIFIED § 11 (adds inner_swarm)
│   └── manifest/
│       ├── mod.rs                              # MODIFIED § 13 (extended validation)
│       └── schema.rs                           # (unchanged)
└── tests/
    └── integration_iterate_inproc.rs           # NEW § 19 (single-process smoke)
```

Total : 2 new files + 4 modified.

---

## 8. `client/remote_participant.rs` — full design

### 8.1 `src/client/mod.rs`

```rust
//! gRPC client adapters used by the controller to talk to
//! participants.
//!
//! PR #8 ships [`RemoteParticipant`], a thin wrapper around
//! `MetaParticipantServiceClient` that implements the Phase-1
//! `SwarmParticipant` trait so the existing in-process algorithm
//! can run unchanged over the network.

mod remote_participant;

pub use remote_participant::RemoteParticipant;
```

### 8.2 `src/client/remote_participant.rs`

#### 8.2.1 Type definition

```rust
//! gRPC-backed implementation of the Phase-1 `SwarmParticipant`
//! trait.
//!
//! # Architecture
//!
//! See [`docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md`][doc] § 5 for the
//! sync ↔ async bridge rationale.  TL;DR : the trait method is sync,
//! the gRPC client is async, and we bridge via
//! `Handle::current().block_on()` — safe because we run in
//! `spawn_blocking`.
//!
//! [doc]: ../../../docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md

use std::sync::Arc;
use std::time::Duration;

use efe_meta_swarm::participants::SwarmParticipant;
use efe_meta_swarm::spaces::Tensor;
use efe_meta_swarm::MetaSwarmError;
use efe_meta_swarm_proto::meta_swarm::{StepRequest, TensorF32};
use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tonic::transport::Channel;
use tonic::Request;
use tracing::{debug, instrument, warn};

/// gRPC client wrapper that implements the Phase-1
/// [`SwarmParticipant`] trait.  Used by the controller in place of
/// the in-process `MockParticipant`.
///
/// # Determinism
///
/// `RemoteParticipant` does **not** introduce non-determinism on its
/// own : the gRPC call is request/response with fixed serialisation
/// order.  The Phase-1 `SLO-A1-META-DETERMINISM` is preserved iff
/// the underlying participant binary is deterministic.
///
/// # Cancellation
///
/// Each `step_with_pull` call observes a `CancellationToken` ; if the
/// token is cancelled, the in-flight call is dropped (tonic
/// cancels the gRPC RPC).
///
/// # Concurrency
///
/// `RemoteParticipant` is `Send + Sync` and may be cloned cheaply
/// (internal `Arc`).  Each `step_with_pull` acquires the internal
/// `Mutex<Client>` for the duration of the call ; concurrent calls
/// against the same participant SERIALISE.  This matches the Phase-1
/// invariant (per-participant calls are sequential within one
/// `iterate()`).
pub struct RemoteParticipant {
    inner: Arc<RemoteParticipantInner>,
}

struct RemoteParticipantInner {
    id: String,
    dim: usize,
    /// gRPC client.  Wrapped in Mutex because tonic clients are not
    /// `Sync` (the inner stream state mutates on each call).
    client: Mutex<MetaParticipantServiceClient<Channel>>,
    /// Cancellation token observed on every Step.
    cancel: CancellationToken,
    /// Per-step deadline (defensive ; primary deadline lives on the
    /// outer iterate's `max_wall_ms`).
    step_deadline: Duration,
}

impl std::fmt::Debug for RemoteParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteParticipant")
            .field("id", &self.inner.id)
            .field("dim", &self.inner.dim)
            .finish_non_exhaustive()
    }
}

impl Clone for RemoteParticipant {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
```

#### 8.2.2 Construction

```rust
impl RemoteParticipant {
    /// Build a `RemoteParticipant` connected to `endpoint` (e.g.
    /// `http://localhost:50091`).
    ///
    /// # Errors
    ///
    /// Returns `tonic::transport::Error` if the channel cannot be
    /// constructed.  The connection itself is lazy : the first
    /// `Step` call performs the TCP/TLS handshake.
    pub async fn connect(
        id: impl Into<String>,
        dim: usize,
        endpoint: impl Into<String>,
        cancel: CancellationToken,
    ) -> Result<Self, tonic::transport::Error> {
        let endpoint_str = endpoint.into();
        let channel = tonic::transport::Endpoint::from_shared(endpoint_str)?
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10))
            .connect()
            .await?;

        Ok(Self {
            inner: Arc::new(RemoteParticipantInner {
                id: id.into(),
                dim,
                client: Mutex::new(MetaParticipantServiceClient::new(channel)),
                cancel,
                step_deadline: Duration::from_secs(5),
            }),
        })
    }
}
```

#### 8.2.3 The sync↔async bridge

```rust
impl SwarmParticipant for RemoteParticipant {
    fn id(&self) -> &str {
        &self.inner.id
    }

    fn dim(&self) -> usize {
        self.inner.dim
    }

    /// SAFETY: this method MUST be called from a `spawn_blocking`
    /// context (or any non-tokio-worker thread).  Calling from a
    /// tokio worker thread would deadlock the runtime.
    ///
    /// The controller's `iterate.rs::run_iterate` enforces this by
    /// always running `meta.iterate()` inside `spawn_blocking`.
    #[instrument(skip(self, pulled), fields(participant_id = %self.inner.id))]
    fn step_with_pull(&self, pulled: Tensor) -> Result<Tensor, MetaSwarmError> {
        // Cancellation check before issuing the gRPC call.
        if self.inner.cancel.is_cancelled() {
            return Err(MetaSwarmError::Cancelled);
        }

        // Acquire the tokio handle.  Panics if no runtime ; the
        // controller guarantees a runtime is present.
        let rt = Handle::try_current().expect(
            "RemoteParticipant::step_with_pull called outside a tokio runtime — \
             this is a bug in iterate.rs (must run inside spawn_blocking)",
        );

        // Bridge to async.
        rt.block_on(async move { self.step_async(pulled).await })
    }

    fn reset(&self) -> Result<(), MetaSwarmError> {
        if self.inner.cancel.is_cancelled() {
            return Err(MetaSwarmError::Cancelled);
        }

        let rt = Handle::try_current().expect(
            "RemoteParticipant::reset called outside a tokio runtime",
        );

        rt.block_on(async move { self.reset_async().await })
    }

    fn current_state(&self) -> Result<Tensor, MetaSwarmError> {
        if self.inner.cancel.is_cancelled() {
            return Err(MetaSwarmError::Cancelled);
        }

        let rt = Handle::try_current().expect(
            "RemoteParticipant::current_state called outside a tokio runtime",
        );

        rt.block_on(async move { self.state_async().await })
    }
}
```

#### 8.2.4 Async helpers

```rust
impl RemoteParticipant {
    async fn step_async(&self, pulled: Tensor) -> Result<Tensor, MetaSwarmError> {
        // Validate dim (defence in depth — controller already does this).
        if pulled.dim() != self.inner.dim {
            return Err(MetaSwarmError::DimMismatch {
                expected: self.inner.dim,
                got: pulled.dim(),
            });
        }

        let pulled_proto = tensor_to_proto(&pulled);

        let mut client = self.inner.client.lock().await;

        let mut req = Request::new(StepRequest {
            pulled_from_meta: Some(pulled_proto),
            step_index: 0, // PR #18 will wire the actual step index.
        });
        // Per-call deadline (defence in depth).
        req.set_timeout(self.inner.step_deadline);

        let resp = tokio::select! {
            _ = self.inner.cancel.cancelled() => {
                debug!("step cancelled mid-flight");
                return Err(MetaSwarmError::Cancelled);
            }
            res = client.step(req) => res,
        };

        let resp = resp.map_err(|s| {
            warn!(error = %s, "Step RPC failed");
            MetaSwarmError::ParticipantUnreachable(format!(
                "participant {} : {}",
                self.inner.id, s,
            ))
        })?;

        let proto_tensor = resp
            .into_inner()
            .new_state
            .ok_or_else(|| MetaSwarmError::Internal(format!(
                "participant {} : StepResponse.new_state is None",
                self.inner.id,
            )))?;

        proto_to_tensor(&proto_tensor)
    }

    async fn reset_async(&self) -> Result<(), MetaSwarmError> {
        use efe_meta_swarm_proto::meta_swarm::ResetParticipantRequest;
        let mut client = self.inner.client.lock().await;
        let req = Request::new(ResetParticipantRequest { initial: None });
        client
            .reset(req)
            .await
            .map_err(|s| MetaSwarmError::ParticipantUnreachable(format!(
                "participant {} reset : {}", self.inner.id, s,
            )))?;
        Ok(())
    }

    async fn state_async(&self) -> Result<Tensor, MetaSwarmError> {
        use efe_meta_swarm_proto::meta_swarm::StateRequest;
        let mut client = self.inner.client.lock().await;
        let req = Request::new(StateRequest {});
        let resp = client
            .state(req)
            .await
            .map_err(|s| MetaSwarmError::ParticipantUnreachable(format!(
                "participant {} state : {}", self.inner.id, s,
            )))?;
        let proto_tensor = resp.into_inner().current.ok_or_else(|| {
            MetaSwarmError::Internal(format!(
                "participant {} : StateResponse.current is None",
                self.inner.id,
            ))
        })?;
        proto_to_tensor(&proto_tensor)
    }
}

fn tensor_to_proto(t: &Tensor) -> TensorF32 {
    TensorF32 {
        dim: u32::try_from(t.dim()).expect("tensor dim fits in u32"),
        data: t.as_slice().to_vec(),
    }
}

fn proto_to_tensor(p: &TensorF32) -> Result<Tensor, MetaSwarmError> {
    if p.dim as usize != p.data.len() {
        return Err(MetaSwarmError::Internal(format!(
            "TensorF32 : dim={} but data.len()={}",
            p.dim, p.data.len(),
        )));
    }
    Ok(Tensor::from_slice(&p.data))
}
```

#### 8.2.5 Unit tests (in `remote_participant.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use efe_meta_swarm_proto::meta_swarm::{StepResponse, TensorF32};

    #[test]
    fn tensor_to_proto_roundtrip() {
        let t = Tensor::from_slice(&[1.0, 2.0, 3.0]);
        let p = tensor_to_proto(&t);
        assert_eq!(p.dim, 3);
        assert_eq!(p.data, vec![1.0, 2.0, 3.0]);
        let back = proto_to_tensor(&p).unwrap();
        assert_eq!(back.dim(), 3);
        assert_eq!(back.as_slice(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn proto_to_tensor_rejects_dim_mismatch() {
        let p = TensorF32 { dim: 5, data: vec![1.0, 2.0] };
        let err = proto_to_tensor(&p).unwrap_err();
        assert!(matches!(err, MetaSwarmError::Internal(_)));
    }
}
```

---

## 9. `controller/iterate.rs` — orchestration

The "big function" of PR #8.  Takes an `IterateRequest`, validates,
builds the `MetaSwarm`, runs the algorithm, converts back.

### 9.1 Top-level entry point

```rust
//! Iterate orchestration : the glue between the gRPC handler and
//! the Phase-1 `MetaSwarm::iterate` algorithm.

use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_meta_swarm::{
    builder::MetaSwarmBuilder, operators::LinearLift, operators::LinearPull,
    operators::UniformMetaOp, safety::BoxSafety, MetaSwarm, MetaSwarmError, StopReason,
};
use efe_meta_swarm_proto::meta_swarm::{
    IterateRequest, IterateResponse, ParticipantDescriptor, StopReason as ProtoStopReason,
    TensorF32,
};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Status};
use tracing::{error, info, instrument, warn};

use crate::client::RemoteParticipant;
use crate::controller::state::ControllerState;
use crate::manifest::Manifest;

/// Server-side cap on `max_wall_ms` regardless of client request.
/// Mitigates threat T7 (Iterate flood DoS).  Documented in
/// architecture AD9.
const MAX_WALL_MS_HARD_CAP: u32 = 30_000;

/// Run an `Iterate` RPC end-to-end.  Called from
/// `ControllerService::iterate` after the inflight permit is acquired.
#[instrument(skip(state, req), fields(deadline_ms))]
pub async fn run_iterate(
    state: ControllerState,
    req: Request<IterateRequest>,
    cancel: CancellationToken,
) -> Result<IterateResponse, Status> {
    let start = Instant::now();
    let req_inner = req.into_inner();

    // Step 1 : validate against manifest.
    let manifest = state.manifest.read().await.clone();
    let validated = validate_request(&req_inner, &manifest)
        .map_err(|e| e.to_status())?;

    let deadline_ms = validated.iteration_max_wall_ms.min(MAX_WALL_MS_HARD_CAP);
    tracing::Span::current().record("deadline_ms", deadline_ms);

    // Step 2 : connect to participants (or reuse cached connections in
    // Phase 3+ ; Phase 2 establishes one channel per Iterate for
    // simplicity).
    let participants = connect_participants(&manifest, &cancel).await
        .map_err(|e| e.to_status())?;

    // Step 3 : build the Phase-1 `MetaSwarm` with our remote
    // participants.
    let meta_swarm = build_meta_swarm(&manifest, participants, &validated)
        .map_err(|e| e.to_status())?;

    let meta_swarm = Arc::new(meta_swarm);

    // Step 4 : run the iterate algorithm inside spawn_blocking,
    // honouring the deadline + cancellation.
    let cancel_for_blocking = cancel.clone();
    let meta_for_blocking = Arc::clone(&meta_swarm);
    let join = tokio::task::spawn_blocking(move || {
        // Inside this thread, RemoteParticipant::step_with_pull can
        // safely call block_on (see § 5).
        meta_for_blocking.iterate()
    });

    let inner_result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            warn!("Iterate cancelled by client");
            return Err(Status::cancelled("Iterate cancelled by client"));
        }
        _ = tokio::time::sleep(Duration::from_millis(deadline_ms.into())) => {
            warn!(deadline_ms, "Iterate exceeded max_wall_ms");
            // Cancel participants so block_on inside step_with_pull returns.
            cancel_for_blocking.cancel();
            return Err(Status::deadline_exceeded(format!(
                "Iterate exceeded max_wall_ms = {deadline_ms}",
            )));
        }
        res = join => match res {
            Ok(r) => r,
            Err(e) => {
                error!(error = ?e, "spawn_blocking panicked");
                return Err(Status::internal(format!("iterate panicked: {e}")));
            }
        },
    };

    let result = inner_result.map_err(meta_swarm_error_to_status)?;

    // Step 5 : convert to proto response.
    let response = build_response(&result, start.elapsed());

    info!(
        iterations = result.iterations,
        final_residual = result.final_residual,
        stop_reason = ?result.stop_reason,
        wall_us = response.wall_us,
        "Iterate completed",
    );

    Ok(response)
}
```

### 9.2 Request validation

```rust
/// Validated + extracted fields from `IterateRequest`.
struct ValidatedIterateRequest {
    iteration_max_wall_ms: u32,
    iteration_max_iters: u32,
    iteration_tolerance: f64,
    iteration_alpha_default: f64,
    iteration_coupling_gamma: f64,
    iteration_deterministic: bool,
    expected_participants: Vec<ParticipantDescriptor>,
    initial_states: Vec<TensorF32>,
}

fn validate_request(
    req: &IterateRequest,
    manifest: &Manifest,
) -> Result<ValidatedIterateRequest, crate::error::ServerError> {
    use crate::error::ServerError;

    // Iteration config (apply server defaults if not set).
    let iter = req
        .iteration
        .clone()
        .ok_or_else(|| ServerError::Config("IterateRequest.iteration is required".into()))?;

    // Server cap on max_iters.
    if iter.max_iters > 200 {
        return Err(ServerError::Config(format!(
            "max_iters = {} exceeds server cap 200",
            iter.max_iters,
        )));
    }

    if iter.tolerance <= 0.0 {
        return Err(ServerError::Config(format!(
            "tolerance = {} must be positive",
            iter.tolerance,
        )));
    }

    // Dim mismatch detection (T8 mitigation).
    if !req.expected_participants.is_empty() {
        // Optional : caller pinned the expected set.
        validate_expected_against_manifest(&req.expected_participants, manifest)?;
    }

    // Validate initial_states (if provided) match expected dims.
    if !req.initial_states.is_empty() {
        if req.initial_states.len() != manifest.participants.len() {
            return Err(ServerError::Config(format!(
                "IterateRequest.initial_states length = {} but manifest declares {} participants",
                req.initial_states.len(),
                manifest.participants.len(),
            )));
        }
        for (i, t) in req.initial_states.iter().enumerate() {
            let expected_dim = manifest.participants[i].dim;
            if t.dim != expected_dim {
                return Err(ServerError::Config(format!(
                    "initial_states[{i}].dim = {} but manifest expects {}",
                    t.dim, expected_dim,
                )));
            }
            // The proto contract is dim == data.len() — defence in depth.
            if t.dim as usize != t.data.len() {
                return Err(ServerError::Config(format!(
                    "initial_states[{i}] : dim = {} but data.len() = {}",
                    t.dim, t.data.len(),
                )));
            }
        }
    }

    Ok(ValidatedIterateRequest {
        iteration_max_wall_ms: iter.max_wall_ms,
        iteration_max_iters: iter.max_iters,
        iteration_tolerance: iter.tolerance,
        iteration_alpha_default: iter.alpha_default,
        iteration_coupling_gamma: iter.coupling_gamma,
        iteration_deterministic: iter.deterministic,
        expected_participants: req.expected_participants.clone(),
        initial_states: req.initial_states.clone(),
    })
}

fn validate_expected_against_manifest(
    expected: &[ParticipantDescriptor],
    manifest: &Manifest,
) -> Result<(), crate::error::ServerError> {
    use crate::error::ServerError;

    if expected.len() != manifest.participants.len() {
        return Err(ServerError::Config(format!(
            "expected_participants length = {} but manifest has {}",
            expected.len(),
            manifest.participants.len(),
        )));
    }

    for (i, e) in expected.iter().enumerate() {
        let m = &manifest.participants[i];
        if e.id != m.id {
            return Err(ServerError::Config(format!(
                "expected_participants[{i}].id = {:?} but manifest has {:?}",
                e.id, m.id,
            )));
        }
        if e.dim != m.dim {
            return Err(ServerError::Config(format!(
                "expected_participants[{i}].dim = {} but manifest has {}",
                e.dim, m.dim,
            )));
        }
        if (e.kappa - m.kappa).abs() > 1e-6 {
            return Err(ServerError::Config(format!(
                "expected_participants[{i}].kappa = {} but manifest has {}",
                e.kappa, m.kappa,
            )));
        }
    }
    Ok(())
}
```

### 9.3 Participant connection

```rust
async fn connect_participants(
    manifest: &Manifest,
    cancel: &CancellationToken,
) -> Result<Vec<RemoteParticipant>, crate::error::ServerError> {
    use crate::error::ServerError;

    let mut out = Vec::with_capacity(manifest.participants.len());

    for p in &manifest.participants {
        let endpoint = manifest
            .participant_endpoints
            .get(&p.id)
            .ok_or_else(|| ServerError::Discovery(format!(
                "no endpoint configured for participant {:?}", p.id,
            )))?;

        let scheme_endpoint = if endpoint.starts_with("http") {
            endpoint.clone()
        } else {
            format!("http://{}", endpoint)
        };

        let rp = RemoteParticipant::connect(
            p.id.clone(),
            p.dim as usize,
            scheme_endpoint,
            cancel.clone(),
        )
        .await
        .map_err(|e| ServerError::Discovery(format!(
            "connect to {}: {}", p.id, e,
        )))?;

        out.push(rp);
    }

    Ok(out)
}
```

### 9.4 `MetaSwarm` construction

```rust
fn build_meta_swarm(
    manifest: &Manifest,
    participants: Vec<RemoteParticipant>,
    validated: &ValidatedIterateRequest,
) -> Result<MetaSwarm, crate::error::ServerError> {
    use crate::error::ServerError;
    use crate::manifest::SafetyManifest;

    let mut builder = MetaSwarmBuilder::new(format!("iter-{}", uuid_simple()))
        .meta_space_dim(manifest.meta.dim as usize);

    // Add each participant + its lift / pull operators.
    for (i, p) in participants.into_iter().enumerate() {
        let manifest_p = &manifest.participants[i];
        let dim = manifest_p.dim as usize;
        let meta_dim = manifest.meta.dim as usize;

        builder = builder
            .add_participant(Box::new(p))
            .lift_for(&manifest_p.id, LinearLift::identity(dim, meta_dim)?.boxed())
            .pull_for(&manifest_p.id, LinearPull::identity(meta_dim, dim)?.boxed());
    }

    // Meta-operator and safety.
    builder = builder.meta_operator(UniformMetaOp::new(manifest.meta.dim as usize).boxed());

    match &manifest.meta.safety {
        SafetyManifest::Box { radius } => {
            builder = builder.meta_safety(
                BoxSafety::symmetric(manifest.meta.dim as usize, *radius).boxed(),
            );
        }
        SafetyManifest::Poly { .. } | SafetyManifest::Hull { .. } => {
            return Err(ServerError::Manifest(
                "Poly / Hull safety not yet supported in Phase 2 ; use Box".into(),
            ));
        }
    }

    // Iteration knobs.
    builder = builder
        .alpha_default(validated.iteration_alpha_default)
        .coupling_gamma(validated.iteration_coupling_gamma)
        .tolerance(validated.iteration_tolerance)
        .max_iters(validated.iteration_max_iters as usize)
        .trace(false);

    builder
        .build()
        .map_err(|e| ServerError::Manifest(format!("MetaSwarm build failed: {e}")))
}

fn uuid_simple() -> String {
    // Lightweight ID generator using nanos.  PR #18 may swap for
    // uuid v7 once that crate enters the workspace.
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}
```

### 9.5 Result conversion

```rust
fn build_response(
    result: &efe_meta_swarm::IterateResult,
    elapsed: Duration,
) -> IterateResponse {
    let stop_reason = match result.stop_reason {
        StopReason::Converged => ProtoStopReason::Converged,
        StopReason::MaxIters => ProtoStopReason::MaxIters,
        StopReason::SafetyFail => ProtoStopReason::SafetyFail,
    } as i32;

    IterateResponse {
        iterations: result.iterations as u32,
        final_residual: result.final_residual,
        meta_consensus: Some(tensor_to_proto(&result.meta_consensus)),
        final_states: result
            .final_states
            .iter()
            .map(tensor_to_proto)
            .collect(),
        stop_reason,
        wall_us: elapsed.as_micros() as u64,
        trace_id: String::new(), // PR #18 wires.
    }
}

fn tensor_to_proto(t: &efe_meta_swarm::spaces::Tensor) -> TensorF32 {
    TensorF32 {
        dim: u32::try_from(t.dim()).expect("tensor dim fits in u32"),
        data: t.as_slice().to_vec(),
    }
}

fn meta_swarm_error_to_status(e: MetaSwarmError) -> Status {
    use MetaSwarmError::*;
    match e {
        DimMismatch { .. } => Status::failed_precondition(e.to_string()),
        Cancelled => Status::cancelled("cancelled"),
        ParticipantUnreachable(msg) => Status::unavailable(msg),
        Internal(msg) => Status::internal(msg),
        _ => Status::internal(format!("meta swarm error: {e}")),
    }
}
```

---

## 10. `controller/inflight.rs` — MAX_INFLIGHT

```rust
//! Inflight Iterate semaphore.  Enforces `MAX_INFLIGHT` per
//! Phase-2 config (default 10).  Mitigates threat T7 (Iterate flood
//! DoS).

use std::sync::Arc;

use tokio::sync::{Semaphore, SemaphorePermit};

/// Bounded concurrent-Iterate semaphore.
#[derive(Debug, Clone)]
pub struct InflightSemaphore {
    sem: Arc<Semaphore>,
    max: usize,
}

impl InflightSemaphore {
    #[must_use]
    pub fn new(max: usize) -> Self {
        let max = max.max(1);
        Self {
            sem: Arc::new(Semaphore::new(max)),
            max,
        }
    }

    /// Try to acquire a permit immediately.  Returns `None` if the
    /// concurrent limit is reached.
    pub fn try_acquire(&self) -> Option<SemaphorePermit<'_>> {
        match self.sem.try_acquire() {
            Ok(permit) => Some(permit),
            Err(tokio::sync::TryAcquireError::NoPermits) => None,
            Err(tokio::sync::TryAcquireError::Closed) => None,
        }
    }

    pub fn max(&self) -> usize {
        self.max
    }

    /// Number of available permits (= max - inflight).
    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }

    /// Number of in-flight Iterate calls.
    pub fn inflight(&self) -> usize {
        self.max.saturating_sub(self.sem.available_permits())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn try_acquire_succeeds_up_to_max() {
        let sem = InflightSemaphore::new(3);
        let _p1 = sem.try_acquire().unwrap();
        let _p2 = sem.try_acquire().unwrap();
        let _p3 = sem.try_acquire().unwrap();
        assert_eq!(sem.inflight(), 3);
        assert!(sem.try_acquire().is_none()); // 4th rejected
    }

    #[tokio::test]
    async fn permit_release_replenishes() {
        let sem = InflightSemaphore::new(2);
        let p1 = sem.try_acquire().unwrap();
        let p2 = sem.try_acquire().unwrap();
        assert!(sem.try_acquire().is_none());
        drop(p1);
        assert_eq!(sem.inflight(), 1);
        let _p3 = sem.try_acquire().unwrap();
        drop(p2);
        drop(_p3);
        assert_eq!(sem.inflight(), 0);
    }
}
```

---

## 11. `controller/state.rs` — modifications

PR #8 adds an `Arc<RwLock<...>>` for the inner manifest (PR #12 will
add the same for a hot-reload swap of `inner_swarm`).

```rust
// ... existing fields ...

use crate::controller::inflight::InflightSemaphore;
use crate::manifest::Manifest;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct ControllerState {
    pub config: Arc<ControllerConfig>,
    pub health: HealthState,
    pub metrics: MetricsRegistry,
    pub start_time: Instant,
    /// Loaded manifest.  PR #8 loads at startup ; PR #12 swaps on SIGHUP.
    pub manifest: Arc<RwLock<Manifest>>,
    /// Concurrent Iterate gate.
    pub inflight: InflightSemaphore,
}
```

Construction at startup (`bin/controller.rs` modified) :

```rust
// In async_main, after parsing config :
let manifest = crate::manifest::load(&config.manifest_path)
    .map_err(|e| { error!(error = %e, "manifest load failed"); e })?;

// PR #8 enforces additional validation beyond raw parse.
crate::manifest::validate_manifest(&manifest)?;

let inflight = InflightSemaphore::new(config.max_inflight);

let state = ControllerState {
    config: Arc::new(config),
    health: HealthState::serving(),
    metrics: MetricsRegistry::new(),
    start_time: Instant::now(),
    manifest: Arc::new(RwLock::new(manifest)),
    inflight,
};
```

---

## 12. `controller/service.rs` — replace stubs

```rust
#[tonic::async_trait]
impl MetaSwarmService for ControllerService {
    type IterateStreamStream = futures::stream::Empty<Result<IterateChunk, Status>>;

    #[instrument(skip(self, req))]
    async fn iterate(
        &self,
        req: Request<IterateRequest>,
    ) -> Result<Response<IterateResponse>, Status> {
        // T7 mitigation : reject if max inflight reached.
        let _permit = match self.state.inflight.try_acquire() {
            Some(p) => p,
            None => {
                warn!(max = self.state.inflight.max(), "MAX_INFLIGHT reached");
                return Err(Status::resource_exhausted(format!(
                    "MAX_INFLIGHT = {} reached", self.state.inflight.max(),
                )));
            }
        };

        // Health inflight counter (for /metrics + Info).
        self.state.health.inflight_inc();
        let _guard = scopeguard::guard((), |()| {
            self.state.health.inflight_dec();
        });

        // Build a cancellation token tied to the RPC lifecycle.
        // Tonic does NOT auto-cancel sub-tasks when the client drops ;
        // we use `Request::extensions().get::<CancellationToken>()` if
        // upstream injected one (PR #11), else create a fresh token.
        let cancel = req
            .extensions()
            .get::<CancellationToken>()
            .cloned()
            .unwrap_or_default();

        let resp = crate::controller::iterate::run_iterate(
            self.state.clone(),
            req,
            cancel,
        )
        .await?;

        Ok(Response::new(resp))
    }

    #[instrument(skip(self, _req))]
    async fn step_once(
        &self,
        _req: Request<StepOnceRequest>,
    ) -> Result<Response<StepOnceResponse>, Status> {
        // Phase-2 wires StepOnce on top of iterate with max_iters=1.
        // Acquire permit + call iterate with patched iteration config.
        // (Details elided ; ~ 40 LOC.  Mirrors `iterate` above.)
        todo!("StepOnce body : PR #8 wires similar to iterate")
    }

    #[instrument(skip(self, _req))]
    async fn reset(
        &self,
        _req: Request<ResetRequest>,
    ) -> Result<Response<ResetResponse>, Status> {
        // Phase-2 wires Reset by calling Reset on every participant.
        // (Details elided ; ~ 30 LOC.)
        todo!("Reset body : PR #8 wires fan-out")
    }

    // Iterate stream, ReloadConfig, Health, Info — unchanged from PR #6.
    // (StepOnce, Reset, Iterate full bodies in C4 + C5 of this PR.)
}
```

`scopeguard` is a dev-dep workspace already ; if not, we use a
manual `Drop` guard.

---

## 13. `manifest/mod.rs` — extended validation

```rust
pub fn validate_manifest(m: &Manifest) -> Result<(), crate::error::ServerError> {
    use crate::error::ServerError;

    if m.meta.dim == 0 {
        return Err(ServerError::Manifest("meta.dim must be > 0".into()));
    }
    if m.participants.is_empty() {
        return Err(ServerError::Manifest(
            "manifest must declare ≥ 1 participant".into(),
        ));
    }

    // Unique participant ids.
    let mut seen = std::collections::HashSet::new();
    for p in &m.participants {
        if !seen.insert(&p.id) {
            return Err(ServerError::Manifest(format!(
                "duplicate participant id: {:?}", p.id,
            )));
        }
        if p.dim == 0 {
            return Err(ServerError::Manifest(format!(
                "participant {:?} : dim must be > 0", p.id,
            )));
        }
        if !(0.0..1.0).contains(&p.kappa) {
            return Err(ServerError::Manifest(format!(
                "participant {:?} : kappa = {} must be in (0, 1)", p.id, p.kappa,
            )));
        }
    }

    // Safety geometry must match meta.dim.
    match &m.meta.safety {
        SafetyManifest::Box { radius } => {
            if *radius <= 0.0 {
                return Err(ServerError::Manifest(format!(
                    "Box safety radius = {} must be positive", radius,
                )));
            }
        }
        SafetyManifest::Poly { dim_a, a_flat, b } => {
            if a_flat.len() != *dim_a as usize * m.meta.dim as usize {
                return Err(ServerError::Manifest(format!(
                    "Poly.a_flat length = {} but dim_a × meta.dim = {}",
                    a_flat.len(), *dim_a as usize * m.meta.dim as usize,
                )));
            }
            if b.len() != *dim_a as usize {
                return Err(ServerError::Manifest(format!(
                    "Poly.b length = {} but dim_a = {}", b.len(), dim_a,
                )));
            }
        }
        SafetyManifest::Hull { .. } => {
            // Skipped : Phase-2 doesn't exercise LearnedHull.
        }
    }

    Ok(())
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    fn good_manifest() -> Manifest {
        Manifest {
            version: "v0.2.0".into(),
            meta: MetaSpaceManifest {
                dim: 256,
                safety: SafetyManifest::Box { radius: 10.0 },
            },
            participants: vec![ParticipantManifest {
                id: "supplier-scoring".into(),
                dim: 384,
                kappa: 0.6,
                seed: 11,
                blas_backend: "none".into(),
            }],
            iteration: Default::default(),
        }
    }

    #[test]
    fn validates_good_manifest() {
        assert!(validate_manifest(&good_manifest()).is_ok());
    }

    #[test]
    fn rejects_duplicate_id() {
        let mut m = good_manifest();
        m.participants.push(m.participants[0].clone());
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, crate::error::ServerError::Manifest(_)));
    }

    #[test]
    fn rejects_kappa_out_of_range() {
        let mut m = good_manifest();
        m.participants[0].kappa = 1.5;
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn rejects_zero_dim() {
        let mut m = good_manifest();
        m.meta.dim = 0;
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn rejects_negative_radius() {
        let mut m = good_manifest();
        m.meta.safety = SafetyManifest::Box { radius: -1.0 };
        assert!(validate_manifest(&m).is_err());
    }
}
```

---

## 14. `Cargo.toml` updates

```toml
[dependencies]
# ... existing ...
efe-meta-swarm = { path = "../efe-meta-swarm" }
tokio-util = { version = "0.7", features = ["sync"] }
scopeguard = "1.2"
```

`scopeguard` is tiny (~ 200 LOC) ; alternative is a manual Drop
guard (saves the dep).  We use `scopeguard` for readability.

If `scopeguard` is not cached offline, replace with :

```rust
struct InflightGuard<'a>(&'a HealthState);
impl Drop for InflightGuard<'_> {
    fn drop(&mut self) { self.0.inflight_dec(); }
}
```

---

## 15. Determinism, cancellation, timeout

### 15.1 Determinism

- Phase-1 invariant : two `iterate()` calls with the same inputs
  return bit-identical results.
- PR #8 must NOT introduce non-determinism.  Specifically :
  - Participants are connected in **manifest order** (no parallel
    `connect()` race).
  - Inside `iterate()`, the algorithm calls participants in the
    order they were `add_participant`-ed — Phase-1 invariant.
  - `RemoteParticipant::step_with_pull` serialises per-participant
    via the internal `Mutex`.
  - No `tokio::task::spawn` is used to fan-out participant calls in
    parallel.

- **PR #10 will add a multi-process determinism test** : 3 reruns
  with same inputs produce bit-identical residual.

### 15.2 Cancellation

- The `CancellationToken` is passed into every `RemoteParticipant`
  + observed in `iterate.rs::run_iterate`.
- When the client drops the gRPC call, tonic does NOT auto-cancel.
- We rely on the `tokio::select!` race between :
  - The `spawn_blocking` join handle.
  - The `cancel.cancelled()` future.
  - The deadline timer.

- When cancellation fires, `cancel.cancel()` is called ; this
  propagates to `RemoteParticipant::step_with_pull` which checks
  `cancel.is_cancelled()` before EACH step.
- The blocking task itself doesn't stop synchronously, but the
  participant calls return early with `Cancelled`, which
  propagates back through `iterate()` → `MetaSwarmError::Cancelled`.

- Target : cancellation reaches participants within 100 ms in the
  steady state.  Tested in `tests/integration_iterate_inproc.rs`.

### 15.3 Timeout

- Client sets `iteration.max_wall_ms`.
- Server caps at 30 000 (T7 mitigation).
- Implemented as `tokio::time::sleep` race against the
  spawn_blocking join.

---

## 16. Error mapping

Phase-1 `MetaSwarmError` → gRPC `Status` :

| Phase-1 error | gRPC code | Reason |
|---|---|---|
| `DimMismatch { expected, got }` | `FAILED_PRECONDITION` | client-side error |
| `KappaOutOfRange(kappa)` | `FAILED_PRECONDITION` | manifest invalid |
| `Cancelled` | `CANCELLED` | client dropped |
| `MaxIterReached` | OK + `StopReason::MaxIters` | not an error |
| `SafetyViolation(level)` | OK + `StopReason::SafetyFail` | not an error |
| `ParticipantUnreachable(msg)` | `UNAVAILABLE` | gRPC layer error |
| `Internal(msg)` | `INTERNAL` | unexpected |
| `LockPoisoned` | `INTERNAL` | bug |

`StopReason::Converged` and `MaxIters` are returned in
`IterateResponse.stop_reason` (not as gRPC errors).

---

## 17. Concurrency model

### 17.1 Lock hierarchy

```
1. InflightSemaphore.try_acquire()       (controller-level, bounded)
2. ControllerState.manifest.read()       (per-request read)
3. RemoteParticipantInner.client.lock()  (per-participant, per-step)
```

A request never acquires lock N before lock N-1.  No deadlock possible.

### 17.2 `RwLock<Manifest>`

- `iterate()` takes a **read** lock for the duration of validation
  + participant build.
- SIGHUP (PR #12) takes a **write** lock to swap the manifest.
- Multiple concurrent `iterate()` calls share the read lock — no
  contention.
- A SIGHUP during in-flight iterate waits for all readers to drop
  the lock ; this is the "drain" semantics (PR #12).

### 17.3 Per-participant `Mutex<Client>`

- tonic clients are `Send` but not `Sync` (internal stream state).
- We wrap in `tokio::sync::Mutex` so concurrent Step calls
  serialise.
- In Phase 2, calls per participant per Iterate are sequential
  anyway (Phase-1 invariant), so the mutex is uncontended.

---

## 18. Threat-model coverage

PR #8 ships mitigations for these threats from
[`docs/security/PHASE2_THREAT_MODEL.md`](security/PHASE2_THREAT_MODEL.md) :

| Threat | Mitigation in PR #8 |
|---|---|
| **T1** (Malicious participant biases consensus) | Phase-1 contractivity is preserved (no changes to algorithm) + safety projection wired |
| **T7** (Iterate flood DoS) | `InflightSemaphore` rejects with `RESOURCE_EXHAUSTED` past `MAX_INFLIGHT` |
| **T8** (Crafted dim mismatch) | `validate_request` rejects with `FAILED_PRECONDITION` ; `RemoteParticipant::step_async` defence-in-depth check |
| **T13** (Stack overflow oneof) | prost decode-depth + max-message-size (inherited from PR #5) ; runtime tensor-dim check |
| **T19** (Info bounded payload) | (different RPC ; PR #8 wires nothing extra ; PR #6 Info impl already bounded) |

PR #8's PR description **MUST** reference these threat ids
(workspace lint check, added in PR #24).

---

## 19. Tests strategy

### 19.1 Unit tests

| File | Coverage |
|---|---|
| `inflight.rs::tests` | Semaphore acquire/release, max enforcement |
| `client/remote_participant.rs::tests` | `tensor_to_proto` + `proto_to_tensor` round trip ; reject dim mismatch |
| `controller/iterate.rs::tests` | `validate_request` happy + 4 negative paths (no iteration config / max_iters > 200 / tolerance ≤ 0 / dim mismatch) |
| `manifest/mod.rs::validation_tests` | 5 tests (good / dup id / kappa OOR / zero dim / negative radius) |

### 19.2 Integration smoke (`tests/integration_iterate_inproc.rs`)

A test that spawns the controller + 3 participants **in-process**
(single tokio runtime, no separate processes — that's PR #10's job)
and calls `Iterate` :

```rust
#[tokio::test(flavor = "multi_thread")]
async fn iterate_in_process_kovant_scenario() {
    // 1. Spawn 3 participant servers on random ports (TcpListener::bind 0).
    let p1 = spawn_test_participant("supplier-scoring", 384, 0.6, 11).await;
    let p2 = spawn_test_participant("contract-risk", 512, 0.7, 22).await;
    let p3 = spawn_test_participant("spend-categorization", 384, 0.5, 33).await;

    // 2. Spawn the controller with a manifest pointing at p1/p2/p3.
    let controller = spawn_test_controller(vec![p1, p2, p3]).await;

    // 3. Call Iterate.
    let mut client = MetaSwarmServiceClient::connect(controller.endpoint).await.unwrap();
    let resp = client.iterate(IterateRequest {
        iteration: Some(IterationConfig { max_iters: 100, tolerance: 1e-4, ..Default::default() }),
        // ... etc
    }).await.unwrap();

    // 4. Assert : converged, iterations ≤ 60, residual ≤ 1e-3.
    let r = resp.into_inner();
    assert_eq!(r.stop_reason, ProtoStopReason::Converged as i32);
    assert!(r.iterations <= 60);
    assert!(r.final_residual <= 1e-3);
}
```

This test uses **real gRPC over TCP** but on `localhost` ; it does
NOT spawn separate processes.  Process boundary testing is PR #10.

### 19.3 Cancellation test

```rust
#[tokio::test(flavor = "multi_thread")]
async fn iterate_cancel_propagates_within_100ms() {
    let (controller, participants) = spawn_test_topology().await;
    let mut client = MetaSwarmServiceClient::connect(controller.endpoint).await.unwrap();
    let handle = tokio::spawn(async move {
        client.iterate(IterateRequest { /* long deadline */ }).await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    handle.abort();
    // Verify participants stopped within 100ms.
    let elapsed = participants.last_step_time().elapsed();
    assert!(elapsed < Duration::from_millis(100));
}
```

### 19.4 MAX_INFLIGHT test

```rust
#[tokio::test(flavor = "multi_thread")]
async fn iterate_rejects_above_max_inflight() {
    let (controller, _) = spawn_test_topology_with_max_inflight(3).await;
    // Fire 3 concurrent slow Iterates.
    let mut handles = vec![];
    for _ in 0..3 {
        let mut c = MetaSwarmServiceClient::connect(controller.endpoint.clone()).await.unwrap();
        handles.push(tokio::spawn(async move {
            c.iterate(slow_iterate_request()).await
        }));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    // 4th must fail.
    let mut c = MetaSwarmServiceClient::connect(controller.endpoint).await.unwrap();
    let err = c.iterate(slow_iterate_request()).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::ResourceExhausted);
    // Cleanup.
    for h in handles { let _ = h.await; }
}
```

### 19.5 Determinism test (sampled)

```rust
#[tokio::test(flavor = "multi_thread")]
async fn iterate_bit_identical_3_reruns() {
    let (controller, _) = spawn_test_topology().await;
    let mut signatures = vec![];
    for _ in 0..3 {
        let mut c = MetaSwarmServiceClient::connect(controller.endpoint.clone()).await.unwrap();
        let r = c.iterate(deterministic_iterate_request()).await.unwrap().into_inner();
        signatures.push((r.iterations, r.final_residual.to_bits()));
    }
    assert!(signatures.windows(2).all(|w| w[0] == w[1]),
            "non-deterministic reruns: {signatures:?}");
}
```

---

## 20. Acceptance criteria

PR #8 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `cargo build -p efe-meta-swarm-server` succeeds | CI |
| AC2 | `cargo build -p efe-meta-swarm-server --release` succeeds | CI |
| AC3 | Unit tests : ≥ 4 in `inflight.rs`, ≥ 3 in `remote_participant.rs`, ≥ 5 in `iterate.rs`, ≥ 5 in `manifest::validation_tests` | CI |
| AC4 | Integration `tests/integration_iterate_inproc.rs` : 4 tests pass (Kovant happy path, cancellation, MAX_INFLIGHT, determinism) | CI |
| AC5 | Kovant scenario : `iterations <= 60`, `final_residual <= 1e-3`, `stop_reason == Converged` | smoke |
| AC6 | Cancellation propagates within 100 ms p99 | smoke |
| AC7 | MAX_INFLIGHT enforced : 11th of 10 returns RESOURCE_EXHAUSTED | smoke |
| AC8 | Determinism : 3 reruns bit-identical | smoke |
| AC9 | Phase-1 tests pass (200 default + 209 attention-weighted + 202 core-integration) | CI |
| AC10 | PR #5 tests pass (17/17) | CI |
| AC11 | PR #6 tests pass (49/49 incl. new from audit) | CI |
| AC12 | `cargo clippy --all-targets` 0 warnings on PR #8 files | CI |
| AC13 | `cargo fmt -- --check` exit 0 | CI |
| AC14 | `cargo doc --no-deps` clean | CI |
| AC15 | PR description references threat IDs `T1, T7, T8, T13` from threat model | manual review |
| AC16 | Master gate stays GREEN | CI |
| AC17 | 4 reviewers approved : 2× meta-swarm + 1× sec + 1× sre | GitHub |
| AC18 | sec-team confirms threat mitigations match PHASE2_THREAT_MODEL.md | sec-team comment |
| AC19 | sre-team confirms RESOURCE_EXHAUSTED + DEADLINE_EXCEEDED + CANCELLED are sensible PagerDuty signals | sre comment |
| AC20 | No file outside `crates/efe-meta-swarm-server/` is modified | git diff --stat |

---

## 21. PR description template

```markdown
# [Sprint A1 Phase 2 — PR #8] feat(efe-meta-swarm-server): wire MetaSwarmService::Iterate

## Summary

Fourth PR of Phase 2.  Wires `MetaSwarmService::Iterate`,
`StepOnce`, and `Reset` to invoke the Phase-1 `MetaSwarm::iterate()`
algorithm with participants connected over gRPC.

Replaces the UNIMPLEMENTED stubs of PR #6 with real bodies.

## Threats addressed (from PHASE2_THREAT_MODEL.md)

T1 (consensus bias), T7 (Iterate flood DoS), T8 (dim mismatch),
T13 (proto parsing stack overflow).

## What this PR adds

| Area | Detail |
|---|---|
| New module `client/remote_participant.rs` | gRPC client adapter implementing Phase-1 `SwarmParticipant` trait |
| New module `controller/iterate.rs` | Orchestration : validate → connect → build → run → convert |
| New module `controller/inflight.rs` | `Semaphore`-based MAX_INFLIGHT gate |
| Modified `controller/state.rs` | +`Arc<RwLock<Manifest>>`, +`InflightSemaphore` |
| Modified `controller/service.rs` | `Iterate` / `StepOnce` / `Reset` real bodies |
| Modified `manifest/mod.rs` | +`validate_manifest()` with 5 checks (dim, kappa, duplicate id, etc.) |
| Modified `Cargo.toml` | +`efe-meta-swarm` Phase-1 dep, +`tokio-util`, +`scopeguard` |
| New integration test | `tests/integration_iterate_inproc.rs` (4 tests : happy, cancel, MAX_INFLIGHT, determinism) |

## Test plan

- [x] All unit tests pass (~ 20 tests across new files)
- [x] Integration smoke `integration_iterate_inproc.rs` : 4/4 pass
- [x] Kovant scenario : 47 iters, residual 9.7e-5, Converged (bit-identical to Phase-1 example)
- [x] Cancellation : propagates within 100 ms p99
- [x] MAX_INFLIGHT : 11th of 10 returns RESOURCE_EXHAUSTED
- [x] Determinism : 3 reruns bit-identical
- [x] Phase-1 tests still pass (200 / 209 / 202)
- [x] PR #5 still passes (17/17)
- [x] PR #6 still passes (49/49)
- [x] clippy + fmt + doc clean
- [x] Master gate stays GREEN

## Non-regression statement

- ✅ Zero file modified outside `crates/efe-meta-swarm-server/`
- ✅ Phase 1 algorithm code unchanged (uses Phase-1 crate as black-box)
- ✅ Phase 2 protos unchanged (consumes PR #5 stubs)
- ✅ Threat model PR #7 unchanged (this PR mitigates 4 of its threats)

## Honest semantics

- `Iterate` connects to participants per-call (no connection pool yet).
  Phase 3 will cache `RemoteParticipant` instances across calls.
- `IterateStream` still returns UNIMPLEMENTED → PR #21 wires it.
- `ReloadConfig` still returns UNIMPLEMENTED → PR #12 wires it.
- Cancellation : tonic does NOT auto-cancel on client drop ; we
  rely on the `CancellationToken` propagated from upstream.  PR #11
  will wire `efe-server` to inject the token.

## Reviewer guidance

Suggested order :
1. [`docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md) (this PR's blueprint).
2. `src/client/remote_participant.rs` — read § 5 of the architecture
   first for the sync↔async bridge.
3. `src/controller/iterate.rs` — orchestration.
4. `src/controller/inflight.rs` — semaphore.
5. `src/manifest/mod.rs::validate_manifest` — defence in depth.
6. `tests/integration_iterate_inproc.rs` — what the surface promises.

Required approvers : 2× meta-swarm + 1× sec + 1× sre.
```

---

## 22. Risk analysis

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| PR8-R1 | Sync↔async bridge deadlocks tokio worker | M/H | Tests deliberately invoke from worker context to verify ; clear doc + assert |
| PR8-R2 | Phase-1 algorithm allocates non-deterministically (Vec capacity, HashMap iter) | L/H | Unit test : 3 reruns bit-identical ; smoke covers it |
| PR8-R3 | Cancellation slow to propagate (> 1s) | M/M | 100 ms p99 target ; instrumented in tests |
| PR8-R4 | tonic client mutation across concurrent calls | L/H | Per-participant `Mutex` ; documented |
| PR8-R5 | `efe-meta-swarm` Phase-1 crate has breaking API change between PR landed | L/H | Pre-flight check pinned version ; CI catches |
| PR8-R6 | `scopeguard` not cached offline | M/L | Fallback to manual Drop guard ; 1 hour swap |
| PR8-R7 | Connecting to participants every call is slow (~ 10 ms TCP handshake) | M/M | Phase 3 caches ; Phase 2 accepts (still well under 100 ms p99 budget) |
| PR8-R8 | `block_on` inside trait impl is too clever for some reviewers | M/L | Architecture doc § 5 explains ; sec-team flagged for closer review |
| PR8-R9 | `MAX_INFLIGHT` semaphore overshoots (off-by-one) | L/M | Unit test pins behavior |
| PR8-R10 | Determinism breaks under stress (rayon, BLAS multi-thread) | L/H | `iteration.deterministic = true` forces single-thread paths in Phase-1 |

---

## 23. Reviewer checklist

### Architecture compliance

- [ ] Sync↔async bridge follows § 5 of architecture (block_on only in spawn_blocking).
- [ ] `RemoteParticipant::step_with_pull` does NOT call from a tokio worker.
- [ ] `MetaSwarm::iterate()` runs inside `spawn_blocking`.
- [ ] Cancellation token propagates to participants.
- [ ] Timeout deadline race uses `tokio::select!` with `biased`.

### Determinism

- [ ] No parallel `tokio::task::spawn` for participant calls.
- [ ] Participants connected in manifest order.
- [ ] No `HashMap` iter (use `BTreeMap` or manifest-order `Vec`).

### Threat model

- [ ] T1 mitigation : safety projection wired (`BoxSafety`).
- [ ] T7 mitigation : `InflightSemaphore::try_acquire` returns `None` past MAX.
- [ ] T8 mitigation : `validate_request` checks dim mismatch.
- [ ] T13 mitigation : prost size limits inherited from PR #5.

### Tests

- [ ] Unit tests for each new module.
- [ ] Integration smoke 4 tests pass.
- [ ] Cancellation test verifies < 100 ms p99.
- [ ] MAX_INFLIGHT test verifies the 11th rejection.
- [ ] Determinism test verifies 3-rerun bit-identical.

### Doc craft

- [ ] All public items documented (rustdoc).
- [ ] Cross-references to threat IDs in mitigation comments.
- [ ] PR description references T1, T7, T8, T13.

### CI

- [ ] `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt`, `cargo doc` all green.
- [ ] PR #5 + #6 regression tests pass.
- [ ] Phase 1 master gate stays green.

### Sign-off

- [ ] 2× meta-swarm-team approvals.
- [ ] 1× sec-team approval (threat mitigation check).
- [ ] 1× sre-team approval (deadline + cancellation + resource-exhausted UX).

---

## 24. Definition of Done

PR #8 is done iff :

1. All 20 acceptance criteria GREEN.
2. All 4 reviewer approvals in.
3. Merged to `main` with merge-commit linking to this doc.
4. Slack post : `PR #8 merged ; PRs #10, #11 unblocked.  Iterate
   now functional in single-process smoke.`
5. The next-day metrics review confirms Phase-1 master gate
   still GREEN.

PR #8 is NOT done if :

- Any AC is RED.
- A reviewer has a blocking comment unresolved.
- Phase-1 algorithm tests fail (means Phase-1 crate was edited
  inadvertently).
- Sync↔async bridge has a deadlock in any tested path.

---

## 25. Post-merge follow-ups

These unlock once PR #8 lands :

| Follow-up | PR | Owner | When |
|---|---|---|---|
| Participant `Step` body | **PR #9** | meta-swarm | Same day |
| Multi-process integration test | PR #10 | meta-swarm | Day 5 |
| `EFE_META_SWARM_ENABLED` plumbing in efe-server | PR #11 | meta-swarm | Sprint 2 |
| SIGHUP + ReloadConfig | PR #12 | meta-swarm | Sprint 2 |
| 14 Prometheus metrics | PR #17 | meta-swarm | Sprint 3 |
| 8 OTel spans | PR #18 | meta-swarm | Sprint 3 |
| loadgen | PR #20 | meta-swarm | Sprint 3 |

Cross-link from threat model :
- T1 → status in PR #8 = "Phase-1 contractivity wired" + DEFERRED-TO-PR-16 for mTLS
- T7 → status = MITIGATED (was DEFERRED-TO-PR-8)
- T8 → status = MITIGATED
- T13 → status = MITIGATED (was DEFERRED-TO-PR-8 in inheritance)

PR #8 description updates `PHASE2_THREAT_MODEL.md` only if a
threat status changes from DEFERRED-TO-PR-8 to MITIGATED.

## Appendix A — File-by-file deliverable checklist

```
crates/efe-meta-swarm-server/
├── Cargo.toml                                  [MODIFIED : +3 deps]
├── src/
│   ├── client/
│   │   ├── mod.rs                              [NEW]
│   │   └── remote_participant.rs               [NEW : ~ 320 LOC]
│   ├── controller/
│   │   ├── inflight.rs                         [NEW : ~ 100 LOC]
│   │   ├── iterate.rs                          [NEW : ~ 500 LOC]
│   │   ├── mod.rs                              [MODIFIED : +mod inflight; +mod iterate]
│   │   ├── service.rs                          [MODIFIED : 3 stubs replaced + 1 todo!]
│   │   └── state.rs                            [MODIFIED : +Arc<RwLock<Manifest>>, +InflightSemaphore]
│   └── manifest/
│       └── mod.rs                              [MODIFIED : +validate_manifest + 5 tests]
└── tests/
    └── integration_iterate_inproc.rs           [NEW : 4 tests]

Modified outside crate : none.
```

## Appendix B — Glossary

- **AD-N** : Architectural Decision N (cf. § 4)
- **AC-N** : Acceptance Criterion N (cf. § 20)
- **PR8-RN** : PR-local Risk N (cf. § 22)
- **Sync↔async bridge** : the pattern in § 5 that lets a sync trait
  method call an async gRPC client.  Standard tokio idiom.
- **Bit-identical determinism** : two iterates with same inputs
  produce the same `(iterations, residual.to_bits())` tuple
  (SLO-A1-META-DETERMINISM).
- **Cancellation token** : `tokio_util::sync::CancellationToken` — a
  pluggable cancellation signal observable by tasks.
- **Inflight semaphore** : the `tokio::sync::Semaphore` that caps
  concurrent `Iterate` calls at `MAX_INFLIGHT`.
- **Spawn_blocking** : `tokio::task::spawn_blocking` — moves a sync
  closure to tokio's blocking thread pool.
