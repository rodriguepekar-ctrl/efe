# `efe-meta-swarm-proto`

Protocol Buffers + tonic-generated client/server stubs for the EFE
meta-swarm gRPC surface.

Part of **Sprint A1 Phase 2** (gRPC transport).  See
[`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_ARCHITECTURE.md)
for the full design and
[`docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md)
for this PR's blueprint.

> **License** : UNLICENSED — see workspace [LICENSE](../../LICENSE)
> + the R5-B1 carve-out in
> [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../../docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md).
> Target license at Phase 5 graduation : Apache-2.0.

## What this crate is

A **wire-protocol-only** crate.  Contains :

- 4 `.proto` files at the workspace root under
  `protos/efe/v1/meta_swarm/` (same layout as the other `efe.v1.*`
  surfaces).
- A `build.rs` that drives `tonic-build` to generate Rust client +
  server stubs into `$OUT_DIR/`.
- A thin `src/lib.rs` that re-exports the generated types under
  ergonomic module aliases.

## What this crate is NOT

- No service implementation.  See `crates/efe-meta-swarm-server/`
  (PR #6+).
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

```rust,no_run
use efe_meta_swarm_proto::{
    controller::MetaSwarmServiceClient,
    meta_swarm::{IterateRequest, IterationConfig, MetaSpaceConfig},
};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let mut client = MetaSwarmServiceClient::connect("https://controller:50080").await?;
let resp = client.iterate(IterateRequest::default()).await?;
println!("iterations = {}", resp.into_inner().iterations);
# Ok(())
# }
```

## Cargo features

| Feature         | Default | Purpose |
|-----------------|:-------:|---------|
| `serde-derives` |   off   | Derives `serde::Serialize` + `serde::Deserialize` on every generated message.  Used by the public bench HTML emitter. |
| `client-only`   |   off   | Drops the server stubs ; saves ~30 % binary size. |
| `server-only`   |   off   | Drops the client stubs ; mirror of above. |
| `full`          |   off   | Convenience : `serde-derives`.  Used by `cargo doc`. |

## Versioning policy

| Surface | Stability promise |
|---|---|
| Proto **package name** `efe.v1.meta_swarm` | Frozen for the life of the v1 API (≥ 12 months from v0.2.0 GA, expected 2026-08-31 → 2027-08-31 at minimum). |
| Proto **message field numbers** 1-99 | Frozen.  Never reassigned.  Never reused. |
| Proto **message field numbers** ≥ 100 | Reserved for v1 additions.  Allocated in this README. |
| Proto **enum values** 0-99 | Frozen. |
| Proto **enum values** ≥ 100 | Reserved for v1 additions. |
| Proto **service RPC names** | Frozen for the life of v1.  Adding a new RPC is backwards-compatible. |
| Proto **service RPC signatures** | Frozen ; adding a field to a request/response is OK (new tag). |
| Generated Rust **module paths** | Stable as long as tonic-build's API contract holds (currently 0.12). |
| Generated Rust **type names** | Stable. |
| `pub mod controller` / `pub mod participant` aliases | Stable for v1. |
| `PROTO_VERSION` / `PROTO_PACKAGE` / `RPC_NAMES` constants | Stable for v1. |

### Breaking-change protocol

If a backward-incompatible change is needed :

1. Open an RFC under `docs/rfcs/`.
2. Create `protos/efe/v2/meta_swarm/` (do not touch v1 files).
3. Implement both services side by side for ≥ 6 months.
4. Announce v1 deprecation date in the L4 or L5 graduation post.
5. Delete v1 in a Phase-6 PR (target : 2027-12-01 at earliest).

## Field-number allocation table

This README is the **source of truth** for field-number allocation
within v1.  Adding a new field requires :

1. Picking an unused number ≥ 100 within the message's reserved
   block.
2. Documenting the allocation in the table below in the same PR.
3. Adding a smoke test that pins the new field's wire identity.

| Message | Fields used | Reserved | Next free |
|---|---|---|---|
| `TensorF32` | 1, 2 | 3-9, 10-99 | 3 |
| `MetaSpaceConfig` | 1, oneof{2,3,4} | 5-99 | 5 |
| `BoxSafety` | 1 | 2-99 | 2 |
| `Polyhedron` | 1, 2, 3 | 4-99 | 4 |
| `LearnedHull` | 1, 2 | 3-99 | 3 |
| `ParticipantDescriptor` | 1-7 | 8-99 | 8 |
| `IterationConfig` | 1-6 | 7-9, 10-99 | 7 |
| `IterateRequest` | 1-5 | 6-99 | 6 |
| `IterateResponse` | 1-7 | 8-99 | 8 |
| `IterateChunk` | 1-3 | 4-99 | 4 |
| `StepOnceRequest` | — | 1-99 | 1 |
| `StepOnceResponse` | 1, 2 | 3-99 | 3 |
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
| `ReloadConfigRequest` | 1, 2, 3 | 4-99 | 4 |
| `ReloadConfigResponse` | 1-4 | 5-99 | 5 |
| `HealthRequest` | — | 1-99 | 1 |
| `HealthResponse` | 1-3 | 4-99 | 4 |
| `InfoRequest` | — | 1-99 | 1 |
| `InfoResponse` | 1-6 | 7-99 | 7 |

### Enum-value reservation

| Enum | Values used | Reserved | Next free |
|---|---|---|---|
| `StopReason` | 0-6 | 7-99 | 7 |
| `HealthResponse.Status` | 0-3 | 4-99 | 4 |
| `MetaSwarmErrorCode` | 0-7, 1001-1005, 2001-2003, 3001-3003, 4001-4003 | 8-99, 1006-1099, 2004-2099, 3004-3099, 4004-4099, 5000-9999 | 8 (general) ; 1006 (runtime) ; 2004 (reload) ; 3004 (discovery) ; 4004 (auth) |

## Repository layout

```
crates/efe-meta-swarm-proto/
├── Cargo.toml
├── build.rs
├── README.md                      ← you are here
├── src/
│   └── lib.rs                     ← re-exports + module aliases
└── tests/
    └── smoke_cross_crate.rs       ← cross-crate import test

protos/efe/v1/meta_swarm/          ← workspace-level protos
├── messages.proto
├── errors.proto
├── service_meta_swarm.proto
└── service_meta_participant.proto
```

## Building locally

```bash
# Default features.
cargo build -p efe-meta-swarm-proto

# All features (used by `cargo doc`).
cargo build -p efe-meta-swarm-proto --features full

# Client-only build (e.g. for a CLI tool).
cargo build -p efe-meta-swarm-proto --no-default-features --features client-only

# Server-only build.
cargo build -p efe-meta-swarm-proto --no-default-features --features server-only
```

## Documentation

```bash
cargo doc -p efe-meta-swarm-proto --features full --no-deps --open
```

## Tests

```bash
cargo test -p efe-meta-swarm-proto --features full
```

10 smoke tests assert the public surface contract (package name, version, RPC
count, default constructibility, encode/decode round-trip, serde JSON
round-trip when `serde-derives` is enabled).

## Cross-references

- Phase 2 architecture : [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_ARCHITECTURE.md)
- Phase 2 roadmap : [`docs/EFE_A1_PHASE2_ROADMAP.md`](../../docs/EFE_A1_PHASE2_ROADMAP.md)
- PR #5 (this PR) detailed architecture : [`docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md)
- Sister crate (Phase 1, in-process algorithms) : [`crates/efe-meta-swarm/README.md`](../efe-meta-swarm/README.md)
