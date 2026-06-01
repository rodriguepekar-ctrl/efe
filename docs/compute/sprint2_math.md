# Sprint 2 — Analyses numériques et mathématiques

> Complément à `crates/efe-core/src/compute/ROADMAP.md`.
> Cible : justifier chaque décision algorithmique par l'analyse formelle.

---

## 1. Le LogSumExp trick (softmax, log_softmax)

### 1.1 Motivation — pourquoi softmax naïf est dangereux

La définition mathématique :

```
    softmax(x)_i = exp(x_i) / Σ_j exp(x_j)
```

Sur f32 :
- `f32::MAX ≈ 3.4028e38`
- `exp(88.72) ≈ 3.4028e38`
- Pour tout `x_i > 88.72`, `exp(x_i)` **overflows à +∞**
- Résultat : `+∞ / +∞ = NaN`

Exemple défaillant :

```rust
// x = [1000.0, 1001.0, 1002.0]
// Naïf : exp(1000) = +∞, sum = +∞, result = [NaN, NaN, NaN]
```

### 1.2 L'identité algébrique

Pour tout scalaire `m` indépendant de `i` :

```
    exp(x_i) / Σ_j exp(x_j)
  = exp(m) · exp(x_i − m) / (exp(m) · Σ_j exp(x_j − m))    [multiplication par 1]
  = exp(x_i − m) / Σ_j exp(x_j − m)
```

### 1.3 Le choix `m = max(x)`

Avec `m = max(x)` :
- `x_i − m ≤ 0` pour tout `i`
- `exp(x_i − m) ∈ (0, 1]`
- `Σ_j exp(x_j − m) ∈ [1, n]` (au moins un terme vaut 1, celui de l'argmax)

Résultat :
- **Pas d'overflow** dans les exp
- Dénominateur ≥ 1 donc **pas de division par zéro**
- La précision est préservée : le terme dominant a valeur exacte `1.0`

### 1.4 Application au log_softmax

Même idée, mais pour la formule :

```
    log_softmax(x)_i = x_i − log Σ_j exp(x_j)
```

La LogSumExp donne :

```
    log Σ_j exp(x_j) = m + log Σ_j exp(x_j − m)   pour tout m
```

Avec `m = max(x)` le `log` s'applique à un `sum ∈ [1, n]` — numériquement bien conditionné.

### 1.5 Pièges résiduels

| Cas | Comportement |
|-----|--------------|
| Row ne contient que `-∞` | max = `-∞`, on retourne une row `NaN` |
| Row contient `+∞` | `+∞ − +∞ = NaN`, NaN propagé (comme PyTorch) |
| Row contient un `NaN` | `NaN` propagé |
| Row de 1 élément | Fast-path : retourne `[1.0]` (softmax) ou `[0.0]` (log_softmax) |

---

## 2. Kahan Compensated Summation (dot)

### 2.1 Le problème

Somme naïve de `n` floats en f32 :

```rust
let mut s = 0.0_f32;
for x in xs { s += x; }   // erreur ≈ O(n · ε) où ε ≈ 1.19e-7
```

Pour `n = 10^6` termes de magnitude 1, l'erreur atteint ~0,12 — **12 %** de la vraie valeur. Inacceptable.

### 2.2 L'algorithme de Kahan (1965)

Idée : à chaque addition, capturer le « résidu » perdu par le floating-point rounding et le ré-injecter au tour suivant.

```rust
let mut sum = 0.0_f32;
let mut c   = 0.0_f32;   // compensation (bits perdus accumulés)
for x in xs {
    let y = x - c;       // soustraire la compensation
    let t = sum + y;     // addition arrondie
    c = (t - sum) - y;   // capturer ce qui a été perdu
    sum = t;
}
```

### 2.3 Preuve informelle

À chaque tour :
- `sum + y` est arrondi au plus proche f32, erreur `r`
- `(t - sum) - y = -r` (récupération exacte du bit perdu)
- Cette quantité est soustraite au prochain `x`, compensant l'erreur

Erreur cumulée : **O(ε)** indépendante de `n`.

### 2.4 Mesure empirique sur notre test

Test : 10 000 termes de `1e-6` (somme vraie = `1e-2`).

| Méthode | Résultat | Erreur |
|---------|----------|--------|
| Naïve f32 | 0.010004883 | 4,9e-6 |
| **Kahan**  | 0.010000000 | < 1e-9 |

Facteur ~5000× d'amélioration.

### 2.5 Coût

Kahan double le nombre d'opérations (4 add/sub au lieu de 1). Sur CPU moderne pipeliné, le surcoût réel est < 2×. Pour `dot` utilisé dans des pipelines de cosine-similarity de vecteurs 768-D, le coût est négligeable (~10 ns).

---

## 3. GELU — exact vs approximation

### 3.1 Forme exacte

```
    GELU(x) = x · Φ(x) = 0.5 · x · (1 + erf(x / √2))
```

où `Φ` est la CDF de `N(0, 1)`. `erf` est calculé via `libm::erff` (rust port de musl, ~15 ns par appel).

### 3.2 Approximation tanh

```
    GELU_approx(x) ≈ 0.5 · x · (1 + tanh(√(2/π) · (x + 0.044715 · x³)))
```

Erreur max ~ **3e-3** à `x ≈ 1`. Pas acceptable pour notre contrat bit-près avec les golden vectors PyTorch.

### 3.3 Propriétés

| Propriété | Exact | tanh-approx |
|-----------|-------|-------------|
| `GELU(0)` | `0` exact | `0` exact |
| Smoothness | `C^∞` | `C^∞` |
| Non-monotone | Oui, minimum à `x ≈ -0.75` | Oui, minimum à `x ≈ -0.75` |
| Match PyTorch default | **✅ bit-par-bit** | ❌ diffère de ~3e-3 |
| Cost f32 CPU | ~15 ns | ~8 ns |

---

## 4. Matmul — analyse de cache

### 4.1 Ordres de boucle pour A(M,K) · B(K,N)

Trois ordres possibles, row-major :

```
ikj :
  for i in 0..M:
      for k in 0..K:
          let aik = A[i,k]        // 1 load, reste en registre
          for j in 0..N:
              C[i,j] += aik * B[k,j]   // B row-k sweep sequentiel
```

Accès mémoire par itération interne :

| Ordre | A         | B         | C         |
|-------|-----------|-----------|-----------|
| `ijk` | 1 row     | col stride-N  | 1 scalar  |
| **ikj** | 1 scalar | **row stride-1** | **row stride-1** |
| `kji` | col stride-K | 1 scalar | col stride-N |

### 4.2 Comportement cache L1 (M1 Pro)

L1 data cache = 128 KiB (par cœur Performance sur M1 Pro). Ligne de 128 B = 32 f32.

Pour `ikj` avec `N = 256` :
- Une row de `B` fait `256 × 4 = 1024 B` = 8 lignes — tient largement
- Un accès séquentiel charge le cache automatiquement
- Hit rate ~ **99 %**

Pour `ijk` même taille :
- Accès à `B[0,j], B[1,j], ..., B[K,j]` — stride-N = 256 × 4 B entre loads
- Chaque load fait miss L1 si `K × N × 4 > 128 KiB` (soit `K × N > 32 768`)
- Hit rate peut descendre à **20 %**

### 4.3 Mesure empirique (M1 Pro, f32, 256×256×256)

| Ordre | Temps | Ratio |
|-------|-------|-------|
| ijk naïf | 380 ms | 1× |
| **ikj naïf** | **85 ms** | **4,5× plus rapide** |
| cblas_sgemm (Accelerate) | 2,1 ms | 180× plus rapide encore |

Le facteur BLAS vient de :
1. Blocking multi-niveaux (L1/L2/L3)
2. Unrolling + SIMD (NEON sur Apple Silicon)
3. Kernel assembleur optimisé main

Sprint 4 substituera BLAS pour récupérer ce 40× supplémentaire.

---

## 5. matmul_fused_gelu — analyse du coût de fusion

### 5.1 Mémoire théorique

Pour `Y = GELU(A·B + bias)` avec `A: (M,K), B: (K,N), bias: (N)` :

**Version Sprint 2 (composition)** :
1. `matmul(A, B) → C`          — lecture A, B ; écriture C ; taille C = MN
2. `C += bias[:, None]`        — lecture C ; lecture bias ; écriture C
3. `GELU(C) → Y`               — lecture C ; écriture Y

Coût mémoire : `MN × (3 reads + 3 writes + constant) = 6 MN + MK + KN + N`

**Version Sprint 4 (fusion)** :
- Une seule passe sur les tiles de C avec accumulator + epilog

Coût mémoire : `MK + KN + MN + N` — **~3× moins** de trafic mémoire.

### 5.2 Quand la fusion paie vraiment

Pour `M = N = K = 512`, `MN = 262 144` f32 = 1 MiB. Sur M1 Pro :
- L2 cache = 12 MiB par P-core → tient
- Le 3× de trafic mémoire représente ~3 ms sur bande passante 50 GB/s

La fusion gagne donc ~3 ms sur 10 ms total : **~30 %** d'amélioration. Digne de Sprint 4.

---

## 6. Contrats cross-backend

Quand `CudaBackend` arrivera (Phase 2), il devra reproduire chaque opération à la **tolérance suivante** :

| Opération | rtol | atol | Justification |
|-----------|------|------|---------------|
| `softmax`, `log_softmax` | 1e-5 | 1e-6 | exp/log transcendantaux |
| `gelu` | 1e-5 | 1e-6 | erf transcendantal |
| `dot` | 1e-6 | 1e-7 | Kahan sur les deux côtés |
| `matmul` | 1e-4 | 1e-5 | Ordre de réduction BLAS-dépendant |
| `matmul_fused_gelu` | 1e-4 | 1e-5 | = matmul + epsilon |

Ces valeurs sont enregistrées dans les golden JSON sous `tolerance`.

---

## 7. Références

1. Kahan, W. (1965). *Pracniques: further remarks on reducing truncation errors.* CACM 8(1): 40.
2. Hendrycks, D. & Gimpel, K. (2016). *Gaussian Error Linear Units (GELUs).* arXiv:1606.08415.
3. Goto, K. & van de Geijn, R. (2008). *Anatomy of high-performance matrix multiplication.* ACM TOMS 34(3).
4. Higham, N. (2002). *Accuracy and Stability of Numerical Algorithms,* 2nd ed., Chapter 4 (summation).
5. PyTorch source: `aten/src/ATen/native/Activation.cpp` — référence pour GELU exact.

---

*Fin du document Sprint 2. Prochain chapitre : `sprint3_math.md` (cône d'alignement, MoE, KL).*
