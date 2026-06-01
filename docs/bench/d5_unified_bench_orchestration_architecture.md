# Sprint D5 — Architecture : Orchestration bench unifiée

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Cible** : refonte complète de
> [`scripts/bench_runner.sh`](../../scripts/bench_runner.sh) et
> [`.github/workflows/bench.yml`](../../.github/workflows/bench.yml)
> pour qu'ils gèrent **TOUS les 10 benches** (Sprint C, S2-4, D1-4)
> de manière déclarative, parallèle, et fail-safe.
> **Position dans la séquence** : suit D1 (backend), D2 (mémoire),
> D3 (compute), D4 (gRPC). D5 est l'**infrastructure d'exécution**,
> pas un nouveau bench. Pas de SLO ajouté ; D5 garantit que les
> 30 SLO existants tournent bien.

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [Inventaire des 10 benches existants](#2-inventaire-des-10-benches-existants)
3. [Gap analysis : 14 défauts du système actuel](#3-gap-analysis--14-défauts-du-système-actuel)
4. [Architecture cible : 3 couches](#4-architecture-cible--3-couches)
5. [Catalog des benches — source unique de vérité](#5-catalog-des-benches--source-unique-de-vérité)
6. [CLI grammar de `bench_runner.sh`](#6-cli-grammar-de-bench_runnersh)
7. [Workflow CI : DAG matrix + aggregation](#7-workflow-ci--dag-matrix--aggregation)
8. [Per-bench timeout & retry strategy](#8-per-bench-timeout--retry-strategy)
9. [Report aggregator + PR comment](#9-report-aggregator--pr-comment)
10. [Path filter dérivé du catalog](#10-path-filter-dérivé-du-catalog)
11. [12 décisions de conception clés](#11-12-décisions-de-conception-clés)
12. [10 risques identifiés et mitigations](#12-10-risques-identifiés-et-mitigations)
13. [Plan jour-par-jour (3 j wallclock)](#13-plan-jour-par-jour-3-j-wallclock)
14. [Critères d'acceptation (DoD)](#14-critères-dacceptation-dod)
15. [Out of scope & roadmap follow-ups](#15-out-of-scope--roadmap-follow-ups)

---

## 1. Contexte & mission

### 1.1 Pourquoi D5 maintenant

Les sprints D1-D4 ont produit 8 benches nouveaux + 2 tests
anti-régression suites. La couverture latentielle est désormais
~100 % (orchestration → mémoire → compute → RPC). **Mais** :

* `bench_runner.sh` a grossi par couches sédimentaires (Sprint C
  d'abord, puis D1 ajouté en bloc, puis D2, D3, D4) — chaque ajout
  était une copie-paste de 6 lignes du bloc précédent. Le résultat
  n'est **pas un système**, c'est une accumulation. Toute
  modification (nouveau bench, nouveau filtre, nouvelle option BLAS)
  implique d'éditer 4 endroits dans le script *et* dans `bench.yml`.
* Le filtre `ONLY=mock|real|both` est trop **gros grain** — on ne
  peut pas dire "ne run que D2", "exclus le rate-limit-bench",
  "run uniquement les compute_sprint*".
* Les **3 micro-benches Sprint S2/S3/S4** existent dans le repo
  mais ne tournent **JAMAIS** en CI. Ils sont littéralement morts
  pour le système d'observabilité performance.
* `bench.yml` hardcode la baseline à `v0.1.0` alors qu'on en a 5
  (`v0.1.0..v0.1.4`). La régression detection est cassée pour
  toutes les keys post-v0.1.0.
* Pas de **matrix CI** : tout en série dans un seul job, ~30 min
  wallclock — 2× ce qu'il faudrait avec parallel jobs.
* Pas de **PR comment automation** — l'opérateur doit cliquer dans
  l'UI Actions pour voir les chiffres.

### 1.2 Mission D5 (énoncé chirurgical)

> Produire :
>
> 1. **`scripts/benches.list`** — un catalog déclaratif unique
>    qui liste les 10 benches avec leurs métadonnées (crate,
>    sprint, BLAS-aware, tier, output_file, paths watch).
> 2. **`scripts/bench_runner.sh` v2** — refonte complète qui
>    consomme `benches.list` et expose une CLI grammaticale :
>    `BENCHES=`, `SPRINT=`, `TIER=`, `EXCLUDE=`, `MODE=`,
>    `BENCH_TIMEOUT=`. Backward-compatible avec les invocations
>    actuelles (les variables d'env existantes restent valides).
>    Chaque bench tourne dans un sous-shell isolé avec
>    `timeout $BENCH_TIMEOUT` et un retry de 1 sur échec.
> 3. **`.github/workflows/bench.yml` v2** — réécriture en **DAG
>    de 5 jobs parallèles** :
>    - `bench-build` (compile une fois, partage l'artefact via cache)
>    - `bench-core-mocks` (swarm_e2e, memory_e2e)
>    - `bench-core-infra` (memory_real_arena)
>    - `bench-core-compute` (compute_pipeline, compute_sprint{2,3,4})
>    - `bench-server` (real_strategies_e2e, real_memory, server_rpc)
>    - `bench-aggregate` (depends on all, downloads artefacts,
>      composes report, posts PR comment, gates regression).
>    Wallclock cible : 12 min (vs 30 min en série).
> 4. **`scripts/aggregate_bench_reports.py`** — fusionne N reports
>    partiels en un rapport canonique unique (même schéma que
>    `parse_criterion_report.py` produit aujourd'hui).
> 5. **`scripts/post_bench_pr_comment.py`** — formatte le rapport
>    en Markdown et l'envoie via `gh pr comment` (commenté en CI).

### 1.3 Ce que D5 N'EST PAS

* Pas de nouveau bench (D5 n'ajoute aucune mesure).
* Pas de nouveau SLO (les 30 existants restent intacts).
* Pas de refactor des benches eux-mêmes (D2-D4 stables).
* Pas de nouveau parser de criterion (le parser
  `parse_criterion_report.py` reste, juste son `WANTED_PREFIXES`
  devient dérivé du catalog).

---

## 2. Inventaire des 10 benches existants

### 2.1 Tableau exhaustif post-D4

| # | Bench | Sprint | Crate | BLAS aware ? | Run en CI aujourd'hui ? | Sub-benches | SLO IDs |
|--:|-------|:------:|-------|:------------:|:-----------------------:|:-----------:|---------|
| 1 | `swarm_e2e` | C | core | non | ✅ | ~12 | SLO-1, SLO-2, SLO-3 |
| 2 | `memory_e2e` | C | core | non | ✅ | ~6 | n/a (heap, pas SLO) |
| 3 | `compute_sprint2` | S2 | core | non | ❌ | ~10 | n/a (micro, pas SLO) |
| 4 | `compute_sprint3` | S3 | core | non | ❌ | ~8 | n/a (micro) |
| 5 | `compute_sprint4` | S4 | core | **oui** | ❌ | ~6 | n/a (micro BLAS comparison) |
| 6 | `memory_real_arena` | D2 | core | non | ✅ | 11 | SLO-D2-1..10 |
| 7 | `compute_pipeline` | D3 | core | **oui** | ✅ | 24 | SLO-D3-1..3 + B variants |
| 8 | `real_strategies_e2e` | D1 | server | **oui** | ✅ | 11 | SLO-RD1-1..3 + T1 |
| 9 | `real_memory` | D1 | server | non | ✅ | 6 | n/a (heap, pas SLO) |
| 10 | `server_rpc` | D4 | server | partiel | ✅ | 10 | SLO-D4-1..3 + T1 |
| **Σ** | — | — | **2 crates** | — | **7/10** | **~104 sub-benches** | **~30 SLO** |

### 2.2 Constat critique

**3 benches sur 10 ne tournent JAMAIS** : `compute_sprint{2,3,4}`.
Ces benches existent depuis le code Sprint S2-S4 (~6 mois de travail)
et fournissent les chiffres micro-bench documentés en
`BENCHMARKS.md` § 6 ("Source : `cargo bench --bench
compute_sprint{2,3,4}`"). Mais ces chiffres n'ont **jamais été
mis à jour automatiquement** — quelqu'un a dû les coller à la main
au moment de la release Sprint S4. Toute régression sur ces ops
(matmul naïve, softmax, gelu, cone_project, weighted_mean,
multinomial_sample, …) passe sous le radar.

### 2.3 Conséquence : couverture *exécutée* vs couverture *écrite*

| Couche | Benches écrits | Benches exécutés en CI | Δ |
|--------|---------------:|----------------------:|--:|
| Compute micro (S2/S3/S4) | 3 | 0 | **−3** |
| Compute pipeline (D3) | 1 | 1 | 0 |
| Memory infra (Sprint C + D2) | 2 | 2 | 0 |
| Server backend (D1) | 2 | 2 | 0 |
| Server RPC (D4) | 1 | 1 | 0 |
| Server mock (Sprint C) | 1 | 1 | 0 |
| **TOTAL** | **10** | **7** | **−3 (30 % gap)** |

D5 ferme ce gap.

---

## 3. Gap analysis : 14 défauts du système actuel

### 3.1 `bench_runner.sh`

| # | Défaut | Impact | Sévérité |
|--:|--------|--------|:--------:|
| G1 | Liste des benches hardcodée en blocs `if`/`echo`/`cargo bench` | Ajouter un bench = 6 LOC × 2 sites | 🔴 |
| G2 | Filtre `ONLY=mock\|real\|both` trop grossier | Pas de "run only D2", pas de "exclude memory_e2e" | 🔴 |
| G3 | `compute_sprint{2,3,4}` absents | 30 % des benches morts en CI | 🟠 |
| G4 | Pas de timeout par bench | Un bench qui hang = job CI rouge à 30 min | 🟠 |
| G5 | Pas de retry sur échec transitoire | Bench flaky = PR rouge sans cause réelle | 🟠 |
| G6 | Hardcodage des noms de fichiers `target/bench-X.txt` | Drift possible vs `bench.yml` artefact list | 🟡 |
| G7 | Pas de mode `smoke` (build only, pas de mesure) | Pas de validation rapide des benches en local | 🟡 |

### 3.2 `bench.yml`

| # | Défaut | Impact | Sévérité |
|--:|--------|--------|:--------:|
| G8 | Single sequential job (~30 min wallclock) | 2× plus lent que parallel possible | 🟠 |
| G9 | Baseline hardcodée à `v0.1.0` | Régression detection cassée pour D1-D4 keys | 🔴 |
| G10 | Liste des artefacts hardcodée | Drift vs `bench_runner.sh` (déjà observé en D2/D3/D4) | 🔴 |
| G11 | Path filter hardcodé | Chaque nouveau bench non listé = silently skipped | 🟠 |
| G12 | Pas de PR comment automation | L'opérateur doit cliquer dans Actions | 🟡 |
| G13 | Pas de matrix per BLAS feature | `_blas` SLOs jamais mesurés en CI (besoin manuel) | 🟠 |
| G14 | `cancel-in-progress` cancel un push to main même si critical | Risque de manquer une mesure post-merge | 🟡 |

### 3.3 Couplage / drift entre les deux

Le pire défaut **structurel** est la **duplication implicite** :
les benches sont listés dans :

* `bench_runner.sh` (1 bloc `cargo bench` par bench),
* `bench.yml` (1 path artefact par bench),
* `parse_criterion_report.py` (`WANTED_PREFIXES` + SLO keys),
* `Cargo.toml` (`[[bench]] name = "..."`).

Quatre sources de vérité divergentes. Toute modification doit
toucher 4 fichiers, et un oubli sur l'un produit un *silent fail* :

* Oublier dans `bench_runner.sh` ⇒ bench non exécuté.
* Oublier dans `bench.yml` artefacts ⇒ pas d'upload, perte du
  rapport.
* Oublier dans `parse_criterion_report.py` ⇒ measures parsées
  mais pas remontées.
* Oublier dans `Cargo.toml` ⇒ erreur compile claire (on s'en
  aperçoit).

D5 unifie en **1 source** : le catalog `scripts/benches.list`.

---

## 4. Architecture cible : 3 couches

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 1 — Catalog déclaratif                                │
│  scripts/benches.list                                        │
│  ↓ source unique de vérité (10 entrées, schéma fixe)          │
└─────────────────────────────────────────────────────────────┘
                          │
        ┌─────────────────┼──────────────────────────────┐
        ▼                 ▼                              ▼
┌──────────────┐  ┌──────────────────┐  ┌────────────────────┐
│  Layer 2.a   │  │  Layer 2.b       │  │  Layer 2.c          │
│  bench_runner│  │  bench.yml       │  │  parse_criterion_   │
│  .sh         │  │  (CI workflow)   │  │  report.py          │
│  (CLI driver)│  │                  │  │  (extraction +      │
│              │  │                  │  │   SLO check)        │
└──────────────┘  └──────────────────┘  └────────────────────┘
        │                 │                              │
        ▼                 ▼                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Layer 3 — Outputs                                           │
│  target/bench-{name}.txt   (per-bench bencher format)        │
│  target/bench-report-*.json (consolidated)                   │
│  PR comment (Markdown)                                       │
└─────────────────────────────────────────────────────────────┘
```

### 4.1 Pourquoi 3 couches

* **Layer 1 (catalog)** est *déclaratif et lisible*. Ajouter un
  bench = 1 ligne. Pas de logique, juste de la donnée.
* **Layer 2 (drivers)** ne contient *que* la logique d'exécution
  (loop, filter, run). Pas d'inventaire de benches.
* **Layer 3 (outputs)** est *standardisé* : tout fichier suit un
  pattern `target/bench-{name}.txt`, donc l'agrégateur peut
  globber sans liste explicite.

Le catalog est consommé par les 3 drivers — toute discipline est
maintenue en un seul endroit.

---

## 5. Catalog des benches — source unique de vérité

### 5.1 Format : fichier sourcé en bash

**Décision** : pas de YAML/TOML/JSON. Le runner est en bash,
et un fichier bash sourcé est trivial à parser. Chaque entrée est
une string colon-séparée :

```
name:crate:sprint:blas-aware:tier:slo-prefix:paths-watch
```

### 5.2 Le fichier complet (`scripts/benches.list`)

```bash
#!/usr/bin/env bash
# scripts/benches.list — Sprint D5 source of truth.
#
# Schema per row (colon-separated, no spaces around colons):
#   name:crate:sprint:blas-aware:tier:slo-prefix:paths-watch
#
# - name           : matches the [[bench]] name in Cargo.toml.
# - crate          : "efe-core" or "efe-server".
# - sprint         : C / S2 / S3 / S4 / D1 / D2 / D3 / D4 (free text).
# - blas-aware     : "blas" if --features accelerate/openblas changes
#                    behaviour, else "no-blas".
# - tier           : "default" (run every PR) or "micro" (run only
#                    when explicitly requested via TIER=all or
#                    micro paths touched).
# - slo-prefix     : prefix used by parse_criterion_report.py to
#                    walk criterion JSON. Empty string when bench
#                    has no SLO (heap-only, micro).
# - paths-watch    : space-separated globs that should trigger this
#                    bench in CI path filtering. The CI workflow
#                    derives its `paths` filter from the union of
#                    all rows.
#
# IMPORTANT: No trailing whitespace. Lines starting with '#' are
# comments. Keep alphabetically sorted by name within each crate.

declare -a EFE_BENCHES=(
  # --- efe-core ---
  "compute_pipeline:efe-core:D3:blas:default:compute_pipeline_:crates/efe-core/src/compute/** crates/efe-core/src/fusion/kernels/**"
  "compute_sprint2:efe-core:S2:no-blas:micro::crates/efe-core/src/compute/cpu/**"
  "compute_sprint3:efe-core:S3:no-blas:micro::crates/efe-core/src/compute/cpu/**"
  "compute_sprint4:efe-core:S4:blas:micro::crates/efe-core/src/compute/cpu/linalg**"
  "memory_e2e:efe-core:C:no-blas:default:memory_e2e_:crates/efe-core/src/memory/**"
  "memory_real_arena:efe-core:D2:no-blas:default:pool_acquire_ arena_alloc_ arena_reset realistic_swarm_alloc_pattern:crates/efe-core/src/memory/**"
  "swarm_e2e:efe-core:C:no-blas:default:swarm_e2e_:crates/efe-core/src/swarm/**"

  # --- efe-server ---
  "real_memory:efe-server:D1:no-blas:default:real_memory_:crates/efe-server/src/mode1.rs"
  "real_strategies_e2e:efe-server:D1:blas:default:real_e2e_ real_phase_:crates/efe-server/src/mode1.rs crates/efe-server/src/services/**"
  "server_rpc:efe-server:D4:blas:default:server_rpc_:crates/efe-server/src/services/** crates/efe-server/src/bridge/** crates/efe-server/src/interceptors/**"
)

# Helper : iterate with a callback `process_bench name crate sprint blas tier slo paths`.
run_bench_catalog() {
  local cb="$1"
  local row name crate sprint blas tier slo paths
  for row in "${EFE_BENCHES[@]}"; do
    IFS=':' read -r name crate sprint blas tier slo paths <<< "$row"
    "$cb" "$name" "$crate" "$sprint" "$blas" "$tier" "$slo" "$paths"
  done
}
```

### 5.3 Lecture du catalog

* **Sourçable depuis `bench_runner.sh`** : `source scripts/benches.list`
* **Sourçable depuis Python** (pour bench.yml path filter generator) :
  `python3 scripts/extract_bench_catalog.py --field paths-watch`
* **Sourçable depuis `parse_criterion_report.py`** : `WANTED_PREFIXES`
  est dérivé de la concat des `slo-prefix` non vides.

### 5.4 Gain immédiat

Ajouter un bench = 1 ligne dans `benches.list`. Aucune autre
modification dans les 3 drivers — ils itèrent automatiquement.

---

## 6. CLI grammar de `bench_runner.sh`

### 6.1 Variables d'environnement reconnues

| Variable | Valeurs | Effet | Backward-compat ? |
|----------|---------|-------|:-----------------:|
| `BENCHES` | csv list (e.g., `swarm_e2e,server_rpc`) | Exécute uniquement ces benches | nouveau |
| `SPRINT` | `C`, `S2`, …, `D4`, `all` | Filtre par sprint | nouveau |
| `TIER` | `default`, `micro`, `all` | Filtre par tier (default = `default`) | nouveau |
| `EXCLUDE` | csv list | Exclut ces benches | nouveau |
| `MODE` | `quick`, `smoke`, `full` | quick=`--quick`, smoke=build-only, full=full criterion | nouveau |
| `BLAS_BACKEND` | `accelerate`, `openblas`, `none` | Active la feature | **conservé** |
| `QUICK` | `1` ou vide | Alias de `MODE=quick` (deprecated mais supporté) | **conservé** |
| `BASELINE` | path | Forward à criterion `--baseline` | **conservé** |
| `ONLY` | `mock`, `real`, `both` | Alias historique : mock = sprint C+D2+D3, real = D1+D4 (deprecated) | **conservé** |
| `BENCH_TIMEOUT` | seconds (default 600) | timeout par bench via `timeout` | nouveau |
| `BENCH_RETRY` | `0` ou `1` (default 1) | Retry une fois sur échec | nouveau |
| `CARGO` | path to cargo | Override du binaire | **conservé** |

### 6.2 Précédence des filtres

```
BENCHES (explicit list) > EXCLUDE > SPRINT > TIER > default
```

* Si `BENCHES=swarm_e2e,server_rpc` est set, on exécute *exactement*
  ces deux benches, en ignorant SPRINT/TIER.
* Sinon : on commence avec tous les benches du TIER (default
  `default`), filtrés par SPRINT (default `all`), puis on retire
  ceux dans EXCLUDE.

### 6.3 Exemples d'invocation

```bash
# CI default : tous les "default" tier
./bench_runner.sh

# Run uniquement D2
SPRINT=D2 ./bench_runner.sh

# Run *all* tiers (incl. micro), no BLAS
TIER=all ./bench_runner.sh

# Run un bench spécifique en mode smoke (just build, no measure)
BENCHES=server_rpc MODE=smoke ./bench_runner.sh

# Run TOUS les benches BLAS-aware avec Apple Accelerate
TIER=all BLAS_BACKEND=accelerate ./bench_runner.sh

# Backward-compat : run "real" benches, --quick
ONLY=real QUICK=1 ./bench_runner.sh

# CI matrix node "core-compute"
SPRINT=D3 BENCHES=compute_pipeline,compute_sprint2,compute_sprint3,compute_sprint4 \
   ./bench_runner.sh
```

### 6.4 Squelette du driver v2

```bash
#!/usr/bin/env bash
# scripts/bench_runner.sh — D5 unified bench runner.

set -euo pipefail

source "$(dirname "$0")/benches.list"

# Defaults
CARGO="${CARGO:-cargo}"
BLAS_BACKEND="${BLAS_BACKEND:-none}"
MODE="${MODE:-${QUICK:+quick}}"  # legacy QUICK=1 → MODE=quick
MODE="${MODE:-quick}"             # default mode
TIER="${TIER:-default}"
SPRINT="${SPRINT:-all}"
BENCHES="${BENCHES:-}"
EXCLUDE="${EXCLUDE:-}"
BENCH_TIMEOUT="${BENCH_TIMEOUT:-600}"
BENCH_RETRY="${BENCH_RETRY:-1}"
BASELINE="${BASELINE:-}"

# Backward compat ONLY → SPRINT
if [ -n "${ONLY:-}" ] && [ -z "$BENCHES" ]; then
  case "$ONLY" in
    mock) SPRINT="C,D2,D3" ;;
    real) SPRINT="D1,D4" ;;
    both) SPRINT="all" ;;
  esac
fi

# Compose list of benches to run
benches_to_run=()
should_run() {
  local name="$1" crate="$2" sprint="$3" blas="$4" tier="$5"

  # Explicit BENCHES list overrides everything else
  if [ -n "$BENCHES" ]; then
    [[ ",$BENCHES," == *",$name,"* ]] && return 0 || return 1
  fi

  # EXCLUDE kicks out
  if [ -n "$EXCLUDE" ]; then
    [[ ",$EXCLUDE," == *",$name,"* ]] && return 1
  fi

  # TIER filter
  case "$TIER" in
    default) [ "$tier" = "default" ] || return 1 ;;
    micro)   [ "$tier" = "micro" ]   || return 1 ;;
    all)     ;; # accept any
    *)       return 1 ;;
  esac

  # SPRINT filter
  case "$SPRINT" in
    all) ;;
    *)   [[ ",$SPRINT," == *",$sprint,"* ]] || return 1 ;;
  esac

  return 0
}
collect_bench() {
  local name="$1" crate="$2" sprint="$3" blas="$4" tier="$5" slo="$6" paths="$7"
  if should_run "$name" "$crate" "$sprint" "$blas" "$tier"; then
    benches_to_run+=("$name|$crate|$blas|$slo")
  fi
}
run_bench_catalog collect_bench

if [ "${#benches_to_run[@]}" -eq 0 ]; then
  echo "ERROR: no benches matched filters (BENCHES=$BENCHES SPRINT=$SPRINT TIER=$TIER EXCLUDE=$EXCLUDE)"
  exit 2
fi

# Pre-flight build (deduplicated by crate)
crates=$(printf "%s\n" "${benches_to_run[@]}" | cut -d'|' -f2 | sort -u)
for crate in $crates; do
  features_flag=""
  if [ "$BLAS_BACKEND" = "accelerate" ] || [ "$BLAS_BACKEND" = "openblas" ]; then
    features_flag="--features $BLAS_BACKEND"
  fi
  echo "==> Pre-flight: building $crate benches ($BLAS_BACKEND)"
  $CARGO build -p "$crate" --benches --release $features_flag \
      > "target/bench-build-$crate.log" 2>&1 || {
    echo "ERROR: $crate benches failed to build, see target/bench-build-$crate.log"
    exit 1
  }
done

[ "$MODE" = "smoke" ] && {
  echo "MODE=smoke: build only, exiting cleanly"
  exit 0
}

# Criterion flags from MODE
CRITERION_FLAGS=""
[ "$MODE" = "quick" ] && CRITERION_FLAGS="$CRITERION_FLAGS --quick"
[ -n "$BASELINE" ] && CRITERION_FLAGS="$CRITERION_FLAGS --baseline $BASELINE"

# Per-bench loop with timeout + retry
failed_benches=()
for entry in "${benches_to_run[@]}"; do
  IFS='|' read -r name crate blas slo <<< "$entry"
  features_flag=""
  if [ "$blas" = "blas" ] && [ "$BLAS_BACKEND" != "none" ]; then
    features_flag="--features $BLAS_BACKEND"
  fi
  output_file="target/bench-${name//_/-}.txt"

  attempt=0
  max_attempts=$((1 + BENCH_RETRY))
  success=0
  while [ "$attempt" -lt "$max_attempts" ]; do
    attempt=$((attempt + 1))
    echo "==> Running $name ($crate, sprint=$sprint, attempt=$attempt/$max_attempts)"
    if timeout "$BENCH_TIMEOUT" $CARGO bench -p "$crate" --bench "$name" \
        $features_flag -- --output-format bencher $CRITERION_FLAGS \
        > "$output_file" 2>&1; then
      success=1
      break
    fi
    echo "WARN: $name attempt $attempt failed (exit code $?), see $output_file"
  done
  if [ "$success" -ne 1 ]; then
    failed_benches+=("$name")
  fi
done

# Compose JSON report (existing parser handles per-bench TXT output via criterion JSON)
TIMESTAMP=$(date -u +%Y%m%dT%H%M%SZ)
SHA=$(git rev-parse --short=8 HEAD 2>/dev/null || echo "nogit")
OUT="target/bench-report-${TIMESTAMP}-${SHA}.json"

# … host metadata capture (kept verbatim from v1) …

python3 scripts/parse_criterion_report.py \
    --metadata-os "$HOST_OS $HOST_VER" \
    # … etc as v1 …
    --output "$OUT"

echo ""
echo "Bench report: $OUT"
echo "Failed benches: ${failed_benches[*]:-none}"
[ "${#failed_benches[@]}" -gt 0 ] && exit 1 || exit 0
```

### 6.5 Pourquoi `timeout` + retry

* **Timeout** : un bench qui hang (deadlock dans tonic, leak
  d'arène) ne fait plus tomber les 30 min de CI. Default 600 s
  (10 min) par bench, configurable.
* **Retry** : criterion peut occasionnellement échouer sur une
  iter (page fault, TCP reset transitoire). Un retry 1× catche
  ces transitoires sans masquer les vrais bugs (qui échouent les 2
  fois). Configurable via `BENCH_RETRY=0` pour désactiver.

---

## 7. Workflow CI : DAG matrix + aggregation

### 7.1 Topology cible

```
                    ┌──────────────────┐
                    │   bench-build    │  (compile une fois,
                    │  (efe-core +     │   cache l'artefact via
                    │   efe-server)    │   actions/cache)
                    └────────┬─────────┘
                             │ artefacts: target/release/...
            ┌────────────────┼────────────────────┬─────────────────┐
            ▼                ▼                    ▼                 ▼
   ┌────────────────┐ ┌───────────────┐ ┌──────────────────┐ ┌──────────────┐
   │ bench-core-    │ │ bench-core-   │ │ bench-core-      │ │ bench-server │
   │ mocks          │ │ infra         │ │ compute          │ │              │
   │ swarm_e2e,     │ │ memory_real_  │ │ compute_pipeline,│ │ real_*,      │
   │ memory_e2e     │ │ arena         │ │ compute_sprint{2,3,4}│server_rpc │
   │ ~5 min         │ │ ~3 min        │ │ ~10 min          │ │ ~12 min     │
   └────────┬───────┘ └──────┬────────┘ └────────┬─────────┘ └──────┬──────┘
            │                │                   │                   │
            └────────────────┴─────────┬─────────┴───────────────────┘
                                       ▼
                           ┌──────────────────┐
                           │ bench-aggregate  │  (downloads all,
                           │                  │   composes report,
                           │                  │   posts PR comment,
                           │                  │   gates regression)
                           └──────────────────┘
```

**Wallclock attendu** : `max(5, 3, 10, 12) + aggregate(2) ≈ 14 min`,
contre 30 min actuel ⇒ **gain ×2.1**.

### 7.2 Exemple `bench.yml` v2 (sketch)

```yaml
name: Bench

on:
  pull_request:
    branches: [main]
    paths:
      # Auto-generated from benches.list paths-watch column.
      # Run scripts/regen_bench_yml_paths.sh to regenerate.
      - 'crates/efe-core/src/**'
      - 'crates/efe-core/benches/**'
      - 'crates/efe-server/src/**'
      - 'crates/efe-server/benches/**'
      - 'crates/efe-*/Cargo.toml'
      - 'scripts/bench*.sh'
      - 'scripts/benches.list'
      - 'scripts/parse_criterion_report.py'
      - 'scripts/aggregate_bench_reports.py'
      - 'scripts/post_bench_pr_comment.py'
      - 'scripts/fixtures/**'
      - '.github/workflows/bench.yml'
  push:
    branches: [main]
  workflow_dispatch:
    inputs:
      tier:
        description: 'Tier (default | micro | all)'
        required: false
        default: 'default'
      blas_backend:
        description: 'BLAS backend (none | accelerate | openblas)'
        required: false
        default: 'none'

concurrency:
  group: bench-${{ github.ref }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}
  # Don't cancel main pushes — every commit on main needs a baseline.

env:
  CI_COEFFICIENT: "1.5"
  CARGO_TERM_COLOR: always

jobs:
  bench-build:
    name: Pre-build benches (cache for matrix jobs)
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.85.0
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: "bench-build"
          cache-on-failure: true
      - run: |
          cargo build -p efe-core --benches --release
          cargo build -p efe-server --benches --release

  bench-core-mocks:
    needs: bench-build
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.85.0
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: "bench-build"
      - name: Run swarm_e2e + memory_e2e
        run: |
          MODE=quick BENCHES=swarm_e2e,memory_e2e \
            BENCH_TIMEOUT=300 \
            scripts/bench_runner.sh
      - uses: actions/upload-artifact@v4
        with:
          name: bench-partial-core-mocks-${{ github.sha }}
          path: |
            target/bench-swarm-e2e.txt
            target/bench-memory-e2e.txt
            target/criterion/**

  bench-core-infra:
    needs: bench-build
    # … same skeleton, BENCHES=memory_real_arena …

  bench-core-compute:
    needs: bench-build
    # … BENCHES=compute_pipeline,compute_sprint2,compute_sprint3,compute_sprint4
    # TIER=all so micro tier (sprint{2,3,4}) is included.

  bench-server:
    needs: bench-build
    # … BENCHES=real_strategies_e2e,real_memory,server_rpc …

  bench-aggregate:
    needs: [bench-core-mocks, bench-core-infra, bench-core-compute, bench-server]
    runs-on: ubuntu-latest
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@v4
      - name: Download all partial artefacts
        uses: actions/download-artifact@v4
        with:
          pattern: bench-partial-*-${{ github.sha }}
          merge-multiple: true
          path: target/
      - name: Aggregate
        run: |
          python3 scripts/aggregate_bench_reports.py \
            --inputs target/criterion \
            --output target/bench-report-aggregated.json
      - name: Show SLO compliance
        run: |
          python3 scripts/show_bench_report.py \
            target/bench-report-aggregated.json
      - name: Compare to latest baseline (regression detection)
        run: |
          # Pick the highest-version baseline that exists.
          LATEST=$(ls -d bench_baselines/v* | sort -V | tail -1)
          BASELINE="$LATEST/baseline.json"
          python3 scripts/check_regression.py \
            --baseline "$BASELINE" \
            --candidate target/bench-report-aggregated.json \
            --threshold 0.10 \
            --warn-threshold 0.05 \
            --ci-coefficient "$CI_COEFFICIENT"
      - name: Post PR comment
        if: github.event_name == 'pull_request'
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          python3 scripts/post_bench_pr_comment.py \
            --report target/bench-report-aggregated.json \
            --pr "${{ github.event.pull_request.number }}"
      - name: Upload aggregated report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: bench-report-${{ github.sha }}
          path: target/bench-report-aggregated.json
          retention-days: 30
```

### 7.3 Pourquoi un `bench-build` job séparé

* Cargo build une fois, cache via `Swatinem/rust-cache@v2` avec
  `shared-key: bench-build`.
* Les 4 jobs matrix réutilisent le cache → pas de rebuild
  redondant (chaque rebuild = 5-8 min sur runner ubuntu).
* Si la build foire, les 4 matrix jobs ne démarrent pas → fail
  fast, économie de runner-minutes.

### 7.4 Pourquoi pas de matrix par BLAS backend

**Décision D8 (cf. §11)** : la matrix par BLAS doublerait le
wallclock. Pour D5, on reste sur **no-BLAS par défaut** ; un
manual `workflow_dispatch` permet d'activer `blas_backend: accelerate`
quand on veut les chiffres BLAS. Une matrix BLAS automatique est
**D5.5 follow-up**.

---

## 8. Per-bench timeout & retry strategy

### 8.1 Timeout : nécessaire mais à doser

| Bench | p95 obs (M3 Pro) | CI estimé (×1.5) | Marge ×3 | `BENCH_TIMEOUT` |
|-------|-----------------:|-----------------:|---------:|----------------:|
| swarm_e2e | ~50 ms | ~75 ms | 30 s | 300 s |
| memory_e2e | ~30 ms | ~45 ms | 30 s | 300 s |
| compute_sprint2 | ~5 ms / op | ~7 ms | — | 300 s |
| compute_sprint3 | ~10 ms / op | ~15 ms | — | 300 s |
| compute_sprint4 | ~50 µs / op | — | — | 300 s |
| memory_real_arena | ~100 µs | ~150 µs | — | 300 s |
| compute_pipeline | ~3 ms / chain | ~5 ms | — | 300 s |
| real_strategies_e2e | ~50 ms / req | ~75 ms | — | **600 s** |
| real_memory | ~50 ms / req | — | — | **600 s** |
| server_rpc | ~370 ms / RPC | ~555 ms | — | **900 s** |

Default = 600 s. `server_rpc` est ajusté à 900 s parce que la
suite full + minimal + concurrent sur 3 fixtures peut atteindre
~10 min sur un runner CI lent.

### 8.2 Retry strategy

| Bench class | Retry par défaut | Justification |
|-------------|:----------------:|---------------|
| Compute deterministic (sprint2/3/4, compute_pipeline) | 0 | Pas de source de variance non-déterministe |
| Memory bench (memory_e2e, memory_real_arena, real_memory) | 1 | OS allocator jitter, page faults |
| RPC bench (server_rpc, real_strategies_e2e) | 1 | Loopback TCP variance, port collisions transitoires |

**Note** : le retry est *au niveau bench*, pas au niveau iter
(criterion gère sa propre stat). Si un retry passe, le résultat
final est celui du retry réussi — il n'y a pas de mécanisme
"average of N attempts" qui masquerait des régressions.

---

## 9. Report aggregator + PR comment

### 9.1 `scripts/aggregate_bench_reports.py`

Le bench actuel a un seul `parse_criterion_report.py` qui lit
`target/criterion/*` directement. Avec 4 jobs CI parallèles, chaque
job a son propre `target/criterion/`. L'aggregateur :

1. Reçoit N dossiers `target/criterion-{job}/` (downloaded via
   `actions/download-artifact`).
2. Merge les sub-dirs (chaque bench a son propre groupe, pas de
   collision).
3. Appelle `parse_criterion_report.py` sur le dossier mergé.
4. Output : un seul `bench-report-aggregated.json`.

Code esquissé :

```python
#!/usr/bin/env python3
"""scripts/aggregate_bench_reports.py — merge per-job criterion outputs."""
import argparse
import os
import shutil
import subprocess
import sys

def main():
    p = argparse.ArgumentParser()
    p.add_argument('--inputs', required=True,
                   help='Root containing per-job criterion dirs')
    p.add_argument('--output', required=True)
    args = p.parse_args()

    # Aggregate by symlinking / copying every <input>/<group> into
    # target/criterion-merged/<group>. Group names are unique
    # (bench groups are namespaced by bench file), so no collisions.
    merged = 'target/criterion-merged'
    os.makedirs(merged, exist_ok=True)
    for entry in os.listdir(args.inputs):
        src = os.path.join(args.inputs, entry)
        if not os.path.isdir(src):
            continue
        # Each entry may be either a criterion root OR a single
        # group; we walk one level deep to find groups.
        for sub in os.listdir(src):
            sub_path = os.path.join(src, sub)
            if not os.path.isdir(sub_path):
                continue
            dst = os.path.join(merged, sub)
            if os.path.exists(dst):
                # Last-writer-wins ; the loop order is deterministic.
                shutil.rmtree(dst)
            shutil.copytree(sub_path, dst)

    # Delegate to the existing parser.
    rc = subprocess.call([
        'python3', 'scripts/parse_criterion_report.py',
        '--criterion-root', merged,
        '--output', args.output,
        # … pass-through metadata flags …
    ])
    sys.exit(rc)

if __name__ == '__main__':
    main()
```

### 9.2 PR comment format

```markdown
## 📊 EFE Bench Report — `${SHA8}`

**Host** : ubuntu-latest (×1.5 CI coefficient applied)
**Mode** : `--quick`
**BLAS** : none

### SLO compliance summary

| Status | Count |
|--------|------:|
| ✅ Pass | 28 |
| ⚠️ Warn (drift > 5%) | 1 |
| ❌ Fail | 0 |
| ⏭ Skipped (BLAS off) | 6 |

### Headline metrics

| Bench | Measured | SLO | Status |
|-------|---------:|----:|:------:|
| `compute_pipeline_chain_naive::medium` | 161 µs | 500 µs | ✅ ×3.1 |
| `real_e2e_latency_naive::medium` | 52 ms | 50 ms | ⚠️ +4 % drift |
| `pool_acquire_warm::default` | 80 ns | 1 µs | ✅ ×12 |
| `server_rpc_full_stack::medium` | 369 ms | 800 ms | ✅ ×2.2 |

[Full report (artefact)](https://github.com/.../actions/runs/...)

### Regression vs baseline `v0.1.4`

🟢 No regression > 10 % detected.

⚠️ Drift > 5 % on 1 key :

* `real_e2e_latency_naive::medium` : ref=50ms cand=52ms (**+4%**) — within tolerance, monitor.
```

### 9.3 `scripts/post_bench_pr_comment.py`

```python
#!/usr/bin/env python3
"""scripts/post_bench_pr_comment.py — render and post PR comment."""
import argparse
import json
import subprocess
import sys

def render_comment(report: dict) -> str:
    # … (build the Markdown from report['slo_compliance'] and
    #    report['results']) …
    return "..."

def main():
    p = argparse.ArgumentParser()
    p.add_argument('--report', required=True)
    p.add_argument('--pr', required=True)
    args = p.parse_args()

    with open(args.report) as f:
        report = json.load(f)
    body = render_comment(report)

    # Use gh CLI (already installed on ubuntu-latest runners).
    # `gh pr comment` is idempotent on the same PR but creates a
    # new comment each time. For dedup we'd need to find an
    # existing bot comment via the API — out of scope for D5.
    rc = subprocess.call([
        'gh', 'pr', 'comment', args.pr,
        '--body', body,
    ])
    sys.exit(rc)

if __name__ == '__main__':
    main()
```

---

## 10. Path filter dérivé du catalog

### 10.1 Problème

Aujourd'hui le `paths` du `bench.yml` est édité à la main. Quand
on ajoute un bench (D2/D3/D4), on doit :

* Ajouter `crates/efe-server/benches/**` au filter (déjà fait
  pour D1).
* Espérer ne rien oublier.

Le catalog résout ça : chaque entrée déclare ses `paths-watch`,
le `bench.yml` les ré-utilise.

### 10.2 Génération automatisée

Décision : `bench.yml` reste **statique** (les workflows GitHub
ne consomment pas de variables dynamiques pour leur `paths`
filter). Mais on fournit un **script générateur** qui produit
le bloc `paths:` à partir du catalog :

```bash
# scripts/regen_bench_yml_paths.sh
source scripts/benches.list

declare -A paths_set=()
collect_paths() {
  local _name="$1" _crate="$2" _sprint="$3" _blas="$4" _tier="$5" _slo="$6" paths="$7"
  for p in $paths; do
    paths_set[$p]=1
  done
}
run_bench_catalog collect_paths

# Always-include paths (independent of any single bench)
paths_set['crates/efe-*/Cargo.toml']=1
paths_set['scripts/bench*.sh']=1
paths_set['scripts/benches.list']=1
paths_set['scripts/parse_criterion_report.py']=1
paths_set['scripts/aggregate_bench_reports.py']=1
paths_set['scripts/post_bench_pr_comment.py']=1
paths_set['scripts/fixtures/**']=1
paths_set['scripts/gen_fixtures_v2.py']=1
paths_set['.github/workflows/bench.yml']=1

# Print sorted
echo "    paths:"
for p in $(echo "${!paths_set[@]}" | tr ' ' '\n' | sort -u); do
  echo "      - '$p'"
done
```

### 10.3 Discipline opérationnelle

* À chaque ajout de bench, l'auteur édite `benches.list`.
* CI inclut un **lint job** qui vérifie que la section `paths:` du
  `bench.yml` matche la sortie de `regen_bench_yml_paths.sh`. Un
  drift fail le PR avec un message clair :
  > "Re-run `scripts/regen_bench_yml_paths.sh > /tmp/expected.yml`
  > and update `bench.yml` paths section to match."

---

## 11. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Rationale |
|---|----------|---------------------|-----------|
| D1 | Fichier `benches.list` sourcé en bash (vs YAML/TOML) | YAML+yq | Pas de dépendance externe, parseable trivialement par bash, lisible par les 3 drivers |
| D2 | Schéma colon-séparé (vs JSON inline) | JSON | Plus court à lire, pas de quote escape cauchemar bash |
| D3 | Tier `default` / `micro` / `all` (vs `frequency`) | Tag-based (multi-tags) | Tier est mutuellement exclusif et simple ; pas besoin de la combinatoire des tags |
| D4 | Backward-compat `ONLY=mock\|real\|both` | Hard break | Limite la casse pour les invocations existantes (operators muscle memory) ; deprecated avec warning |
| D5 | `MODE=smoke` build-only (vs `--no-run`) | criterion's own | criterion `--no-run` est mal supporté en bencher format ; build-then-exit est plus contrôlé |
| D6 | `BENCH_TIMEOUT` per-bench (vs global) | Single timeout | Permet de donner 900s à server_rpc sans étirer le default |
| D7 | Retry 1× par défaut (vs 0) | No retry | Loopback TCP / OS allocator variance produit des fails transitoires non-représentatifs |
| D8 | Pas de matrix BLAS auto en CI (vs matrix [none, accelerate]) | matrix BLAS | Doublerait le wallclock (15→30 min) ; manual `workflow_dispatch` couvre le cas BLAS |
| D9 | DAG 5 jobs (vs single job) | Single sequential | Wallclock ×2.1 plus rapide |
| D10 | `bench-build` shared via cache (vs build-per-job) | Build redondant | Économie ~30 min de runner-minutes par PR |
| D11 | `cancel-in-progress: true` PR uniquement (vs always) | Always cancel | Push to main ne doit jamais perdre une mesure (besoin baseline historique) |
| D12 | PR comment via `gh pr comment` (vs API direct) | github3.py | gh CLI déjà installé sur ubuntu-latest, no extra deps |

---

## 12. 10 risques identifiés et mitigations

### R1 — Drift entre `benches.list` et `Cargo.toml`

**Impact** : un bench listé dans `benches.list` mais pas dans
`Cargo.toml` ⇒ erreur `cargo bench` "no bench target found".

**Mitigation** : le `bench-build` job exécute `cargo build -p
$crate --benches --release` *avant* la matrix. Si `Cargo.toml`
manque un target, le build échoue avant même que la matrix
démarre. Lint job en complément (cf. §10.3).

### R2 — Cache `bench-build` divergence entre PR

**Impact** : 2 PRs concurrents partagent le cache `bench-build`.
Si le cache est corrompu par PR-A, PR-B échoue mystérieusement.

**Mitigation** : `Swatinem/rust-cache@v2` utilise un hash de
`Cargo.lock` + `RUSTFLAGS` comme cache key. Les PRs avec lockfile
divergent ont des caches distincts. Si le hash matche mais le
contenu diverge, c'est un cache poisoning rare — `cache-on-failure`
le purge au prochain success.

### R3 — Artefact size explosion

**Impact** : 4 jobs uploadant chacun `target/criterion/**` =
~50 MB × 4 = 200 MB par PR × N PRs/jour = limite GitHub atteinte.

**Mitigation** :
* `retention-days: 30` (déjà dans v1).
* Filter l'artefact à `target/criterion/*/new/estimates.json`
  uniquement (suffisant pour le parser) ⇒ ~5 MB/job.
* Compresser avec `tar+zstd` avant upload si vraiment trop gros.

### R4 — `BENCH_TIMEOUT` insuffisant sur runner lent

**Impact** : Un runner ubuntu-latest exceptionnellement chargé
peut dépasser le timeout sur server_rpc (déjà 900 s).

**Mitigation** :
* Retry 1× rattrape ~80 % des cas.
* Si récurrent : bump `BENCH_TIMEOUT` dans la `bench-server` job
  uniquement.
* Marker la mesure comme "skipped" plutôt que "fail" si timeout
  ⇒ ne casse pas la PR.

### R5 — `gh pr comment` quotas

**Impact** : `gh pr comment` consomme l'API rate limit. Sur PRs
nombreux, on peut taper le quota (5 000 req/h pour `GITHUB_TOKEN`).

**Mitigation** :
* 1 comment par bench-aggregate run (pas par bench).
* Idempotency : un follow-up D5.5 implémente "find existing
  comment, edit instead of new".

### R6 — `compute_sprint{2,3,4}` produit des SLO fail

**Impact** : ces benches n'ont *jamais* tourné en CI. Première
exécution peut révéler des chiffres très différents de ceux
documentés dans `BENCHMARKS.md` § 6.

**Mitigation** :
* Le tier `micro` n'est PAS gated par défaut (pas de SLO pour
  eux dans `parse_criterion_report.py`).
* Premier run : capture les chiffres dans `bench_baselines/v0.1.5/`
  comme baseline pour les futurs.

### R7 — `aggregate_bench_reports.py` collisions

**Impact** : si 2 jobs CI uploadent un dossier `target/criterion/group_X/`
avec le même nom de groupe, le merge écrase silencieusement.

**Mitigation** : aucun bench ne partage de groupe avec un autre
(les noms `swarm_e2e_*`, `compute_pipeline_*`, etc. sont
naturellement disjoints — on l'a vérifié au moment du parser
unification). Si jamais 2 benches partagent un nom de groupe,
le lint job le détecte (vérification statique du catalog).

### R8 — Backward compat `ONLY` mapping incorrect

**Impact** : un opérateur lance `ONLY=real` espérant les benches
D1+D4 mais obtient autre chose (la mapping a divergé).

**Mitigation** : un test `tests/bench_runner_test.sh`
(out-of-scope D5, follow-up D5.6) qui vérifie le mapping. Pour
D5, on documente clairement et on émet un warning :
```bash
[ -n "${ONLY:-}" ] && echo "WARN: ONLY=$ONLY is deprecated, use SPRINT= or BENCHES= instead"
```

### R9 — Workflow YAML plus complexe = plus fragile

**Impact** : 5 jobs vs 1 = 5× plus de syntaxe YAML, 5× plus de
chemins d'erreur (artefact upload/download, dependencies).

**Mitigation** :
* Jobs partagent le même squelette (template via YAML
  reusable workflows si besoin futur).
* `actions/checkout@v4`, `actions/upload-artifact@v4`,
  `actions/download-artifact@v4` sont stables et bien testés.
* Le `bench-aggregate` job sert de canary : si lui foire, c'est
  visible immédiatement.

### R10 — Concurrent jobs share `target/` ?

**Impact** : si 2 matrix jobs tournent sur le **même** runner
(impossible avec `runs-on: ubuntu-latest` ⇒ 1 fresh runner per
job, mais on n'est jamais à l'abri d'un changement futur).

**Mitigation** : `actions/cache` keys distincts pour les outputs
(`bench-output-{job-name}`). Le shared cache `bench-build` est
read-only depuis la matrix.

---

## 13. Plan jour-par-jour (3 j wallclock)

### D5.1 (½ j) — Catalog `benches.list`

* Créer `scripts/benches.list` avec les 10 entrées.
* Implémenter `run_bench_catalog` helper.
* Lint script `scripts/check_benches_list.sh` qui vérifie :
  * Chaque entrée a 7 champs.
  * Chaque `name` matche un `[[bench]]` dans le Cargo.toml du
    `crate` correspondant.
  * Pas de doublons.
* `cargo test scripts/check_benches_list.sh` (manuel pour
  l'instant ; tests/d'intégration follow-up).

**DoD** :
- [ ] `benches.list` valide.
- [ ] `check_benches_list.sh` passe.

### D5.2 (1 j) — `bench_runner.sh` v2

* Refonte complète selon §6.4.
* Backward-compat `ONLY` + `QUICK` mappings.
* Per-bench timeout + retry.
* Test manuel : `./bench_runner.sh BENCHES=swarm_e2e MODE=smoke`
  → build only.

**DoD** :
- [ ] `BENCHES=` filter fonctionne.
- [ ] `SPRINT=` filter fonctionne.
- [ ] `TIER=micro` exécute compute_sprint{2,3,4}.
- [ ] `MODE=smoke` build only.
- [ ] `ONLY=mock` backward-compat (avec warning).
- [ ] Timeout déclenche correctement (test manuel : `BENCH_TIMEOUT=1
      ./bench_runner.sh BENCHES=swarm_e2e` doit timeout).

### D5.3 (½ j) — `aggregate_bench_reports.py` + `post_bench_pr_comment.py`

* Aggregator : merge N criterion roots en un seul.
* PR comment : Markdown rendering + `gh pr comment`.

**DoD** :
- [ ] Aggregator merge sans collision sur les 4 jobs simulés.
- [ ] PR comment rendering testé manuellement (output Markdown).

### D5.4 (½ j) — `bench.yml` v2

* DAG 5 jobs.
* `bench-build` partagé via cache.
* Matrix runs `bench_runner.sh` avec `BENCHES=` per-job.
* `bench-aggregate` télécharge tous les artefacts, agrège, post
  comment, gate régression.
* Path filter régénéré via `regen_bench_yml_paths.sh`.

**DoD** :
- [ ] YAML valide (`actionlint` clean).
- [ ] PR de test passe en < 15 min.

### D5.5 (½ j) — Tooling annexe + docs

* `scripts/regen_bench_yml_paths.sh` (générateur paths).
* `scripts/check_regression.py` (factorise le bloc inline du v1).
* `scripts/show_bench_report.py` (factorise le `python -c` v1).
* `BENCHMARKS.md` § 7 « Reproductibilité » mis à jour.
* `docs/bench/d5_unified_bench_orchestration_architecture.md` (ce
  document) référencé en intro de `BENCHMARKS.md`.

**DoD** :
- [ ] Tous les helpers Python ont un test smoke.
- [ ] BENCHMARKS.md mentionne le catalog.

---

## 14. Critères d'acceptation (DoD)

### 14.1 Catalog

- [ ] `scripts/benches.list` existe avec 10 entrées valides.
- [ ] `scripts/check_benches_list.sh` passe.
- [ ] Chaque entrée a un `name` qui matche un `[[bench]]` du
      Cargo.toml.

### 14.2 `bench_runner.sh` v2

- [ ] CLI grammar (BENCHES / SPRINT / TIER / EXCLUDE / MODE) fonctionne.
- [ ] Backward-compat `ONLY=mock|real|both` + `QUICK=1` fonctionne
      avec warning.
- [ ] `BENCH_TIMEOUT` + retry exercé en test manuel.
- [ ] **Tous les 10 benches** sont exécutables individuellement
      via `BENCHES=<name>`.
- [ ] `compute_sprint{2,3,4}` runs avec `TIER=micro` ou
      `BENCHES=compute_sprint2,compute_sprint3,compute_sprint4`.

### 14.3 `bench.yml` v2

- [ ] 5 jobs : `bench-build` + 4 matrix + `bench-aggregate`.
- [ ] PR de test verte avec wallclock < 20 min.
- [ ] Aggregate report contient TOUS les sub-benches des 4
      matrix jobs.
- [ ] Régression detection compare contre la dernière baseline
      (v0.1.4), pas v0.1.0.
- [ ] PR comment posté sur la PR de test.
- [ ] `cancel-in-progress` actif sur PR uniquement.

### 14.4 Tooling Python

- [ ] `aggregate_bench_reports.py` produit un JSON identique à
      ce que `parse_criterion_report.py` produit en single-job.
- [ ] `post_bench_pr_comment.py` rend Markdown propre.
- [ ] `check_regression.py` reproduit le comportement actuel
      inline du v1 (10 % threshold + warn 5 %, ×CI_COEFFICIENT).

### 14.5 Documentation

- [ ] `BENCHMARKS.md` § 7 mis à jour avec le nouveau workflow.
- [ ] `docs/bench/d5_unified_bench_orchestration_architecture.md`
      (ce document).
- [ ] `scripts/benches.list` documenté en tête (schéma).

### 14.6 Validation finale

- [ ] PR de test : tous les 10 benches s'exécutent en CI.
- [ ] Wallclock CI < 20 min (objectif 15 min).
- [ ] Aucun bench skipped silencieusement.
- [ ] PR comment apparaît automatiquement.

---

## 15. Out of scope & roadmap follow-ups

### 15.1 Hors-scope D5

* **Matrix CI par BLAS backend** — D5 reste no-BLAS par défaut ;
  manual `workflow_dispatch` couvre. **D5.5 follow-up** ajoute la
  matrix `[none, accelerate]` automatique pour main only (pas PR).
* **Idempotent PR comment** — D5 crée un nouveau comment à chaque
  run. **D5.6 follow-up** : `find existing bot comment, edit in
  place`.
* **Runner self-hosted** pour M3 Pro reference numbers — out of
  scope. La calibration M3 Pro reste manuelle.
* **Test suite pour `bench_runner.sh`** — D5.7 follow-up via
  bash testing framework (bats, shellcheck).
* **Migration des micro-benches Sprint S2/S3/S4 vers tier `default`** —
  les chiffres devront être validés une fois (D5 first run) avant
  de promouvoir. D5.8 follow-up.

### 15.2 Roadmap après D5

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D5.5** | Matrix BLAS auto sur main pushes | Tracker speedup BLAS dans les baselines historiques |
| **D5.6** | PR comment idempotent | UX comment thread propre |
| **D5.7** | Test suite `bench_runner_test.sh` | Sécuriser le driver contre régression |
| **D5.8** | Promote `compute_sprint{2,3,4}` à `tier=default` | Une fois validés post-D5 |
| **D6** | Bench `consensus_signals_pipeline` | Couvrir le 6-signal `MultiCriteriaDetector` |

### 15.3 Métrique de succès post-D5

À 30 jours après merge D5 :

> *« Tout nouveau bench est ajouté en 1 ligne dans `benches.list`,
>  s'exécute automatiquement dans la matrix CI au prochain PR, et
>  les SLO sont gates sans intervention manuelle. Le wallclock CI
>  reste < 20 min. La couverture exécutée est = couverture
>  écrite (10/10). »*

---

## Annexe A — Le fichier `benches.list` complet (référence)

Voir §5.2. Schéma :
`name:crate:sprint:blas-aware:tier:slo-prefix:paths-watch`.

## Annexe B — Diagramme du DAG d'exécution CI

```
Time →

T+0    [bench-build]██████  (5 min, compile both crates)
                      │
T+5    ┌──────────────┴──────────────┬───────────────────┬──────────────────┐
       ▼                             ▼                   ▼                  ▼
       [bench-core-mocks]████        [bench-core-infra]██ [bench-core-compute]████████  [bench-server]████████████
       (5 min)                       (3 min)              (10 min)                       (12 min)
       │                             │                    │                              │
T+12   │                             │                    │                              ▼
T+13   │                             │                    │                              [bench-aggregate]██  (2 min)
       │                             │                    │                              │
T+15   │                             │                    │                              ✅ PR comment posted
                                                                                         ✅ Regression check
                                                                                         ✅ Artefact uploaded

Total wallclock  : ~15 min (max(5+12, …) + 2 min aggregate)
Sequential equiv : ~30 min (sum of all)
Speedup          : ×2.0
```

## Annexe C — Exemple de PR comment

Voir §9.2. Le format inclut :
* Header (SHA, host, mode, BLAS).
* Compteur SLO (pass/warn/fail/skipped).
* Top 4 headline metrics.
* Lien vers full artefact.
* Section régression vs baseline.

## Annexe D — Checklist d'exécution architect → impl

```
□ D5.1.1  scripts/benches.list (10 entries, 7 fields each)
□ D5.1.2  scripts/check_benches_list.sh (lint validator)
□ D5.1.3  Manual run : `bash check_benches_list.sh` exit 0

□ D5.2.1  Refactor scripts/bench_runner.sh
  □ source benches.list at top
  □ filter logic (BENCHES > EXCLUDE > SPRINT > TIER)
  □ pre-flight build (deduplicated by crate)
  □ MODE=smoke → exit after build
  □ per-bench timeout + retry loop
  □ backward-compat ONLY → SPRINT translation
  □ output filename derived from name (s/_/-/g)
□ D5.2.2  Manual smoke tests (BENCHES, SPRINT, TIER, EXCLUDE)

□ D5.3.1  scripts/aggregate_bench_reports.py
□ D5.3.2  scripts/post_bench_pr_comment.py
□ D5.3.3  scripts/check_regression.py (factor out from bench.yml)
□ D5.3.4  scripts/show_bench_report.py (factor out from bench.yml)

□ D5.4.1  Refactor .github/workflows/bench.yml v2
  □ bench-build job (shared cache)
  □ bench-core-mocks (matrix node 1)
  □ bench-core-infra (matrix node 2)
  □ bench-core-compute (matrix node 3, includes micro tier)
  □ bench-server (matrix node 4)
  □ bench-aggregate (depends on all matrix nodes)
  □ workflow_dispatch with `tier`, `blas_backend` inputs
  □ paths section auto-generated from regen_bench_yml_paths.sh
□ D5.4.2  PR test : all 10 benches run, wallclock < 20 min

□ D5.5.1  scripts/regen_bench_yml_paths.sh (paths generator)
□ D5.5.2  BENCHMARKS.md § 7 update (mention catalog)
□ D5.5.3  docs/bench/d5_unified_bench_orchestration_architecture.md ref

□ Validation finale
  □ shellcheck scripts/bench_runner.sh
  □ shellcheck scripts/benches.list
  □ shellcheck scripts/regen_bench_yml_paths.sh
  □ shellcheck scripts/check_benches_list.sh
  □ python3 -m py_compile scripts/aggregate_bench_reports.py
  □ python3 -m py_compile scripts/post_bench_pr_comment.py
  □ python3 -m py_compile scripts/check_regression.py
  □ actionlint .github/workflows/bench.yml (or yamllint)
  □ End-to-end: PR opened, all 5 jobs pass, comment appears
```

---

**Fin de l'architecture D5.** Le sprinter peut implémenter en
suivant Annexe D, en revenant à §6 pour le détail du driver et §7
pour le DAG CI.

> **Approbation requise avant implémentation** :
> - [ ] Architecte performance (revue catalog + CLI grammar).
> - [ ] Lead infra (revue DAG CI + cache strategy).
> - [ ] Owner observability (revue PR comment format).
