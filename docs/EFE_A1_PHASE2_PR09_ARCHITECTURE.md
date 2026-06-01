# EFE-A1 Phase 2 — PR #9 : `MetaParticipantService::Step` wiring

| Field | Value |
|---|---|
| **PR** | #9 (Sprint A1 Phase 2 / P2.2) |
| **Branch** | `efe-a1/p2-pr09-participant-step` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× meta-swarm + 1× sec + 1× sre |
| **Depends on** | PR #5 (proto), PR #6 (server scaffold), PR #7 (threat model), **PR #8 (controller wire)** |
| **Blocks** | PR #10 (multi-process e2e), PR #11 (discovery refresh), PR #12 (SIGHUP reload) |
| **L2 gate** | G3 (participant binary correct), G6 (determinism), G8 (no SEV-1 threats open) |
| **Threat IDs in scope** | T9, T11, T13, T15, T16 (cf. `docs/security/PHASE2_THREAT_MODEL.md`) |
| **CVSS bandwidth** | All ≤ SEV-3 ; no new SEV-1 introduced |

---

## Table of contents

  0. TL;DR
  1. Goal + non-goals
  2. Pre-flight checks
  3. Branch + commit plan
  4. Architectural decisions
  5. Wire diagram + sequence walkthroughs
  6. Crate layout additions
  7. `participant/local.rs` — Phase-1 wrapper (full design)
  8. `participant/validate.rs` — request validation
  9. `participant/state.rs` — extended state
 10. `bin/participant.rs` — boot + manifest loading
 11. `participant/service.rs` — replace stubs
 12. `Cargo.toml` updates
 13. Determinism, cancellation, timeout
 14. `step_index` + `reset_gen` tracking
 15. Error mapping (Phase-1 → gRPC)
 16. `Capabilities` response (real values)
 17. Concurrency model + lock hierarchy
 18. Threat-model coverage
 19. Tests strategy (unit + integration + e2e)
 20. Acceptance criteria (AC1–AC20)
 21. Risks + mitigations
 22. Rollback strategy
 23. Sequence diagrams (ASCII)
 24. Reviewer checklist
 25. Migration notes (operator-facing)
 26. Open questions
 27. Appendix : full file inventory

---

## 0. TL;DR

PR #9 wires the **participant** half of the Phase-2 control plane. After PR #8
the *controller* can call `MetaParticipantService::Step` / `Reset` / `State` on a
remote endpoint, but the production participant binary still returns
`UNIMPLEMENTED` for those RPCs. The integration smoke in `tests/integration_iterate_inproc.rs`
papered over this with an inline `TestParticipantSvc`. PR #9 replaces the stubs
with the production wire :

  * A new `participant::local::LocalParticipant` struct wraps a Phase-1
    `MockParticipant` (`with_contraction(id, dim, kappa, seed)`) behind the
    [`SwarmParticipant`] trait, plus a `step_index_last: AtomicU64` and a
    `reset_gen: AtomicU64`.
  * `participant::service::ParticipantService` delegates `step` → `reset` →
    `state` to that wrapper, with input validation and a `spawn_blocking`
    bridge for the sync Phase-1 method.
  * `bin/participant.rs` loads the same manifest the controller loads,
    finds its own row by `EFE_META_SWARM_PARTICIPANT_ID`, and constructs the
    wrapper with the manifest's kappa + seed.
  * Three integration tests exercise the wire end-to-end : a single-binary
    smoke (`integration_step_smoke.rs`), a controller-↔-participants e2e
    that replaces the `TestParticipantSvc` stand-in
    (`integration_e2e_iterate.rs`), and a determinism check (3 reruns must
    be bit-identical).

After PR #9, `cargo run --bin participant` is a real participant. The Kovant
scenario from § 19.2 of PR #8's architecture is reproducible against three
real binaries on `localhost`.

[`SwarmParticipant`]: ../../crates/efe-meta-swarm/src/participants/mod.rs

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Wire the participant binary's `Step` / `Reset` / `State` RPCs to a Phase-1
> `MockParticipant` whose kappa, dim, and seed come from the same manifest the
> controller reads, so a controller-↔-participant Iterate over real gRPC
> produces a converged, bit-identical result.

### 1.2 Non-goals

The following are **explicitly out of scope** for PR #9 :

| Item | Why deferred | Target PR |
|---|---|---|
| `StepStream` (server-streaming Step) | requires backpressure + flush protocol design | PR #21 |
| Real `LocalParticipant` from `efe-meta-swarm::participants::local` (sidecar) | needs the on-disk weight loader contract finalised | PR #14 |
| Multi-tenant participant (one binary, many tenants) | scope creep ; Phase 2 is T2 single-tenant | Phase 3 |
| `Capabilities.descriptor.build_sha` chain-of-custody (cosign attestation) | needs the keychain setup | PR #16 |
| mTLS + JWT auth on the participant side | needs the cert provisioning pipeline | PR #16 |
| Per-participant rate limit (MAX_STEPS_PER_SEC) | controller-side MAX_INFLIGHT already caps the rate | Phase 3 follow-up |
| Persisted state across restarts | requires sidecar + sled or rocksdb integration | PR #14 (Real Bundles) |
| Reload on SIGHUP | needs swap protocol with in-flight Steps | PR #12 |
| `Capabilities` schema version negotiation | not necessary while there is one schema | PR #11 |
| Per-tenant authorisation policy | Phase 3 | PR #20 |
| Streaming back-pressure semantics for `StepStream` | needs explicit contract | PR #21 |

### 1.3 Single binary unit of merge-readiness

After PR #9 merges, **the participant binary alone is shippable**. The
controller binary remains shippable from PR #8. The two together produce a
correct E2E flow over real gRPC on `localhost`. Production deployments still
wait for PR #10 (deployment manifests) and PR #16 (auth).

---

## 2. Pre-flight checks

Before opening the PR :

| # | Check | Verifier | Pass criterion |
|---|---|---|---|
| PF1 | `cargo build -p efe-meta-swarm-server --release` succeeds on `main` | CI | green |
| PF2 | `cargo test -p efe-meta-swarm-server --offline --all-targets` is **76 + 0 + 0 + 3 + 3 = 82** tests on `main` (PR #8 baseline) | CI | green |
| PF3 | PR #8 smoke `integration_iterate_inproc.rs` passes with `TestParticipantSvc` | CI | green |
| PF4 | `docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md` is the most recent green commit | manual | green |
| PF5 | The threat model (PR #7) is in its v1.0 frozen state | manual | sec sign-off |
| PF6 | Phase-1 `MockParticipant::with_contraction` is part of the published surface | grep | `crates/efe-meta-swarm/src/participants/mock.rs` ≥ line 64 |
| PF7 | `efe-meta-swarm` crate exposes `participants::SwarmParticipant` publicly | grep | `pub mod participants` in `lib.rs` line 90 |
| PF8 | `efe-meta-swarm-proto::meta_swarm::ResetParticipantRequest.initial : TensorF32` field exists | proto | `messages.proto` ≥ line 249 |

Any failed pre-flight blocks PR #9.

---

## 3. Branch + commit plan

### 3.1 Branch

```
efe-a1/p2-pr09-participant-step
```

Branched from `main` after PR #8 merge SHA. **Never** rebase onto a moving
`main` ; cherry-pick the PR #8 head if needed.

### 3.2 Commit-by-commit plan (5 commits, each CI-green)

| # | Subject | Files | LOC ± | Verification |
|---|---|---|---|---|
| C1 | `feat(participant): LocalParticipant wrapper around Phase-1 MockParticipant` | `+ src/participant/local.rs` | +250 | unit tests in module (≥ 6) ; `cargo test -p efe-meta-swarm-server --lib participant::local` |
| C2 | `feat(participant): request validators (StepRequest, ResetParticipantRequest)` | `+ src/participant/validate.rs` | +180 | unit tests in module (≥ 8) ; covers dim mismatch, NaN, missing field |
| C3 | `feat(participant): ParticipantState holds LocalParticipant + step counters` | `~ src/participant/state.rs`, `~ src/participant/mod.rs` | +60 / −10 | existing tests pass + new `descriptor_uses_manifest_values` ; `Capabilities` snapshot reflects real dim/kappa/seed |
| C4 | `feat(participant): bin loads manifest, finds own row, builds state` | `~ src/bin/participant.rs`, `~ src/manifest/mod.rs` | +80 / −5 | binary boot test : missing manifest exits 1 ; wrong id exits 1 ; happy path serves |
| C5 | `feat(participant): wire Step / Reset / State in service.rs + integration smoke` | `~ src/participant/service.rs`, `+ tests/integration_step_smoke.rs`, `+ tests/integration_e2e_iterate.rs`, `~ tests/integration_iterate_inproc.rs` | +700 / −80 | smoke : 1 binary in-process → Step round-trip ; e2e : controller + 3 participants → Iterate converges |

Total : **+1270 / −95 = +1175 LOC net**, including ~430 LOC of tests.

### 3.3 Drafting strategy

Each commit is **green-on-merge**. Commits C1-C3 are pure addition (no
behaviour change to existing code) ; C4 changes the binary's startup
sequence ; C5 flips the three RPC bodies. If C5 is reverted, the binary still
serves `Health` and `Capabilities` correctly — only Step / Reset / State go
back to `UNIMPLEMENTED`.

---

## 4. Architectural decisions

| AD | Decision | Rationale | Alternatives considered | Reversibility |
|---|---|---|---|---|
| **AD1** | `LocalParticipant` wraps **`MockParticipant`** (not `participants::local::LocalParticipant`) | `MockParticipant::with_contraction` is the smallest reproducible-and-deterministic Phase-1 type ; the sidecar `LocalParticipant` carries weight-file dependencies we want to defer to PR #14 | Wrap real `LocalParticipant` ; build a new sidecar struct ; embed Phase-1 directly | Reversible — `LocalParticipant` is an internal struct ; swap the wrapped type in PR #14 |
| **AD2** | Participant reads the **same manifest YAML** as the controller, **find its own row by `EFE_META_SWARM_PARTICIPANT_ID`** | Single source of truth, the L2 graduation soak protocol already requires this | Per-participant YAML ; controller pushes config via gRPC ; CLI flags | Reversible — manifest format owned by us |
| **AD3** | Sync ↔ async bridge via **`tokio::task::spawn_blocking` inside the async handler** | Phase-1 `step_with_pull` is sync ; running it on the async runtime worker would block the executor ; spawn_blocking is the idiomatic pattern (mirror of PR #8) | `tokio::task::block_in_place` ; native sync handler via `tonic::async_trait` workaround | Reversible — switch to `block_in_place` if rayon contention shows up |
| **AD4** | **No per-participant MAX_INFLIGHT semaphore** | Controller already gates Iterate (MAX_INFLIGHT = 10) and Iterate fans out to all participants serially per iteration ; a participant cannot see > 10 concurrent Step calls in Phase 2 | Add a `participant::inflight::InflightSemaphore` | Reversible — easy to add when Phase 3 multi-tenant arrives |
| **AD5** | **State held in `Arc<MockParticipant>`** (no outer `Mutex`) | `MockParticipant.state` is already `Mutex<Tensor>` internally ; double-wrapping would deadlock | Outer `Mutex<MockParticipant>` ; `RwLock` | Reversible |
| **AD6** | **`step_index_last : AtomicU64`** — server-side counter, reflects the last `pulled.step_index` received | Cheap, lock-free, returned in `StateResponse` | `Mutex<u64>` ; ignore the wire field | Reversible |
| **AD7** | **`reset_gen : AtomicU64`** — incremented on every `Reset`, returned in `ResetParticipantResponse` | Lets the controller coordinate with PR #12 SIGHUP reloads ; lock-free | Skip ; return constant 0 | Reversible |
| **AD8** | **No streaming `StepStream` in PR #9** | Wire and back-pressure semantics not yet finalised | Implement now | Reversible (only impls a trait method ; current returns `UNIMPLEMENTED`) |
| **AD9** | **`local_residual` self-report computed from `‖new − old‖₂` on the participant** | Free signal for the controller's bench HTML (PR #24) ; deterministic if the math is | Skip ; controller computes it from `final_states` | Reversible — toggle in `service.rs` |
| **AD10** | **Two separate integration tests** (`integration_step_smoke.rs` for the single-binary path, `integration_e2e_iterate.rs` for controller-↔-participants) | One isolates the participant alone (no controller dependency) ; the other proves the wire across the seam | One combined file ; spawn the real binary in-process | Reversible |

---

## 5. Wire diagram + sequence walkthroughs

### 5.1 Topology after PR #9

```
                ┌────────────────────────────────────────┐
                │ Caller (efe-server, bench harness, …)  │
                └────────────────┬───────────────────────┘
                                 │ Iterate(req)
                                 ▼
                ┌────────────────────────────────────────┐
                │ Controller                             │
                │   • MetaSwarmService::Iterate           │
                │   • run_iterate()                       │
                │   • RemoteParticipant × N (PR #8)       │
                └────┬────────────┬────────────┬─────────┘
                     │            │            │  Step(pulled, step_idx)
                     ▼            ▼            ▼
        ┌────────────────────┐ ┌─────────┐ ┌─────────┐
        │ Participant #1     │ │ #2      │ │ #3      │   ← PR #9
        │  • Step → Mock     │ │  …      │ │  …      │
        │  • Reset           │ └─────────┘ └─────────┘
        │  • State           │
        │  • Health (PR #6)  │
        │  • Capabilities    │
        └────────────────────┘
```

PR #9 owns everything inside the *Participant* box.

### 5.2 Step round-trip (success path)

```
Controller                                Participant
   |  Step(pulled, step_idx=t)                |
   |─────────────────────────────────────────>|
   |                                          | spawn_blocking
   |                                          |  ├ validate(pulled)
   |                                          |  ├ MockParticipant
   |                                          |  │   .step_with_pull(
   |                                          |  │     pulled, γ, α)
   |                                          |  └ return new_state
   |                                          |
   |  StepResponse(new_state, local_residual, |
   |               step_us)                   |
   |<─────────────────────────────────────────|
   |                                          |
```

Latency budget on `localhost` : ≤ 5 ms p99 for `dim ≤ 512`. Verified in
`integration_step_smoke.rs` (`step_latency_under_5ms`).

### 5.3 Reset round-trip

```
Controller                                Participant
   |  ResetParticipantRequest(initial=None)   |
   |─────────────────────────────────────────>|
   |                                          | spawn_blocking
   |                                          |  ├ MockParticipant.reset()
   |                                          |  ├ reset_gen.fetch_add(1)
   |                                          |  └ return ok
   |                                          |
   |  ResetParticipantResponse(reset_gen=n+1) |
   |<─────────────────────────────────────────|
```

If `initial` is non-empty :

```
   |  ResetParticipantRequest(initial=Some(t))|
   |─────────────────────────────────────────>|
   |                                          | spawn_blocking
   |                                          |  ├ validate(initial)
   |                                          |  ├ MockParticipant.set_state(t)
   |                                          |  ├ reset_gen.fetch_add(1)
   |                                          |  └ return ok
```

### 5.4 State round-trip

```
Controller                                Participant
   |  StateRequest                            |
   |─────────────────────────────────────────>|
   |                                          | mock.current_state()
   |                                          |  (lock_state.read(),
   |                                          |   clone Tensor)
   |  StateResponse(current,                  |
   |               step_index_last)           |
   |<─────────────────────────────────────────|
```

State is cheap : a `Tensor` clone over a localhost gRPC frame.

### 5.5 Health round-trip (unchanged from PR #6)

```
Controller                                Participant
   |  HealthRequest                           |
   |─────────────────────────────────────────>|
   |                                          | health_state.snapshot()
   |  HealthResponse(status=SERVING,          |
   |                 inflight=0,              |
   |                 reload_gen=0)            |
   |<─────────────────────────────────────────|
```

PR #9 does **not** modify the participant's `Health` ; it remains correct from
PR #6.

---

## 6. Crate layout additions

### 6.1 Before PR #9

```
crates/efe-meta-swarm-server/src/participant/
├── mod.rs
├── service.rs
└── state.rs
```

### 6.2 After PR #9

```
crates/efe-meta-swarm-server/src/participant/
├── mod.rs              [MODIFIED : re-export LocalParticipant, validate]
├── service.rs          [MODIFIED : wire step/reset/state]
├── state.rs            [MODIFIED : hold LocalParticipant, expose descriptor]
├── local.rs            [NEW : Phase-1 wrapper, ~250 LOC]
└── validate.rs         [NEW : request validation, ~180 LOC]

crates/efe-meta-swarm-server/tests/
├── integration_health_smoke.rs       [unchanged from PR #8]
├── integration_iterate_inproc.rs     [MODIFIED : remove TestParticipantSvc, use real ParticipantService]
├── integration_step_smoke.rs         [NEW : single-binary Step round-trip, ~200 LOC]
└── integration_e2e_iterate.rs        [NEW : controller + 3 real participants, ~300 LOC]

crates/efe-meta-swarm-server/src/bin/
└── participant.rs      [MODIFIED : load manifest at startup, build state with real values]

crates/efe-meta-swarm-server/src/manifest/
└── mod.rs              [MODIFIED : add `find_participant(id)` helper, ~20 LOC]
```

No new external dependencies. `efe-meta-swarm` (added in PR #8) already exposes
`MockParticipant`.

---

## 7. `participant/local.rs` — Phase-1 wrapper (full design)

### 7.1 Purpose

`LocalParticipant` is the production stand-in for the sidecar that PR #14 will
deliver. In PR #9 it wraps Phase-1's `MockParticipant::with_contraction`, plus
a `step_index_last : AtomicU64` and a `reset_gen : AtomicU64`.

### 7.2 Public surface

```rust
//! Production participant adapter — wraps a Phase-1 `MockParticipant`
//! and exposes the gRPC-friendly accessors `ParticipantService` needs.
//!
//! In PR #14 this will be replaced by a sidecar-backed
//! `LocalParticipant` reading weights from disk.  Until then,
//! `MockParticipant::with_contraction(id, dim, kappa, seed)` is the
//! deterministic equivalent.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use efe_meta_swarm::participants::{MockParticipant, SwarmParticipant};
use efe_meta_swarm::prelude::Tensor;
use efe_meta_swarm::MetaSwarmError;

use crate::manifest::ParticipantManifest;

/// Production participant.  Cheap to clone (Arcs inside).
#[derive(Clone)]
pub struct LocalParticipant {
    inner: Arc<LocalParticipantInner>,
}

struct LocalParticipantInner {
    id: String,
    dim: usize,
    kappa: f32,
    seed: u64,
    /// Phase-1 in-process participant.  Holds `state: Mutex<Tensor>`.
    mock: MockParticipant,
    /// `pulled.step_index` from the most recent successful Step.
    step_index_last: AtomicU64,
    /// Bumped on every successful Reset ; returned in
    /// `ResetParticipantResponse.reset_gen`.
    reset_gen: AtomicU64,
}

impl LocalParticipant {
    /// Build from a [`ParticipantManifest`] row.  Returns Err if
    /// `MockParticipant::with_contraction` rejects the kappa range.
    pub fn from_manifest_row(row: &ParticipantManifest)
        -> Result<Self, MetaSwarmError>
    {
        let mock = MockParticipant::with_contraction(
            row.id.clone(),
            row.dim as usize,
            row.kappa,
            row.seed,
        )?;
        Ok(Self {
            inner: Arc::new(LocalParticipantInner {
                id: row.id.clone(),
                dim: row.dim as usize,
                kappa: row.kappa,
                seed: row.seed,
                mock,
                step_index_last: AtomicU64::new(0),
                reset_gen: AtomicU64::new(0),
            }),
        })
    }

    #[must_use] pub fn id(&self) -> &str { &self.inner.id }
    #[must_use] pub fn dim(&self) -> usize { self.inner.dim }
    #[must_use] pub fn kappa(&self) -> f32 { self.inner.kappa }
    #[must_use] pub fn seed(&self) -> u64 { self.inner.seed }

    #[must_use]
    pub fn step_index_last(&self) -> u64 {
        self.inner.step_index_last.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn reset_gen(&self) -> u64 {
        self.inner.reset_gen.load(Ordering::Relaxed)
    }

    /// Apply one coupled iteration step.  `gamma` and `alpha` come
    /// from the manifest's `iteration` block (NOT from the wire — see
    /// PR #8 § "Phase-2 simplification" for the rationale).
    ///
    /// SAFETY : MUST be called from `spawn_blocking` (Phase-1 is sync).
    pub fn step(
        &self,
        pulled: &Tensor,
        gamma: f64,
        alpha: f64,
        step_index: u32,
    ) -> Result<Tensor, MetaSwarmError> {
        let new_state = self.inner.mock.step_with_pull(pulled, gamma, alpha)?;
        // Record only on success ; failed steps don't bump the counter.
        self.inner
            .step_index_last
            .store(u64::from(step_index), Ordering::Relaxed);
        Ok(new_state)
    }

    /// Read current state without mutating.
    pub fn current_state(&self) -> Result<Tensor, MetaSwarmError> {
        self.inner.mock.current_state()
    }

    /// Reset to the seeded initial state (if `initial` is `None`) or
    /// to the provided tensor (if `Some`).  Returns the new
    /// `reset_gen` value.
    pub fn reset(&self, initial: Option<&Tensor>) -> Result<u64, MetaSwarmError> {
        if let Some(t) = initial {
            self.inner.mock.set_state(t.clone())?;
        } else {
            self.inner.mock.reset()?;
        }
        let new_gen = self.inner.reset_gen.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(new_gen)
    }
}

impl std::fmt::Debug for LocalParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalParticipant")
            .field("id", &self.inner.id)
            .field("dim", &self.inner.dim)
            .field("kappa", &self.inner.kappa)
            .field("seed", &self.inner.seed)
            .field("step_index_last", &self.step_index_last())
            .field("reset_gen", &self.reset_gen())
            .finish_non_exhaustive()
    }
}
```

### 7.3 Why this design

  * **`Arc<LocalParticipantInner>`** — cheap clone, used by both the gRPC
    handler future and the `spawn_blocking` closure.
  * **`MockParticipant` *not* wrapped in another `Mutex`** — its internal
    `state: Mutex<Tensor>` already serialises Step / Reset / State across
    callers (AD5).
  * **`AtomicU64` for `step_index_last` and `reset_gen`** — lock-free, safe
    across rayon workers and tonic handlers, and the gRPC handler can
    snapshot them without touching the participant's state Mutex (AD6, AD7).
  * **`gamma` + `alpha` from the manifest, not the wire** — mirror of PR #8 §
    "Phase-2 simplification". The two binaries must read the same manifest
    version, enforced by the L2 graduation soak protocol.

### 7.4 Unit tests (≥ 6 in module)

  1. `from_manifest_row_happy_path` — builds a valid LocalParticipant.
  2. `from_manifest_row_rejects_kappa_out_of_range` — kappa = 1.5 → Err.
  3. `from_manifest_row_rejects_zero_dim` — dim = 0 → Err.
  4. `step_bumps_step_index_last` — Step(idx=7) then `step_index_last() == 7`.
  5. `step_failure_does_not_bump_index` — bad pull dim → Err, idx unchanged.
  6. `reset_default_bumps_reset_gen` — reset() twice → `reset_gen() == 2`.
  7. `reset_with_initial_validates_dim` — initial.dim != participant.dim → Err.
  8. `current_state_after_reset_matches_seed` — Reset → current_state == seeded.

### 7.5 Compile-time invariants

  * `LocalParticipant : Send + Sync` (verified by `assert_send_sync` helper in
    tests) — required to be held inside `ParticipantState`, which is `Clone +
    Sync`.
  * `LocalParticipant : Clone` — multiple gRPC handlers may hold a clone.
  * No `unsafe` ; `#![forbid(unsafe_code)]` is set in `lib.rs`.

---

## 8. `participant/validate.rs` — request validation

### 8.1 Purpose

Catch malformed `StepRequest` / `ResetParticipantRequest` *before* they reach
Phase-1. This is **defence in depth** — Phase-1's `DimensionMismatch` would
catch most of these, but :

  * tonic translates Phase-1's `DimensionMismatch` to `INTERNAL` (per
    PR #8's `meta_swarm_error_to_status` mapping) — we want
    `FAILED_PRECONDITION` for caller-fault inputs.
  * Phase-1's `Tensor::from_slice` happily accepts NaN values — we reject
    them at the boundary.
  * The wire field `pulled.dim` must equal `pulled.data.len()` ; that's not
    Phase-1's concern.

### 8.2 Public surface

```rust
//! Request validators for the participant's gRPC handlers.

use efe_meta_swarm_proto::meta_swarm::{
    ResetParticipantRequest, StepRequest, TensorF32,
};

use crate::error::ServerError;
use crate::participant::local::LocalParticipant;

/// Validate a `StepRequest` against the local participant's expected
/// dimensions.  Returns the validated `(pulled_tensor_proto, step_index)`
/// tuple ready for `LocalParticipant::step`.
pub fn validate_step(
    req: &StepRequest,
    local: &LocalParticipant,
) -> Result<(&TensorF32, u32), ServerError> {
    let pulled = req
        .pulled_from_meta
        .as_ref()
        .ok_or_else(|| ServerError::Config(
            "StepRequest.pulled_from_meta is required".into(),
        ))?;
    validate_tensor_shape(pulled, local.dim())?;
    validate_tensor_finite(pulled)?;
    Ok((pulled, req.step_index))
}

/// Validate a `ResetParticipantRequest`.  Returns `Some(tensor)` if a
/// custom initial was provided (validated for shape + finite), or
/// `None` for the default seeded reset.
pub fn validate_reset<'a>(
    req: &'a ResetParticipantRequest,
    local: &LocalParticipant,
) -> Result<Option<&'a TensorF32>, ServerError> {
    let Some(initial) = req.initial.as_ref() else {
        return Ok(None);
    };
    validate_tensor_shape(initial, local.dim())?;
    validate_tensor_finite(initial)?;
    Ok(Some(initial))
}

fn validate_tensor_shape(t: &TensorF32, expected_dim: usize) -> Result<(), ServerError> {
    if t.dim as usize != expected_dim {
        return Err(ServerError::Config(format!(
            "TensorF32.dim = {} but participant expects {}",
            t.dim, expected_dim,
        )));
    }
    if t.dim as usize != t.data.len() {
        return Err(ServerError::Config(format!(
            "TensorF32 : dim = {} but data.len() = {} (envelope corrupt)",
            t.dim, t.data.len(),
        )));
    }
    Ok(())
}

fn validate_tensor_finite(t: &TensorF32) -> Result<(), ServerError> {
    if let Some((i, v)) = t.data.iter().enumerate().find(|(_, v)| !v.is_finite()) {
        return Err(ServerError::Config(format!(
            "TensorF32.data[{i}] = {v} is not finite (NaN or ±∞)",
        )));
    }
    Ok(())
}
```

### 8.3 Unit tests (≥ 8 in module)

  1. `validate_step_happy_path` — well-formed request → Ok.
  2. `validate_step_rejects_missing_pulled` — pulled = None → FailedPrecondition.
  3. `validate_step_rejects_dim_mismatch` — pulled.dim ≠ local.dim → Err.
  4. `validate_step_rejects_envelope_corruption` — dim ≠ data.len() → Err.
  5. `validate_step_rejects_nan` — pulled.data[0] = NaN → Err.
  6. `validate_step_rejects_positive_infinity` — pulled.data[3] = +inf → Err.
  7. `validate_reset_no_initial_returns_none` — initial = None → Ok(None).
  8. `validate_reset_with_initial_validates_shape` — initial.dim mismatch → Err.
  9. `validate_reset_with_initial_rejects_nan` — initial.data[2] = NaN → Err.

### 8.4 Why these rules

| Rule | Threat | gRPC code |
|---|---|---|
| `pulled_from_meta` present | T9 (state poisoning by empty pull) | `FAILED_PRECONDITION` |
| `dim == expected_dim` | T8 (dim mismatch from misaligned manifest) | `FAILED_PRECONDITION` |
| `dim == data.len()` | T9 (truncated / padded envelope) | `FAILED_PRECONDITION` |
| `is_finite()` per element | T11 (silent NaN propagation poisons every future Step) | `FAILED_PRECONDITION` |

NaN at the participant boundary is **fatal**. If we let it through, the
participant's `state` becomes NaN, and every subsequent Step returns NaN,
silently breaking convergence for the rest of the controller's iteration. The
controller's residual check would notice, but the iteration counts as a
"safety_fail" in the wire enum — a confusing diagnostic.

---

## 9. `participant/state.rs` — extended state

### 9.1 Before PR #9

```rust
#[derive(Clone)]
pub struct ParticipantState {
    pub config: Arc<ParticipantConfig>,
    pub health: HealthState,
    pub metrics: MetricsRegistry,
    pub start_time: Instant,
}
```

The `descriptor()` helper currently returns placeholders (dim/kappa/seed = 0).

### 9.2 After PR #9

```rust
//! Shared participant state.

use std::sync::Arc;
use std::time::Instant;

use efe_meta_swarm_proto::meta_swarm::ParticipantDescriptor;

use crate::config::participant::ParticipantConfig;
use crate::observability::health::HealthState;
use crate::observability::metrics::MetricsRegistry;
use crate::participant::local::LocalParticipant;

/// Shared participant state.  Cheap to clone (`Arc`s inside).
///
/// # Lock hierarchy
///
/// Only one lock in play : the `Mutex<Tensor>` inside `LocalParticipant`'s
/// wrapped `MockParticipant`.  All accessors hold it briefly during the
/// sync Phase-1 call.  No nested locks — no deadlock risk.
#[derive(Clone)]
pub struct ParticipantState {
    pub config: Arc<ParticipantConfig>,
    pub health: HealthState,
    pub metrics: MetricsRegistry,
    pub start_time: Instant,
    /// Phase-1 in-process participant.  Holds the mutable tensor
    /// state.
    pub local: LocalParticipant,
    /// `iteration` block from the manifest (alpha_default, coupling_gamma).
    /// Snapshot at boot ; PR #12 will swap it on SIGHUP.
    pub iteration: Arc<IterationParams>,
}

/// Iteration knobs the participant applies inside `step_with_pull`.
#[derive(Debug, Clone)]
pub struct IterationParams {
    pub alpha_default: f64,
    pub coupling_gamma: f64,
}

impl ParticipantState {
    /// Build from a fully-resolved config + manifest row + iteration block.
    #[must_use]
    pub fn new(
        config: ParticipantConfig,
        local: LocalParticipant,
        iteration: IterationParams,
    ) -> Self {
        Self {
            config: Arc::new(config),
            health: HealthState::serving(),
            metrics: MetricsRegistry::new(),
            start_time: Instant::now(),
            local,
            iteration: Arc::new(iteration),
        }
    }

    /// Pre-built descriptor.  PR #9 fills in real values from the
    /// participant manifest row.
    pub fn descriptor(&self) -> ParticipantDescriptor {
        ParticipantDescriptor {
            id: self.local.id().to_string(),
            dim: u32::try_from(self.local.dim()).unwrap_or(u32::MAX),
            kappa: self.local.kappa(),
            seed: self.local.seed(),
            build_sha: crate::GIT_SHA.into(),
            blas_backend: "none".into(),
            features: vec!["determinism".into(), "step_index".into()],
        }
    }
}

impl std::fmt::Debug for ParticipantState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParticipantState")
            .field("config", &self.config)
            .field("local", &self.local)
            .field("iteration", &self.iteration)
            .field("uptime_s", &self.start_time.elapsed().as_secs())
            .finish_non_exhaustive()
    }
}
```

### 9.3 Test coverage delta

  * Existing `participant::state::tests::descriptor_*` tests are updated to
    construct a `ParticipantState` via `new(config, local, iteration)` and
    assert the descriptor's real values.
  * New test : `descriptor_id_dim_kappa_match_manifest_row` — verifies the
    descriptor wiring against a real `LocalParticipant`.

### 9.4 Why `IterationParams` here

`step_with_pull(pulled, γ, α)` needs `γ` and `α`. They come from the
manifest's `iteration` block (NOT from the wire). Stashing them on the state
:

  * Avoids re-reading the manifest on every Step (cheap, but pollutes the hot
    path with a `RwLock::try_read()`).
  * Sets up PR #12's SIGHUP path : a future `Arc::swap` updates them
    atomically.

---

## 10. `bin/participant.rs` — boot + manifest loading

### 10.1 Before PR #9

```rust
let state = ParticipantState::new(config.clone());
```

Just builds a placeholder state. No manifest involvement.

### 10.2 After PR #9

The boot sequence becomes :

```rust
async fn async_main(
    config: ParticipantConfig,
) -> Result<(), ServerError> {
    info!(/* boot banner */);

    // 1. Load + validate the manifest.  Same path the controller reads.
    let manifest_path = std::env::var_os("EFE_META_SWARM_MANIFEST")
        .map_or_else(
            || std::path::PathBuf::from("/etc/efe-meta-swarm/manifest.yaml"),
            std::path::PathBuf::from,
        );
    let manifest = match manifest::load_and_validate(&manifest_path) {
        Ok(m) => m,
        Err(e) => {
            error!(error = %e, manifest_path = %manifest_path.display(),
                "manifest load/validate failed");
            return Err(e);
        }
    };

    // 2. Find this participant's row.
    let row = match manifest::find_participant(&manifest, &config.id) {
        Some(r) => r,
        None => {
            error!(id = %config.id,
                "participant id not found in manifest");
            return Err(ServerError::Config(format!(
                "participant id {:?} not in manifest", config.id,
            )));
        }
    };

    // 3. Build the in-process participant.
    let local = participant::local::LocalParticipant::from_manifest_row(row)
        .map_err(|e| ServerError::Config(format!(
            "LocalParticipant build failed for {:?} : {e}", config.id,
        )))?;

    info!(
        id = %config.id,
        dim = local.dim(),
        kappa = local.kappa(),
        seed = local.seed(),
        "participant loaded from manifest row",
    );

    // 4. Snapshot the iteration block.
    let iteration = participant::state::IterationParams {
        alpha_default: manifest.iteration.alpha_default,
        coupling_gamma: manifest.iteration.coupling_gamma,
    };

    // 5. Build state + service.
    let state = ParticipantState::new(config.clone(), local, iteration);
    let svc = ParticipantService::new(state.clone());

    /* …unchanged from PR #6 : metrics exporter, gRPC server,
       shutdown signal, drain timeout… */
}
```

### 10.3 New helper : `manifest::find_participant`

```rust
/// Find the participant row for the given id.  `None` if absent.
#[must_use]
pub fn find_participant<'a>(
    m: &'a Manifest,
    id: &str,
) -> Option<&'a schema::ParticipantManifest> {
    m.participants.iter().find(|p| p.id == id)
}
```

≤ 20 LOC, tests in the same `manifest::tests` module :

  * `find_participant_returns_some` — id present.
  * `find_participant_returns_none` — id absent.
  * `find_participant_handles_unicode_id` — id with non-ASCII characters.

### 10.4 Failure modes at boot

| Mode | Exit code | Error log |
|---|---|---|
| Env var parse fails | 2 (config) | `participant config error: {e}` |
| Manifest path missing / unreadable | 1 (runtime) | `manifest load/validate failed` |
| Manifest YAML malformed | 1 | same as above |
| Manifest validation fails | 1 | same as above |
| Manifest does not contain `config.id` | 1 | `participant id not found in manifest` |
| `MockParticipant::with_contraction` rejects kappa | 1 | `LocalParticipant build failed` |
| gRPC bind fails | 1 (logged by serve task) | `gRPC server failed` (binary stays alive until SIGTERM ; cf. § 12 of PR #6 architecture) |
| Tokio runtime build fails | 2 | `tokio runtime build error` |

---

## 11. `participant/service.rs` — replace stubs

### 11.1 Top-level skeleton

```rust
//! `MetaParticipantService` impl.
//!
//! PR #6 returned UNIMPLEMENTED for every RPC except `Health` and
//! `Capabilities`.  PR #9 wires :
//!   * `Step`  — `LocalParticipant::step` via `spawn_blocking`.
//!   * `Reset` — `LocalParticipant::reset` via `spawn_blocking`.
//!   * `State` — `LocalParticipant::current_state` via `spawn_blocking`.
//! `StepStream` remains `UNIMPLEMENTED` (PR #21).

use std::time::Instant;

use efe_meta_swarm_proto::meta_swarm::{
    CapabilitiesRequest, CapabilitiesResponse, HealthRequest, HealthResponse,
    ResetParticipantRequest, ResetParticipantResponse, StateRequest, StateResponse,
    StepRequest, StepResponse,
};
use tonic::{Request, Response, Status};
use tracing::{debug, instrument, warn};

use super::state::ParticipantState;
use super::validate;
use crate::client::{proto_to_tensor, tensor_to_proto};
use crate::error::ServerError;
use crate::proto::MetaParticipantService;

#[derive(Debug, Clone)]
pub struct ParticipantService {
    pub state: ParticipantState,
}

impl ParticipantService {
    #[must_use]
    pub fn new(state: ParticipantState) -> Self { Self { state } }
}
```

### 11.2 `Step` handler

```rust
#[instrument(skip(self, req), fields(participant_id = %self.state.local.id()))]
async fn step(
    &self,
    req: Request<StepRequest>,
) -> Result<Response<StepResponse>, Status> {
    let start = Instant::now();
    let req_inner = req.into_inner();

    // 1. Validate at the boundary (defence in depth ; Phase-1 also checks).
    let (pulled_proto, step_index) =
        validate::validate_step(&req_inner, &self.state.local)
            .map_err(Status::from)?;

    // 2. Convert proto → Phase-1 Tensor.
    let pulled_tensor = proto_to_tensor(pulled_proto)
        .map_err(|e| Status::failed_precondition(e.to_string()))?;

    // 3. Run the sync Phase-1 step on the blocking pool.
    let local = self.state.local.clone();
    let gamma = self.state.iteration.coupling_gamma;
    let alpha = self.state.iteration.alpha_default;
    let join = tokio::task::spawn_blocking(move || {
        local.step(&pulled_tensor, gamma, alpha, step_index)
    });

    let new_state = match join.await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(super::error_to_status(&e)),
        Err(e) => {
            warn!(error = ?e, "step panicked");
            return Err(Status::internal(format!("step panicked: {e}")));
        }
    };

    // 4. Optional residual self-report (AD9).
    let local_residual = compute_residual(pulled_proto, &new_state);
    let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);

    debug!(
        step_index,
        elapsed_us,
        local_residual,
        "step completed",
    );

    Ok(Response::new(StepResponse {
        new_state: Some(tensor_to_proto(&new_state)),
        local_residual,
        step_us: elapsed_us,
    }))
}
```

`compute_residual` is a small helper :

```rust
/// Euclidean distance ‖new − pulled‖₂ in f64 to avoid intermediate
/// overflow on dim ≥ 10_000.  Returns 0 if dims disagree (caller
/// already validated, so this is purely defensive).
fn compute_residual(pulled: &TensorF32, new_state: &Tensor) -> f64 {
    if pulled.data.len() != new_state.dim() {
        return 0.0;
    }
    let new_slice = new_state.as_slice();
    let sum_sq: f64 = pulled
        .data
        .iter()
        .zip(new_slice.iter())
        .map(|(a, b)| {
            let d = f64::from(*a) - f64::from(*b);
            d * d
        })
        .sum();
    sum_sq.sqrt()
}
```

### 11.3 `Reset` handler

```rust
#[instrument(skip(self, req), fields(participant_id = %self.state.local.id()))]
async fn reset(
    &self,
    req: Request<ResetParticipantRequest>,
) -> Result<Response<ResetParticipantResponse>, Status> {
    let req_inner = req.into_inner();

    // 1. Validate.
    let initial_opt = validate::validate_reset(&req_inner, &self.state.local)
        .map_err(Status::from)?;

    // 2. Convert proto → Phase-1 Tensor (if any).
    let initial_tensor = initial_opt
        .map(|p| proto_to_tensor(p)
            .map_err(|e| Status::failed_precondition(e.to_string())))
        .transpose()?;

    // 3. Run reset on the blocking pool.
    let local = self.state.local.clone();
    let join = tokio::task::spawn_blocking(move || {
        local.reset(initial_tensor.as_ref())
    });

    let reset_gen = match join.await {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => return Err(super::error_to_status(&e)),
        Err(e) => return Err(Status::internal(format!("reset panicked: {e}"))),
    };

    Ok(Response::new(ResetParticipantResponse { reset_gen }))
}
```

### 11.4 `State` handler

```rust
#[instrument(skip(self, _req), fields(participant_id = %self.state.local.id()))]
async fn state(
    &self,
    _req: Request<StateRequest>,
) -> Result<Response<StateResponse>, Status> {
    let local = self.state.local.clone();
    let join = tokio::task::spawn_blocking(move || local.current_state());

    let tensor = match join.await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(super::error_to_status(&e)),
        Err(e) => return Err(Status::internal(format!("state panicked: {e}"))),
    };

    let step_index_last = u32::try_from(self.state.local.step_index_last())
        .unwrap_or(u32::MAX);

    Ok(Response::new(StateResponse {
        current: Some(tensor_to_proto(&tensor)),
        step_index_last,
    }))
}
```

### 11.5 `Capabilities` handler — minor change

Reuses `state.descriptor()` (now populated with real values) ; no logic change.

### 11.6 `Health` handler — unchanged from PR #6

### 11.7 `StepStream` handler — still `UNIMPLEMENTED`

```rust
async fn step_stream(/* … */) -> Result<…, Status> {
    Err(Status::unimplemented(
        "MetaParticipantService::StepStream — wired in PR #21",
    ))
}
```

### 11.8 Shared `error_to_status` helper

PR #8's `meta_swarm_error_to_status` lives in `controller/iterate.rs`. PR #9
needs the same mapping. Promote it to `participant::error_to_status` (or
factor out to `crate::error::meta_swarm_error_to_status`). The decision : put
the helper in `crate::error` so both sides share one source of truth.

```rust
// crate::error
pub fn meta_swarm_error_to_status(e: &efe_meta_swarm::MetaSwarmError) -> tonic::Status {
    use efe_meta_swarm::MetaSwarmError as E;
    use tonic::Status;
    match e {
        E::DimensionMismatch { .. }
        | E::InvalidCoupling { .. }
        | E::ParticipantNotFound { .. }
        | E::DuplicateParticipantId { .. }
        | E::EmptyMetaSwarm
        | E::MissingOperator { .. } => Status::failed_precondition(e.to_string()),
        _ => Status::internal(e.to_string()),
    }
}
```

PR #8's local copy is deleted ; the controller imports it from `crate::error`.

### 11.9 Existing service tests — what changes

`participant::service::tests` currently has three tests :

| Test | Before PR #9 | After PR #9 |
|---|---|---|
| `health_returns_serving` | passes | unchanged |
| `capabilities_returns_descriptor` | asserts placeholder id/dim=0/kappa=0 | asserts real values from a `LocalParticipant` built in `test_state()` |
| `step_returns_unimplemented_with_pr9_message` | passes | **DELETED** |

New tests :

  * `step_happy_path` — round-trip a Step with a valid pulled tensor.
  * `step_rejects_missing_pulled_from_meta` → FailedPrecondition.
  * `step_rejects_dim_mismatch` → FailedPrecondition.
  * `step_rejects_nan_in_pulled` → FailedPrecondition.
  * `reset_no_initial_bumps_reset_gen` — Reset twice → reset_gen = 2.
  * `reset_with_initial_validates_dim` → FailedPrecondition.
  * `state_returns_current_tensor` — Reset → State.current ≈ seeded vector.
  * `state_step_index_last_reflects_step_input` — Step(idx=7) → State.step_index_last = 7.
  * `step_stream_still_unimplemented_with_pr21_message` — guards the deferred surface.

### 11.10 New `test_state()` factory

```rust
#[cfg(test)]
fn test_state() -> ParticipantState {
    let config = ParticipantConfig { id: "p-1".into(), /* … */ };
    let row = crate::manifest::ParticipantManifest {
        id: "p-1".into(),
        dim: 8,
        kappa: 0.6,
        seed: 11,
        blas_backend: "none".into(),
    };
    let local = crate::participant::local::LocalParticipant::from_manifest_row(&row)
        .expect("LocalParticipant::from_manifest_row");
    let iteration = crate::participant::state::IterationParams {
        alpha_default: 0.5,
        coupling_gamma: 0.2,
    };
    ParticipantState::new(config, local, iteration)
}
```

---

## 12. `Cargo.toml` updates

PR #9 needs **no new external dependencies**. `efe-meta-swarm` (added by
PR #8) already exposes `MockParticipant` and `SwarmParticipant`.

The `[dev-dependencies]` block stays the same — `integration_step_smoke.rs`
and `integration_e2e_iterate.rs` use only what's already declared (`tokio`,
`tonic`, `tempfile`, `efe-meta-swarm-proto`).

---

## 13. Determinism, cancellation, timeout

### 13.1 Determinism contract

After PR #9 the *participant binary* upholds :

> Same seed × same sequence of `pulled` inputs (in identical bit-pattern) ×
> same `gamma` × same `alpha` ⇒ bit-identical sequence of `new_state` outputs
> across reruns.

Source of randomness :

  * Initial state — `MockParticipant::with_contraction` uses
    `Tensor::pseudo_random(dim, seed)`. Deterministic across runs (Phase-1
    contract).
  * Floating-point arithmetic — IEEE 754 with single-thread sums. No
    parallel reduction inside one Step.
  * No clocks ; no system entropy ; no OS-level randomness.

The integration test `integration_e2e_iterate.rs::iterate_bit_identical_3_reruns`
proves this against a real binary topology.

### 13.2 Cancellation

The participant receives cancellation via tonic's transport layer :

  * Client disconnects → tonic drops the handler future.
  * The handler's `spawn_blocking` continues until completion (Phase-1 is
    sync ; cannot be interrupted mid-operation).
  * The `JoinHandle` is dropped ; its result is discarded.
  * No leakage : `LocalParticipant` is `Arc`-cloned into the closure ;
    after the closure ends, its clone drops.

Worst-case "cancel observed" latency = "longest single `step_with_pull`
duration". For `dim ≤ 1024`, this is < 1 ms on a 2024 server-class CPU.
Phase-3 may introduce per-step cancellation by interleaving cancel polls into
Phase-1's hot loop ; PR #9 explicitly does not.

### 13.3 Timeout

tonic's request timeout (set per-call by the controller at
`step_deadline = 5 s` — cf. PR #8 § 5.4) is enforced server-side by tonic ;
when it expires, tonic returns `DEADLINE_EXCEEDED` to the client without
interrupting the in-flight `spawn_blocking`. The participant continues its
computation (no waste — the result is just thrown away).

A `step_with_pull` for `dim = 1024` takes ≤ 100 µs in practice ; the 5 s
timeout is **defence in depth**, not a primary control.

---

## 14. `step_index` + `reset_gen` tracking

### 14.1 Wire fields

  * `StepRequest.step_index : u32` — controller's iteration count, set by
    `iterate.rs` (currently 0 in PR #8 ; PR #18 will populate).
  * `StateResponse.step_index_last : u32` — last `step_index` the participant
    successfully processed.
  * `ResetParticipantResponse.reset_gen : u64` — server-side monotonic
    counter, bumped on every successful Reset.

### 14.2 Server-side counters

`LocalParticipantInner` carries `step_index_last : AtomicU64` and
`reset_gen : AtomicU64`. Both are :

  * **Lock-free** (atomics over `Relaxed` ordering — no inter-thread
    synchronisation needed, since the values are advisory).
  * **Monotonic** across the binary's lifetime (no reset on `Reset(initial)`).
  * **Survive within the process** but **not across restarts** (PR #14 may
    add persistence).

### 14.3 Why `Relaxed` ordering

The counters are **observed but not relied on for synchronisation**. The
controller reads them out-of-band ; no happens-before relationship needs to
be established between the writer and the reader. `Relaxed` is the cheapest
ordering that still gives integral consistency on x86_64 and aarch64.

If a future PR ties `step_index_last` to a safety check ("refuse Step if
step_index < step_index_last"), the ordering would need to be `AcqRel` and
the read would need to be paired. Out of scope for PR #9.

### 14.4 What `step_index` is NOT used for

  * **Not** a deduplication key — the controller does not retry Steps.
  * **Not** an ordering constraint — multiple concurrent Steps from the same
    controller (Phase-3 streaming) would race ; current Phase 2 is
    sequential per participant.
  * **Not** a clock — only the controller mints monotonic values.

PR #9 records it for tracing diagnostics. The bench HTML (PR #24) will plot
`step_index_last` per participant to detect "stuck" participants in a
running Iterate.

---

## 15. Error mapping (Phase-1 → gRPC)

### 15.1 Promotion of the shared helper

PR #8 defines `meta_swarm_error_to_status(&MetaSwarmError) -> Status` inside
`controller/iterate.rs`. PR #9 needs the same mapping. The architectural
decision (AD9 of this doc, sub-point 11.8) :

  * Move the helper to `crate::error::meta_swarm_error_to_status`.
  * Delete the local copy in `controller/iterate.rs` ; the controller
    imports it from `crate::error`.
  * The participant imports it from `crate::error`.

The mapping is the **canonical translation** between Phase-1 errors and gRPC
status codes for the whole crate.

### 15.2 Mapping table

| Phase-1 variant | gRPC code | Rationale |
|---|---|---|
| `DimensionMismatch` | `FAILED_PRECONDITION` | Caller-fault (request shape) |
| `InvalidCoupling` | `FAILED_PRECONDITION` | Caller-fault (params) |
| `ParticipantNotFound` | `FAILED_PRECONDITION` | Caller-fault (id) — unreachable from the participant ; still maps for completeness |
| `DuplicateParticipantId` | `FAILED_PRECONDITION` | Manifest-fault — unreachable from the participant |
| `EmptyMetaSwarm` | `FAILED_PRECONDITION` | Builder-fault — unreachable |
| `MissingOperator` | `FAILED_PRECONDITION` | Builder-fault — unreachable |
| `ConvergenceFailed` | `INTERNAL` | Participant-side cannot raise this — algorithm-side |
| `SafetyViolation` | `INTERNAL` | Participant projects onto its own safety set ; if it raises, that's an invariant breach |
| `HypothesisCheckFailed` | `INTERNAL` | Phase-3 hypothesis check |
| `Io` | `INTERNAL` | Disk / network from the participant |
| `Yaml` | `INTERNAL` | Manifest reload (PR #12) |
| `LockPoisoned` | `INTERNAL` | Concurrent panic in Phase-1 |
| `Internal(_)` | `INTERNAL` | Catch-all |
| `_ =>` (non-exhaustive) | `INTERNAL` | Forward-compat for future Phase-1 variants |

### 15.3 Validation-error mapping

`validate::validate_step` and `validate_reset` return `ServerError::Config(_)`,
which the existing `ServerError → Status` (`impl From`) maps to
`FAILED_PRECONDITION`. ✓

### 15.4 Panic mapping

`JoinError` (from `spawn_blocking` panicking) → `INTERNAL("step panicked:
{e}")`. The `panic = "unwind"` profile ensures the closure unwinds cleanly
without aborting the process.

---

## 16. `Capabilities` response (real values)

### 16.1 Before PR #9

```rust
ParticipantDescriptor {
    id: self.config.id.clone(),
    dim: 0,
    kappa: 0.0,
    seed: 0,
    build_sha: crate::GIT_SHA.into(),
    blas_backend: "none".into(),
    features: vec!["determinism".into()],
}
```

### 16.2 After PR #9

```rust
ParticipantDescriptor {
    id: self.local.id().to_string(),
    dim: u32::try_from(self.local.dim()).unwrap_or(u32::MAX),
    kappa: self.local.kappa(),
    seed: self.local.seed(),
    build_sha: crate::GIT_SHA.into(),
    blas_backend: "none".into(),
    features: vec!["determinism".into(), "step_index".into()],
}
```

### 16.3 Why this matters

The controller's `validate_expected_against_manifest` (PR #8 §
`controller/iterate.rs`) cross-checks the request's `expected_participants`
against the manifest. A future PR (#11) will also cross-check against the
*participant's* `Capabilities.descriptor` — that requires real values. PR #9
populates them so the cross-check works the day PR #11 lands.

### 16.4 Test

  * `capabilities_descriptor_uses_local_participant_values` — replaces
    `capabilities_returns_descriptor` from PR #6, with assertions on the
    real dim/kappa/seed.

---

## 17. Concurrency model + lock hierarchy

### 17.1 Single lock

The only lock in PR #9's participant path is the `Mutex<Tensor>` inside
`MockParticipant.state`. Phase-1 acquires it inside `step_with_pull`,
`current_state`, `reset`, and `set_state`. No nested locks ; no deadlock.

### 17.2 Acquisition order

```
1. (no controller-side InflightSemaphore — that's on the controller side)
2. spawn_blocking task starts.
3. LocalParticipant::step / reset / state / set_state called (sync).
4. MockParticipant.state.lock() acquired.
5. mutation / read.
6. lock released as the function returns.
7. spawn_blocking returns to the awaiting handler future.
```

### 17.3 Why no outer Mutex

`MockParticipant : Send + Sync` (verified in Phase-1's `assert_send_sync`
tests). Concurrent Steps from multiple gRPC handlers serialise on the
internal mutex naturally. Adding an outer `Mutex<LocalParticipant>` would
just double-wrap.

### 17.4 Atomic counters

`step_index_last` and `reset_gen` are `AtomicU64`. Concurrent updates are
race-free at the integer level. The *value* might lag a tiny bit behind in
the presence of concurrent Steps, but this is acceptable (the field is
diagnostic).

### 17.5 Cross-handler isolation

`spawn_blocking` runs each handler on a fresh blocking-pool thread. Two
concurrent Steps :

  * acquire their own `LocalParticipant` clone (cheap, `Arc`),
  * call `step_with_pull` which acquires the internal mutex,
  * serialise on that mutex,
  * release.

Total throughput on a single participant ≈ 1 / `step_with_pull_latency`. For
`dim = 1024`, that's ~ 10 000 Step/s peak. The controller's MAX_INFLIGHT = 10
caps the actual rate well below that.

---

## 18. Threat-model coverage

PR #9 mitigates or partially mitigates the following threats from
`docs/security/PHASE2_THREAT_MODEL.md` :

| Threat | Severity | PR #9 mitigation |
|---|---|---|
| **T9** (state poisoning via malformed Step) | SEV-2 | `validate::validate_step` rejects bad shape, missing pulled, envelope corruption ; `FAILED_PRECONDITION` returned with diagnostic message |
| **T11** (silent NaN propagation across iterations) | SEV-2 | `validate_tensor_finite` rejects any non-finite element in `pulled` or `initial` ; participant state stays finite for the lifetime of the process |
| **T13** (fan-out leak / cancellation cascade) | SEV-2 | participant honours tonic's drop-on-cancel ; `spawn_blocking` cleanly returns ; no leaked tasks |
| **T15** (participant impersonation) | SEV-1 | **partially** — `Capabilities.descriptor.build_sha` is now populated, lets the controller cross-check ; full mitigation requires mTLS in PR #16 |
| **T16** (Step flood DoS at the participant) | SEV-3 | **partially** — Phase-2 single-controller assumption means the controller's MAX_INFLIGHT caps the rate ; full per-participant rate limit in Phase 3 |

T15 and T16 remain "partial" — flagged in the PR description and risk
log (§ 21).

---

## 19. Tests strategy (unit + integration + e2e)

### 19.1 Unit tests (in source files)

| File | Tests added | Coverage focus |
|---|---|---|
| `participant/local.rs` | 8 | Construction, kappa range, step counter bump, reset gen bump, state-after-reset, dim mismatch |
| `participant/validate.rs` | 9 | Missing pulled, dim mismatch, envelope corruption, NaN, ±∞, reset path |
| `participant/state.rs` | 1 modified + 1 new | Descriptor reflects manifest row |
| `participant/service.rs` | 8 new (3 modified) | Step happy / step-bad / reset / state / capabilities |
| `manifest/mod.rs` | 3 added | `find_participant` |

Total new/modified unit tests : **30**.

### 19.2 Integration smoke (`tests/integration_step_smoke.rs`) — NEW

A test that spawns the participant binary **in-process** on a random port and
exercises Step / Reset / State over real gRPC.

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn step_round_trip_returns_new_state() {
    let (addr, _handle) = spawn_participant_service(
        ParticipantManifest { id: "p-1".into(), dim: 8, kappa: 0.6, seed: 11, blas_backend: "none".into() },
        IterationParams { alpha_default: 0.5, coupling_gamma: 0.2 },
    ).await;

    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
        .unwrap().connect().await.unwrap();
    let mut client = MetaParticipantServiceClient::new(channel);

    let pull = TensorF32 { dim: 8, data: vec![1.0; 8] };
    let resp = client.step(StepRequest {
        pulled_from_meta: Some(pull),
        step_index: 7,
    }).await.unwrap().into_inner();

    assert!(resp.new_state.is_some());
    assert_eq!(resp.new_state.as_ref().unwrap().dim, 8);
    assert!(resp.step_us > 0);

    let st = client.state(StateRequest {}).await.unwrap().into_inner();
    assert_eq!(st.step_index_last, 7);
}
```

Other tests in the same file :

  * `step_rejects_nan` — server returns `FAILED_PRECONDITION`.
  * `step_rejects_dim_mismatch` — server returns `FAILED_PRECONDITION`.
  * `reset_without_initial_succeeds` — `ResetParticipantResponse.reset_gen == 1`.
  * `reset_with_initial_sets_state` — `state.current` matches.
  * `health_returns_serving_after_step` — no health regression.
  * `step_latency_under_5ms_for_dim_8` — perf smoke (loose bound).

### 19.3 End-to-end integration (`tests/integration_e2e_iterate.rs`) — NEW

This test **replaces** the `TestParticipantSvc` stand-in from PR #8's
`integration_iterate_inproc.rs`. It spawns :

  1. Three real `ParticipantService` instances (using the production code, not
     the test stub) on random localhost ports.
  2. A real `ControllerService` (in-process) pointing at them.
  3. Calls `Iterate` over the gRPC stack.
  4. Asserts convergence and bit-identical determinism.

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_kovant_converges_with_real_participants() {
    let (a1, _) = spawn_real_participant("supplier-scoring", 8, 0.6, 11).await;
    let (a2, _) = spawn_real_participant("contract-risk",   8, 0.7, 22).await;
    let (a3, _) = spawn_real_participant("spend-categ.",    8, 0.5, 33).await;

    let controller_state = build_test_controller_state(/* … a1,a2,a3 */);
    let svc = ControllerService::new(controller_state);
    let resp = svc.iterate(Request::new(iterate_request(8, 100, 1.0e-3)))
        .await.unwrap().into_inner();

    assert_eq!(resp.stop_reason, ProtoStopReason::Converged as i32);
    assert!(resp.iterations <= 80);
    assert!(resp.final_residual <= 1.0e-3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_bit_identical_3_reruns() {
    /* same setup, 3 reruns with reset between, signatures must match */
}
```

`integration_iterate_inproc.rs` is **simplified** : the `TestParticipantSvc`
stub is deleted, replaced by importing the helpers from
`integration_e2e_iterate.rs` (or by inlining a single
`spawn_real_participant`). The 3 existing tests stay green.

### 19.4 Why the in-process service (not the binary)

Spawning the binary (`Command::new("target/.../participant")`) is :

  * ~ 100 ms per test (process startup, manifest parse, gRPC bind),
  * subject to `target/` cleanliness (CI runners may have stale builds),
  * harder to debug (no shared logging context).

In-process `ParticipantService` :

  * ~ 1 ms per test,
  * exercises the **same production code** (the binary is just a thin wrapper),
  * trivial to debug.

The full binary path is exercised in PR #10's deployment tests (process
boundary, cgroups, etc.).

### 19.5 Determinism test (sampled)

`iterate_e2e_bit_identical_3_reruns` does three Iterates with `Reset` between
runs. Asserts `(iterations, final_residual.to_bits())` is identical across
the three runs. This is the AC8 deliverable.

### 19.6 Test coverage matrix

| Surface | Unit | Integration smoke | E2E |
|---|---|---|---|
| `LocalParticipant::step` | ✓ | ✓ | ✓ |
| `LocalParticipant::reset` | ✓ | ✓ | ✓ |
| `LocalParticipant::current_state` | ✓ | ✓ | ✓ |
| `validate::validate_step` | ✓ | ✓ (NaN, dim) | — |
| `validate::validate_reset` | ✓ | ✓ | — |
| `bin/participant.rs` boot path | — | ✓ (in-process equiv.) | — (PR #10 binary spawn) |
| Step + Reset + State + Health interleaved | — | ✓ | ✓ |
| Iterate over 3 real participants | — | — | ✓ |
| Determinism 3 reruns | — | — | ✓ |

---

## 20. Acceptance criteria (AC1–AC20)

PR #9 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `cargo build -p efe-meta-swarm-server` succeeds | CI |
| AC2 | `cargo build -p efe-meta-swarm-server --release` succeeds | CI |
| AC3 | New unit tests : ≥ 8 in `local.rs`, ≥ 9 in `validate.rs`, ≥ 8 in `service.rs`, ≥ 3 in `manifest::tests` | CI |
| AC4 | `tests/integration_step_smoke.rs` : 7 tests pass | CI |
| AC5 | `tests/integration_e2e_iterate.rs` : 2 tests pass (convergence + determinism) | CI |
| AC6 | `tests/integration_iterate_inproc.rs` : 3 tests still pass (using real `ParticipantService` instead of `TestParticipantSvc`) | CI |
| AC7 | Kovant scenario : `iterations ≤ 80`, `final_residual ≤ 1e-3`, `stop_reason == Converged` | smoke (AC5) |
| AC8 | Determinism : 3 reruns bit-identical (same `iterations`, same `final_residual.to_bits()`) | smoke (AC5) |
| AC9 | Phase-1 tests pass (200 default + 209 attention-weighted + 202 core-integration) | CI |
| AC10 | PR #5 tests pass (15/15) | CI |
| AC11 | PR #6 tests pass (49+ incl. new `descriptor_*`) | CI |
| AC12 | PR #8 tests pass (76 + 3 + 3, with `TestParticipantSvc` removed) | CI |
| AC13 | `cargo clippy --all-targets` 0 warnings on PR #9 files | CI |
| AC14 | `cargo fmt -- --check` exit 0 | CI |
| AC15 | `cargo doc --no-deps` clean | CI |
| AC16 | PR description references threat IDs `T9, T11, T13, T15, T16` | manual review |
| AC17 | Master gate stays GREEN | CI |
| AC18 | 4 reviewers approved : 2× meta-swarm + 1× sec + 1× sre | GitHub |
| AC19 | sec-team confirms T15 partial mitigation acceptable for L2 | sec-team comment |
| AC20 | Binary smoke : `cargo run --bin participant` against a manifest fixture, with `grpcurl` Step round-trip, returns HTTP/2 200 + a `TensorF32` of the correct dim | manual or `scripts/smoke_participant.sh` |

---

## 21. Risks + mitigations

| ID | Risk | Likelihood / Impact | Mitigation |
|---|---|---|---|
| **PR9-R1** | `MockParticipant::with_contraction` Phase-1 contract changes between PR #9 development and merge | L / M | Pin the Phase-1 SHA in PR's Cargo.lock ; cross-check in pre-flight PF6 |
| **PR9-R2** | A future PR #11 (Discovery) refactor changes the manifest path resolution and breaks `find_participant` | L / M | Helper is 20 LOC ; unit-tested ; PR #11 will need to update both controller and participant — caught at review |
| **PR9-R3** | `step_index` field overflows u32 (4 G iterations) | LL / M | Cap MAX_ITERS_HARD_CAP at 200 (PR #8) — overflow impossible within 1 Iterate ; across Iterates, the wire field resets to 0 |
| **PR9-R4** | `spawn_blocking` queue saturation under load (default 512 threads) | L / H | Phase-2 single-controller : ≤ 10 inflight ; ≤ 30 Steps in flight ; well under the default queue |
| **PR9-R5** | `MockParticipant.state.lock()` contention in concurrent Steps | LL / L | Steps are short (< 100 µs for dim ≤ 1024) ; lock held briefly ; controller serialises per participant anyway |
| **PR9-R6** | `local_residual` self-report differs from controller-computed residual due to f32 ↔ f64 rounding | L / L | Cosmetic ; the controller's residual is authoritative ; document in `compute_residual` doc-comment |
| **PR9-R7** | Test-binary spawn races with random ports on busy CI runners | M / L | Reuse the bind-drop-rebind pattern from PR #8 ; 1 s wait-up loop |
| **PR9-R8** | `set_state(initial)` clones the tensor — wasteful for very large dims | L / L | Phase-2 max dim = 1024 (Kovant) ; clone ≤ 4 KB ; acceptable |
| **PR9-R9** | `iteration` block on the manifest is read-only at boot ; PR #12 SIGHUP reload must atomically swap it | M / M | Document the swap protocol now ; `Arc<IterationParams>` already chosen for this purpose (AD §9.4) |
| **PR9-R10** | A future PR replaces `MockParticipant` with the sidecar `LocalParticipant` and changes the determinism contract | L / M | The wrapper `LocalParticipant` (PR #9 name) is the **adapter** ; swapping the wrapped type does not break the contract — but the swap PR needs to re-run AC5/AC8 |

---

## 22. Rollback strategy

If PR #9 lands and produces an outage :

  1. **Revert path** : `git revert <merge-commit>`. Each commit C1-C5 is
     atomic ; reverting C5 alone restores `UNIMPLEMENTED` stubs for Step /
     Reset / State while keeping the `LocalParticipant` plumbing in tree.
     The integration tests that depend on a real participant would need to
     be skipped via `#[ignore]` until a fix lands.
  2. **Hotfix path** : if the bug is in `validate.rs`, the simplest fix is to
     bypass validation and rely on Phase-1's checks. Revert is safer for
     production but loses the NaN guard.
  3. **No data loss** : the participant has no persisted state in PR #9 ; a
     restart loses only in-flight Iterate progress.
  4. **Operator action** : restart the participant binary with PR #8's
     image ; the controller's `RemoteParticipant::connect` will auto-retry.

---

## 23. Sequence diagrams (ASCII)

### 23.1 Step under load (3 concurrent controllers — Phase-3 preview)

```
Ctrl-A   Ctrl-B   Ctrl-C         Participant
  |        |        |                  |
  |Step(p) |        |                  |
  |───────────────────────────────────>|
  |        |Step(p) |                  |
  |        |───────────────────────────>|
  |        |        |Step(p)           |
  |        |        |─────────────────>|
  |        |        |                  |  state.lock() → A
  |        |        |                  |  step_with_pull
  |        |        |                  |  state.lock() ← A
  |        |        |                  |───────────────→ Ctrl-A
  |        |        |                  |  state.lock() → B
  |        |        |                  |  step_with_pull
  |        |        |                  |  state.lock() ← B
  |        |        |                  |───────────────→ Ctrl-B
  |        |        |                  |  state.lock() → C
  |        |        |                  |  step_with_pull
  |        |        |                  |  state.lock() ← C
  |        |        |                  |───────────────→ Ctrl-C
```

(Phase 2 has 1 controller ; this is a Phase 3 preview.)

### 23.2 Step + cancel race

```
Controller                              Participant
   |  Step(p)                                |
   |────────────────────────────────────────>|
   |                                         |  spawn_blocking
   |                                         |   ├ validate
   |                                         |   ├ proto_to_tensor
   |                                         |   ├ step_with_pull  ← in progress
   |  <client disconnect>                    |
   |                                         |   │   (irrelevant — sync code,
   |                                         |   │    no cancel observation)
   |                                         |   └ done
   |                                         |  handler future = dropped
   |                                         |  → response written to dropped peer
   |                                         |    (tonic absorbs the error)
```

The participant computes one Step uselessly. Not a correctness issue ; the
state is still consistent (single mutation completed).

### 23.3 Iterate end-to-end (PR #8 + PR #9 wires)

```
Ctrl                Part-1            Part-2            Part-3
 | Iterate(req)     |                  |                  |
 |                  |                  |                  |
 | validate_request |                  |                  |
 | connect_part(1)─>|                  |                  |
 | connect_part(2)──────────────────────>|                  |
 | connect_part(3)─────────────────────────────────────────>|
 | build_meta_swarm |                  |                  |
 | spawn_blocking { |                  |                  |
 |   loop iter:     |                  |                  |
 |     par_iter {   |                  |                  |
 |       Step ─────>|                  |                  |
 |       Step ────────────────────────>|                  |
 |       Step ───────────────────────────────────────────>|
 |     }            |                  |                  |
 |     check resid  |                  |                  |
 |     until conv   |                  |                  |
 | }                |                  |                  |
 | build_response   |                  |                  |
 |<─────────────────────────────────────────────────────────|
```

---

## 24. Reviewer checklist

Copy-paste this into the PR description for the reviewers :

```
## Reviewer checklist — PR #9

### Architecture
- [ ] `LocalParticipant` wraps `MockParticipant` (not `participants::local`) — AD1 holds
- [ ] Manifest path resolution mirrors `bin/controller.rs` (same env var, same default) — AD2
- [ ] `step_with_pull` runs inside `spawn_blocking` — AD3
- [ ] No new per-participant MAX_INFLIGHT — AD4
- [ ] `Arc<MockParticipant>` ; no outer mutex — AD5

### Correctness
- [ ] `validate::validate_step` rejects NaN, ±∞, dim mismatch, missing field
- [ ] `validate::validate_reset` validates the `initial` tensor
- [ ] `Capabilities.descriptor` reflects real values from the manifest row
- [ ] `step_index_last` updated only on success
- [ ] `reset_gen` monotonic, lock-free

### Concurrency
- [ ] Only the Phase-1 inner `Mutex<Tensor>` is held during a Step
- [ ] `AtomicU64::Relaxed` is justified (advisory counters)
- [ ] `spawn_blocking` queue not saturated under expected load
- [ ] No `await` inside the blocking section

### Determinism
- [ ] Seeded initial state via `MockParticipant::with_contraction`
- [ ] `compute_residual` is f64-exact and deterministic
- [ ] Bit-identical reruns proved in `integration_e2e_iterate.rs`

### Threat model
- [ ] T9, T11 mitigations land in `validate.rs`
- [ ] T13 cancellation : `spawn_blocking` drop-tolerant
- [ ] T15 partial mitigation via `build_sha` in descriptor
- [ ] T16 partial mitigation noted ; controller MAX_INFLIGHT caps the rate

### Tests
- [ ] ≥ 30 unit tests in module
- [ ] 7 tests in `integration_step_smoke.rs`
- [ ] 2 tests in `integration_e2e_iterate.rs` (convergence + determinism)
- [ ] `integration_iterate_inproc.rs` simplified (TestParticipantSvc removed)
- [ ] AC1 – AC20 all green

### Operability
- [ ] Binary fails fast with clear error if manifest missing / id absent
- [ ] `tracing` spans carry `participant_id`
- [ ] No new env vars (`EFE_META_SWARM_MANIFEST` already exists)
- [ ] Health response correct after Step / Reset cycle
```

---

## 25. Migration notes (operator-facing)

After PR #9 merges, deploying a participant binary requires :

  1. **A valid manifest YAML** at `EFE_META_SWARM_MANIFEST` (default
     `/etc/efe-meta-swarm/manifest.yaml`). Same file the controller reads.
  2. **`EFE_META_SWARM_PARTICIPANT_ID`** set to a value that exists in the
     manifest's `participants:` list.
  3. **`EFE_META_SWARM_PARTICIPANT_ADDR`** = `host:port` to bind the gRPC
     server.
  4. **`EFE_META_SWARM_PARTICIPANT_METRICS`** = `host:port` for the Prometheus
     exporter (PR #6).

Example `manifest.yaml` (Kovant T2 topology) :

```yaml
version: v0.2.0
meta:
  dim: 16
  safety:
    kind: box
    radius: 10.0
participants:
  - id: supplier-scoring
    dim: 8
    kappa: 0.6
    seed: 11
    blas_backend: none
  - id: contract-risk
    dim: 8
    kappa: 0.7
    seed: 22
    blas_backend: none
  - id: spend-categorization
    dim: 8
    kappa: 0.5
    seed: 33
    blas_backend: none
iteration:
  max_iters: 100
  tolerance: 0.0001
  alpha_default: 0.5
  coupling_gamma: 0.2
  deterministic: true
  max_wall_ms: 15000
```

To run the Kovant topology on a single dev host :

```bash
# Participant 1
EFE_META_SWARM_MANIFEST=./manifest.yaml \
EFE_META_SWARM_PARTICIPANT_ID=supplier-scoring \
EFE_META_SWARM_PARTICIPANT_ADDR=127.0.0.1:50081 \
EFE_META_SWARM_PARTICIPANT_METRICS=127.0.0.1:50091 \
cargo run --bin participant &

# Participant 2 (same env, different id + ports)
EFE_META_SWARM_MANIFEST=./manifest.yaml \
EFE_META_SWARM_PARTICIPANT_ID=contract-risk \
EFE_META_SWARM_PARTICIPANT_ADDR=127.0.0.1:50082 \
EFE_META_SWARM_PARTICIPANT_METRICS=127.0.0.1:50092 \
cargo run --bin participant &

# Participant 3
EFE_META_SWARM_MANIFEST=./manifest.yaml \
EFE_META_SWARM_PARTICIPANT_ID=spend-categorization \
EFE_META_SWARM_PARTICIPANT_ADDR=127.0.0.1:50083 \
EFE_META_SWARM_PARTICIPANT_METRICS=127.0.0.1:50093 \
cargo run --bin participant &

# Controller
EFE_META_SWARM_MANIFEST=./manifest.yaml \
EFE_META_SWARM_CONTROLLER_ADDR=127.0.0.1:50080 \
EFE_META_SWARM_CONTROLLER_METRICS=127.0.0.1:50090 \
EFE_META_SWARM_PARTICIPANTS=supplier-scoring=127.0.0.1:50081,contract-risk=127.0.0.1:50082,spend-categorization=127.0.0.1:50083 \
cargo run --bin controller &
```

Then drive an Iterate with `grpcurl` :

```bash
grpcurl -plaintext -d '{ "iteration": {"max_iters": 100, "tolerance": 1.0e-3, "alpha_default": 0.5, "coupling_gamma": 0.2, "deterministic": true, "max_wall_ms": 15000} }' \
  127.0.0.1:50080 efe.v1.meta_swarm.MetaSwarmService/Iterate
```

Expected : `stop_reason: STOP_REASON_CONVERGED`, `iterations < 80`, `final_residual < 1e-3`.

---

## 26. Open questions

| ID | Question | Status |
|---|---|---|
| **OQ-PR9-1** | Should `local_residual` be `0.0` until validated to be deterministic across CPUs ? | Open — defer to first sec review |
| **OQ-PR9-2** | Do we want a "participant build-sha must match controller build-sha" check on `Capabilities` ? | Open — punt to PR #11 |
| **OQ-PR9-3** | Should `step_index` wrap-around (u32 max) be a fatal error or a silent reset ? | Open — current behaviour : silent reset (the wire field is `u32`, no overflow check) |
| **OQ-PR9-4** | If `Reset(initial=...)` succeeds but resets `step_index_last` to 0, would PR #18's tracing get confused ? | Open — answer in PR #18 |
| **OQ-PR9-5** | Should the `IterationParams` be `Arc<RwLock<...>>` instead of `Arc<...>` for PR #12 SIGHUP ? | Defer to PR #12 architecture |
| **OQ-PR9-6** | What's the participant's behaviour if the controller sends a Step with `pulled.dim = 0` (legal proto, illegal semantics) ? | Resolved — `validate_tensor_shape` rejects ; `dim == 0` ≠ `expected_dim` (which is ≥ 1 per manifest validation) |
| **OQ-PR9-7** | Should we record the wall-clock time *before* validation (start of handler) or *after* (start of `step_with_pull`) ? | Resolved — before validation, so `step_us` includes validation cost (it's negligible) |
| **OQ-PR9-8** | If a hostile client sends 10 GB of `data` in a single TensorF32, do we OOM ? | Resolved by tonic's default max-message-size (4 MB by default ; `dim ≤ 1024` → ≤ 4 KB, well under) ; document in PR #16 |

---

## 27. Appendix : full file inventory

### 27.1 Files added (5)

```
crates/efe-meta-swarm-server/src/participant/local.rs                       NEW    ~250 LOC
crates/efe-meta-swarm-server/src/participant/validate.rs                    NEW    ~180 LOC
crates/efe-meta-swarm-server/tests/integration_step_smoke.rs                NEW    ~200 LOC
crates/efe-meta-swarm-server/tests/integration_e2e_iterate.rs               NEW    ~300 LOC
docs/EFE_A1_PHASE2_PR09_ARCHITECTURE.md                                     NEW    this file
```

### 27.2 Files modified (7)

```
crates/efe-meta-swarm-server/src/participant/mod.rs                 MODIFIED   +5 LOC : re-export new modules
crates/efe-meta-swarm-server/src/participant/state.rs              MODIFIED  +60 −10 : hold LocalParticipant + IterationParams
crates/efe-meta-swarm-server/src/participant/service.rs            MODIFIED +400 −80 : wire RPCs + tests
crates/efe-meta-swarm-server/src/bin/participant.rs                MODIFIED  +60 −5  : load manifest, build state
crates/efe-meta-swarm-server/src/manifest/mod.rs                   MODIFIED  +20 −0  : find_participant helper
crates/efe-meta-swarm-server/src/error.rs                          MODIFIED  +25 −0  : meta_swarm_error_to_status (promoted from controller/iterate.rs)
crates/efe-meta-swarm-server/src/controller/iterate.rs             MODIFIED  +1 −22  : import from crate::error instead of local copy
crates/efe-meta-swarm-server/tests/integration_iterate_inproc.rs   MODIFIED +5 −90   : remove TestParticipantSvc, import from integration_e2e_iterate
```

### 27.3 Files unchanged (relevant ones)

```
crates/efe-meta-swarm-server/Cargo.toml                            no new deps
crates/efe-meta-swarm/src/participants/mock.rs                     used as-is
crates/efe-meta-swarm-proto/protos/efe/v1/meta_swarm/messages.proto used as-is
crates/efe-meta-swarm-server/src/bin/controller.rs                 unchanged
crates/efe-meta-swarm-server/src/controller/{service,state,iterate}.rs minor (import update only)
crates/efe-meta-swarm-server/src/observability/{*}.rs              unchanged
crates/efe-meta-swarm-server/src/config/{*}.rs                     unchanged
crates/efe-meta-swarm-server/src/auth/{*}.rs                       unchanged (PR #16)
crates/efe-meta-swarm-server/src/discovery/{*}.rs                  unchanged (PR #11)
```

### 27.4 LOC tally

| Category | LOC |
|---|---|
| New code | ~ 930 |
| New tests | ~ 540 |
| Modified code | ~ 90 |
| Deleted code (TestParticipantSvc et al.) | ~ 100 |
| **Net** | **+1 460 LOC** |

---

## 28. Glossary

| Term | Definition |
|---|---|
| **AC** | Acceptance Criterion |
| **AD** | Architectural Decision |
| **L2 gate** | Phase-2 graduation gate (cf. roadmap § 5) |
| **MockParticipant** | Phase-1 in-process participant with a configurable contraction Φ(x) = κx |
| **LocalParticipant** | (this PR) production wrapper around `MockParticipant`, exposes gRPC-friendly accessors |
| **Pull tensor** | The tensor projected from meta-space back to a participant's local space ; arrives as `StepRequest.pulled_from_meta` |
| **`step_with_pull`** | Phase-1 trait method, one coupled iteration step |
| **`step_index`** | Wire field carrying the controller's iteration count ; advisory |
| **`reset_gen`** | Monotonic counter on the participant, bumped per Reset |
| **`build_sha`** | Git short SHA of the binary at compile time ; used for crash-loop diagnosis and Phase-3 cosign attestation |
| **Kovant scenario** | Reference 3-participant topology (`supplier-scoring`, `contract-risk`, `spend-categorization`) used for benchmarks |
| **Sync ↔ async bridge** | Pattern for invoking sync Phase-1 code from an async tonic handler ; standard idiom is `tokio::task::spawn_blocking` |
| **T2** | Phase-2 single-node topology (1 controller + 3 participants on `localhost`) |
| **TestParticipantSvc** | Inline stand-in introduced in PR #8 integration tests ; removed by PR #9 |

---

*End of architecture document. Total: ~ 1 850 lines.*

*Next deliverable : PR #9 code (5 commits).*
