# PR #5 — `efe-meta-swarm-proto` scaffold — Architecture ultra-détaillée

> **Document type** : Execution-ready implementation blueprint for PR #5.
> **Scope** : The very first PR of Phase 2 (Sprint A1.P2.1, Week 1, day 1-3).
> **Owner** : meta-swarm-team — assigned lead : J. Tan (back-up : M. Okafor).
> **Reviewers (required)** : 1 meta-swarm-team + 1 sec-team + 1 platform-team.
> **Estimate** : 3 FTE-days (engineering) + 1 FTE-day (review) = 4 FTE-days.
> **Branch** : `A1.P2/05-meta-swarm-proto-scaffold`.
> **Deadline** : Wed 2026-07-08 23:59 UTC (so PR #6 and #8 can start Thu).
> **Parent docs** : [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 4 ; [`EFE_A1_PHASE2_ROADMAP.md`](EFE_A1_PHASE2_ROADMAP.md) § 2.1.

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Goal + non-goals](#1-goal--non-goals)
2. [Pre-flight checks](#2-pre-flight-checks)
3. [Branch + commit plan](#3-branch--commit-plan)
4. [Workspace integration](#4-workspace-integration)
5. [Crate layout](#5-crate-layout)
6. [`Cargo.toml` — full](#6-cargotoml-full)
7. [`build.rs` — full](#7-buildrs-full)
8. [Proto file #1 — `messages.proto`](#8-proto-file-1--messagesproto)
9. [Proto file #2 — `errors.proto`](#9-proto-file-2--errorsproto)
10. [Proto file #3 — `service_meta_swarm.proto`](#10-proto-file-3--service_meta_swarmproto)
11. [Proto file #4 — `service_meta_participant.proto`](#11-proto-file-4--service_meta_participantproto)
12. [`src/lib.rs` — full](#12-srclibrs-full)
13. [Generated code expectations](#13-generated-code-expectations)
14. [`README.md` — full](#14-readmemd-full)
15. [Versioning policy](#15-versioning-policy)
16. [Field-number reservation strategy](#16-field-number-reservation-strategy)
17. [Naming conventions](#17-naming-conventions)
18. [Smoke test — cross-crate import](#18-smoke-test--cross-crate-import)
19. [CI validation](#19-ci-validation)
20. [Acceptance criteria (binary)](#20-acceptance-criteria-binary)
21. [PR description template](#21-pr-description-template)
22. [Risk analysis (PR-local)](#22-risk-analysis-pr-local)
23. [Reviewer checklist](#23-reviewer-checklist)
24. [Definition of Done](#24-definition-of-done)
25. [Post-merge follow-ups](#25-post-merge-follow-ups)

---

## 0. TL;DR

PR #5 lands the **wire-protocol foundation** for the entire Phase 2 :

- 1 new workspace crate : `efe-meta-swarm-proto` (~600 LOC including protos).
- 4 `.proto` files (~700 LOC of protobuf with inline documentation).
- 1 `build.rs` (~80 LOC) that drives `tonic-build` with explicit
  attribute injection (serde derives, doc comment forwarding).
- 1 `src/lib.rs` (~120 LOC) of re-exports + module organisation.
- 1 `README.md` (~250 LOC) documenting the versioning + field-reservation policy.

After merge, **any other crate in the workspace** can write :

```rust
use efe_meta_swarm_proto::meta_swarm::{
    MetaSwarmServiceClient,
    MetaParticipantServiceClient,
    IterateRequest,
    IterateResponse,
    /* ... */
};
```

and get fully-typed access to the gRPC surface.

**No service implementation in this PR.**  No business logic.  No tests
beyond a 5-line cross-crate import smoke test.  The PR's purpose is
purely **wire-protocol enablement**.

Critical-path significance : **PRs #6, #8, #9, #11, #20, #21 all depend
on PR #5**.  A late merge of #5 cascades into a ≥ 1 day delay on Phase 2.

---

## 1. Goal + non-goals

### 1.1 Goal (single sentence)

> Land the canonical, frozen-at-v1 Protocol Buffers definitions for the
> `MetaSwarmService` + `MetaParticipantService` gRPC surfaces, generate
> tonic client + server stubs, and expose them as a separate, dependency-
> minimal crate that the rest of the workspace can pull in without
> linking the entire algorithmic surface of `efe-meta-swarm`.

### 1.2 Non-goals

Items deliberately excluded from PR #5 :

| # | Excluded | Lives in | Why |
|---|---|---|---|
| NG1 | Service implementation (any RPC body beyond `todo!()`) | PR #6 + #8 + #9 | Keeps PR #5 small + reviewable. |
| NG2 | Authentication / mTLS code | PR #16 | Auth is a server-side concern ; wire format is auth-agnostic. |
| NG3 | Discovery (static / k8s-dns) | PR #11 | Same reason as NG2. |
| NG4 | Metrics (Prometheus registry) | PR #17 | Same. |
| NG5 | OTel span emission | PR #18 | Same. |
| NG6 | Client adapter `RemoteMetaSwarm` | PR #6 | This is a thin wrapper over the generated client ; ships in the server crate. |
| NG7 | Server adapter `MetaSwarmGrpcServer` | PR #8 | Wraps Phase-1 `MetaSwarm` ; depends on `efe-meta-swarm`. |
| NG8 | Integration test (multi-process) | PR #10 | Needs PR #6, #8, #9 first. |
| NG9 | Helm chart changes | PR #13 | Chart references the binary, not the proto crate. |
| NG10 | Documentation in `EFE_A1_BENCH_TRACKING.md` extension for A2 SLOs | PR #22 | A2 SLOs don't exist yet at PR #5 merge. |

### 1.3 Single binary unit of merge-readiness

PR #5 is mergeable iff a sibling toy crate (created in the PR's
self-contained smoke test) can `use efe_meta_swarm_proto::...` and
`cargo build` succeeds, with **0 warnings on a fresh `cargo check`** and
`cargo doc -p efe-meta-swarm-proto --no-deps` clean.

Anything else (service impl, business logic) is a non-goal for this PR.

---

## 2. Pre-flight checks

Before opening the branch, verify :

| # | Check | How | Expected |
|---|---|---|---|
| PF1 | Phase 2 architecture frozen | `git log -1 docs/EFE_A1_PHASE2_ARCHITECTURE.md` | Last commit before 2026-07-06 |
| PF2 | Phase 2 roadmap frozen | `git log -1 docs/EFE_A1_PHASE2_ROADMAP.md` | id. |
| PF3 | Phase 1 master gate still GREEN on `main` | `gh run list -w ops-master-gate.yml -L 3` | All 3 PASS |
| PF4 | rustc toolchain pinned 1.88 | `cat rust-toolchain.toml` | `channel = "1.88"` |
| PF5 | `tonic-build` version available offline | `ls ~/.cargo/registry/cache/*/tonic-build-0.12*` | present |
| PF6 | `protoc` available (host) | `protoc --version` | ≥ 3.20 — OR rely on the `protoc` bundled by tonic-build (recommended : YES, see § 7) |
| PF7 | OQ1 + OQ2 decided | `docs/A1_PHASE2_DECISIONS.md` | f32 wire (OQ1=f32) ; StepStream optional (OQ2=optional) |

If any PF check fails, the PR is **blocked at preflight**.  Do not
branch ; resolve the upstream issue first.

---

## 3. Branch + commit plan

### 3.1 Branch

```
A1.P2/05-meta-swarm-proto-scaffold
```

(Naming convention : `<sprint>/<PR-number>-<short-slug>`.)

### 3.2 Commit-by-commit plan

The PR ships in **5 commits**, each individually green (CI passes on
each).  Reviewer can step through them in order :

| # | Commit message | Files | LOC ± | Verifies |
|---|---|---|---|---|
| C1 | `chore(workspace): register efe-meta-swarm-proto crate` | `Cargo.toml`, `crates/efe-meta-swarm-proto/Cargo.toml`, `crates/efe-meta-swarm-proto/src/lib.rs` (placeholder), `crates/efe-meta-swarm-proto/README.md` (placeholder) | +120 | `cargo check -p efe-meta-swarm-proto` (placeholder lib builds) |
| C2 | `feat(proto): add messages.proto + errors.proto + build.rs` | +`build.rs`, +`proto/efe/v1/meta_swarm/messages.proto`, +`proto/efe/v1/meta_swarm/errors.proto` | +400 | `cargo build -p efe-meta-swarm-proto` (no services yet, but messages compile) |
| C3 | `feat(proto): add MetaSwarmService` | +`proto/efe/v1/meta_swarm/service_meta_swarm.proto`, +tonic-build wiring | +200 | `cargo build` ; client + server traits exist |
| C4 | `feat(proto): add MetaParticipantService` | +`proto/efe/v1/meta_swarm/service_meta_participant.proto` | +180 | Same as C3, both services exist |
| C5 | `docs(proto): README + versioning policy + smoke test` | `README.md` (full), `tests/smoke_cross_crate.rs` | +280 | `cargo test -p efe-meta-swarm-proto` + `cargo doc` |

Total : ~ **1180 LOC** (incl. proto comments, README, lib.rs, build.rs).

Each commit has a one-paragraph commit body following the workspace
convention (`<type>(<scope>): <summary>` + blank line + rationale).

### 3.3 Drafting strategy

The author opens the PR as a **Draft** after C1, then pushes C2-C5
incrementally.  Reviewers can start reading from C1 ; CI runs on each
push.  PR moves to "Ready for review" once C5 is up.

---

## 4. Workspace integration

### 4.1 `Cargo.toml` (workspace) — diff

Add the new member.  Keep the existing alphabetical / by-area ordering :

```diff
 [workspace]
 resolver = "2"
 members = [
     "crates/efe-core",
     "crates/efe-meta-swarm",
+    "crates/efe-meta-swarm-proto",
     "crates/efe-server",
     # ... (rest unchanged)
 ]
```

And in `[workspace.dependencies]`, add the Phase 2 transport deps so
all member crates use the same pinned versions :

```diff
 [workspace.dependencies]
 # ... existing deps unchanged ...
+
+# === Phase 2 (PR #5+) — gRPC transport ===
+tonic            = { version = "0.12", default-features = false, features = ["codegen", "prost", "tls", "tls-roots", "transport"] }
+tonic-build      = { version = "0.12", default-features = false, features = ["prost", "transport"] }
+prost            = { version = "0.13", default-features = false, features = ["std", "derive"] }
+prost-types      = { version = "0.13", default-features = false, features = ["std"] }
```

**Rationale** :
- `default-features = false` everywhere : minimise transitive deps,
  reduce build time, keep the supply chain auditable.
- `tonic` is pinned at `0.12` (last minor before `0.13`).  Pin until
  Phase 3 (where a deliberate bump happens with sec-team review).
- `prost 0.13` matches `tonic 0.12`'s requirement.
- `tls-roots` : exposes the system root certs (`rustls-native-certs`)
  for outbound TLS validation in dev.  Server-side TLS material comes
  from cert-manager at runtime (cf. PR #16).

### 4.2 `Cargo.lock`

`Cargo.lock` will be updated automatically by `cargo build`.  The PR
**must commit the lockfile diff** to keep CI builds deterministic.
The diff is expected to be ~30 lines (transitive deps of tonic 0.12).

If a reviewer sees more than 100 lockfile lines added, that's a red
flag : something pulled an unexpected transitive — investigate.

---

## 5. Crate layout

The full tree of the new crate :

```
crates/efe-meta-swarm-proto/
├── Cargo.toml                                   # § 6
├── build.rs                                     # § 7
├── README.md                                    # § 14
├── proto/
│   └── efe/
│       └── v1/
│           └── meta_swarm/
│               ├── messages.proto               # § 8
│               ├── errors.proto                 # § 9
│               ├── service_meta_swarm.proto     # § 10
│               └── service_meta_participant.proto  # § 11
├── src/
│   └── lib.rs                                   # § 12
└── tests/
    └── smoke_cross_crate.rs                     # § 18
```

**Why this exact layout** :

- `proto/efe/v1/meta_swarm/` mirrors the protobuf package
  `efe.v1.meta_swarm`.  Makes import paths predictable in any language
  that consumes the protos later (go/python/java/typescript), even
  though Phase 2 only uses Rust.
- `proto/` not `src/proto/` : keeps the protos discoverable for the
  upcoming public proto-distribution work (Phase 3 will ship them as a
  separate buf module).
- 4 separate files (instead of 1 monolithic) : reviewers can scope
  changes to one surface at a time ; tonic-build glues them via
  `import` statements.

**Why NOT** :

- ❌ `src/` containing both `proto/` and `rs/` : confuses cargo's
  default include rules.
- ❌ A single `meta_swarm.proto` file with everything : 700+ LOC, hard
  to review, hard to evolve.
- ❌ Protos under `crates/efe-meta-swarm-proto/src/proto/` : tonic-build
  works but search tools like grep/IDE struggle.

---

## 6. `Cargo.toml` — full

```toml
[package]
name        = "efe-meta-swarm-proto"
version     = "0.2.0"            # Phase-2 minor, since Phase-1 was 0.1.x
edition     = "2021"
rust-version = "1.88"
description = "Protocol Buffers + tonic-generated client/server stubs for the EFE meta-swarm gRPC surface (MetaSwarmService + MetaParticipantService)."
publish     = false              # carve-out R5-B1 ; UNLICENSED until Phase 5
license     = "Apache-2.0"       # advisory only ; not yet effective per R5-B1

# ─────────────────────────────────────────────────────────────────
# Dependencies (runtime).
# ─────────────────────────────────────────────────────────────────
[dependencies]
tonic        = { workspace = true }
prost        = { workspace = true }
prost-types  = { workspace = true }

# Optional : serde derives on generated message types.
# Used by `bench_public/` (PR #21) to emit JSON for the HTML report,
# and by future Phase-3 customer-facing CLI tools.
serde        = { version = "1.0", features = ["derive"], optional = true }

# ─────────────────────────────────────────────────────────────────
# Dependencies (build).
# ─────────────────────────────────────────────────────────────────
[build-dependencies]
tonic-build  = { workspace = true }
prost-build  = { workspace = true }

# ─────────────────────────────────────────────────────────────────
# Features.
# ─────────────────────────────────────────────────────────────────
[features]
# Default : minimal, no JSON.  Most consumers don't need it.
default = []

# Enable serde derives on every generated message.  ~50 kB binary size
# overhead.  Required by `bench_public` + future CLI / SDK.
serde-derives = ["dep:serde"]

# Enable client-only build (no server stubs).  Drops ~30 % of generated
# code for binaries that only consume the surface.  Used by efe-server
# in Phase 2 to keep its size lean.
client-only = []

# Enable server-only build.  Mirror of `client-only`.  Used in builds
# that ship only the controller/participant binaries.
server-only = []

# All-features convenience : everything.  Used by `cargo doc` + CI
# matrix.
full = ["serde-derives"]

# ─────────────────────────────────────────────────────────────────
# Lints — match workspace convention.
# ─────────────────────────────────────────────────────────────────
[lints]
workspace = true

# Note : we explicitly allow some lints that fire on tonic-generated
# code (we don't control its style).  These are *crate-level* allows ;
# downstream crates are unaffected.
[lints.rust]
unsafe_code = "forbid"

[lints.clippy]
# Generated code triggers these — silence at crate level.
module_inception        = "allow"
missing_const_for_fn    = "allow"
needless_pass_by_value  = "allow"
must_use_candidate      = "allow"
return_self_not_must_use = "allow"
unreadable_literal      = "allow"
derive_partial_eq_without_eq = "allow"
large_enum_variant      = "allow"

# ─────────────────────────────────────────────────────────────────
# Documentation hints (rustdoc).
# ─────────────────────────────────────────────────────────────────
[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

### 6.1 Decisions encoded

| Item | Choice | Why |
|---|---|---|
| Package version | `0.2.0` | Phase 2 = minor bump over Phase 1's `0.1.x` line. |
| `publish = false` | yes | Carve-out R5-B1. |
| `license = "Apache-2.0"` advisory | yes | Documents target license post-R5-B1 ; legal review tag remains UNLICENSED. |
| `serde` optional | yes | Most consumers don't need it ; ~50 kB binary size. |
| `client-only` / `server-only` features | yes | Lets efe-server drop server stubs (smaller binary, less attack surface). |
| `default = []` | yes | Smallest surface by default. Consumers opt in. |
| `[lints.clippy]` allow list | yes | tonic-generated code triggers them ; not under our control. |

---

## 7. `build.rs` — full

```rust
//! Build script for `efe-meta-swarm-proto`.
//!
//! Compiles 4 .proto files into Rust client + server stubs via
//! `tonic-build` (which delegates to `prost-build`).  All
//! configuration is explicit — we do not rely on
//! `tonic_build::compile_protos()` defaults — because the resulting
//! code is shipped to production and we want every choice auditable.
//!
//! ## Outputs
//!
//! `OUT_DIR/efe.v1.meta_swarm.rs` — the single generated file
//! containing all proto messages + service traits.  This is included
//! in `src/lib.rs` via `tonic::include_proto!("efe.v1.meta_swarm")`.
//!
//! ## Re-run conditions
//!
//! By default `cargo` re-runs `build.rs` on any change to the
//! `build.rs` itself or any file in the crate.  We emit explicit
//! `rerun-if-changed` for each `.proto` file so a touch to a proto
//! triggers a regeneration but other crate-internal changes don't.

use std::path::PathBuf;

fn main() {
    let proto_dir = PathBuf::from("proto");
    let protos: Vec<PathBuf> = [
        "efe/v1/meta_swarm/messages.proto",
        "efe/v1/meta_swarm/errors.proto",
        "efe/v1/meta_swarm/service_meta_swarm.proto",
        "efe/v1/meta_swarm/service_meta_participant.proto",
    ]
    .iter()
    .map(|p| proto_dir.join(p))
    .collect();

    // Tell cargo to re-run on any proto change.
    for p in &protos {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed=build.rs");

    let mut builder = tonic_build::configure()
        // Where to emit the generated code (default = OUT_DIR ; explicit).
        .out_dir(std::env::var("OUT_DIR").expect("OUT_DIR set by cargo"))
        // Enable / disable client + server based on features.
        // Pattern : `(client-only, server-only) → (build_client, build_server)`.
        .build_client(!cfg!(feature = "server-only"))
        .build_server(!cfg!(feature = "client-only"))
        // Generate descriptive doc comments on every generated item.
        // tonic-build forwards `//` proto comments to rustdoc by default
        // ; this is just defensive.
        .build_transport(true)
        // Strict naming : tonic-build's default is `EfeV1MetaSwarmServiceClient`
        // ; we shorten to `MetaSwarmServiceClient` via package-relative
        // type-name elision (default behaviour ; explicit for clarity).
        .compile_well_known_types(false)
        // Suppress the auto-generated `protoc` include path bin from
        // ending up in the docs.
        .disable_comments(&[]);

    // Inject `serde::{Serialize, Deserialize}` derives on every
    // generated message when the `serde-derives` feature is enabled.
    // We list the types explicitly so future schema additions don't
    // implicitly become serde-able (forces a deliberate decision).
    if cfg!(feature = "serde-derives") {
        let types_with_serde: &[&str] = &[
            "TensorF32",
            "MetaSpaceConfig",
            "BoxSafety",
            "Polyhedron",
            "LearnedHull",
            "ParticipantDescriptor",
            "IterationConfig",
            "IterateRequest",
            "IterateResponse",
            "IterateChunk",
            "StopReason",
            "StepRequest",
            "StepResponse",
            "ResetRequest",
            "ResetResponse",
            "ResetParticipantRequest",
            "ResetParticipantResponse",
            "StateRequest",
            "StateResponse",
            "CapabilitiesRequest",
            "CapabilitiesResponse",
            "ReloadConfigRequest",
            "ReloadConfigResponse",
            "HealthRequest",
            "HealthResponse",
            "InfoRequest",
            "InfoResponse",
            "MetaSwarmErrorCode",
        ];
        for ty in types_with_serde {
            builder = builder.type_attribute(
                ty,
                "#[derive(::serde::Serialize, ::serde::Deserialize)]",
            );
        }
    }

    // Compile.  This is the single fallible step ; failure here is a
    // hard error and aborts the build with a clear diagnostic.
    builder
        .compile(&protos, &[proto_dir])
        .expect("tonic-build : failed to compile .proto files");
}
```

### 7.1 Why each option

| Option | Why |
|---|---|
| Explicit `rerun-if-changed` per proto file | Avoids the default "any source change re-runs" behaviour ; faster incremental builds. |
| `build_client(!server-only)` / `build_server(!client-only)` | Lets consumers drop unused half. |
| `compile_well_known_types(false)` | We do not use `google.protobuf.*` ; skip pulling those in. |
| `protoc` source | Bundled by `tonic-build` 0.12 (uses `protoc-bin-vendored`) — no host `protoc` install required.  Important : CI runners don't have protoc by default. |
| `type_attribute` for serde | Selective : only listed types get serde.  Future additions require an explicit ack. |

### 7.2 Build failure modes

| Symptom | Cause | Fix |
|---|---|---|
| `protoc: command not found` | `tonic-build` could not find / extract protoc | Verify `tonic-build = "0.12"` (NOT `tonic-build = "0.10"`) ; the bundled-protoc feature is on by default ≥ 0.12. |
| `error[E0432]: unresolved import` in `tonic::include_proto!` | tonic-build wrote to a different `OUT_DIR` | Run `cargo clean` ; verify `OUT_DIR` env var is set (always true under cargo). |
| `field number conflict` | Two fields in the same message share a number | Cf. § 16 ; renumber. |
| `package "efe.v1.meta_swarm" does not import "google/..."` | Old `prost-build` insists on resolving well-known | Set `compile_well_known_types(false)`. |

---

## 8. Proto file #1 — `messages.proto`

This file defines the shared messages : tensors, configs, descriptors.
No services, no errors.  Imported by both service files.

```protobuf
// Copyright (C) 2026 EFE.  UNLICENSED — see ../../../README.md.
//
// Phase-2 (Sprint A1) messages : tensors, scenario configs,
// participant descriptors.  Stable from v0.2.0 onwards ; field
// numbers 1-100 are reserved for Phase 2 ; ≥ 100 are reserved for
// Phase 3+ additions.  See `crates/efe-meta-swarm-proto/README.md`
// for the versioning policy.

syntax = "proto3";

package efe.v1.meta_swarm;

// =============================================================================
//  TensorF32 — dense float32 tensor on the wire.
// =============================================================================
//
// Single-precision floats are the Phase-2 default.  Rationale :
//   * The bench scenario (`three_swarms_kovant`) operates in f32 in
//     Phase-1 already ; round-trip via f32 preserves bit identity.
//   * Halves bandwidth vs f64 (256-d × 4 bytes = 1 KiB per state ; vs
//     2 KiB for f64).  Material at 47 iters × 3 participants × 100 rps.
//   * Tenants requiring f64 will opt in via the (yet-to-be-added)
//     `precision` field of `IterationConfig` in Phase 3 ;
//     a `TensorF64` message will be added then.
//
// Layout :
//   - `dim` is the logical dimension.
//   - `data` is row-major, length == dim.  Length mismatch is rejected
//     server-side with META_ERROR_DIM_MISMATCH.
//
// Wire size : 8 bytes (varint dim) + 4 × dim bytes (packed f32) + ~4
// bytes (tag overhead).
message TensorF32 {
  uint32 dim       = 1;
  repeated float data = 2 [packed = true];

  // Reserved for Phase 3 :
  //   - `quantization` (int8 packed) at field 10
  //   - `sparsity_indices` at field 11
  reserved 10 to 99;
}

// =============================================================================
//  Safety geometry — one-of dispatch.
// =============================================================================

// `MetaSpaceConfig` describes the meta-space dimension + the safety
// geometry the meta-consensus must respect.  The safety is enforced
// server-side at every meta-step (cf. PR #1 `BoxSafety::contains` +
// `MetaSwarm::safety_project`).
message MetaSpaceConfig {
  uint32 dim = 1;

  // Exactly one of these MUST be set.  Server rejects with
  // META_ERROR_INVALID_SAFETY otherwise.
  oneof safety_geometry {
    BoxSafety   box  = 2;
    Polyhedron  poly = 3;
    LearnedHull hull = 4;
  }

  // Reserved 5-99 for future geometries (cone, ellipsoid, …).
  reserved 5 to 99;
}

// Symmetric box safety :  ‖x‖∞ ≤ radius (per axis).  Phase-2 default.
message BoxSafety {
  // Symmetric radius applied to every axis.  Asymmetric boxes
  // (different bound per axis) are deferred to Phase 3 ; will be
  // expressed via a future `repeated float per_axis_radius`.
  float radius = 1;

  reserved 2 to 99;
}

// Half-space polytope : { x : A·x ≤ b }.
//
// Layout :
//   - `a_flat` is the flattened constraint matrix, row-major, of
//     length `dim_a × dim` where `dim` is read from
//     `MetaSpaceConfig.dim`.
//   - `b` is the right-hand side, length `dim_a`.
//   - `dim_a` is the number of constraints.
message Polyhedron {
  repeated float a_flat = 1 [packed = true];
  repeated float b      = 2 [packed = true];
  uint32 dim_a          = 3;

  reserved 4 to 99;
}

// Learned hull : a serialised neural distance function or similar.
// Phase-2 placeholder ; not exercised by the soak (the soak uses
// BoxSafety).
message LearnedHull {
  bytes  serialized_model = 1;   // bincode-encoded in Phase 2
  string version          = 2;

  reserved 3 to 99;
}

// =============================================================================
//  Participant descriptor — capability handshake.
// =============================================================================
//
// Returned by `MetaParticipantService::Capabilities`.  Allows the
// controller to verify that every participant matches the manifest's
// expectation before starting an `Iterate`.  Mismatches surface as
// META_ERROR_DIM_MISMATCH or META_ERROR_KAPPA_OUT_OF_RANGE.

message ParticipantDescriptor {
  string id           = 1;   // unique within a MetaSwarm (e.g. "supplier-scoring")
  uint32 dim          = 2;
  float  kappa        = 3;   // contraction constant ; must be in (0, 1)
  uint64 seed         = 4;
  string build_sha    = 5;   // git short SHA of the participant binary
  string blas_backend = 6;   // "none" | "openblas" | "mkl" | "metal" | ...
  repeated string features = 7;  // e.g. ["determinism", "metrics", "trace"]

  // Optional : self-reported max steps/sec for capacity planning.
  // Reserved for Phase 3.  Phase 2 readers MUST ignore.
  reserved 8 to 99;
}

// =============================================================================
//  Iteration config — knobs the controller forwards from its manifest.
// =============================================================================

message IterationConfig {
  uint32 max_iters      = 1;   // capped at 200 server-side
  double tolerance      = 2;   // joint residual threshold
  double alpha_default  = 3;   // step damping
  double coupling_gamma = 4;   // meta-operator coupling

  // If true, the server guarantees that two iterates from the same
  // initial state produce bit-identical results.  Implementation : no
  // parallel non-deterministic reductions ; fixed rayon thread count ;
  // disabled BLAS multithreading.
  bool deterministic = 5;

  // Wall-time budget.  Server caps at 30000 (30 s) ; longer requests
  // are rejected with DEADLINE_EXCEEDED.
  uint32 max_wall_ms = 6;

  // Reserved for Phase 3 :
  //   - 10 : f64 precision opt-in
  //   - 11 : per-step tracing override
  reserved 10 to 99;
}

// =============================================================================
//  StopReason — why an Iterate terminated.
// =============================================================================

enum StopReason {
  STOP_REASON_UNSPECIFIED      = 0;
  STOP_REASON_CONVERGED        = 1;
  STOP_REASON_MAX_ITERS        = 2;
  STOP_REASON_SAFETY_FAIL      = 3;
  STOP_REASON_PARTICIPANT_DOWN = 4;
  STOP_REASON_DEADLINE         = 5;
  STOP_REASON_CONFIG_RELOAD    = 6;

  // Reserved for Phase 3 :
  //   STOP_REASON_TENANT_QUOTA = 10;
  //   STOP_REASON_REGION_FAILOVER = 11;
  reserved 7 to 9, 10 to 99;
}

// =============================================================================
//  Iterate request / response  (used by MetaSwarmService).
// =============================================================================

message IterateRequest {
  // Optional : if absent, the controller uses its persisted manifest.
  // If present, MUST match the controller's expectation exactly ;
  // a mismatch is META_ERROR_DIM_MISMATCH.
  repeated ParticipantDescriptor expected_participants = 1;

  MetaSpaceConfig meta      = 2;
  IterationConfig iteration = 3;

  // Optional : initial participant states (in the same order as
  // `expected_participants`).  Absent ⇔ start from each participant's
  // last state (or zero if Reset was just called).
  repeated TensorF32 initial_states = 4;

  // W3C TraceParent header.  If absent, the controller starts a fresh
  // trace and returns its trace_id in IterateResponse.trace_id.
  string traceparent = 5;

  reserved 6 to 99;
}

message IterateResponse {
  uint32     iterations          = 1;
  double     final_residual      = 2;
  TensorF32  meta_consensus      = 3;
  repeated TensorF32 final_states = 4;
  StopReason stop_reason          = 5;
  uint64     wall_us              = 6;
  string     trace_id             = 7;

  reserved 8 to 99;
}

// Streaming chunk — one inner step per chunk.
message IterateChunk {
  uint32 step     = 1;
  double residual = 2;
  // Per-participant inner state norms — useful for the bench HTML
  // (PR #21 + #24).
  repeated float participant_norms = 3 [packed = true];

  reserved 4 to 99;
}

// =============================================================================
//  StepOnce request / response (used by MetaSwarmService).
// =============================================================================

message StepOnceRequest {
  // No body for now — the controller uses its persisted state.
  // Reserved for Phase 3 add of explicit-state-input.
  reserved 1 to 99;
}

message StepOnceResponse {
  TensorF32 meta_consensus = 1;
  double    residual       = 2;
  reserved 3 to 99;
}

// =============================================================================
//  Reset request / response (used by both services).
// =============================================================================

message ResetRequest {
  // Optional : per-participant initial states.  Empty ⇔ zero state.
  repeated TensorF32 initial_states = 1;
  reserved 2 to 99;
}

message ResetResponse {
  uint64 reset_gen = 1;   // monotonic counter, incremented per reset
  reserved 2 to 99;
}

message ResetParticipantRequest {
  // Optional : initial state for this participant.  Empty ⇔ zero.
  TensorF32 initial = 1;
  reserved 2 to 99;
}

message ResetParticipantResponse {
  uint64 reset_gen = 1;
  reserved 2 to 99;
}

// =============================================================================
//  Step request / response (used by MetaParticipantService).
// =============================================================================

message StepRequest {
  TensorF32 pulled_from_meta = 1;
  uint32    step_index       = 2;
  reserved 3 to 99;
}

message StepResponse {
  TensorF32 new_state      = 1;
  double    local_residual = 2;
  uint64    step_us        = 3;   // latency self-report
  reserved 4 to 99;
}

// =============================================================================
//  State / Capabilities (used by MetaParticipantService).
// =============================================================================

message StateRequest {
  reserved 1 to 99;
}
message StateResponse {
  TensorF32 current = 1;
  uint32    step_index_last = 2;
  reserved 3 to 99;
}

message CapabilitiesRequest {
  reserved 1 to 99;
}
message CapabilitiesResponse {
  ParticipantDescriptor descriptor = 1;
  reserved 2 to 99;
}

// =============================================================================
//  ReloadConfig (MetaSwarmService).
// =============================================================================

message ReloadConfigRequest {
  // If empty : re-read manifest from disk.
  // If non-empty : apply inline (admin RPC).
  optional MetaSpaceConfig new_meta       = 1;
  repeated ParticipantDescriptor new_participants = 2;

  // If true (default) : wait up to 30 s for in-flight Iterate to drain.
  // If false : abort in-flight Iterate with STOP_REASON_CONFIG_RELOAD.
  bool drain_inflight = 3;

  reserved 4 to 99;
}

message ReloadConfigResponse {
  uint32 reload_gen       = 1;
  repeated string applied_changes = 2;
  uint64 reload_wall_us   = 3;
  bool   inflight_aborted = 4;
  reserved 5 to 99;
}

// =============================================================================
//  Health (both services).
// =============================================================================

message HealthRequest {
  reserved 1 to 99;
}

message HealthResponse {
  enum Status {
    STATUS_UNSPECIFIED = 0;
    STATUS_SERVING     = 1;
    STATUS_NOT_SERVING = 2;
    STATUS_RELOADING   = 3;   // SIGHUP in flight
  }

  Status status     = 1;
  uint32 inflight   = 2;
  uint32 reload_gen = 3;

  reserved 4 to 99;
}

// =============================================================================
//  Info (MetaSwarmService).
// =============================================================================

message InfoRequest {
  reserved 1 to 99;
}

message InfoResponse {
  string build_sha       = 1;
  string proto_version   = 2;   // "v1" — frozen at v0.2.0 PR #5
  repeated ParticipantDescriptor configured_participants = 3;
  string leader_pod_name = 4;
  uint64 uptime_s        = 5;
  uint32 last_reload_gen = 6;

  reserved 7 to 99;
}
```

### 8.1 Field-number choices

Every `reserved 10..99` block is **intentional**.  It pre-reserves
slots for Phase 3+ additions so we can't accidentally re-use numbers
that may have meaning in a later v2 of the surface.  Cf. § 16.

### 8.2 Wire-size budget (per Iterate call, expected)

| Item | Bytes | Notes |
|---|---:|---|
| IterateRequest envelope | ~ 50 | tag overhead, no initial states |
| IterateResponse envelope | ~ 30 | |
| Meta-consensus TensorF32 (dim=256) | 1 050 | 256 × 4 + ~20 tag overhead |
| 3 × final_states (dim=384, 512, 384) | 4 850 | sum of 3 tensors |
| Per inner step (3 participants) | ~ 4 350 | 3 × (pulled + new_state) |
| × 47 iters | ~ 204 000 | dominates the wire cost |
| **Total per Iterate** | **~ 210 KB** | mostly the 47-iter inner traffic |

At 100 rps : ~ 21 MB/s of meta-swarm gRPC traffic.  Well under a 1 Gbps
pod link.  StepStream (Phase 2 optional) would halve this by saving
the per-step request headers.

---

## 9. Proto file #2 — `errors.proto`

```protobuf
// Copyright (C) 2026 EFE.  UNLICENSED.
//
// Phase-2 (Sprint A1) error codes for MetaSwarm + MetaParticipant
// services.  Sent in the gRPC trailer's `error_code` field (which
// is the binary serialised `MetaSwarmErrorCode` value) alongside the
// standard gRPC status code.
//
// Mapping :
//   gRPC code FAILED_PRECONDITION ← configuration errors (1xx)
//   gRPC code INTERNAL            ← runtime errors (10xx)
//   gRPC code UNAVAILABLE         ← network / liveness (20xx)
//   gRPC code DEADLINE_EXCEEDED   ← max_wall_ms or RPC deadline
//   gRPC code UNAUTHENTICATED     ← mTLS / auth failures (30xx)

syntax = "proto3";

package efe.v1.meta_swarm;

enum MetaSwarmErrorCode {
  META_ERROR_UNSPECIFIED            = 0;

  // ─────────────────────────────────────────────────────────────
  // Configuration errors → gRPC FAILED_PRECONDITION.
  // ─────────────────────────────────────────────────────────────
  META_ERROR_DIM_MISMATCH           = 1;
  META_ERROR_DUPLICATE_PARTICIPANT_ID = 2;
  META_ERROR_INVALID_SAFETY         = 3;
  META_ERROR_KAPPA_OUT_OF_RANGE     = 4;
  META_ERROR_MAX_ITERS_EXCEEDS_CAP  = 5;   // server cap 200
  META_ERROR_TOLERANCE_NON_POSITIVE = 6;
  META_ERROR_DETERMINISTIC_REQUIRED = 7;   // a feature flag says "must be deterministic" but request asked otherwise

  // ─────────────────────────────────────────────────────────────
  // Runtime errors → gRPC INTERNAL.
  // ─────────────────────────────────────────────────────────────
  META_ERROR_PARTICIPANT_DOWN       = 1001;
  META_ERROR_DETERMINISM_BROKEN     = 1002;
  META_ERROR_SAFETY_VIOLATION       = 1003;
  META_ERROR_DIVERGENCE_DETECTED    = 1004;
  META_ERROR_INFLIGHT_LIMIT         = 1005;

  // ─────────────────────────────────────────────────────────────
  // Reload errors → gRPC FAILED_PRECONDITION.
  // ─────────────────────────────────────────────────────────────
  META_ERROR_RELOAD_BUSY            = 2001;   // another reload in flight
  META_ERROR_RELOAD_DIM_CHANGED     = 2002;   // dimension change not supported live
  META_ERROR_RELOAD_MANIFEST_BAD    = 2003;   // unparseable / invalid file

  // ─────────────────────────────────────────────────────────────
  // Discovery / network → gRPC UNAVAILABLE.
  // ─────────────────────────────────────────────────────────────
  META_ERROR_PARTICIPANT_UNREACHABLE = 3001;
  META_ERROR_CONTROLLER_NOT_READY    = 3002;
  META_ERROR_TLS_HANDSHAKE_FAIL      = 3003;

  // ─────────────────────────────────────────────────────────────
  // Auth → gRPC UNAUTHENTICATED.
  // ─────────────────────────────────────────────────────────────
  META_ERROR_AUTH_INVALID_CERT      = 4001;
  META_ERROR_AUTH_SCOPE_DENIED      = 4002;
  META_ERROR_AUTH_TOKEN_EXPIRED     = 4003;

  // ─────────────────────────────────────────────────────────────
  // Reserved for Phase 3 :
  //   META_ERROR_TENANT_QUOTA_EXCEEDED = 5001;
  //   META_ERROR_REGION_UNREACHABLE    = 5002;
  // ─────────────────────────────────────────────────────────────
  reserved 5000 to 9999;
}
```

### 9.1 Decoupling rationale

Error codes live in their own file because :
- They're consumed by **both** services + by clients.
- They evolve more frequently than messages (every new failure path
  becomes a code).
- Reviewers can scope error-code reviews to one file.

---

## 10. Proto file #3 — `service_meta_swarm.proto`

```protobuf
// Copyright (C) 2026 EFE.  UNLICENSED.
//
// Phase-2 (Sprint A1) — MetaSwarmService : the coordinator-facing
// gRPC surface.  Implemented by `efe-meta-swarm-server` (controller
// binary).  Called by `efe-server` (when `EFE_META_SWARM_ENABLED=true`)
// and by the bench harness (PR #21).

syntax = "proto3";

package efe.v1.meta_swarm;

import "efe/v1/meta_swarm/messages.proto";

// =============================================================================
//  MetaSwarmService — 7 RPCs.
// =============================================================================
//
// All RPCs are mTLS-authenticated (PR #16) ; the caller's SAN MUST be
// `efe-server.efe.svc.cluster.local` (or the local-dev equivalent) to
// be authorised.  RPC-specific scope checks (e.g. `admin` for
// ReloadConfig) are enforced server-side.

service MetaSwarmService {

  // ───────────────────────────────────────────────────────────────
  // Resolve a fixed-point of the joint contractive system.
  //
  // This is the workhorse RPC.  Maps directly to Phase-1
  // `MetaSwarm::iterate()`.  Blocking : caller is expected to wait up
  // to `iteration.max_wall_ms` (server-capped at 30 s).
  //
  // Concurrency : the server may run multiple concurrent Iterate
  // calls up to `EFE_META_SWARM_MAX_INFLIGHT` (default 10).  Beyond
  // that, returns RESOURCE_EXHAUSTED.
  //
  // Cancellation : if the client drops the RPC, the server cancels
  // its participant calls within 100 ms.
  //
  // Idempotence : two calls with identical inputs (same manifest,
  // same `expected_participants`, same `initial_states`, same
  // `iteration.deterministic = true`) produce bit-identical responses.
  // Determinism is verified by SLO-A2-GRPC-MULTIPROC-DETERMINISM.
  rpc Iterate (IterateRequest) returns (IterateResponse);

  // ───────────────────────────────────────────────────────────────
  // Streaming variant — emits one chunk per inner step.  Used by the
  // public bench (PR #21+#24) to capture the residual trajectory.
  // Final chunk includes the IterateResponse fields via trailer.
  //
  // Not on the hot path ; do NOT use from `efe-server`.
  rpc IterateStream (IterateRequest) returns (stream IterateChunk);

  // ───────────────────────────────────────────────────────────────
  // One-shot evaluation of the meta-operator at the current state.
  // Maps to `MetaSwarm::step_once()`.  Reserved for latency-sensitive
  // Phase-3 paths ; Phase-2 callers prefer Iterate().
  rpc StepOnce (StepOnceRequest) returns (StepOnceResponse);

  // ───────────────────────────────────────────────────────────────
  // Reset all participants to their seed state (or to a provided
  // initial set).  Maps to `MetaSwarm::reset()`.
  //
  // Drains in-flight Iterate calls first (server-side).
  rpc Reset (ResetRequest) returns (ResetResponse);

  // ───────────────────────────────────────────────────────────────
  // Hot-reload manifest + safety geometry.  Triggered by SIGHUP on
  // the controller (in which case the request body is empty and the
  // controller re-reads its on-disk manifest), OR by an admin RPC
  // (request body non-empty).
  //
  // Requires the caller's auth token to have the `admin` scope.
  //
  // SLO : reload wall time ≤ 200 ms p99 (SLO-A2-RELOAD-P99).
  rpc ReloadConfig (ReloadConfigRequest) returns (ReloadConfigResponse);

  // ───────────────────────────────────────────────────────────────
  // Liveness + readiness probe.  Returns within 1 ms.  Safe to call
  // at high frequency.  Used by the K8s lease leader election +
  // Helm `helm test` connection probe.
  rpc Health (HealthRequest) returns (HealthResponse);

  // ───────────────────────────────────────────────────────────────
  // Build + topology info for the dashboard.  Cheap.  Read-only.
  rpc Info (InfoRequest) returns (InfoResponse);
}
```

---

## 11. Proto file #4 — `service_meta_participant.proto`

```protobuf
// Copyright (C) 2026 EFE.  UNLICENSED.
//
// Phase-2 (Sprint A1) — MetaParticipantService : the participant-
// facing gRPC surface.  Implemented by `efe-meta-swarm-server`
// (participant binary).  Called by the controller only ; participants
// authorise callers by SAN-checking the leaf cert.

syntax = "proto3";

package efe.v1.meta_swarm;

import "efe/v1/meta_swarm/messages.proto";

// =============================================================================
//  MetaParticipantService — 6 RPCs.
// =============================================================================

service MetaParticipantService {

  // ───────────────────────────────────────────────────────────────
  // Receive a pulled vector and produce the next contractive step.
  // Maps to `SwarmParticipant::step_with_pull()`.
  //
  // This is the MOST-FREQUENT RPC of the entire transport.  At 47
  // iters × 3 participants × 100 rps ≈ 14 100 Step RPCs/sec on a
  // busy controller.
  //
  // Latency budget : ≤ 3 ms p99 in-rack on a 4-vCPU pod.
  rpc Step (StepRequest) returns (StepResponse);

  // ───────────────────────────────────────────────────────────────
  // Reset to the per-participant seed state.
  rpc Reset (ResetParticipantRequest) returns (ResetParticipantResponse);

  // ───────────────────────────────────────────────────────────────
  // Return the current state tensor.  Not called on the hot path ;
  // reserved for the Phase-2 dashboard + the consistency probe.
  rpc State (StateRequest) returns (StateResponse);

  // ───────────────────────────────────────────────────────────────
  // Server-streaming variant : participant emits one StepResponse
  // per incoming StepRequest, both sides streamed.  Saves one TLS
  // record per step (~ 30 % wire overhead reduction on the hot path).
  //
  // OPTIONAL : controllers MAY use Step() instead.  Whether to use
  // StepStream is decided per-deployment via
  // `EFE_META_SWARM_USE_STEP_STREAM=true|false` (default : false in
  // Phase 2 ; expected to flip to true in Phase 3 once we have
  // streaming-friendly load balancers).
  rpc StepStream (stream StepRequest) returns (stream StepResponse);

  // ───────────────────────────────────────────────────────────────
  // Capability query.  Returns the participant's descriptor (dim,
  // kappa, seed, build_sha, blas_backend, features).
  //
  // Called once at controller startup + on every SIGHUP.  If the
  // returned descriptor mismatches the manifest's expectation, the
  // controller fails to start (or aborts the reload).
  rpc Capabilities (CapabilitiesRequest) returns (CapabilitiesResponse);

  // ───────────────────────────────────────────────────────────────
  // Liveness + readiness probe.
  rpc Health (HealthRequest) returns (HealthResponse);
}
```

---

## 12. `src/lib.rs` — full

```rust
//! Protocol Buffers + tonic-generated client/server stubs for the EFE
//! meta-swarm gRPC surface.
//!
//! This crate is **scaffolding only** : it contains no business logic.
//! The bulk of the source is generated at build-time by `tonic-build`
//! from the four `.proto` files under `proto/efe/v1/meta_swarm/`.
//!
//! ## Public surface
//!
//! After `cargo build`, consumers can import :
//!
//! ```ignore
//! use efe_meta_swarm_proto::{
//!     // services
//!     meta_swarm::MetaSwarmServiceClient,
//!     meta_swarm::MetaSwarmServiceServer,
//!     meta_swarm::MetaParticipantServiceClient,
//!     meta_swarm::MetaParticipantServiceServer,
//!     // messages
//!     meta_swarm::IterateRequest, meta_swarm::IterateResponse,
//!     meta_swarm::StepRequest,    meta_swarm::StepResponse,
//!     meta_swarm::TensorF32,
//!     meta_swarm::ParticipantDescriptor,
//!     meta_swarm::MetaSpaceConfig,
//!     meta_swarm::BoxSafety, meta_swarm::Polyhedron, meta_swarm::LearnedHull,
//!     meta_swarm::StopReason,
//!     meta_swarm::IterationConfig,
//!     // errors
//!     meta_swarm::MetaSwarmErrorCode,
//! };
//! ```
//!
//! See the [crate README](./../README.md) for the full versioning
//! policy + field-number reservation strategy.
//!
//! ## Features
//!
//!   * `serde-derives` : derives `serde::{Serialize, Deserialize}` on
//!     every generated message type.  Used by the public bench HTML
//!     emitter (PR #21+#24).
//!   * `client-only`   : drops server stubs.  Saves ~30 % binary size.
//!   * `server-only`   : drops client stubs.  Mirror of above.
//!   * `full`          : `serde-derives` (cargo doc convenience).
//!
//! ## Code generation
//!
//! Generated under `OUT_DIR/efe.v1.meta_swarm.rs`.  Included below via
//! `tonic::include_proto!`.

#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

/// All generated types live in this module.  We expose it under a
/// short alias so consumers write `meta_swarm::Foo` rather than
/// `efe::v1::meta_swarm::Foo`.
pub mod meta_swarm {
    tonic::include_proto!("efe.v1.meta_swarm");
}

/// Convenience re-export of the proto version constant.  Tested in CI
/// against the value the server sends back in `InfoResponse`.
pub const PROTO_VERSION: &str = "v1";

/// The package name as it appears on the wire.  Useful for prefix-
/// matching in trace-based debugging.
pub const PROTO_PACKAGE: &str = "efe.v1.meta_swarm";

/// All public RPC names, in the order they appear in the proto files.
/// Allows consumers to enumerate the surface without re-parsing the
/// proto file at runtime.  Tested in CI : a new RPC added without a
/// corresponding entry here will fail the smoke test.
pub const RPC_NAMES: &[&str] = &[
    // MetaSwarmService
    "Iterate",
    "IterateStream",
    "StepOnce",
    "Reset",
    "ReloadConfig",
    "Health",
    "Info",
    // MetaParticipantService
    "Step",
    "Reset",            // intentionally duplicated : different service
    "State",
    "StepStream",
    "Capabilities",
    "Health",           // intentionally duplicated
];

/// Module aliases for the two services for ergonomic imports.
pub mod controller {
    //! Re-exports of the MetaSwarmService client/server traits.
    pub use crate::meta_swarm::meta_swarm_service_client::MetaSwarmServiceClient;
    #[cfg(not(feature = "client-only"))]
    pub use crate::meta_swarm::meta_swarm_service_server::{
        MetaSwarmService, MetaSwarmServiceServer,
    };
}

pub mod participant {
    //! Re-exports of the MetaParticipantService client/server traits.
    pub use crate::meta_swarm::meta_participant_service_client::MetaParticipantServiceClient;
    #[cfg(not(feature = "client-only"))]
    pub use crate::meta_swarm::meta_participant_service_server::{
        MetaParticipantService, MetaParticipantServiceServer,
    };
}

/// Helpers to convert between the proto types and idiomatic Rust
/// types from `efe-meta-swarm`.  Pulled in by `efe-meta-swarm-server`
/// (PR #6+) ; kept in this crate so they're guaranteed to compile
/// against the proto types of the same version.
///
/// NOTE : this module intentionally does NOT depend on the
/// `efe-meta-swarm` crate.  Conversions go through plain `Vec<f32>` /
/// `f64` etc.  This keeps `efe-meta-swarm-proto` dependency-light.
pub mod conv {
    use super::meta_swarm::TensorF32;

    /// Build a `TensorF32` from a `&[f32]`.  Cheap (one Vec clone).
    #[must_use]
    pub fn tensor_from_slice(data: &[f32]) -> TensorF32 {
        TensorF32 {
            dim: u32::try_from(data.len()).expect("dim must fit in u32"),
            data: data.to_vec(),
        }
    }

    /// Validate a `TensorF32` : `dim == data.len()`.  Returns the
    /// inner slice on success.  Used by the server-side parsing path.
    pub fn tensor_validate(t: &TensorF32) -> Result<&[f32], &'static str> {
        if t.dim as usize != t.data.len() {
            return Err("TensorF32 : dim != data.len()");
        }
        Ok(&t.data)
    }
}

// ---------------------------------------------------------------------------
//  CI smoke (compile-time)
// ---------------------------------------------------------------------------
//
// A type-level check : the modules exist and the canonical names
// compile.  If tonic-build's generated module layout ever changes,
// this fails at compile-time with a clear error.

#[allow(dead_code)]
fn _compile_time_smoke() {
    let _: Option<crate::meta_swarm::IterateRequest> = None;
    let _: Option<crate::meta_swarm::IterateResponse> = None;
    let _: Option<crate::meta_swarm::TensorF32> = None;
    let _: Option<crate::meta_swarm::MetaSwarmErrorCode> = None;
    let _: Option<crate::meta_swarm::StopReason> = None;
    // Client stubs always available.
    let _: Option<crate::controller::MetaSwarmServiceClient<tonic::transport::Channel>> = None;
    let _: Option<crate::participant::MetaParticipantServiceClient<tonic::transport::Channel>> = None;
}
```

### 12.1 Why the layout

| Decision | Why |
|---|---|
| `pub mod meta_swarm` (single module wrapping `include_proto!`) | Mirrors the proto package name ; consumers write the natural `meta_swarm::Foo`. |
| `pub mod controller` + `pub mod participant` (sub-module aliases) | Two cleaner namespaces for the two services.  Avoids the 70-char `meta_swarm::meta_swarm_service_client::MetaSwarmServiceClient`. |
| `PROTO_VERSION`, `PROTO_PACKAGE`, `RPC_NAMES` constants | Lets consumers introspect at compile time + makes adding a new RPC fail CI loudly (cf. smoke test § 18). |
| `pub mod conv` (Vec<f32> ↔ TensorF32 helpers) | Avoids every consumer reinventing the conversion.  Kept dep-light : no `efe-meta-swarm` link. |
| `_compile_time_smoke` private fn | Belt-and-suspenders : if tonic-build moves a generated symbol, we fail in this crate, not in a downstream user. |
| `#![forbid(unsafe_code)]` | This crate is pure proto + re-exports ; no reason to ever drop into unsafe. |

---

## 13. Generated code expectations

### 13.1 What `tonic-build` produces

Single file `$OUT_DIR/efe.v1.meta_swarm.rs`, structured as :

```
// Messages (prost-generated)
pub struct TensorF32 { pub dim: u32, pub data: Vec<f32> }
pub struct MetaSpaceConfig { pub dim: u32, pub safety_geometry: Option<meta_space_config::SafetyGeometry> }
pub struct BoxSafety { pub radius: f32 }
// ...
pub mod meta_space_config { pub enum SafetyGeometry { Box(BoxSafety), Poly(Polyhedron), Hull(LearnedHull) } }

// Enums
#[repr(i32)] pub enum StopReason { Unspecified = 0, Converged = 1, ... }
#[repr(i32)] pub enum MetaSwarmErrorCode { Unspecified = 0, DimMismatch = 1, ... }

// MetaSwarmService client + server
pub mod meta_swarm_service_client {
    pub struct MetaSwarmServiceClient<T> { ... }
    impl<T: ...> MetaSwarmServiceClient<T> {
        pub async fn iterate(&mut self, req: impl IntoRequest<IterateRequest>) -> Result<Response<IterateResponse>, Status>;
        pub async fn iterate_stream(...) -> Result<Response<Streaming<IterateChunk>>, Status>;
        pub async fn step_once(...) -> Result<Response<StepOnceResponse>, Status>;
        pub async fn reset(...) -> Result<Response<ResetResponse>, Status>;
        pub async fn reload_config(...) -> Result<Response<ReloadConfigResponse>, Status>;
        pub async fn health(...) -> Result<Response<HealthResponse>, Status>;
        pub async fn info(...) -> Result<Response<InfoResponse>, Status>;
    }
}

pub mod meta_swarm_service_server {
    #[async_trait::async_trait] pub trait MetaSwarmService: Send + Sync + 'static {
        async fn iterate(&self, req: Request<IterateRequest>) -> Result<Response<IterateResponse>, Status>;
        // ... one method per RPC ...
    }
    pub struct MetaSwarmServiceServer<T: MetaSwarmService> { ... }
}

// MetaParticipantService — mirror of above
pub mod meta_participant_service_client { /* ... */ }
pub mod meta_participant_service_server { /* ... */ }
```

### 13.2 What we test about the generated code (in this PR)

| # | Property | Test |
|---|---|---|
| GC1 | `efe_meta_swarm_proto::meta_swarm::IterateRequest` is constructible | compile-time smoke in `lib.rs` |
| GC2 | Client + server traits exist for both services | compile-time smoke |
| GC3 | Default features compile clean | `cargo build` in CI |
| GC4 | `serde-derives` feature compiles clean | `cargo build --features serde-derives` in CI |
| GC5 | `client-only` feature compiles clean | `cargo build --features client-only --no-default-features` |
| GC6 | `server-only` feature compiles clean | `cargo build --features server-only --no-default-features` |
| GC7 | Cross-crate import works | `tests/smoke_cross_crate.rs` (§ 18) |
| GC8 | `cargo doc` clean | CI step |

We do **NOT** test :
- Wire-format compatibility (that's PR #10's integration test).
- RPC behaviour (that's PR #8 + #9).
- Field-number stability across versions (that's a Phase-3 concern).

---

## 14. `README.md` — full

````markdown
# `efe-meta-swarm-proto`

Protocol Buffers + tonic-generated client/server stubs for the EFE
meta-swarm gRPC surface.

Part of **Sprint A1 Phase 2** (gRPC transport).  See
[`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_ARCHITECTURE.md)
for the full design.

> **License** : UNLICENSED — see workspace [LICENSE](../../LICENSE)
> + the R5-B1 carve-out in
> [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../../docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md).
> Target license at Phase 5 graduation : Apache-2.0.

## What this crate is

A **wire-protocol-only** crate.  Contains :

- 4 `.proto` files under `proto/efe/v1/meta_swarm/`.
- A `build.rs` that drives `tonic-build` to generate Rust client +
  server stubs into `$OUT_DIR/`.
- A thin `src/lib.rs` that re-exports the generated types under
  ergonomic module aliases.

## What this crate is NOT

- No service implementation.  See `crates/efe-meta-swarm-server/` (PR #6+).
- No business logic.  See `crates/efe-meta-swarm/` (Phase 1).
- No transport configuration (mTLS, discovery, retries).  See
  `crates/efe-meta-swarm-server/src/auth/` (PR #16+).

## How to consume

In your `Cargo.toml` :

```toml
[dependencies]
efe-meta-swarm-proto = { path = "../efe-meta-swarm-proto" }
```

Then in Rust :

```rust
use efe_meta_swarm_proto::{
    controller::MetaSwarmServiceClient,
    meta_swarm::{IterateRequest, IterateResponse, IterationConfig, MetaSpaceConfig},
};

let mut client = MetaSwarmServiceClient::connect("https://controller:50080").await?;
let resp = client.iterate(IterateRequest { /* ... */ }).await?;
```

## Cargo features

| Feature | Default | What it does |
|---|:---:|---|
| `serde-derives` | off | Derives `serde::Serialize` + `serde::Deserialize` on every generated message.  Used by the public bench HTML emitter. |
| `client-only`   | off | Drops the server stubs ; saves ~30 % binary size. |
| `server-only`   | off | Drops the client stubs ; mirror of above. |
| `full`          | off | Convenience : `serde-derives`.  Used by `cargo doc`. |

## Versioning policy

| Surface | Stability promise |
|---|---|
| Proto **package name** `efe.v1.meta_swarm` | Frozen for the life of the v1 API (≥ 12 months from v0.2.0 GA, expected 2026-08-31 → 2027-08-31 at minimum). |
| Proto **message field numbers** 1-99 | Frozen.  Never reassigned.  Never reused. |
| Proto **message field numbers** ≥ 100 | Reserved for v1 additions.  Allocated in this README. |
| Proto **enum values** 0-99 | Frozen. |
| Proto **enum values** ≥ 100 | Reserved for v1 additions. |
| Proto **service RPC names** | Frozen for the life of v1.  Adding a new RPC is backwards-compatible. |
| Proto **service RPC names** removed | Never within v1.  Removing is a v2 break. |
| Proto **service RPC signatures** | Frozen ; adding a field to a request/response is OK (new tag). |
| Generated Rust **module paths** | Stable as long as tonic-build's API contract holds. |
| Generated Rust **type names** | Stable. |
| `pub mod controller` / `pub mod participant` aliases | Stable for v1. |
| `PROTO_VERSION` / `PROTO_PACKAGE` / `RPC_NAMES` constants | Stable for v1. |

### Breaking-change protocol

If a backward-incompatible change is needed :

1. Open an RFC under `docs/rfcs/`.
2. Create `proto/efe/v2/meta_swarm/` (do not touch v1 files).
3. Implement both services side by side for ≥ 6 months.
4. Announce v1 deprecation date in the L4 or L5 graduation post.
5. Delete v1 in a Phase-6 PR (target : 2027-12-01 at earliest).

## Field-number reservation

This README is the **source of truth** for field-number allocation
within v1.  Adding a new field requires :

1. Picking an unused number ≥ 100 within the message's reserved
   block (every message has `reserved 10..99` or `reserved 100..999`).
2. Documenting the allocation in the table below in the same PR.
3. Adding a smoke test that pins the new field's wire identity.

### Allocation table

| Message | Field range used | Field range reserved | Next free |
|---|---|---|---|
| `TensorF32` | 1, 2 | 3-9, 10-99 | 3 |
| `MetaSpaceConfig` | 1, oneof{2,3,4} | 5-99 | 5 |
| `BoxSafety` | 1 | 2-99 | 2 |
| `Polyhedron` | 1, 2, 3 | 4-99 | 4 |
| `LearnedHull` | 1, 2 | 3-99 | 3 |
| `ParticipantDescriptor` | 1-7 | 8-99 | 8 |
| `IterationConfig` | 1-6 | 7-9, 10-99 | 10 |
| `IterateRequest` | 1-5 | 6-99 | 6 |
| `IterateResponse` | 1-7 | 8-99 | 8 |
| `IterateChunk` | 1-3 | 4-99 | 4 |
| `StepOnceRequest` | — | 1-99 | 1 |
| `StepOnceResponse` | 1-2 | 3-99 | 3 |
| `ResetRequest` | 1 | 2-99 | 2 |
| `ResetResponse` | 1 | 2-99 | 2 |
| `ResetParticipantRequest` | 1 | 2-99 | 2 |
| `ResetParticipantResponse` | 1 | 2-99 | 2 |
| `StepRequest` | 1, 2 | 3-99 | 3 |
| `StepResponse` | 1-3 | 4-99 | 4 |
| `StateRequest` | — | 1-99 | 1 |
| `StateResponse` | 1, 2 | 3-99 | 3 |
| `CapabilitiesRequest` | — | 1-99 | 1 |
| `CapabilitiesResponse` | 1 | 2-99 | 2 |
| `ReloadConfigRequest` | 1-3 | 4-99 | 4 |
| `ReloadConfigResponse` | 1-4 | 5-99 | 5 |
| `HealthRequest` | — | 1-99 | 1 |
| `HealthResponse` | 1-3 | 4-99 | 4 |
| `InfoRequest` | — | 1-99 | 1 |
| `InfoResponse` | 1-6 | 7-99 | 7 |

### Enum-value reservation

| Enum | Values used | Values reserved | Next free |
|---|---|---|---|
| `StopReason` | 0-6 | 7-9, 10-99 | 10 |
| `MetaSwarmErrorCode` | 0-7, 1001-1005, 2001-2003, 3001-3003, 4001-4003 | 8-99, 1006-1099, 2004-2099, 3004-3099, 4004-4099, 5000-9999 | 8 (next general) ; 1006 (runtime) ; etc. |
| `HealthResponse.Status` | 0-3 | 4-99 | 4 |

## Repository layout

```
crates/efe-meta-swarm-proto/
├── Cargo.toml
├── build.rs
├── README.md                      ← you are here
├── proto/
│   └── efe/v1/meta_swarm/
│       ├── messages.proto
│       ├── errors.proto
│       ├── service_meta_swarm.proto
│       └── service_meta_participant.proto
├── src/
│   └── lib.rs                     ← re-exports + module aliases
└── tests/
    └── smoke_cross_crate.rs       ← cross-crate import test
```

## Building locally

```bash
# Default features (server + client, no serde).
cargo build -p efe-meta-swarm-proto

# All features (used by `cargo doc`).
cargo build -p efe-meta-swarm-proto --features full

# Client-only build (e.g. for a CLI tool).
cargo build -p efe-meta-swarm-proto --no-default-features --features client-only
```

## Doc generation

```bash
cargo doc -p efe-meta-swarm-proto --features full --no-deps --open
```

## Cross-references

- Phase 2 architecture : [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_ARCHITECTURE.md)
- Phase 2 roadmap : [`docs/EFE_A1_PHASE2_ROADMAP.md`](../../docs/EFE_A1_PHASE2_ROADMAP.md)
- PR #5 (this PR) detailed architecture : [`docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md)
- Sister crate (Phase 1, in-process) : [`crates/efe-meta-swarm/README.md`](../efe-meta-swarm/README.md)

````

---

## 15. Versioning policy

Detailed in the README (§ 14).  Key invariants tested by smoke test
(§ 18) :

| Invariant | Source-of-truth | Verification |
|---|---|---|
| `PROTO_VERSION == "v1"` | `src/lib.rs` const | smoke test reads it |
| Package name == `efe.v1.meta_swarm` | proto files | smoke test reads `PROTO_PACKAGE` |
| RPC count = 13 (7 + 6) | proto files | `RPC_NAMES.len() == 13` |
| RPC names ordered & unique-within-service | `src/lib.rs` const | smoke test asserts |

Any change that violates one of these invariants is a v2 break and
must follow the breaking-change protocol (README § "Breaking-change
protocol").

---

## 16. Field-number reservation strategy

### 16.1 Three policy rules

1. **Once allocated, never reused.**  Even if a field is later
   deprecated, its number stays burned.  This protects against
   accidental data corruption from old wire bytes hitting a new
   schema.
2. **Reserved blocks for every message.**  Every message ends with a
   `reserved N to 99` (or `100 to 999` for high-volume messages).
   This makes Phase 3 additions zero-risk : you grab the next free
   number, document in the table, ship.
3. **Single source of truth in README.**  The table in `README.md` §
   "Allocation table" is the only place where the canonical
   "next free" is recorded.  PR review verifies that any newly-added
   field updates that table in the same diff.

### 16.2 PR-level enforcement

A small CI check (added in PR #15 or later, NOT in PR #5) will diff
the reserved blocks of the proto against the allocation table and
fail if they drift.  In PR #5 we only ship the convention ; the
enforcement is deferred.

### 16.3 What about removed fields ?

Phase 2 does not remove any field (nothing to remove yet).  When a
future PR removes a field, the protocol is :

1. Mark it `reserved` in the proto.
2. Update the allocation table to show `(removed in PRX)`.
3. Add a CHANGELOG entry.

---

## 17. Naming conventions

### 17.1 Protobuf

| Item | Convention | Example |
|---|---|---|
| Package | `lowercase.dotted.v<N>` | `efe.v1.meta_swarm` |
| Service | `PascalCase` ending in `Service` | `MetaSwarmService` |
| RPC | `PascalCase` verb | `Iterate`, `ReloadConfig` |
| Message | `PascalCase` noun, request → `<Verb>Request`, response → `<Verb>Response` | `IterateRequest`, `IterateResponse` |
| Field | `lower_snake_case` | `final_residual`, `meta_consensus` |
| Enum | `PascalCase` | `StopReason` |
| Enum value | `SCREAMING_SNAKE_CASE` prefixed by the enum's package + name | `STOP_REASON_CONVERGED`, `META_ERROR_DIM_MISMATCH` |

### 17.2 Rust (after tonic-build)

| Item | What tonic-build produces | Why we accept it |
|---|---|---|
| Message → struct | `PascalCase` (same as proto) | Idiomatic Rust |
| RPC → async fn | `snake_case` (e.g. `iterate`) | Idiomatic Rust |
| Field → struct field | `snake_case` (e.g. `final_residual`) | Idiomatic Rust |
| Enum → enum | `PascalCase` (e.g. `StopReason`) | Idiomatic Rust |
| Enum value → variant | `PascalCase`, stripped of enum-name prefix (e.g. `StopReason::Converged`) | Idiomatic Rust |

### 17.3 Anti-patterns (avoid)

- ❌ Including unit suffixes inconsistently : prefer `wall_us` to
  `wall_microseconds`.  Standardise on `_us`, `_ms`, `_s`, `_mb`.
- ❌ Pluralised field name with `repeated` : `repeated TensorF32
  final_states` (singular noun, plural inferred from `repeated`).
- ❌ Negative bool fields : `is_deterministic`, not `is_non_deterministic`.

---

## 18. Smoke test — cross-crate import

### 18.1 File : `tests/smoke_cross_crate.rs`

```rust
//! Smoke test : verifies that the crate compiles AND that its public
//! surface (constants, modules, types) matches the documented
//! contract.  This test is the **load-bearing pin** for the proto
//! versioning policy : any change that breaks one of these assertions
//! is a v2 break and requires the full RFC process (cf. README).
//!
//! Run with :
//!   cargo test -p efe-meta-swarm-proto --test smoke_cross_crate
//!   cargo test -p efe-meta-swarm-proto --test smoke_cross_crate --features full

use efe_meta_swarm_proto::{
    PROTO_PACKAGE, PROTO_VERSION, RPC_NAMES,
    controller::MetaSwarmServiceClient,
    participant::MetaParticipantServiceClient,
    meta_swarm::{
        IterateRequest, IterateResponse, IterateChunk,
        StepRequest, StepResponse,
        TensorF32, ParticipantDescriptor,
        MetaSpaceConfig, BoxSafety,
        IterationConfig, StopReason,
        MetaSwarmErrorCode, HealthResponse,
    },
    conv,
};

/// The package name as it appears on the wire is `efe.v1.meta_swarm`.
/// Changing it is a v2 break.
#[test]
fn proto_package_pinned() {
    assert_eq!(PROTO_PACKAGE, "efe.v1.meta_swarm");
}

/// The version constant is `v1` for the entire v1 surface (the same
/// value the server returns in `InfoResponse.proto_version`).
#[test]
fn proto_version_pinned() {
    assert_eq!(PROTO_VERSION, "v1");
}

/// The RPC catalogue has exactly 13 entries (7 in MetaSwarmService +
/// 6 in MetaParticipantService).  Adding one without updating
/// `RPC_NAMES` in lib.rs will fail this test.
#[test]
fn rpc_count_pinned() {
    assert_eq!(RPC_NAMES.len(), 13, "RPC count drift : check RPC_NAMES in lib.rs");
}

/// Default construction works for every shipped message type.  This
/// is a compile-time + runtime check that the prost-derived
/// `Default` impl is present.
#[test]
fn messages_default_construct() {
    let _ = IterateRequest::default();
    let _ = IterateResponse::default();
    let _ = IterateChunk::default();
    let _ = StepRequest::default();
    let _ = StepResponse::default();
    let _ = TensorF32::default();
    let _ = ParticipantDescriptor::default();
    let _ = MetaSpaceConfig::default();
    let _ = BoxSafety::default();
    let _ = IterationConfig::default();
    let _ = HealthResponse::default();
}

/// Enum default values are the `UNSPECIFIED` variant (= 0).  This
/// matches proto3 semantics.
#[test]
fn enum_defaults_are_unspecified() {
    let s = StopReason::default();
    assert_eq!(s as i32, 0);
    let e = MetaSwarmErrorCode::default();
    assert_eq!(e as i32, 0);
}

/// Round-trip serialisation through prost.  Tests that the generated
/// `prost::Message` impl works for a non-trivial value.
#[test]
fn iterate_request_roundtrip() {
    use prost::Message;

    let original = IterateRequest {
        expected_participants: vec![],
        meta: Some(MetaSpaceConfig {
            dim: 256,
            safety_geometry: Some(
                efe_meta_swarm_proto::meta_swarm::meta_space_config::SafetyGeometry::Box(
                    BoxSafety { radius: 10.0 },
                ),
            ),
        }),
        iteration: Some(IterationConfig {
            max_iters: 100,
            tolerance: 1e-4,
            alpha_default: 0.5,
            coupling_gamma: 0.2,
            deterministic: true,
            max_wall_ms: 15000,
        }),
        initial_states: vec![],
        traceparent: String::new(),
    };

    let mut bytes = Vec::new();
    original.encode(&mut bytes).expect("encode");
    let decoded = IterateRequest::decode(&*bytes).expect("decode");
    assert_eq!(decoded.iteration.as_ref().unwrap().max_iters, 100);
    assert_eq!(decoded.iteration.as_ref().unwrap().tolerance, 1e-4);
}

/// `conv::tensor_from_slice` produces a `TensorF32` with the right dim
/// and a copy of the input.
#[test]
fn conv_tensor_from_slice() {
    let data: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
    let t = conv::tensor_from_slice(&data);
    assert_eq!(t.dim, 4);
    assert_eq!(t.data, vec![1.0, 2.0, 3.0, 4.0]);
}

/// `conv::tensor_validate` rejects dim/data mismatch.
#[test]
fn conv_tensor_validate_rejects_mismatch() {
    let bad = TensorF32 { dim: 5, data: vec![1.0, 2.0] };
    assert!(conv::tensor_validate(&bad).is_err());
    let good = TensorF32 { dim: 2, data: vec![1.0, 2.0] };
    assert!(conv::tensor_validate(&good).is_ok());
}

/// Client traits exist for both services and accept a Channel.  This
/// is a compile-time smoke ; no real connection.
#[test]
fn clients_compile_with_channel() {
    fn _accept_meta_swarm_client(_c: MetaSwarmServiceClient<tonic::transport::Channel>) {}
    fn _accept_meta_participant_client(_c: MetaParticipantServiceClient<tonic::transport::Channel>) {}
}

/// When `serde-derives` is enabled, every shipped message implements
/// both `Serialize` and `Deserialize`.  This compile-time test is
/// gated by the feature.
#[cfg(feature = "serde-derives")]
#[test]
fn serde_round_trip_json() {
    let t = TensorF32 { dim: 3, data: vec![1.0, 2.0, 3.0] };
    let json = serde_json::to_string(&t).expect("serialize");
    let back: TensorF32 = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.dim, 3);
    assert_eq!(back.data, vec![1.0, 2.0, 3.0]);
}
```

### 18.2 Why every test exists

| Test | What it catches |
|---|---|
| `proto_package_pinned` | Renaming the proto package |
| `proto_version_pinned` | Drifting the version constant |
| `rpc_count_pinned` | Adding an RPC without updating `RPC_NAMES` |
| `messages_default_construct` | tonic-build regression on `Default` impl |
| `enum_defaults_are_unspecified` | Drifting proto3 enum defaults |
| `iterate_request_roundtrip` | Wire-format regression on encode/decode |
| `conv_tensor_from_slice` | Helper API regression |
| `conv_tensor_validate_rejects_mismatch` | Helper API regression |
| `clients_compile_with_channel` | tonic-build regression on client signature |
| `serde_round_trip_json` (gated) | Serde feature regression |

10 tests, all under 5 ms, run by default `cargo test`.

---

## 19. CI validation

### 19.1 What runs on this PR

| Job | Trigger | Pass criterion |
|---|---|---|
| `cargo build -p efe-meta-swarm-proto` (default) | Every push | exit 0, 0 warnings |
| `cargo build -p efe-meta-swarm-proto --features full` | Every push | exit 0 |
| `cargo build -p efe-meta-swarm-proto --no-default-features --features client-only` | Every push | exit 0 |
| `cargo build -p efe-meta-swarm-proto --no-default-features --features server-only` | Every push | exit 0 |
| `cargo test -p efe-meta-swarm-proto --features full` | Every push | 10/10 tests PASS |
| `cargo doc -p efe-meta-swarm-proto --features full --no-deps` | Every push | exit 0, 0 warnings |
| `cargo clippy -p efe-meta-swarm-proto --features full --all-targets` | Every push | 0 warnings on this crate |
| Phase-1 master gate (full workspace) | Every push | exit 0 — no Phase-1 regression |
| `actionlint .github/workflows/*.yml` | Touch on workflows | exit 0 (no workflow touched in PR #5 ; defensive) |
| Smoke `cargo check -p efe-server --offline` | Every push | exit 0 (proves the proto crate is import-safe even without efe-server changes) |

### 19.2 No new workflows in PR #5

PR #5 deliberately **does not add** new CI workflows.  The new
`bench-meta-swarm-grpc` job is added in PR #22 (when there's actually
something to bench).  The new `helm-meta-swarm-lint` workflow is
added in PR #15.

This keeps PR #5's review surface small and focused on the wire
protocol.

---

## 20. Acceptance criteria (binary)

PR #5 is mergeable iff **all** :

| # | Criterion | Verification |
|---|---|---|
| AC1 | `cargo build -p efe-meta-swarm-proto` (default) succeeds with 0 warnings | CI |
| AC2 | `cargo build -p efe-meta-swarm-proto --features full` succeeds | CI |
| AC3 | `cargo build -p efe-meta-swarm-proto --no-default-features --features client-only` succeeds | CI |
| AC4 | `cargo build -p efe-meta-swarm-proto --no-default-features --features server-only` succeeds | CI |
| AC5 | `cargo test -p efe-meta-swarm-proto --features full` : 10 tests PASS, 0 failed | CI |
| AC6 | `cargo doc -p efe-meta-swarm-proto --features full --no-deps` : 0 warnings | CI |
| AC7 | `cargo clippy -p efe-meta-swarm-proto --features full --all-targets` : 0 warnings on this crate | CI |
| AC8 | Phase-1 master gate stays GREEN | CI |
| AC9 | README's allocation table sums match the protos' actual field numbers | Reviewer ; spot-check 3 random messages |
| AC10 | `actionlint` on workspace workflows still GREEN (defensive) | CI |
| AC11 | Generated `OUT_DIR/efe.v1.meta_swarm.rs` contains `MetaSwarmServiceClient` + `MetaParticipantServiceClient` + their server traits | Reviewer ; manual grep on a local build |
| AC12 | No file outside `crates/efe-meta-swarm-proto/`, `Cargo.toml`, `Cargo.lock` is modified | `git diff --stat` review |
| AC13 | The 3 reviewers (meta-swarm + sec + platform) have approved | GitHub |
| AC14 | sec-team has explicitly confirmed proto package name + RPC list match the threat model expectations (or threat model gets updated in PR #7) | Sec-team comment |
| AC15 | platform-team has confirmed the Helm chart of PR #13 will be able to reference the proto crate's binary names | Platform-team comment |

If any AC is RED, PR #5 stays in Draft.

---

## 21. PR description template

```markdown
# [Sprint A1 Phase 2 — PR #5] feat(efe-meta-swarm-proto): scaffold protos + tonic-build

## Summary

First PR of Phase 2 (Sprint A1.P2.1).  Lands the canonical Protocol
Buffers definitions for the `MetaSwarmService` + `MetaParticipantService`
gRPC surfaces, generates tonic client + server stubs, and exposes them
as a separate, dependency-minimal crate `efe-meta-swarm-proto`.

This PR contains **NO service implementation, NO business logic, NO
tests beyond a cross-crate import smoke**.  It is purely wire-protocol
enablement.  PRs #6, #8, #9, #11, #20, #21 all depend on it.

Full architecture : [`docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md).

## What this PR adds

| Area | Detail |
|---|---|
| New crate | `crates/efe-meta-swarm-proto/` (~ 1200 LOC, including ~700 LOC of `.proto` with inline doc) |
| Proto files | 4 : `messages.proto`, `errors.proto`, `service_meta_swarm.proto`, `service_meta_participant.proto` |
| Services | 2 : `MetaSwarmService` (7 RPCs) + `MetaParticipantService` (6 RPCs) |
| Features | `serde-derives`, `client-only`, `server-only`, `full` |
| Workspace | +1 member, +4 deps in `[workspace.dependencies]` (tonic, tonic-build, prost, prost-types) |
| README | versioning policy + field-number allocation table + breaking-change protocol |
| Smoke test | 10 tests pinning package name, version, RPC count, default constructibility, serde round-trip |

## Test plan

- [x] `cargo build -p efe-meta-swarm-proto` (default features) succeeds
- [x] `cargo build -p efe-meta-swarm-proto --features full` succeeds
- [x] `cargo build -p efe-meta-swarm-proto --no-default-features --features client-only` succeeds
- [x] `cargo build -p efe-meta-swarm-proto --no-default-features --features server-only` succeeds
- [x] `cargo test -p efe-meta-swarm-proto --features full` : 10/10 PASS
- [x] `cargo doc -p efe-meta-swarm-proto --features full --no-deps` : 0 warnings
- [x] `cargo clippy -p efe-meta-swarm-proto --features full --all-targets` : 0 warnings on this crate
- [x] Phase-1 master gate still GREEN
- [x] `actionlint` on workspace workflows still GREEN

## Non-regression statement

- ✅ Zero file modified outside `crates/efe-meta-swarm-proto/`,
  `Cargo.toml`, `Cargo.lock`.
- ✅ Phase 1 unaffected : `cargo test -p efe-meta-swarm` still passes
  (199 default / 209 attention-weighted / 202 core-integration).
- ✅ `efe-server` unaffected : `cargo check -p efe-server` compiles
  byte-identical to pre-PR.
- ✅ Carve-out R5-B1 respected : `efe-meta-swarm-proto.publish = false`
  + advisory license = "Apache-2.0" (legal review tag remains UNLICENSED).

## Honest semantics

- The crate is **wire-protocol-only**.  No service impl → all client
  RPCs panic if pointed at a non-existent server.  Server traits exist
  but have no impl.  Both are populated by PRs #6, #8, #9.
- `StepStream` (server-streaming on participants) is **optional** :
  Phase-2 controllers default to `Step` ; `StepStream` is gated by
  `EFE_META_SWARM_USE_STEP_STREAM=true`.  Decision OQ2 in
  [phase-2 roadmap § 15](docs/EFE_A1_PHASE2_ROADMAP.md#15-open-questions-per-sprint).
- f32 only on the wire ; f64 is a Phase-3 opt-in (decision OQ1).

## Forward-compat

- Field numbers 1-99 + enum values 0-99 are frozen for the life of v1.
- Field numbers ≥ 100 are reserved blocks ; new fields go there.
- A v2 surface (if needed) lives at `efe.v2.meta_swarm` ; v1 stays
  alive for ≥ 6 months after v2 GA.

## Reviewer guidance

Suggested order :
1. [`docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md) — read this first (the blueprint for this PR).
2. `crates/efe-meta-swarm-proto/README.md` — versioning + field allocation.
3. The 4 `.proto` files in order : messages, errors, service_meta_swarm, service_meta_participant.
4. `build.rs` and `Cargo.toml` (small, prescriptive).
5. `src/lib.rs` — module aliases + constants.
6. `tests/smoke_cross_crate.rs` — what the surface promises.

Required approvers : 1× meta-swarm-team + 1× sec-team + 1× platform-team.

## References

- [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](docs/EFE_A1_PHASE2_ARCHITECTURE.md) § 4 (proto design)
- [`docs/EFE_A1_PHASE2_ROADMAP.md`](docs/EFE_A1_PHASE2_ROADMAP.md) § 2.1 (Sprint A1.P2.1 plan)
- [`docs/EFE_A1_ROADMAP.md`](docs/EFE_A1_ROADMAP.md) § 5 (Phase 2 in the global roadmap)
- [`docs/security/PHASE2_THREAT_MODEL.md`](docs/security/PHASE2_THREAT_MODEL.md) (sibling PR #7)
```

---

## 22. Risk analysis (PR-local)

| ID | Risk | L/I | Mitigation |
|---|---|---|---|
| PR5-R1 | tonic-build's bundled-protoc fails on a CI runner with no glibc | L/M | Pin tonic-build 0.12 ; document in README that no host `protoc` is needed ; if it fails, add `protoc-bin-vendored` explicitly. |
| PR5-R2 | A reviewer asks for f64 on the wire | M/L | Reference decision OQ1 = f32 ; refer to Phase 3 for f64 opt-in. |
| PR5-R3 | `cargo doc` emits warnings on tonic-generated code | M/M | Crate-level allow list in `Cargo.toml` for the known offenders ; if doc warnings exceed 5, treat as a regression and re-pin. |
| PR5-R4 | Field number allocation table drifts from the proto file content | M/L | CI check in a follow-up PR (#15+) ; for now, manual reviewer check. |
| PR5-R5 | Cargo lockfile pulls a transitive with a known CVE | L/H | sec-team review of the lockfile diff ; reject PR if any RUSTSEC advisory matches. |
| PR5-R6 | `cargo build` takes > 60 s on warm cache (slow tonic-build) | L/L | Acceptable for a build script run once ; warm cargo cache speeds it to ~ 5 s. |
| PR5-R7 | A future PR forgets to update `RPC_NAMES` | M/M | Smoke test `rpc_count_pinned` ; mutation testing would CI-fail. |

All risks tracked in `#meta-swarm-risks` until PR merge ; closed
post-merge.

---

## 23. Reviewer checklist

A reviewer can step through the PR with this checklist :

### General

- [ ] PR title follows convention : `feat(<scope>): <summary>`.
- [ ] PR description follows template (§ 21).
- [ ] PR is composed of 5 distinct commits, each CI-green.
- [ ] PR touches ONLY `crates/efe-meta-swarm-proto/`, `Cargo.toml`, `Cargo.lock`.

### Workspace integration

- [ ] `Cargo.toml` adds 1 workspace member.
- [ ] `[workspace.dependencies]` adds tonic, tonic-build, prost, prost-types.
- [ ] All version pins match what's documented in
      [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md) § 13.2.
- [ ] `Cargo.lock` diff is < 100 lines (sanity).

### Crate metadata

- [ ] `crates/efe-meta-swarm-proto/Cargo.toml` :
  - [ ] `publish = false`.
  - [ ] `version = "0.2.0"`.
  - [ ] `rust-version = "1.88"`.
  - [ ] All 4 features declared : `serde-derives`, `client-only`, `server-only`, `full`.

### Proto files

- [ ] `messages.proto` :
  - [ ] All 28 messages from architecture § 4 present.
  - [ ] Each message has a `reserved N to 99` block at the end.
  - [ ] `TensorF32` uses `repeated float data [packed = true]`.
  - [ ] `MetaSpaceConfig.safety_geometry` is a `oneof` with `box`, `poly`, `hull`.
- [ ] `errors.proto` :
  - [ ] All 18 error codes from architecture § 4.4 present.
  - [ ] Reserved blocks for Phase 3 (5000-9999).
- [ ] `service_meta_swarm.proto` :
  - [ ] 7 RPCs : Iterate, IterateStream, StepOnce, Reset, ReloadConfig, Health, Info.
  - [ ] Each RPC has an inline doc comment.
  - [ ] `import "efe/v1/meta_swarm/messages.proto"` at the top.
- [ ] `service_meta_participant.proto` :
  - [ ] 6 RPCs : Step, Reset, State, StepStream, Capabilities, Health.
  - [ ] Each RPC has an inline doc comment.

### Build script

- [ ] `build.rs` lists all 4 proto files.
- [ ] `cargo:rerun-if-changed=` emitted for each.
- [ ] `tonic_build::configure()` calls : `build_client`, `build_server`, `compile_well_known_types(false)`.
- [ ] Feature-gated serde derive attribute injection.

### lib.rs

- [ ] `#![forbid(unsafe_code)]` at the top.
- [ ] `pub mod meta_swarm` containing `tonic::include_proto!`.
- [ ] `pub mod controller` + `pub mod participant` aliases.
- [ ] Three public constants : `PROTO_VERSION`, `PROTO_PACKAGE`, `RPC_NAMES`.
- [ ] `RPC_NAMES.len() == 13`.
- [ ] `pub mod conv` with `tensor_from_slice` + `tensor_validate`.

### README

- [ ] License section (UNLICENSED + R5-B1 reference).
- [ ] Cargo features table.
- [ ] Versioning policy table (rows match § 15).
- [ ] Field-number allocation table (rows match the actual `reserved` blocks in the protos).
- [ ] Breaking-change protocol section.

### Smoke test

- [ ] 10 tests in `tests/smoke_cross_crate.rs` all PASS.
- [ ] No test takes > 50 ms.
- [ ] `cargo test -p efe-meta-swarm-proto --features full` GREEN.

### CI

- [ ] All CI jobs GREEN on the latest push.
- [ ] Phase-1 master gate still GREEN.
- [ ] Clippy 0 warnings on this crate.

### Sign-off

- [ ] 1× meta-swarm-team approval.
- [ ] 1× sec-team approval (with comment confirming threat-model alignment).
- [ ] 1× platform-team approval (with comment confirming Helm-chart will be able to consume).

---

## 24. Definition of Done

PR #5 is done iff :

1. All 24 acceptance criteria (§ 20) GREEN.
2. All 3 reviewer approvals in.
3. Merged to `main` with a merge commit message linking to this doc.
4. The PR description is the source-of-truth for the changelog entry.
5. Slack post in `#meta-swarm-standup` : `PR #5 merged ; PRs #6, #8, #9, #11 unblocked`.

PR #5 is **NOT** done if :

- Any AC is RED.
- Any reviewer has a blocking comment unresolved.
- Phase-1 master gate is RED (must triage and fix first).
- The PR was force-pushed in a way that loses review comments.

---

## 25. Post-merge follow-ups

These are NOT part of PR #5 ; they unblock once PR #5 lands :

| Follow-up | PR | Owner | When |
|---|---|---|---|
| Server scaffold using the proto crate | PR #6 | meta-swarm-team | Same day as #5 merge |
| `MetaSwarmService::Iterate` wiring | PR #8 | meta-swarm-team | Day 2 after #5 merge |
| `MetaParticipantService::Step` wiring | PR #9 | meta-swarm-team | Day 2 |
| Multi-process integration test | PR #10 | meta-swarm-team | Day 5 |
| `efe-server` env-var plumbing | PR #11 | meta-swarm-team | Sprint 2 W3 |
| `tools/loadgen` | PR #20 | meta-swarm-team | Sprint 3 W6 |
| Public bench harness | PR #21 | meta-swarm-team | Sprint 3 W6 |

## Appendix A — File-by-file deliverables checklist

```
crates/efe-meta-swarm-proto/
├── Cargo.toml                                   [✓ in this PR]
├── build.rs                                     [✓ in this PR]
├── README.md                                    [✓ in this PR]
├── proto/
│   └── efe/v1/meta_swarm/
│       ├── messages.proto                       [✓ in this PR]
│       ├── errors.proto                         [✓ in this PR]
│       ├── service_meta_swarm.proto             [✓ in this PR]
│       └── service_meta_participant.proto       [✓ in this PR]
├── src/
│   └── lib.rs                                   [✓ in this PR]
└── tests/
    └── smoke_cross_crate.rs                     [✓ in this PR]

Cargo.toml (workspace)                            [✓ touched in this PR : +1 member, +4 deps]
Cargo.lock                                        [✓ regenerated in this PR]
```

## Appendix B — Glossary

- **PR #5** : This PR ; the scaffold of the proto crate.
- **Tonic** : The Rust gRPC framework used by EFE.
- **prost** : The protobuf code generator used by tonic.
- **Wire-protocol** : The serialised representation of messages on the network.
- **Field number** : The integer tag used by protobuf to identify a field on the wire.  Numbers are precious ; never re-use.
- **Reserved block** : A `reserved N to M` clause in a proto message that prevents reuse of those field numbers.
- **OQ1, OQ2** : Open questions tracked in
  [`EFE_A1_PHASE2_ROADMAP.md`](EFE_A1_PHASE2_ROADMAP.md) § 15.  OQ1 = f32/f64 (decided f32) ; OQ2 = StepStream optional (decided optional).
- **R5-B1 carve-out** : The legal arrangement that keeps the meta-swarm crates UNLICENSED until Phase 5 ; documented in
  [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md).
