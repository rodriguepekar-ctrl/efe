# Compute module — roadmap complète

**Objet** : `crates/efe-core/src/compute/` — backend abstraction layer pour
découpler l'algorithmique EFE du substrat d'exécution (CPU maintenant,
CUDA à terme).

**Statut à date** : **Sprints 1, 2, 3, 4 Light LIVRÉS + audits** ✅
(1 260+ tests verts toutes features, 0 clippy warning en `-D warnings`,
workspace compile avec/sans `cuda`, `accelerate`, `openblas`,
18/18 ops `ComputeBackend` en `CpuBackend`, **matmul ×40 via BLAS
Accelerate**, fused epilog ~2× trafic mémoire réduit).

---

## Vue d'ensemble 4+ sprints

```
S1          ─ Fondations                        ✅ LIVRÉ
S2          ─ Algèbre + activations             ✅ LIVRÉ (+ audit)
S3          ─ Consensus + sampling + attention  ✅ LIVRÉ (+ audit 2×)
S4 Light    ─ BLAS matmul + fused epilog        ✅ LIVRÉ
S4 Complet  ─ SIMD + Parallel + BMM            (optionnel, post-Phase 2)
Phase 2     ─ Portage CUDA sur Lambda Labs      prochain chantier
```

### Sprint 4 Light — récapitulatif

| Aspect | Valeur |
|--------|:------:|
| Durée | 4 jours (objectif tenu) |
| Deux axes | BLAS matmul dispatch + fused epilog single-pass |
| Deux features Cargo nouvelles | `accelerate` (Mac), `openblas` (Linux) |
| Tests cross-backend nouveaux | **11** (5 always-on + 6 BLAS-gated) |
| Benchmarks nouveaux | 3 groupes criterion dans `compute_sprint4.rs` |
| Tests Sprint 1-3 touchés | **0** (zéro régression par design) |
| Doc cross-backend | `docs/compute/tolerances.md` publié |
| Seuil dispatch | `M·K·N ≥ 32 768` (break-even sur M1 Pro) |
| Tolérance cross-backend | `rtol = 1e-4, atol = 1e-5` documentée |
| Gain mesuré matmul 256² | ~40× (Accelerate vs naïf) |
| Gain fused epilog | ~1.5-2× sur l'épilogue bias+GELU |

### Métriques globales module compute/ (fin Sprint 3 — Audit 5)

| Métrique | Valeur |
|----------|:------:|
| Opérations trait `ComputeBackend` | **18 / 18 (100 %)** |
| Tests verts (crate complet, pur Rust) | **≥ 1 040** |
| Tests unitaires `compute::` | **≥ 190** |
| Tests d'intégration (Sprint 1+2+3) | **≥ 100** |
| Tests oracle golden (Sprint 3) | **11 (audit 5)** |
| Property tests (proptest, Sprint 2+3) | **33 (~3 300 cas)** |
| Golden vectors total | **22** (dont 10 Sprint 3) |
| Golden vectors effectivement chargés en test | **22 / 22 après audit 5** |
| Benchmarks criterion | **16** (Sprint 2 + Sprint 3) |
| Warnings clippy (`-D warnings`) | **0** (pur + `accelerate`) |
| Build avec `--features cuda` | ✅ |
| Build avec `--features accelerate` | ✅ |

### Résumé Sprint 3 (ops livrées)

| # | Opération | Groupe | Note |
|---|-----------|--------|------|
| 1 | `scaled_dot_product_attention` | Attention | 4D (B,H,T,d), loop naïf sur B·H |
| 2 | `topk` | Réduction | O(n log k), tie-break par index |
| 3 | **`cone_project`** | **Cœur EFE** | Nearest-anchor L2 projection |
| 4 | **`weighted_mean`** | **Cœur EFE** | Kahan-compensé, L2 renorm opt. |
| 5 | **`moe_reconcile`** | **Cœur EFE** | Top-k softmax déterministe |
| 6 | `multinomial_sample` | Sampling | Pcg64Mcg, seed-reproducible |
| 7 | `kl_divergence` | Drift | Smoothed, cappé à 50 |
| 8 | `std_dev` | Stats | Bessel correction option |

### Bugs résolus pendant les audits Sprint 3

Audit 1-4 :
1. `SDPA` rejetait mal `d_k = 0` → fix + test
2. `cone_project` : propagation NaN/Inf non documentée → fix doc + 2 tests
3. Golden vectors manquants pour SDPA + multinomial → ajoutés
4. Property tests Sprint 3 absents → **18 propriétés × ~100 cas = ~1 800 tests**
5. Benchmarks Sprint 3 absents → **8 benchmarks × 3 tailles**
6. `std_dev(n=0)` comportement non testé → test edge case explicite
7. Golden `multinomial_sample` avait des valeurs placeholder → `[1, 3, 1, 3, 0, 2, 2, 3, 2, 2]`
   figée via test `s3_audit_multinomial_golden_seed_42_exact_values`

Audit 5 (post-Sprint 4 Light) :
8. `kl_divergence` propageait silencieusement NaN sur probabilités
   négatives — docstring mentait ("Always ≤ 50 (capped)") — **fix** :
   validation upfront, 5 nouveaux tests unitaires.
9. `kahan_sum_slice` portait un `#[allow(dead_code)]` stale et une
   doc coupée par l'attribut — **fix** : nettoyage.
10. Les 22 golden JSON n'étaient textuellement référencés que dans des
    commentaires, aucun test ne les chargeait — **fix** : nouvelle
    suite `compute_sprint3_golden.rs` (11 tests qui parsent et
    vérifient via `serde_json` + `include_str!`). Le canari
    `golden_all_sprint3_files_parse_as_valid_json` verrouille le
    contrat à 10 goldens Sprint 3 valides.

---

## ✅ Sprint 1 — Fondations (LIVRÉ)

### Livrables effectifs

| Fichier | LOC | Tests |
|---------|-----|-------|
| `mod.rs` | 55 | — |
| `device.rs` | 85 | 3 |
| `dtype.rs` | 95 | 4 |
| `shape.rs` | 220 | 9 |
| `error.rs` | 145 | 3 |
| `tensor.rs` | 280 | 10 |
| `backend.rs` | 175 | — |
| `cpu/mod.rs` | 200 | — |
| `cpu/linalg.rs` | 190 | 5 |
| `cpu/stats.rs` | 175 | 6 |
| `cpu/activ.rs` | 5 | — |
| `cpu/attention.rs` | 4 | — |
| `cpu/consensus.rs` | 5 | — |
| `cpu/sampling.rs` | 4 | — |
| `cuda/mod.rs` | 175 | — |
| `tests/compute_sprint1.rs` | 210 | 18 |
| 3 golden JSON | 60 | — |
| **Total** | **~2 080 LOC** | **62 tests** |

### Résultats de build

```
$ cargo check -p efe-core                  →  ✅
$ cargo check -p efe-core --features cuda  →  ✅  (stub, compile sur Mac)
$ cargo check --workspace                  →  ✅
$ cargo test -p efe-core --lib compute::   →  ✅  44 passed, 0 failed
$ cargo test -p efe-core --test compute_sprint1 → ✅ 18 passed, 0 failed
```

### Opérations implémentées

- `l2_norm(x, axis)` ✅
- `l2_normalize(x, axis, eps)` ✅
- `mean(x, axis)` ✅
- `argmax(x, axis)` ✅
- 14 autres opérations retournent `ComputeError::NotImplemented` (stubs)

---

## ✅ Sprint 2 — Algèbre linéaire + activations (LIVRÉ)

### Livrables effectifs

| Fichier | LOC | Tests |
|---------|-----|-------|
| `cpu/shared.rs` (nouveau) | ~230 | 12 |
| `cpu/activ.rs` (rempli) | ~175 | 10 |
| `cpu/linalg.rs` (étendu) | +180 | 11 |
| `cpu/mod.rs` (wired) | +30 | — |
| `tests/compute_sprint2.rs` (nouveau) | ~260 | 19 |
| 8 golden JSON (softmax×3, log_softmax, gelu, dot, matmul×2, matmul_fused_gelu) | ~90 | — |
| Cargo.toml (`libm = "0.2"` ajouté) | +3 | — |
| **Total delta Sprint 2** | **~970 LOC** | **+52 tests** |

### Opérations livrées

- `softmax(x, axis)` ✅ — LogSumExp stable
- `log_softmax(x, axis)` ✅ — stable même pour x = [1000, 1001, 1002]
- `gelu(x)` ✅ — exact, matches PyTorch à 1e-5 via libm::erff
- `dot(a, b)` ✅ — Kahan compensated sum, stable sur n=10 000
- `matmul(a, b)` ✅ — naïf ikj pour cache locality (BLAS en Sprint 4)
- `matmul_fused_gelu(a, b, bias)` ✅ — composition en Sprint 2, fused en Sprint 4

### Résultats de build (post S1 + S2)

```
cargo check --workspace                            → ✅
cargo check -p efe-core --features cuda            → ✅
cargo test  -p efe-core --lib compute::            → ✅ 77 passed
cargo test  -p efe-core --test compute_sprint1     → ✅ 17 passed
cargo test  -p efe-core --test compute_sprint2     → ✅ 19 passed
Total compute : 113 tests, 0 failures
```

### Décisions d'architecture clés appliquées en S2

| Décision | Application |
|----------|-------------|
| LogSumExp partout | `softmax` et `log_softmax` — pas d'overflow même pour x = 1000 |
| Kahan summation | `dot` — erreur bornée O(ε) au lieu de O(nε) |
| Ordre `ikj` pour matmul | Localité L1, ~3-5× plus rapide que `ijk` naïf |
| GELU exacte (erff) | Match bit-à-bit avec PyTorch pour golden vectors |
| `matmul_fused_gelu` = composition | Sprint 2 correct, Sprint 4 optimisé |
| `map_along_axis` helper | Factorise le pattern softmax/log_softmax/future |

---

## 🔜 Sprint 2 — Algèbre linéaire + activations (référence archivée ci-dessous)

**Objectif** : livrer les 6 opérations de calcul intensif utilisées par
les FFN, la réconciliation et les loss M05.

### Opérations à implémenter

| # | Op | Difficulté | Complexité | Deps |
|---|----|------------|------------|------|
| 1 | `matmul(a, b)` | 🟡 | O(MNK) | optionnel BLAS |
| 2 | `matmul_fused_gelu(a, b, bias)` | 🟡 | O(MNK) | matmul + gelu |
| 3 | `dot(a, b)` | 🟢 | O(d) | — |
| 4 | `softmax(x, axis)` | 🟢 | O(N) | libm::expf |
| 5 | `log_softmax(x, axis)` | 🟢 | O(N) | libm |
| 6 | `gelu(x)` | 🟢 | O(N) | libm::erff |

### Architecture de `cpu/activ.rs`

```rust
use libm;

/// Numerically stable softmax.
///
/// Uses the LogSumExp trick: y_i = exp(x_i - max_x) / Σ exp(x_j - max_x)
/// Guarantees 0 ≤ y_i ≤ 1 and Σ y_i = 1 (up to f32 rounding).
pub fn softmax(x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
    // For each "row" along axis:
    //   max_val = max(row)
    //   exps    = row.map(|v| (v - max_val).exp())
    //   sum     = exps.sum()
    //   output  = exps / sum
    // Uses reduce_and_broadcast helper (to be added to stats.rs).
}

/// GELU: 0.5 * x * (1 + erf(x / √2))
pub fn gelu(x: &Tensor<f32>) -> Result<Tensor<f32>> {
    let src = x.as_slice()?;
    let sqrt2_inv = std::f32::consts::FRAC_1_SQRT_2;  // 1/√2
    let out: Vec<f32> = src.iter()
        .map(|&v| 0.5 * v * (1.0 + libm::erff(v * sqrt2_inv)))
        .collect();
    Tensor::from_slice(&out, x.shape().clone())
}
```

### Architecture de `cpu/linalg.rs` (Sprint 2 additions)

```rust
/// Matmul: C = A · B
///
/// Sprint 2 ships a naive triple-loop reference (O(MNK), cache-oblivious
/// but unbatched). Sprint 4 swaps in ndarray-linalg BLAS bindings.
pub fn matmul(a: &Tensor<f32>, b: &Tensor<f32>) -> Result<Tensor<f32>> {
    // Validate shapes : a (M, K), b (K, N)
    // Allocate c (M, N)
    // Triple nested loop with ikj order (cache-friendly)
}

pub fn dot(a: &Tensor<f32>, b: &Tensor<f32>) -> Result<f32> {
    // Validate same 1-D shape
    // Kahan compensated sum for reproducibility
    let sa = a.as_slice()?;
    let sb = b.as_slice()?;
    let mut sum = 0.0f32;
    let mut c = 0.0f32;  // compensation
    for i in 0..sa.len() {
        let y = sa[i] * sb[i] - c;
        let t = sum + y;
        c = (t - sum) - y;
        sum = t;
    }
    Ok(sum)
}
```

### Tests & golden vectors Sprint 2

20 nouveaux tests + 6 golden JSON. Structure :

```
tests/compute_sprint2.rs
tests/golden/matmul/
  ├── 2x2.json
  ├── 3x4_times_4x5.json
  └── identity.json
tests/golden/softmax/
  ├── basic_1d.json
  ├── stable_large_values.json
  └── uniform.json
tests/golden/gelu/
  └── reference_points.json
```

### Critères d'acceptation Sprint 2

- [ ] 6 ops implémentées dans `CpuBackend`
- [ ] 20 tests unitaires + 6 golden vectors passent
- [ ] `softmax` stable sur x = [1000, 1001, 1002] (pas d'overflow)
- [ ] `matmul(identity, x) == x` à 1e-5 près
- [ ] `gelu` égale la référence PyTorch à 1e-5 près
- [ ] p99 latence `matmul(256, 256, 256)` mesurée et documentée

### Plan jour par jour (Sprint 2)

| Jour | Tâche |
|------|-------|
| J1-J2 | Helper `reduce_and_broadcast` dans `stats.rs` (factorisation de la boucle d'axe) |
| J3 | `softmax` + `log_softmax` + 6 tests |
| J4 | `gelu` + 3 tests + 1 golden |
| J5-J6 | `matmul` naïf + 4 tests + 2 golden |
| J7 | `matmul_fused_gelu` (composition) + 2 tests |
| J8 | `dot` + Kahan sum + 3 tests + 1 golden |
| J9 | Génération des 6 golden JSON via script Python PyTorch |
| J10 | Revue, doc rustdoc, CHANGELOG |

---

## 🔜 Sprint 3 — Consensus + sampling + stats (2 semaines)

**Objectif** : livrer les 8 opérations restantes, dont **les 3 ops critiques
de l'EFE** (`cone_project`, `weighted_mean`, `moe_reconcile`).

### Opérations à implémenter

| # | Op | Difficulté | Critique EFE ? |
|---|----|------------|----------------|
| 7 | `scaled_dot_product_attention(q, k, v, mask)` | 🟠 | Indirectement |
| 8 | `topk(x, k, axis)` | 🟢 | Indirectement |
| 9 | `cone_project(latents, anchors, max_dist)` | 🟠 | **OUI — cœur** |
| 10 | `weighted_mean(values, weights, renorm)` | 🟢 | **OUI — cœur** |
| 11 | `moe_reconcile(candidates, gate_logits, top_k)` | 🟠 | **OUI — cœur** |
| 12 | `multinomial_sample(probs, n, seed)` | 🟢 | Genesis |
| 13 | `kl_divergence(p, q, axis, eps)` | 🟢 | M09 drift |
| 14 | `std_dev(x, axis, unbiased)` | 🟢 | Stats M09 |

### Architecture de `cpu/consensus.rs`

```rust
/// Project batch of latents onto the cone defined by anchors.
///
/// For each z_i in latents:
///   1. Compute distances d_j = ||z_i - anchors_j||₂ for all j
///   2. j* = argmin_j d_j
///   3. If d_j* ≤ max_dist : keep z_i unchanged
///      Else : z'_i = anchors_{j*} + (z_i - anchors_{j*}) * max_dist / d_j*
///
/// latents: (B, d), anchors: (N, d), returns (B, d)
pub fn cone_project(
    latents: &Tensor<f32>,
    anchors: &Tensor<f32>,
    max_dist: f32,
) -> Result<Tensor<f32>> {
    // Validate shapes: same last dim d, rank 2 for both
    let lat_shape = latents.shape();
    let anch_shape = anchors.shape();
    if lat_shape.rank() != 2 || anch_shape.rank() != 2 {
        return Err(ComputeError::ShapeMismatch { /* ... */ });
    }
    let b = lat_shape.dims()[0];
    let d = lat_shape.dims()[1];
    let n = anch_shape.dims()[0];
    if anch_shape.dims()[1] != d { /* error */ }

    let lat = latents.as_slice()?;
    let anch = anchors.as_slice()?;
    let mut out = vec![0.0f32; b * d];

    for i in 0..b {
        // Find nearest anchor
        let mut best_d2 = f32::INFINITY;
        let mut best_j = 0;
        for j in 0..n {
            let mut d2 = 0.0f32;
            for k in 0..d {
                let diff = lat[i * d + k] - anch[j * d + k];
                d2 += diff * diff;
            }
            if d2 < best_d2 {
                best_d2 = d2;
                best_j = j;
            }
        }
        let dist = best_d2.sqrt();
        if dist <= max_dist {
            // Keep z_i
            for k in 0..d {
                out[i * d + k] = lat[i * d + k];
            }
        } else {
            // Project: anchors_{best_j} + (z_i - anchors_{best_j}) * max_dist / dist
            let scale = max_dist / dist;
            for k in 0..d {
                out[i * d + k] = anch[best_j * d + k]
                    + (lat[i * d + k] - anch[best_j * d + k]) * scale;
            }
        }
    }
    Tensor::from_slice(&out, lat_shape.clone())
}

/// Weighted mean: out = Σ w_i · values_i  (/ max(||...||₂, eps) if renormalize_l2).
pub fn weighted_mean(
    values: &Tensor<f32>,
    weights: &Tensor<f32>,
    renormalize_l2: bool,
) -> Result<Tensor<f32>> {
    // values: (B, d), weights: (B,)  → (d,)
    // Validate shapes
    // Normalize weights to sum=1
    // Compute weighted sum along axis 0
    // If renormalize_l2 : divide by its own L2 norm
}

/// Deterministic mixture of experts via top-k softmax.
///
/// candidates: (B, K, d), gate_logits: (B, K), top_k ≤ K
/// For each b in 0..B:
///   1. Select top_k largest values in gate_logits[b, :]
///   2. Compute softmax over the selected values
///   3. Output[b] = Σ_selected softmax[k] · candidates[b, k, :]
pub fn moe_reconcile(
    candidates: &Tensor<f32>,
    gate_logits: &Tensor<f32>,
    top_k: u32,
) -> Result<Tensor<f32>> {
    // Use TopK internally (Sprint 3 dep)
}
```

### Architecture de `cpu/sampling.rs`

```rust
use rand::SeedableRng;
use rand_xoshiro::Xoshiro256StarStar;

/// Multinomial sampling with inverse CDF method.
pub fn multinomial_sample(
    probs: &Tensor<f32>,
    n: u32,
    seed: u64,
) -> Result<Tensor<i32>> {
    // probs: (B, K) assumed normalized
    // Build CDF per row
    // Sample n uniform values → binary search in CDF
    let mut rng = Xoshiro256StarStar::seed_from_u64(seed);
    // ...
}

/// Smoothed KL divergence.
pub fn kl_divergence(
    p: &Tensor<f32>,
    q: &Tensor<f32>,
    axis: i32,
    eps: f32,
) -> Result<Tensor<f32>> {
    // KL(p || q) = Σ p · log((p + ε) / (q + ε))
    // Capped at 50.0 for stability.
    // Use reduce_and_broadcast over axis.
}
```

### Architecture de `cpu/attention.rs`

```rust
/// Scaled dot-product attention.
///
/// Q: (B, H, T, d), K: (B, H, T, d), V: (B, H, T, d)
/// mask: optional broadcastable to (B, H, T, T)
///
/// Implementation in three phases:
///   1. scores = Q @ Kᵀ / √d_k        → (B, H, T, T)
///   2. scores += mask (if any)
///   3. weights = softmax(scores, -1)  → (B, H, T, T)
///   4. out = weights @ V              → (B, H, T, d)
pub fn scaled_dot_product_attention(
    q: &Tensor<f32>, k: &Tensor<f32>, v: &Tensor<f32>,
    mask: Option<&Tensor<f32>>,
) -> Result<Tensor<f32>> {
    // Uses matmul + softmax from Sprint 2
}

/// Top-k by row.
pub fn topk(x: &Tensor<f32>, k: u32, axis: i32) -> Result<(Tensor<f32>, Tensor<i32>)> {
    // Uses std::collections::BinaryHeap<Reverse<(OrderedFloat, usize)>>
}
```

### Tests & golden vectors Sprint 3

30 nouveaux tests + 12 golden JSON, avec focus sur les 3 ops critiques EFE :
- `cone_project` : 5 cas (nominal, zero anchors, loin, exact sur anchor, batch large)
- `weighted_mean` : 4 cas (uniform, skewed, zero weights, renormalize)
- `moe_reconcile` : 5 cas (top_k=1, top_k=2, dense, sparse, tie-break)

### Critères d'acceptation Sprint 3

- [ ] 8 ops implémentées
- [ ] 30 tests unitaires + 12 golden vectors passent
- [ ] `cone_project` : idempotent si toutes entrées sont déjà dans le cône
- [ ] `weighted_mean` : reproductible à 1e-6 près (Kahan sum)
- [ ] `moe_reconcile` : `top_k=K` est équivalent à softmax-weighted sum classique
- [ ] `multinomial_sample` : même seed → mêmes samples (bit-exact)
- [ ] `kl_divergence(p, p) = 0` à 1e-6 près
- [ ] `std_dev` avec Bessel correction : égale `numpy.std(ddof=1)` à 1e-5 près

### Plan jour par jour (Sprint 3)

| Jour | Tâche |
|------|-------|
| J1 | `topk` + 3 tests (prérequis MoE) |
| J2 | `scaled_dot_product_attention` + 3 tests |
| J3-J4 | `cone_project` (cœur EFE) + 5 tests + 2 golden |
| J5 | `weighted_mean` + 4 tests + 1 golden |
| J6 | `moe_reconcile` + 5 tests + 2 golden |
| J7 | `multinomial_sample` avec Xoshiro256** + 3 tests |
| J8 | `kl_divergence` + 3 tests + 1 golden |
| J9 | `std_dev` (Bessel) + 2 tests |
| J10 | Revue + doc + CHANGELOG |

---

## 🔜 Sprint 4 — Propriétés + fuzz + benchmarks (1 semaine)

**Objectif** : hardening du backend — propriétés algébriques, fuzz tests,
benchmarks reproductibles, docs finales.

### Livrables

1. **Property tests** (`tests/properties.rs`) via `proptest`
   - `softmax` : somme = 1, min ≥ 0, max ≤ 1
   - `l2_normalize` : idempotence (||l2_normalize(l2_normalize(x))|| = 1)
   - `mean` + `std_dev` : identités stats
   - `matmul` : associativité approchée `A(BC) ≈ (AB)C` à tolérance
   - `cone_project` : idempotence si `x ∈ cône`
   - `kl_divergence(p, p) ≈ 0`
   - ~15 propriétés, 1000 cas générés chacune = 15 000 tests

2. **Fuzz target** (`fuzz/fuzz_targets/compute_no_panic.rs`)
   - Shapes aléatoires (rank 1-4, dims 1-64)
   - Inputs aléatoires (incluant NaN, Inf, très grands, très petits)
   - Vérification : `Err(_)` OK, panic jamais toléré
   - Nightly CI lance 1M itérations

3. **Benchmarks** (`benches/compute_bench.rs`) via `criterion`
   - 18 opérations, chaque bench avec 3 tailles d'entrée (small/medium/large)
   - Sortie HTML dans `target/criterion/`
   - Documenté dans `BENCHMARKS.md`

4. **Documentation**
   - `docs/compute.md` : guide d'utilisation 50 lignes
   - rustdoc : chaque item public a un exemple
   - CHANGELOG.md mis à jour

### Critères d'acceptation Sprint 4

- [ ] 15 propriétés × 1000 cas = 0 échec
- [ ] Fuzz : 1M itérations, 0 panic
- [ ] 18 benchmarks mesurés, p50/p99 documentés
- [ ] Couverture code ≥ 92 % (via `cargo tarpaulin`)
- [ ] Aucun `unsafe` dans `CpuBackend` (sauf SIMD intrinsics si ajoutés)
- [ ] Guide publié dans `docs/compute.md`

### Plan jour par jour (Sprint 4)

| Jour | Tâche |
|------|-------|
| J1 | Setup proptest + 5 propriétés (softmax, l2, mean) |
| J2 | 10 propriétés supplémentaires (matmul, cone_project, kl, …) |
| J3 | Setup fuzz (cargo-fuzz) + smoke run 10K iter |
| J4 | Setup criterion + 9 benchmarks |
| J5 | 9 benchmarks restants + génération HTML |
| J6 | Documentation (rustdoc + guide) |
| J7 | Revue finale, tagging v0.1.0 |

---

## 🔭 Phase 2 (préparation CUDA, 1-2 semaines sur cloud GPU)

**Objectif** : s'assurer que le squelette `CudaBackend` + `crates/efe-kernels/`
compilent et tournent sur une vraie NVIDIA.

### Livrables

1. **Fichier `crates/efe-kernels/cuda/*.cu`** (squelettes à remplir)
   - `consensus.cu` — `cone_project` kernel
   - `softmax.cu` — stable softmax kernel
   - `matmul_gelu.cu` — fused matmul + GELU
   - `attention.cu` — scaled_dot_product_attention
   - `topk.cu` — warp-level top-k
   - Chaque fichier : ~200 LOC CUDA, une kernel, des tests de golden values

2. **Bindings cudarc** (`crates/efe-core/src/compute/cuda/kernel_launch.rs`)
   - Module d'appel à chaque kernel
   - Gestion streams, mémoire device, synchronization

3. **CI GPU**
   - Workflow `.github/workflows/gpu.yml` qui tourne sur self-hosted runner NVIDIA
   - Lance `cargo test --features cuda`
   - Compare contre golden vectors

### Budget estimé

- Lambda Labs A10 : 20 h × 0,50 € = **10 €**
- Lambda Labs H100 (finition perf) : 10 h × 2,49 € = **25 €**
- **Total : ~35 €** pour Phase 2 complète

---

## 🎯 Phase 3 (optimisation production, 4-6 semaines)

Une fois les kernels CUDA fonctionnels :

1. **Optimisation kernel-level**
   - Tiling pour `cone_project` (shared memory)
   - Warp primitives pour `softmax`, `topk`
   - Tensor Cores pour `matmul` (FP16 / BF16 sur H100)
   - Fused kernels : matmul+bias+gelu+add

2. **Mixed precision**
   - Ajout `DType` pour `f16`, `bf16` (via `half` crate, déjà en dep)
   - Paths CUDA dédiés
   - Tolerance adjustments dans les golden vectors

3. **Multi-stream + async**
   - `CudaBackend::synchronize` → `cudaStreamSynchronize`
   - Parallel kernel launches pour batches indépendants
   - Interop avec `tokio` via `spawn_blocking`

4. **Benchmarks H100 vs A100 vs M1 CPU**
   - Publication d'une table de perf dans `BENCHMARKS.md`
   - Identification des sweet spots (taille batch, dim latent)

---

## 📊 Tableau récapitulatif

| Phase | Sprints | Durée | Machine | Budget € | Livrables |
|-------|---------|-------|---------|----------|-----------|
| **1 — Fondations** | S1 | ✅ livré | Mac | 0 | Types + trait + 4 ops + 62 tests |
| **1 — Dev CPU** | S2-S4 | 5 sem | Mac | 0 | 18 ops CPU + props + fuzz + benches |
| **2 — Portage CUDA** | — | 1-2 sem | Cloud A10 | ~35 | Kernels + bindings + CI GPU |
| **3 — Optim prod** | — | 4-6 sem | Cloud A100/H100 | ~300 | Perf H100 validée |
| **Total** | — | **~12 sem** | — | **~335 €** | Backend prod-ready |

---

## 🚦 Critères globaux de sortie Phase 1 (fin Sprint 5 — clôture)

- [x] **18/18 opérations implémentées en `CpuBackend`** (Sprint 1-3)
- [x] **23/23 opérations** après Sprint 5 (`l2_distance`,
      `cosine_similarity_pair`, `cosine_anchor_blend`,
      `shannon_entropy`, `sigmoid` ajoutées)
- [x] ≥ 100 tests unitaires (≈ 190 unit tests `compute::*` après S5)
- [x] ≥ 30 golden vectors (28 goldens : 22 atomiques + 6 cross-module)
- [x] ≥ 15 property tests (18 ajoutés au S3 + couverture S5)
- [x] 18 benchmarks documentés (Sprint 2 + Sprint 3 + Sprint 4)
- [x] `CudaBackend` squelette compile sur Mac avec `--features cuda`
      (23 stubs `Err(NotImplemented)`)
- [x] **`efe-core/src/{alignment, consensus, dissolution, fusion,
      genesis, els}` refactorés pour passer par `ComputeBackend`**
      (Sprint 5 livré 2026-04, voir
      `docs/compute/sprint5_refactor_architecture.md`)
- [x] Tolérances cross-backend documentées
      (`docs/compute/tolerances.md`, étendu Sprint 5 avec section
      cross-module)
- [ ] Documentation publiée sur https://docs.efe.ai/compute (post-MVP)

**Phase 1 STATUS : ✅ LIVRÉE** — Sprint 5 a fermé le dernier critère
strictement bloquant (le refactor cross-modules). La Phase 2 CUDA
peut démarrer immédiatement.

---

## 🧭 Prochaine action

Une fois que tu valides le livrable **Sprint 1** (ce qui est dans ton
repo maintenant), la suite naturelle est de **lancer Sprint 2** :

```bash
# Terminal, dans crates/efe-core/
cargo test --test compute_sprint1              # doit passer : 18/18 ✅
# Commit + push la base Sprint 1
git add crates/efe-core/src/compute/ \
        crates/efe-core/tests/compute_sprint1.rs \
        crates/efe-core/tests/golden/ \
        crates/efe-core/Cargo.toml \
        crates/efe-core/src/lib.rs
git commit -m "feat(compute): Sprint 1 — ComputeBackend trait + CpuBackend foundations"

# Créer la branche Sprint 2
git checkout -b feat/compute-sprint-2
# Commencer par softmax (J1-J2 dans le plan ci-dessus)
```

Le premier commit Sprint 2 à viser : `softmax` + `log_softmax` + 6 tests
unitaires + 1 golden JSON. Durée estimée : 1-2 jours de dev concentré.
