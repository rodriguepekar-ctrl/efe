# [Sprint A1 P1] feat(efe-meta-swarm): scaffold hierarchical multi-swarm crate

## Summary

Introduces the `efe-meta-swarm` crate (Sprint A1 Phase 1 — *Core
implementation*).  Provides the in-process scaffolding for hierarchical
(meta-)swarm coordination : multiple EFE swarms collaborating via a
shared latent meta-space.

This PR is **scaffold-only** : no other crate in the workspace depends
on `efe-meta-swarm`, no production binary changes, and no behavioural
change is exposed to clients.  Activation will be progressively opened
in Phases 2-5 along the L0 → L5 roadmap.

## What this PR adds

| Area | Detail |
|---|---|
| New crate | `crates/efe-meta-swarm/` — 19 source modules, 3 810 LOC Rust |
| Public API | `SwarmParticipant`, `Lift`, `Pull`, `MetaOperator`, `SafetySet` traits + `MetaSwarm` + `MetaSwarmBuilder` |
| Algorithm | Synchronous coupled iteration (Eq. 1* of A1 architecture), `rayon`-parallelised per step |
| Phase-1 impls | `MockParticipant`, `LinearLift`, `LinearPull`, `UniformMetaOp`, `AttentionMetaOp` (feature-gated), `BoxSafety`, `PolyhedronSafety` (Dykstra-style), `LearnedSafety` |
| Tests | 70 (65 unit + 3 integration + 2 doctest) — `cargo test -p efe-meta-swarm` |
| Examples | `two_swarms_uniform`, `three_swarms_kovant`, `multimodal_three_swarms`, `safety_violation_attempt` |
| Bench | `three_swarms_kovant_latency` (Criterion ; target p99 < 100 ms) |
| Forward-compat hooks | `MetaSwarmTracer` trait, `BundleCertificate` enum, `tenant_id: Option<TenantId>`, `attention-weighted` feature |
| Workspace | `+10 lines` to root `Cargo.toml` (member entry with comment block) |

## Test plan

- [x] `cargo check -p efe-meta-swarm --offline` — passes in 4.32 s.
- [x] `cargo check -p efe-core --offline` — non-regression, passes in 2.52 s.
- [x] `cargo test -p efe-meta-swarm --offline` — 70 / 70 pass.
- [x] `cargo metadata --offline` — workspace resolves cleanly.
- [x] Example `three_swarms_kovant` converges in 47 iterations / 13.5 ms on dev box.
- [x] No file modification to any pre-existing crate.
- [ ] *(deferred to Phase 1 week 3)* `LocalParticipant<S: efe_core::Swarm>` adapter.
- [ ] *(deferred to Phase 2)* gRPC service `MetaSwarmService` + sub-chart Helm.

## Architecture

Phase 1 ships an **in-process scaffold** under a `meta-swarm` Cargo
feature (off by default).  The seven architecture decisions of the
Phase 1 doc are reflected in the code :

* **DA-1** — adapter pattern : `SwarmParticipant` trait separates the
  meta-swarm from `efe-core`'s internals.  In Phase 1 we ship only
  `MockParticipant` to keep the crate self-contained ;  the
  `LocalParticipant<S: efe_core::Swarm>` adapter lands in week 3.
* **DA-2** — dynamic dimensions, validated at `MetaSwarmBuilder::build()`.
* **DA-3** — fluent builder with deferred validation.
* **DA-4** — synchronous barrier-based iteration via `rayon::par_iter`.
* **DA-5** — `MetaOperator` is a trait (open extension).
* **DA-6** — safety projections are pluggable.
* **DA-7** — Cargo feature `meta-swarm` not enabled on `efe-server` in
  Phase 1.  Workspace member only.

## Forward-compat checklist for Phases 2-5

| Future feature | Hook in this PR |
|---|---|
| Phase 2 — gRPC `MetaParticipantService` | `SwarmParticipant` trait already abstract enough for a `RemoteParticipant` |
| Phase 3 — `AdminService.SetTenantFeature(meta_swarm)` | `MetaSwarmBuilder::tenant_id(TenantId)` field present, currently ignored |
| Phase 3 — `efe-prove` certificates | `BundleCertificate` enum with `Stub` / `LocalSignature` / `SigstoreBundle` variants |
| Phase 3 — OpenTelemetry | `MetaSwarmTracer` trait + `NoopTracer` default |
| Phase 4 — SDK clients v0.4.0 | YAML `MetaSwarmConfig` Serde-deserialisable |
| Phase 5 — Attention-weighted production | `attention-weighted` feature, gated, compile-time choice |

## Non-regression statement (master gate)

This PR explicitly verifies the master-gate invariants of Sprint A1 P1 :

- ✅ All pre-existing tests pass : `cargo check -p efe-core --offline` succeeds.
- ✅ `efe-server` does not depend on `efe-meta-swarm`.  Its `Cargo.toml` is **not touched**.
- ✅ The 30 closed defects of sprint Φ-D-OPS (cf. `docs/audits/phi_d_ops_closure.md`) remain closed — none of them touched files in `crates/efe-meta-swarm/`.
- ✅ The dual-license carve-out of R5-B1 holds : this crate is `UNLICENSED` (workspace policy), consistent with all other `crates/*` ;  Apache-2.0 is reserved for `clients/*`.

## Reviewer guidance

Suggested review order :

1. `Cargo.toml` (root)        — verify the single member-list addition.
2. `crates/efe-meta-swarm/Cargo.toml`  — verify workspace deps + lints alignment.
3. `crates/efe-meta-swarm/src/lib.rs`  — module map and forward-compat narrative.
4. `crates/efe-meta-swarm/src/iteration/coupled.rs`  — the algorithm.
5. `crates/efe-meta-swarm/src/builder.rs` — validation at build time.
6. Skim the rest.

## Next PRs (follow-up in Phase 1)

1. `feat(efe-meta-swarm): add LocalParticipant<S: efe_core::Swarm> adapter` (week 3).
2. `feat(efe-meta-swarm): 130+ proptest cases reaching the 200 target` (week 7).
3. `feat(efe-meta-swarm): bench baseline tracking + master-gate hook` (week 8).
4. `docs: Phase-1 end-of-sprint progress report` (week 8).

## References

- `docs/EFE_A1_HIERARCHICAL_SWARMS_ARCHITECTURE.md` — full A1 architecture
- `docs/EFE_A1_PHASE1_ARCHITECTURE.md` — Phase 1 spec
- `docs/EFE_A1_ROADMAP_USER_ACTIVATABLE.md` — 5-tier activation roadmap
- `docs/EFE_A5_PROVABLE_CONVERGENCE_ARCHITECTURE.md` — theoretical foundation (convergence theorem)
- `docs/audits/phi_d_ops_closure.md` — non-regression baseline (Sprint Φ-D-OPS)
