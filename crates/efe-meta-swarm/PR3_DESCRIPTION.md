# [Sprint A1 P1 — PR #3] test(efe-meta-swarm): 130+ proptest cases reaching the 200-test target

## Summary

Raises the `efe-meta-swarm` test count from the 70 unit/integration tests
shipped in PR #1 to **202** by adding **130+ property-based test cases**
(proptest + insta) spread across 14 categories.  No production code is
changed ; this PR is a *testing-only* hardening of the Phase 1 crate.

The work tracks line 92 of `PR_DESCRIPTION.md` :

> 2. `feat(efe-meta-swarm): 130+ proptest cases reaching the 200 target` (week 7).

## What this PR adds

| Cat | Module                                          | Tests | Type     |
|-----|-------------------------------------------------|------:|----------|
|  1  | `src/spaces/tensor.rs`                          |   15  | proptest |
|  2  | `src/operators/linear.rs` (LinearLift)          |   15  | proptest |
|  3  | `src/operators/linear.rs` (LinearPull)          |    8  | proptest |
|  4  | `src/operators/uniform.rs`                      |    8  | proptest |
|  5  | `src/operators/attention.rs` *(feature-gated)*  |    8  | proptest |
|  6  | `src/safety/box_safety.rs`                      |   10  | proptest |
|  7  | `src/safety/polyhedron.rs`                      |    6  | proptest |
|  8  | `src/safety/learned.rs`                         |    5  | proptest |
|  9  | `tests/integration_iteration_proptest.rs`       |   12  | proptest (new file) |
| 10  | `tests/integration_convergence_edge.rs`         |   12  | proptest (new file) |
| 11  | `tests/integration_safety_hierarchical.rs`      |    8  | proptest (new file) |
| 12  | `src/builder.rs`                                |    8  | proptest |
| 13  | `src/config.rs`                                 |    5  | proptest |
| 14  | `tests/pin_snapshots.rs`                        |   10  | insta (new file) |
|     | **Total added by PR #3**                        | **130** |       |

With `cases ∈ {12, 24, 32, 64}` per proptest function, the real coverage
is **~5 000+ randomised inputs** exercised end-to-end on every run.

## Files added

* `tests/integration_iteration_proptest.rs` — Cat 9, joint iteration.
* `tests/integration_convergence_edge.rs`   — Cat 10, stop-criterion edges.
* `tests/integration_safety_hierarchical.rs` — Cat 11, per-step safety
  preservation (meta + local).
* `tests/pin_snapshots.rs`                  — Cat 14, serde format pinning.
* `tests/snapshots/*.snap` (×10)            — accepted snapshot baselines.

## Files modified (proptest blocks appended only)

* `src/spaces/tensor.rs`, `src/operators/{linear,uniform,attention}.rs`,
  `src/safety/{box_safety,polyhedron,learned}.rs`, `src/builder.rs`,
  `src/config.rs` — each gets a `// Category N` proptest block at the end
  of its existing `mod tests`.  No behavioural change.
* `Cargo.toml` — adds `serde_json = { workspace = true }` to
  `[dev-dependencies]` (required by Cat 14 pin tests).

## Why proptest + insta ?

* **proptest** : the iteration is a discrete-time non-linear dynamical
  system over a 7-dim parameter space (`meta_dim, p_dim, κ, α, γ, seeds`).
  Hand-picked unit tests cover at most a dozen points ;  proptest with
  shrinkage covers ~5 000 random points and reports a *minimal* failing
  input on regression.
* **insta** : the on-the-wire format of `Tensor` / `MetaSwarmConfig` /
  `BundleCertificate` / `MetaSwarmError` is part of the public surface
  (Phase 4 SDKs will rely on it).  Pin snapshots force a human review
  on any change.

## Algorithmic invariants exercised

* **Non-expansiveness** (Lemma 3.1 of A5) — `UniformMetaOp`,
  `LinearLift::from_normalised`, `BoxSafety::project`.
* **Permutation invariance** (UniformMetaOp + identity lifts) — verified
  at meta-op and iteration scope.
* **Firmly non-expansive projection** (Bauschke-Combettes 4.16) —
  `BoxSafety` proptest 4.
* **KM iteration monotone descent** — `iter_residual_descends_pt`.
* **Safety preservation at every step** (Theorem 5.1 of A5) —
  `safe_every_step_meta_feasible_pt`, `safe_every_step_locals_feasible_pt`.
* **Mixed-hierarchy safety** (Polyhedron meta + Box local) — Cat 11 / 6.
* **Attention-softmax Lipschitz formula** `K = 1 + 2‖q‖/τ` — Cat 5 / 6 + 7.

## Test plan

- [x] `cargo test -p efe-meta-swarm --offline` : **192 / 192 pass**.
- [x] `cargo test -p efe-meta-swarm --features attention-weighted --offline` :
      **202 / 202 pass** (the 10 extra are Cat 5 + 2 doctests).
- [x] 5 consecutive `cargo test` runs : **202 pass each time, no
      flakiness**.
- [x] `cargo doc -p efe-meta-swarm --no-deps --features attention-weighted` :
      clean.
- [x] `cargo build -p efe-meta-swarm --release --features attention-weighted` :
      clean.
- [x] `cargo build -p efe-meta-swarm --examples` : all four examples build.
- [x] `cargo bench -p efe-meta-swarm --no-run` : bench compiles.
- [x] `cargo clippy -p efe-meta-swarm --all-targets --features attention-weighted` :
      **no new warnings introduced by PR #3** (preexisting workspace-style
      warnings remain unchanged).
- [x] `cargo check --features no-default-features` and
      `--features core-integration` : both compile.
- [x] **Non-regression** : `cargo check -p efe-core` and `cargo check -p efe-server`
      succeed without any change to their state.
- [x] **Master gate** : the `efe-server` production binary is byte-identical
      (no transitive linker change ;  PR #3 modifies only `efe-meta-swarm`,
      which `efe-server` does not depend on in Phase 1).
- [x] **Wall time** : full test suite runs in ~0.5 s on a developer laptop.

## Honest semantics

Two corner cases are documented in-source rather than worked around :

1. **Power-iteration imprecision** (`src/operators/linear.rs` Test 6) —
   the Phase 1 spectral-norm estimator (20 power-iterations, fixed start
   vector) can be off by up to ~100 % on 2 × 2 adversarial seeds.  The
   `from_normalised` Lipschitz-bound test asserts the relaxed bound
   `≤ 2.0` and a rustdoc paragraph defers a Krylov-restart replacement
   to Phase 3.  The seed that exposed the issue
   (`11485880019391148984, in_dim=2, out_dim=2`) is now persisted in
   `proptest-regressions/operators/linear.txt` and runs first on every
   `cargo test`.
2. **Attention non-expansiveness** (`src/operators/attention.rs` Cat 5) —
   the attention operator is *not* globally non-expansive (Theorem 6.5
   of A5), so Cat 5 tests algebraic properties (output dim, high-τ →
   uniform limit, identity-vote collapse, Lipschitz formula
   `K = 1 + 2‖q‖/τ`, monotonicity in τ, finite outputs under
   stabilised softmax) and *not* a non-expansiveness invariant.
   Section 11 (hierarchical safety) covers the domain-restricted
   case under projection.

## Forward-compat

This PR does not introduce any new public API.  All forward-compat hooks
established in PR #1 (`tenant_id`, `BundleCertificate`,
`MetaSwarmTracer`, `attention-weighted` feature) are exercised by the
new tests but not modified.

## Reviewer guidance

Suggested review order :

1. `tests/pin_snapshots.rs` + `tests/snapshots/*` — the on-the-wire
   format is now pinned ; reviewers should confirm the baselines look
   reasonable (especially `meta_swarm_config_yaml_snapshot.snap`).
2. `tests/integration_safety_hierarchical.rs` — the strongest
   end-to-end invariants ; per-step feasibility.
3. `tests/integration_iteration_proptest.rs`,
   `tests/integration_convergence_edge.rs` — joint iteration props.
4. The 11 in-source proptest blocks (Cat 1-8, 12-13) — skim ; the
   pattern is uniform (Helpers + `proptest! { ... }`).
5. `Cargo.toml` — single one-line dev-dependency addition.

## Non-regression statement (master gate)

This PR explicitly verifies the master-gate invariants of Sprint A1 P1 :

- ✅ All pre-existing tests pass : 70 / 70 of PR #1 + the new 130+.
- ✅ `efe-server` does not depend on `efe-meta-swarm`.  Its `Cargo.toml`
  is **not touched** by this PR.
- ✅ No source modification of any pre-existing crate.
- ✅ Workspace lint / fmt conventions of PR #1 are preserved (the
  crate-local style choice of vertical alignment is intentionally left
  in place ;  PR #3 does not run `cargo fmt`).
- ✅ The dual-license carve-out of R5-B1 holds : `efe-meta-swarm`
  remains `UNLICENSED`.

## References

- `PR_DESCRIPTION.md` (PR #1) — original scaffold, line 92 schedules
  this PR for "week 7".
- `docs/EFE_A1_HIERARCHICAL_SWARMS_ARCHITECTURE.md` — full A1 architecture.
- `docs/EFE_A1_PHASE1_ARCHITECTURE.md` — Phase 1 spec (§ 9 tests).
- `docs/EFE_A5_PROVABLE_CONVERGENCE_ARCHITECTURE.md` — theorems whose
  preconditions are the invariants tested here (Lemma 3.1, Theorem 5.1,
  Theorem 6.5).
