# EFE — Service Level Objectives

> **Statut** : v1.0 — Sprint C, Bench Foundation.
> **Référence** : `BENCHMARKS.md` (mesures publiées),
> `docs/bench/bench_foundation_architecture.md` (architecture),
> `crates/efe-core/tests/bench_slo_assertions.rs` (vérification).

Ce document fixe les **engagements de performance** de la couche
`efe-core` mesurés via le bench end-to-end `swarm_e2e`. Toute
régression supérieure à 10 % bloque la PR via le workflow CI
`.github/workflows/bench.yml`.

## 1. Cinq SLO publiés

| ID | Engagement | Bench cible | Cible (locale, ref M1) | Coeff CI |
|:--:|------------|-------------|:----------------------:|:--------:|
| **SLO-1** | Latence p95 d'une requête `small` | `swarm_e2e_small_latency::execute` | **50 ms** | × 1.5 |
| **SLO-2** | Latence p95 d'une requête `medium` | `swarm_e2e_medium_latency::execute` | **200 ms** | × 1.5 |
| **SLO-3** | Latence p95 d'une requête `large` | `swarm_e2e_large_latency::execute` | **1 s** | × 1.5 |
| **SLO-4** | Débit minimum @ Nc=4 sur `small` | `swarm_e2e_small_throughput/Nc4` | **≥ 50 req/s** | × 0.7 |
| **SLO-5** | Heap peak d'une requête `medium` | `memory_e2e_medium_heap::peak_kb` | **< 50 MB** | × 1.5 |

## 2. Tailles d'essaim de référence

Les SLO sont mesurés sur les fixtures déterministes
`scripts/fixtures/realistic_{small,medium,large}.json`. Tout engagement
sur une autre taille de spec doit être négocié explicitement.

| Spec | dim | N agents | Anchors | Rounds | Latents | Cible p95 |
|------|----:|---------:|--------:|-------:|--------:|----------:|
| **small** | 128 | 8 | 4 | 8 | 4 KB | **50 ms** |
| **medium** | 256 | 32 | 8 | 8 | 32 KB | **200 ms** |
| **large** | 256 | 128 | 16 | 8 | 128 KB | **1 s** |

## 3. Coefficient CI

L'environnement GitHub Actions `ubuntu-latest` (2 vCPU partagés)
est ~1.5× plus lent qu'un Mac M1 Pro standalone, et avec une
variance plus élevée. Le workflow `bench.yml` applique automatiquement
un coefficient :

* **Latence** : `target_ns × 1.5`
* **Débit** : `target_req_per_s × 0.7` (CI ne tient que 70 % du débit local)
* **Mémoire** : `target_kb × 1.5` (l'allocator GHA peut différer)

Pour des chiffres comparables au référentiel local, lancer le bench sur
un runner self-hosted (ARM ou un GHA dedicated runner) avec `CI=false`.

## 4. Méthode de mesure (« approximate p95 »)

Criterion en `--output-format bencher` n'expose pas explicitement le
p95. Le parser `scripts/parse_criterion_report.py` lit
`target/criterion/<group>/<id>/new/estimates.json` et **approxime
le p95** par :

```
p95_approx ≈ median + 2 × MAD
```

où `MAD = median absolute deviation`. Pour des distributions
quasi-normales (cas typique de bench répétitif sans GC stalls),
cette approximation est correcte à ±5 %. Pour une mesure plus
précise, parser `sample.json` et calculer le quantile empirique.

## 5. Régression

| Δ par rapport à `v0.1.0` | Action CI |
|-------------------------|-----------|
| ≤ +5 % | ✅ silencieux |
| +5 % à +10 % | ⚠️ comment WARN sur la PR |
| > +10 % | ❌ fail CI, blocage merge |

## 6. Rafraîchissement de la baseline

La baseline `v0.1.0` est figée dans `target/criterion/baseline_v0_1_0/`
et versionnée. Elle est **rafraîchie à chaque release majeure**
(v0.2, v1.0, …) ou lors d'un changement matériel CI. Procédure :

```bash
# Sur une machine propre, après un cargo clean :
cargo bench --bench swarm_e2e --bench memory_e2e \
    -- --save-baseline v0.2.0
git add target/criterion/baseline_v0_2_0/
git commit -m "chore(bench): refresh baseline to v0.2.0"
```

## 7. Limites connues

* Les benchs utilisent des **mocks** (`BenchMock*` dans
  `crates/efe-core/benches/common/mod.rs`) qui ne font PAS de calcul
  numérique réel. Les SLO mesurent donc l'**overhead du contrôleur +
  arena + telemetry**, pas les ops compute. Pour le coût réel
  numérique, voir `BENCHMARKS.md` § « micro-benchs ».
* Les SLO sont mesurés en `--release` avec `target-cpu=native`. En
  CI on perd le `target-cpu=native` (binaire portable) → ~10 %
  plus lent.
* Aucun SLO sur la latence Mode 1 RPC end-to-end (HTTP / gRPC) n'est
  publié dans cette version — Sprint suivant.

## 8. Évolution future

| Version SLO | Cible |
|-------------|-------|
| v1.0 (cette version) | 5 SLO `efe-core::swarm_e2e` |
| v1.1 (Sprint suivant) | + SLO Mode 1 server (RPC latency) |
| v2.0 (Phase 2 CUDA) | SLO révisés pour exploiter CUDA × 10-15 |
| v3.0 (production) | + SLO multi-tenant (P99 isolation, fairness) |
