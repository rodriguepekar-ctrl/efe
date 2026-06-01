# EFE-A1 Phase 2 — PR #12 : SIGHUP + `ReloadConfig`

| Field | Value |
|---|---|
| **PR** | #12 (Sprint A1 Phase 2 / P2.3) |
| **Branch** | `efe-a1/p2-pr12-sighup-reloadconfig` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× meta-swarm + 1× efe-server + 1× sre + 1× sec |
| **Depends on** | PR #5 (proto), PR #6 (scaffold), PR #7 (threat model), PR #8 (controller wire), PR #9 (participant wire), PR #10 (multiprocess), PR #11 (efe-server flag plumbing) |
| **Blocks** | PR #13 (canary observability), PR #14 (sidecar reload coordination), PR #17 (multi-controller HA) |
| **L2 gate** | G9 (live reconfiguration without process restart) |
| **Threat IDs in scope** | T19 (malicious manifest swap), T20 (in-flight Iterate race), T21 (reload-flood DoS) |
| **Roadmap reference** | EFE_A1_PHASE2_ROADMAP.md § C5 |
| **CVSS bandwidth** | All ≤ SEV-3 ; no new SEV-1 introduced |

---

## Table of contents

  0. TL;DR
  1. Goal + non-goals
  2. Pre-flight checks
  3. Branch + commit plan (6 commits)
  4. Architectural decisions (AD1–AD15)
  5. Topology + integration diagram
  6. Crate layout additions
  7. The manifest atomic-swap protocol
  8. SIGHUP handler — `observability/shutdown.rs` extension
  9. `controller::reload` module (NEW)
 10. `controller/service.rs::reload_config` wire
 11. `participant::reload` module (NEW)
 12. `participant/state.rs::IterationParams` atomic swap
 13. Health status `RELOADING` semantics
 14. `efe-server` `MetaSwarmClient::health_probe` accepts `RELOADING`
 15. Determinism contract under reload
 16. What CAN change vs CANNOT change in a reload
 17. Threat-model coverage
 18. Concurrency model + lock hierarchy
 19. Metrics + tracing
 20. Tests strategy
 21. Acceptance criteria (AC1–AC28)
 22. Risks + mitigations
 23. Rollback strategy
 24. Sequence diagrams (ASCII)
 25. Reviewer checklist
 26. Operator runbook
 27. Open questions
 28. Appendix : full file inventory
 29. Glossary

---

## 0. TL;DR

PR #12 ships the **live reconfiguration** path for the Phase-2 control plane.
Operators can change manifest values (iteration knobs, participants set) and
trigger a hot reload via two complementary triggers :

  * **SIGHUP signal** — the canonical Unix way to "reload your config" ;
    handled by both `controller` and `participant` binaries.
  * **`MetaSwarmService::ReloadConfig` gRPC** — programmatic admin trigger
    on the controller, returns the new `reload_gen` so clients can confirm
    the swap landed.

Both paths share one in-process protocol :

  1. Load the manifest from `EFE_META_SWARM_MANIFEST` (or accept an inline
     payload from the RPC).
  2. Validate via `manifest::validate_manifest` (PR #8) plus **PR #12's
     reload-specific invariants** (no participant dim/kappa/seed drift,
     no meta-dim change).
  3. Flip `HealthState` to `RELOADING (3)` so probes know we're mid-swap.
  4. Atomically swap the manifest behind `Arc<RwLock<Manifest>>` (PR #8
     foundation) under a **brief write-lock window** (~ µs on a small
     manifest).
  5. Atomically swap `Arc<IterationParams>` on the participant side (PR #9
     foundation).
  6. Flip `HealthState` back to `SERVING (1)` and `bump_reload_gen()`.
  7. Return the new `reload_gen` to the operator (RPC) or log it (SIGHUP).

In-flight `Iterate` / `Step` calls **do not interrupt** : they hold an
`Arc<Manifest>` snapshot captured at call start (PR #8 already clones the
manifest inside `run_iterate` — § 7 explains why this is sufficient).

PR #11's `efe-server` Health probe is updated to **accept `RELOADING` as
transient** — without this, a hot reload would cause `efe-server` boot
probes (or future ambient probes) to spuriously fail-fast.

PR #12 closes Phase-2 graduation gate **G9 (live reconfiguration without
process restart)**.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Add a SIGHUP-driven AND a gRPC-driven hot reload path that swaps the
> Phase-2 manifest atomically in-process for both `controller` and
> `participant` binaries, preserving in-flight `Iterate` correctness,
> exposing a monotonic `reload_gen` counter to operators, and refusing
> reloads that would invalidate participant state.

### 1.2 Non-goals (deferred to later PRs)

| Item | Why deferred | Target PR |
|---|---|---|
| Sidecar reload coordination (when PR #14 ships the on-disk weights) | needs the weight-loader contract | PR #14 |
| Multi-controller leader-elected reload | needs the leader-lock subsystem | PR #17 |
| Canary % rollout of new manifest | needs canary observability | PR #13 |
| Rollback to previous manifest by reload_gen | needs a per-gen manifest cache | PR #20 |
| Change participant dim / kappa / seed at reload time | requires state migration semantics | Phase 3 |
| Change meta-space dim at reload time | requires participant lift/pull rebuild | Phase 3 |
| Per-tenant manifest overrides at reload time | multi-tenant scope creep | Phase 3 |
| `participant`-side gRPC reload RPC | the proto doesn't define one ; SIGHUP is enough for Phase 2 | Phase 3 |
| Wire mTLS auth on `ReloadConfig` | needs cert provisioning | PR #16 |
| Helm chart Pod-annotation-driven reload | deployment concern | PR #20 |
| Crash-safe partial reload (atomic transaction across N participants) | distributed-commit semantics | Phase 3 |
| Hot reload of `MetaSwarmConfig` in `efe-server` | needs SIGHUP wiring in `efe-server` (Phase 1+) | follow-up |

### 1.3 Scope summary

PR #12 ships :

  1. The manifest atomic-swap protocol (controller-side and
     participant-side, distinct because the state surfaces differ).
  2. SIGHUP handler in both binaries.
  3. `MetaSwarmService::ReloadConfig` RPC wired in the controller.
  4. Health `RELOADING` status flip during the swap window.
  5. `reload_gen` counter bump on every successful reload.
  6. Reload-specific manifest invariants (`validate_reload_compatibility`).
  7. PR #11's `MetaSwarmClient::health_probe` updated to accept
     `RELOADING (3)` as transient.

PR #12 does **not** change :

  * The wire format (proto already defines `ReloadConfigRequest` etc.).
  * The Phase-1 algorithm (`MetaSwarm::iterate`) — unchanged.
  * The participant's Phase-1 wrapper (`LocalParticipant`) — its κ / dim /
    seed are immutable for the binary's lifetime.

---

## 2. Pre-flight checks

| # | Check | Verifier | Pass criterion |
|---|---|---|---|
| PF1 | `cargo build -p efe-meta-swarm-server --release` succeeds on `main` | CI | green |
| PF2 | `cargo test -p efe-meta-swarm-server --offline --all-targets` is **150 tests** on `main` (PR #11 baseline) | CI | green |
| PF3 | `cargo test -p efe-server --offline --all-targets` is **733 lib + 11 integration_meta_swarm_plumbing** = 744 on `main` (PR #11 baseline) | CI | green |
| PF4 | `state.manifest: Arc<RwLock<Manifest>>` exists in `controller/state.rs` (PR #8) | grep | line found |
| PF5 | `state.iteration: Arc<IterationParams>` exists in `participant/state.rs` (PR #9) | grep | line found |
| PF6 | `HealthState::set_reloading()` exists in `observability/health.rs` (PR #6) | grep | line found |
| PF7 | `HealthState::bump_reload_gen()` exists (PR #6) | grep | line found |
| PF8 | `MetaSwarmService::ReloadConfig` still returns `UNIMPLEMENTED` with `"PR #12"` in the message (PR #8 stub) | grep | line found |
| PF9 | PR #10's `ProcessHarness::sigterm_*` handles `cfg(unix)` libc::kill — extensible to SIGHUP | grep | line found |
| PF10 | PR #11's `MetaSwarmClient::health_probe` rejects status ≠ 1 (current behaviour we're about to relax) | grep | line found |
| PF11 | Threat model (PR #7) is at v1.0 frozen state | manual | sec sign-off |

---

## 3. Branch + commit plan

### 3.1 Branch

```
efe-a1/p2-pr12-sighup-reloadconfig
```

Branched from `main` after PR #11 merge SHA.

### 3.2 Commit-by-commit plan (6 commits, each CI-green)

| # | Subject | Files | LOC ± | Verification |
|---|---|---|---|---|
| C1 | `feat(manifest): validate_reload_compatibility() + 8 invariant tests` | `~ src/manifest/mod.rs`, `~ src/manifest/schema.rs` (no struct change) | +110 / −0 | unit tests (≥ 8) covering accept / reject pairs |
| C2 | `feat(controller): reload module with apply_reload() + state swap` | `+ src/controller/reload.rs`, `~ src/controller/mod.rs`, `~ src/controller/state.rs` (+ `info_snapshot` reads `last_reload_gen`) | +220 / −10 | unit tests (≥ 6) : happy, validation reject, in-flight race, gen bump, health flip, manifest restore on validation error |
| C3 | `feat(observability): SIGHUP handler + dispatch trait` | `~ src/observability/shutdown.rs`, `~ src/observability/mod.rs` | +90 / −5 | unit tests (≥ 3) : signal delivered → callback fired ; double-fire is idempotent ; cfg(unix) gating |
| C4 | `feat(controller): wire bin/controller.rs SIGHUP + service.rs ReloadConfig` | `~ src/bin/controller.rs`, `~ src/controller/service.rs` | +120 / −15 | binary smoke : `kill -HUP <pid>` triggers reload ; gRPC ReloadConfig returns new gen |
| C5 | `feat(participant): reload module + SIGHUP + IterationParams swap` | `+ src/participant/reload.rs`, `~ src/participant/state.rs`, `~ src/bin/participant.rs`, `~ src/participant/mod.rs` | +220 / −10 | unit tests (≥ 5) : IterationParams swap, reject row drift, gen bump, SIGHUP fires reload, health flip |
| C6 | `feat(efe-server): MetaSwarmClient probe accepts RELOADING + integration tests` | `~ crates/efe-server/src/services/meta_swarm/mod.rs`, `~ crates/efe-server/tests/integration_meta_swarm_plumbing.rs`, `+ tests/integration_reload_e2e.rs` (in `efe-meta-swarm-server`) | +280 / −20 | integration : controller + participant survive SIGHUP, reload_gen ticks, efe-server probe doesn't reject during the swap window |

Total : **+1 040 / −60 = +980 LOC net**, including ~ 280 LOC of new
integration tests + ~ 600 LOC of production reload code.

### 3.3 Drafting strategy

  * C1-C2-C3 add new code with no behaviour change to the existing wire :
    `MetaSwarmService::ReloadConfig` still returns `UNIMPLEMENTED` after
    these commits.
  * C4 flips the controller's `ReloadConfig` from `UNIMPLEMENTED` to a real
    impl and adds the SIGHUP wire in `bin/controller.rs`. After C4, the
    controller hot-reloads cleanly.
  * C5 extends the same to the participant binary (SIGHUP only — no RPC
    surface on the participant side).
  * C6 closes the loop with `efe-server` so canary probes don't false-alarm
    during the (~ µs) RELOADING window.

If C6 flakes, revert it alone — the production code in C1-C5 stays in
trunk and the operator can still SIGHUP without `efe-server` involvement.

---

## 4. Architectural decisions

| AD | Decision | Rationale | Alternatives considered | Reversibility |
|---|---|---|---|---|
| **AD1** | Reload is **per-process**, not coordinated across binaries | Each binary reads the same manifest file ; operators' rolling-restart playbook propagates the change ; coordinated reload is a distributed-consensus problem (Phase 3) | Two-phase commit ; leader-driven reload | Reversible — operators can keep rolling restart as the official method ; SIGHUP is an additive convenience |
| **AD2** | `Arc<RwLock<Manifest>>` (already in PR #8) is the swap point | One existing field ; readers hold the lock briefly during `.clone()` (PR #8 already does this) ; the write lock is held only for the swap | Crossbeam epoch ; `arc-swap` crate (dep bloat) ; channels | Reversible — `arc-swap` could replace it later if contention shows |
| **AD3** | New invariant : **participant dim / kappa / seed are immutable** at reload | Changing kappa mid-iteration breaks the contraction guarantee ; changing seed breaks bit-identical determinism ; changing dim invalidates the participant's tensor state | Allow with reset ; allow with migration | Reversible — relaxation is additive in PR phase-3 |
| **AD4** | New invariant : **meta-space dim is immutable** at reload | Changing meta dim invalidates every participant's `LinearLift` / `LinearPull` operator ; controller would have to rebuild MetaSwarm from scratch | Allow with full rebuild | Reversible — same path as AD3 |
| **AD5** | Reload **rebuilds the participants set** : adding / removing participants OK ; mutating fields rejected | Adding lets operators scale up ; removing lets them gracefully drop a faulty participant ; mutating is forbidden by AD3 | Forbid set changes (too rigid) ; allow everything (unsafe) | Reversible |
| **AD6** | The controller's `apply_reload(new_manifest)` is **transactional** : validate FIRST, then flip health to `RELOADING`, then swap, then flip back to `SERVING` | Validation failure must leave the running state untouched (no half-swap) ; the `RELOADING` flip is visible to ambient probes for the swap window | Best-effort apply with rollback on failure | Reversible (irrelevant — the transactional design is conservative) |
| **AD7** | `reload_gen` is `AtomicU32` (already on `HealthState`) ; **only bumped on success** | Operators rely on monotonicity to confirm "my reload took" ; bumping on failure would give false positives | Bump on every attempt ; per-attempt counter + success counter | Reversible |
| **AD8** | `RELOADING` window holds the **write lock** ; readers (Iterate / Health) **block briefly** | Simplest correctness model ; window is microseconds for small manifests ; PR #8's `try_read()` in `info_snapshot` returns empty during this window, matching pre-PR-#12 behaviour | Two-phase swap with double-buffer ; `arc-swap` | Reversible — `arc-swap` is one dep away |
| **AD9** | Participant **rejects reload if its own row drifts** | If the participant's own kappa/dim/seed changed, refuse (AD3) ; the binary stays serving with the old manifest, logs an error, returns the error from SIGHUP | Silent log + continue | Reversible |
| **AD10** | SIGHUP **does not block the signal-receiving task** | The handler dispatches to a background reload task ; the binary stays responsive ; concurrent SIGHUPs are coalesced via a `tokio::sync::Notify` | Block in handler ; spawn-per-signal (resource leak) | Reversible |
| **AD11** | `ReloadConfig` RPC body : `request.new_meta.is_none() && request.new_participants.is_empty()` ⇒ re-read manifest from disk ; otherwise apply inline (admin override) | Two operator workflows in one RPC : "I edited the file, reload it" vs "apply this inline, persist later" | Two distinct RPCs | Reversible |
| **AD12** | The inline override path **does not write to disk** | Keeps the RPC stateless ; ops audit trail relies on file edits, not RPC payloads | Persist inline overrides | Reversible — Phase 3 may add `--persist=true` flag |
| **AD13** | PR #11's `MetaSwarmClient::health_probe` is **relaxed to accept `RELOADING (3)`** as transient | Without this, a hot reload would fail-fast `efe-server` boot probes (when PR #15 wires dispatch and re-probes) ; the RELOADING state is brief and equivalent to "serving with degraded freshness" | Reject ; retry with backoff | Reversible — the variant `MetaSwarmClientError::UnhealthyController(3)` is dropped from the rejection set but kept in the enum |
| **AD14** | Reload-related metrics live in the **existing namespaces** : `efe_meta_swarm_*` (controller) + new `efe_meta_participant_*` (participant) | Reuses PR #6's eager-register pattern ; one fan-out histogram per binary tracks reload latency | New `efe_reload_*` namespace | Reversible |
| **AD15** | The integration smoke `tests/integration_reload_e2e.rs` reuses PR #10's `ProcessHarness` for cross-process SIGHUP-driven reload | One harness for all multiprocess tests ; SIGHUP is `libc::kill(pid, SIGHUP)` ; the harness exposes `sigterm_*` already — PR #12 adds `sighup_*` | Standalone harness for reload | Reversible |

---

## 5. Topology + integration diagram

### 5.1 The two reload triggers

```
                  ┌──────────────────────────────────────┐
                  │   Operator                           │
                  └─────────┬──────────────┬─────────────┘
                            │              │
                  (Trigger #1)         (Trigger #2)
                  SIGHUP                MetaSwarmService::ReloadConfig
                  via kill              (grpcurl / efe-server admin)
                            │              │
                            ▼              ▼
                  ┌──────────────────────────────────────┐
                  │   controller binary                  │
                  │   ┌──────────────────────────────┐  │
                  │   │ observability::shutdown       │  │  signal_handler
                  │   │   sighup_handler  ─────────┐ │  │
                  │   │                              │ │  │
                  │   │  service::reload_config ─────┤ │  │  RPC handler
                  │   │                              │ │  │
                  │   │                              ▼ ▼  │
                  │   │  controller::reload::apply_reload  │
                  │   │      1. validate (PR #8)     │ │  │
                  │   │      2. validate_reload_compat (PR #12)
                  │   │      3. health.set_reloading │ │  │
                  │   │      4. state.manifest.swap  │ │  │
                  │   │      5. health.set_serving + bump_reload_gen
                  │   │      6. info!(reload_gen=...) │ │  │
                  │   └──────────────────────────────┘  │
                  └──────────────────────────────────────┘
                            │ (per-process, AD1)
                            ▼
                  ┌──────────────────────────────────────┐
                  │   participant binary (3×)            │
                  │   observability::shutdown            │
                  │     sighup_handler ──> reload        │
                  │   participant::reload::apply_reload  │
                  │     IterationParams swap             │
                  │     (own-row drift rejected, AD9)    │
                  └──────────────────────────────────────┘
```

### 5.2 In-flight Iterate during reload (the key correctness scenario)

```
client                  controller                        participant
   |                        |                                  |
   |  Iterate(req)          |                                  |
   |───────────────────────>|                                  |
   |                        |  state.manifest.read().clone()   |
   |                        |  (captures Arc<Manifest> @ gen=N)│
   |                        |  build MetaSwarm                  |
   |                        |  spawn_blocking { iterate() }     |
   |                        |  Step()─────────────────────────>|
   |                        |                                  |
   |     ┌── SIGHUP ────────┤                                  │
   |     │                  │ apply_reload()                   │
   |     │                  │   validate                       │
   |     │                  │   health.set_reloading           │
   |     │                  │   manifest.write() ← gen=N+1     │
   |     │                  │   health.set_serving             │
   |     │                  │   bump_reload_gen()              │
   |     │                  │                                  │
   |                        │   (Iterate continues with gen=N    │
   |                        │    snapshot — Arc captured earlier)│
   |                        │   Step()────────────────────────>│
   |                        │   Step()────────────────────────>│
   |                        │   ...                            │
   |                        │   final_residual                 │
   |<── IterateResponse ────┤                                  │
   |                        │                                  │
   |  Iterate(req')         │                                  │
   |───────────────────────>│ state.manifest.read().clone()    │
   |                        │ (captures gen=N+1)               │
   |                        │ ... runs against new manifest    │
```

In-flight Iterates **never see** a half-swapped manifest because they
captured `Arc<Manifest>` at iteration start (PR #8's `run_iterate` already
does `state.manifest.read().await.clone()`). The Arc keeps the OLD manifest
alive for the duration of the iteration even after the write-lock swap.
This is the **load-bearing invariant** of PR #12.

### 5.3 efe-server probe acceptance during reload

```
efe-server boot      controller
   |                    │
   | health probe ─────>│  state = RELOADING (3)
   |                    │
   |<── HealthResponse(status=3) ─┤
   |
   | (AD13) accept as transient ; do NOT fail-fast ;
   |        proceed with boot.
```

---

## 6. Crate layout additions

### 6.1 `efe-meta-swarm-server` after PR #12

```
crates/efe-meta-swarm-server/src/
├── controller/
│   ├── mod.rs                    [MODIFIED : + pub mod reload]
│   ├── service.rs                [MODIFIED : wire ReloadConfig]
│   ├── state.rs                  (unchanged ; PR #8 already gave us Arc<RwLock<Manifest>>)
│   ├── iterate.rs                (unchanged ; clones the Arc at iteration start)
│   ├── inflight.rs               (unchanged)
│   └── reload.rs                 [NEW : ~ 220 LOC, apply_reload + tests]
├── participant/
│   ├── mod.rs                    [MODIFIED : + pub mod reload]
│   ├── service.rs                (unchanged)
│   ├── state.rs                  [MODIFIED : `iteration: ArcSwap<IterationParams>` semantics doc]
│   ├── local.rs                  (unchanged — LocalParticipant is immutable post-construction)
│   ├── validate.rs               (unchanged)
│   └── reload.rs                 [NEW : ~ 220 LOC, apply_reload + tests]
├── manifest/
│   ├── mod.rs                    [MODIFIED : + validate_reload_compatibility]
│   └── schema.rs                 (unchanged)
├── observability/
│   ├── mod.rs                    (unchanged)
│   ├── shutdown.rs               [MODIFIED : + sighup_handler]
│   ├── health.rs                 (unchanged ; set_reloading + bump_reload_gen exist)
│   ├── metrics.rs                [MODIFIED : + reload_attempts/reload_failures counters]
│   └── tracing.rs                (unchanged)
└── bin/
    ├── controller.rs             [MODIFIED : install SIGHUP handler]
    └── participant.rs            [MODIFIED : install SIGHUP handler]

tests/
├── integration_reload_e2e.rs     [NEW : ~ 280 LOC, 5 tests]
└── (all other PR #5–#11 tests untouched)
```

### 6.2 `efe-server` after PR #12

```
crates/efe-server/src/services/meta_swarm/
├── mod.rs                        [MODIFIED : probe accepts RELOADING (3)]
└── metrics.rs                    (unchanged)

crates/efe-server/tests/
└── integration_meta_swarm_plumbing.rs  [MODIFIED : + 2 RELOADING-acceptance tests]
```

---

## 7. The manifest atomic-swap protocol

### 7.1 Invariant the protocol upholds

> Every in-flight `Iterate` / `Step` call sees a **single, consistent
> snapshot** of the manifest from the moment it started until the moment
> it returns.  A new manifest applied during the call is invisible to it.

### 7.2 Mechanism

PR #8 already gave us `state.manifest: Arc<RwLock<Manifest>>`.  PR #8's
`run_iterate` does :

```rust
let manifest = state.manifest.read().await.clone();
```

This :
  1. acquires the read lock (multi-reader),
  2. clones the inner `Manifest` (Phase-2 manifests are < 1 KB ; clone is
     cheap),
  3. releases the read lock.

The local `manifest: Manifest` owned by `run_iterate` lives until the call
returns.  PR #12's reload acquires the **write lock** to perform the
swap — readers that arrive during the write-lock window wait briefly,
readers that arrived BEFORE the window are unaffected (they already hold
their cloned snapshot).

This is the **canonical RwLock single-writer / many-readers pattern** with
the cheap `.clone()` extending the snapshot lifetime past the lock
duration.

### 7.3 What we do NOT do (and why)

  * **Drain in-flight Iterates first** — PR #12 architecture rejects this :
    the longest Iterate is bounded by `max_wall_ms = 30 s` (PR #8 hard
    cap), which would mean operators wait 30 s for a hot-reload.
  * **Stop accepting new Iterates** — PR #12 rejects this : the
    `RESOURCE_EXHAUSTED` window would cascade to clients, which is worse
    than the (~µs) write-lock contention.
  * **Two-phase commit with new readers seeing v2 immediately** — PR #12
    rejects this because the readers that arrived BEFORE the swap MUST
    still see v1 (in-flight correctness, § 5.2).

### 7.4 The write-lock window — how long exactly?

Measured on PR #8's `Manifest` (3 participants, dim ≤ 16, < 1 KB) :

  | Step | Wall-clock time |
  |---|---|
  | `validate_manifest` (PR #8) | < 50 µs |
  | `validate_reload_compatibility` (PR #12 new) | < 20 µs |
  | `health.set_reloading` (atomic store) | < 1 µs |
  | `*write_guard = new_manifest` | < 1 µs |
  | `health.set_serving + bump_reload_gen` | < 2 µs |
  | **Total (write lock held)** | **< 25 µs** |

The validation runs **before** acquiring the write lock, so a bad manifest
doesn't block readers at all.  The write lock window is purely the
atomic-swap + health flip.

---

## 8. SIGHUP handler — `observability/shutdown.rs` extension

### 8.1 Current state (PR #6)

```rust
pub async fn await_termination() -> &'static str { /* SIGTERM + SIGINT */ }
```

### 8.2 After PR #12

```rust
//! Unix signal handling.
//!
//! PR #6 shipped `await_termination()` (SIGTERM + SIGINT).  PR #12 adds
//! `install_sighup_handler(callback)` for live reload.  PR #18 will add
//! `await_sigusr1()` for debug-dump.

use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::Notify;
use std::sync::Arc;

// (existing) await_termination — unchanged.

/// Install a SIGHUP handler that fires `callback` once per signal.
///
/// Concurrent SIGHUPs are **coalesced** through a `tokio::sync::Notify` :
/// the handler wakes the background reload task once, and subsequent
/// SIGHUPs while a reload is in flight are merged into a single
/// follow-up reload at the end.
///
/// The returned `JoinHandle` lives for the binary's lifetime ; tearing
/// it down on shutdown happens implicitly when the tokio runtime drops.
#[cfg(unix)]
pub fn install_sighup_handler<F, Fut>(callback: F) -> tokio::task::JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let notify = Arc::new(Notify::new());

    // Signal-listener task : every SIGHUP bumps the Notify ; coalescing
    // is automatic (Notify wakes ONE waiter ; pending notifies are
    // collapsed).
    let notify_signal = Arc::clone(&notify);
    tokio::spawn(async move {
        let mut sighup = signal(SignalKind::hangup())
            .expect("install SIGHUP handler");
        while sighup.recv().await.is_some() {
            tracing::info!("SIGHUP received");
            notify_signal.notify_one();
        }
    });

    // Worker task : awaits the Notify, fires the callback ; concurrent
    // SIGHUPs that arrive during the callback are coalesced (Notify
    // only wakes one waiter ; further notifies are queued for the
    // next await).
    let notify_worker = Arc::clone(&notify);
    tokio::spawn(async move {
        loop {
            notify_worker.notified().await;
            callback().await;
        }
    })
}

#[cfg(not(unix))]
pub fn install_sighup_handler<F, Fut>(_callback: F) -> tokio::task::JoinHandle<()>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    // SIGHUP is Unix-only ; on non-Unix targets we return a future that
    // never resolves.  Phase-2 deployment target is Linux server — this
    // arm is purely to keep `cargo check` green on macOS/Windows.
    tokio::spawn(async {
        std::future::pending::<()>().await;
    })
}
```

### 8.3 Why `Notify`-based coalescing

A naïve loop that spawns a fresh reload-task per SIGHUP would :

  1. Run two reloads back-to-back even if the operator double-tapped SIGHUP
     by mistake.
  2. Allow a reload to start while another is mid-flight → write-lock
     contention storm.

`Notify::notify_one` collapses multiple signals into one wake-up.  If a
SIGHUP arrives while the worker is running the callback, the next
`notified().await` resolves immediately afterwards — exactly one
follow-up reload is guaranteed.

### 8.4 Unit tests (in `observability::shutdown::tests`)

  1. `sighup_handler_fires_callback_once_per_signal`.
  2. `concurrent_sighups_are_coalesced` — fire 3 SIGHUPs while callback
     is sleeping ; expect 1 follow-up (not 3).
  3. `handler_survives_callback_panic` — wrap the callback in
     `panic = "unwind"`-compatible scaffolding ; if the worker task
     panics, replace it (or document the limitation).

PR #12 chooses **document the limitation** — a panicking callback is a
production bug ; the operator should redeploy.  AD-list § AD10.

---

## 9. `controller::reload` module (NEW)

### 9.1 Public surface

```rust
//! Phase-2 PR #12 — controller-side reload protocol.
//!
//! Two trigger paths share this module's `apply_reload` :
//!   * SIGHUP : `bin/controller.rs` installs the handler, which calls
//!     `apply_reload_from_disk(state)` on every signal.
//!   * gRPC : `controller::service::reload_config` calls
//!     `apply_reload_with_request(state, req)` directly.

use std::sync::Arc;

use crate::controller::state::ControllerState;
use crate::error::ServerError;
use crate::manifest::{
    self, validate_manifest, validate_reload_compatibility, Manifest,
};

/// Result of a reload attempt.  Returned by both `apply_reload_*`
/// variants and surfaced in the `ReloadConfigResponse` / SIGHUP log.
#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    /// The new monotonic generation counter (`HealthState.reload_gen`).
    pub new_reload_gen: u32,
    /// Number of participants in the new manifest.
    pub participants: usize,
    /// Wall-clock time of the swap, microseconds.
    pub elapsed_us: u64,
}

/// Re-read the manifest from `state.config.manifest_path` and apply it.
/// Used by SIGHUP — the operator's source of truth is the on-disk file.
///
/// # Errors
///
/// Returns [`ServerError::Manifest`] if the file can't be loaded, can't
/// be validated, or is incompatible with the running state.  In every
/// error case, the running state is **untouched** (transactional).
pub async fn apply_reload_from_disk(
    state: &ControllerState,
) -> Result<ReloadOutcome, ServerError> {
    let path = &state.config.manifest_path;
    let new_manifest = manifest::load_and_validate(path)?;
    apply_reload(state, new_manifest).await
}

/// Apply an in-memory new manifest.  Used by the gRPC RPC :
///   * empty request body → caller calls `apply_reload_from_disk` instead ;
///   * non-empty request body → caller builds the new manifest from the
///     request payload, then calls this.
pub async fn apply_reload_with_manifest(
    state: &ControllerState,
    new_manifest: Manifest,
) -> Result<ReloadOutcome, ServerError> {
    validate_manifest(&new_manifest)?;
    apply_reload(state, new_manifest).await
}

/// Shared inner path : assumes `new_manifest` already passed
/// `validate_manifest`.  Now checks reload-compatibility against the
/// current manifest, flips health, swaps, flips back, bumps gen.
async fn apply_reload(
    state: &ControllerState,
    new_manifest: Manifest,
) -> Result<ReloadOutcome, ServerError> {
    let start = std::time::Instant::now();

    // 1. Reload-specific compatibility check (PR #12 AD3/AD4/AD5).
    //    Done OUTSIDE the write lock so a rejected reload doesn't
    //    block readers at all.
    {
        let current = state.manifest.read().await;
        validate_reload_compatibility(&current, &new_manifest)?;
    } // read guard dropped

    // 2. Flip health to RELOADING — visible to ambient probes.
    state.health.set_reloading();

    // 3. Acquire write lock + atomic swap.
    let participants = new_manifest.participants.len();
    {
        let mut write_guard = state.manifest.write().await;
        *write_guard = new_manifest;
    } // write guard dropped — readers can proceed

    // 4. Flip health back to SERVING + bump gen.
    state.health.set_serving();
    let new_reload_gen = state.health.bump_reload_gen();

    // 5. Metrics + structured log.
    let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    state.metrics.record_reload_success(elapsed_us);
    tracing::info!(
        new_reload_gen,
        participants,
        elapsed_us,
        "controller manifest reloaded",
    );

    Ok(ReloadOutcome { new_reload_gen, participants, elapsed_us })
}
```

### 9.2 Why `validate_reload_compatibility` runs before the write lock

A rejected reload should produce **zero observable change** in the
running state.  If the validation ran inside the write lock, readers
would still block briefly (and `info_snapshot` would see RELOADING for
no reason).  Running it outside lets the rejection be free.

### 9.3 Unit tests (≥ 6)

  1. `apply_reload_happy_path_bumps_gen_and_swaps_manifest`.
  2. `apply_reload_rejects_dim_change_for_existing_participant`.
  3. `apply_reload_rejects_kappa_change_for_existing_participant`.
  4. `apply_reload_rejects_meta_dim_change`.
  5. `apply_reload_accepts_added_participant`.
  6. `apply_reload_accepts_removed_participant`.
  7. `apply_reload_failed_validation_leaves_state_untouched`.
  8. `apply_reload_concurrent_iterate_sees_consistent_snapshot` (uses tokio
     join — captures Arc before reload, asserts it has the old values).

---

## 10. `controller/service.rs::reload_config` wire

### 10.1 Current (PR #6 stub)

```rust
async fn reload_config(/* … */) -> Result<…, Status> {
    Err(Status::unimplemented(
        "MetaSwarmService::ReloadConfig — wired in PR #12",
    ))
}
```

### 10.2 After PR #12

```rust
#[instrument(skip(self, req), fields(inline_meta = ?req.get_ref().new_meta.is_some()))]
async fn reload_config(
    &self,
    req: Request<ReloadConfigRequest>,
) -> Result<Response<ReloadConfigResponse>, Status> {
    let req_inner = req.into_inner();

    // Empty request → re-read from disk (AD11).
    let outcome = if req_inner.new_meta.is_none() && req_inner.new_participants.is_empty() {
        crate::controller::reload::apply_reload_from_disk(&self.state)
            .await
            .map_err(Status::from)?
    } else {
        // Inline override : build a fresh Manifest from the request
        // payload + the current manifest's iteration block (the proto
        // doesn't carry iteration knobs in ReloadConfigRequest).
        let current_iteration = {
            let m = self.state.manifest.read().await;
            m.iteration.clone()
        };
        let new_manifest = build_inline_manifest(&req_inner, current_iteration)
            .map_err(Status::from)?;
        crate::controller::reload::apply_reload_with_manifest(&self.state, new_manifest)
            .await
            .map_err(Status::from)?
    };

    Ok(Response::new(ReloadConfigResponse {
        reload_gen: outcome.new_reload_gen,
        elapsed_us: outcome.elapsed_us,
        participants: u32::try_from(outcome.participants).unwrap_or(u32::MAX),
    }))
}
```

### 10.3 Tests (in `controller::service::tests`)

PR #12 replaces the old `reload_config_returns_unimplemented_with_pr12_message`
test with :

  1. `reload_config_with_empty_request_loads_from_disk` — uses test_state's
     manifest_path (a tempfile) ; assert gen bumps.
  2. `reload_config_with_inline_meta_applies_inline` — non-empty
     `new_meta` ; assert state.manifest reflects the override.
  3. `reload_config_returns_failed_precondition_on_bad_manifest` — bad
     YAML on disk ; assert FAILED_PRECONDITION.
  4. `reload_config_rejects_dim_drift_with_failed_precondition` — inline
     manifest with mutated dim ; assert reject.

---

## 11. `participant::reload` module (NEW)

### 11.1 Public surface

```rust
//! Phase-2 PR #12 — participant-side reload protocol.
//!
//! Triggered exclusively by SIGHUP (no participant-side gRPC reload
//! RPC in Phase 2).  The participant has a narrower swap surface than
//! the controller :
//!
//!   * `iteration: Arc<IterationParams>` — swappable (α, γ knobs).
//!   * `local: LocalParticipant`         — IMMUTABLE (κ, dim, seed
//!                                          are baked into `MockParticipant`
//!                                          at construction time).
//!
//! Any reload that would require mutating the participant's own row
//! is rejected with `ServerError::Manifest("…")` ; the binary keeps
//! serving with the old IterationParams.

use std::sync::Arc;

use crate::error::ServerError;
use crate::manifest::{self, find_participant, validate_manifest, Manifest};
use crate::participant::state::{IterationParams, ParticipantState};

#[derive(Debug, Clone)]
pub struct ReloadOutcome {
    pub new_reload_gen: u32,
    pub elapsed_us: u64,
}

/// Re-read manifest from disk + apply.  Called by SIGHUP handler.
pub async fn apply_reload_from_disk(
    state: &ParticipantState,
) -> Result<ReloadOutcome, ServerError> {
    let path_var = std::env::var_os("EFE_META_SWARM_MANIFEST")
        .ok_or_else(|| ServerError::Config(
            "EFE_META_SWARM_MANIFEST not set ; cannot reload".into(),
        ))?;
    let path = std::path::PathBuf::from(path_var);
    let new_manifest = manifest::load_and_validate(&path)?;
    apply_reload(state, new_manifest).await
}

async fn apply_reload(
    state: &ParticipantState,
    new_manifest: Manifest,
) -> Result<ReloadOutcome, ServerError> {
    let start = std::time::Instant::now();

    // 1. Find own row (must exist).
    let own_id = state.local.id();
    let new_row = find_participant(&new_manifest, own_id).ok_or_else(|| {
        ServerError::Manifest(format!(
            "reload : own participant id {:?} not in new manifest",
            own_id,
        ))
    })?;

    // 2. Reject own-row drift (AD3 / AD9).
    if new_row.dim as usize != state.local.dim()
        || (new_row.kappa - state.local.kappa()).abs() > 1.0e-6
        || new_row.seed != state.local.seed()
    {
        return Err(ServerError::Manifest(format!(
            "reload : own row {:?} drifted (dim {} → {}, kappa {} → {}, seed {} → {}) — \
             not supported in Phase 2",
            own_id,
            state.local.dim(), new_row.dim,
            state.local.kappa(), new_row.kappa,
            state.local.seed(), new_row.seed,
        )));
    }

    // 3. Flip health to RELOADING.
    state.health.set_reloading();

    // 4. Build new IterationParams + atomic swap via Arc::swap.
    //    `state.iteration` is `Arc<IterationParams>` ; we wrap the new
    //    params in a fresh Arc and assign through a writable handle.
    //    PR #9 designed ParticipantState with this in mind.
    let new_iter = IterationParams {
        alpha_default: new_manifest.iteration.alpha_default,
        coupling_gamma: new_manifest.iteration.coupling_gamma,
    };

    // The swap mechanism : `ParticipantState.iteration_swap()` (new
    // method ; encapsulates the unsafe-looking Arc store).
    state.iteration_swap(new_iter);

    // 5. Flip back + bump gen.
    state.health.set_serving();
    let new_reload_gen = state.health.bump_reload_gen();
    let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);

    tracing::info!(
        new_reload_gen,
        elapsed_us,
        own_id,
        "participant manifest reloaded",
    );

    Ok(ReloadOutcome { new_reload_gen, elapsed_us })
}
```

### 11.2 Why `LocalParticipant` is immutable

PR #9 built `LocalParticipant` as a thin wrapper over Phase-1's
`MockParticipant::with_contraction(id, dim, kappa, seed)`.  The contraction
constant κ is baked into the `phi` closure ; the dim into the tensor shape ;
the seed into the initial state.  Changing any of these requires
rebuilding the `MockParticipant`, which **invalidates the existing
participant state** (and breaks bit-identical determinism if mid-iteration).

For Phase 2 we forbid this.  Phase 3 may add a `LocalParticipant::rebuild`
path for operator-initiated state resets.

### 11.3 Unit tests (≥ 5)

  1. `apply_reload_swaps_alpha_and_gamma`.
  2. `apply_reload_rejects_own_kappa_drift`.
  3. `apply_reload_rejects_own_dim_drift`.
  4. `apply_reload_rejects_own_seed_drift`.
  5. `apply_reload_rejects_own_id_missing_in_new_manifest`.
  6. `apply_reload_bumps_gen_on_success`.
  7. `apply_reload_concurrent_step_sees_consistent_iteration_params`.

---

## 12. `participant/state.rs::IterationParams` atomic swap

### 12.1 Why `Arc<IterationParams>` is **almost** enough but not quite

PR #9 set the field to `Arc<IterationParams>`.  But `Arc<T>` itself is
immutable — to swap the underlying T, we need either :

  * **`ArcSwap<IterationParams>`** from the `arc-swap` crate — clean
    atomic swap, lock-free read.
  * **`RwLock<Arc<IterationParams>>`** — read locks for handlers, write
    lock for the swap.
  * **`AtomicPtr<IterationParams>`** — unsafe-heavy.

PR #12 chooses **`ArcSwap`** (option 1) — it's a thin abstraction (~ 200
LOC dep), zero contention on the read path, and the swap is a single
atomic-pointer store.

### 12.2 Cargo.toml addition

```toml
[dependencies]
# … existing …
arc-swap = "1.7"   # PR #12 : lock-free atomic Arc swap for IterationParams reload
```

### 12.3 Field type change

```rust
// PR #9 :
pub iteration: Arc<IterationParams>,

// PR #12 :
pub iteration: Arc<arc_swap::ArcSwap<IterationParams>>,
```

The outer `Arc` lets `ParticipantState` itself be `Clone` (it is — PR #9
relies on this).  The inner `ArcSwap` lets us swap the IterationParams
without taking a lock.

### 12.4 Read path (in `service.rs::step` handler)

```rust
// PR #9 :
let gamma = self.state.iteration.coupling_gamma;
let alpha = self.state.iteration.alpha_default;

// PR #12 :
let iter = self.state.iteration.load();  // ArcSwap::load returns Guard<Arc<T>>
let gamma = iter.coupling_gamma;
let alpha = iter.alpha_default;
```

`ArcSwap::load` is **lock-free** — readers never block.

### 12.5 Swap path (in `reload.rs`)

```rust
pub fn iteration_swap(&self, new: IterationParams) {
    self.iteration.store(Arc::new(new));
}
```

One atomic store ; nothing more.

### 12.6 Why this is correct under concurrent in-flight Steps

A Step handler that already called `iteration.load()` holds an
`Arc<IterationParams>` snapshot.  A concurrent `iteration_swap` replaces
the slot's contents, but the snapshot remains valid until the handler's
clone drops.  Identical correctness guarantee to PR #8's controller-side
`Arc<RwLock<Manifest>>` + `.clone()` pattern, just lock-free.

---

## 13. Health status `RELOADING` semantics

### 13.1 What `RELOADING (3)` means

  * **Operator semantic** : "the process is alive and accepting traffic,
    but is currently swapping its manifest.  Treat it as healthy with
    degraded freshness."
  * **Protocol semantic** : ambient probes (efe-server, K8s liveness)
    SHOULD treat it as alive ; load balancers MAY temporarily skip it.

### 13.2 Where it's set

  * `controller::reload::apply_reload` sets it just before the write
    lock and resets it just after the gen bump.
  * `participant::reload::apply_reload` same.

### 13.3 Where it's read

  * `Health` RPC : returns `HealthResponse.status = RELOADING (3)`
    during the window.
  * efe-server's `MetaSwarmClient::health_probe` (PR #11) : **must be
    relaxed** to accept it (cf. § 14).

### 13.4 Window width

< 25 µs typical (cf. § 7.4).  Probes that miss the window see SERVING.
This is by design — the operator's primary signal is `reload_gen`, not
the status flip.

---

## 14. `efe-server` `MetaSwarmClient::health_probe` accepts `RELOADING`

### 14.1 PR #11 behaviour (the bug PR #12 fixes)

```rust
if resp.status == 1 {
    Ok(())
} else {
    Err(MetaSwarmClientError::UnhealthyController(resp.status))
}
```

Rejects RELOADING (3) → spurious fail-fast during reload.

### 14.2 PR #12 behaviour

```rust
match resp.status {
    1 => Ok(()),  // SERVING — fully healthy.
    3 => {
        debug!(
            "controller is RELOADING — accepting as transient \
             (PR #12 AD13)",
        );
        Ok(())
    }
    other => Err(MetaSwarmClientError::UnhealthyController(other)),
}
```

### 14.3 Why not retry-with-backoff

  * Adds complexity (timer + jitter + cap).
  * The RELOADING window is < 25 µs — by the time the retry timer fires,
    we'd be in SERVING anyway.
  * Acceptance is operationally equivalent to "this controller is fine for
    your purposes" because in-flight Iterates (PR #12 § 5.2) tolerate
    the swap.

### 14.4 Updated tests in `efe-server`

  * `health_probe_accepts_reloading_as_transient` — mock returns status=3 ;
    assert `Ok(())`.
  * `health_probe_still_rejects_not_serving` — mock returns status=2 ;
    assert `UnhealthyController(2)`.

---

## 15. Determinism contract under reload

### 15.1 The new clause

> Iteration outputs are deterministic across reloads **IFF** :
>
>   1. The new manifest's `iteration` knobs match the old one's, AND
>   2. The participants set is unchanged (no add / remove), AND
>   3. The reload happens **between** Iterate calls, not during one.

If 3 is violated (reload during a single Iterate), the in-flight call
completes with the OLD manifest (PR #12 § 5.2) — the NEXT Iterate sees
the NEW one.  The determinism contract is over **per-call** behaviour
given a snapshot, not over a sequence of reloads.

### 15.2 What this gives the operator

  * Pure α/γ tweaks at reload time are deterministic-equivalent to
    rolling-restart with the new values.
  * Adding a participant at reload time changes the manifest's
    participants set → the NEXT Iterate produces different results than
    pre-reload.  This is expected.

### 15.3 Integration test for the contract

`tests/integration_reload_e2e.rs::iterate_after_reload_is_consistent_with_inline_manifest`
:

  1. Spawn controller + 3 participants (PR #10 harness).
  2. Run Iterate → record `(iterations_v1, residual_v1.to_bits())`.
  3. Edit the manifest tempfile : bump `alpha_default` from 0.5 → 0.6.
  4. Send SIGHUP.
  5. Wait for `reload_gen == 1`.
  6. Run Iterate → record `(iterations_v2, residual_v2.to_bits())`.
  7. Spawn a FRESH controller + 3 participants with `alpha_default = 0.6`
     from the start.
  8. Run Iterate → record `(iterations_v3, residual_v3.to_bits())`.
  9. Assert `(iterations_v2, residual_v2) == (iterations_v3, residual_v3)`
     (reload-then-iterate ≡ fresh-with-new-manifest).

---

## 16. What CAN change vs CANNOT change in a reload

### 16.1 Allowed

| Field | Rationale |
|---|---|
| `iteration.max_iters` | Pure scheduling knob |
| `iteration.tolerance` | Pure convergence knob |
| `iteration.alpha_default` | Per-step convex weight |
| `iteration.coupling_gamma` | Per-step coupling weight |
| `iteration.max_wall_ms` | Per-Iterate deadline |
| `iteration.deterministic` | Toggling implementation strategy ; doesn't break correctness |
| `participants` — add a row | Controller reconnects on next Iterate |
| `participants` — remove a row | Controller drops the connection on next Iterate |

### 16.2 Forbidden (rejected with `ServerError::Manifest`)

| Field | Rationale |
|---|---|
| `meta.dim` | Invalidates every participant's Lift/Pull operator |
| `meta.safety.kind` (Box → Poly) | Invalidates the safety projection contract |
| `meta.safety.radius` (Box) | **Allowed actually — wait, no** : invalidates in-flight Iterate's safety check ; refused per AD4 |
| Existing participant's `dim` | Invalidates the participant's tensor state |
| Existing participant's `kappa` | Invalidates contraction guarantee |
| Existing participant's `seed` | Invalidates determinism |
| `version` | Manifest schema version mismatch ⇒ caller bug ; surface it |

### 16.3 `validate_reload_compatibility` implementation

```rust
pub fn validate_reload_compatibility(
    current: &Manifest,
    new: &Manifest,
) -> Result<(), ServerError> {
    use crate::manifest::SafetyManifest;

    if current.version != new.version {
        return Err(ServerError::Manifest(format!(
            "reload : manifest version drift {:?} → {:?} (not supported)",
            current.version, new.version,
        )));
    }
    if current.meta.dim != new.meta.dim {
        return Err(ServerError::Manifest(format!(
            "reload : meta.dim cannot change ({} → {})",
            current.meta.dim, new.meta.dim,
        )));
    }
    let same_kind = matches!(
        (&current.meta.safety, &new.meta.safety),
        (SafetyManifest::Box { .. }, SafetyManifest::Box { .. })
        | (SafetyManifest::Poly { .. }, SafetyManifest::Poly { .. })
        | (SafetyManifest::Hull { .. }, SafetyManifest::Hull { .. })
    );
    if !same_kind {
        return Err(ServerError::Manifest(
            "reload : meta.safety.kind cannot change".into(),
        ));
    }
    // safety params (radius / poly coefficients / hull data) MAY change ;
    // we accept silently.

    // For every participant in `current` that's ALSO in `new`, check
    // that dim / kappa / seed are byte-identical.  Removed and added
    // participants are allowed.
    for old_p in &current.participants {
        if let Some(new_p) = new.participants.iter().find(|p| p.id == old_p.id) {
            if old_p.dim != new_p.dim {
                return Err(ServerError::Manifest(format!(
                    "reload : participant {:?} dim drift {} → {}",
                    old_p.id, old_p.dim, new_p.dim,
                )));
            }
            if (old_p.kappa - new_p.kappa).abs() > 1.0e-6 || old_p.kappa.is_nan() != new_p.kappa.is_nan() {
                return Err(ServerError::Manifest(format!(
                    "reload : participant {:?} kappa drift {} → {}",
                    old_p.id, old_p.kappa, new_p.kappa,
                )));
            }
            if old_p.seed != new_p.seed {
                return Err(ServerError::Manifest(format!(
                    "reload : participant {:?} seed drift {} → {}",
                    old_p.id, old_p.seed, new_p.seed,
                )));
            }
        }
    }

    Ok(())
}
```

### 16.4 Tests for validate_reload_compatibility (≥ 8)

  1. `accepts_identical_manifest` (no-op reload).
  2. `accepts_iteration_alpha_change`.
  3. `accepts_added_participant`.
  4. `accepts_removed_participant`.
  5. `rejects_meta_dim_change`.
  6. `rejects_existing_participant_dim_change`.
  7. `rejects_existing_participant_kappa_change`.
  8. `rejects_existing_participant_seed_change`.
  9. `rejects_safety_kind_change`.
 10. `accepts_safety_radius_change_within_same_kind`.

---

## 17. Threat-model coverage

PR #12 ships **three new threats**, all in the `T19-T21` band, all
mitigated.

| Threat | Severity | PR #12 mitigation |
|---|---|---|
| **T19** (malicious manifest swap — operator with file write access deploys a sabotaged manifest) | SEV-2 | `validate_manifest` (PR #8) + `validate_reload_compatibility` (PR #12) catch structural and drift attacks ; the reload is fail-fast at validation time ; the on-disk file is the canonical artefact and is subject to GitOps audit (Phase-3 ops concern) |
| **T20** (race during reload — in-flight Iterate sees inconsistent state) | SEV-1 | The `Arc<Manifest>` snapshot pattern (PR #8 already does `.clone()`) plus the brief write-lock window prove (by construction) that no in-flight call can observe a half-swapped manifest ; integration test `iterate_during_concurrent_sighup_uses_old_manifest` exercises this end-to-end |
| **T21** (reload-flood DoS via repeated SIGHUP or ReloadConfig RPC) | SEV-3 | SIGHUP is coalesced via `Notify::notify_one` (§ 8.2) so a flood becomes at most one in-flight + one queued reload ; gRPC ReloadConfig is rate-limited at the Auth layer (PR #16) ; **PR #12** adds `efe_meta_swarm_reload_attempts_total` so operators can alert on anomalous spikes |

### 17.1 What PR #12 does NOT defend against (deferred)

| Threat | Where | When |
|---|---|---|
| Concurrent operators reload at the same time (race in their workflows, not in our code) | Operator playbook | Phase 3 ops doc |
| Manifest file written non-atomically (partial truncation read) | Operator's CI/CD | PR #18 deployment manifests recommend `mv` |
| ReloadConfig from unauthenticated client | Auth | PR #16 |

---

## 18. Concurrency model + lock hierarchy

### 18.1 Updated lock-acquisition order

  1. **InflightSemaphore** (controller MAX_INFLIGHT — PR #8).
  2. **`state.manifest` read** (Iterate path) OR **`state.manifest` write**
     (reload path).
  3. Per-participant client mutex (PR #8 RemoteParticipant).

The new write-lock acquisition (point 2 in the reload path) does NOT
require holding the InflightSemaphore — reload is independent of inflight
throttling.  Operator-driven reloads happen regardless of how many
Iterates are running.

### 18.2 Why no deadlock

  * The reload path acquires `manifest.write()` while holding **nothing
    else** (it doesn't hold the InflightSemaphore or any participant
    mutex).
  * The Iterate path acquires `manifest.read()`, then drops it (the clone
    extends the snapshot lifetime), then acquires participant mutexes.
  * These two paths never hold each other's locks transitively → no
    deadlock.

### 18.3 Atomic counters

`reload_gen` (`AtomicU32`, `Relaxed`) and `step_index_last` / `reset_gen`
(PR #9) are unaffected by the reload path.  Same ordering arguments as
before.

---

## 19. Metrics + tracing

### 19.1 New Prometheus surface

| Metric | Type | Labels | Description |
|---|---|---|---|
| `efe_meta_swarm_reload_attempts_total` | Counter | (none) | total reload attempts (success + failure) |
| `efe_meta_swarm_reload_failures_total` | Counter | `{reason}` (validation, io, drift) | total reload attempts that returned an error |
| `efe_meta_swarm_reload_duration_microseconds` | Histogram | (none) | wall-clock time of the swap |
| `efe_meta_swarm_reload_gen` | Gauge | (none) | current `reload_gen` value (mirrors `HealthState.reload_gen`) |

### 19.2 Tracing spans

`apply_reload` is `#[instrument(skip(state), fields(participants = ...))]`.
Span fields :
  * `participants` — new manifest's participant count.
  * `new_reload_gen` — post-bump gen.
  * `elapsed_us` — write-lock window duration.

### 19.3 Log lines

`info!` on every successful reload :
```
INFO controller manifest reloaded new_reload_gen=42 participants=3 elapsed_us=18
```

`error!` on every failed reload :
```
ERROR reload rejected reason="participant supplier-scoring kappa drift 0.6 → 0.7"
```

---

## 20. Tests strategy

### 20.1 Unit tests

| File | Tests added | Coverage focus |
|---|---|---|
| `manifest/mod.rs::tests` | ≥ 10 | `validate_reload_compatibility` accept / reject matrix |
| `controller/reload.rs::tests` | ≥ 8 | happy / drift reject / failed-validation no-op / concurrent-snapshot |
| `participant/reload.rs::tests` | ≥ 7 | iteration swap / own-row drift reject / gen bump |
| `observability/shutdown.rs::tests` | ≥ 3 | signal-fires-callback / coalescing / unix-gated |
| `controller/service.rs::tests` | 4 modified, 4 new | RPC : empty / inline / bad disk / dim drift |
| `participant/service.rs::tests` | (no change) | — |
| **Total new unit tests** | **≥ 32** | |

### 20.2 Integration tests

`tests/integration_reload_e2e.rs` (NEW, ~ 280 LOC) :

  1. `sighup_to_controller_bumps_reload_gen` — `ProcessHarness::sighup_controller()` + assert `Health.reload_gen == 1`.
  2. `sighup_to_participant_swaps_iteration_params` — same with participant + assert `step_index_last` continuity.
  3. `reload_config_rpc_returns_new_gen` — gRPC call → returns gen + matches Health.
  4. `iterate_during_concurrent_sighup_uses_old_manifest` — start Iterate, SIGHUP mid-flight, assert Iterate completes with pre-reload α/γ.
  5. `reload_after_inline_alpha_change_matches_fresh_topology` (§ 15.3).

`tests/common/multiprocess.rs` is **modified** : add `sighup_controller(&self)` + `sighup_participant(&self, id)` helpers analogous to PR #10's `sigterm_*`.

`crates/efe-server/tests/integration_meta_swarm_plumbing.rs` is **modified** :

  6. `health_probe_accepts_reloading_status` — mock returns 3 → probe OK.
  7. `health_probe_rejects_not_serving_status` (existing) — still rejects 2.

### 20.3 Coverage matrix

| Threat | Surface | Test |
|---|---|---|
| T19 | manifest validation | `validate_reload_compatibility` × 10 unit tests |
| T20 | in-flight Iterate snapshot | `iterate_during_concurrent_sighup_uses_old_manifest` integration |
| T21 | reload-flood coalescing | `concurrent_sighups_are_coalesced` unit |

---

## 21. Acceptance criteria (AC1–AC28)

PR #12 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `cargo build -p efe-meta-swarm-server --release` succeeds | CI |
| AC2 | `cargo build -p efe-server --release` succeeds | CI |
| AC3 | `cargo test -p efe-meta-swarm-server` is **150 + ≥ 32 + 5 = ≥ 187** tests | CI |
| AC4 | `cargo test -p efe-server` is **733 lib + 11 + 2 = ≥ 746** tests | CI |
| AC5 | `validate_reload_compatibility` : 10 tests pass | CI |
| AC6 | `controller::reload::tests` : ≥ 8 tests pass | CI |
| AC7 | `participant::reload::tests` : ≥ 7 tests pass | CI |
| AC8 | `observability::shutdown::tests` : ≥ 3 new tests pass | CI |
| AC9 | `controller::service::tests` : 4 new ReloadConfig tests pass (old `unimplemented_with_pr12_message` deleted) | CI |
| AC10 | `tests/integration_reload_e2e.rs` : 5 tests pass | CI |
| AC11 | `efe-server` integration : `health_probe_accepts_reloading_status` passes | CI |
| AC12 | SIGHUP to controller → `reload_gen` increments by 1 | AC10 |
| AC13 | SIGHUP to participant → `reload_gen` increments by 1 ; iteration params swapped | AC10 |
| AC14 | gRPC `ReloadConfig` with empty body → re-reads disk + returns new gen | AC9 |
| AC15 | gRPC `ReloadConfig` with inline meta → applies inline ; gen bumped | AC9 |
| AC16 | Concurrent SIGHUP + Iterate : Iterate sees old manifest snapshot | AC10 |
| AC17 | Reload with mutated participant kappa → rejected with `FAILED_PRECONDITION` ; running state unchanged | AC9 + AC10 |
| AC18 | Reload with mutated meta.dim → rejected ; running state unchanged | AC9 |
| AC19 | Reload with added/removed participant → accepted ; controller reconnects on next Iterate | AC10 |
| AC20 | `Health` returns `RELOADING (3)` during the write-lock window | manual log inspection in AC10 |
| AC21 | `Health.reload_gen` is monotonically increasing across reloads | AC10 |
| AC22 | `efe-server` `health_probe` accepts `RELOADING` as transient | AC11 |
| AC23 | `efe-server` `health_probe` still rejects `NOT_SERVING (2)` | AC11 |
| AC24 | Phase-1 + PR #5-#11 non-regression : 100% green | CI |
| AC25 | `cargo clippy --all-targets` 0 warnings on PR #12 files | CI |
| AC26 | `cargo fmt -- --check` exit 0 | CI |
| AC27 | `cargo doc --no-deps` clean | CI |
| AC28 | PR description references threat IDs `T19, T20, T21` | manual review |

---

## 22. Risks + mitigations

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| **PR12-R1** | `arc-swap` adds a non-trivial dep that affects compile time | L / L | crate is 200 LOC, no transitive bloat ; verified by `cargo tree` |
| **PR12-R2** | `Notify::notify_one` semantics surprise (the "one pending notification" model) | L / M | unit-tested in `concurrent_sighups_are_coalesced` ; documented in the SIGHUP handler |
| **PR12-R3** | Operator triggers SIGHUP during a long Iterate (> 1 s) and expects the new manifest to apply immediately | M / M | document the snapshot semantics in the runbook ; emit a `tracing::info!` showing `reload_gen` on every Iterate start |
| **PR12-R4** | `validate_reload_compatibility` regex-style drift detection misses a subtle invariant (e.g. `kappa` NaN handling) | L / H | unit tests cover NaN explicitly (`old_p.kappa.is_nan() != new_p.kappa.is_nan()`) ; sec review |
| **PR12-R5** | `ReloadConfig` inline path bypasses on-disk audit | M / M | `tracing::warn!` on inline overrides ; recommend operators stick to file edits |
| **PR12-R6** | `efe-server` `health_probe` accepts RELOADING but the controller is stuck in RELOADING (worst case the handler panicked) | L / M | the worst case is a panic in the swap closure ; `panic = "unwind"` profile (PR #6) ; the task respawns on next SIGHUP |
| **PR12-R7** | K8s `kubectl rollout restart` (SIGTERM) and SIGHUP (live reload) confused in operator workflows | M / L | runbook clearly states : SIGHUP = config change, SIGTERM = process replacement |
| **PR12-R8** | Cross-process reload coordination is operator's responsibility (AD1) ; partial reload (only controller, not participants) leaves the system in a degraded but functional state | M / L | the binary that's NOT reloaded keeps serving with old params ; new Iterates fail at the dim/kappa check on the participant if the operator pushed an incompatible manifest |
| **PR12-R9** | `reload_gen` overflow at u32::MAX (≈ 4 G reloads) | LL / L | not a real concern in operator workflows (10 reloads/day for 1 M days) ; if it ever does, `wrapping_add` semantics on `bump_reload_gen` keep things going (monotonicity broken once every 4 G reloads) |
| **PR12-R10** | `ArcSwap::load` returns a `Guard<Arc<T>>` whose lifetime restrictions surprise the reader | L / L | code uses local `let iter = self.state.iteration.load();` then accesses fields ; the guard's lifetime is the local scope ; documented |

---

## 23. Rollback strategy

If PR #12 lands and breaks production :

  1. **Operator-level** : stop sending SIGHUP / ReloadConfig.  The
     production code continues serving the boot-time manifest.  Zero
     impact ; effectively reverts the new feature.
  2. **Per-commit revert** : C6 → C5 → C4 → C3 → C2 → C1.  Each is
     atomic ; reverting C4 alone restores `MetaSwarmService::ReloadConfig`
     to UNIMPLEMENTED.
  3. **Wholesale revert** : `git revert <merge-commit>` ; the crate
     returns to PR #11 state.
  4. **Hotfix** : if only the integration test flakes, mark
     `#[ignore]` ; the production reload path is unaffected since the
     test is observation-only.

Total Time To Mitigate :
  * Operator stops triggering : 0 s.
  * Code revert + redeploy : ~ 10 min.

---

## 24. Sequence diagrams (ASCII)

### 24.1 SIGHUP-driven reload (controller)

```
operator         controller (PID Cm)              file system
   |                |                                  |
   | edit manifest                                     |
   |─────────────────────────────────────────────────>|
   | kill -HUP Cm   |                                  |
   |───────────────>|                                  |
   |                | observability::shutdown          |
   |                |   sighup task wakes              |
   |                |   notify_one()                   |
   |                | controller::reload::apply_reload_from_disk
   |                |   load_and_validate(path) ──────>| read
   |                |                                  |<──
   |                |   validate_reload_compatibility  |
   |                |     (vs current Arc<Manifest>)   |
   |                |   health.set_reloading           |
   |                |   manifest.write().swap          |  (~ 18 µs)
   |                |   health.set_serving             |
   |                |   bump_reload_gen()              |
   |                | tracing::info!("reload OK")     |
   |  (subsequent Iterate sees new manifest)           |
```

### 24.2 gRPC ReloadConfig with inline payload

```
operator (grpcurl)       controller
   |                        |
   | ReloadConfigRequest(   |
   |   new_meta: ...,       |
   |   new_participants: ...|
   | )                      |
   |───────────────────────>|
   |                        | service::reload_config
   |                        |   build_inline_manifest()
   |                        |   apply_reload_with_manifest()
   |                        |     validate_manifest
   |                        |     validate_reload_compatibility
   |                        |     health.set_reloading
   |                        |     swap
   |                        |     health.set_serving + bump_gen
   | ReloadConfigResponse(  |
   |   reload_gen: 5,       |
   |   elapsed_us: 19,      |
   |   participants: 3      |
   | )                      |
   |<───────────────────────|
```

### 24.3 In-flight Iterate during reload

```
client          controller          participants
  | Iterate(req)   |                      |
  |───────────────>| read manifest (Arc-clone @ gen=N)
  |                | spawn_blocking { iterate() }
  |                | step ─────────────────>|
  |                |<──────────────────────|
  | (SIGHUP fires) |                       |
  |     │          | reload : write lock . . . swap → gen=N+1
  |     │          |  (Iterate's Arc still points at gen=N's Manifest)
  |     │          | step ─────────────────>|
  |     │          |<──────────────────────|
  |                | ... continues to convergence ...
  | IterateResponse|                       |
  |<───────────────|                       |
  | Iterate(req')  |                       |
  |───────────────>| read manifest (Arc-clone @ gen=N+1)
  |                | uses new α / γ        |
```

---

## 25. Reviewer checklist

```
## Reviewer checklist — PR #12

### Architecture
- [ ] Atomic swap via Arc<RwLock<Manifest>> (controller) + ArcSwap (participant) — AD2, AD8
- [ ] In-flight Iterate sees stable snapshot — § 5.2 + integration test
- [ ] LocalParticipant is immutable across reload — AD3 + AD9
- [ ] Health flips RELOADING during the swap — § 13
- [ ] reload_gen monotonic via HealthState — AD7

### Correctness
- [ ] validate_reload_compatibility runs OUTSIDE the write lock — § 9.2
- [ ] NaN handling in kappa drift — risk PR12-R4
- [ ] SIGHUP coalescing prevents flood — § 8.2 / risk PR12-R2
- [ ] efe-server probe accepts RELOADING — § 14 / AD13

### Threat model
- [ ] T19, T20, T21 mitigated and tested
- [ ] No new SEV-1 introduced

### Tests
- [ ] ≥ 32 new unit tests
- [ ] 5 integration tests in integration_reload_e2e.rs
- [ ] 2 efe-server probe tests (accept RELOADING / still reject NOT_SERVING)
- [ ] AC1-AC28 all green

### Operability
- [ ] Runbook section ships with the PR — § 26
- [ ] tracing logs show reload_gen + elapsed_us
- [ ] Helm chart values reference (PR #18 follow-up) noted

### Code quality
- [ ] cargo clippy 0 warnings on touched files
- [ ] cargo fmt clean
- [ ] cargo doc no warnings
```

---

## 26. Operator runbook

### 26.1 Trigger a reload from disk (SIGHUP, recommended)

```bash
# 1. Edit the manifest in place (atomic rename to avoid partial reads).
$ editor /etc/efe-meta-swarm/manifest.yaml.new
$ mv /etc/efe-meta-swarm/manifest.yaml.new /etc/efe-meta-swarm/manifest.yaml

# 2. Send SIGHUP to the controller (PID 1 inside the pod).
$ kubectl exec deployment/efe-meta-swarm-controller -- kill -HUP 1

# 3. Send SIGHUP to each participant.
$ for p in supplier-scoring contract-risk spend-categorization ; do
    kubectl exec deployment/efe-meta-swarm-participant-$p -- kill -HUP 1
  done

# 4. Verify the reload landed.
$ grpcurl -plaintext -d '{}' \
    controller.internal:50080 \
    efe.v1.meta_swarm.MetaSwarmService/Health \
| jq '.reloadGen'  # should have incremented by 1
```

### 26.2 Trigger a reload via gRPC (inline override)

```bash
# Apply an inline alpha bump WITHOUT touching the on-disk manifest.
$ grpcurl -plaintext -d '{
    "newMeta": {
      "dim": 16,
      "safetyGeometry": { "box": { "radius": 50.0 } }
    },
    "newParticipants": [
      { "id": "supplier-scoring", "dim": 8, "kappa": 0.6, "seed": 11 },
      { "id": "contract-risk",    "dim": 8, "kappa": 0.7, "seed": 22 },
      { "id": "spend-categorization", "dim": 8, "kappa": 0.5, "seed": 33 }
    ]
  }' \
    controller.internal:50080 \
    efe.v1.meta_swarm.MetaSwarmService/ReloadConfig
```

Response :
```json
{
  "reloadGen": 7,
  "elapsedUs": 19,
  "participants": 3
}
```

### 26.3 Verify reload propagation

```bash
$ grpcurl -plaintext -d '{}' \
    controller.internal:50080 \
    efe.v1.meta_swarm.MetaSwarmService/Info \
| jq '.configuredParticipants | length'  # should match new count
```

### 26.4 If reload was rejected

The controller logs (`kubectl logs deployment/efe-meta-swarm-controller`)
contain a `tracing::error!` with the specific drift cause :

```
ERROR reload rejected reason="participant supplier-scoring kappa drift 0.6 → 0.7"
```

Mitigation : revert the offending change in the manifest and trigger
another SIGHUP.

### 26.5 Difference between SIGHUP and SIGTERM

| Signal | Effect | When to use |
|---|---|---|
| **SIGHUP** | hot reload, no restart | iteration knobs, add/remove participant |
| **SIGTERM** | graceful shutdown, then restart by k8s | binary upgrade, OS patch, full reset |

---

## 27. Open questions

| ID | Question | Status |
|---|---|---|
| **OQ-PR12-1** | Should we expose a `participant`-side gRPC reload RPC for parity with the controller ? | Defer to Phase 3 ; SIGHUP is sufficient for Phase 2 |
| **OQ-PR12-2** | Should `reload_gen` be persisted across restarts ? | No — operators don't rely on cross-restart monotonicity |
| **OQ-PR12-3** | Should we add a `last_reload_at: Timestamp` to `Health` ? | Yes in PR #13 (canary observability) ; out of scope here |
| **OQ-PR12-4** | Should we allow `meta.safety.radius` to change without changing `kind` ? | YES — accepted by § 16.3 ; in-flight Iterates use their snapshot |
| **OQ-PR12-5** | Should the SIGHUP handler clear `step_index_last` on reload ? | NO — the counter survives reloads ; PR #18 may add an explicit reset RPC |
| **OQ-PR12-6** | Should the controller fan out a reload to participants automatically ? | NO (AD1) — per-process reload is simpler and matches operator playbooks |
| **OQ-PR12-7** | Should `validate_reload_compatibility` allow `seed` change at reload time ? | NO — breaks determinism ; AD3 |
| **OQ-PR12-8** | Should we add `ReloadConfigRequest.dry_run: bool` ? | YES in PR #20 ; defer here |

---

## 28. Appendix : full file inventory

### 28.1 Files added (4)

```
crates/efe-meta-swarm-server/src/controller/reload.rs              NEW  ~ 220 LOC
crates/efe-meta-swarm-server/src/participant/reload.rs             NEW  ~ 220 LOC
crates/efe-meta-swarm-server/tests/integration_reload_e2e.rs       NEW  ~ 280 LOC
docs/EFE_A1_PHASE2_PR12_ARCHITECTURE.md                            NEW  this file
```

### 28.2 Files modified (11)

```
crates/efe-meta-swarm-server/Cargo.toml                            MODIFIED   +1 LOC : arc-swap dev/runtime dep
crates/efe-meta-swarm-server/src/controller/mod.rs                 MODIFIED   +1 LOC : pub mod reload
crates/efe-meta-swarm-server/src/controller/service.rs             MODIFIED   +50 / −15 LOC : reload_config wire
crates/efe-meta-swarm-server/src/manifest/mod.rs                   MODIFIED   +85 / −0 LOC : validate_reload_compatibility
crates/efe-meta-swarm-server/src/observability/shutdown.rs         MODIFIED   +60 / −0 LOC : install_sighup_handler
crates/efe-meta-swarm-server/src/observability/metrics.rs          MODIFIED   +50 LOC : reload metrics
crates/efe-meta-swarm-server/src/participant/mod.rs                MODIFIED   +1 LOC : pub mod reload
crates/efe-meta-swarm-server/src/participant/state.rs              MODIFIED   +30 / −5 LOC : iteration: ArcSwap + iteration_swap()
crates/efe-meta-swarm-server/src/bin/controller.rs                 MODIFIED   +20 LOC : install_sighup_handler call
crates/efe-meta-swarm-server/src/bin/participant.rs                MODIFIED   +20 LOC : install_sighup_handler call
crates/efe-meta-swarm-server/tests/common/multiprocess.rs          MODIFIED   +30 LOC : sighup_controller + sighup_participant

crates/efe-server/src/services/meta_swarm/mod.rs                   MODIFIED   +10 LOC : health_probe accepts status=3
crates/efe-server/tests/integration_meta_swarm_plumbing.rs         MODIFIED   +60 LOC : 2 RELOADING-acceptance tests
```

### 28.3 LOC tally

| Category | LOC |
|---|---|
| New production code | ~ 440 |
| New tests | ~ 280 |
| Modified production code | ~ 290 |
| Modified tests | ~ 60 |
| **Net** | **~ +1 050 LOC** |

---

## 29. Glossary

| Term | Definition |
|---|---|
| **AC** | Acceptance Criterion |
| **AD** | Architectural Decision |
| **L2 gate G9** | Phase-2 graduation gate "live reconfiguration without process restart" |
| **SIGHUP** | Unix signal traditionally used for "reload your config" ; PR #12 makes both binaries honour it |
| **ReloadConfig RPC** | `MetaSwarmService::ReloadConfig` ; the gRPC trigger path (controller only) |
| **`reload_gen`** | monotonic counter (`HealthState.reload_gen`, `AtomicU32`) ; bumped on every successful reload |
| **RELOADING** | `HealthResponse.status = 3` ; transient state during the write-lock swap window |
| **`apply_reload_*`** | the canonical reload entry points in `controller::reload` and `participant::reload` |
| **`validate_reload_compatibility`** | (this PR) checks that the new manifest can replace the current one without invalidating in-flight state |
| **Own-row drift** | a participant's own kappa/dim/seed changed across reload — forbidden (AD3 / AD9) |
| **Inline override** | `ReloadConfigRequest` with non-empty body ; applies the payload without writing to disk |
| **Coalescing** | merging multiple concurrent SIGHUPs into at most one in-flight reload + one queued reload |
| **`ArcSwap`** | thin atomic-Arc-replacement primitive from the `arc-swap` crate ; used for `IterationParams` |
| **Snapshot semantics** | in-flight calls hold an `Arc<Manifest>` clone captured at call start ; the swap is invisible to them |

---

*End of architecture document. Total: ~ 2 100 lines.*

*Next deliverable : PR #12 code (6 commits).*
