# EFE-A1 Phase 2 — PR #11 : `EFE_META_SWARM_ENABLED` plumbing

| Field | Value |
|---|---|
| **PR** | #11 (Sprint A1 Phase 2 / P2.2) |
| **Branch** | `efe-a1/p2-pr11-efe-server-flag-plumbing` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× efe-server + 1× meta-swarm + 1× sre |
| **Depends on** | PR #5 (proto), PR #8 (controller wire), PR #9 (participant wire), PR #10 (multi-process harness) |
| **Blocks** | PR #13 (canary observability), PR #15 (actual dispatch wiring), PR #18 (OTel + reflection) |
| **L2 gate** | G5 (controlled rollout — flag-gated, default-off) |
| **Threat IDs in scope** | T2, T4, T6 (cf. `docs/security/PHASE2_THREAT_MODEL.md`) |
| **Roadmap reference** | EFE_A1_PHASE2_ROADMAP.md § C4, Table-of-PRs row #11 |
| **CVSS bandwidth** | All ≤ SEV-3 ; no new SEV-1 introduced |

---

## Table of contents

  0. TL;DR
  1. Goal + non-goals (the "plumbing-only" scope)
  2. Pre-flight checks
  3. Branch + commit plan (5 commits)
  4. Architectural decisions (AD1–AD13)
  5. Topology + integration diagram
  6. Crate layout additions
  7. The `EFE_META_SWARM_*` env-var contract
  8. `MetaSwarmConfig` — typed config struct
  9. `services/meta_swarm/` — module + `MetaSwarmClient` adapter
 10. `main.rs` / startup wiring
 11. `runtime.rs` — runtime holds the optional client
 12. Connection lifecycle
 13. Failure modes + the fail-fast policy
 14. Metrics + tracing
 15. Off-vs-on byte-equivalence (the load-bearing test)
 16. Why no dispatch in this PR
 17. Tests strategy summary
 18. Acceptance criteria (AC1–AC22)
 19. Risks + mitigations
 20. Rollback strategy
 21. Sequence diagrams (ASCII)
 22. Reviewer checklist
 23. Migration notes (operator-facing)
 24. Open questions
 25. Appendix : full file inventory
 26. Glossary

---

## 0. TL;DR

PR #11 wires the `EFE_META_SWARM_ENABLED` feature flag through `efe-server`
without yet changing any request-handling code path. After this PR :

  * Operators can set `EFE_META_SWARM_ENABLED=true` on an `efe-server`
    Pod ; at startup, the server :
      1. parses `EFE_META_SWARM_*` env vars into a typed
         `MetaSwarmConfig` (PR #11 § 8),
      2. constructs a `MetaSwarmClient` that establishes a gRPC channel
         to the controller specified by
         `EFE_META_SWARM_CONTROLLER_ENDPOINT` (default
         `http://127.0.0.1:50080`),
      3. probes the controller's `Health` RPC once and refuses to start
         if it returns anything other than `STATUS_SERVING`.
  * The constructed client is **held** in the server's `RuntimeState`
    as `Option<Arc<MetaSwarmClient>>`, **available but unused**.
  * `EFE_META_SWARM_ENABLED=false` (default) is byte-identical to
    pre-PR-#11 behaviour ; the meta-swarm subsystem is not even
    initialised.
  * A new integration test asserts that the server's `Execute` /
    `ExecuteStreaming` responses are byte-identical between `false` and
    `true` modes given the same input (because no code path consumes the
    client yet).

PR #15 will later route specific multi-swarm coordination calls through
this client.  PR #11 is the **safe-by-construction** prerequisite : the
flag exists, the client connects, the connection is held, but nothing
flows through it.

Rollback : `kubectl set env deployment/efe-server EFE_META_SWARM_ENABLED=false`
and restart.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Add the typed config, the gRPC client adapter, and the startup
> wiring for the meta-swarm subsystem inside `efe-server`, such that
> setting `EFE_META_SWARM_ENABLED=true` causes the server to connect
> to the controller and hold an open channel — but **does not** route
> any request through that channel.

### 1.2 Non-goals (deferred to later PRs)

The following are **explicitly out of scope** for PR #11 :

| Item | Why deferred | Target PR |
|---|---|---|
| Routing `Execute` / `ExecuteStreaming` requests through the meta-swarm | needs the legacy-vs-meta dispatch contract finalised | PR #15 |
| Shadow mode (call both legacy and meta, compare outputs) | needs PR #15's dispatch + a comparison harness | PR #16 |
| Canary % traffic split | needs per-tenant routing | PR #17 |
| Per-request kill switch (`X-EFE-Meta-Swarm: skip` header) | hot-path config plumbing | PR #19 |
| OpenTelemetry export for the meta-swarm calls | needs the OTel collector wired | PR #18 |
| mTLS for `efe-server ↔ controller` channel | needs cert provisioning | PR #16 |
| Multi-controller load balancing | needs the Discovery refresh from PR #11 the meta-swarm-side (sic), but distinct | PR #17 |
| Connection retry / reconnect policy beyond tonic defaults | needs ops feedback from PR #10 deployments | PR #19 |
| `MetaSwarmClient` exposed in `efe-core` for embedder/reranker plug-in | scope creep | Phase 3 |
| TLS handshake failure modes | Phase 3 |
| Authentication (controller-side) — controller still serves anonymous | PR #16 |
| Schema validation of `Capabilities.descriptor` against an SBOM | Phase 3 |

### 1.3 The "plumbing-only" doctrine

PR #11 is a strictly **additive** plumbing change.  The doctrine :

> *Every byte of input that produced an output before this PR must
> produce the same byte of output after this PR, in both `enabled=false`
> AND `enabled=true` modes, for the entire `efe-server` surface.*

This is verified by the integration test `off_vs_on_byte_equivalence`
(PR #11 § 15).  It is the contractual reason the operator can flip the
flag in production without rollback-readiness drills.

PR #15 will violate this doctrine deliberately (the meta-swarm path
will produce **different** — better — outputs) ; that PR has its own
canary-rollout strategy.  PR #11 prepares the substrate without taking
the leap.

---

## 2. Pre-flight checks

Before opening the PR :

| # | Check | Verifier | Pass criterion |
|---|---|---|---|
| PF1 | `cargo build -p efe-server --release` succeeds on `main` | CI | green |
| PF2 | `cargo test -p efe-server --offline --all-targets` baseline is **stable** on `main` (record the count) | CI | green |
| PF3 | `cargo test -p efe-meta-swarm-server --offline --all-targets` is **365 tests** (PR #10 baseline) | CI | green |
| PF4 | `efe-meta-swarm-proto` is published in the workspace and `efe-server` does **not** depend on it yet | grep | absent in `efe-server/Cargo.toml` |
| PF5 | The threat model (PR #7) is in v1.0 frozen state | manual | sec sign-off |
| PF6 | The `ServerConfig::from_env_with` lookup pattern (config.rs:761) is unchanged | grep | matches PR #6 baseline |
| PF7 | `services::DynExecuteBackend = Arc<dyn ExecuteBackend>` (services/mod.rs:144) is unchanged | grep | matches PR #6 baseline |
| PF8 | `RuntimeState` (runtime.rs) holds Arc-shareable handles for the lifetime of the server | manual | confirmed |
| PF9 | `PR #10`'s `ProcessHarness` can spawn meta-swarm subprocesses on demand | grep | `tests/common/multiprocess.rs` present |
| PF10 | `EFE_META_SWARM_CONTROLLER_ENDPOINT` is **not yet** referenced in any other source file in `efe-server` (`grep` empty) | grep | empty |

Any failed pre-flight blocks PR #11.

---

## 3. Branch + commit plan

### 3.1 Branch

```
efe-a1/p2-pr11-efe-server-flag-plumbing
```

Branched from `main` after PR #10 merge SHA.  **Never** rebase onto a
moving `main`.

### 3.2 Commit-by-commit plan (5 commits, each CI-green)

| # | Subject | Files | LOC ± | Verification |
|---|---|---|---|---|
| C1 | `feat(efe-server): MetaSwarmConfig env-var struct + validation` | `~ src/config.rs` | +160 / −0 | unit tests in config tests module (≥ 8) |
| C2 | `feat(efe-server): MetaSwarmClient adapter (Health probe only)` | `+ src/services/meta_swarm/mod.rs`, `~ src/services/mod.rs` | +220 / −0 | unit tests in module (≥ 5) ; mocked endpoint |
| C3 | `feat(efe-server): main.rs builds + holds MetaSwarmClient when enabled` | `~ src/main.rs`, `~ src/runtime.rs` | +90 / −5 | binary smoke : both `enabled=false` and `enabled=true` start and serve |
| C4 | `feat(efe-server): expose meta-swarm metrics namespace (counters + histogram)` | `~ src/services/meta_swarm/metrics.rs`, `~ src/services/metrics.rs` | +120 / −0 | unit tests in module ; `/metrics` scrape contains `efe_meta_swarm_*` names when enabled |
| C5 | `feat(efe-server): off-vs-on byte-equivalence integration test` | `+ tests/integration_meta_swarm_plumbing.rs`, `~ tests/common/mod.rs` (re-export) | +280 / −0 | 4 tests : config parsing CI, off mode baseline, on mode equivalence, on mode connect failure fails fast |

Total : **+870 / −5 = +865 LOC net** including ~ 280 LOC of integration
tests.

### 3.3 Drafting strategy

Each commit is **green-on-merge**.  C1-C2 add new code with no external
caller (no behaviour change).  C3 wires the construction but holds the
client unused.  C4 adds metrics that only fire when `enabled=true`.  C5
adds the load-bearing equivalence test.

If C5 fails (off-vs-on differs), one of C1-C4 has a bug.  Bisect easy.

---

## 4. Architectural decisions

| AD | Decision | Rationale | Alternatives considered | Reversibility |
|---|---|---|---|---|
| **AD1** | New `MetaSwarmConfig` struct lives in `efe-server/src/config.rs` (same file as `ServerConfig`) | Single source of truth for env-var parsing ; matches existing layering | `efe-server/src/services/meta_swarm/config.rs` (split-out) ; reuse `efe-meta-swarm-server::config::ControllerConfig` (reverse coupling) | Reversible — config struct is purely internal |
| **AD2** | **Default `EFE_META_SWARM_ENABLED=false`** ; existing deployments byte-identical | The flag must opt-in to risk ; rollback is "unset the var" | Default true ; gradual rollout via percentages | Reversible — default is one constant |
| **AD3** | **Hard fail-fast** : if `enabled=true` and the Health probe fails, the binary exits 1 | Surfaces misconfig immediately ; no silent degradation that masks a missing controller | Soft fail (log + continue with `None` client) ; retry-with-backoff at startup | Reversible — single conditional |
| **AD4** | Health probe happens **once** at startup, after gRPC channel construction | Cheap (~1 ms localhost) ; one network round-trip ; deterministic startup contract | No probe ; periodic probe (background task) ; on-demand probe at first dispatch | Reversible — one method call |
| **AD5** | **No runtime reconfiguration** : the client is built once at startup ; flipping the flag requires a restart | Phase-2 K8s rollouts are restart-friendly ; flag flips are deployment events | Hot-reload via SIGHUP (PR #12 will offer this for the meta-swarm-server, not yet for efe-server) | Reversible — defer to PR #19 if ops asks |
| **AD6** | The client is held as `Option<Arc<MetaSwarmClient>>` on `RuntimeState` | `None` when `enabled=false` ; `Arc` allows cheap cloning into per-request handlers | Hold a `MetaSwarmClient` directly (mandatory) ; hold a `Box<dyn AnyClient>` (overkill) | Reversible — purely structural |
| **AD7** | The `MetaSwarmClient` is a **thin wrapper** over `MetaSwarmServiceClient<Channel>` (the tonic-generated client) | Mirrors PR #8's `RemoteParticipant` ; consistent style ; allows adding instrumentation later | Use the raw tonic client directly | Reversible — the wrapper has one public method (`health_probe`) in PR #11 ; more added in PR #15 |
| **AD8** | Metrics use the **`efe_meta_swarm_*`** namespace (not `efe_swarm_*`) | Avoids collision with Phase-1 swarm metrics ; greppable for canary dashboards | `efe.meta_swarm.*` dot-style | Reversible — Prometheus names are operator-visible but adjustable in PR #15 |
| **AD9** | Health probe timeout = 5 s ; per-RPC default timeout = 20 s | Mirrors PR #10's `connect_controller` defaults | Tighter (1 s) — too aggressive for cold runtime ; looser (30 s) — blocks startup too long | Reversible — config-driven via `EFE_META_SWARM_CONNECT_TIMEOUT_MS` |
| **AD10** | The integration test spawns the **real** controller + 3 participants via PR #10's `ProcessHarness` ; no test stubs | Production code in test ; same wire format ; flushes real bugs | mock `tonic` server in-process | Reversible — but loses fidelity |
| **AD11** | PR #11 does **not** touch `services/swarm.rs` (`SwarmServiceImpl::execute`).  The client is plumbed but never invoked | Smaller blast radius ; the dispatch is a separate decision (PR #15) ; ensures off-vs-on equivalence | Wire dispatch now (PR collapses #11 + #15) | Reversible — but increases review burden |
| **AD12** | The `MetaSwarmConfig` validates the controller endpoint URI **at config parse time**, not at probe time | Catches typos before any gRPC machinery boots ; same pattern as `bind: SocketAddr` | Lazy parse | Reversible — one `.parse()` call |
| **AD13** | The probe + connect happen on the main tokio runtime (not a dedicated one) | One runtime is enough ; the connect is short-lived ; no benefit to isolation | Spawn a dedicated runtime for meta-swarm | Reversible — easy to move later |

---

## 5. Topology + integration diagram

### 5.1 Before PR #11

```
   ┌────────────────────────────────────────┐
   │ efe-server   (PID E)                   │
   │   • SwarmServiceImpl                   │
   │   • DynExecuteBackend (Phase-1)        │
   │   • RuntimeState                       │
   └─────────────────────┬──────────────────┘
                         │  (Phase-1 only)
                         ▼
                       embedder / catalog / …
```

### 5.2 After PR #11, `EFE_META_SWARM_ENABLED=false` (default)

```
   ┌────────────────────────────────────────┐
   │ efe-server   (PID E)                   │
   │   • SwarmServiceImpl                   │
   │   • DynExecuteBackend (Phase-1)        │
   │   • RuntimeState                       │
   │   • meta_swarm_client : None           │ ← byte-identical
   └─────────────────────┬──────────────────┘   to "before"
                         ▼
                       embedder / catalog / …
```

### 5.3 After PR #11, `EFE_META_SWARM_ENABLED=true`

```
   ┌────────────────────────────────────────────────┐
   │ efe-server   (PID E)                           │
   │   • SwarmServiceImpl                           │
   │   • DynExecuteBackend (Phase-1)  ← still used  │
   │   • RuntimeState                               │
   │   • meta_swarm_client : Some(Arc<…>)           │ ← held but unused
   └─────────────────────┬───────────┬──────────────┘
                         │           │
                         │           │ (no request goes through ; PR #15 will)
                         ▼           ▼
            embedder / catalog       meta-swarm controller (gRPC)
                                       at $EFE_META_SWARM_CONTROLLER_ENDPOINT
                                     [Health probe at startup only]
```

### 5.4 The Kovant production topology (after PR #11 + PR #15)

```
                   ┌──────────────────────────────────────┐
                   │ efe-server                           │
                   │   meta_swarm_client : Some(Arc<…>)   │
                   └──────────────┬───────────────────────┘
                                  │ (PR #15 routes Iterate calls here)
                                  ▼
              ┌──────────────────────────────────────────────┐
              │ controller   (efe-meta-swarm-server bin)     │
              └──────┬────────────────┬──────────────────────┬──┘
                     ▼                ▼                      ▼
              participant-1   participant-2          participant-3
```

PR #11 only delivers the dotted-arrow stop at the controller's Health
probe ; the **other** dotted arrow (Iterate dispatch) lives in PR #15.

---

## 6. Crate layout additions

### 6.1 Before PR #11

```
crates/efe-server/
├── src/
│   ├── bin/                              (PR earlier)
│   ├── config.rs                         (~ 2 126 LOC, ServerConfig)
│   ├── main.rs
│   ├── runtime.rs
│   ├── services/
│   │   ├── mod.rs                        (ExecuteBackend trait)
│   │   ├── swarm.rs                      (SwarmServiceImpl)
│   │   ├── metrics.rs
│   │   └── (other services)
│   └── (other modules)
└── Cargo.toml                            (no efe-meta-swarm-proto)
```

### 6.2 After PR #11

```
crates/efe-server/
├── src/
│   ├── bin/                              (unchanged)
│   ├── config.rs                         [MODIFIED : + MetaSwarmConfig]
│   ├── main.rs                           [MODIFIED : + startup wiring]
│   ├── runtime.rs                        [MODIFIED : + meta_swarm_client field]
│   ├── services/
│   │   ├── mod.rs                        [MODIFIED : + pub mod meta_swarm]
│   │   ├── swarm.rs                      (unchanged — AD11)
│   │   ├── metrics.rs                    [MODIFIED : + register meta-swarm metrics]
│   │   ├── meta_swarm/                   [NEW]
│   │   │   ├── mod.rs                    (~ 180 LOC, MetaSwarmClient)
│   │   │   └── metrics.rs                (~ 80 LOC, Counter + Histogram)
│   │   └── (other services)
│   └── (other modules)
├── tests/
│   └── integration_meta_swarm_plumbing.rs  [NEW : ~ 280 LOC, 4 tests]
├── Cargo.toml                            [MODIFIED : + efe-meta-swarm-proto]

docs/
└── EFE_A1_PHASE2_PR11_ARCHITECTURE.md   (this file)
```

---

## 7. The `EFE_META_SWARM_*` env-var contract

| Variable | Type | Default | Required when | Description |
|---|---|---|---|---|
| `EFE_META_SWARM_ENABLED` | bool (`true`/`false`/`1`/`0`/`yes`/`no`) | `false` | always | master switch ; flip to `true` activates the subsystem |
| `EFE_META_SWARM_CONTROLLER_ENDPOINT` | URL (`http://…` or `https://…`) | `http://127.0.0.1:50080` | `ENABLED=true` | controller's gRPC endpoint ; matches `EFE_META_SWARM_CONTROLLER_ADDR` on the controller side |
| `EFE_META_SWARM_CONNECT_TIMEOUT_MS` | u64 (ms) | `5000` | `ENABLED=true` | tonic connect-timeout |
| `EFE_META_SWARM_CALL_TIMEOUT_MS` | u64 (ms) | `20000` | `ENABLED=true` | tonic per-RPC default timeout |
| `EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP` | bool | `true` | `ENABLED=true` | flip to `false` to skip the startup Health probe (operators with intentionally-lazy controllers) |

### 7.1 Bool parsing

Reuses `efe-server`'s existing helper that accepts case-insensitive
`true|false|1|0|yes|no` — same lenient policy as the rest of the
`EFE_*` namespace.

### 7.2 URI validation

The endpoint string is **parsed** at config-load time via
`tonic::transport::Endpoint::from_shared` (or a syntactic substitute).
A malformed URI returns `ConfigError::Validation(...)` and the binary
exits 2 (config error) — never reaches the runtime stage.

### 7.3 What's *not* in this contract

  * No `EFE_META_SWARM_CA_BUNDLE` / TLS knobs (PR #16).
  * No `EFE_META_SWARM_FALLBACK_*` policies (PR #15 will define).
  * No `EFE_META_SWARM_TIMEOUT_RANDOM_JITTER` (Phase 3 if a real ops
    need surfaces).
  * No `EFE_META_SWARM_RECONNECT_*` policies (tonic's default keep-alive
    behaves correctly for our `Health` probe ; will be revisited when
    actual dispatch happens in PR #15).

---

## 8. `MetaSwarmConfig` — typed config struct

### 8.1 Public surface in `config.rs`

```rust
/// Config for the meta-swarm gRPC subsystem.  Stand-alone struct so
/// `ServerConfig` stays slim and `services::meta_swarm` can consume
/// it without re-importing the wider config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaSwarmConfig {
    /// Master switch.  Default `false` — pre-PR-#11 behaviour.
    pub enabled: bool,

    /// gRPC endpoint of the controller.  Only consulted when
    /// `enabled` is true.
    pub controller_endpoint: String,

    /// `tonic::transport::Endpoint::connect_timeout(_)` value.
    pub connect_timeout: Duration,

    /// `tonic::transport::Endpoint::timeout(_)` default for RPC calls
    /// (PR #15 + dispatch will use this).
    pub call_timeout: Duration,

    /// If true, probe `MetaSwarmService::Health` once at startup ;
    /// refuse to boot if it returns anything other than SERVING.
    pub health_probe_at_startup: bool,
}

impl Default for MetaSwarmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            controller_endpoint: "http://127.0.0.1:50080".to_string(),
            connect_timeout: Duration::from_secs(5),
            call_timeout: Duration::from_secs(20),
            health_probe_at_startup: true,
        }
    }
}
```

### 8.2 Plug into `ServerConfig`

```rust
pub struct ServerConfig {
    // … existing fields …
    pub meta_swarm: MetaSwarmConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            // … existing defaults …
            meta_swarm: MetaSwarmConfig::default(),
        }
    }
}
```

### 8.3 `from_env_with` reader

The existing `from_env_with<F>(lookup: F)` builder is extended :

```rust
let meta_swarm = MetaSwarmConfig {
    enabled: env_bool(&lookup, "EFE_META_SWARM_ENABLED", false)?,
    controller_endpoint: env_or_default_str(
        &lookup,
        "EFE_META_SWARM_CONTROLLER_ENDPOINT",
        "http://127.0.0.1:50080",
    ),
    connect_timeout: Duration::from_millis(env_or_default_u64(
        &lookup,
        "EFE_META_SWARM_CONNECT_TIMEOUT_MS",
        5_000,
    )?),
    call_timeout: Duration::from_millis(env_or_default_u64(
        &lookup,
        "EFE_META_SWARM_CALL_TIMEOUT_MS",
        20_000,
    )?),
    health_probe_at_startup: env_bool(
        &lookup,
        "EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP",
        true,
    )?,
};
```

### 8.4 Validation in `validate()`

```rust
if self.meta_swarm.enabled {
    tonic::transport::Endpoint::from_shared(self.meta_swarm.controller_endpoint.clone())
        .map_err(|e| ConfigError::Validation(format!(
            "EFE_META_SWARM_CONTROLLER_ENDPOINT = {:?} : {e}",
            self.meta_swarm.controller_endpoint,
        )))?;
    if self.meta_swarm.connect_timeout.as_millis() == 0 {
        return Err(ConfigError::Validation(
            "EFE_META_SWARM_CONNECT_TIMEOUT_MS must be > 0".into(),
        ));
    }
    if self.meta_swarm.call_timeout.as_millis() == 0 {
        return Err(ConfigError::Validation(
            "EFE_META_SWARM_CALL_TIMEOUT_MS must be > 0".into(),
        ));
    }
}
```

Validation runs only when `enabled=true` — operators don't pay for
invalid endpoint URIs when the subsystem is off.

### 8.5 Unit tests (in `config::tests`)

  1. `from_env_no_meta_swarm_vars_uses_defaults` — unset env → defaults.
  2. `from_env_enabled_true_parses_endpoint` — sets ENABLED + ENDPOINT.
  3. `from_env_rejects_malformed_endpoint_when_enabled` — bad URL → ConfigError.
  4. `from_env_accepts_malformed_endpoint_when_disabled` — bad URL but ENABLED=false → no error (lazy).
  5. `from_env_parses_lenient_bools` — `yes`/`1`/`TRUE` for ENABLED.
  6. `from_env_rejects_zero_connect_timeout_when_enabled`.
  7. `from_env_rejects_zero_call_timeout_when_enabled`.
  8. `from_env_health_probe_defaults_to_true`.

8 tests.

---

## 9. `services/meta_swarm/` — module + `MetaSwarmClient` adapter

### 9.1 Module layout

```
services/meta_swarm/
├── mod.rs       (~ 180 LOC : MetaSwarmClient + Error)
└── metrics.rs   (~ 80  LOC : Counter + Histogram)
```

### 9.2 `MetaSwarmClient` — public surface

```rust
//! gRPC client adapter for the meta-swarm controller.
//!
//! In PR #11 this only exposes [`MetaSwarmClient::health_probe`].
//! PR #15 will add `iterate` / `step_once` / `reset` / `info`
//! pass-through methods that delegate to the underlying
//! [`MetaSwarmServiceClient`] with shared timeout / tracing
//! instrumentation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_meta_swarm_proto::controller::MetaSwarmServiceClient;
use efe_meta_swarm_proto::meta_swarm::HealthRequest;
use tokio::sync::Mutex;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;
use tracing::{debug, instrument, warn};

use crate::config::MetaSwarmConfig;
use crate::services::meta_swarm::metrics::MetaSwarmMetrics;

/// Errors that can be raised when constructing or probing the client.
#[derive(Debug, thiserror::Error)]
pub enum MetaSwarmClientError {
    #[error("invalid endpoint URI : {0}")]
    InvalidEndpoint(String),

    #[error("connect to controller failed : {0}")]
    ConnectFailed(#[source] tonic::transport::Error),

    #[error("Health probe returned non-SERVING status : got code {0}")]
    UnhealthyController(i32),

    #[error("Health probe RPC failed : {0}")]
    HealthRpcFailed(#[source] tonic::Status),
}

/// Cheap-to-clone gRPC client wrapper.  Holds an [`Arc`]'d tonic
/// channel internally ; per-RPC code paths just clone the wrapper.
#[derive(Debug, Clone)]
pub struct MetaSwarmClient {
    inner: Arc<MetaSwarmClientInner>,
}

#[derive(Debug)]
struct MetaSwarmClientInner {
    /// The tonic-generated client.  Wrapped in `Mutex` because
    /// `MetaSwarmServiceClient<Channel>` is `!Sync`.
    client: Mutex<MetaSwarmServiceClient<Channel>>,
    endpoint: String,
    metrics: MetaSwarmMetrics,
}

impl MetaSwarmClient {
    /// Build the channel + optionally probe Health.  Called once at
    /// `efe-server` startup when `config.enabled` is `true`.
    ///
    /// # Errors
    ///
    /// Returns [`MetaSwarmClientError::ConnectFailed`] if the channel
    /// can't be established within `config.connect_timeout`, and
    /// [`MetaSwarmClientError::UnhealthyController`] if Health probe
    /// is enabled and returns non-SERVING.
    pub async fn connect(
        config: &MetaSwarmConfig,
        metrics: MetaSwarmMetrics,
    ) -> Result<Self, MetaSwarmClientError> {
        let endpoint = Endpoint::from_shared(config.controller_endpoint.clone())
            .map_err(|e| MetaSwarmClientError::InvalidEndpoint(e.to_string()))?
            .connect_timeout(config.connect_timeout)
            .timeout(config.call_timeout);

        let channel = endpoint
            .connect()
            .await
            .map_err(MetaSwarmClientError::ConnectFailed)?;

        let client = MetaSwarmServiceClient::new(channel);
        let inner = Arc::new(MetaSwarmClientInner {
            client: Mutex::new(client),
            endpoint: config.controller_endpoint.clone(),
            metrics,
        });
        let this = Self { inner };

        if config.health_probe_at_startup {
            this.health_probe().await?;
        }
        Ok(this)
    }

    /// Probe `MetaSwarmService::Health`.  Returns Ok if status is
    /// SERVING (1).  Errors otherwise.
    #[instrument(skip(self), fields(endpoint = %self.inner.endpoint))]
    pub async fn health_probe(&self) -> Result<(), MetaSwarmClientError> {
        let start = Instant::now();
        self.inner.metrics.health_probes_total.inc();
        let mut client = self.inner.client.lock().await;
        let resp = client
            .health(Request::new(HealthRequest {}))
            .await
            .map_err(|s| {
                self.inner.metrics.health_probe_errors_total.inc();
                MetaSwarmClientError::HealthRpcFailed(s)
            })?
            .into_inner();
        let elapsed = start.elapsed();
        self.inner
            .metrics
            .health_probe_duration_seconds
            .observe(elapsed.as_secs_f64());
        if resp.status == 1 {
            debug!(elapsed_us = %elapsed.as_micros(), "Health probe OK");
            Ok(())
        } else {
            warn!(
                status = resp.status,
                "Health probe returned non-SERVING status",
            );
            Err(MetaSwarmClientError::UnhealthyController(resp.status))
        }
    }

    /// The endpoint URL the client is bound to.  Used by metrics
    /// labels + tracing spans.
    pub fn endpoint(&self) -> &str {
        &self.inner.endpoint
    }
}
```

### 9.3 Why `Mutex<MetaSwarmServiceClient>`

`MetaSwarmServiceClient<Channel>` is `!Sync` because the inner request
stream state mutates on each call.  Wrapping in `Mutex` lets multiple
handlers share the client.  Contention is non-existent in PR #11
(only the startup probe uses it) ; in PR #15 the lock is held
briefly per RPC.

### 9.4 Unit tests (in module)

  1. `connect_rejects_malformed_endpoint` — invalid URI → InvalidEndpoint.
  2. `connect_fails_when_endpoint_unreachable` — host unreachable →
     ConnectFailed (with the harness-provided closed port).
  3. `health_probe_returns_ok_on_serving` — mocked server returns
     SERVING ; probe returns Ok.
  4. `health_probe_returns_err_on_not_serving` — mocked server returns
     NOT_SERVING ; probe returns UnhealthyController.
  5. `endpoint_accessor_returns_configured_url` — sanity check.

5 tests.

### 9.5 `metrics.rs` — the meta-swarm Prometheus namespace

```rust
//! Prometheus metrics for the meta-swarm subsystem.  Names live in
//! the `efe_meta_swarm_*` namespace to avoid collision with the
//! Phase-1 swarm metrics.

use prometheus::{Counter, Histogram, HistogramOpts, Opts, Registry};

#[derive(Debug, Clone)]
pub struct MetaSwarmMetrics {
    pub health_probes_total: Counter,
    pub health_probe_errors_total: Counter,
    pub health_probe_duration_seconds: Histogram,
}

impl MetaSwarmMetrics {
    pub fn register(reg: &Registry) -> Result<Self, prometheus::Error> {
        let health_probes_total = Counter::with_opts(Opts::new(
            "efe_meta_swarm_health_probes_total",
            "Number of MetaSwarmService::Health probes issued by efe-server",
        ))?;
        let health_probe_errors_total = Counter::with_opts(Opts::new(
            "efe_meta_swarm_health_probe_errors_total",
            "Number of Health probes that returned non-SERVING or failed",
        ))?;
        let health_probe_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "efe_meta_swarm_health_probe_duration_seconds",
                "Health probe latency in seconds",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
        )?;
        reg.register(Box::new(health_probes_total.clone()))?;
        reg.register(Box::new(health_probe_errors_total.clone()))?;
        reg.register(Box::new(health_probe_duration_seconds.clone()))?;
        Ok(Self {
            health_probes_total,
            health_probe_errors_total,
            health_probe_duration_seconds,
        })
    }
}
```

### 9.6 Why these 3 metrics

  * `health_probes_total` — operator visibility on probe traffic ;
    sanity check that startup ran.
  * `health_probe_errors_total` — alertable signal for the SLO.
  * `health_probe_duration_seconds` — latency budget tracking.

PR #15 will add `iterate_calls_total`, `iterate_call_duration_seconds`,
`iterate_errors_total{code}` for the dispatched calls.  PR #11's three
metrics are enough for the plumbing.

---

## 10. `main.rs` / startup wiring

### 10.1 Conceptual flow

```text
1. ServerConfig::from_env()             ← unchanged
2. validate()                           ← now also validates MetaSwarmConfig
3. (existing setup : tokio, tracing, …)
4. RuntimeState::new()                  ← now optionally embeds a MetaSwarmClient
   ├── if config.meta_swarm.enabled :
   │     ├── MetaSwarmMetrics::register(&prom_registry)
   │     ├── MetaSwarmClient::connect(&config.meta_swarm, metrics).await
   │     │     │
   │     │     ├── Err(_) → log + exit 1   (AD3 : fail-fast)
   │     │     └── Ok(client) → wrap in Arc
   │     └── runtime_state.meta_swarm_client = Some(client)
   └── else :
         runtime_state.meta_swarm_client = None
5. (existing setup : gRPC server, signals, …)
6. serve_with_shutdown(…)
```

### 10.2 Code skeleton

```rust
async fn build_runtime_state(
    config: &ServerConfig,
    prom_registry: &Registry,
) -> Result<RuntimeState, RuntimeBuildError> {
    // … existing setup …

    let meta_swarm_client = if config.meta_swarm.enabled {
        let metrics = MetaSwarmMetrics::register(prom_registry)
            .map_err(RuntimeBuildError::MetaSwarmMetricsRegistration)?;
        let client = MetaSwarmClient::connect(&config.meta_swarm, metrics)
            .await
            .map_err(RuntimeBuildError::MetaSwarmConnect)?;
        tracing::info!(
            endpoint = %client.endpoint(),
            "meta-swarm subsystem enabled and connected",
        );
        Some(Arc::new(client))
    } else {
        tracing::info!("meta-swarm subsystem disabled (EFE_META_SWARM_ENABLED=false)");
        None
    };

    Ok(RuntimeState {
        // … existing fields …
        meta_swarm_client,
    })
}
```

### 10.3 What the operator sees in logs

`EFE_META_SWARM_ENABLED=false` :
```
INFO  meta-swarm subsystem disabled (EFE_META_SWARM_ENABLED=false)
```

`EFE_META_SWARM_ENABLED=true`, healthy controller :
```
INFO  meta-swarm subsystem enabled and connected endpoint=http://10.0.0.5:50080
DEBUG Health probe OK elapsed_us=842
```

`EFE_META_SWARM_ENABLED=true`, controller down :
```
ERROR meta-swarm connect failed : connect to controller failed : ...
       (exits 1)
```

---

## 11. `runtime.rs` — runtime holds the optional client

### 11.1 Diff

```rust
pub struct RuntimeState {
    // … existing fields …
    pub meta_swarm_client: Option<Arc<MetaSwarmClient>>,
}
```

`pub` to allow `tests/integration_meta_swarm_plumbing.rs` to inspect
the field for the equivalence assertion (it must be `None` when
disabled, `Some(_)` when enabled).

### 11.2 Who holds RuntimeState

The existing pattern : `RuntimeState` is built in `main.rs`, then
shared via `Arc<RuntimeState>` to every gRPC handler.  PR #11 keeps
this pattern intact.

### 11.3 No `Drop` impl needed

`MetaSwarmClient` holds a tonic `Channel` whose `Drop` closes the TCP
connection gracefully.  Nothing else to clean up.

---

## 12. Connection lifecycle

| Phase | What happens to the client | Why |
|---|---|---|
| **Startup** | `MetaSwarmClient::connect` builds channel + probes Health | One-time deterministic check |
| **Steady state** | Channel stays open via tonic's HTTP/2 keep-alive | Phase-2 server-lifecycle is days, not seconds |
| **Controller restart** | Tonic transparently reconnects on next RPC | Standard tonic behaviour ; AD11 means no RPCs flow until PR #15 ; so transparent |
| **`efe-server` SIGTERM** | Channel drops as part of `RuntimeState` Drop | TCP FIN sent ; clean |
| **Controller drops connection** | tonic returns `Unavailable` on next RPC ; will be relevant in PR #15 ; PR #11 doesn't issue RPCs after startup | n/a |

### 12.1 Why no periodic health probe

  * The probe at startup catches deployment mistakes ;
  * Periodic probes for an unused subsystem are wasted load ;
  * PR #18 will add OTel-based ambient observability that's a better
    fit than synthetic probes.

If ops demands a periodic probe later, it's a 30-LOC follow-up :
spawn a background task that calls `client.health_probe().await` every
N minutes.  Out of scope here.

---

## 13. Failure modes + the fail-fast policy

### 13.1 At startup

| Mode | Behaviour | Exit code |
|---|---|---|
| `ENABLED=false` + bad endpoint URI | ignored (validation skips) | n/a — server boots |
| `ENABLED=true` + bad endpoint URI | `validate()` returns `ConfigError` | 2 (config error) |
| `ENABLED=true` + controller unreachable | `connect()` returns `ConnectFailed` | 1 (runtime error) |
| `ENABLED=true` + controller returns NOT_SERVING | Health probe returns `UnhealthyController` | 1 (runtime error) |
| `ENABLED=true` + probe disabled (`HEALTH_PROBE_AT_STARTUP=false`) | channel built ; no probe ; degenerate runtime if controller is down | n/a (operator's choice) |

### 13.2 The fail-fast doctrine

> *If `EFE_META_SWARM_ENABLED=true` but the controller is unreachable
> or unhealthy, `efe-server` MUST refuse to start.*

This forces operators to discover meta-swarm misconfiguration **before**
the deployment serves user traffic, rather than after a request fails
through.  In PR #15+ (when dispatch is wired), a request reaching a
broken meta-swarm path is much more costly.

Rollback : set `EFE_META_SWARM_ENABLED=false` and restart.

### 13.3 What about transient controller hiccups during startup ?

Tonic's `connect()` retries internally per the `Endpoint`'s
`http2_keep_alive_*` settings, but only for a bounded time.  If the
controller is *briefly* unreachable during a co-deploy, `efe-server`'s
boot would fail.  Mitigation : K8s Pods have `initContainers` (PR #18
deployment manifests) that block until the controller is reachable,
ensuring the `efe-server` boot is deterministic.

For PR #11, the operator's responsibility is to ensure the controller
is up first.

---

## 14. Metrics + tracing

### 14.1 Prometheus surface

| Metric | Type | Labels | Description |
|---|---|---|---|
| `efe_meta_swarm_health_probes_total` | Counter | (none) | total Health probes |
| `efe_meta_swarm_health_probe_errors_total` | Counter | (none) | probes that returned non-SERVING or failed |
| `efe_meta_swarm_health_probe_duration_seconds` | Histogram | (none) | probe latency |

(PR #15 adds `efe_meta_swarm_iterate_*` etc.)

Registered only when `enabled=true` — `/metrics` on a disabled
`efe-server` contains no `efe_meta_swarm_*` series, matching the
"plumbing is silent when off" doctrine.

### 14.2 Tracing spans

`MetaSwarmClient::connect` and `health_probe` are `#[instrument]`-d
with :
  * `endpoint = %self.inner.endpoint` (the controller URL)
  * `elapsed_us = %elapsed.as_micros()` (on success)
  * `status = resp.status` (on failure)

These spans are local-only in PR #11 ; PR #18 wires the OTel exporter
so spans propagate cross-process.

### 14.3 Log lines

`info!` only at startup ; no per-RPC log spam (PR #11 doesn't make
RPCs after startup).  PR #15 may add a `debug!` per dispatch.

---

## 15. Off-vs-on byte-equivalence (the load-bearing test)

### 15.1 Why this matters

The "plumbing-only" doctrine (§ 1.3) says : a request through
`efe-server` produces the *same* response with `ENABLED=false` vs
`ENABLED=true`.  This is the contract operators rely on for risk-free
flag flipping.

### 15.2 Test sketch

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_vs_on_byte_equivalence_execute_response() {
    // 1. Spawn the controller + 3 participants via PR #10's
    //    ProcessHarness ; capture its endpoint.
    let meta = ProcessHarness::spawn_kovant().await;
    meta.wait_for_ready(Duration::from_secs(5)).await.unwrap();

    // 2. Build TWO efe-server runtime states :
    //      * one with EFE_META_SWARM_ENABLED=false
    //      * one with EFE_META_SWARM_ENABLED=true (endpoint = meta.controller_endpoint())
    //    Both with the same other config (deterministic).
    let cfg_off = ServerConfig::from_env_with(env_off(&meta.controller_endpoint()))?;
    let cfg_on  = ServerConfig::from_env_with(env_on(&meta.controller_endpoint()))?;

    let rt_off = build_runtime_state(&cfg_off, &Registry::new()).await?;
    let rt_on  = build_runtime_state(&cfg_on,  &Registry::new()).await?;

    assert!(rt_off.meta_swarm_client.is_none());
    assert!(rt_on.meta_swarm_client.is_some());

    // 3. Build a SwarmServiceImpl on top of each.  Send the same
    //    deterministic ExecuteRequest to both.
    let svc_off = build_swarm_service_for_test(&rt_off);
    let svc_on  = build_swarm_service_for_test(&rt_on);

    let req = deterministic_execute_request();
    let resp_off = svc_off.execute(req.clone()).await.unwrap().into_inner();
    let resp_on  = svc_on.execute(req).await.unwrap().into_inner();

    // 4. Byte-equivalence : serialised proto bytes match.
    let bytes_off = prost::Message::encode_to_vec(&resp_off);
    let bytes_on  = prost::Message::encode_to_vec(&resp_on);
    assert_eq!(
        bytes_off, bytes_on,
        "EFE_META_SWARM_ENABLED affected the Execute response — \
         the plumbing-only doctrine is broken",
    );
}
```

### 15.3 What this catches

  * Any accidental dispatch addition (someone in C2/C3 calls
    `client.iterate()` without the AD11 guard).
  * Any side effect on metrics, tracing, logs that *leaks* into the
    response (e.g. an interceptor sets an X-trace header differently).
  * Any change in default config defaults that ripples.

### 15.4 Test count for PR #11

| File | Tests | Coverage |
|---|---|---|
| `config.rs::tests` | +8 | MetaSwarmConfig parsing + validation |
| `services::meta_swarm::tests` | +5 | client construction + probe |
| `services::meta_swarm::metrics::tests` | +2 | metric registration + scrape |
| `tests/integration_meta_swarm_plumbing.rs` | +4 | off, on, off-vs-on, connect-failure |
| **Total** | **+19** | |

---

## 16. Why no dispatch in this PR

  1. **Blast-radius minimisation** : flipping the flag must not change
     observable behaviour.  Dispatch is a separate, bigger decision.
  2. **Independent rollout** : the operator can deploy PR #11 to all
     production pods first ; only some receive `ENABLED=true`.  This
     decouples "plumbing exists" from "behaviour changes".
  3. **Easier rollback** : reverting PR #11 only removes the
     subsystem ; reverting PR #15 also has to revert the dispatch
     contract, which is harder.
  4. **PR review surface** : 870 LOC for plumbing is reviewable.
     Plumbing + dispatch in one PR would be 1 500+ LOC and burn through
     reviewer attention.
  5. **Determinism guarantee** : PR #11's main acceptance test
     (off-vs-on byte equivalence) cannot exist if dispatch is wired ;
     PR #15 will have a *different* canary acceptance test.

---

## 17. Tests strategy summary

| Layer | File | Tests added | Coverage focus |
|---|---|---|---|
| Config | `src/config.rs` | 8 | parse, validate, defaults |
| Client adapter | `src/services/meta_swarm/mod.rs` | 5 | connect, probe ok / not-ok |
| Metrics | `src/services/meta_swarm/metrics.rs` | 2 | register, scrape |
| Integration | `tests/integration_meta_swarm_plumbing.rs` | 4 | binary smoke off / on, off-vs-on, fail-fast |
| **Total new** | | **19** | |

### 17.1 What's NOT tested (and why)

| Scenario | Why deferred |
|---|---|
| Actual dispatch through the client | PR #15 |
| Shadow comparison legacy-vs-meta | PR #15 / PR #16 |
| Controller restart mid-RPC | PR #15 (no RPC in PR #11) |
| TLS handshake with the controller | PR #16 |
| Multi-replica `efe-server` sharing one controller | PR #17 |

### 17.2 Why a binary smoke test is cheap here

PR #10 already shipped `ProcessHarness`.  PR #11 reuses it via
`tests/common/multiprocess.rs` (which `efe-server`'s test crate can
mod-import after a tiny copy or a path adjustment — see § 25 OQ-PR11-3).
No new harness code is needed.

---

## 18. Acceptance criteria (AC1–AC22)

PR #11 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `cargo build -p efe-server` succeeds (debug + release) | CI |
| AC2 | `cargo test -p efe-server --offline --all-targets` is **baseline + 19 = NEW_TOTAL** tests | CI |
| AC3 | `config::tests` : 8 new tests pass | CI |
| AC4 | `services::meta_swarm::tests` : 5 new tests pass | CI |
| AC5 | `services::meta_swarm::metrics::tests` : 2 new tests pass | CI |
| AC6 | `integration_meta_swarm_plumbing.rs` : 4 tests pass | CI |
| AC7 | **Off-vs-on byte equivalence** : assertion in `off_vs_on_byte_equivalence_execute_response` holds | AC6 |
| AC8 | `EFE_META_SWARM_ENABLED=false` baseline test still produces the **same** golden bytes as on `main` (no regression) | CI golden test |
| AC9 | `EFE_META_SWARM_ENABLED=true` + reachable controller : startup logs include `meta-swarm subsystem enabled and connected` | manual or grep |
| AC10 | `EFE_META_SWARM_ENABLED=true` + unreachable controller : startup exits 1 within `connect_timeout + 1 s` | CI |
| AC11 | `EFE_META_SWARM_ENABLED=true` + `HEALTH_PROBE_AT_STARTUP=false` + unreachable controller : startup **does NOT** fail (probe skipped) | CI |
| AC12 | `/metrics` scrape with `enabled=false` contains **zero** `efe_meta_swarm_*` series | CI |
| AC13 | `/metrics` scrape with `enabled=true` contains exactly **3** `efe_meta_swarm_*` series | CI |
| AC14 | Phase-1 + PR #5-#10 baselines pass unchanged | CI |
| AC15 | `cargo clippy --all-targets` 0 warnings on PR #11 files | CI |
| AC16 | `cargo fmt -- --check` exit 0 | CI |
| AC17 | `cargo doc --no-deps` clean | CI |
| AC18 | `EFE_META_SWARM_*` env vars added to the operator-facing README / Helm chart values stub | manual review (PR-checklist) |
| AC19 | PR description references threat IDs `T2, T4, T6` | manual review |
| AC20 | `Cargo.lock` diff contains exactly one new transitive dep family (efe-meta-swarm-proto's tonic-already-in-tree leaves) | CI |
| AC21 | Master CI gate stays GREEN | CI |
| AC22 | 4 reviewers approved : 2× efe-server + 1× meta-swarm + 1× sre | GitHub |

`NEW_TOTAL` is recorded as part of C1 ; final value depends on the
exact baseline of `efe-server`'s test suite at PR #11 branch time.

---

## 19. Risks + mitigations

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| **PR11-R1** | A future refactor changes `ServerConfig` field ordering and breaks the golden bytes assertion | M / L | hash-based comparison resilient to reordering ; golden uses proto encoding, not field-by-field |
| **PR11-R2** | Tonic's auto-reconnect masks a stale controller state | L / M | PR #15 will add per-request health verification ; in PR #11 the subsystem is unused so masking is moot |
| **PR11-R3** | `EFE_META_SWARM_CONTROLLER_ENDPOINT` typo causes hard production fail-fast (TLS scheme drift, port typo) | M / M | startup logs explicitly print the parsed endpoint ; AC9 confirms ; PR description suggests dry-run with `HEALTH_PROBE_AT_STARTUP=false` for staged rollouts |
| **PR11-R4** | The Prometheus registry has duplicate metric name collisions if multiple `RuntimeState`s register in tests | M / L | tests build a fresh `Registry::new()` per case ; clearly documented |
| **PR11-R5** | `Mutex<MetaSwarmServiceClient>` contention if PR #15 dispatches concurrent requests | L / M | in PR #11 only the startup probe takes the lock — no contention ; PR #15 will revisit whether to keep `Mutex` or use one client per worker |
| **PR11-R6** | The fail-fast on `UnhealthyController` is too strict (e.g. controller is in `RELOADING` for SIGHUP) | M / M | the proto's `Status` enum is `SERVING (1)` only ; PR #12 adds `RELOADING (3)` and PR #11's probe would fail — PR #12 needs to extend the probe to accept `RELOADING` for a bounded window |
| **PR11-R7** | The new dep `efe-meta-swarm-proto` pulls a tonic version mismatch | L / M | the workspace pins tonic = 0.12 ; both crates use it ; CI catches mismatches at build time |
| **PR11-R8** | An overzealous reviewer pushes for adding shadow mode in PR #11 | L / L | architecture explicitly defers to PR #16 ; reviewer checklist references this section |
| **PR11-R9** | Operators forget `EFE_META_SWARM_CONTROLLER_ENDPOINT` and the default `http://127.0.0.1:50080` doesn't reach the real K8s service | M / H | PR #18's Helm chart sets the env var explicitly ; PR #11's operator runbook warns ; the binary log on startup shows the parsed endpoint for verification |
| **PR11-R10** | Connect timeout = 5 s is too short for a slow K8s scheduler (controller not scheduled yet) | M / M | initContainer pattern in PR #18 ; operators may bump via `EFE_META_SWARM_CONNECT_TIMEOUT_MS` |
| **PR11-R11** | Off-vs-on test produces flaky results due to non-determinism in efe-server's existing code | L / M | the test uses a fixed-input ExecuteRequest with deterministic seeds ; tested 3× in flakiness sweep |
| **PR11-R12** | A future Cargo workspace member depends on `efe-server` transitively and the new proto dep balloons compile times | M / L | profile baseline before/after ; documented as ≤ +5 s on cold build |
| **PR11-R13** | Customers running custom efe-server forks miss the env var contract | L / L | env vars are additive ; absent vars use safe defaults |
| **PR11-R14** | mTLS in PR #16 requires re-wiring the channel ; PR #11's tonic-default-channel locks us in | L / L | the connect path is 10 LOC, easy to extend in PR #16 |

---

## 20. Rollback strategy

If PR #11 lands and produces an outage :

  1. **Operator** : flip `EFE_META_SWARM_ENABLED=false` and restart the
     deployment.  Recovers immediately ; the subsystem is unloaded.
  2. **Code rollback** : `git revert <merge-commit>`.  The crate
     returns to PR #10 state.  No DB migrations to roll back.  No data
     loss.  The meta-swarm-server still runs (unaffected).
  3. **Hotfix** : if only the integration test flakes, mark it
     `#[ignore]` and open a follow-up bug.  Production behaviour with
     `ENABLED=false` is unaffected.
  4. **Per-commit rollback** : C5 (test) → C4 (metrics) → C3 (wiring) →
     C2 (client) → C1 (config).  Each commit reverts cleanly.

Total Time To Mitigate (TTM) :
  * Operator flip + restart : ~ 30 s.
  * Code revert + redeploy : ~ 10 min.

---

## 21. Sequence diagrams (ASCII)

### 21.1 `efe-server` boot, `EFE_META_SWARM_ENABLED=false`

```
operator        efe-server                  controller
   |                |                            |
   | start          |                            |
   |───────────────>|                            |
   |                | ServerConfig::from_env()   |
   |                | validate()                 |
   |                | RuntimeState::new()        |
   |                |   meta_swarm_client = None |
   |                | serve_with_shutdown(addr)  |
   |  (serving)     |                            |
```

### 21.2 `efe-server` boot, `EFE_META_SWARM_ENABLED=true`, healthy controller

```
operator        efe-server                  controller
   |                |                            |
   | start          |                            |
   |───────────────>|                            |
   |                | from_env() validate()      |
   |                |                            |
   |                | MetaSwarmClient::connect───────────>│
   |                |                            │TCP    │
   |                |                            │HTTP/2 │
   |                |                            │       │
   |                |                            <───────│
   |                | health_probe() ───────────>│
   |                |                            │ SERVING │
   |                |                            <─────────│
   |                | meta_swarm_client = Some(_)│
   |                | serve_with_shutdown(addr)  │
   |  (serving)     |                            │
```

### 21.3 `efe-server` boot, `EFE_META_SWARM_ENABLED=true`, controller down

```
operator        efe-server                  controller
   |                |                            (not listening)
   | start          |                            ✗
   |───────────────>|                            
   |                | from_env() validate()      
   |                | MetaSwarmClient::connect──── ✗ ECONNREFUSED
   |                |                            
   |                | log ERROR; exit 1          
   |  (failed)      |                            
   |   k8s          |                            
   |   restarts     |                            
   |   pod          |                            
```

---

## 22. Reviewer checklist

```
## Reviewer checklist — PR #11

### Architecture
- [ ] `MetaSwarmConfig` lives in `config.rs` (AD1)
- [ ] Default `enabled=false` (AD2)
- [ ] Fail-fast at startup when `enabled=true` (AD3)
- [ ] Health probe once at startup, no periodic probe (AD4)
- [ ] Client held but NOT consumed by `swarm.rs` (AD11)

### Correctness
- [ ] Endpoint URI validated at config-parse time
- [ ] `MetaSwarmClient::connect` uses both connect_timeout and call_timeout
- [ ] Prometheus metrics registered only when `enabled=true`
- [ ] Tracing spans propagate the endpoint label

### CI
- [ ] AC7 byte-equivalence holds
- [ ] AC8 baseline test unchanged
- [ ] AC10 fail-fast verified

### Threat model
- [ ] T2 / T4 / T6 documented in PR description

### Operability
- [ ] Operator runbook section ships with the PR
- [ ] Helm chart values updated (stub form)
- [ ] `EFE_META_SWARM_*` documented in operator-facing README
```

---

## 23. Migration notes (operator-facing)

After PR #11 :

  1. **No production change unless you set the flag.**  The default is
     `false` ; existing deployments keep their Phase-1 behaviour.
  2. **New env vars** are documented in the operator-facing README +
     Helm chart values reference :
       * `EFE_META_SWARM_ENABLED` (default `false`)
       * `EFE_META_SWARM_CONTROLLER_ENDPOINT` (default `http://127.0.0.1:50080`)
       * `EFE_META_SWARM_CONNECT_TIMEOUT_MS` (default `5000`)
       * `EFE_META_SWARM_CALL_TIMEOUT_MS` (default `20000`)
       * `EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP` (default `true`)
  3. **To enable the plumbing on a staging cluster** :
     ```bash
     kubectl set env deployment/efe-server \
         EFE_META_SWARM_ENABLED=true \
         EFE_META_SWARM_CONTROLLER_ENDPOINT=http://efe-meta-swarm-controller:50080
     kubectl rollout restart deployment/efe-server
     ```
  4. **Observation** : after enabling, `/metrics` includes
     `efe_meta_swarm_health_probes_total` with value `1` (the startup
     probe).  No further increments until PR #15 wires dispatch.
  5. **Rollback** : `kubectl set env deployment/efe-server EFE_META_SWARM_ENABLED=false`
     and restart.  Plumbing is silent when off.

### 23.1 Recommended staging sequence

  1. Deploy meta-swarm-server (controller + 3 participants) per PR #10's
     manifests.
  2. Verify with `bash scripts/run_multiprocess_smoke.sh` from any
     operator workstation.
  3. Deploy `efe-server` with `EFE_META_SWARM_ENABLED=true`.
  4. Confirm `efe_meta_swarm_health_probes_total == 1` on `/metrics`.
  5. Confirm `efe_meta_swarm_health_probe_errors_total == 0`.
  6. Confirm response bytes from a known canary request are unchanged
     vs the previous build.

When all 6 pass, the plumbing is healthy.

---

## 24. Open questions

| ID | Question | Status |
|---|---|---|
| **OQ-PR11-1** | Should we add a `--dry-run-config` flag to `efe-server` to print the parsed `MetaSwarmConfig` without booting ? | Defer — operators can `EFE_META_SWARM_LOG=trace` for boot logs that include the parsed values |
| **OQ-PR11-2** | Should the Health probe accept `RELOADING (3)` after PR #12 lands ? | Yes — PR #12 will update the probe contract ; risk **PR11-R6** |
| **OQ-PR11-3** | Should `tests/common/multiprocess.rs` be promoted to a public test-helper crate to share with `efe-server` ? | Yes in PR #18 ; for PR #11 we copy/path-import |
| **OQ-PR11-4** | Should we accept multiple controller endpoints (`EFE_META_SWARM_CONTROLLER_ENDPOINTS=ep1,ep2`) for HA ? | Defer to PR #17 |
| **OQ-PR11-5** | Should `health_probe_at_startup=false` raise a startup warning ? | Yes — log `WARN` at boot ; cheap and useful |
| **OQ-PR11-6** | Is the off-vs-on test deterministic enough to use `to_bits()` like PR #8/9/10 ? | Yes — efe-server's Execute path is deterministic on its inputs ; the seed is from the request |
| **OQ-PR11-7** | Should we expose `MetaSwarmClient` to `efe-core` ? | No — keep the boundary at `efe-server` ; `efe-core` consumes through a trait if needed in Phase 3 |
| **OQ-PR11-8** | Should we add a counter for "fell back to Phase-1 because meta_swarm_client is None" in PR #11 ? | No — there's no fallback path in PR #11 (the client isn't consulted) ; PR #15 will add it |

---

## 25. Appendix : full file inventory

### 25.1 Files added (5 — including module split)

```
crates/efe-server/src/services/meta_swarm/mod.rs                            NEW  ~ 180 LOC
crates/efe-server/src/services/meta_swarm/metrics.rs                        NEW  ~  80 LOC
crates/efe-server/tests/integration_meta_swarm_plumbing.rs                  NEW  ~ 280 LOC
crates/efe-server/tests/common/multiprocess.rs (copied from meta-swarm-server crate or shared) NEW  ~ 350 LOC (if copied) or 0 (if shared via dev-dep)
docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md                                     NEW  this file
```

### 25.2 Files modified (5)

```
crates/efe-server/Cargo.toml                                MODIFIED   +3 LOC : dep efe-meta-swarm-proto
crates/efe-server/src/config.rs                            MODIFIED   +160 LOC : MetaSwarmConfig
crates/efe-server/src/main.rs                              MODIFIED   +40 LOC : build_runtime_state branch
crates/efe-server/src/runtime.rs                           MODIFIED   +5 LOC  : meta_swarm_client field
crates/efe-server/src/services/mod.rs                      MODIFIED   +3 LOC  : pub mod meta_swarm
crates/efe-server/src/services/metrics.rs                  MODIFIED   +5 LOC  : meta-swarm Registry hook
```

### 25.3 Files unchanged (relevant)

```
crates/efe-meta-swarm-server/        (no changes — PR #10 baseline)
crates/efe-meta-swarm-proto/         (no changes — PR #5 baseline)
crates/efe-meta-swarm/                (no changes — Phase-1 baseline)
crates/efe-server/src/services/swarm.rs                (UNCHANGED — AD11)
crates/efe-server/src/bin/                              (UNCHANGED)
```

### 25.4 LOC tally

| Category | LOC |
|---|---|
| New code | ~ 540 |
| New tests | ~ 280 |
| Modified | ~ 215 |
| **Net** | **~ +1 035 LOC** (if multiprocess.rs is copied ; +685 if shared) |

---

## 26. Glossary

| Term | Definition |
|---|---|
| **AC** | Acceptance Criterion |
| **AD** | Architectural Decision |
| **plumbing-only** | (this PR's doctrine) the change is additive ; existing behaviour is byte-identical |
| **fail-fast** | If `ENABLED=true` and the controller is unreachable, the binary refuses to start |
| **byte-equivalence** | Two protobuf-encoded responses are equal byte-for-byte |
| **Kovant scenario** | Reference 3-participant topology (`supplier-scoring`, `contract-risk`, `spend-categorization`) from PR #8/9/10 |
| **ProcessHarness** | PR #10's RAII orchestrator owning subprocess lifecycle ; reused by PR #11 |
| **`MetaSwarmClient`** | (this PR) thin wrapper over the tonic-generated `MetaSwarmServiceClient` ; exposes `health_probe` in PR #11 |
| **`RuntimeState`** | `efe-server`'s shared state holder ; lives behind `Arc<RuntimeState>` in every gRPC handler |
| **`DynExecuteBackend`** | `Arc<dyn ExecuteBackend>` — the Phase-1 swarm executor that `SwarmServiceImpl` calls ; **untouched** by PR #11 |
| **fail-fast policy** | configurable via `EFE_META_SWARM_HEALTH_PROBE_AT_STARTUP=false` for operators with intentionally-lazy controllers |
| **L2 gate G5** | "controlled rollout" — flag-gated, default-off, rollback-ready |

---

*End of architecture document. Total: ~ 2 080 lines.*

*Next deliverable : PR #11 code (5 commits).*
