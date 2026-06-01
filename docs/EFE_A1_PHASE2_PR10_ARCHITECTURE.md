# EFE-A1 Phase 2 — PR #10 : `integration_grpc_multiprocess`

| Field | Value |
|---|---|
| **PR** | #10 (Sprint A1 Phase 2 / P2.3) |
| **Branch** | `efe-a1/p2-pr10-multiprocess-integration` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× meta-swarm + 1× sre + 1× sec |
| **Depends on** | PR #5 (proto), PR #6 (scaffold), PR #7 (threat model), PR #8 (controller wire), **PR #9 (participant wire)** |
| **Blocks** | PR #11 (Discovery refresh), PR #12 (SIGHUP reload), PR #16 (mTLS), PR #18 (reflection + OTel) |
| **L2 gate** | G4 (process-boundary correctness), G6 (cross-process determinism), G7 (graceful drain) |
| **Threat IDs in scope** | T7, T12, T13, T17, T18 (cf. `docs/security/PHASE2_THREAT_MODEL.md`) |
| **CVSS bandwidth** | All ≤ SEV-3 ; no new SEV-1 introduced |

---

## Table of contents

  0. TL;DR
  1. Goal + non-goals
  2. Pre-flight checks
  3. Branch + commit plan
  4. Architectural decisions
  5. Topology + process tree diagram
  6. Crate layout additions
  7. `tests/common/multiprocess.rs` — process harness (full design)
  8. `tests/integration_multiprocess_e2e.rs` — happy paths
  9. `tests/integration_multiprocess_failure.rs` — failure modes
 10. `tests/integration_multiprocess_lifecycle.rs` — shutdown drain
 11. `scripts/run_multiprocess_smoke.sh` — operator script
 12. `Cargo.toml` updates
 13. Determinism across the wire
 14. CI gating + perf budget
 15. Error mapping over process boundary
 16. Threat-model coverage
 17. Concurrency model + lifecycle invariants
 18. Tests strategy summary
 19. Acceptance criteria (AC1–AC25)
 20. Risks + mitigations
 21. Rollback strategy
 22. Sequence diagrams (ASCII)
 23. Reviewer checklist
 24. Migration notes (operator-facing)
 25. Open questions
 26. Appendix : full file inventory
 27. Glossary

---

## 0. TL;DR

PR #10 adds the first **process-boundary** integration tests for the Phase-2
control plane. Until now :

  * PR #8 wired the controller and tested it in-process against a fake
    `TestParticipantSvc`.
  * PR #9 wired the participant and tested it in-process against the real
    `ParticipantService`, plus a single-binary smoke
    (`integration_participant_binary_smoke.rs`).
  * PR #9's `integration_e2e_iterate.rs` runs controller + 3 real
    participants but **all in one process** (single tokio runtime, single
    address space, no syscall-level boundary).

PR #10 closes the gap by orchestrating **separate OS processes** for
the controller and each participant, then driving real gRPC traffic between
them over `127.0.0.1`. This is the T2 single-node topology in production-like
form.

Deliverables :

  * **A new harness** (`tests/common/multiprocess.rs`, ~ 350 LOC) :
    `ProcessHarness` orchestrates port allocation, manifest fixture
    generation, subprocess spawn, readiness polling, and graceful shutdown.
    RAII-clean : a panicking test still SIGKILLs all children on Drop.
  * **9 integration tests** across three files :
    * `integration_multiprocess_e2e.rs` — 4 happy-path tests (convergence,
      determinism over the wire, capabilities discovery, info snapshot).
    * `integration_multiprocess_failure.rs` — 3 failure-mode tests
      (participant unreachable, participant killed mid-Iterate, manifest
      mismatch).
    * `integration_multiprocess_lifecycle.rs` — 2 lifecycle tests
      (controller SIGTERM drains in-flight Iterate ; participant SIGTERM
      makes the controller fail gracefully).
  * **`scripts/run_multiprocess_smoke.sh`** — a 70-line shell helper for
    operators to reproduce the Kovant scenario manually, with optional
    `grpcurl` Iterate trigger.

After PR #10 :

  * `cargo test -p efe-meta-swarm-server --all-targets` exercises the
    complete control plane across real process boundaries.
  * Phase-2 L2 graduation gate G4 (process-boundary correctness) and G7
    (graceful drain) flip GREEN.
  * The deployment manifests in PR #18 + PR #20 have a baseline that's
    been verified end-to-end at the syscall level.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Verify the Phase-2 control plane (1 controller + 3 participants on
> `localhost`) crossing real OS process boundaries : binaries spawned
> from cargo's built artefacts, communicating via TCP/HTTP-2/gRPC, with
> failure modes injected at the syscall level (`kill -SIGTERM`,
> port-collision, partial crash), proving convergence, determinism, and
> graceful drain match the in-process baseline.

### 1.2 Non-goals

The following are **explicitly out of scope** for PR #10 :

| Item | Why deferred | Target PR |
|---|---|---|
| Multi-host deployment (cross-pod, cross-node, cross-zone) | needs K8s manifest design | PR #20 |
| mTLS / JWT auth on the wire | requires cert provisioning pipeline | PR #16 |
| Streaming (`StepStream` / `IterateStream`) over the wire | backpressure contract not finalised | PR #21 |
| Cosign attestation of `Capabilities.build_sha` | needs keychain setup | PR #16 |
| OpenTelemetry exporter | needs collector endpoint configurable | PR #18 |
| gRPC reflection | needs the reflection service wired | PR #18 |
| Persisted state across restarts | requires sidecar + sled or rocksdb | PR #14 |
| SIGHUP reload | needs swap protocol | PR #12 |
| Process-level resource limits (cgroups, prlimit) | platform-specific, deployment concern | PR #20 |
| Crash-loop detection / restart policies | systemd / k8s concern | PR #20 |
| Network partition injection (iptables, tc) | requires root + namespace setup | Phase 3 |
| TLS handshake failure modes | Phase 3 |
| Multi-controller fan-out | Phase 3 |
| Process isolation (namespaces, seccomp) | Phase 3 |

### 1.3 Single binary unit of merge-readiness

After PR #10 merges, the **deployment confidence baseline** is :

  > A developer running `cargo test -p efe-meta-swarm-server --all-targets`
  > on a fresh checkout, with no manual setup, gets a **bit-identical
  > convergence** across the Kovant 3-participant topology, spawned as
  > real OS processes, in under 15 seconds wall time.

That confidence is the green light for PR #18's deployment manifests.

---

## 2. Pre-flight checks

Before opening the PR :

| # | Check | Verifier | Pass criterion |
|---|---|---|---|
| PF1 | `cargo build -p efe-meta-swarm-server --release` succeeds on `main` | CI | green |
| PF2 | `cargo test -p efe-meta-swarm-server --offline --all-targets` is **352 tests** on `main` (PR #9 baseline) | CI | green |
| PF3 | PR #9's `integration_e2e_iterate.rs` passes (in-process e2e) | CI | green |
| PF4 | PR #9's `integration_participant_binary_smoke.rs` passes (single-binary subprocess) | CI | green |
| PF5 | `integration_health_smoke.rs` passes (controller-binary subprocess, PR #6) | CI | green |
| PF6 | `target/debug/controller` and `target/debug/participant` produced by `cargo build --tests` | manual | both binaries present |
| PF7 | Localhost ports 53280–53400 are not occupied on the CI runner | CI | `ss -ltn` shows free |
| PF8 | `libc::kill` available on the target platform (Unix-only ; cf. AD11) | manual | `cfg(unix)` is true |
| PF9 | `tempfile` crate is in dev-dependencies | grep | `Cargo.toml [dev-dependencies]` |
| PF10 | The threat model (PR #7) is in v1.0 frozen state | manual | sec sign-off |

Any failed pre-flight blocks PR #10.

---

## 3. Branch + commit plan

### 3.1 Branch

```
efe-a1/p2-pr10-multiprocess-integration
```

Branched from `main` after PR #9 merge SHA. **Never** rebase onto a moving
`main`.

### 3.2 Commit-by-commit plan (5 commits, each CI-green)

| # | Subject | Files | LOC ± | Verification |
|---|---|---|---|---|
| C1 | `feat(test-harness): ProcessHarness for multi-process integration` | `+ tests/common/multiprocess.rs` (~ 350 LOC), `~ tests/common/mod.rs` (re-export) | +360 / −0 | unit tests in module (≥ 4) ; harness compiles ; happy-path smoke spawn + Health round-trip |
| C2 | `feat(integration): multi-process e2e happy paths` | `+ tests/integration_multiprocess_e2e.rs` (~ 250 LOC) | +250 | 4 tests pass : convergence, determinism over wire, capabilities, info |
| C3 | `feat(integration): multi-process failure modes` | `+ tests/integration_multiprocess_failure.rs` (~ 250 LOC) | +250 | 3 tests pass : unreachable, mid-Iterate kill, manifest mismatch |
| C4 | `feat(integration): multi-process lifecycle + SIGTERM drain` | `+ tests/integration_multiprocess_lifecycle.rs` (~ 200 LOC) | +200 | 2 tests pass : controller SIGTERM drain, participant SIGTERM |
| C5 | `feat(scripts): run_multiprocess_smoke.sh + Cargo.toml dev-dep libc` | `+ scripts/run_multiprocess_smoke.sh` (~ 80 LOC), `~ crates/efe-meta-swarm-server/Cargo.toml` (+ libc dev-dep) | +80 / −0 | `bash scripts/run_multiprocess_smoke.sh` produces a Converged result via `grpcurl` |

Total : **+1 140 / −0 = +1 140 LOC net** including ~ 940 LOC of tests + 80
LOC of shell.

### 3.3 Drafting strategy

Each commit is **green-on-merge**.  C1 introduces the harness behind no
existing user (the harness module is unused after C1 lands but compiles
cleanly).  C2-C4 each add a separate test file, each runnable in isolation
via `cargo test --test integration_multiprocess_*`.  C5 is a pure tooling
addition (no Rust code change beyond the Cargo.toml dev-dep line).

If a commit causes flakiness, revert that single commit ; the others stay
green.

---

## 4. Architectural decisions

| AD | Decision | Rationale | Alternatives considered | Reversibility |
|---|---|---|---|---|
| **AD1** | Tests spawn binaries via `std::process::Command` using `env!("CARGO_BIN_EXE_<name>")` | cargo provides the binary paths automatically ; no PATH manipulation | spawn via shell ; embed binary paths in env vars at compile time | Reversible — only test code |
| **AD2** | Port allocation by **bind-drop-rebind** on `127.0.0.1:0` (same pattern as PR #8/9) | sufficient on localhost for serial allocations ; no external port reservation service needed | hardcoded port ranges ; `ephemeral-port-reservations` crate ; OS-level port hold via `SO_REUSEADDR` | Reversible |
| **AD3** | Manifest is materialised to a **per-test `tempfile::NamedTempFile`** | each test has an isolated manifest ; clean tear-down on Drop | `/tmp/efe-test-manifest.yaml` (race-prone) ; in-memory manifest (binary can't read) | Reversible |
| **AD4** | The harness exposes a **`ProcessHarness` RAII type** ; tests hold it for the duration ; Drop SIGKILLs all children | guaranteed clean-up on panic, on early return, on test timeout | manual cleanup with `defer!` ; per-test guard struct | Reversible |
| **AD5** | Graceful shutdown via **`libc::kill(pid, SIGTERM)`** (Unix-only, `cfg(unix)`) | `child.kill()` sends SIGKILL ; we need SIGTERM for drain tests | `nix` crate (transitive bloat) ; spawn `kill` subprocess (gross) | Reversible — `cfg(unix)` gate keeps Windows builds working (the tests simply don't compile there) |
| **AD6** | Tests are **not** `#[ignore]`-gated | the value of testing real process boundaries outweighs the ~ 10 s extra wall time | `#[ignore]` + explicit `cargo test -- --ignored` ; feature flag `multiprocess-tests` | Reversible — easy to flip if CI runners struggle |
| **AD7** | Tests bind ports in a **dedicated range** (53280–53400) distinct from PR #6/9 ranges | predictable namespace ; easier to grep in `ss -ltn` ; avoids accidental collisions across test binaries | random ports (already done via AD2) ; per-test-file ranges | Hybrid : we use bind-drop for actual ports, but TEST documentation references this range for traceability |
| **AD8** | All test logs (stdout/stderr of children) **go to `/dev/null` by default** ; on failure, tests rely on `cargo test --nocapture` for raw output | clean test output ; deterministic failure messages | per-test buffered capture (would require background reader tasks) ; tee to file | Reversible — a future PR can wire a `capture_logs: bool` flag in the harness |
| **AD9** | Tests use the **default Kovant topology** (3 participants : `supplier-scoring` dim=8 κ=0.6 seed=11, `contract-risk` dim=8 κ=0.7 seed=22, `spend-categorization` dim=8 κ=0.5 seed=33) **unless** a test specifically requires variants | one mental model for all tests ; matches PR #8/9 conventions | Per-test custom topology | The harness provides both `spawn_kovant()` (default) and `spawn_with_rows(...)` (custom) |
| **AD10** | The harness drives **5-second readiness polling** with a 250 ms back-off ; 10-second hard cap | binary boot is ~ 50–150 ms on a 2024 server ; 5 s × 4 = 20 attempts ; well over what's needed | longer cap (slower failures) ; explicit "ready" message in stdout (couples test to log format) | Reversible — bump timeouts in one place |
| **AD11** | **Unix-only** tests (`cfg(unix)`) | Phase 2 deployment target is Linux ; macOS local dev OK ; Windows out of scope until Phase 5 | Cross-platform shim layer | Reversible — Windows support can be added with a `cfg(windows)` branch sending Ctrl+Break |
| **AD12** | The lifecycle tests use **`libc::kill` + `Child::wait_with_timeout`** wrapper (we synthesise the timeout via `tokio::time::timeout` around the blocking `wait`) | needed for graceful drain assertions ; std lib has no built-in wait-with-timeout | `nix::sys::wait::waitpid` ; busy-loop with `try_wait` | Reversible |

---

## 5. Topology + process tree diagram

### 5.1 Process tree at peak (a single happy-path test)

```
test runner (cargo test)
└── integration_multiprocess_e2e.iterate_e2e_kovant_converges (tokio task)
    ├── ProcessHarness
    │   ├── controller     (PID Cm, binds 127.0.0.1:Pc1 gRPC, 127.0.0.1:Pc2 metrics)
    │   ├── participant-1  (PID P1, binds 127.0.0.1:Pp1a gRPC, 127.0.0.1:Pp1b metrics)
    │   ├── participant-2  (PID P2, binds 127.0.0.1:Pp2a gRPC, 127.0.0.1:Pp2b metrics)
    │   ├── participant-3  (PID P3, binds 127.0.0.1:Pp3a gRPC, 127.0.0.1:Pp3b metrics)
    │   └── manifest.yaml  (tempfile : /tmp/.tmpXXXXXX.yaml)
    └── MetaSwarmServiceClient
        └── http2 → controller's gRPC port
            └── controller orchestrates Step calls to each participant
                via three independent gRPC channels
```

Total : **5 OS processes** per test (test runner + 1 controller + 3
participants).  Total ports allocated : **8** (4 gRPC + 4 metrics).

### 5.2 Process boundary vs in-process e2e (PR #9)

| | PR #9 in-process | PR #10 multi-process |
|---|---|---|
| Number of OS processes | 1 (just the test runner) | 5 (test runner + controller + 3 participants) |
| Number of tokio runtimes | 1 (shared by all services) | 5 (one per binary, one in the test runner) |
| gRPC transport | localhost TCP / loopback | localhost TCP / loopback (identical wire format) |
| Cancellation propagation | tokio cancel token + Arc-shared | TCP RST + tonic disconnect + cancel token (per-process) |
| SIGTERM behaviour | N/A (single process) | tested |
| Manifest dispatch | shared in-memory struct | shared tempfile YAML |
| Determinism | bit-identical | bit-identical (validated) |
| Boot time per test | ~ 5 ms | ~ 150 ms |
| CI memory footprint per test | ~ 50 MB | ~ 150 MB (incl. 4 binaries) |

### 5.3 The Kovant scenario, multi-process edition

```
                  ┌─────────────────────────────────┐
                  │ test client (gRPC over TCP)     │
                  └────────────┬────────────────────┘
                               │ Iterate(req)
                               ▼
                  ┌─────────────────────────────────┐
                  │ controller (PID Cm)             │
                  │  • MetaSwarmService             │
                  │  • RemoteParticipant × 3        │
                  └──┬──────────────┬──────────────┬┘
                     │              │              │  Step(pulled)
                     ▼              ▼              ▼
   ┌──────────────────────┐ ┌─────────────┐ ┌────────────────┐
   │ supplier-scoring (P1)│ │contract-r..│ │spend-categ. (P3)│
   │  • MetaParticipant.. │ │     (P2)   │ │                 │
   └──────────────────────┘ └─────────────┘ └────────────────┘
```

After PR #9, each box was a `tokio::spawn`-ed task in one process ; after
PR #10, each box is a separate OS process with its own page table.

---

## 6. Crate layout additions

### 6.1 Before PR #10

```
crates/efe-meta-swarm-server/
├── src/                                 (PR #9 surface)
├── tests/
│   ├── common/
│   │   └── mod.rs                       (in-process helpers from PR #9)
│   ├── integration_e2e_iterate.rs       (PR #9)
│   ├── integration_health_smoke.rs      (PR #6)
│   ├── integration_iterate_inproc.rs    (PR #8 / 9)
│   ├── integration_participant_binary_smoke.rs   (PR #9)
│   └── integration_step_smoke.rs        (PR #9)
└── Cargo.toml                            (no libc)
```

### 6.2 After PR #10

```
crates/efe-meta-swarm-server/
├── src/                                 (unchanged from PR #9)
├── tests/
│   ├── common/
│   │   ├── mod.rs                       [MODIFIED : re-export multiprocess]
│   │   └── multiprocess.rs              [NEW : ~ 350 LOC, harness]
│   ├── integration_e2e_iterate.rs       (PR #9, unchanged)
│   ├── integration_health_smoke.rs      (PR #6, unchanged)
│   ├── integration_iterate_inproc.rs    (PR #9, unchanged)
│   ├── integration_multiprocess_e2e.rs        [NEW : ~ 250 LOC, 4 tests]
│   ├── integration_multiprocess_failure.rs    [NEW : ~ 250 LOC, 3 tests]
│   ├── integration_multiprocess_lifecycle.rs  [NEW : ~ 200 LOC, 2 tests]
│   ├── integration_participant_binary_smoke.rs   (PR #9, unchanged)
│   └── integration_step_smoke.rs        (PR #9, unchanged)
├── Cargo.toml                            [MODIFIED : + libc dev-dep]

scripts/
└── run_multiprocess_smoke.sh            [NEW : ~ 80 LOC, operator helper]
```

---

## 7. `tests/common/multiprocess.rs` — process harness (full design)

### 7.1 Purpose

A single module owning **everything** about subprocess orchestration so
tests stay focused on the scenario, not on plumbing. The module is
test-only (`#![allow(dead_code)]` for unused-by-some-test-binary
helpers).

### 7.2 Public surface

```rust
//! Multi-process integration harness.
//!
//! Owns the orchestration of controller + participant binaries spawned
//! as OS subprocesses.  Each test holds a [`ProcessHarness`] for the
//! duration of the test ; Drop SIGKILLs every child + cleans the
//! tempfile manifest.
//!
//! Unix-only — Phase 2 deployment target is Linux server.

#![cfg(unix)]
#![allow(dead_code)]

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use efe_meta_swarm_proto::controller::MetaSwarmServiceClient;
use efe_meta_swarm_proto::meta_swarm::HealthRequest;
use efe_meta_swarm_proto::participant::MetaParticipantServiceClient;
use efe_meta_swarm_server::manifest::ParticipantManifest;
use tempfile::NamedTempFile;
use tonic::Request;

/// One participant subprocess.
pub struct ParticipantChild {
    pub id: String,
    pub grpc_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
    child: Child,
}

/// The controller subprocess.
pub struct ControllerChild {
    pub grpc_addr: SocketAddr,
    pub metrics_addr: SocketAddr,
    child: Child,
}

/// Full process-tree handle.  Drop SIGKILLs everything.
pub struct ProcessHarness {
    pub controller: ControllerChild,
    pub participants: Vec<ParticipantChild>,
    _manifest: NamedTempFile,
}

impl ProcessHarness {
    /// Spawn the default Kovant topology : 3 participants on dim=8 with
    /// (κ, seed) = (0.6, 11), (0.7, 22), (0.5, 33).
    pub async fn spawn_kovant() -> Self {
        Self::spawn_with_rows(vec![
            ParticipantManifest { id: "supplier-scoring".into(),     dim: 8, kappa: 0.6, seed: 11, blas_backend: "none".into() },
            ParticipantManifest { id: "contract-risk".into(),        dim: 8, kappa: 0.7, seed: 22, blas_backend: "none".into() },
            ParticipantManifest { id: "spend-categorization".into(), dim: 8, kappa: 0.5, seed: 33, blas_backend: "none".into() },
        ])
        .await
    }

    /// Spawn with custom participant rows.  Each row gets its own
    /// participant subprocess on a fresh port.
    pub async fn spawn_with_rows(rows: Vec<ParticipantManifest>) -> Self { /* … */ }

    /// Spawn with custom participant rows AND a custom controller config
    /// override (e.g. `max_inflight = 1` for the inflight test).
    pub async fn spawn_with_rows_and_inflight(
        rows: Vec<ParticipantManifest>,
        max_inflight: usize,
    ) -> Self { /* … */ }

    /// Spawn with custom rows BUT do NOT spawn one participant id — used to
    /// test the "participant unreachable" scenario.  The manifest still
    /// declares the absent participant ; the controller's
    /// `EFE_META_SWARM_PARTICIPANTS` env var includes a bogus address.
    pub async fn spawn_kovant_with_missing(absent_id: &str) -> Self { /* … */ }

    /// gRPC endpoint string `http://127.0.0.1:PORT` for the controller.
    pub fn controller_endpoint(&self) -> String {
        format!("http://{}", self.controller.grpc_addr)
    }

    /// gRPC endpoint for a participant by id.
    pub fn participant_endpoint(&self, id: &str) -> Option<String> {
        self.participants
            .iter()
            .find(|p| p.id == id)
            .map(|p| format!("http://{}", p.grpc_addr))
    }

    /// Poll Health on every binary until all return SERVING, or fail
    /// after `timeout`.
    pub async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String> { /* … */ }

    /// Send SIGTERM to a participant by id.  Returns whether the
    /// participant was found (it might already have been killed by a
    /// prior call).  Use [`wait_participant`] to reap it.
    pub fn sigterm_participant(&self, id: &str) -> bool { /* … */ }

    /// Send SIGTERM to the controller.  Use [`wait_controller`] to reap.
    pub fn sigterm_controller(&self) { /* … */ }

    /// SIGKILL a participant immediately (no drain).  Used by
    /// failure-mode tests to simulate a hard crash.
    pub fn sigkill_participant(&mut self, id: &str) -> bool { /* … */ }

    /// Wait for the controller to exit, capped at `timeout`.
    pub async fn wait_controller(
        &mut self,
        timeout: Duration,
    ) -> Result<std::process::ExitStatus, String> { /* … */ }

    /// Wait for a participant to exit, capped at `timeout`.
    pub async fn wait_participant(
        &mut self,
        id: &str,
        timeout: Duration,
    ) -> Result<std::process::ExitStatus, String> { /* … */ }
}

impl Drop for ProcessHarness {
    fn drop(&mut self) {
        // Last-resort SIGKILL ; tests that wanted a clean shutdown
        // should have used `sigterm_*` + `wait_*` already.
        for p in &mut self.participants {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
        let _ = self.controller.child.kill();
        let _ = self.controller.child.wait();
        // `_manifest` drops, cleaning the tempfile.
    }
}
```

### 7.3 Spawn flow

The harness goes through these steps in `spawn_with_rows` :

```
1. Allocate gRPC + metrics ports for each participant   (2 × N ports)
2. Allocate gRPC + metrics ports for the controller     (2 ports)
3. Render the manifest YAML from `rows`
4. Write it to a NamedTempFile
5. Spawn each participant subprocess :
     EFE_META_SWARM_PARTICIPANT_ID         = <row.id>
     EFE_META_SWARM_PARTICIPANT_ADDR       = 127.0.0.1:<grpc_port>
     EFE_META_SWARM_PARTICIPANT_METRICS    = 127.0.0.1:<metrics_port>
     EFE_META_SWARM_MANIFEST               = <tempfile path>
     EFE_META_SWARM_LOG                    = warn
6. Spawn the controller subprocess :
     EFE_META_SWARM_CONTROLLER_ADDR        = 127.0.0.1:<grpc_port>
     EFE_META_SWARM_CONTROLLER_METRICS     = 127.0.0.1:<metrics_port>
     EFE_META_SWARM_MANIFEST               = <tempfile path>
     EFE_META_SWARM_PARTICIPANTS           = id1=h1:p1,id2=h2:p2,...
     EFE_META_SWARM_LOG                    = warn
7. Wait for all to be ready (Health polling)
8. Return the ProcessHarness handle
```

Steps 1–4 are sequential (writing depends on ports).  Steps 5–6 are
parallel via `tokio::join!`.  Step 7 polls Health concurrently
(`tokio::join!` again).

### 7.4 Readiness probing

```rust
async fn ping_controller(addr: SocketAddr) -> bool {
    let endpoint = format!("http://{addr}");
    let Ok(ep) = tonic::transport::Endpoint::from_shared(endpoint) else {
        return false;
    };
    let Ok(ch) = ep
        .timeout(Duration::from_millis(200))
        .connect_timeout(Duration::from_millis(200))
        .connect()
        .await
    else {
        return false;
    };
    let mut client = MetaSwarmServiceClient::new(ch);
    client.health(Request::new(HealthRequest {}))
        .await
        .map(|r| r.into_inner().status == 1)
        .unwrap_or(false)
}

async fn ping_participant(addr: SocketAddr) -> bool { /* same, but with MetaParticipantServiceClient */ }
```

`wait_for_ready` polls all endpoints concurrently (via `tokio::join_all`)
every 250 ms until all return SERVING or the cap fires.

### 7.5 Graceful shutdown

```rust
pub fn sigterm_participant(&self, id: &str) -> bool {
    if let Some(p) = self.participants.iter().find(|p| p.id == id) {
        // SAFETY : libc::kill is safe to call with any pid ; the
        // failure mode is just an `errno`-set return value.
        // PID can wrap-around on Linux ; the worst case is sending
        // SIGTERM to an unrelated process that happens to have
        // recycled the same PID.  The test runner's process won't
        // see this in practice : we hold the Child until Drop.
        unsafe {
            libc::kill(p.child.id() as libc::pid_t, libc::SIGTERM);
        }
        true
    } else {
        false
    }
}
```

This is `#[cfg(unix)]` — Windows would need `GenerateConsoleCtrlEvent` or
similar.  Out of scope.

### 7.6 Cleanup (Drop)

The Drop impl is the **last-resort** SIGKILL.  Tests that need a graceful
drain MUST call `sigterm_*` + `wait_*` before dropping the harness.  If a
test panics mid-execution, Drop kicks in and tears everything down.

```rust
impl Drop for ProcessHarness {
    fn drop(&mut self) {
        for p in &mut self.participants {
            // `kill()` sends SIGKILL on Unix ; `wait()` reaps the
            // child so we don't leave zombies.
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
        let _ = self.controller.child.kill();
        let _ = self.controller.child.wait();
    }
}
```

### 7.7 Port allocation

Inherits the bind-drop-rebind pattern from PR #8's `tests/common/mod.rs`
`allocate_port`.  TOCTOU window is microseconds on localhost ; empirically
no flakes across PR #8/9 test runs.  PR #10 reuses the same helper.

```rust
pub async fn allocate_port() -> SocketAddr {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    addr
}
```

If the harness times out 50× during a single CI run (failure-mode :
host running out of ephemeral ports), the harness logs a clear error
naming the exhausted resource.

### 7.8 Manifest fixture generation

```rust
fn render_manifest_yaml(rows: &[ParticipantManifest]) -> String {
    let mut buf = String::new();
    buf.push_str("version: v0.2.0\n");
    buf.push_str("meta:\n");
    buf.push_str("  dim: 16\n");
    buf.push_str("  safety:\n");
    buf.push_str("    kind: box\n");
    buf.push_str("    radius: 50.0\n");
    buf.push_str("participants:\n");
    for r in rows {
        buf.push_str(&format!(
            "  - id: {}\n    dim: {}\n    kappa: {}\n    seed: {}\n    blas_backend: \"none\"\n",
            r.id, r.dim, r.kappa, r.seed,
        ));
    }
    buf.push_str("iteration:\n");
    buf.push_str("  max_iters: 100\n");
    buf.push_str("  tolerance: 0.0001\n");
    buf.push_str("  alpha_default: 0.5\n");
    buf.push_str("  coupling_gamma: 0.2\n");
    buf.push_str("  deterministic: true\n");
    buf.push_str("  max_wall_ms: 15000\n");
    buf
}
```

No `serde_yaml` round-trip — we generate the YAML directly to keep
test-time fast.  The format matches `manifest::load_and_validate`'s
expectations 1-to-1 ; if the schema drifts, the test smoke at C1 catches
it.

### 7.9 Unit tests (in module)

C1 ships these in-module tests :

  1. `harness_spawns_kovant_and_health_passes` — Kovant topology comes up,
     all 4 binaries report SERVING within 5 s.
  2. `harness_drop_kills_all_children` — wrap a harness in a function,
     return ; child PIDs are no longer in `/proc/<pid>`.
  3. `harness_sigterm_controller_then_wait` — SIGTERM the controller, wait
     for exit, status code is 0.
  4. `harness_participant_endpoint_returns_some_for_known_id` — endpoint
     lookup works ; bogus id returns None.

These tests don't go through any production code logic — they exercise
the harness itself.

---

## 8. `tests/integration_multiprocess_e2e.rs` — happy paths

### 8.1 Test list (4 tests)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_kovant_converges_across_real_processes() { /* … */ }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_e2e_bit_identical_3_reruns_over_wire() { /* … */ }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_info_lists_all_participants() { /* … */ }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn participant_capabilities_reflects_manifest_across_processes() { /* … */ }
```

### 8.2 `iterate_e2e_kovant_converges_across_real_processes`

```rust
async fn iterate_e2e_kovant_converges_across_real_processes() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness.wait_for_ready(Duration::from_secs(5)).await.expect("ready");

    let channel = Endpoint::from_shared(harness.controller_endpoint())
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = MetaSwarmServiceClient::new(channel);

    let resp = client
        .iterate(Request::new(iterate_request(8, 100, 1.0e-3)))
        .await
        .expect("iterate")
        .into_inner();

    assert_eq!(resp.stop_reason, ProtoStopReason::Converged as i32);
    assert!(resp.iterations <= 80);
    assert!(resp.final_residual <= 1.0e-3);
    assert_eq!(resp.final_states.len(), 3);
    // Harness Drop teardown handles cleanup.
}
```

### 8.3 `iterate_e2e_bit_identical_3_reruns_over_wire`

```rust
async fn iterate_e2e_bit_identical_3_reruns_over_wire() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness.wait_for_ready(Duration::from_secs(5)).await.expect("ready");

    let mut client = MetaSwarmServiceClient::new(/* … */);

    let mut sigs = Vec::new();
    for _ in 0..3 {
        client.reset(Request::new(ResetRequest::default())).await.unwrap();
        let r = client.iterate(Request::new(iterate_request(8, 100, 1.0e-3))).await.unwrap().into_inner();
        sigs.push((r.iterations, r.final_residual.to_bits()));
    }
    assert!(
        sigs.windows(2).all(|w| w[0] == w[1]),
        "non-deterministic reruns over real process boundary : {sigs:?}",
    );
}
```

This is the **gold standard** : if convergence is bit-identical across 3
reruns over real TCP, the gRPC wire format and the participant binaries
are deterministic.  Mirrors PR #9's in-process determinism test but at the
process level.

### 8.4 `controller_info_lists_all_participants`

```rust
async fn controller_info_lists_all_participants() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness.wait_for_ready(Duration::from_secs(5)).await.unwrap();

    let mut client = MetaSwarmServiceClient::new(/* … */);
    let info = client.info(Request::new(InfoRequest {})).await.unwrap().into_inner();
    assert_eq!(info.proto_version, "v1");
    assert_eq!(info.configured_participants.len(), 3);
    let ids : Vec<&str> = info.configured_participants.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"supplier-scoring"));
    assert!(ids.contains(&"contract-risk"));
    assert!(ids.contains(&"spend-categorization"));
}
```

Validates that the controller's `info_snapshot` (PR #8 added the
manifest-driven populate) propagates through real gRPC.

### 8.5 `participant_capabilities_reflects_manifest_across_processes`

```rust
async fn participant_capabilities_reflects_manifest_across_processes() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness.wait_for_ready(Duration::from_secs(5)).await.unwrap();

    // Talk directly to participant #1.
    let endpoint = harness.participant_endpoint("supplier-scoring").unwrap();
    let channel = Endpoint::from_shared(endpoint).unwrap().connect().await.unwrap();
    let mut client = MetaParticipantServiceClient::new(channel);

    let cap = client.capabilities(Request::new(CapabilitiesRequest {})).await.unwrap().into_inner();
    let desc = cap.descriptor.expect("descriptor present");
    assert_eq!(desc.id, "supplier-scoring");
    assert_eq!(desc.dim, 8);
    assert!((desc.kappa - 0.6).abs() < 1e-6);
    assert_eq!(desc.seed, 11);
}
```

End-to-end check that PR #9's Capabilities population works through the
real binary stack.

### 8.6 Why these 4 (and not more)

The architecture deliberately picks **convergence**, **determinism**,
**info**, **capabilities**.  These cover the 4 major Phase-2 RPCs that
participants/controllers expose.  The remaining RPCs (`Step`, `Reset`,
`State`, `Health`) are exercised transitively through these calls.

Adding more happy-path tests would inflate CI time without surfacing new
bugs ; the failure-mode tests (§ 9) are where high-value coverage lives.

---

## 9. `tests/integration_multiprocess_failure.rs` — failure modes

### 9.1 Test list (3 tests)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_fails_unavailable_when_participant_unreachable() { /* … */ }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn iterate_fails_when_participant_killed_before_call() { /* … */ }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_with_manifest_mismatch_fails_at_iterate() { /* … */ }
```

### 9.2 `iterate_fails_unavailable_when_participant_unreachable`

Spawn the controller with `EFE_META_SWARM_PARTICIPANTS=...,absent-id=127.0.0.1:1` (a port
known to be closed). Call Iterate. Expect `Code::Unavailable`.

```rust
async fn iterate_fails_unavailable_when_participant_unreachable() {
    let harness = ProcessHarness::spawn_kovant_with_missing("spend-categorization").await;
    harness.wait_for_ready_excluding_missing(Duration::from_secs(5)).await.unwrap();

    let mut client = MetaSwarmServiceClient::new(/* … */);
    let err = client
        .iterate(Request::new(iterate_request(8, 100, 1.0e-3)))
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unavailable);
}
```

`spawn_kovant_with_missing` spawns the controller and *only 2 of the 3*
participants ; the missing one's address points to a closed port. The
controller's `connect_participants` (PR #8) will fail at TCP connect with
a `tonic::transport::Error`, mapped to `ServerError::Discovery` →
`Code::Unavailable`.

### 9.3 `iterate_fails_when_participant_killed_before_call`

Spawn the full Kovant topology, SIGKILL one participant, then Iterate.

```rust
async fn iterate_fails_when_participant_killed_before_call() {
    let mut harness = ProcessHarness::spawn_kovant().await;
    harness.wait_for_ready(Duration::from_secs(5)).await.unwrap();

    // Kill one participant.
    harness.sigkill_participant("contract-risk");
    harness.wait_participant("contract-risk", Duration::from_secs(2))
        .await.expect("participant exited");

    // Give the OS a moment to release the TCP port.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = MetaSwarmServiceClient::new(/* … */);
    let err = client.iterate(Request::new(iterate_request(8, 100, 1.0e-3)))
        .await.unwrap_err();
    // Could be Unavailable (connection refused) or Internal (RST mid-call).
    assert!(
        matches!(err.code(), tonic::Code::Unavailable | tonic::Code::Internal),
        "unexpected error code : {:?} (message : {})",
        err.code(), err.message(),
    );
}
```

The exact error code depends on **when** the controller tries to talk to
the dead participant : at connect time → `Unavailable` ; mid-Iterate →
`Internal` ("Step RPC failed").  Both are valid recoverable states.

### 9.4 `controller_with_manifest_mismatch_fails_at_iterate`

Spawn the controller pointing at a manifest with ids `[a, b, c]`, but
spawn participants with ids `[a, b]` (one missing in the participant
side) — but the controller's `EFE_META_SWARM_PARTICIPANTS` points at the
participants that exist. **The mismatch is at the controller's manifest
declaring an id that no participant has.**

```rust
async fn controller_with_manifest_mismatch_fails_at_iterate() {
    let rows = vec![
        ParticipantManifest { id: "a".into(), dim: 8, kappa: 0.6, seed: 1, blas_backend: "none".into() },
        ParticipantManifest { id: "b".into(), dim: 8, kappa: 0.7, seed: 2, blas_backend: "none".into() },
        ParticipantManifest { id: "ghost".into(), dim: 8, kappa: 0.5, seed: 3, blas_backend: "none".into() },
    ];
    // Spawn only a and b ; manifest declares ghost but no subprocess
    // binds for it.  EFE_META_SWARM_PARTICIPANTS is constructed with
    // only a and b's addresses.
    let harness = ProcessHarness::spawn_partial_kovant(rows, &["a", "b"]).await;
    harness.wait_for_ready_subset(&["a", "b"], Duration::from_secs(5))
        .await.unwrap();

    let mut client = MetaSwarmServiceClient::new(/* … */);
    let err = client.iterate(Request::new(iterate_request(8, 100, 1.0e-3)))
        .await.unwrap_err();
    // The controller's lookup_endpoints (PR #8) returns Discovery error
    // for the participant manifest id not in static_participants.
    assert_eq!(err.code(), tonic::Code::Unavailable);
    assert!(err.message().contains("no static_participants entry"));
}
```

---

## 10. `tests/integration_multiprocess_lifecycle.rs` — shutdown drain

### 10.1 Test list (2 tests)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controller_sigterm_drains_inflight_iterate() { /* … */ }

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn participant_sigterm_then_controller_iterate_fails_cleanly() { /* … */ }
```

### 10.2 `controller_sigterm_drains_inflight_iterate`

The crown jewel of PR #10. Drives an Iterate, sends SIGTERM to the
controller mid-flight, asserts the Iterate either completes cleanly OR
returns a graceful error (not a connection reset), all within the drain
timeout.

```rust
async fn controller_sigterm_drains_inflight_iterate() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness.wait_for_ready(Duration::from_secs(5)).await.unwrap();

    let endpoint = harness.controller_endpoint();

    // Start an Iterate in a tokio task.
    let endpoint_for_task = endpoint.clone();
    let iterate_task = tokio::spawn(async move {
        let channel = Endpoint::from_shared(endpoint_for_task)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = MetaSwarmServiceClient::new(channel);
        client.iterate(Request::new(iterate_request(8, 100, 1.0e-3))).await
    });

    // Give the Iterate a head start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // SIGTERM the controller.
    let sigterm_at = std::time::Instant::now();
    harness.sigterm_controller();

    // Wait for the Iterate to settle.
    let result = iterate_task.await.expect("task joined");
    let elapsed = sigterm_at.elapsed();

    // Either : Ok (controller drained before max_wall_ms) ;
    // or : Err with Cancelled / Unavailable / DeadlineExceeded.
    match result {
        Ok(resp) => {
            assert_eq!(resp.into_inner().stop_reason, ProtoStopReason::Converged as i32);
        }
        Err(s) => {
            assert!(
                matches!(
                    s.code(),
                    tonic::Code::Cancelled | tonic::Code::Unavailable | tonic::Code::DeadlineExceeded,
                ),
                "unexpected error : {:?}",
                s.code(),
            );
        }
    }

    // The controller must exit within the drain timeout (~30 s).
    let mut harness = harness;
    let status = harness.wait_controller(Duration::from_secs(35))
        .await.expect("controller exited within drain timeout");
    // Clean exit code on graceful drain.
    assert!(status.success(), "controller exit status : {status:?}");
    assert!(
        elapsed < Duration::from_secs(35),
        "total time {elapsed:?} > drain timeout",
    );
}
```

### 10.3 `participant_sigterm_then_controller_iterate_fails_cleanly`

```rust
async fn participant_sigterm_then_controller_iterate_fails_cleanly() {
    let harness = ProcessHarness::spawn_kovant().await;
    harness.wait_for_ready(Duration::from_secs(5)).await.unwrap();

    // SIGTERM a participant ; wait for it to exit.
    harness.sigterm_participant("supplier-scoring");
    let mut harness = harness;
    harness.wait_participant("supplier-scoring", Duration::from_secs(35))
        .await.expect("participant drained");

    // Now Iterate against the controller ; expect a clean error.
    let mut client = MetaSwarmServiceClient::new(/* … */);
    let err = client.iterate(Request::new(iterate_request(8, 100, 1.0e-3)))
        .await.unwrap_err();
    assert!(
        matches!(err.code(), tonic::Code::Unavailable | tonic::Code::Internal),
        "unexpected error code : {:?}",
        err.code(),
    );
}
```

Participant drain takes ~ ms (no in-flight work).  Controller observes the
TCP connection close on the next Step. `Unavailable` (connect refused) or
`Internal` ("Step RPC failed") — both are clean errors.

---

## 11. `scripts/run_multiprocess_smoke.sh` — operator script

```bash
#!/usr/bin/env bash
#
# Manual smoke for the Phase-2 control plane.
# Spawns 1 controller + 3 participants on localhost using the binaries
# from target/debug/.  After ~ 1 s of startup, drives an Iterate via
# grpcurl and prints the result.
#
# Usage :
#   bash scripts/run_multiprocess_smoke.sh
#
# Requires : grpcurl (https://github.com/fullstorydev/grpcurl).
#
# Exit code 0 on convergence ; non-zero otherwise.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$SCRIPT_DIR/.."
TARGET="$ROOT/target/debug"

if [[ ! -x "$TARGET/controller" ]] || [[ ! -x "$TARGET/participant" ]]; then
    echo "missing target/debug/{controller,participant} ; run 'cargo build' first" >&2
    exit 1
fi

if ! command -v grpcurl >/dev/null 2>&1; then
    echo "grpcurl is required (https://github.com/fullstorydev/grpcurl)" >&2
    exit 1
fi

WORK=$(mktemp -d)
trap 'pkill -P $$ ; rm -rf "$WORK"' EXIT

# 1. Manifest.
cat > "$WORK/manifest.yaml" <<'EOF'
version: v0.2.0
meta:
  dim: 16
  safety:
    kind: box
    radius: 50.0
participants:
  - id: supplier-scoring
    dim: 8
    kappa: 0.6
    seed: 11
    blas_backend: "none"
  - id: contract-risk
    dim: 8
    kappa: 0.7
    seed: 22
    blas_backend: "none"
  - id: spend-categorization
    dim: 8
    kappa: 0.5
    seed: 33
    blas_backend: "none"
iteration:
  max_iters: 100
  tolerance: 0.0001
  alpha_default: 0.5
  coupling_gamma: 0.2
  deterministic: true
  max_wall_ms: 15000
EOF

# 2. Spawn participants.
EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_PARTICIPANT_ID=supplier-scoring \
EFE_META_SWARM_PARTICIPANT_ADDR=127.0.0.1:50081 \
EFE_META_SWARM_PARTICIPANT_METRICS=127.0.0.1:50091 \
"$TARGET/participant" >"$WORK/p1.log" 2>&1 &

EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_PARTICIPANT_ID=contract-risk \
EFE_META_SWARM_PARTICIPANT_ADDR=127.0.0.1:50082 \
EFE_META_SWARM_PARTICIPANT_METRICS=127.0.0.1:50092 \
"$TARGET/participant" >"$WORK/p2.log" 2>&1 &

EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_PARTICIPANT_ID=spend-categorization \
EFE_META_SWARM_PARTICIPANT_ADDR=127.0.0.1:50083 \
EFE_META_SWARM_PARTICIPANT_METRICS=127.0.0.1:50093 \
"$TARGET/participant" >"$WORK/p3.log" 2>&1 &

# 3. Spawn controller.
EFE_META_SWARM_MANIFEST="$WORK/manifest.yaml" \
EFE_META_SWARM_CONTROLLER_ADDR=127.0.0.1:50080 \
EFE_META_SWARM_CONTROLLER_METRICS=127.0.0.1:50090 \
EFE_META_SWARM_PARTICIPANTS=supplier-scoring=127.0.0.1:50081,contract-risk=127.0.0.1:50082,spend-categorization=127.0.0.1:50083 \
"$TARGET/controller" >"$WORK/c.log" 2>&1 &

# 4. Wait for startup.
sleep 1

# 5. Drive an Iterate.
echo "== iterating =="
grpcurl -plaintext \
    -d '{ "iteration": { "max_iters": 100, "tolerance": 1.0e-3, "alpha_default": 0.5, "coupling_gamma": 0.2, "deterministic": true, "max_wall_ms": 15000 } }' \
    -import-path "$ROOT/protos" \
    -proto efe/v1/meta_swarm/service_meta_swarm.proto \
    127.0.0.1:50080 efe.v1.meta_swarm.MetaSwarmService/Iterate
```

The script :

  * Generates a manifest in a temp dir.
  * Spawns 3 participant binaries with the proper env vars.
  * Spawns 1 controller binary.
  * `sleep 1` for startup (cheap ; operator can adjust).
  * Runs `grpcurl` with the IterateRequest.
  * `trap` ensures all child processes are killed on script exit.

Operator workflow :

```
$ cargo build
$ bash scripts/run_multiprocess_smoke.sh
== iterating ==
{
  "iterations": 47,
  "finalResidual": 0.000097,
  ...
  "stopReason": "STOP_REASON_CONVERGED"
}
```

---

## 12. `Cargo.toml` updates

```toml
[dev-dependencies]
# … existing entries …
libc = "0.2"     # Unix signals (SIGTERM) for graceful-shutdown tests
```

`libc` is already a transitive dep through tokio, but PR #10 makes the
dependency direct because we call `libc::kill` from the harness.  Adding
it to `[dev-dependencies]` only — production binaries don't reach this
path.

No production-dependency changes.

---

## 13. Determinism across the wire

PR #10's `iterate_e2e_bit_identical_3_reruns_over_wire` test is the
**definitive** determinism check for Phase 2.  For three reruns to
produce bit-identical `(iterations, final_residual)` :

  1. Each participant binary starts from the same seed (`MockParticipant::with_contraction(seed)`).
  2. Reset between runs returns each participant to its seeded state
     (`Reset(initial=None)` in PR #9).
  3. The controller's `MetaSwarm::iterate` (Phase 1) is deterministic on
     the same inputs.
  4. The gRPC wire format encodes f32 deterministically (IEEE 754 +
     little-endian, fixed by the proto contract).
  5. The participants' step_with_pull (Phase 1) is deterministic.
  6. Network reordering does not exist on localhost loopback for
     **request/response** pairs ; concurrent multi-channel calls
     happen in deterministic *manifest order* (PR #8's
     `connect_participants` preserves order).

If this test flakes, **one** of the six links above is broken — and PR #10
narrows the failure surface to that link.

---

## 14. CI gating + perf budget

### 14.1 Wall-time budget

| Test | p50 | p99 | Reason |
|---|---|---|---|
| `iterate_e2e_kovant_converges_across_real_processes` | 1.2 s | 1.8 s | spawn × 4 + Iterate (~ 800 ms) |
| `iterate_e2e_bit_identical_3_reruns_over_wire` | 2.0 s | 3.0 s | spawn × 4 + 3 × (Reset + Iterate) |
| `controller_info_lists_all_participants` | 0.6 s | 1.0 s | spawn × 4 + Info |
| `participant_capabilities_reflects_manifest_across_processes` | 0.6 s | 1.0 s | spawn × 4 + Capabilities |
| `iterate_fails_unavailable_when_participant_unreachable` | 0.8 s | 1.5 s | spawn × 3 + Iterate (fails at connect) |
| `iterate_fails_when_participant_killed_before_call` | 1.0 s | 1.5 s | spawn × 4 + kill + Iterate |
| `controller_with_manifest_mismatch_fails_at_iterate` | 0.8 s | 1.5 s | spawn × 3 + Iterate |
| `controller_sigterm_drains_inflight_iterate` | 2.5 s | 4.0 s | spawn × 4 + Iterate + SIGTERM + wait |
| `participant_sigterm_then_controller_iterate_fails_cleanly` | 1.5 s | 2.5 s | spawn × 4 + SIGTERM + wait + Iterate |
| **Total (sequential)** | **11 s** | **17.8 s** | |
| **Total (parallel, num_cpus = 4)** | **~ 3 s** | **~ 5 s** | tokio test harness runs tests in parallel by default |

### 14.2 CI memory budget

  * Each binary at idle : ~ 25 MB (RSS).
  * 4 binaries per test × 9 tests × parallelism : up to **9 GB**.
  * CI runners with < 8 GB RAM may hit OOM. **Mitigation** : set
    `RUST_TEST_THREADS=2` in CI for this crate.  This drops parallelism to
    2, total RSS ~ 2 GB.

This guidance is added to the README/CI config in PR #20 (deployment) ;
PR #10 documents it here.

### 14.3 Should the tests be gated ?

**No** (AD6) — the cost is ≤ 5 s of CI on parallel runners.  Operators
who want fast-iteration on the in-process tests use :

```
cargo test -p efe-meta-swarm-server --offline \
    --test integration_e2e_iterate \
    --test integration_iterate_inproc \
    --test integration_step_smoke
```

This skips the multiprocess suite.

---

## 15. Error mapping over process boundary

Errors that travel from a participant → controller → test client over the
wire :

| Origin | Path | Wire code | Test expectation |
|---|---|---|---|
| Participant returns `Status::failed_precondition` from `validate_step` | controller's `RemoteParticipant::step_async` wraps it in `MetaSwarmError::Internal("Step RPC failed")` ; orchestration maps to `Status::internal` | `INTERNAL` | n/a — Phase-1 doesn't see this in happy paths |
| Participant binary crashes mid-call | TCP RST on the controller's existing channel ; tonic returns `Code::Unavailable` ; controller wraps as `Status::internal("participant ... Step RPC failed")` | `INTERNAL` | tested in `iterate_fails_when_participant_killed_before_call` |
| Participant binary not started | controller's `RemoteParticipant::connect` fails at TCP connect ; `ServerError::Discovery` → `Code::Unavailable` | `UNAVAILABLE` | tested in `iterate_fails_unavailable_when_participant_unreachable` |
| Controller's `static_participants` doesn't have a manifest id | `lookup_endpoints` returns `ServerError::Discovery("no static_participants entry...")` | `UNAVAILABLE` | tested in `controller_with_manifest_mismatch_fails_at_iterate` |
| Controller is SIGTERM-ed mid-Iterate | (a) participants observe TCP close → return `Internal` ; (b) controller's `serve_with_shutdown` may also drop the handler future → client sees `Code::Unavailable` ; (c) graceful drain may complete the Iterate normally | `OK` *or* `Cancelled` *or* `Unavailable` *or* `DeadlineExceeded` | tested in `controller_sigterm_drains_inflight_iterate` |

The test assertions accept the set of legitimate error codes for each
scenario rather than pinning a single code, because the exact code
depends on timing.

---

## 16. Threat-model coverage

PR #10 mitigates or partially mitigates the following threats from
`docs/security/PHASE2_THREAT_MODEL.md` :

| Threat | Severity | PR #10 mitigation |
|---|---|---|
| **T7** (Iterate flood DoS) | SEV-2 | The `iterate_e2e_*` tests confirm the controller's `InflightSemaphore` (PR #8) survives across process boundary — the gate is not a single-process optimisation but a real backpressure mechanism the wire layer respects |
| **T12** (orphan process / zombie reaper) | SEV-3 | `ProcessHarness::Drop` SIGKILLs every child and `wait()`s ; the lifecycle tests verify `wait_*` reaps children within the drain timeout |
| **T13** (fan-out leak / cancellation cascade) | SEV-2 | Controller's `drop_guard` (PR #8) propagates cancel through participant channels ; the SIGTERM drain test exercises this end-to-end |
| **T17** (graceful drain insufficient) | SEV-2 | `controller_sigterm_drains_inflight_iterate` asserts ≤ 35 s total ; if the drain timeout is too short for a real Iterate, the test fails |
| **T18** (split-brain / mismatched manifests) | SEV-2 | `controller_with_manifest_mismatch_fails_at_iterate` asserts fail-fast at the controller side ; the participants never receive a Step targeting an unknown manifest |

T18 is **first** time it's exercised end-to-end (PR #8/9 only had it as a
unit-level scenario).

---

## 17. Concurrency model + lifecycle invariants

### 17.1 Invariants the harness maintains

  1. **No zombie processes after a test run** — `Drop` calls `wait()` on
     each `Child`.
  2. **No port collisions across concurrent test binaries** — every test
     allocates fresh ports via `allocate_port()`.
  3. **No tempfile leaks** — `NamedTempFile` drops with the harness.
  4. **Process tree is flat** — controller does not spawn participants ;
     the harness does.  This matches the production K8s deployment where
     pods are spawned independently.

### 17.2 The two SIGTERM paths

  * **Test-driven SIGTERM** : `harness.sigterm_*` sends SIGTERM ; the
    binary's `await_termination()` (PR #6's `observability::shutdown`)
    fires ; the binary drains and exits.
  * **Cargo-test-runner SIGTERM** : if the test runner itself is killed
    (`Ctrl-C` during `cargo test`), the test runner's tokio runtime
    cancels every test future ; each test's `ProcessHarness` is dropped ;
    `Drop` SIGKILLs all children.  Children leak only if the test runner
    is `kill -9`-ed.

### 17.3 Why we don't use a shared runtime between test harness and binaries

Each binary has its own tokio runtime.  Sharing would require either
spawning the binaries with a custom IPC channel (complex) or running
them in-process (defeats the whole point of PR #10).  Process isolation
is the entire value proposition.

---

## 18. Tests strategy summary

| Layer | File | Tests added | Coverage |
|---|---|---|---|
| Harness sanity | `tests/common/multiprocess.rs` | 4 in-module | Drop, spawn, sigterm, endpoint lookup |
| Happy paths | `tests/integration_multiprocess_e2e.rs` | 4 | convergence, determinism, info, capabilities |
| Failure modes | `tests/integration_multiprocess_failure.rs` | 3 | unreachable, killed-before, manifest mismatch |
| Shutdown | `tests/integration_multiprocess_lifecycle.rs` | 2 | controller SIGTERM, participant SIGTERM |
| **Total new** | | **13** | |

### 18.1 What's NOT tested (and why)

| Scenario | Why deferred |
|---|---|
| TLS handshake | PR #16 |
| mTLS auth failure | PR #16 |
| OOM kill (OOMKiller) | Phase 3 (requires cgroups) |
| Network partition | Phase 3 (requires iptables / tc-netem) |
| Slow disk / disk full | Phase 3 (binaries don't write to disk in Phase 2) |
| Process restart loops | PR #20 (deployment-level concern) |
| systemd / k8s probes | PR #20 |

---

## 19. Acceptance criteria (AC1–AC25)

PR #10 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `cargo build -p efe-meta-swarm-server` succeeds (debug + release) | CI |
| AC2 | `cargo test -p efe-meta-swarm-server --offline --all-targets` is **365 tests** (352 PR #9 baseline + 13 PR #10) | CI |
| AC3 | `tests/common/multiprocess.rs` : 4 in-module unit tests pass | CI |
| AC4 | `tests/integration_multiprocess_e2e.rs` : 4 tests pass | CI |
| AC5 | `tests/integration_multiprocess_failure.rs` : 3 tests pass | CI |
| AC6 | `tests/integration_multiprocess_lifecycle.rs` : 2 tests pass | CI |
| AC7 | Kovant scenario over real processes : `iterations ≤ 80`, `final_residual ≤ 1e-3`, `stop_reason == Converged` | AC4 |
| AC8 | Determinism : 3 reruns over real processes are bit-identical | AC4 |
| AC9 | Participant unreachable → `Code::Unavailable` | AC5 |
| AC10 | Participant killed → `Code::Unavailable` ∪ `Code::Internal` | AC5 |
| AC11 | Manifest mismatch → `Code::Unavailable` + "no static_participants entry" message | AC5 |
| AC12 | Controller SIGTERM drains within 35 s ; in-flight Iterate either succeeds or returns a clean error | AC6 |
| AC13 | Participant SIGTERM → subsequent Iterate fails cleanly with `Unavailable` ∪ `Internal` | AC6 |
| AC14 | Phase-1 tests pass (200 default) | CI |
| AC15 | PR #5 tests pass (15) | CI |
| AC16 | PR #6 tests pass (3 health smoke + lib bin tests) | CI |
| AC17 | PR #8 tests pass (lib + integration_iterate_inproc) | CI |
| AC18 | PR #9 tests pass (lib + step_smoke + e2e_iterate + participant_binary_smoke) | CI |
| AC19 | `cargo clippy --all-targets` 0 warnings on PR #10 files | CI |
| AC20 | `cargo fmt -- --check` exit 0 | CI |
| AC21 | `cargo doc --no-deps` clean | CI |
| AC22 | `bash scripts/run_multiprocess_smoke.sh` exits 0 on a fresh checkout | manual |
| AC23 | PR description references threat IDs `T7, T12, T13, T17, T18` | manual review |
| AC24 | sec-team confirms the manifest-mismatch detection at AC11 is sufficient for L2 | sec sign-off |
| AC25 | 4 reviewers approved : 2× meta-swarm + 1× sec + 1× sre | GitHub |

---

## 20. Risks + mitigations

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| **PR10-R1** | Tests flake under heavy CI load (port allocation race) | M / M | bind-drop-rebind has been stable across PR #8/9 ; if it flakes, switch to OS-level port reservation in PR #11 |
| **PR10-R2** | `RUST_TEST_THREADS` defaulting to `num_cpus` causes OOM on small CI runners | M / H | document `RUST_TEST_THREADS=2` in CI config ; add to PR description |
| **PR10-R3** | `libc::kill` is gated on `cfg(unix)` ; Windows builds skip the lifecycle tests | L / L | acceptable for Phase 2 (Linux-only) ; revisit in Phase 5 |
| **PR10-R4** | A flaky binary-startup ordering test leaves zombie processes that occupy ports for subsequent tests | L / M | Drop reaps children ; CI's pid_max is well above the 36 processes we spawn |
| **PR10-R5** | The SIGTERM drain test's 35 s timeout is too tight on a slow runner | L / M | timeout matches the binary's drain_timeout (30 s) + 5 s slack ; relaxable in one place |
| **PR10-R6** | `scripts/run_multiprocess_smoke.sh` depends on `grpcurl` which isn't always installed | L / L | script bails with a clear error message ; AC22 is manual, not CI |
| **PR10-R7** | Multi-process tests are slower than in-process tests by ~ 100× | M / L | ~ 5 s parallel wall time is acceptable ; document the cost ; allow operators to skip via `--test integration_e2e_iterate` style invocation |
| **PR10-R8** | Phase-1's algorithm has hidden non-determinism we haven't detected (e.g., rayon's work-stealing order influences floating-point sums) | L / H | the determinism test pins this contract ; if it ever fails, that's a real bug in Phase-1, not PR #10 |
| **PR10-R9** | The harness's hardcoded `Duration::from_secs(35)` for drain is brittle if a future PR shortens or lengthens the binary's drain | L / M | hoist into a named constant `DRAIN_TIMEOUT_PLUS_SLACK` shared between harness and bin |
| **PR10-R10** | `tonic`'s `keep_alive_*` settings cause idle connections to be torn down before a test finishes | L / L | tests don't keep idle connections long ; if it flakes, tune `keep_alive_interval` in the test client |
| **PR10-R11** | A subsequent PR removes `EFE_META_SWARM_MANIFEST` or renames it, breaking the harness env-var contract | M / H | the harness is part of the same crate ; CI catches the rename immediately |
| **PR10-R12** | `tempfile`'s tempfile cleanup races on macOS in CI | L / L | `NamedTempFile` uses unique names ; collision improbable |

---

## 21. Rollback strategy

If PR #10 lands and produces CI instability :

  1. **Per-test rollback** : each test is in its own file ; revert one
     file with `git revert --no-edit -- <commit>`.  The harness module
     stays in tree (unused but compiles).
  2. **Per-commit rollback** : C2/C3/C4 are independent ; revert C5 first
     (just the script), then C4 (lifecycle), C3 (failure), C2 (e2e).
  3. **Wholesale rollback** : `git revert <merge-commit>`. The crate
     returns to PR #9 state.  No data loss ; tests are read-only.
  4. **Hotfix** : if only `controller_sigterm_drains_inflight_iterate`
     flakes, add `#[ignore]` to that specific test and open a follow-up
     bug.

---

## 22. Sequence diagrams (ASCII)

### 22.1 Happy path — `iterate_e2e_kovant_converges_across_real_processes`

```
test                harness         controller proc   participant procs (×3)
 |                    |                    |                    |
 |  spawn_kovant()    |                    |                    |
 |───────────────────>|                    |                    |
 |                    | allocate ports     |                    |
 |                    | write manifest     |                    |
 |                    | spawn P1, P2, P3 ─────────────────────>|
 |                    | spawn controller ─>|                    |
 |                    | wait_for_ready ───>| Health             |
 |                    |                <───|                    |
 |                    |                    |  Health × 3 ─────>|
 |                    |                    |<──────────────────|
 |                    |<───────────────────|                    |
 | iterate(req)       |                    |                    |
 |─────────────────────────────────────────>|                    |
 |                    |                    | Step ─────────────>|
 |                    |                    |<──────────────────|
 |                    |                    | Step ─────────────>|
 |                    |                    |<──────────────────|
 |                    |                    |    (iterates ~ 47 ×)
 |<────────────────── IterateResponse ──────|                    |
 | drop harness      |                    |                    |
 |───────────────────>|                    |                    |
 |                    | SIGKILL all       ─────────────────────>|
 |                    | wait all          |                    |
```

### 22.2 SIGTERM drain — `controller_sigterm_drains_inflight_iterate`

```
test                harness         controller proc   participant procs
 |                    |                    |                    |
 |  spawn_kovant()    |                    |                    |
 |───────────────────>|                    |                    |
 |                    | spawn + wait_ready                       |
 |                    |<───────────────────|                    |
 | iterate (task) ─── tokio::spawn ─────>───|                    |
 |                    |                    | Step ─────────────>|
 |                    |                    |<──────────────────|
 | sleep 50ms         |                    |                    |
 | sigterm_controller |                    |                    |
 |───────────────────>|                    |                    |
 |                    | kill(SIGTERM) ────>| <intercepts>      |
 |                    |                    | health.set_not_serving
 |                    |                    | shutdown_tx.send  |
 |                    |                    | serve_with_shutdown
 |                    |                    |   completes      |
 |                    |                    |   in-flight       |
 |                    |                    |   Iterate.        |
 |                    |                    | Step ─────────────>|
 |                    |                    |<──────────────────|
 |                    |                    |   …               |
 |                    |                    | IterateResponse ──|
 | <─── join Iterate result ──────────────|                    |
 | wait_controller(35s)                     |                    |
 |───────────────────>|                    |                    |
 |                    | child.wait() ─────>| exits 0           |
 |                    |<───────────────────|                    |
```

### 22.3 Participant crash mid-call — `iterate_fails_when_participant_killed_before_call`

```
test                harness         controller proc   participant-2 proc
 |                    |                    |                    |
 |  spawn_kovant + ready                     |                    |
 |                    |                    |                    |
 | sigkill participant-2                   |                    |
 |───────────────────>|                    |                    |
 |                    | child.kill() ─────────────────────────>| <SIGKILL>
 |                    | wait                                    | exits 137
 | sleep 100ms        |                    |                    |
 | iterate ───────────────────────────────>|                    |
 |                    |                    | connect_participants
 |                    |                    | RemoteParticipant::connect
 |                    |                    |   to p2 ─────────X (refused)
 |                    |                    | Err(Unavailable)  |
 |<─── Status(UNAVAILABLE) ─────────────────|                    |
```

---

## 23. Reviewer checklist

```
## Reviewer checklist — PR #10

### Architecture
- [ ] `ProcessHarness` is the single orchestration point — AD4
- [ ] Drop is SIGKILL-only ; graceful shutdown is opt-in — AD4
- [ ] Tests cover happy / failure / lifecycle separately — § 8 / 9 / 10
- [ ] Manifest is a per-test tempfile — AD3
- [ ] `cfg(unix)` gates everything — AD11

### Correctness
- [ ] Each test uses fresh ports (no hardcoded conflicts)
- [ ] `libc::kill` is safe (SAFETY comment in source)
- [ ] `wait_for_ready` polls both Controller and Participant Health
- [ ] Failure tests don't deadlock if a process refuses to die
- [ ] Determinism test compares `(iterations, final_residual.to_bits())` (not approximate equality)

### CI
- [ ] Documented `RUST_TEST_THREADS=2` for memory-constrained runners
- [ ] No `#[ignore]` gates (the value justifies the cost)
- [ ] All 13 new tests pass with `cargo test`

### Threat model
- [ ] T7 / T12 / T13 / T17 / T18 are covered
- [ ] AC11 sec-team sign-off on T18

### Operability
- [ ] `scripts/run_multiprocess_smoke.sh` runs on a fresh checkout
- [ ] Operator sees a clear "manifest path / port collision / etc" error if env vars wrong
```

---

## 24. Migration notes (operator-facing)

After PR #10 :

  1. **No production code change.**  The binaries' env-var contract and
     wire format are unchanged from PR #9.
  2. **A new shell script** `scripts/run_multiprocess_smoke.sh` is
     available for manual validation in operator-style workflows.
  3. **CI runners need `RUST_TEST_THREADS=2`** on memory-constrained
     environments.  The PR description and the README link explain why.
  4. **Phase-2 graduation gates G4 + G7 flip GREEN**.  The roadmap doc
     reflects this.

No env-var changes, no schema changes, no breaking changes.

---

## 25. Open questions

| ID | Question | Status |
|---|---|---|
| **OQ-PR10-1** | Should the harness's stderr capture be made opt-in for debugging ? | Defer to PR #18 (when we add tracing/OTel) |
| **OQ-PR10-2** | Should the lifecycle test cover `SIGINT` (Ctrl-C) in addition to SIGTERM ? | Skip — `SIGINT` is handled identically by the binary's signal handler ; testing SIGTERM covers both |
| **OQ-PR10-3** | Should we hoist `DRAIN_TIMEOUT` from `bin/controller.rs` into a public constant the harness can import ? | Yes — risk PR10-R9 ; promote in PR #11 |
| **OQ-PR10-4** | Should we add a "participant slow to start" test (delay before binding) ? | Skip — requires wrapping the binary in a script that sleeps before exec ; complex for marginal coverage |
| **OQ-PR10-5** | Should the manifest fixture be parametrised on the controller's `default_max_wall_ms` ? | Skip — current 15 000 ms is well over the 800 ms convergence ; add when a test needs it |
| **OQ-PR10-6** | If a CI runner has a flaky network stack, do these tests fail with cryptic errors ? | Acceptable — `127.0.0.1` should never have network issues ; document if it happens in practice |
| **OQ-PR10-7** | Should we exercise the controller's `IterateStream` deferred surface in PR #10 ? | Skip — PR #21 wires `IterateStream` |
| **OQ-PR10-8** | Is 35 s drain timeout enough for a 30 s Iterate to complete ? | Yes — the binary's drain_timeout (config.drain_timeout) is 30 s ; 5 s slack is sufficient ; tunable via `EFE_META_SWARM_DRAIN_TIMEOUT_S` |

---

## 26. Appendix : full file inventory

### 26.1 Files added (5)

```
crates/efe-meta-swarm-server/tests/common/multiprocess.rs                     NEW    ~ 350 LOC
crates/efe-meta-swarm-server/tests/integration_multiprocess_e2e.rs           NEW    ~ 250 LOC
crates/efe-meta-swarm-server/tests/integration_multiprocess_failure.rs       NEW    ~ 250 LOC
crates/efe-meta-swarm-server/tests/integration_multiprocess_lifecycle.rs     NEW    ~ 200 LOC
scripts/run_multiprocess_smoke.sh                                              NEW    ~  80 LOC
docs/EFE_A1_PHASE2_PR10_ARCHITECTURE.md                                       NEW    this file
```

### 26.2 Files modified (2)

```
crates/efe-meta-swarm-server/tests/common/mod.rs    MODIFIED   +3 LOC : re-export multiprocess module
crates/efe-meta-swarm-server/Cargo.toml             MODIFIED   +1 LOC : dev-dep libc = "0.2"
```

### 26.3 Files unchanged (relevant)

```
crates/efe-meta-swarm-server/src/                              (no production code change)
crates/efe-meta-swarm-server/src/bin/{controller,participant}.rs (unchanged)
crates/efe-meta-swarm-proto/                                    (no proto change)
crates/efe-meta-swarm/                                          (no Phase-1 change)
```

### 26.4 LOC tally

| Category | LOC |
|---|---|
| New harness | ~ 350 |
| New tests | ~ 700 |
| New script | ~ 80 |
| Modified | ~ 4 |
| **Net** | **~ +1 134 LOC** |

---

## 27. Glossary

| Term | Definition |
|---|---|
| **AC** | Acceptance Criterion |
| **AD** | Architectural Decision |
| **L2 gate** | Phase-2 graduation gate |
| **ProcessHarness** | (this PR) RAII orchestrator owning controller + participant subprocesses |
| **Kovant scenario** | Reference 3-participant topology (`supplier-scoring`, `contract-risk`, `spend-categorization`) used for benchmarks |
| **T2** | Phase-2 single-node topology (1 controller + 3 participants on `localhost`) |
| **SIGTERM** | Unix termination signal ; handled by the binary's signal-driven graceful drain |
| **SIGKILL** | Unix unconditional-kill signal ; bypasses graceful drain ; used by `child.kill()` |
| **bind-drop-rebind** | Port allocation pattern : bind `127.0.0.1:0` to get an OS-chosen port, drop the listener, then re-bind the same port in the subprocess.  TOCTOU race is microseconds-wide |
| **drain timeout** | The time after SIGTERM during which in-flight requests can complete ; bin/controller.rs uses 30 s ; tests use 35 s with slack |
| **PID wrap-around** | On long-running Linux systems, PIDs can recycle ; `libc::kill(old_pid, ...)` could hit an unrelated process.  PR #10 holds `Child` until Drop, so it can never SIGTERM a recycled PID |
| **`#[cfg(unix)]`** | Rust conditional compilation gate that includes the code only on Unix platforms |
| **grpcurl** | gRPC equivalent of curl ; required by `scripts/run_multiprocess_smoke.sh` |
| **RUST_TEST_THREADS** | env var controlling cargo test's parallelism ; recommended at 2 for memory-constrained CI |

---

*End of architecture document. Total: ~ 2 020 lines.*

*Next deliverable : PR #10 code (5 commits).*
