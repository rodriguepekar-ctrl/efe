# Sprint 3 — Analyses numériques et mathématiques

> Complément à `crates/efe-core/src/compute/ROADMAP.md` et suite à
> `sprint2_math.md`. Couvre les **8 opérations** livrées en Sprint 3,
> dont les **3 opérations cœur EFE** (`cone_project`, `weighted_mean`,
> `moe_reconcile`).

---

## 1. Vue d'ensemble des 8 opérations

| # | Op | Groupe | Rôle EFE |
|---|----|--------|----------|
| 1 | `scaled_dot_product_attention` | Attention | Réconciliation multi-tête |
| 2 | `topk` | Réduction | Support MoE, sélection |
| 3 | **`cone_project`** | Consensus | **Projecteur d'alignement** ⭐ |
| 4 | **`weighted_mean`** | Consensus | **Agrégation d'essaim** ⭐ |
| 5 | **`moe_reconcile`** | Consensus | **Routeur top-k** ⭐ |
| 6 | `multinomial_sample` | Sampling | Genesis, sélection stochastique |
| 7 | `kl_divergence` | Probabilités | M09 drift detection |
| 8 | `std_dev` | Statistiques | Analyse de distribution |

---

## 2. `cone_project` — projection radiale sur union de boules

### 2.1 Formulation

Soit `L = {z_i}_{i=0..B}` les latents et `A = {a_j}_{j=0..N}` les anchors,
tous dans `ℝ^d`. Le cône d'alignement est :

```
    C(A, r) = ⋃_j B(a_j, r)   avec r = max_dist
```

où `B(a_j, r) = { x ∈ ℝ^d : ‖x − a_j‖₂ ≤ r }`.

Pour chaque latent `z_i` :

1. **Anchor le plus proche** :

   ```
       j*(i) = argmin_j  ‖z_i − a_j‖₂
       d_min(i) = ‖z_i − a_{j*(i)}‖₂
   ```

2. **Projection radiale** vers l'anchor le plus proche :

   ```
       z'_i = ⎧ z_i                                            si d_min(i) ≤ r
              ⎨ a_{j*(i)} + r · (z_i − a_{j*(i)}) / d_min(i)  sinon
              ⎩
   ```

### 2.2 Propriétés géométriques

* **Contractive** : pour tout `z`, `‖proj(z) − a_{j*}‖₂ ≤ ‖z − a_{j*}‖₂`.
* **Idempotente** : `proj(proj(z)) = proj(z)`, car `proj(z) ∈ C(A, r)`.
* **Continue** sauf aux frontières de Voronoi (changement de `j*`).
  En pratique la fonction est continue Lipschitz sur chaque cellule de Voronoi.

### 2.3 Pourquoi la comparaison en distance au carré

Le code utilise `best_sq` (somme des carrés) et compare à `max_dist²`.
Ceci évite un `sqrt` par anchor :

```
    best_sq ≤ max_dist²   ⟺   √best_sq ≤ max_dist   (pour valeurs non-négatives)
```

Seule la meilleure distance (l'anchor sélectionné) nécessite un `sqrt`
final pour calculer le facteur de mise à l'échelle. Gain : **`N−1`
`sqrt` évités par latent**.

### 2.4 Comportement avec inputs dégénérés

| Input | Comportement |
|-------|--------------|
| `max_dist = 0` | `Err(NumericalInstability)` — géométriquement mal défini |
| `max_dist < 0` ou `NaN`/`Inf` | `Err(NumericalInstability)` |
| `N = 0` anchors | `Err(ShapeMismatch)` |
| latent exactement sur un anchor (`d_min = 0`) | `0 ≤ r` vrai → `z_i` conservé, pas de division par 0 |
| latent à `+Inf` | `d_min = +Inf`, `scale = r/∞ = 0` → output contient NaN (propagé) |
| anchor avec NaN | anchor jamais sélectionné (NaN < x est faux) ; si TOUS les anchors sont NaN, propagation |

### 2.5 Complexité

* **Temps** : `O(B · N · d)` — pas de structure spatiale (kd-tree etc.) en Sprint 3
* **Espace** : `O(B · d)` pour l'output ; pas de workspace intermédiaire
* **Sur M1 Pro, `B=128, N=10, d=128`** : ~ 200 µs (Sprint 3) → optimisable en Sprint 4 Complet via rayon

---

## 3. `weighted_mean` — moyenne pondérée Kahan-compensée

### 3.1 Formulation

Pour `v : (B, d)`, `w : (B,)` avec `∑ w_b > 0` :

```
    μ_k = (∑_b w_b · v_{b,k}) / (∑_b w_b)   pour k ∈ [0, d)
```

Si `renormalize_l2` est activé :

```
    μ ← μ / max(‖μ‖₂, ε)    avec ε = 1e-9
```

### 3.2 Pourquoi Kahan par dimension

Le dénominateur `∑ w_b` est calculé une seule fois via Kahan.
**Le numérateur de chaque dimension k a sa propre accumulation
Kahan indépendante.**

Pourquoi ? Parce que `μ_k` ne dépend que de la colonne `k` de `v`.
Faire Kahan par dim garantit que chaque `μ_k` a une erreur O(ε) au
lieu de O(B·ε). Pour `B = 10 000`, cela divise l'erreur par 10 000.

### 3.3 Invariants algébriques testés

| Propriété | Test |
|-----------|------|
| Uniform weights → mean | `weighted_mean(v, 1_B) ≡ mean(v, axis=0)` |
| Scale invariance | `weighted_mean(v, α·w) = weighted_mean(v, w)` pour `α > 0` |
| Single-weight concentration | `w = [0,…,1,…,0]` avec 1 à position `i*` → sortie = `v_{i*}` |
| Permutation invariance | Pas testé explicitement (serait un property test de plus) |

### 3.4 Cas d'erreur explicites

| Cas | Raison | Retour |
|-----|--------|--------|
| `∑ w ≤ 0` | Division par 0 ou négatif | `Err(NumericalInstability)` |
| `∑ w` non-fini (Inf/NaN) | Idem | `Err(NumericalInstability)` |
| `values` rank ≠ 2 | Précondition | `Err(ShapeMismatch)` |
| `weights` rank ≠ 1 | Précondition | `Err(ShapeMismatch)` |
| `v.B ≠ w.B` | Précondition | `Err(ShapeMismatch)` |

### 3.5 Pourquoi le `max(‖μ‖, 1e-9)` en renormalisation

Si `‖μ‖₂ = 0` (cas dégénéré : tous les `v_b` s'annulent sous les poids),
diviser par zéro donnerait Inf/NaN. Le plancher à `1e-9` garantit que
la sortie reste finite, même si elle n'est plus unit-norm dans ce cas
extrême. C'est un choix de *graceful degradation*.

---

## 4. `moe_reconcile` — mixture-of-experts top-k déterministe

### 4.1 Formulation

Pour chaque batch `b` :

1. **Sélection** : top-`k` indices des `gate_logits_b` (ties rompus par
   plus petit index).

   ```
       (values_b, indices_b) = topk(gate_logits_b, top_k)
   ```

2. **Normalisation** : softmax restreint aux top-k :

   ```
       w_b,ℓ = exp(values_b,ℓ − max_b) / ∑_m exp(values_b,m − max_b)
   ```

3. **Combinaison** :

   ```
       out_b,j = ∑_ℓ w_b,ℓ · candidates_{b, indices_b,ℓ, j}
   ```

### 4.2 Équivalences algébriques

* **`top_k = 1`** ⟺ `out_b = candidates[b, argmax(gate_logits_b), :]`
* **`top_k = K`** ⟺ `out_b,j = ∑_ℓ softmax(gate_logits_b)_ℓ · candidates[b, ℓ, j]`

Ces deux équivalences sont testées par property tests
(`moe_top_k_one_equals_argmax_candidate` et
`moe_top_k_equal_k_matches_full_softmax`).

### 4.3 Déterminisme du tie-breaking

Quand plusieurs logits ont la même valeur numérique, `topk_row` utilise
`Reverse(usize)` comme clé secondaire dans le heap. Résultat : l'index
**le plus petit** gagne. Conséquence :

```
    gate_logits = [0.5, 0.5, 0.5]    top_k = 1
    → indices = [0]
    → out = candidates[b, 0, :]
```

Cette règle est **bit-stable** : même inputs → même output, à travers
les re-runs, les plateformes, les versions du compilateur.

### 4.4 Stabilité numérique

Le softmax interne utilise le *LogSumExp trick* :

```
    max = max(values)
    w_ℓ = exp(values_ℓ − max) / ∑_m exp(values_m − max)
```

Ceci garantit que l'argument de `exp` est dans `(−∞, 0]`, donc
`exp` ∈ `(0, 1]` — pas d'overflow même si `gate_logits` contient
des valeurs de grande magnitude (comme les scores non-normalisés
de l'attention).

### 4.5 Défense NaN / Inf

```rust
let max_val = top_values.iter().fold(NEG_INFINITY, f32::max);
if !max_val.is_finite() { return Err(NumericalInstability); }
let sum_exp = ...;
if sum_exp <= 0.0 || !sum_exp.is_finite() { return Err(NumericalInstability); }
```

Cas couverts par les tests :
- All `-∞` gate → `max_val = -∞` → erreur
- All `+∞` gate → `max_val = +∞` → erreur
- Mixed NaN + finite → NaN propagé via `TotalF32` ordering, puis attrapé
  par `!max_val.is_finite()` OU `!sum_exp.is_finite()`

Tests correspondants : `moe_reconcile_all_neg_inf_gate_returns_error`,
`moe_reconcile_all_pos_inf_gate_returns_error`,
`s3_audit_moe_reconcile_mixed_nan_gate_errors`.

---

## 5. `scaled_dot_product_attention` — attention multi-tête

### 5.1 Formulation standard

```
    Attention(Q, K, V) = softmax( Q · Kᵀ / √d_k + mask ) · V
```

Avec `Q, K, V ∈ ℝ^(B × H × T × d_k)` et `mask ∈ ℝ^(T × T)` (additif).

### 5.2 Pourquoi le scaling par `1/√d_k`

Sans scaling, le produit scalaire `q · k^T` a une variance qui croît
avec `d_k` (si `q_i, k_i` sont iid de variance 1, alors `var(q · k) = d_k`).
Pour `d_k = 1024`, les scores atteignent typiquement `±32`, saturant
le softmax (qui sature à `|x| > 10` en f32).

Le facteur `1/√d_k` ramène la variance à ~1, préservant la dynamique
du softmax.

### 5.3 Décomposition en matmul + softmax + matmul

Le code Sprint 3 boucle explicitement sur `(batch, head)` et appelle
pour chaque pair :

```rust
for bi in 0..B {
    for hi in 0..H {
        let q_bh = extract_2d(&q, bi, hi);          // (T_q, d_k)
        let k_bh = extract_2d(&k, bi, hi);          // (T_k, d_k)
        let v_bh = extract_2d(&v, bi, hi);          // (T_v, d_v)
        let k_t = transpose_2d(&k_bh);              // (d_k, T_k)
        let scores = matmul(&q_bh, &k_t);           // (T_q, T_k)
        scale_inplace(&mut scores, 1 / sqrt(d_k));
        apply_mask(&mut scores, mask);
        let weights = softmax(&scores, -1);          // (T_q, T_k)
        let out_bh = matmul(&weights, &v_bh);        // (T_q, d_v)
        scatter_2d(&mut out, &out_bh, bi, hi);
    }
}
```

### 5.4 Impact de Sprint 4 Light sur SDPA

Les deux `matmul` internes dispatchent vers BLAS quand
`T · d_k · T_k ≥ 32 768`. Sur `B=1, H=1, T=64, d_k=64` cela vaut
`262 144`, largement au-dessus du seuil. **SDPA sous `--features
accelerate` devient ~5× plus rapide** grâce au BLAS, mais la
tolérance cross-backend monte à `2e-4` (composition de deux matmul
BLAS, chacun à `1e-4`).

### 5.5 Pourquoi rejeter `d_k = 0`

Si `d_k = 0`, le scaling factor devient `1/√0 = +∞`, propagé dans
tous les scores et déclenchant NaN après mask + softmax. Le code
rejette explicitement ce cas avec `Err(ShapeMismatch)`.

### 5.6 Mask conventionnel

* **Mask additif** : `scores += mask` avant softmax.
* **Bloquer une position** : mettre `-∞` à la position correspondante →
  `exp(-∞) = 0` → attention à cette position = 0.
* **Mask causal** : triangulaire supérieur de `-∞` → chaque position
  n'attend que les positions ≤ elle-même. Test :
  `s3_deep_sdpa_causal_mask_produces_lower_triangular_attention`.

---

## 6. `topk` — top-k avec tie-breaking déterministe

### 6.1 Algorithme min-heap

```
    heap: BinaryHeap<Reverse<(TotalF32, Reverse<usize>)>>    (max size k)
    pour chaque (i, v) dans row :
        entry = Reverse((TotalF32(v), Reverse(i)))
        si heap.len() < k :
            heap.push(entry)
        sinon si v est meilleur que heap.peek() :
            heap.pop()
            heap.push(entry)
    trier le résultat par valeur desc, ties par index asc
```

Complexité : `O(n · log k)` vs `O(n · log n)` pour un tri complet.
**Avantage** : `k = 5, n = 1 000 000` → 1M · 2.3 ops vs 1M · 20 ops ≈ 10×.

### 6.2 `TotalF32` et NaN

`f32` n'implémente pas `Ord` standard à cause de NaN. `TotalF32` wrap
f32 en utilisant `f32::total_cmp` (IEEE 754-2019 `totalOrder`), qui
traite `NaN > +∞`. Conséquences :

* `topk` avec NaN dans l'input : les NaN sont classés **au-dessus** de
  toutes les valeurs finies et apparaissent en premier dans le top-k.
* Downstream (`moe_reconcile`) doit détecter et rejeter ces NaN — ce
  que fait la vérification `is_finite()` après extraction.

### 6.3 Tie-breaking

Clé secondaire : `Reverse(usize)`. Pourquoi `Reverse` ? Parce que l'outer
`Reverse(...)` transforme le max-heap en min-heap. Pour que les **petits
indices** gagnent en cas d'égalité, il faut que les petits indices
soient **« plus petits »** pour le comparateur total, donc **« plus
grands »** après inversion. D'où le double `Reverse`.

Résultat : l'extraction triée donne valeurs desc, puis indices asc.

### 6.4 `k = dim` (boundary)

Test : `s3_deep_topk_boundary_k_equals_dim` — `k = dim` donne un tri
complet desc avec tie-breaking. C'est un cas utile pour valider que
le comportement à la borne est bien celui d'un `argsort`.

---

## 7. `multinomial_sample` — sampling catégoriel reproductible

### 7.1 Algorithme inverse CDF

```
    pour chaque batch b :
        normaliser row_b par sa somme → proba
        cdf_b = cumulative sum de proba
        pour chaque sample k in 0..n :
            u = uniform(0, 1) via Pcg64Mcg
            idx = binary_search(cdf_b, u)    (partition_point)
            out[b, k] = idx
```

Complexité : `O(B · K)` pour la normalisation + CDF,
puis `O(n · log K)` pour les `n` samples.

### 7.2 Contrat de reproductibilité

**RNG** : `Pcg64Mcg` (Permuted Congruential Generator, 128-bit state,
64-bit output, MCG variant). Passe BigCrush, cheap seeding.

**Contract** : pour tout `(probs, n, seed)` identique, l'output est
identique bit-à-bit sur :
* macOS aarch64 (Apple Silicon)
* Linux x86_64
* Linux aarch64

Cette propriété est testée par `s3_audit_multinomial_golden_seed_42_exact_values`
qui lock explicitement la séquence pour `seed=42, probs=[0.25;4], n=10` :

```
    [1, 3, 1, 3, 0, 2, 2, 3, 2, 2]
```

Si cette séquence change, c'est que `rand_pcg` a bumpé son
implémentation (très rare) ou qu'une régression a été introduite
dans le code appelant.

### 7.3 Anti-biais dans la conversion uniforme

```rust
let u = (f64::from(rng.next_u32()) / f64::from(u32::MAX) + 1e-12) as f32;
let u = u.min(1.0 - f32::EPSILON);
```

* `f64::from(rng.next_u32()) / f64::from(u32::MAX)` ∈ `[0, 1]`
* `+ 1e-12` garantit `u > 0` même si `rng.next_u32() = 0`.
  Pourquoi ? Parce que si `probs[0] = 0`, alors `cdf[0] = 0`, et
  `partition_point(|v| v < 0)` retournerait `0` à tort (index dont
  la probabilité est nulle).
* `u.min(1 - EPSILON)` empêche `u = 1.0` exactement, qui
  déclencherait un hors-borne dans `partition_point`.

### 7.4 Edge cases

| Cas | Comportement |
|-----|--------------|
| `n = 0` | Output shape `(B, 0)`, vide |
| Row toute à zéro | `Err(NumericalInstability)` (somme ≤ 0) |
| Row avec valeur négative | `Err(NumericalInstability)` |
| Probs non normalisées mais positives | Normalisées automatiquement |
| `K = 1` | Toujours retourne `[0]` |

---

## 8. `kl_divergence` — divergence lissée cap à 50

### 8.1 Formulation

```
    KL(p ‖ q) = ∑_i  p_i · log( (p_i + ε) / (q_i + ε) )
```

Capée à `max(0.0, min(50.0, KL))`.

### 8.2 Pourquoi le lissage ε

Sans `ε` :
* `q_i = 0, p_i > 0` → `log(+∞) = +∞` → KL = +∞
* `p_i = 0, q_i > 0` → `0 · log(0) = NaN` (indéterminé)

Avec `ε > 0` :
* `q_i = 0, p_i > 0` → `log((p+ε)/ε)` = fini (grand mais borné)
* `p_i = 0, q_i > 0` → `0 · log((ε)/(q+ε)) = 0` ✓ (correctement)

### 8.3 Pourquoi le cap à 50

Au-delà de KL ≈ 50, les distributions sont « effectivement disjointes »
pour toute application EFE (M09 drift). Plutôt que de laisser des
valeurs de `KL = 10^12` polluer les moyennes downstream, on cap à 50.

50 correspond à `exp(-50) ≈ 2 · 10⁻²²`, soit probabilité négligeable
à résolution f32.

### 8.4 Invariants

| Propriété | Vérifié par |
|-----------|-------------|
| `KL(p ‖ p) ≈ 0` | `s3_kl_self_is_zero` (integration) + `kl_self_is_zero` (property) |
| `KL ≥ 0` (Gibbs) | `kl_non_negative` (property, ajouté Sprint 3 audit 4) |
| Asymétrie | `s3_audit_kl_divergence_preserves_asymmetry` |
| Cap à 50 | `s3_kl_capped_at_50` |

### 8.5 Pièges numériques

* `(p_i + ε) / (q_i + ε)` : si `p_i` et `q_i` sont très petits devant
  `ε`, le ratio tend vers 1, `log(1) = 0`. Cela biaise légèrement
  l'estimation KL quand les distributions sont très sparse. En
  pratique avec `ε = 1e-9`, ce biais est négligeable pour des
  distributions bien conditionnées.
* `p_i < 0` ou non-fini : **durci à l'audit 5** — le code rejette
  désormais explicitement via `Err(NumericalInstability)` plutôt que
  laisser `log(négatif) = NaN` se propager. Contrat aligné avec
  `multinomial_sample`. Tests : `kl_rejects_negative_p`,
  `kl_rejects_negative_q`, `kl_rejects_nan_in_p`, `kl_rejects_inf_in_q`,
  `kl_always_finite_and_bounded_after_hardening`.

---

## 9. `std_dev` — déviation standard avec correction Bessel

### 9.1 Formulation

Soit `x_1, …, x_N` ∈ `ℝ` :

```
    μ = (1/N) · ∑ x_i
    σ² = (1/D) · ∑ (x_i − μ)²     avec D = N si biased, N−1 si unbiased
    σ = √σ²
```

### 9.2 Pourquoi la correction Bessel

L'estimateur biased `σ²_N = (1/N) · ∑ (x−μ̂)²` sous-estime la variance
vraie (car `μ̂` minimise précisément cette somme par définition).

L'estimateur unbiased `σ²_{N−1}` corrige ce biais d'un facteur `N/(N-1)`,
donnant un estimateur sans biais de la variance de la population.

Utilisation :
* **`unbiased = true`** (défaut recommandé) : matche `numpy.std(..., ddof=1)`,
  `torch.std(..., correction=1)`.
* **`unbiased = false`** : matche `numpy.std(..., ddof=0)`, utilisé
  quand on a accès à **toute** la population (pas un échantillon).

### 9.3 Invariants algébriques

| Propriété | Test |
|-----------|------|
| `std(c) = 0` (constant) | `std_dev_constant_is_zero` |
| `std(x + c) = std(x)` (shift) | `std_dev_shift_invariant` (property) |
| `std(α·x) = \|α\| · std(x)` (scale) | `std_dev_scale_linear` (property, ajouté audit 4) |
| Match numpy `ddof=1` | `std_dev_unbiased_matches_numpy` |
| Match numpy `ddof=0` | `std_dev_biased_matches_numpy` |

### 9.4 Edge cases critiques

| N | unbiased | Résultat |
|---|:-------:|----------|
| 0 | toute | NaN (réduction vide indéfinie) |
| 1 | true | NaN (division par N−1 = 0) |
| 1 | false | 0 (déviation trivialement nulle) |
| ≥ 2 | toute | Valeur standard |

---

## 10. Contrats de tolérance cross-backend (rappel Sprint 4 Light)

| Op Sprint 3 | Utilise matmul ? | Tolerance héritée BLAS |
|-------------|:-----------------:|:-----------------------:|
| `cone_project` | Non | `1e-5` (pur) |
| `weighted_mean` | Non | `1e-5` (pur) |
| `moe_reconcile` | Non (topk + softmax interne) | `1e-5` (pur) |
| `multinomial_sample` | Non | `EXACT` (seed-reproducible) |
| `kl_divergence` | Non | `1e-5` (pur) |
| `std_dev` | Non | `1e-5` (pur) |
| `topk` | Non | `EXACT` (tie-breaking déterministe) |
| **`scaled_dot_product_attention`** | **Oui ×2** | **`2e-4` (composé)** |

Voir `docs/compute/tolerances.md` pour les détails complets.

---

## 11. Comptage tests Sprint 3 (fin audit 5)

| Catégorie | Nombre | Fichier |
|-----------|:------:|---------|
| Unit tests (dans `cpu/*.rs`) | ~70 | attention.rs, consensus.rs, sampling.rs, stats.rs |
| Integration tests | 50 | `tests/compute_sprint3.rs` |
| Property tests | 18 | `tests/compute_sprint3_properties.rs` |
| Golden oracle tests (audit 5) | 11 | `tests/compute_sprint3_golden.rs` |
| Golden vectors Sprint 3 | 10 | `tests/golden/{cone_project, weighted_mean, …}/*.json` |
| Cross-backend tests (S4) | 7 | `tests/compute_sprint4_cross_backend.rs` |

**Total tests Sprint 3 + périphérique** : ~160 tests déterministes + ~1 800 cas générés par proptest.

### Nouveautés audit 5 (post-Sprint 4 Light)

1. **Durcissement `kl_divergence`** : validation upfront des probabilités
   négatives ou non-finies. 5 nouveaux tests unitaires dans `sampling.rs`.
2. **Harness d'oracle golden** : les 10 fichiers `tests/golden/**/*.json`
   Sprint 3 sont désormais chargés via `include_str!` + `serde_json` et
   comparés avec le backend sous tolérance. Avant l'audit 5, ces fichiers
   n'étaient que documentaires.
3. **Nettoyage `kahan_sum_slice`** : suppression du `#[allow(dead_code)]`
   stale et de la documentation coupée par l'attribut.

---

## 12. Références académiques

1. Kahan, W. (1965). *Pracniques: further remarks on reducing truncation errors.* CACM 8(1).
2. O'Neill, M. E. (2014). *PCG: A Family of Simple Fast Space-Efficient Statistically Good Algorithms for Random Number Generation.* Harvey Mudd College TR.
3. Bessel, F. W. *Untersuchungen über die Bestimmung der Bewegung der Planeten und Monde.* (1838) — correction de variance.
4. Vaswani, A. et al. (2017). *Attention is All You Need.* NeurIPS — formulation originale SDPA.
5. Shazeer, N. et al. (2017). *Outrageously Large Neural Networks: The Sparsely-Gated Mixture-of-Experts Layer.* ICLR — MoE routing.
6. Hendrycks, D. & Gimpel, K. (2016). *Gaussian Error Linear Units (GELUs).* (utilisé en Sprint 2 mais référencé ici).

---

*Fin du document Sprint 3. Prochain chapitre envisageable :
`sprint4_light_blas.md` avec analyse BLAS Accelerate vs naïf,
`phase2_cuda.md` avec les 8 kernels CUDA ciblés.*
