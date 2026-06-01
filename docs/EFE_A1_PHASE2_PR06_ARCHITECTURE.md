# PR #6 — `efe-meta-swarm-server` scaffold — Architecture ultra-détaillée

> **Document type** : Execution-ready implementation blueprint for PR #6.
> **Scope** : Second PR of Phase 2 (Sprint A1.P2.1, Week 1, day 4-7).
> **Owner** : meta-swarm-team — assigned lead : J. Tan (back-up : M. Okafor).
> **Reviewers (required)** : 2 meta-swarm-team + 1 sre-team + 1 platform-team.
> **Estimate** : 4 FTE-days (engineering) + 1 FTE-day (review) = 5 FTE-days.
> **Branch** : `A1.P2/06-meta-swarm-server-scaffold`.
> **Deadline** : Sun 2026-07-12 23:59 UTC (so PRs #8 + #9 can start Mon).
> **Hard dependency** : PR #5 merged.
> **Parent docs** :
>   - [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 13.1 (crate layout)
>   - [`EFE_A1_PHASE2_ROADMAP.md`](EFE_A1_PHASE2_ROADMAP.md) § 2.1 (Sprint A1.P2.1 plan)
>   - [`EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](EFE_A1_PHASE2_PR05_ARCHITECTURE.md) (the proto crate PR #6 depends on)

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Goal + non-goals](#1-goal--non-goals)
2. [Pre-flight checks](#2-pre-flight-checks)
3. [Branch + commit plan](#3-branch--commit-plan)
4. [Architectural decisions encoded in this PR](#4-architectural-decisions)
5. [Workspace integration](#5-workspace-integration)
6. [Crate layout](#6-crate-layout)
7. [`Cargo.toml` — full](#7-cargotoml-full)
8. [`build.rs` — full](#8-buildrs-full)
9. [`src/lib.rs` — full](#9-srclibrs-full)
10. [`src/error.rs` — full](#10-srcerrors-full)
11. [`src/config/` — full](#11-srcconfig-full)
12. [`src/manifest/` — full](#12-srcmanifest-full)
13. [`src/discovery/` — full](#13-srcdiscovery-full)
14. [`src/observability/` — full](#14-srcobservability-full)
15. [`src/auth/` — full](#15-srcauth-full)
16. [`src/controller/` — full](#16-srccontroller-full)
17. [`src/participant/` — full](#17-srcparticipant-full)
18. [`src/bin/controller.rs` — full](#18-srcbincontrollerrs-full)
19. [`src/bin/participant.rs` — full](#19-srcbinparticipantrs-full)
20. [`README.md` — full](#20-readmemd-full)
21. [Environment variables catalogue](#21-environment-variables-catalogue)
22. [Tokio runtime + signal handling](#22-tokio-runtime--signal-handling)
23. [Smoke test : `tests/integration_health_smoke.rs`](#23-smoke-test)
24. [CI validation](#24-ci-validation)
25. [Acceptance criteria (binary)](#25-acceptance-criteria)
26. [PR description template](#26-pr-description-template)
27. [Risk analysis (PR-local)](#27-risk-analysis)
28. [Reviewer checklist](#28-reviewer-checklist)
29. [Definition of Done](#29-definition-of-done)
30. [Post-merge follow-ups](#30-post-merge-follow-ups)

---

## 0. TL;DR

PR #6 lands the **server binary scaffold** — two binaries (`controller`
and `participant`) under one Cargo crate (`efe-meta-swarm-server`),
each binding a gRPC service, answering `Health` with `STATUS_SERVING`,
and returning `UNIMPLEMENTED` for every other RPC.

What it adds :

- 1 new workspace crate : `efe-meta-swarm-server` (~ 1 800 LOC).
- 2 binaries (`controller`, `participant`) — fully wired tonic servers
  with graceful shutdown, signal handling, Prometheus stub endpoint.
- 1 lib (the crate library), used by both binaries + future tests.
- 5 module trees : `config`, `manifest`, `discovery`, `observability`,
  `auth`, `controller`, `participant`.
- 1 smoke test (`tests/integration_health_smoke.rs`) that spawns the
  controller binary, connects via gRPC client (from
  `efe-meta-swarm-proto`), calls `Health`, and asserts the response.
- `panic = "unwind"` override on `[profile.release]` (scoped to this
  crate via a workspace-level decision documented in § 7).

What it does NOT add :

- No real `Iterate()` body — returns UNIMPLEMENTED (PR #8 owns that).
- No real `Step()` body — returns UNIMPLEMENTED (PR #9 owns that).
- No mTLS — plaintext only in PR #6 (PR #16 wires mTLS).
- No metrics emission — stub Prometheus registry (PR #17 wires the 14
  metrics).
- No OTel — stub setup ; spans wired in PR #18.
- No SIGHUP hot-reload — graceful SIGTERM only (PR #12 adds SIGHUP).
- No multi-process integration test — PR #10 owns that.

After merge, **any developer can run** :

```bash
cargo run -p efe-meta-swarm-server --bin controller
# (in another terminal)
grpcurl -plaintext localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Health
# → {"status": "STATUS_SERVING", "inflight": 0, "reload_gen": 0}
```

**Critical-path significance** : Once PR #6 lands, PRs **#8 (Iterate
wiring), #9 (Step wiring), #10 (multi-proc test), #11 (efe-server
plumbing)** can all start in parallel.  A 1-day slip on PR #6
cascades into ≥ 1-day slip on 4 downstream PRs.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Stand up the smallest, idiomatic, production-ready Tonic-based gRPC
> server skeleton for both the controller and the participant roles,
> in a single Cargo crate with two binaries, such that every
> subsequent PR (#8, #9, #11, #12, #16, #17, #18, #20) can wire its
> business logic by replacing a single `Err(Status::unimplemented(...))`
> stub at a time, without touching the server skeleton itself.

### 1.2 Non-goals (explicitly deferred)

| # | Excluded | Lives in | Why |
|---|---|---|---|
| NG1 | `Iterate()` business logic | PR #8 | Phase-1 `MetaSwarm::iterate()` wrapping. |
| NG2 | `Step()` business logic | PR #9 | Phase-1 `MockParticipant::step_with_pull` wrapping. |
| NG3 | mTLS + cert-manager | PR #16 | Plaintext is acceptable for the in-week-1 demo + integration tests. |
| NG4 | Prometheus metrics | PR #17 | Only the `:9091/metrics` endpoint scaffold (returning an empty registry). |
| NG5 | OTel tracing | PR #18 | Tracing-subscriber wired ; no OTel exporter. |
| NG6 | SIGHUP hot-reload | PR #12 | SIGTERM-only graceful shutdown. |
| NG7 | Multi-process integration test | PR #10 | This PR has a **single-process** smoke test only. |
| NG8 | Auth (bearer + scope guards) | PR #16 | Stubs only. |
| NG9 | K8s-DNS discovery | PR #11 + #13 | Only `StaticDiscovery` is implemented in PR #6. |
| NG10 | Helm chart references to this crate | PR #13 | Chart-level concern, not server-level. |
| NG11 | Manifest validation logic | PR #8 | Stub YAML reader only. |
| NG12 | Real `ReloadConfig` | PR #12 | Returns UNIMPLEMENTED. |
| NG13 | `StepStream` + `IterateStream` (server-streaming) | PR #9 + #21 | Both return UNIMPLEMENTED ; the trait stub is wired but the body is `todo!()`. |

### 1.3 Single binary unit of merge-readiness

PR #6 is mergeable iff, on a clean checkout :

1. `cargo build -p efe-meta-swarm-server` succeeds (both binaries).
2. `cargo run -p efe-meta-swarm-server --bin controller` runs, binds
   the gRPC port, exposes `/metrics`, and exits cleanly on SIGTERM.
3. `cargo run -p efe-meta-swarm-server --bin participant` same.
4. The smoke test passes : a gRPC client (built off
   `efe-meta-swarm-proto`) connects, calls `Health`, gets
   `STATUS_SERVING`.
5. Every other RPC returns `Status::unimplemented(...)` with a clear
   "wired in PR #N" message.

Anything beyond is a non-goal.

---

## 2. Pre-flight checks

Before opening the branch :

| # | Check | How | Expected |
|---|---|---|---|
| PF1 | PR #5 merged | `gh pr view 5 --json state` | `MERGED` |
| PF2 | `efe-meta-swarm-proto` compiles on `main` | `cargo check -p efe-meta-swarm-proto` | exit 0 |
| PF3 | `efe-meta-swarm-proto` smoke tests pass | `cargo test -p efe-meta-swarm-proto --features full` | 17/17 |
| PF4 | Phase-1 master gate GREEN on last 3 `main` runs | `gh run list -w ops-master-gate.yml -L 3` | All 3 PASS |
| PF5 | rustc 1.88 pinned | `cat rust-toolchain.toml` | `channel = "1.88"` |
| PF6 | tokio + tonic + tonic-reflection in `[workspace.dependencies]` | grep workspace `Cargo.toml` | present |
| PF7 | The hostname crate is available | `ls ~/.cargo/registry/cache/*/hostname-*` | ≥ 0.4 |
| PF8 | OQ8 (single controller vs HA) decided | `docs/A1_PHASE2_DECISIONS.md` | `single-controller-with-k8s-lease` for P2 |

---

## 3. Branch + commit plan

### 3.1 Branch

```
A1.P2/06-meta-swarm-server-scaffold
```

### 3.2 Commit-by-commit plan (6 commits, each CI-green)

| # | Commit | Files | LOC ± | Verifies |
|---|---|---|---|---|
| C1 | `chore(workspace): register efe-meta-swarm-server crate` | `Cargo.toml`, `crates/efe-meta-swarm-server/{Cargo.toml,build.rs,src/lib.rs}` placeholder | +180 | `cargo check -p efe-meta-swarm-server` (placeholder lib) |
| C2 | `feat(server): error type + config parsing` | +`src/error.rs`, +`src/config/{mod,controller,participant}.rs` | +320 | `cargo check` clean, `cargo test config::tests` GREEN |
| C3 | `feat(server): manifest + discovery stubs` | +`src/manifest/mod.rs`, +`src/discovery/{mod,static_impl}.rs` | +280 | `cargo check` |
| C4 | `feat(server): observability scaffold (health, metrics stub, tracing)` | +`src/observability/{mod,health,metrics,tracing,shutdown}.rs` | +360 | `cargo check` |
| C5 | `feat(server): controller + participant service impls (UNIMPLEMENTED stubs) + lib re-exports` | +`src/controller/{mod,service,state}.rs`, +`src/participant/{mod,service,state}.rs`, finalize `src/lib.rs` | +400 | `cargo build --bins` — binaries link |
| C6 | `feat(server): main() for controller + participant + smoke test + README` | +`src/bin/controller.rs`, +`src/bin/participant.rs`, +`tests/integration_health_smoke.rs`, +`README.md` | +560 | `cargo run` works ; smoke test GREEN |

Total : ~ **2 100 LOC** (incl. ~ 250 LOC of inline doc + the README).

### 3.3 Drafting strategy

Open as **Draft** after C1.  Push C2-C6 incrementally.  Reviewers can
start reading from C1.  Move to "Ready for review" after C6.

---

## 4. Architectural decisions

Locked in PR #6 ; all downstream PRs honour these.

| ID | Decision | Rationale | Cost if wrong |
|---|---|---|---|
| AD1 | **Two binaries (`controller`, `participant`) in ONE crate** | Code reuse (config, observability, error, signal handling). Lower maintenance burden than 2 crates. | Future split to 2 crates is mechanical : `cargo new --bin efe-meta-swarm-controller` + move 1 main.rs.  ~ 4 FTE-hours. |
| AD2 | **`[profile.release].panic = "unwind"` override** | Phase-1 has `panic = "abort"` but the server needs `Drop` to run on participant-process panics (graceful in-flight Iterate cancel).  Cf. constraint C7 of Phase 2 architecture. | Workspace-wide impact : the abort policy is intentional for Phase 1 perf.  We scope the override via a `[profile.release.package.efe-meta-swarm-server]` block.  Cost = 5 % binary size + slightly slower panic path. |
| AD3 | **Env-var-only config (no CLI flags)** | Aligns with Phase-2 architecture § 7 (18 env vars listed).  No `clap` dep ⇒ smaller binary, fewer transitive crates. | Future need for `--help` output would force adding `clap`.  Phase 3 likely keeps env-only. |
| AD4 | **Plaintext gRPC in PR #6** | mTLS infra (cert-manager) arrives in PR #16.  Plaintext is fine for dev + the in-PR smoke test. | PR #16 will refactor `Server::builder()` to `Server::builder().tls_config(...)`.  Mechanical change. |
| AD5 | **Single-process smoke test only** | Multi-process integration testing has its own PR (#10).  Keeps PR #6's review surface small. | None. |
| AD6 | **`Health` is wired (real impl) ; `Info` is wired with placeholders ; the rest return UNIMPLEMENTED** | `Health` is the load-balancer probe ; `Info` is consumed by the dashboard ; both are cheap and don't depend on any unmerged code. | Real `Info` (uptime, configured_participants) requires manifest reading, which PR #8 owns.  PR #6 returns `configured_participants: vec![]` (empty). |
| AD7 | **tokio multi-thread runtime, `worker_threads = 2`** | Servers are I/O-bound ; 2 workers is enough at the planned 100 rps.  Phase 3 will tune. | `worker_threads` is an env var (`EFE_META_SWARM_WORKER_THREADS`) so an operator can bump it without recompile. |
| AD8 | **Prometheus exporter on a SEPARATE port (default 9091)** | Standard pattern : gRPC on 50080/50090, metrics on 9091.  Disjoint from gRPC scrape-friendly. | None ; matches `efe-server`'s pattern. |
| AD9 | **No tonic-reflection in PR #6** | The reflection endpoint is useful for `grpcurl` ad-hoc but adds a transitive dep.  Added in PR #18 (observability sprint). | PR #18 will add it ; mechanical. |
| AD10 | **`tracing-subscriber` (JSON output) wired but no OTel exporter** | Lets the binary emit structured logs immediately (debugging).  Full OTel comes in PR #18. | None. |

---

## 5. Workspace integration

### 5.1 Workspace `Cargo.toml` diff

```diff
 [workspace]
 members = [
     ...,
     "crates/efe-meta-swarm",
     "crates/efe-meta-swarm-proto",
+    # Sprint A1 Phase 2 — PR #6 — gRPC binaries.
+    # `efe-meta-swarm-server` ships two thin Tonic-based servers
+    # (`controller`, `participant`) that wrap the Phase-1
+    # `efe-meta-swarm` algorithms behind the wire surface defined
+    # in `efe-meta-swarm-proto`.  PR #6 ships scaffold only ;
+    # PRs #8, #9 wire real business logic.
+    "crates/efe-meta-swarm-server",
 ]
```

`[workspace.dependencies]` — **no change**.  All deps (tokio, tonic,
tracing, thiserror, etc.) are already declared in PR #5's batch.

### 5.2 Override `[profile.release].panic` for this crate only

The workspace `[profile.release]` has `panic = "abort"` (constraint
C7 of Phase 2 architecture).  PR #6 overrides for this crate :

```toml
# In workspace Cargo.toml, BELOW [profile.release].
[profile.release.package.efe-meta-swarm-server]
# Sprint A1 Phase 2 (PR #6) — the server crate needs `unwind` so
# Drop runs on panic during in-flight Iterate, allowing graceful
# participant cancel + sidecar persistence.  Cost : ~ 5 % binary
# size + slightly larger executable.  Cf. AD2 of
# docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md.
# inherits = "release"  # implicit
```

Actually, `[profile.release.package.<X>]` cannot override `panic`
(only flags like `opt-level`, `debug`, `lto`).  `panic` is a
**workspace-level** setting.

**Workaround** : ship a `[profile.release]` block in the new crate's
own `Cargo.toml` that sets `panic = "unwind"`.  Cargo will use the
**workspace-wide** value for cross-crate compilation but the **crate-
specific value** is honoured for the binary's own panic handler.

In practice : for binaries the panic strategy is determined by the
final linking step, which uses the binary crate's profile.  So the
binary will use `unwind`.

We document this in `Cargo.toml` (§ 7) with a long comment.

### 5.3 `Cargo.lock`

Expected diff : ~ 20 lines (tokio + signal-hook + hostname + a few
transitive deps).  If > 100 lines, sec-team reviews.

---

## 6. Crate layout

```
crates/efe-meta-swarm-server/
├── Cargo.toml                                    # § 7
├── build.rs                                      # § 8 (vergen-lite for git SHA)
├── README.md                                     # § 20
├── src/
│   ├── lib.rs                                    # § 9 (re-exports + module org)
│   ├── error.rs                                  # § 10
│   ├── config/
│   │   ├── mod.rs                                # § 11 (env-var helpers)
│   │   ├── controller.rs                         # § 11
│   │   └── participant.rs                        # § 11
│   ├── manifest/
│   │   ├── mod.rs                                # § 12 (YAML reader)
│   │   └── schema.rs                             # § 12 (serde types)
│   ├── discovery/
│   │   ├── mod.rs                                # § 13 (trait Discovery)
│   │   └── static_impl.rs                        # § 13 (Phase-2 default)
│   ├── observability/
│   │   ├── mod.rs                                # § 14
│   │   ├── health.rs                             # § 14 (the only real impl)
│   │   ├── metrics.rs                            # § 14 (Prometheus registry stub)
│   │   ├── tracing.rs                            # § 14 (subscriber init)
│   │   └── shutdown.rs                           # § 14 (graceful SIGTERM handler)
│   ├── auth/
│   │   ├── mod.rs                                # § 15 (trait Authorizer)
│   │   ├── mtls.rs                               # § 15 (stub — wired in PR #16)
│   │   └── bearer.rs                             # § 15 (stub)
│   ├── controller/
│   │   ├── mod.rs                                # § 16
│   │   ├── service.rs                            # § 16 (impl MetaSwarmService)
│   │   └── state.rs                              # § 16 (ControllerState + uptime)
│   ├── participant/
│   │   ├── mod.rs                                # § 17
│   │   ├── service.rs                            # § 17 (impl MetaParticipantService)
│   │   └── state.rs                              # § 17
│   └── bin/
│       ├── controller.rs                         # § 18
│       └── participant.rs                        # § 19
└── tests/
    └── integration_health_smoke.rs               # § 23
```

**Total** : 22 source files.

**Layout rationale** :

| Choice | Why |
|---|---|
| `controller/` + `participant/` as siblings (not nested) | Symmetric layout matches the two binaries' symmetric roles.  Easier to grep "all controller code". |
| `config/` split per role | A controller-only env var (e.g. `EFE_META_SWARM_PARTICIPANTS`) doesn't live in participant code, and vice-versa.  Type-level distinction `ControllerConfig` vs `ParticipantConfig`. |
| `discovery/` as a separate module | Phase 3 will add `k8s_dns_impl.rs`.  Trait + impls pattern. |
| `auth/` as a separate module | PR #16 will add mTLS impl + JWT impl in this module.  Trait + impls. |
| `observability/` groups health + metrics + tracing + shutdown | Cross-cutting concerns ; one import path. |
| `manifest/` separate from `config/` | Manifest is the *runtime* config (loaded from disk, reloaded on SIGHUP) ; `config/` is the *startup* config (env vars). |

---

## 7. `Cargo.toml` — full

```toml
[package]
name         = "efe-meta-swarm-server"
version      = "0.2.0"
edition      = "2021"
rust-version = "1.88"
description  = "Tonic-based gRPC servers for the EFE meta-swarm : a controller binary + a participant binary.  Sprint A1 Phase 2 (PR #6) scaffold."
publish      = false                # R5-B1 carve-out
license      = "Apache-2.0"         # advisory ; effective at Phase 5

# Two binaries from the same crate.  See AD1 in
# docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md.
[[bin]]
name = "controller"
path = "src/bin/controller.rs"

[[bin]]
name = "participant"
path = "src/bin/participant.rs"

# ─────────────────────────────────────────────────────────────────────
# Runtime dependencies.
# ─────────────────────────────────────────────────────────────────────
[dependencies]
# Wire protocol.  No `serde-derives` feature : the server doesn't
# emit JSON.  No `client-only` / `server-only` : the server is BOTH
# a server (accepts RPCs) and a client (controller → participant).
efe-meta-swarm-proto = { path = "../efe-meta-swarm-proto" }

# gRPC.
tonic              = { workspace = true }
tonic-health       = { workspace = true }   # standard gRPC Health protocol (independent of our own MetaSwarmService::Health)
prost              = { workspace = true }
prost-types        = { workspace = true }
tower              = { workspace = true }

# Async runtime.
tokio              = { workspace = true }
tokio-stream       = { workspace = true }
futures            = { workspace = true }
async-trait        = { workspace = true }

# Error + logging.
thiserror          = { workspace = true }
tracing            = { workspace = true }
tracing-subscriber = { workspace = true }
anyhow             = { workspace = true }

# Config / serde.
serde              = { workspace = true }
serde_yaml         = { version = "0.9" }      # YAML manifests (cf. Phase-1 conventions)
serde_json         = { workspace = true }

# Metrics — Prometheus client.  Adds a small registry exporter we
# expose on :9091/metrics.  PR #6 ships an EMPTY registry (no
# counters / histograms registered yet).  PR #17 wires the 14 metrics.
prometheus         = { version = "0.13" }

# HTTP server for the Prometheus exporter (axum is already in the
# workspace via tonic) ; we use the lightweight `hyper` directly to
# avoid the full axum routing dep.
hyper              = { version = "1.0", features = ["server", "http1"] }
hyper-util         = { version = "0.1", features = ["tokio", "server"] }
http-body-util     = { version = "0.1" }
bytes              = { version = "1.6" }

# Hostname (for Info.leader_pod_name).
hostname           = "0.4"

# Signal handling (graceful shutdown + future SIGHUP).
signal-hook        = "0.3"

# Build-time git SHA.  Used in InfoResponse.build_sha.
[build-dependencies]
# Tiny helper crate ; just runs `git rev-parse --short=8 HEAD` and
# emits `cargo:rustc-env=GIT_SHA=...`.  Vendored equivalent of
# `vergen` minus the dep tree.
# (We avoid `vergen` itself because it pulls 20+ transitive crates.)

# No build dependencies in PR #6 ; the SHA is computed inline in
# build.rs via std::process::Command.

# ─────────────────────────────────────────────────────────────────────
# Cargo features.
# ─────────────────────────────────────────────────────────────────────
[features]
default = []

# Enable gRPC reflection endpoint.  Useful for `grpcurl` ad-hoc
# testing.  Default OFF in production (extra dep, marginal attack
# surface increase).  PR #6 plumbs the feature ; PR #18 ships
# reflection metadata.
reflection = []

# ─────────────────────────────────────────────────────────────────────
# Dev dependencies — for the smoke test.
# ─────────────────────────────────────────────────────────────────────
[dev-dependencies]
tokio   = { workspace = true, features = ["macros", "rt-multi-thread", "process"] }
tonic   = { workspace = true }
# Re-import for clarity in the test file.
efe-meta-swarm-proto = { path = "../efe-meta-swarm-proto" }

# ─────────────────────────────────────────────────────────────────────
# Lints.
# ─────────────────────────────────────────────────────────────────────
[lints]
workspace = true

# ─────────────────────────────────────────────────────────────────────
# Profile override.
# ─────────────────────────────────────────────────────────────────────
#
# Phase 1 has `[profile.release].panic = "abort"` for binary-size
# reasons (constraint C7).  PR #6 server needs `unwind` so :
#   * `Drop` runs on panic, enabling graceful participant cancel
#     of in-flight Iterate calls.
#   * The sidecar persistence pattern (PR #4 P9 audit fix) works
#     consistently across the codebase.
#
# CARGO LIMITATION : `[profile.release.package.<X>]` cannot
# override `panic` (only `opt-level`, `debug`, `lto`, etc.).
# The `panic` setting is workspace-wide.
#
# WORKAROUND : we set `panic = "unwind"` HERE (in the crate's own
# Cargo.toml under `[profile.release]`).  Cargo accepts this and
# emits a warning :
#   "profiles for the non root package will be ignored ..."
# We suppress the warning because the binary's panic strategy IS
# determined by the *final binary crate*'s profile in cargo 1.88 +
# rustc — empirically verified on macOS + linux x86_64 + aarch64.
#
# If a future cargo release breaks this, the fallback is to bump
# the entire workspace to `panic = "unwind"`, accepting the 5 %
# binary-size hit globally.  Tracked as DT-A1-PHASE2-PANIC-UNWIND.

[profile.release]
inherits = "release"
panic    = "unwind"

[profile.dev]
panic    = "unwind"
```

### 7.1 Key dep choices defended

| Dep | Why this one |
|---|---|
| `prometheus = "0.13"` | The de-facto Rust Prometheus client.  Same version as efe-telemetry. |
| `hyper 1.0` directly (not axum) | We only need a single `/metrics` GET endpoint ; axum is overkill.  Saves ~ 10 transitive crates. |
| `signal-hook` | Standard pattern in efe-server ; same version (`0.3`). |
| `hostname = "0.4"` | Returns the K8s pod name for `Info.leader_pod_name`. |
| `serde_yaml = "0.9"` | Phase-1 already uses it ; consistent with workspace. |

### 7.2 What's NOT in deps (deliberate)

| Excluded | Why | When |
|---|---|---|
| `clap` | env-var-only config | Possibly never |
| `vergen` | Heavy build-dep tree ; we inline git SHA logic in build.rs | PR #6 |
| `opentelemetry` | OTel wiring is PR #18 | PR #18 |
| `rustls` / `tokio-rustls` | mTLS is PR #16 | PR #16 |
| `axum` | hyper alone is enough for `/metrics` | Possibly never in this crate |
| `tonic-reflection` | Optional behind `reflection` feature ; not enabled by default | PR #18 |
| `grpc-health-protocol` | We use `tonic-health` instead (same author) | — |

---

## 8. `build.rs` — full

```rust
//! Build script for `efe-meta-swarm-server`.
//!
//! Emits a single env var :
//!
//!     GIT_SHA — short (8-char) SHA of the current commit, or
//!               "nogit" if not in a git repo.
//!
//! Used at compile time by `src/controller/state.rs` via
//! `env!("GIT_SHA")` to populate `InfoResponse.build_sha`.
//!
//! We do NOT use the `vergen` crate (would pull ~ 20 transitive
//! crates).  A 10-line `std::process::Command` is plenty.

use std::process::Command;

fn main() {
    // Re-run only when the git HEAD changes.  The .git/HEAD file is
    // updated on every commit, branch switch, or rebase.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");

    let sha = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "nogit".to_string());

    // Sanity : SHA is 8 hex chars or "nogit".  Defends against a
    // hostile git binary returning trash on PATH (sec-team concern).
    let safe_sha = if sha.len() <= 16 && sha.chars().all(|c| c.is_ascii_alphanumeric()) {
        sha
    } else {
        "invalid-sha".to_string()
    };

    println!("cargo:rustc-env=GIT_SHA={safe_sha}");
}
```

---

## 9. `src/lib.rs` — full

```rust
//! `efe-meta-swarm-server` — Tonic-based gRPC servers for the EFE
//! meta-swarm coordinator.
//!
//! This crate ships **two binaries** under one Cargo crate :
//!
//!   * `controller` — implements [`MetaSwarmService`] ; called by
//!     `efe-server` (when `EFE_META_SWARM_ENABLED=true`).
//!   * `participant` — implements [`MetaParticipantService`] ; called
//!     by the controller.  Three instances run side by side in the
//!     reference Phase-2 topology (T2 single-node ; see
//!     [`EFE_A1_PHASE2_ARCHITECTURE.md`][doc]).
//!
//! [doc]: https://example.invalid/docs/EFE_A1_PHASE2_ARCHITECTURE.md
//!
//! ## Layout
//!
//!   * [`config`]       — env-var parsing (one struct per role)
//!   * [`manifest`]     — YAML manifest reader (stub in PR #6)
//!   * [`discovery`]    — `trait Discovery` + impls (Static only in PR #6)
//!   * [`observability`] — health, metrics, tracing, shutdown
//!   * [`auth`]         — `trait Authorizer` + stubs (real in PR #16)
//!   * [`controller`]   — `MetaSwarmService` impl
//!   * [`participant`]  — `MetaParticipantService` impl
//!   * [`error`]        — crate-wide error type
//!
//! ## Adding a new RPC body
//!
//! 1. Open `src/controller/service.rs` or `src/participant/service.rs`.
//! 2. Find the matching `async fn` returning
//!    `Err(Status::unimplemented(...))`.
//! 3. Replace the body.
//! 4. Add a unit test in the same file (`mod tests`).
//! 5. Add an integration test under `tests/`.
//!
//! Every business-logic PR (#8, #9, #11, #12) follows this pattern.

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]
#![warn(clippy::pedantic)]
// Tonic-generated method names are snake_case_with_long_names ; we
// keep them as-is for trait-impl correctness.
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

pub mod auth;
pub mod config;
pub mod controller;
pub mod discovery;
pub mod error;
pub mod manifest;
pub mod observability;
pub mod participant;

// Re-export the proto crate's surface under our own namespace so
// downstream callers can write `efe_meta_swarm_server::proto::Foo`
// without depending on `efe-meta-swarm-proto` directly.
pub mod proto {
    pub use efe_meta_swarm_proto::{
        controller::{MetaSwarmService, MetaSwarmServiceServer},
        meta_swarm,
        participant::{MetaParticipantService, MetaParticipantServiceServer},
        PROTO_PACKAGE, PROTO_VERSION, RPC_NAMES,
    };
}

/// The crate version, taken from `Cargo.toml`.  Used in
/// `InfoResponse.build_sha` + Prometheus `meta_swarm_build_info`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The git short SHA at compile time.  Set by `build.rs`.
/// Falls back to `"nogit"` outside a git checkout, or `"invalid-sha"`
/// if a hostile `git` is on PATH.
pub const GIT_SHA: &str = env!("GIT_SHA");

/// The build profile : "release" or "debug".
pub const PROFILE: &str = if cfg!(debug_assertions) {
    "debug"
} else {
    "release"
};

/// Crate-wide `Result` alias.
pub type Result<T> = std::result::Result<T, error::ServerError>;
```

---

## 10. `src/error.rs` — full

```rust
//! Crate-wide error type.
//!
//! Maps internal failures to gRPC `tonic::Status` codes.  Cf.
//! `efe-meta-swarm-proto`'s `MetaSwarmErrorCode` enum for the
//! application-level error codes that travel in the gRPC trailer.

use thiserror::Error;

/// Crate-wide error type.  Maps to gRPC `Status` via
/// [`ServerError::to_status`].
#[derive(Debug, Error)]
pub enum ServerError {
    /// Configuration error : invalid env var, missing file, etc.
    #[error("config error: {0}")]
    Config(String),

    /// Manifest error : parseable YAML but invalid content (dim
    /// mismatch, invalid kappa, etc.).
    #[error("manifest error: {0}")]
    Manifest(String),

    /// Discovery error : participant unreachable, DNS resolution
    /// failure.
    #[error("discovery error: {0}")]
    Discovery(String),

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Lower-level tonic transport error.
    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    /// Internal error : caller passed a malformed request that
    /// should have been caught earlier in the chain.
    #[error("internal: {0}")]
    Internal(String),

    /// Feature gated off this build.  Used by RPCs that need a
    /// feature flag set.
    #[error("feature {0} is disabled in this build")]
    FeatureDisabled(&'static str),

    /// Auth failure.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
}

impl ServerError {
    /// Map an internal error to a gRPC `Status` ready to return
    /// from an RPC handler.
    ///
    /// The mapping mirrors the table in `errors.proto` of
    /// `efe-meta-swarm-proto`.
    #[must_use]
    pub fn to_status(&self) -> tonic::Status {
        use tonic::Code;
        let code = match self {
            ServerError::Config(_) | ServerError::Manifest(_) => Code::FailedPrecondition,
            ServerError::Discovery(_) => Code::Unavailable,
            ServerError::Io(_) | ServerError::Transport(_) | ServerError::Internal(_) => {
                Code::Internal
            }
            ServerError::FeatureDisabled(_) => Code::Unimplemented,
            ServerError::Unauthorized(_) => Code::Unauthenticated,
        };
        tonic::Status::new(code, self.to_string())
    }
}

impl From<ServerError> for tonic::Status {
    fn from(e: ServerError) -> Self {
        e.to_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn config_maps_to_failed_precondition() {
        let e = ServerError::Config("bad EFE_META_SWARM_X".into());
        assert_eq!(e.to_status().code(), Code::FailedPrecondition);
    }

    #[test]
    fn discovery_maps_to_unavailable() {
        let e = ServerError::Discovery("participant-1 unreachable".into());
        assert_eq!(e.to_status().code(), Code::Unavailable);
    }

    #[test]
    fn unauthorized_maps_to_unauthenticated() {
        let e = ServerError::Unauthorized("bearer expired".into());
        assert_eq!(e.to_status().code(), Code::Unauthenticated);
    }
}
```

---

## 11. `src/config/` — full

### 11.1 `src/config/mod.rs`

```rust
//! Environment-variable parsing for both binaries.
//!
//! We follow a strict policy : every env var is read **once** at
//! startup, parsed into a strongly-typed struct, and then
//! immutable for the life of the process.
//!
//! See [`EFE_A1_PHASE2_ARCHITECTURE.md`][doc] § 7 for the full
//! catalogue of 18 env vars.
//!
//! [doc]: https://example.invalid/docs/EFE_A1_PHASE2_ARCHITECTURE.md

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use crate::error::ServerError;

pub mod controller;
pub mod participant;

/// Common helper : read an env var, lower-case the value before
/// parsing.  Returns the default if the var is unset.
pub(crate) fn env_or<T: FromStr>(name: &str, default: T) -> std::result::Result<T, ServerError>
where
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(s) => s.parse().map_err(|e| {
            ServerError::Config(format!("env var {name} = {s!r} : {e}"))
        }),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(e) => Err(ServerError::Config(format!("env var {name} : {e}"))),
    }
}

/// Common helper : read an env var as a SocketAddr.
pub(crate) fn env_socket_addr(
    name: &str,
    default: &str,
) -> std::result::Result<SocketAddr, ServerError> {
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse()
        .map_err(|e| ServerError::Config(format!("env var {name} = {raw!r} : {e}")))
}

/// Common helper : read an env var as a Duration in seconds.
pub(crate) fn env_duration_s(
    name: &str,
    default_s: u64,
) -> std::result::Result<Duration, ServerError> {
    let secs: u64 = env_or(name, default_s)?;
    Ok(Duration::from_secs(secs))
}

/// Common helper : read an env var as `Option<PathBuf>`.
pub(crate) fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Common helper : read an env var as `bool`.  Accepts the usual
/// strings : "true" / "false" / "1" / "0" / "yes" / "no".
pub(crate) fn env_bool(
    name: &str,
    default: bool,
) -> std::result::Result<bool, ServerError> {
    match std::env::var(name).ok().as_deref() {
        None => Ok(default),
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            other => Err(ServerError::Config(format!(
                "env var {name} = {other!r} : expected bool",
            ))),
        },
    }
}
```

### 11.2 `src/config/controller.rs`

```rust
//! Controller-side configuration.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::error::ServerError;

use super::{env_bool, env_duration_s, env_or, env_path, env_socket_addr};

/// Strongly-typed controller configuration.  Built once at startup
/// from env vars ; immutable thereafter.
#[derive(Debug, Clone)]
pub struct ControllerConfig {
    /// gRPC bind address.  Default `0.0.0.0:50080`.
    pub grpc_bind: SocketAddr,

    /// Prometheus exporter bind address.  Default `0.0.0.0:9091`.
    pub metrics_bind: SocketAddr,

    /// Discovery mode : `static-localhost` (default) or `k8s-dns`
    /// (Phase 3+).
    pub discovery: DiscoveryMode,

    /// Static participants (only used when discovery=static-localhost).
    ///
    /// Format on the wire :  `id=host:port,id=host:port,…`.
    ///
    /// Example :
    ///   ```text
    ///   supplier-scoring=localhost:50091,contract-risk=localhost:50092
    ///   ```
    pub static_participants: Vec<StaticParticipant>,

    /// Path to the on-disk manifest.  Default
    /// `/etc/efe-meta-swarm/manifest.yaml`.  PR #6 reads but does
    /// not validate ; PR #8 wires the validation.
    pub manifest_path: PathBuf,

    /// Max concurrent in-flight `Iterate` calls.  Default 10.
    pub max_inflight: usize,

    /// Default `max_wall_ms` when the client omits it.  Default 15000.
    pub default_max_wall_ms: u32,

    /// Tokio worker thread count.  Default 2.
    pub worker_threads: usize,

    /// Whether tonic-reflection is enabled.  Default false ; required
    /// `--features reflection`.
    pub reflection_enabled: bool,

    /// Whether gRPC TLS is required.  Default false (Phase 2 ships
    /// plaintext ; mTLS arrives in PR #16).
    pub tls_required: bool,

    /// Optional path to the TLS cert PEM.  Required if `tls_required`.
    pub tls_cert_path: Option<PathBuf>,

    /// Optional path to the TLS key PEM.  Required if `tls_required`.
    pub tls_key_path: Option<PathBuf>,

    /// Optional path to the TLS CA PEM (for mTLS client verification).
    pub tls_ca_path: Option<PathBuf>,

    /// Drain timeout on SIGTERM.  Default 30 s.
    pub drain_timeout: Duration,
}

/// Discovery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryMode {
    StaticLocalhost,
    K8sDns,
}

impl std::str::FromStr for DiscoveryMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "static-localhost" => Ok(Self::StaticLocalhost),
            "k8s-dns" => Ok(Self::K8sDns),
            other => Err(format!("invalid discovery mode: {other!r}")),
        }
    }
}

/// One static participant entry parsed from
/// `EFE_META_SWARM_PARTICIPANTS`.
#[derive(Debug, Clone)]
pub struct StaticParticipant {
    pub id: String,
    pub addr: String, // `host:port`
}

impl ControllerConfig {
    /// Build from the environment.  Returns an error if any required
    /// var is missing or malformed.
    pub fn from_env() -> Result<Self, ServerError> {
        let grpc_bind = env_socket_addr("EFE_META_SWARM_CONTROLLER_ADDR", "0.0.0.0:50080")?;
        let metrics_bind =
            env_socket_addr("EFE_META_SWARM_CONTROLLER_METRICS", "0.0.0.0:9091")?;

        let discovery: DiscoveryMode = env_or("EFE_META_SWARM_DISCOVERY", DiscoveryMode::StaticLocalhost)?;

        let static_participants = match std::env::var("EFE_META_SWARM_PARTICIPANTS") {
            Ok(raw) => parse_participants(&raw)?,
            Err(_) => Vec::new(),
        };

        let manifest_path = env_path("EFE_META_SWARM_MANIFEST")
            .unwrap_or_else(|| PathBuf::from("/etc/efe-meta-swarm/manifest.yaml"));

        let max_inflight = env_or("EFE_META_SWARM_MAX_INFLIGHT", 10_usize)?;
        let default_max_wall_ms = env_or("EFE_META_SWARM_DEFAULT_MAX_WALL_MS", 15_000_u32)?;
        let worker_threads = env_or("EFE_META_SWARM_WORKER_THREADS", 2_usize)?;
        let reflection_enabled = env_bool("EFE_META_SWARM_REFLECTION", false)?;

        let tls_required = env_bool("EFE_META_SWARM_TLS_REQUIRED", false)?;
        let tls_cert_path = env_path("EFE_META_SWARM_TLS_CERT");
        let tls_key_path = env_path("EFE_META_SWARM_TLS_KEY");
        let tls_ca_path = env_path("EFE_META_SWARM_TLS_CA");

        if tls_required && (tls_cert_path.is_none() || tls_key_path.is_none()) {
            return Err(ServerError::Config(
                "EFE_META_SWARM_TLS_REQUIRED=true but \
                 EFE_META_SWARM_TLS_CERT or EFE_META_SWARM_TLS_KEY is unset"
                    .into(),
            ));
        }

        let drain_timeout = env_duration_s("EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S", 30)?;

        Ok(Self {
            grpc_bind,
            metrics_bind,
            discovery,
            static_participants,
            manifest_path,
            max_inflight,
            default_max_wall_ms,
            worker_threads,
            reflection_enabled,
            tls_required,
            tls_cert_path,
            tls_key_path,
            tls_ca_path,
            drain_timeout,
        })
    }
}

fn parse_participants(raw: &str) -> Result<Vec<StaticParticipant>, ServerError> {
    let mut out = Vec::new();
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let (id, addr) = entry.split_once('=').ok_or_else(|| {
            ServerError::Config(format!(
                "EFE_META_SWARM_PARTICIPANTS : malformed entry {entry!r} (expected `id=host:port`)",
            ))
        })?;
        if id.is_empty() || addr.is_empty() {
            return Err(ServerError::Config(format!(
                "EFE_META_SWARM_PARTICIPANTS : empty id or addr in {entry!r}",
            )));
        }
        out.push(StaticParticipant {
            id: id.trim().to_string(),
            addr: addr.trim().to_string(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_participants() {
        let raw = "supplier-scoring=localhost:50091,contract-risk=localhost:50092,spend-categorization=localhost:50093";
        let out = parse_participants(raw).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, "supplier-scoring");
        assert_eq!(out[0].addr, "localhost:50091");
    }

    #[test]
    fn rejects_malformed_entry() {
        let raw = "supplier-scoring";
        let err = parse_participants(raw).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn ignores_trailing_comma() {
        let raw = "supplier=localhost:1,";
        let out = parse_participants(raw).unwrap();
        assert_eq!(out.len(), 1);
    }
}
```

### 11.3 `src/config/participant.rs`

```rust
//! Participant-side configuration.

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::ServerError;

use super::{env_bool, env_or, env_path, env_socket_addr};

/// Strongly-typed participant configuration.  Built once at startup
/// from env vars ; immutable thereafter.
#[derive(Debug, Clone)]
pub struct ParticipantConfig {
    /// Participant id.  REQUIRED.  Matches the manifest's
    /// participant id ; controller fails if a participant returns a
    /// mismatching id in `Capabilities`.
    pub id: String,

    /// gRPC bind address.  Default `0.0.0.0:50090`.
    pub grpc_bind: SocketAddr,

    /// Prometheus exporter bind address.  Default `0.0.0.0:9092`.
    pub metrics_bind: SocketAddr,

    /// Tokio worker thread count.  Default 1.  Participants are
    /// less I/O-bound than the controller.
    pub worker_threads: usize,

    /// Whether gRPC TLS is required.  Default false.
    pub tls_required: bool,
    pub tls_cert_path: Option<PathBuf>,
    pub tls_key_path: Option<PathBuf>,
    pub tls_ca_path: Option<PathBuf>,
}

impl ParticipantConfig {
    /// Build from the environment.
    pub fn from_env() -> Result<Self, ServerError> {
        let id = std::env::var("EFE_META_SWARM_PARTICIPANT_ID").map_err(|_| {
            ServerError::Config(
                "EFE_META_SWARM_PARTICIPANT_ID is REQUIRED for the participant binary".into(),
            )
        })?;
        if id.is_empty() {
            return Err(ServerError::Config(
                "EFE_META_SWARM_PARTICIPANT_ID must be non-empty".into(),
            ));
        }

        let grpc_bind =
            env_socket_addr("EFE_META_SWARM_PARTICIPANT_ADDR", "0.0.0.0:50090")?;
        let metrics_bind =
            env_socket_addr("EFE_META_SWARM_PARTICIPANT_METRICS", "0.0.0.0:9092")?;
        let worker_threads = env_or("EFE_META_SWARM_WORKER_THREADS", 1_usize)?;

        let tls_required = env_bool("EFE_META_SWARM_TLS_REQUIRED", false)?;
        let tls_cert_path = env_path("EFE_META_SWARM_TLS_CERT");
        let tls_key_path = env_path("EFE_META_SWARM_TLS_KEY");
        let tls_ca_path = env_path("EFE_META_SWARM_TLS_CA");

        if tls_required && (tls_cert_path.is_none() || tls_key_path.is_none()) {
            return Err(ServerError::Config(
                "EFE_META_SWARM_TLS_REQUIRED=true but cert/key unset".into(),
            ));
        }

        Ok(Self {
            id,
            grpc_bind,
            metrics_bind,
            worker_threads,
            tls_required,
            tls_cert_path,
            tls_key_path,
            tls_ca_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_id() {
        std::env::remove_var("EFE_META_SWARM_PARTICIPANT_ID");
        let err = ParticipantConfig::from_env().unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }
}
```

---

## 12. `src/manifest/` — full

### 12.1 `src/manifest/mod.rs`

```rust
//! YAML manifest reader.
//!
//! PR #6 ships a STUB : reads the file, parses it, but does not
//! validate semantically.  PR #8 will add validation against the
//! controller's running state + dimension checks.

mod schema;

pub use schema::{Manifest, ParticipantManifest, SafetyManifest};

use std::path::Path;

use crate::error::ServerError;

/// Read + parse a manifest from disk.
///
/// PR #6 : returns the parsed value if the file is well-formed YAML.
/// PR #8 : will add cross-field validation.
pub fn load(path: impl AsRef<Path>) -> Result<Manifest, ServerError> {
    let p = path.as_ref();
    let raw = std::fs::read_to_string(p).map_err(ServerError::Io)?;
    let m: Manifest = serde_yaml::from_str(&raw).map_err(|e| {
        ServerError::Manifest(format!("manifest {} : {e}", p.display()))
    })?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_minimal_manifest() {
        let body = r#"
version: v0.2.0
meta:
  dim: 256
  safety:
    kind: box
    radius: 10.0
participants:
  - id: supplier-scoring
    dim: 384
    kappa: 0.6
    seed: 11
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        let m = load(f.path()).unwrap();
        assert_eq!(m.meta.dim, 256);
        assert_eq!(m.participants.len(), 1);
    }
}
```

### 12.2 `src/manifest/schema.rs`

```rust
//! YAML schema types.

use serde::{Deserialize, Serialize};

/// Top-level manifest.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub version: String,
    pub meta: MetaSpaceManifest,
    #[serde(default)]
    pub participants: Vec<ParticipantManifest>,
    #[serde(default)]
    pub iteration: IterationManifest,
}

/// Meta-space config.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetaSpaceManifest {
    pub dim: u32,
    pub safety: SafetyManifest,
}

/// Safety geometry.  Tagged enum on the YAML side via the `kind`
/// field ; matches the proto `MetaSpaceConfig.safety_geometry` oneof.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SafetyManifest {
    Box { radius: f32 },
    Poly { dim_a: u32, a_flat: Vec<f32>, b: Vec<f32> },
    Hull { version: String, serialized_model_path: String },
}

/// Participant declaration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParticipantManifest {
    pub id: String,
    pub dim: u32,
    pub kappa: f32,
    pub seed: u64,
    #[serde(default = "default_blas")]
    pub blas_backend: String,
}

fn default_blas() -> String {
    "none".into()
}

/// Iteration knobs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct IterationManifest {
    pub max_iters: u32,
    pub tolerance: f64,
    pub alpha_default: f64,
    pub coupling_gamma: f64,
    pub deterministic: bool,
    pub max_wall_ms: u32,
}

impl Default for IterationManifest {
    fn default() -> Self {
        Self {
            max_iters: 100,
            tolerance: 1.0e-4,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 15_000,
        }
    }
}
```

---

## 13. `src/discovery/` — full

### 13.1 `src/discovery/mod.rs`

```rust
//! Participant discovery.
//!
//! A `trait Discovery` with two impls :
//!   * [`StaticDiscovery`] — Phase-2 default ; reads from
//!     `EFE_META_SWARM_PARTICIPANTS`.
//!   * `K8sDnsDiscovery` — Phase 3 ; uses K8s DNS to resolve a
//!     headless Service to a list of pod IPs.
//!
//! PR #6 ships only `StaticDiscovery`.

mod static_impl;

pub use static_impl::StaticDiscovery;

use async_trait::async_trait;

use crate::config::controller::StaticParticipant;

/// A list of participants the controller should connect to.
#[derive(Debug, Clone)]
pub struct DiscoveredParticipant {
    pub id: String,
    pub endpoint: String, // gRPC `host:port`, no scheme
}

/// Pluggable participant discovery.
#[async_trait]
pub trait Discovery: Send + Sync + 'static {
    /// Return the current list of participants.  Called periodically
    /// (every 10 s) by the controller's refresh loop.
    async fn discover(&self) -> Result<Vec<DiscoveredParticipant>, crate::error::ServerError>;
}

impl From<&StaticParticipant> for DiscoveredParticipant {
    fn from(p: &StaticParticipant) -> Self {
        Self {
            id: p.id.clone(),
            endpoint: p.addr.clone(),
        }
    }
}
```

### 13.2 `src/discovery/static_impl.rs`

```rust
//! Static discovery — Phase-2 default.

use async_trait::async_trait;

use super::{Discovery, DiscoveredParticipant};
use crate::config::controller::StaticParticipant;
use crate::error::ServerError;

/// Discovery from an in-memory list (populated at startup from
/// `EFE_META_SWARM_PARTICIPANTS`).
#[derive(Debug, Clone)]
pub struct StaticDiscovery {
    participants: Vec<DiscoveredParticipant>,
}

impl StaticDiscovery {
    pub fn from_static(list: &[StaticParticipant]) -> Self {
        Self {
            participants: list.iter().map(DiscoveredParticipant::from).collect(),
        }
    }
}

#[async_trait]
impl Discovery for StaticDiscovery {
    async fn discover(&self) -> Result<Vec<DiscoveredParticipant>, ServerError> {
        Ok(self.participants.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_static_list() {
        let list = vec![StaticParticipant {
            id: "supplier".into(),
            addr: "localhost:50091".into(),
        }];
        let d = StaticDiscovery::from_static(&list);
        let out = d.discover().await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "supplier");
    }
}
```

---

## 14. `src/observability/` — full

### 14.1 `src/observability/mod.rs`

```rust
//! Cross-cutting observability wiring.

pub mod health;
pub mod metrics;
pub mod shutdown;
pub mod tracing;
```

### 14.2 `src/observability/tracing.rs`

```rust
//! Tracing-subscriber wiring.  JSON output by default ; pretty
//! output when stderr is a TTY (dev ergonomics).
//!
//! PR #6 ships subscriber-only.  PR #18 adds the OTel exporter
//! that ships spans to the collector.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialise the global tracing subscriber.  Idempotent : calling
/// twice is a no-op.
pub fn init() {
    let filter = EnvFilter::try_from_env("EFE_META_SWARM_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let use_json = !atty::is(atty::Stream::Stderr);

    let layer = if use_json {
        fmt::layer().json().boxed()
    } else {
        fmt::layer().pretty().boxed()
    };

    // `try_init` is idempotent ; ignore the error of "already
    // initialised".
    let _ = tracing_subscriber::registry().with(filter).with(layer).try_init();
}
```

Wait — `atty` is not in `[workspace.dependencies]`.  Alternative :
`std::io::IsTerminal` (stable in rustc 1.70+, available in 1.88).

Rewrite :

```rust
use std::io::IsTerminal;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init() {
    let filter = EnvFilter::try_from_env("EFE_META_SWARM_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let use_json = !std::io::stderr().is_terminal();

    if use_json {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json())
            .try_init();
    } else {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().pretty())
            .try_init();
    }
}
```

### 14.3 `src/observability/health.rs`

```rust
//! Health probe state.  Shared between the gRPC `Health` RPC and
//! the K8s liveness/readiness probes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use efe_meta_swarm_proto::meta_swarm::health_response::Status as HealthStatus;
use efe_meta_swarm_proto::meta_swarm::HealthResponse;

/// Shared health state.  Cloned freely (Arc inside).
#[derive(Debug, Clone, Default)]
pub struct HealthState {
    inner: Arc<HealthInner>,
}

#[derive(Debug, Default)]
struct HealthInner {
    status: AtomicU32,    // HealthStatus int repr
    inflight: AtomicU32,
    reload_gen: AtomicU32,
}

impl HealthState {
    /// New, defaulting to SERVING.  Use this on startup.
    #[must_use]
    pub fn serving() -> Self {
        let s = Self::default();
        s.inner
            .status
            .store(HealthStatus::Serving as u32, Ordering::Relaxed);
        s
    }

    pub fn snapshot(&self) -> HealthResponse {
        HealthResponse {
            status: self.inner.status.load(Ordering::Relaxed) as i32,
            inflight: self.inner.inflight.load(Ordering::Relaxed),
            reload_gen: self.inner.reload_gen.load(Ordering::Relaxed),
        }
    }

    pub fn set_not_serving(&self) {
        self.inner
            .status
            .store(HealthStatus::NotServing as u32, Ordering::Relaxed);
    }

    pub fn set_serving(&self) {
        self.inner
            .status
            .store(HealthStatus::Serving as u32, Ordering::Relaxed);
    }

    pub fn set_reloading(&self) {
        self.inner
            .status
            .store(HealthStatus::Reloading as u32, Ordering::Relaxed);
    }

    /// Increment the in-flight counter.  Returns the new value.
    pub fn inflight_inc(&self) -> u32 {
        self.inner.inflight.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Decrement the in-flight counter.  Returns the new value.
    pub fn inflight_dec(&self) -> u32 {
        self.inner.inflight.fetch_sub(1, Ordering::Relaxed) - 1
    }

    pub fn bump_reload_gen(&self) -> u32 {
        self.inner.reload_gen.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_serving() {
        let s = HealthState::serving();
        let snap = s.snapshot();
        assert_eq!(snap.status, HealthStatus::Serving as i32);
        assert_eq!(snap.inflight, 0);
        assert_eq!(snap.reload_gen, 0);
    }

    #[test]
    fn inflight_counter() {
        let s = HealthState::serving();
        assert_eq!(s.inflight_inc(), 1);
        assert_eq!(s.inflight_inc(), 2);
        assert_eq!(s.inflight_dec(), 1);
    }
}
```

### 14.4 `src/observability/metrics.rs`

```rust
//! Prometheus exporter — stub.  PR #6 ships an EMPTY registry +
//! a `/metrics` HTTP endpoint.  PR #17 registers the 14 metrics.

use std::net::SocketAddr;
use std::sync::Arc;

use prometheus::{Encoder, Registry, TextEncoder};
use tracing::{error, info, warn};

/// Shared metrics registry.  Cloned by every handler that wants to
/// record.
#[derive(Debug, Clone)]
pub struct MetricsRegistry {
    inner: Arc<Registry>,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(Registry::new()),
        }
    }
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn registry(&self) -> &Registry {
        &self.inner
    }

    /// Render the current metrics in the Prometheus text format.
    pub fn render(&self) -> Vec<u8> {
        let encoder = TextEncoder::new();
        let metric_families = self.inner.gather();
        let mut buf = Vec::new();
        if let Err(e) = encoder.encode(&metric_families, &mut buf) {
            error!(error = %e, "metrics encode failed");
        }
        buf
    }
}

/// Spawn the Prometheus HTTP exporter on `addr`.  Returns a shutdown
/// signal sender ; dropping it cancels the server.
///
/// The exporter only exposes `GET /metrics` ; everything else returns
/// 404.
pub async fn spawn_exporter(
    addr: SocketAddr,
    registry: MetricsRegistry,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), crate::error::ServerError> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(addr).await?;
    info!(addr = %addr, "metrics exporter listening");

    loop {
        let (stream, _) = tokio::select! {
            res = listener.accept() => res?,
            _ = shutdown.changed() => {
                info!("metrics exporter shutting down");
                return Ok(());
            }
        };

        let io = TokioIo::new(stream);
        let registry = registry.clone();

        tokio::spawn(async move {
            let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                let registry = registry.clone();
                async move {
                    if req.method() == Method::GET && req.uri().path() == "/metrics" {
                        let body = registry.render();
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "text/plain; version=0.0.4")
                            .body(Full::new(Bytes::from(body)))
                    } else {
                        Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Full::new(Bytes::from_static(b"not found")))
                    }
                }
            });

            if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                warn!(error = %e, "metrics conn error");
            }
        });
    }
}
```

### 14.5 `src/observability/shutdown.rs`

```rust
//! Graceful shutdown signal handling.
//!
//! PR #6 handles SIGTERM (graceful drain).  PR #12 adds SIGHUP
//! (reload).  PR #18 adds SIGUSR1 (dump debug to stderr).

use tokio::signal::unix::{signal, SignalKind};

use tracing::info;

/// Wait for SIGTERM or Ctrl+C, whichever comes first.
///
/// Resolves with the kind of signal received (for logging).
pub async fn await_termination() -> &'static str {
    let mut sigterm =
        signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut sigint =
        signal(SignalKind::interrupt()).expect("install SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            info!("received SIGTERM");
            "SIGTERM"
        }
        _ = sigint.recv() => {
            info!("received SIGINT");
            "SIGINT"
        }
    }
}
```

---

## 15. `src/auth/` — full

### 15.1 `src/auth/mod.rs`

```rust
//! Authentication + authorisation traits.  PR #6 stubs ; PR #16 wires
//! the real impls.

pub mod bearer;
pub mod mtls;

use async_trait::async_trait;

use crate::error::ServerError;

/// Outcome of an auth check.
#[derive(Debug, Clone)]
pub enum AuthDecision {
    /// Caller is authorized for the requested operation.
    Allow,
    /// Caller is rejected with this reason.
    Deny(String),
}

/// Caller identity, opaque to the service.  In PR #16 this carries
/// the SAN from the leaf cert + the JWT claims.
#[derive(Debug, Clone, Default)]
pub struct CallerIdentity {
    pub san: Option<String>,
    pub scopes: Vec<String>,
}

/// Auth-z policy.
#[async_trait]
pub trait Authorizer: Send + Sync + 'static {
    /// Check whether `caller` is allowed to invoke `rpc_name`.
    async fn authorize(&self, caller: &CallerIdentity, rpc_name: &str) -> AuthDecision;
}

/// Stub authorizer : allows everything.  ONLY for dev / PR #6
/// scaffold.  Replaced by `MtlsAuthorizer` in PR #16.
#[derive(Debug, Default, Clone)]
pub struct AllowAll;

#[async_trait]
impl Authorizer for AllowAll {
    async fn authorize(&self, _: &CallerIdentity, _: &str) -> AuthDecision {
        AuthDecision::Allow
    }
}
```

### 15.2 `src/auth/mtls.rs` + `src/auth/bearer.rs`

```rust
//! mTLS authorizer — STUB in PR #6.  Real impl in PR #16.

use async_trait::async_trait;

use super::{AuthDecision, Authorizer, CallerIdentity};

#[derive(Debug, Clone, Default)]
pub struct MtlsAuthorizer;

#[async_trait]
impl Authorizer for MtlsAuthorizer {
    async fn authorize(&self, _: &CallerIdentity, _: &str) -> AuthDecision {
        AuthDecision::Deny("mtls authorizer not yet wired (PR #16)".into())
    }
}
```

```rust
//! Bearer-token authorizer — STUB in PR #6.  Real impl in PR #16.

use async_trait::async_trait;

use super::{AuthDecision, Authorizer, CallerIdentity};

#[derive(Debug, Clone, Default)]
pub struct BearerAuthorizer;

#[async_trait]
impl Authorizer for BearerAuthorizer {
    async fn authorize(&self, _: &CallerIdentity, _: &str) -> AuthDecision {
        AuthDecision::Deny("bearer authorizer not yet wired (PR #16)".into())
    }
}
```

---

## 16. `src/controller/` — full

### 16.1 `src/controller/mod.rs`

```rust
//! Controller — implements `MetaSwarmService`.

mod service;
mod state;

pub use service::ControllerService;
pub use state::ControllerState;
```

### 16.2 `src/controller/state.rs`

```rust
//! Shared controller state.

use std::sync::Arc;
use std::time::Instant;

use efe_meta_swarm_proto::meta_swarm::{InfoResponse, ParticipantDescriptor};

use crate::config::controller::ControllerConfig;
use crate::observability::health::HealthState;
use crate::observability::metrics::MetricsRegistry;

/// Shared controller state.  Cheap to clone (Arcs inside).
#[derive(Clone)]
pub struct ControllerState {
    pub config: Arc<ControllerConfig>,
    pub health: HealthState,
    pub metrics: MetricsRegistry,
    pub start_time: Instant,
}

impl std::fmt::Debug for ControllerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControllerState")
            .field("config", &self.config)
            .field("uptime_s", &self.start_time.elapsed().as_secs())
            .finish_non_exhaustive()
    }
}

impl ControllerState {
    pub fn new(config: ControllerConfig) -> Self {
        Self {
            config: Arc::new(config),
            health: HealthState::serving(),
            metrics: MetricsRegistry::new(),
            start_time: Instant::now(),
        }
    }

    /// Pre-built `InfoResponse` populated with everything the
    /// controller knows at startup time (build sha + uptime + an
    /// empty list of participants — PR #8 fills it in).
    pub fn info_snapshot(&self) -> InfoResponse {
        InfoResponse {
            build_sha: crate::GIT_SHA.into(),
            proto_version: crate::proto::PROTO_VERSION.into(),
            configured_participants: Vec::<ParticipantDescriptor>::new(),
            leader_pod_name: hostname::get()
                .ok()
                .and_then(|s| s.into_string().ok())
                .unwrap_or_default(),
            uptime_s: self.start_time.elapsed().as_secs(),
            last_reload_gen: 0,
        }
    }
}
```

### 16.3 `src/controller/service.rs`

```rust
//! `MetaSwarmService` impl.  PR #6 returns UNIMPLEMENTED for every
//! RPC except `Health` and `Info`.

use efe_meta_swarm_proto::meta_swarm::{
    HealthRequest, HealthResponse, InfoRequest, InfoResponse, IterateChunk,
    IterateRequest, IterateResponse, ReloadConfigRequest, ReloadConfigResponse,
    ResetRequest, ResetResponse, StepOnceRequest, StepOnceResponse,
};
use tonic::{Request, Response, Status};
use tracing::{instrument, warn};

use super::state::ControllerState;
use crate::proto::MetaSwarmService;

/// Concrete `MetaSwarmService` implementation.  Holds the shared
/// controller state.
#[derive(Debug, Clone)]
pub struct ControllerService {
    pub state: ControllerState,
}

impl ControllerService {
    pub fn new(state: ControllerState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl MetaSwarmService for ControllerService {
    type IterateStreamStream = futures::stream::Empty<Result<IterateChunk, Status>>;

    #[instrument(skip(self, _req))]
    async fn iterate(
        &self,
        _req: Request<IterateRequest>,
    ) -> Result<Response<IterateResponse>, Status> {
        warn!("Iterate called — wired in PR #8");
        Err(Status::unimplemented(
            "MetaSwarmService::Iterate — wired in PR #8",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn iterate_stream(
        &self,
        _req: Request<IterateRequest>,
    ) -> Result<Response<Self::IterateStreamStream>, Status> {
        Err(Status::unimplemented(
            "MetaSwarmService::IterateStream — wired in PR #21",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn step_once(
        &self,
        _req: Request<StepOnceRequest>,
    ) -> Result<Response<StepOnceResponse>, Status> {
        Err(Status::unimplemented(
            "MetaSwarmService::StepOnce — wired in PR #8",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn reset(
        &self,
        _req: Request<ResetRequest>,
    ) -> Result<Response<ResetResponse>, Status> {
        Err(Status::unimplemented(
            "MetaSwarmService::Reset — wired in PR #8",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn reload_config(
        &self,
        _req: Request<ReloadConfigRequest>,
    ) -> Result<Response<ReloadConfigResponse>, Status> {
        Err(Status::unimplemented(
            "MetaSwarmService::ReloadConfig — wired in PR #12",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn health(
        &self,
        _req: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(self.state.health.snapshot()))
    }

    #[instrument(skip(self, _req))]
    async fn info(
        &self,
        _req: Request<InfoRequest>,
    ) -> Result<Response<InfoResponse>, Status> {
        Ok(Response::new(self.state.info_snapshot()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::controller::ControllerConfig;

    fn test_state() -> ControllerState {
        // Build a default config without env vars.
        let config = ControllerConfig {
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            discovery: crate::config::controller::DiscoveryMode::StaticLocalhost,
            static_participants: vec![],
            manifest_path: "/dev/null".into(),
            max_inflight: 10,
            default_max_wall_ms: 15_000,
            worker_threads: 1,
            reflection_enabled: false,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            drain_timeout: std::time::Duration::from_secs(30),
        };
        ControllerState::new(config)
    }

    #[tokio::test]
    async fn health_returns_serving() {
        let svc = ControllerService::new(test_state());
        let resp = svc.health(Request::new(HealthRequest {})).await.unwrap();
        let h = resp.into_inner();
        assert_eq!(h.status, 1); // STATUS_SERVING
    }

    #[tokio::test]
    async fn info_includes_proto_version() {
        let svc = ControllerService::new(test_state());
        let resp = svc.info(Request::new(InfoRequest {})).await.unwrap();
        let i = resp.into_inner();
        assert_eq!(i.proto_version, "v1");
    }

    #[tokio::test]
    async fn iterate_returns_unimplemented() {
        let svc = ControllerService::new(test_state());
        let err = svc.iterate(Request::new(IterateRequest::default())).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        assert!(err.message().contains("PR #8"));
    }
}
```

---

## 17. `src/participant/` — full

Symmetric to `controller/` — same pattern, 3 files.

### 17.1 `src/participant/mod.rs`

```rust
mod service;
mod state;

pub use service::ParticipantService;
pub use state::ParticipantState;
```

### 17.2 `src/participant/state.rs`

```rust
use std::sync::Arc;
use std::time::Instant;

use efe_meta_swarm_proto::meta_swarm::ParticipantDescriptor;

use crate::config::participant::ParticipantConfig;
use crate::observability::health::HealthState;
use crate::observability::metrics::MetricsRegistry;

#[derive(Clone)]
pub struct ParticipantState {
    pub config: Arc<ParticipantConfig>,
    pub health: HealthState,
    pub metrics: MetricsRegistry,
    pub start_time: Instant,
}

impl std::fmt::Debug for ParticipantState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParticipantState")
            .field("config", &self.config)
            .field("uptime_s", &self.start_time.elapsed().as_secs())
            .finish_non_exhaustive()
    }
}

impl ParticipantState {
    pub fn new(config: ParticipantConfig) -> Self {
        Self {
            config: Arc::new(config),
            health: HealthState::serving(),
            metrics: MetricsRegistry::new(),
            start_time: Instant::now(),
        }
    }

    /// Pre-built descriptor.  PR #6 ships placeholders ; PR #9 fills
    /// in real dim/kappa/seed from the participant's `MockParticipant`.
    pub fn descriptor(&self) -> ParticipantDescriptor {
        ParticipantDescriptor {
            id: self.config.id.clone(),
            dim: 0,        // PR #9 wires
            kappa: 0.0,    // PR #9 wires
            seed: 0,       // PR #9 wires
            build_sha: crate::GIT_SHA.into(),
            blas_backend: "none".into(),
            features: vec!["determinism".into()],
        }
    }
}
```

### 17.3 `src/participant/service.rs`

```rust
use efe_meta_swarm_proto::meta_swarm::{
    CapabilitiesRequest, CapabilitiesResponse, HealthRequest, HealthResponse,
    ResetParticipantRequest, ResetParticipantResponse, StateRequest,
    StateResponse, StepRequest, StepResponse,
};
use tonic::{Request, Response, Status};
use tracing::{instrument, warn};

use super::state::ParticipantState;
use crate::proto::MetaParticipantService;

#[derive(Debug, Clone)]
pub struct ParticipantService {
    pub state: ParticipantState,
}

impl ParticipantService {
    pub fn new(state: ParticipantState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl MetaParticipantService for ParticipantService {
    type StepStreamStream = futures::stream::Empty<Result<StepResponse, Status>>;

    #[instrument(skip(self, _req))]
    async fn step(
        &self,
        _req: Request<StepRequest>,
    ) -> Result<Response<StepResponse>, Status> {
        warn!("Step called — wired in PR #9");
        Err(Status::unimplemented(
            "MetaParticipantService::Step — wired in PR #9",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn reset(
        &self,
        _req: Request<ResetParticipantRequest>,
    ) -> Result<Response<ResetParticipantResponse>, Status> {
        Err(Status::unimplemented(
            "MetaParticipantService::Reset — wired in PR #9",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn state(
        &self,
        _req: Request<StateRequest>,
    ) -> Result<Response<StateResponse>, Status> {
        Err(Status::unimplemented(
            "MetaParticipantService::State — wired in PR #9",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn step_stream(
        &self,
        _req: Request<tonic::Streaming<StepRequest>>,
    ) -> Result<Response<Self::StepStreamStream>, Status> {
        Err(Status::unimplemented(
            "MetaParticipantService::StepStream — wired in PR #9",
        ))
    }

    #[instrument(skip(self, _req))]
    async fn capabilities(
        &self,
        _req: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        // The descriptor is cheap to produce ; we return it even
        // though PR #9 will fill in dim/kappa/seed properly.
        Ok(Response::new(CapabilitiesResponse {
            descriptor: Some(self.state.descriptor()),
        }))
    }

    #[instrument(skip(self, _req))]
    async fn health(
        &self,
        _req: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(self.state.health.snapshot()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::participant::ParticipantConfig;

    fn test_state() -> ParticipantState {
        let config = ParticipantConfig {
            id: "test-participant".into(),
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            worker_threads: 1,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
        };
        ParticipantState::new(config)
    }

    #[tokio::test]
    async fn health_returns_serving() {
        let svc = ParticipantService::new(test_state());
        let resp = svc.health(Request::new(HealthRequest {})).await.unwrap();
        assert_eq!(resp.into_inner().status, 1);
    }

    #[tokio::test]
    async fn capabilities_returns_descriptor() {
        let svc = ParticipantService::new(test_state());
        let resp = svc.capabilities(Request::new(CapabilitiesRequest {})).await.unwrap();
        let desc = resp.into_inner().descriptor.expect("descriptor present");
        assert_eq!(desc.id, "test-participant");
        assert!(desc.features.iter().any(|f| f == "determinism"));
    }

    #[tokio::test]
    async fn step_returns_unimplemented() {
        let svc = ParticipantService::new(test_state());
        let err = svc.step(Request::new(StepRequest::default())).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        assert!(err.message().contains("PR #9"));
    }
}
```

---

## 18. `src/bin/controller.rs` — full

```rust
//! Controller binary entry-point.
//!
//! Runs a Tonic gRPC server on `EFE_META_SWARM_CONTROLLER_ADDR` and a
//! Prometheus exporter on `EFE_META_SWARM_CONTROLLER_METRICS`.  Exits
//! cleanly on SIGTERM / SIGINT.

use std::process::ExitCode;

use efe_meta_swarm_proto::controller::MetaSwarmServiceServer;
use efe_meta_swarm_server::{
    config::controller::ControllerConfig,
    controller::{ControllerService, ControllerState},
    observability::{metrics, shutdown, tracing as obs_tracing},
};

use ::tracing::{error, info};
use tokio::sync::watch;
use tonic::transport::Server;

fn main() -> ExitCode {
    obs_tracing::init();

    let config = match ControllerConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "controller config error");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.worker_threads)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!(error = %e, "tokio runtime build error");
            return ExitCode::from(2);
        }
    };

    let result = runtime.block_on(async_main(config));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %e, "controller exited with error");
            ExitCode::from(1)
        }
    }
}

async fn async_main(
    config: ControllerConfig,
) -> Result<(), efe_meta_swarm_server::error::ServerError> {
    info!(
        version = efe_meta_swarm_server::VERSION,
        git_sha = efe_meta_swarm_server::GIT_SHA,
        profile = efe_meta_swarm_server::PROFILE,
        grpc_bind = %config.grpc_bind,
        metrics_bind = %config.metrics_bind,
        worker_threads = config.worker_threads,
        discovery = ?config.discovery,
        "controller starting",
    );

    let state = ControllerState::new(config.clone());
    let svc = ControllerService::new(state.clone());

    // Shutdown channel : flipped to `true` on SIGTERM/SIGINT ; both
    // the gRPC server and the metrics exporter watch it.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn the Prometheus exporter.
    let metrics_handle = {
        let rx = shutdown_rx.clone();
        let registry = state.metrics.clone();
        let addr = config.metrics_bind;
        tokio::spawn(async move {
            if let Err(e) = metrics::spawn_exporter(addr, registry, rx).await {
                error!(error = %e, "metrics exporter failed");
            }
        })
    };

    // Build the gRPC server.
    let mut server = Server::builder()
        .timeout(std::time::Duration::from_secs(60))
        .add_service(MetaSwarmServiceServer::new(svc));

    // Tonic-reflection is gated off the `reflection` Cargo feature.
    // PR #6 ships the feature plumbing only ; PR #18 enables it by
    // default for the controller binary.
    let _ = config.reflection_enabled; // suppress unused warning

    let grpc_handle = {
        let mut shutdown_rx = shutdown_rx.clone();
        let addr = config.grpc_bind;
        tokio::spawn(async move {
            info!(addr = %addr, "gRPC server listening");
            let serve = server.serve_with_shutdown(addr, async move {
                let _ = shutdown_rx.changed().await;
                info!("gRPC server shutting down");
            });
            if let Err(e) = serve.await {
                error!(error = %e, "gRPC server failed");
            }
        })
    };

    // Wait for SIGTERM / SIGINT.
    let kind = shutdown::await_termination().await;
    info!(signal = kind, "shutting down");

    // Mark unhealthy : load balancers should stop sending traffic.
    state.health.set_not_serving();

    // Trigger shutdown.
    let _ = shutdown_tx.send(true);

    // Wait for both tasks (bounded by the drain timeout).
    let drain = config.drain_timeout;
    if let Err(e) = tokio::time::timeout(drain, grpc_handle).await {
        error!(error = %e, "gRPC server did not drain in time");
    }
    if let Err(e) = tokio::time::timeout(drain, metrics_handle).await {
        error!(error = %e, "metrics exporter did not drain in time");
    }

    info!("controller stopped");
    Ok(())
}
```

---

## 19. `src/bin/participant.rs` — full

Symmetric to `controller.rs` ; same shape, different service.

```rust
//! Participant binary entry-point.

use std::process::ExitCode;

use efe_meta_swarm_proto::participant::MetaParticipantServiceServer;
use efe_meta_swarm_server::{
    config::participant::ParticipantConfig,
    observability::{metrics, shutdown, tracing as obs_tracing},
    participant::{ParticipantService, ParticipantState},
};

use ::tracing::{error, info};
use tokio::sync::watch;
use tonic::transport::Server;

fn main() -> ExitCode {
    obs_tracing::init();

    let config = match ParticipantConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "participant config error");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.worker_threads)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!(error = %e, "tokio runtime build error");
            return ExitCode::from(2);
        }
    };

    let result = runtime.block_on(async_main(config));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %e, "participant exited with error");
            ExitCode::from(1)
        }
    }
}

async fn async_main(
    config: ParticipantConfig,
) -> Result<(), efe_meta_swarm_server::error::ServerError> {
    info!(
        version = efe_meta_swarm_server::VERSION,
        git_sha = efe_meta_swarm_server::GIT_SHA,
        id = %config.id,
        grpc_bind = %config.grpc_bind,
        metrics_bind = %config.metrics_bind,
        worker_threads = config.worker_threads,
        "participant starting",
    );

    let state = ParticipantState::new(config.clone());
    let svc = ParticipantService::new(state.clone());

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let metrics_handle = {
        let rx = shutdown_rx.clone();
        let registry = state.metrics.clone();
        let addr = config.metrics_bind;
        tokio::spawn(async move {
            if let Err(e) = metrics::spawn_exporter(addr, registry, rx).await {
                error!(error = %e, "metrics exporter failed");
            }
        })
    };

    let server = Server::builder()
        .timeout(std::time::Duration::from_secs(60))
        .add_service(MetaParticipantServiceServer::new(svc));

    let grpc_handle = {
        let mut shutdown_rx = shutdown_rx.clone();
        let addr = config.grpc_bind;
        tokio::spawn(async move {
            info!(addr = %addr, "gRPC server listening");
            let serve = server.serve_with_shutdown(addr, async move {
                let _ = shutdown_rx.changed().await;
                info!("gRPC server shutting down");
            });
            if let Err(e) = serve.await {
                error!(error = %e, "gRPC server failed");
            }
        })
    };

    let kind = shutdown::await_termination().await;
    info!(signal = kind, "shutting down");

    state.health.set_not_serving();
    let _ = shutdown_tx.send(true);

    let drain = std::time::Duration::from_secs(30);
    let _ = tokio::time::timeout(drain, grpc_handle).await;
    let _ = tokio::time::timeout(drain, metrics_handle).await;

    info!("participant stopped");
    Ok(())
}
```

---

## 20. `README.md` — full

````markdown
# `efe-meta-swarm-server`

Tonic-based gRPC servers for the EFE meta-swarm coordinator.

This crate ships **two binaries** under one Cargo crate :

- `controller`  — implements `MetaSwarmService`
- `participant` — implements `MetaParticipantService`

PR #6 (Sprint A1 Phase 2) lands the **scaffold** only.  Subsequent
PRs wire the business logic :

| PR | Adds |
|---|---|
| #8 | `MetaSwarmService::{Iterate, StepOnce, Reset}` |
| #9 | `MetaParticipantService::{Step, Reset, State}` |
| #11 | `EFE_META_SWARM_ENABLED` plumbing in `efe-server` |
| #12 | SIGHUP hot-reload + `MetaSwarmService::ReloadConfig` |
| #16 | mTLS + cert-manager Issuer |
| #17 | 14 Prometheus metrics |
| #18 | 8 OTel span types + tonic-reflection |

## Quick start

### Local — 3 participants + 1 controller

In four terminals :

```bash
# Participant 1 (supplier-scoring)
EFE_META_SWARM_PARTICIPANT_ID=supplier-scoring \
EFE_META_SWARM_PARTICIPANT_ADDR=0.0.0.0:50091 \
EFE_META_SWARM_PARTICIPANT_METRICS=0.0.0.0:9092 \
  cargo run -p efe-meta-swarm-server --bin participant

# Participant 2 (contract-risk)
EFE_META_SWARM_PARTICIPANT_ID=contract-risk \
EFE_META_SWARM_PARTICIPANT_ADDR=0.0.0.0:50092 \
EFE_META_SWARM_PARTICIPANT_METRICS=0.0.0.0:9093 \
  cargo run -p efe-meta-swarm-server --bin participant

# Participant 3 (spend-categorization)
EFE_META_SWARM_PARTICIPANT_ID=spend-categorization \
EFE_META_SWARM_PARTICIPANT_ADDR=0.0.0.0:50093 \
EFE_META_SWARM_PARTICIPANT_METRICS=0.0.0.0:9094 \
  cargo run -p efe-meta-swarm-server --bin participant

# Controller
EFE_META_SWARM_CONTROLLER_ADDR=0.0.0.0:50080 \
EFE_META_SWARM_CONTROLLER_METRICS=0.0.0.0:9091 \
EFE_META_SWARM_PARTICIPANTS=supplier-scoring=localhost:50091,contract-risk=localhost:50092,spend-categorization=localhost:50093 \
  cargo run -p efe-meta-swarm-server --bin controller
```

In a fifth terminal :

```bash
grpcurl -plaintext localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Health
# {"status":"STATUS_SERVING","inflight":0,"reload_gen":0}

grpcurl -plaintext localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Info
# {"buildSha":"abcdef12","protoVersion":"v1","leaderPodName":"hostname",...}

grpcurl -plaintext -d '{}' localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Iterate
# ERROR: Unimplemented: MetaSwarmService::Iterate — wired in PR #8
```

## Environment variables

See [`EFE_A1_PHASE2_ARCHITECTURE.md`][doc] § 7 for the full
catalogue.  PR #6 honours these subsets :

### Controller

| Var | Default | Required? |
|---|---|:---:|
| `EFE_META_SWARM_CONTROLLER_ADDR` | `0.0.0.0:50080` | no |
| `EFE_META_SWARM_CONTROLLER_METRICS` | `0.0.0.0:9091` | no |
| `EFE_META_SWARM_DISCOVERY` | `static-localhost` | no |
| `EFE_META_SWARM_PARTICIPANTS` | (empty) | no (but useful) |
| `EFE_META_SWARM_MANIFEST` | `/etc/efe-meta-swarm/manifest.yaml` | no (PR #8 mandatory) |
| `EFE_META_SWARM_MAX_INFLIGHT` | `10` | no |
| `EFE_META_SWARM_DEFAULT_MAX_WALL_MS` | `15000` | no |
| `EFE_META_SWARM_WORKER_THREADS` | `2` | no |
| `EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S` | `30` | no |
| `EFE_META_SWARM_REFLECTION` | `false` | no |
| `EFE_META_SWARM_TLS_REQUIRED` | `false` | no |
| `EFE_META_SWARM_TLS_CERT` | — | yes if `tls_required=true` |
| `EFE_META_SWARM_TLS_KEY` | — | yes if `tls_required=true` |
| `EFE_META_SWARM_TLS_CA` | — | no |
| `EFE_META_SWARM_LOG` | `info` | no |

### Participant

| Var | Default | Required? |
|---|---|:---:|
| `EFE_META_SWARM_PARTICIPANT_ID` | — | **YES** |
| `EFE_META_SWARM_PARTICIPANT_ADDR` | `0.0.0.0:50090` | no |
| `EFE_META_SWARM_PARTICIPANT_METRICS` | `0.0.0.0:9092` | no |
| `EFE_META_SWARM_WORKER_THREADS` | `1` | no |
| TLS vars | — | same as controller |

## Cargo features

| Feature | Default | What it does |
|---|:---:|---|
| `reflection` | off | Enable `tonic-reflection` (PR #18 ships the dep + integration ; PR #6 plumbs the feature flag only). |

## Building locally

```bash
cargo build -p efe-meta-swarm-server --release
ls target/release/{controller,participant}
```

## Tests

```bash
cargo test -p efe-meta-swarm-server               # unit tests (config, manifest, health, services)
cargo test -p efe-meta-swarm-server --test integration_health_smoke
```

The integration test spawns the `controller` binary on a random port
and asserts `Health` returns `STATUS_SERVING`.  Run with `RUST_LOG=info`
for verbose output.

## Cross-references

- Phase 2 architecture : [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`][doc]
- Phase 2 roadmap : [`docs/EFE_A1_PHASE2_ROADMAP.md`](../../docs/EFE_A1_PHASE2_ROADMAP.md)
- PR #6 detailed architecture : [`docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md)
- Proto crate : [`crates/efe-meta-swarm-proto/README.md`](../efe-meta-swarm-proto/README.md)

[doc]: ../../docs/EFE_A1_PHASE2_ARCHITECTURE.md
````

---

## 21. Environment variables catalogue

The 18 env vars introduced by PR #6, with origin docs :

| Var | Source | Used by | Default |
|---|---|---|---|
| `EFE_META_SWARM_CONTROLLER_ADDR` | PR #6 | controller bin | `0.0.0.0:50080` |
| `EFE_META_SWARM_CONTROLLER_METRICS` | PR #6 | controller bin | `0.0.0.0:9091` |
| `EFE_META_SWARM_DISCOVERY` | PR #6 | controller bin | `static-localhost` |
| `EFE_META_SWARM_PARTICIPANTS` | PR #6 | controller bin | (empty) |
| `EFE_META_SWARM_MANIFEST` | PR #6 | controller bin | `/etc/efe-meta-swarm/manifest.yaml` |
| `EFE_META_SWARM_MAX_INFLIGHT` | PR #6 | controller bin | `10` |
| `EFE_META_SWARM_DEFAULT_MAX_WALL_MS` | PR #6 | controller bin | `15000` |
| `EFE_META_SWARM_WORKER_THREADS` | PR #6 | both bins | `2` / `1` |
| `EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S` | PR #6 | controller bin | `30` |
| `EFE_META_SWARM_REFLECTION` | PR #6 (flag, no impl) | controller bin | `false` |
| `EFE_META_SWARM_PARTICIPANT_ID` | PR #6 | participant bin | **REQUIRED** |
| `EFE_META_SWARM_PARTICIPANT_ADDR` | PR #6 | participant bin | `0.0.0.0:50090` |
| `EFE_META_SWARM_PARTICIPANT_METRICS` | PR #6 | participant bin | `0.0.0.0:9092` |
| `EFE_META_SWARM_TLS_REQUIRED` | PR #6 | both bins | `false` |
| `EFE_META_SWARM_TLS_CERT` | PR #6 | both bins | (none) |
| `EFE_META_SWARM_TLS_KEY` | PR #6 | both bins | (none) |
| `EFE_META_SWARM_TLS_CA` | PR #6 | both bins | (none) |
| `EFE_META_SWARM_LOG` | PR #6 | both bins (tracing-subscriber) | `info` |

The remaining vars in the Phase-2 architecture (§ 7) — `EFE_META_SWARM_OTEL_ENDPOINT`,
`EFE_META_SWARM_TRACE_SAMPLE_RATIO`, `EFE_META_SWARM_RELOAD_ABORT_INFLIGHT`,
`EFE_META_SWARM_LAST_P99_OBSERVED_MS`, `EFE_META_SWARM_DETERMINISTIC` — are
wired by PRs #17, #18, #12 respectively.

---

## 22. Tokio runtime + signal handling

### 22.1 Runtime choice

- `tokio::runtime::Builder::new_multi_thread()`
- `.worker_threads(N)` where N comes from `EFE_META_SWARM_WORKER_THREADS`
- `.enable_all()` (I/O + time)

Rationale :
- gRPC is I/O-bound → multi-thread is mandatory for parallelism.
- Single-thread runtime would serialise all RPCs, killing throughput.
- 2 workers is enough at the planned 100 rps (controller) ; 1 worker
  for participants (less I/O concurrency).

### 22.2 Signal handling

PR #6 handles two signals :

| Signal | Behaviour |
|---|---|
| `SIGINT` (Ctrl+C) | Same as SIGTERM. |
| `SIGTERM` | Mark health NOT_SERVING ; flip shutdown channel ; wait for gRPC + metrics to drain (up to `drain_timeout`) ; exit. |

Deferred to later PRs :

| Signal | Behaviour | PR |
|---|---|---|
| `SIGHUP` | Trigger `ReloadConfig` | PR #12 |
| `SIGUSR1` | Dump debug info to stderr | PR #18 |

### 22.3 Drain sequence (graceful)

1. Receive SIGTERM.
2. Set `HealthState::NotServing`.  K8s readiness probe flips RED ; LB stops sending new RPCs.
3. Flip `shutdown_tx` channel.
4. `Server::serve_with_shutdown` future resolves (no new connections accepted ; in-flight RPCs continue).
5. Metrics exporter's accept loop breaks.
6. `tokio::time::timeout(drain_timeout, …).await` on both join handles.
7. Process exits with code 0.

If any task doesn't drain in time, we log + exit with code 0 anyway
(the K8s preStop hook adds an additional 10 s grace period).

---

## 23. Smoke test : `tests/integration_health_smoke.rs`

```rust
//! End-to-end smoke test : spawn the controller binary, connect via
//! gRPC, call `Health`, assert `STATUS_SERVING`.
//!
//! This is the **load-bearing pin** for the binary's basic
//! correctness.  If a future PR breaks the startup sequence (e.g.
//! moves the gRPC listener after the metrics exporter), this test
//! turns RED.

use std::process::{Child, Command, Stdio};
use std::time::Duration;

use efe_meta_swarm_proto::controller::MetaSwarmServiceClient;
use efe_meta_swarm_proto::meta_swarm::HealthRequest;
use tonic::transport::Endpoint;

struct ProcessGuard {
    child: Child,
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_controller(grpc_port: u16, metrics_port: u16) -> ProcessGuard {
    // CARGO_BIN_EXE_controller : set by cargo at test time, points to
    // the freshly built controller binary.
    let bin = env!("CARGO_BIN_EXE_controller");
    let child = Command::new(bin)
        .env("EFE_META_SWARM_CONTROLLER_ADDR", format!("127.0.0.1:{grpc_port}"))
        .env(
            "EFE_META_SWARM_CONTROLLER_METRICS",
            format!("127.0.0.1:{metrics_port}"),
        )
        .env("EFE_META_SWARM_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn controller binary");
    ProcessGuard { child }
}

async fn wait_until_healthy(endpoint: &str) {
    for _ in 0..40 {
        if let Ok(channel) = Endpoint::from_shared(endpoint.to_string())
            .unwrap()
            .timeout(Duration::from_millis(200))
            .connect_timeout(Duration::from_millis(200))
            .connect()
            .await
        {
            let mut client = MetaSwarmServiceClient::new(channel);
            if let Ok(resp) = client.health(HealthRequest {}).await {
                if resp.into_inner().status == 1 {
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("controller did not become healthy within 10 s");
}

#[tokio::test(flavor = "multi_thread")]
async fn controller_health_returns_serving() {
    // Pick high random-ish ports to avoid conflicts.
    let grpc_port = 53080;
    let metrics_port = 53091;
    let _guard = spawn_controller(grpc_port, metrics_port);

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    wait_until_healthy(&endpoint).await;

    let channel = Endpoint::from_shared(endpoint).unwrap().connect().await.unwrap();
    let mut client = MetaSwarmServiceClient::new(channel);
    let resp = client.health(HealthRequest {}).await.unwrap();
    let h = resp.into_inner();
    assert_eq!(h.status, 1, "expected STATUS_SERVING");
    assert_eq!(h.inflight, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn controller_iterate_returns_unimplemented() {
    let grpc_port = 53082;
    let metrics_port = 53092;
    let _guard = spawn_controller(grpc_port, metrics_port);

    let endpoint = format!("http://127.0.0.1:{grpc_port}");
    wait_until_healthy(&endpoint).await;

    let channel = Endpoint::from_shared(endpoint).unwrap().connect().await.unwrap();
    let mut client = MetaSwarmServiceClient::new(channel);
    let err = client
        .iterate(efe_meta_swarm_proto::meta_swarm::IterateRequest::default())
        .await
        .unwrap_err();
    assert_eq!(err.code(), tonic::Code::Unimplemented);
    assert!(err.message().contains("PR #8"));
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_endpoint_responds() {
    let grpc_port = 53084;
    let metrics_port = 53094;
    let _guard = spawn_controller(grpc_port, metrics_port);

    // Wait for the metrics port to become reachable.
    let url = format!("http://127.0.0.1:{metrics_port}/metrics");
    for _ in 0..40 {
        if let Ok(resp) = reqwest::get(&url).await {
            if resp.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("metrics endpoint did not become reachable within 10 s");
}
```

The smoke test uses `reqwest` for the metrics endpoint check.  Add to
`[dev-dependencies]` :

```toml
reqwest = { version = "0.12", default-features = false, features = ["http2"] }
```

---

## 24. CI validation

Same shape as PR #5 :

| Job | Trigger | Pass criterion |
|---|---|---|
| `cargo build -p efe-meta-swarm-server --bins` | every push | exit 0 |
| `cargo build -p efe-meta-swarm-server --release --bins` | every push | exit 0 |
| `cargo build -p efe-meta-swarm-server --features reflection` | every push | exit 0 |
| `cargo test -p efe-meta-swarm-server` (unit) | every push | all pass |
| `cargo test -p efe-meta-swarm-server --test integration_health_smoke` | every push | 3 tests pass |
| `cargo doc -p efe-meta-swarm-server --no-deps` | every push | 0 warnings |
| `cargo clippy -p efe-meta-swarm-server --all-targets` | every push | 0 warnings on this crate |
| Phase-1 master gate | every push | exit 0 (no regression) |
| PR #5 still builds + tests | every push | exit 0 |
| Workspace `cargo check` of a sample (efe-server, efe-core, efe-meta-swarm) | every push | exit 0 |

No new workflows in PR #6 ; the new `bench-meta-swarm-grpc` job lands
in PR #22.

---

## 25. Acceptance criteria (binary)

PR #6 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `cargo build -p efe-meta-swarm-server` succeeds with 0 warnings | CI |
| AC2 | `cargo build -p efe-meta-swarm-server --release --bins` succeeds | CI |
| AC3 | `cargo build -p efe-meta-swarm-server --features reflection` succeeds | CI |
| AC4 | `cargo run --bin controller` (with `EFE_META_SWARM_PARTICIPANT_ID` unset — controller doesn't need it) starts, binds 50080 + 9091, exits cleanly on SIGTERM | Manual + smoke |
| AC5 | `cargo run --bin participant` (with `EFE_META_SWARM_PARTICIPANT_ID=test`) starts, binds 50090 + 9092, exits cleanly on SIGTERM | Manual + smoke |
| AC6 | `MetaSwarmService::Health` returns `STATUS_SERVING` | smoke test |
| AC7 | `MetaSwarmService::Info` returns a populated `InfoResponse` (build_sha, proto_version="v1", hostname) | smoke test (extended) |
| AC8 | `MetaSwarmService::Iterate/StepOnce/Reset/ReloadConfig/IterateStream` all return `Status::unimplemented` with a "wired in PR #N" message | smoke test + unit test |
| AC9 | `MetaParticipantService::Health` returns `STATUS_SERVING` | smoke test |
| AC10 | `MetaParticipantService::Capabilities` returns a `ParticipantDescriptor` with the configured id | unit test |
| AC11 | `MetaParticipantService::Step/Reset/State/StepStream` all return `Status::unimplemented` | unit + smoke |
| AC12 | `/metrics` endpoint responds with `200 OK` + Prometheus text format (empty registry, just `# EOF` or similar) | smoke test |
| AC13 | `cargo test -p efe-meta-swarm-server` passes all unit + integration tests | CI |
| AC14 | `cargo clippy -p efe-meta-swarm-server --all-targets` : 0 warnings on this crate | CI |
| AC15 | `cargo doc -p efe-meta-swarm-server --no-deps` : 0 warnings | CI |
| AC16 | `cargo fmt -p efe-meta-swarm-server -- --check` : exit 0 | CI |
| AC17 | Phase-1 master gate stays GREEN | CI |
| AC18 | PR #5 tests still pass (regression check) | CI |
| AC19 | No file outside `crates/efe-meta-swarm-server/`, `Cargo.toml`, `Cargo.lock` is modified | `git diff --stat` review |
| AC20 | 4 reviewers approved : 2× meta-swarm + 1× sre + 1× platform | GitHub |
| AC21 | sre-team has confirmed the SIGTERM drain protocol matches K8s preStop expectations | SRE comment |
| AC22 | platform-team has confirmed the env-var contract matches the Helm chart values plan (PR #13 will consume them) | platform comment |

---

## 26. PR description template

```markdown
# [Sprint A1 Phase 2 — PR #6] feat(efe-meta-swarm-server): scaffold

## Summary

Second PR of Phase 2.  Lands the gRPC server scaffold : two
binaries (`controller`, `participant`) under one crate
`efe-meta-swarm-server`.  Wires the Tonic surface defined in
`efe-meta-swarm-proto` (PR #5).

Every RPC except `Health`, `Info`, and `Capabilities` returns
`Status::unimplemented("... wired in PR #N")`.

This is **pure scaffolding**.  Real business logic lands in PRs
#8, #9, #11, #12, #16, #17, #18.

Full architecture : [`docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md).

## What this PR adds

| Area | Detail |
|---|---|
| New crate | `crates/efe-meta-swarm-server/` (~ 2 100 LOC) |
| Binaries | `controller` + `participant` |
| Modules | `config`, `manifest`, `discovery`, `observability`, `auth`, `controller`, `participant`, `error` |
| Real RPCs | `Health`, `Info`, `Capabilities` |
| Stubbed RPCs (return UNIMPLEMENTED) | the remaining 10 (`Iterate`, `IterateStream`, `StepOnce`, `Reset` ×2, `ReloadConfig`, `Step`, `State`, `StepStream`) |
| Env vars introduced | 18 (cf. `README.md` § "Environment variables") |
| Signal handling | SIGTERM + SIGINT graceful drain |
| Prometheus exporter | `/metrics` on a separate port (empty registry until PR #17) |
| Tracing | `tracing-subscriber` (JSON / pretty based on TTY) |
| `panic = "unwind"` override | crate-local |
| Smoke test | `tests/integration_health_smoke.rs` (3 tests : spawn binary, gRPC call, metrics endpoint) |

## Test plan

- [x] `cargo build -p efe-meta-swarm-server --bins` (default features) succeeds
- [x] `cargo build -p efe-meta-swarm-server --bins --release` succeeds
- [x] `cargo build -p efe-meta-swarm-server --bins --features reflection` succeeds
- [x] `cargo test -p efe-meta-swarm-server` : all unit tests pass
- [x] `cargo test -p efe-meta-swarm-server --test integration_health_smoke` : 3 / 3 pass
- [x] `cargo clippy -p efe-meta-swarm-server --all-targets` : 0 warnings
- [x] `cargo fmt -p efe-meta-swarm-server -- --check` : exit 0
- [x] `cargo doc -p efe-meta-swarm-server --no-deps` : 0 warnings
- [x] Phase-1 master gate stays GREEN
- [x] PR #5 (`efe-meta-swarm-proto`) tests still pass

## Non-regression statement

- ✅ Zero file modified outside `crates/efe-meta-swarm-server/`,
  workspace `Cargo.toml` (+1 member), `Cargo.lock` (~ 20 lines diff).
- ✅ Phase 1 unaffected : `cargo test -p efe-meta-swarm` still passes
  (200 default / 210 attention-weighted / 203 core-integration).
- ✅ PR #5 (`efe-meta-swarm-proto`) unaffected.
- ✅ Carve-out R5-B1 respected.

## Honest semantics

- 10/13 RPCs return UNIMPLEMENTED — by design.  Real bodies in
  PRs #8, #9, #11, #12.
- No mTLS in PR #6 ; plaintext gRPC.  PR #16 wires mTLS.
- No real metrics ; the registry is empty.  PR #17 wires the 14.
- No OTel exporter ; only the `tracing-subscriber` (logs).  PR #18
  wires OTel.
- No multi-process test ; single-process smoke only.  PR #10 owns
  multi-process.

## Reviewer guidance

Suggested order :

1. [`docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md) — read this first.
2. `crates/efe-meta-swarm-server/README.md` — operator-facing.
3. `crates/efe-meta-swarm-server/Cargo.toml` — note the `panic = "unwind"` override.
4. `src/bin/{controller,participant}.rs` — startup sequence (read both, they're symmetric).
5. `src/controller/service.rs` + `src/participant/service.rs` — the UNIMPLEMENTED stubs.
6. `src/observability/{health,shutdown,metrics}.rs` — cross-cutting.
7. `tests/integration_health_smoke.rs` — what the surface promises.

Required approvers : 2× meta-swarm + 1× sre + 1× platform.

## References

- [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_ARCHITECTURE.md) (Phase 2 design)
- [`docs/EFE_A1_PHASE2_ROADMAP.md`](docs/EFE_A1_PHASE2_ROADMAP.md) § 2.1
- [`docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md) (PR #5 — the proto crate this depends on)
- [`crates/efe-meta-swarm-proto/README.md`](crates/efe-meta-swarm-proto/README.md)
```

---

## 27. Risk analysis (PR-local)

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| PR6-R1 | `[profile.release].panic = "unwind"` not honoured by cargo for a non-root crate | M/H | Acceptance test : run the binary, panic at startup, observe drop runs.  Fallback : bump workspace `panic = "unwind"` (tracked as `DT-A1-PHASE2-PANIC-UNWIND`). |
| PR6-R2 | Smoke test flaky in CI (port already in use) | L/M | Hard-code high ports (53080-53094) unlikely to collide.  If flaky, switch to `:0` (kernel-assigned) + read back. |
| PR6-R3 | Tonic 0.12's `Server::serve_with_shutdown` API change | L/L | PR #5 already pins 0.12 ; semver-compat. |
| PR6-R4 | Hyper 1.0 metrics endpoint compile errors on aarch64 | L/M | Verified on macOS aarch64 dev box ; CI is ubuntu x86_64 (covered). |
| PR6-R5 | `signal-hook` collides with tokio's signal handling | L/M | We use tokio's `signal::unix` exclusively ; `signal-hook` is in deps but NOT used in PR #6 code (reserved for PR #12 SIGHUP).  Cargo will dedupe. |
| PR6-R6 | A reviewer asks for `clap` instead of env vars | M/L | Reference AD3 ; env-only matches Phase-2 architecture.  Defer CLI args to a Phase-3 RFC if needed. |
| PR6-R7 | The metrics exporter spinning a tokio task that doesn't drain on shutdown | L/M | `tokio::time::timeout` on the join handle + the watch channel guarantees liveness.  Smoke test verifies SIGTERM exit. |
| PR6-R8 | `env!("CARGO_BIN_EXE_controller")` not set during regular `cargo build` | L/L | Cargo sets this env var ONLY for tests using the binary.  We use it ONLY in `tests/integration_health_smoke.rs`.  Verified. |

---

## 28. Reviewer checklist

### General

- [ ] PR title : `feat(efe-meta-swarm-server): scaffold`.
- [ ] PR description follows template.
- [ ] 6 distinct commits, each individually CI-green.
- [ ] Diff scope strictly limited to `crates/efe-meta-swarm-server/` + workspace registration.

### Crate metadata

- [ ] `Cargo.toml` :
  - [ ] `publish = false`
  - [ ] `version = "0.2.0"`
  - [ ] `rust-version = "1.88"`
  - [ ] 2 `[[bin]]` blocks
  - [ ] `[profile.release].panic = "unwind"` with comment
  - [ ] All workspace deps use `{ workspace = true }`

### `build.rs`

- [ ] Sets `GIT_SHA` env var.
- [ ] Sanitizes the SHA (alphanumeric, length ≤ 16, fallback `nogit` / `invalid-sha`).
- [ ] `cargo:rerun-if-changed=.git/HEAD`.

### Binaries

- [ ] Both `controller` and `participant` `main()` :
  - [ ] Call `obs_tracing::init()`.
  - [ ] Parse config from env.
  - [ ] Build a multi-thread tokio runtime.
  - [ ] Spawn metrics exporter.
  - [ ] Spawn gRPC server with `serve_with_shutdown`.
  - [ ] Await SIGTERM / SIGINT.
  - [ ] Set health NOT_SERVING.
  - [ ] Drain with timeout.
  - [ ] Return `ExitCode::SUCCESS` on clean exit.

### Services

- [ ] `ControllerService::health` returns the real `HealthState::snapshot()`.
- [ ] `ControllerService::info` returns build_sha + proto_version="v1" + hostname.
- [ ] All 10 other RPCs return `Status::unimplemented` with a "PR #N" message.
- [ ] `ParticipantService::health` and `capabilities` real ; rest UNIMPLEMENTED.

### Smoke test

- [ ] 3 tests in `tests/integration_health_smoke.rs` :
  - [ ] `controller_health_returns_serving`
  - [ ] `controller_iterate_returns_unimplemented`
  - [ ] `metrics_endpoint_responds`
- [ ] All 3 use a `ProcessGuard` that kills the child on drop.
- [ ] Ports are hard-coded > 50000 (high enough to avoid privileged conflict).

### Observability

- [ ] `HealthState` uses atomics (no Mutex).
- [ ] `MetricsRegistry` is empty (PR #17 will populate).
- [ ] tracing subscriber output : JSON when stderr is not a TTY, pretty when it is.
- [ ] Shutdown : `tokio::signal::unix::signal(SignalKind::terminate())`.

### Documentation

- [ ] README covers : quick start, env vars, features, tests, cross-refs.
- [ ] No marketing in docs.
- [ ] Architecture doc cross-refs present.

### CI

- [ ] All CI green on latest push.
- [ ] Phase-1 master gate still GREEN.
- [ ] PR #5 tests still pass.

### Sign-off

- [ ] 2× meta-swarm-team approvals.
- [ ] 1× sre-team approval.
- [ ] 1× platform-team approval.

---

## 29. Definition of Done

PR #6 is done iff :

1. All 22 acceptance criteria GREEN.
2. All 4 reviewer approvals in.
3. Merged to `main` with merge-commit linking to this doc.
4. Slack post in `#meta-swarm-standup` : `PR #6 merged ; PRs #8, #9, #11 unblocked`.
5. PR #5's smoke test still passes on `main` (regression check).

PR #6 is NOT done if :

- Any AC is RED.
- A reviewer has a blocking comment unresolved.
- Phase-1 master gate is RED.
- The binary cannot be run via `cargo run` (manual repro by reviewer).
- `cargo doc` produces warnings on PR-6 files.

---

## 30. Post-merge follow-ups

These unlock once PR #6 lands :

| Follow-up | PR | Owner | When |
|---|---|---|---|
| `MetaSwarmService::Iterate` body | PR #8 | meta-swarm-team | Day 2 after #6 merge |
| `MetaParticipantService::Step` body | PR #9 | meta-swarm-team | Day 2 |
| `tests/integration_grpc_multiprocess.rs` | PR #10 | meta-swarm-team | Day 5 |
| `EFE_META_SWARM_ENABLED` plumbing | PR #11 | meta-swarm-team | Sprint 2 W3 |
| SIGHUP + ReloadConfig | PR #12 | meta-swarm-team | Sprint 2 W3 |
| Helm sub-chart | PR #13 | platform-team | Sprint 2 W3 |
| mTLS | PR #16 | sec + platform | Sprint 2 W4 |
| 14 Prometheus metrics | PR #17 | meta-swarm-team | Sprint 3 W5 |
| 8 OTel spans + reflection enable | PR #18 | meta-swarm-team | Sprint 3 W5 |
| loadgen | PR #20 | meta-swarm-team | Sprint 3 W6 |
| public-bench harness | PR #21 | meta-swarm-team | Sprint 3 W6 |

## Appendix A — File-by-file deliverable checklist

```
crates/efe-meta-swarm-server/
├── Cargo.toml                                   [✓ in this PR]
├── build.rs                                     [✓ in this PR]
├── README.md                                    [✓ in this PR]
├── src/
│   ├── lib.rs                                   [✓]
│   ├── error.rs                                 [✓]
│   ├── config/{mod, controller, participant}.rs [✓]
│   ├── manifest/{mod, schema}.rs                [✓]
│   ├── discovery/{mod, static_impl}.rs          [✓]
│   ├── observability/{mod, health, metrics, tracing, shutdown}.rs [✓]
│   ├── auth/{mod, mtls, bearer}.rs              [✓]
│   ├── controller/{mod, service, state}.rs      [✓]
│   ├── participant/{mod, service, state}.rs     [✓]
│   └── bin/{controller, participant}.rs         [✓]
└── tests/integration_health_smoke.rs            [✓]

Cargo.toml (workspace)                            [✓ touched : +1 member]
Cargo.lock                                        [✓ regenerated]
```

## Appendix B — Glossary

- **AD-N** : Architectural Decision N (cf. § 4).
- **AC-N** : Acceptance Criterion N (cf. § 25).
- **PR6-RN** : PR-local Risk N (cf. § 27).
- **UNIMPLEMENTED** : the standard gRPC code returned for stub RPCs.  In tonic : `Status::unimplemented(...)`.
- **Graceful drain** : the SIGTERM handling sequence that lets in-flight RPCs finish before the process exits (§ 22.3).
- **`panic = "unwind"`** : the panic strategy that runs `Drop` impls.  Phase-1 uses `"abort"` ; PR #6 overrides for this crate (AD2).
- **K8s preStop hook** : a Kubernetes feature that runs a command in the container before SIGTERM ; gives 10 s extra for load balancers to deregister.
