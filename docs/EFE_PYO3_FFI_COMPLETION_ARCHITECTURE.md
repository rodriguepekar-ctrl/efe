# EFE — PyO3 FFI Complétion (Φ-C-FFI, D26 → D28)

> Document architecture **complétion / gap-closing** pour `efe-ffi`.
> Lecture obligatoire **avant** `EFE_FFI_ARCHITECTURE.md` (D25 baseline)
> car les artefacts livrés ici **ferment** les SLO contractuels F-1..F-15
> en transformant la prose de l'archi D25 en gates CI exécutables.
>
> **Sprint** : Φ-C-FFI (D26 → D28).
> **Statut** : architecture **prête à exécution** — code drop joint.
> **Doctrine** : pas un seul SLO décrit en prose ne reste hors-CI ; pas
> un seul instrument déclaré ne reste sans appelant production ; pas
> un seul script référencé par workflow ne reste absent.

---

## Table des matières

- **§ 0** — Méta, conventions, glossaire chirurgical
- **§ 1** — Mission Φ-C-FFI & contrat opposable
- **§ 2** — Position vs `EFE_FFI_ARCHITECTURE.md` (= gap analysis)
- **§ 3** — État AS-IS chirurgical (post-D25)
- **§ 4** — Décomposition du budget Python→Rust→Python
- **§ 5** — Surface 1 : SLO formels (15 SLO YAML-parseable)
- **§ 6** — Surface 2 : Build & wheel matrix (5 wheels + SBOM + Sigstore)
- **§ 7** — Surface 3 : Numerical equivalence Python ↔ gRPC
- **§ 8** — Surface 4 : Memory safety harness (refcount + leaks + GIL)
- **§ 9** — Surface 5 : Cold-start & import budget
- **§ 10** — Surface 6 : Stub `.pyi` ↔ Rust API sync
- **§ 11** — Surface 7 : ABI3 verification (cross-Python compat)
- **§ 12** — Surface 8 : Observabilité in-process (8 instruments)
- **§ 13** — Surface 9 : Defensive — GIL contention, panic safety, drop ordering
- **§ 14** — Surface 10 : Bench harness (Criterion Rust + pytest-benchmark)
- **§ 15** — Surface 11 : Contractuelle — ABI stability promise + dropin
- **§ 16** — Matrice complète des artefacts Φ-C-FFI (30 entrées)
- **§ 17** — Plan d'implémentation D26 → D28 chirurgical
- **§ 18** — Risques identifiés & mitigations (20 risques)
- **§ 19** — Décisions de conception clés (18)
- **§ 20** — Anti-régression vs gRPC client
- **§ 21** — Critères d'acceptation (DoD à 100 %)
- **§ 22** — Hand-off & dépendances downstream
- **§ 23** — Annexes (A → H)

---

## § 0. Méta, conventions, glossaire chirurgical

### 0.1 Doctrine

| Doctrine | Énoncé chirurgical |
|---|---|
| **D-FFI-1 — SLO opposable** | Tout SLO décrit en prose dans l'archi D25 doit avoir un equivalent **YAML parseable par `check_slo.py`**. Aucune exception. |
| **D-FFI-2 — Production caller** | Tout instrument Prometheus déclaré doit avoir **un appelant production documenté** (`grep -r` retourne ≥ 1 hit hors `#[cfg(test)]`). |
| **D-FFI-3 — Script référencé = script existant** | Tout script référencé par un workflow CI ou un runbook doit exister dans l'arbre à la version freeze. Pas un seul `command not found` toléré. |
| **D-FFI-4 — Pin test sync** | Toute API publique Python (`.pyi`) doit avoir un pin test qui FAIT cassure si la surface Rust dérive. |
| **D-FFI-5 — Determinism cross-runner** | Verdict bit-identique cross-OS, cross-arch, cross-Python pour `(text, catalog, seed)` fixés. 100 runs. |
| **D-FFI-6 — ABI3 strict** | Une seule wheel `*.abi3.whl` par OS×arch couvre Python 3.10/3.11/3.12/3.13. Audit `verify_abi3.py` enforce. |
| **D-FFI-7 — Memory safety stable** | RSS delta ≤ 5 % après 10 000 classify (tolerance gauging via `tracemalloc` + `psutil.Process().memory_info()`). |

### 0.2 Glossaire chirurgical

| Terme | Définition |
|---|---|
| **ABI3** | Application Binary Interface stable Python 3.10+. Une wheel ABI3 charge sur 3.10 → 3.13 (et up) sans rebuild. |
| **Cold start** | Mesure `time.perf_counter()` entre `import efe` et l'instant où `efe.version()` est appelable. SLO F-6 : ≤ 100 ms p95. |
| **GIL release ratio** | Fraction du temps total d'une opération pendant lequel le GIL est relâché via `py.allow_threads`. SLO F-8 : ≥ 95 %. |
| **Numerical equivalence** | Pour la même `(text, catalog, seed)`, le verdict Python (PyO3) et le verdict gRPC sont **bit-identiques** sur 100 fixtures. SLO F-7. |
| **Wheel size budget** | Taille compressée d'une wheel `*.whl`. SLO F-3 : ≤ 8 MiB manylinux x86_64. |
| **Import budget** | Somme des `import` ms pour `import efe`. SLO F-6. |
| **Maturin** | Build backend Python qui transforme un crate Rust en wheel. |
| **manylinux** | Standard PyPA pour wheels Linux compatibles cross-distro (CentOS 7 base). |
| **DT-F-X** | Deferred Task FFI X. Référence à un ticket post-D25 (cf. §17). |
| **`efe._native`** | Module C-extension produit par maturin ; `efe/__init__.py` re-export. |
| **Stub `.pyi`** | Fichier d'annotations typées pour mypy/pylance. Hand-maintained dans D25, validé en CI dans Φ-C-FFI. |
| **Sigstore Rekor** | Transparency log public ; les wheels signées Φ-C-FFI sont indexées. |
| **PEP 600** | Standard manylinux ; manylinux_2_17 = compatible glibc ≥ 2.17 (CentOS 7). |

### 0.3 Conventions de référence

* SLO contractuel : `SLO-FFI-<KEY>` (= 15 entrées dans `bench_baselines/v0.6.0/ffi_slos.yaml`).
* Instrument Prometheus : `efe_ffi_<noun>_<unit>` (cf. § 12).
* Artefact tracking : `Φ-C-FFI.A<N>` (où N est dans la matrice § 16).
* Workflow CI : `ffi-test.yml` (PR-gate), `ffi-publish.yml` (release), `ffi-bench.yml` (nightly + tag).

---

## § 1. Mission Φ-C-FFI & contrat opposable

### 1.1 Mission

> Faire que la wheel `efe-x.y.z-cp310-abi3-manylinux_2_17_x86_64.whl`
> soit **production-grade, contractuellement opposable, et hardware-
> validée cross-platform**.
>
> Concrètement :
>
> 1. Une commande `pip install efe` réussit sur Python 3.10 → 3.13,
>    Linux x86_64/aarch64, macOS x86_64/arm64, Windows x86_64 — soit
>    **20 combinaisons OS×arch×py** couvertes par **5 wheels ABI3**.
> 2. Un `import efe` complet en **< 100 ms p95** (cold start).
> 3. Un `efe.classify_zero_copy(text=...)` répond en **p99 ≤ 40 ms**
>    en CPU-only, **≥ 5× plus rapide que `EfeClient` (gRPC)**.
> 4. Le verdict est **bit-identique vs gRPC** sur 100 fixtures
>    `(text, catalog, seed)`.
> 5. Le GIL est relâché ≥ 95 % du temps total d'un classify.
> 6. La wheel pèse ≤ 8 MiB compressée.
> 7. Aucune fuite mémoire après 10 000 classify (RSS delta ≤ 5 %).
> 8. Toutes les wheels sont **signées via Sigstore cosign + Rekor
>    transparency log**.
> 9. Un SBOM CycloneDX accompagne chaque release.

### 1.2 Contrat opposable

| Promesse | SLO | Mesure | Gating |
|---|---|---|---|
| Latence | **SLO-FFI-LATENCY-P99** ≤ 40 ms | `tests/bench/native_vs_grpc.py` p99 | nightly `ffi-bench.yml` |
| Speedup | **SLO-FFI-SPEEDUP** ≥ 5× | `native_p99 / grpc_p99` | tag-gate `ffi-bench.yml` |
| Equivalence | **SLO-FFI-EQUIVALENCE** = 100/100 | `test_numerical_equiv.py` | PR-gate `ffi-test.yml` |
| GIL | **SLO-FFI-GIL-RELEASED** ≥ 0.95 | `test_gil_release.py` | PR-gate |
| Wheel | **SLO-FFI-WHEEL-SIZE** ≤ 8 MiB | `du -k dist/*.whl` | tag-gate `ffi-publish.yml` |
| Cold start | **SLO-FFI-COLD-START** ≤ 100 ms p95 | `test_cold_start.py` | nightly |
| Determinism | **SLO-FFI-DETERMINISM** 100/100 | `test_determinism_strict.py` | PR-gate |
| Memory | **SLO-FFI-MEMORY-DRIFT** ≤ 5 % | `test_memory_leak.py` | nightly |
| ABI3 | **SLO-FFI-ABI3** PASS | `verify_abi3.py` | tag-gate |
| Stub sync | **SLO-FFI-STUB-SYNC** PASS | `test_stub_pyi_in_sync` | PR-gate |
| Cross-Python | **SLO-FFI-CROSS-PY** 3.10..3.13 PASS | matrix `ffi-test.yml` | PR-gate |
| SBOM | **SLO-FFI-SBOM** présent | `dist/*.cdx.json` | tag-gate |
| Sigstore | **SLO-FFI-SIGSTORE** présent | `dist/*.sig` + `.pem` | tag-gate |
| Drop-in | **SLO-FFI-DROPIN** PASS | `test_dropin_migration.py` | PR-gate |
| Wheels | **SLO-FFI-WHEEL-COUNT** ≥ 5 | `find dist -name '*.whl' \| wc -l` | tag-gate |

### 1.3 Anti-mission (ce que Φ-C-FFI **ne fait pas**)

* Ne réécrit pas la surface Rust. La surface publique `efe._native`
  est gelée au snapshot D25 ; Φ-C-FFI **valide** + **mesure** +
  **gate** cette surface.
* Ne livre pas le **runtime tokio multi-tenant** (= DT-F-ASYNC-3 reste
  en D29).
* Ne livre pas la **zero-copy ELS via ndarray** (= DT-F-ELS-1 reste
  D29).
* Ne livre pas le **gRPC drop-in shim** (= DT-F-DROPIN-1 reste D29).
* Pas de bench GPU CUDA-side ici : la couverture GPU dérive du
  `EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md`.

---

## § 2. Position vs `EFE_FFI_ARCHITECTURE.md` (= gap analysis)

### 2.1 Ce que l'archi D25 couvre déjà

* Topologie crate (`efe-ffi → efe-core → efe-kernels`).
* Module layout (10 modules Rust).
* Toolchain (PyO3 0.21, abi3-py310, maturin 1.7+).
* API publique Python (15 ops).
* Build path (`maturin develop` / `maturin build`).
* Error mapping Rust → Python (10 classes).
* Stratégie de tests (pyramide).
* CI workflow basique (`ffi-test.yml` + `ffi-publish.yml`).
* SLO F-1..F-15 décrits **en prose**.

### 2.2 Ce que Φ-C-FFI ajoute (= les gaps fermés)

| # | Gap | Étage | Sévérité | Φ-C-FFI artefact |
|---|---|---|---|---|
| G1 | Pas de SLO YAML parseable | SLO | **CRITIQUE** | `bench_baselines/v0.6.0/ffi_slos.yaml` |
| G2 | Pas de bench Criterion Rust-side | Mesure | HIGH | `crates/efe-ffi/benches/ffi_overhead.rs` |
| G3 | Pas de test numerical equivalence formel | Tests | **CRITIQUE** | `test_numerical_equiv.py` + fixtures |
| G4 | Pas de memory leak harness | Tests | HIGH | `test_memory_leak.py` |
| G5 | Pas de cold start budget gate | Tests | MED | `test_cold_start.py` |
| G6 | Pas de wheel size gate | Build | MED | `verify_wheel_size.py` |
| G7 | Pas de ABI3 verification | Build | HIGH | `verify_abi3.py` |
| G8 | Pas de stub sync pin test | DX | HIGH | `test_efe_ffi_structure.py` |
| G9 | Pas de Sigstore wheel signing | Sécurité | HIGH | extension `ffi-publish.yml` |
| G10 | Pas de SBOM CycloneDX | Compliance | MED | extension `ffi-publish.yml` |
| G11 | Pas d'observability in-process | Obs | HIGH | `efe-telemetry/src/metrics/ffi_instruments.rs` |
| G12 | Pas de runbook FFI ops | Ops | MED | 4 runbooks |
| G13 | Pas de plan day-by-day | Process | MED | § 17 |
| G14 | Pas de matrix Python 3.11/3.13 | Tests | HIGH | extension `ffi-test.yml` |
| G15 | Pas de GIL released % measurement runtime | Tests | HIGH | `test_gil_release.py` extended |
| G16 | Pas de async overhead bench | Mesure | MED | `test_async_overhead.py` |
| G17 | Pas de LRU bundle cache | Perf | HIGH | `crates/efe-ffi/src/cache.rs` |
| G18 | Pas de drop-in migration test | Migration | MED | `test_dropin_migration.py` |
| G19 | Pas de cross-platform determinism | Tests | HIGH | `test_determinism_strict.py` |
| G20 | Pas de Criterion → SLO bridge | Mesure | HIGH | `scripts/bench/criterion_ffi_to_slo.py` |

### 2.3 Doctrine de gap-closing

Le pattern Φ-D CUDA Kernels Completion (récent) a montré que les
défauts "silently broken SLO gating" surviennent quand :

* Un champ YAML est utilisé que `check_slo.py` n'interprète pas → SLO **silencieusement skippé**.
* Une métrique Prometheus est déclarée mais n'a pas d'appelant production → graphes plats.
* Un script est référencé par workflow CI sans exister sur disque → CI fail au premier run.

Φ-C-FFI réplique chirurgicalement le pattern : **chaque artefact ouvert
ferme un SLO ou un instrument ou un script.**

---

## § 3. État AS-IS chirurgical (post-D25)

### 3.1 Inventaire code existant

```
crates/efe-ffi/
├── Cargo.toml                  # 80 LOC — features cuda/nccl
├── pyproject.toml              # 90 LOC — maturin + pytest config
├── README.md                   # ~50 LOC
├── src/
│   ├── lib.rs                  # 137 LOC — #[pymodule] entry
│   ├── error.rs                # 170 LOC — 10 exceptions + map
│   ├── verdict.rs              # 219 LOC — PyVerdict frozen
│   ├── bundle.rs               # 270 LOC — PyBundle + load_bundle
│   ├── types.rs                # 321 LOC — SwarmSpec + Builder + AlignmentCone
│   ├── els.rs                  # 177 LOC — PyELS (STUB ; copy-not-zero)
│   ├── controller.rs           # 356 LOC — PySwarmController (STUB)
│   ├── client.rs               # 220 LOC — PyEfeNativeClient
│   ├── classify.rs             # 137 LOC — top-level one-shot
│   └── async_.rs               # 114 LOC — aclassify_zero_copy
└── python/
    └── efe/
        ├── __init__.py         # ~80 LOC — re-exports
        ├── py.typed            # marker file
        ├── _native.pyi         # ~250 LOC — hand-maintained stubs
        └── tests/
            ├── conftest.py     # 68 LOC — bundle + catalog_path fixtures
            ├── test_smoke.py   # 41 LOC
            ├── test_classify.py        # 63 LOC
            ├── test_async.py           # 60 LOC
            ├── test_determinism.py     # 75 LOC
            ├── test_errors.py          # 114 LOC
            ├── test_gil_release.py     # 53 LOC
            ├── test_zero_copy.py       # 55 LOC
            └── bench/
                └── native_vs_grpc.py   # 170 LOC
```

**Total Rust** : ~2 121 LOC.
**Total Python tests** : ~530 LOC.
**Total .pyi** : ~250 LOC.

### 3.2 Statut par sous-système

| Sous-système | Statut D25 | Statut post-Φ-C-FFI | Gap fermé |
|---|---|---|---|
| Module entry point | ✅ stable | ✅ + cold start metric | G11 |
| Exception hierarchy | ✅ stable | ✅ stable | — |
| `PyVerdict` | ✅ stable | ✅ + equality strict mode | — |
| `PyBundle` + `load_bundle` | ✅ basique | ✅ + LRU cache (G17) | G17 |
| `PySwarmSpec` + Builder | ✅ stable | ✅ stable | — |
| `PyELS` | ⚠️ stub (copy) | ⚠️ stub (DT-F-ELS-1 D29) | — |
| `PySwarmController` | ⚠️ stub | ⚠️ stub (DT-F-WIRE-1 D29) | — |
| `PyEfeNativeClient` | ✅ basique | ✅ + observability | G11 |
| `classify_zero_copy` | ✅ basique | ✅ + cache hit metric | G11/G17 |
| `aclassify_zero_copy` | ✅ basique (spawn_blocking) | ✅ + async overhead bench | G16 |
| SLO formels YAML | ❌ | ✅ 15 SLO | G1 |
| Numerical equiv | ❌ | ✅ 100 fixtures | G3 |
| Memory leak | ❌ | ✅ 10 000 iter | G4 |
| Cold start | ❌ | ✅ 100 ms p95 gate | G5 |
| Wheel size | ❌ | ✅ 8 MiB gate | G6 |
| ABI3 verify | ❌ | ✅ symbol scanner | G7 |
| Stub sync | ❌ | ✅ pin test | G8 |
| Sigstore signing | ❌ | ✅ cosign + Rekor | G9 |
| SBOM | ❌ | ✅ CycloneDX | G10 |
| Observability | ❌ | ✅ 8 instruments | G11 |
| Runbooks | ❌ | ✅ 4 docs | G12 |
| Plan D26→D28 | ❌ | ✅ day-by-day | G13 |
| Cross-Python matrix | ⚠️ 3.10/3.12 | ✅ 3.10/3.11/3.12/3.13 | G14 |
| Determinism strict | ⚠️ informational | ✅ 100 runs strict | G19 |
| Criterion bench Rust | ❌ | ✅ `benches/ffi_overhead.rs` | G2 |
| Async overhead | ❌ | ✅ bench | G16 |
| Drop-in migration | ❌ | ✅ test + helper | G18 |

### 3.3 Mesure quantitative AS-IS

| Métrique | Mesure D25 | Cible Φ-C-FFI |
|---|---|---|
| Tests Rust unit | 18 verts | 30+ verts |
| Tests Python | 8 fichiers, ~ 25 tests | 14 fichiers, ~ 60 tests |
| LOC Rust | 2 121 | ~ 3 000 |
| LOC Python tests | 530 | ~ 1 400 |
| Workflows CI | 2 (`ffi-test`, `ffi-publish`) | 3 (+ `ffi-bench`) |
| Runbooks | 0 | 4 |
| SLO YAML entries | 0 | 15 |
| Instruments Prom | 0 | 8 |

---

## § 4. Décomposition du budget Python→Rust→Python

Pour rendre l'analyse opposable, on décompose le temps d'un
`classify_zero_copy` en **étages** mesurables individuellement. Chaque
étage a un budget contractuel.

### 4.1 Étages canoniques

```text
┌─────────────────────────────────────────────────────────────────┐
│  T = T_marshal_in + T_acquire + T_kernel + T_release + T_marshal_out │
│                  └──────────── T_inside_GIL ────────────┘            │
│                            └──── T_GIL_released ────┘                │
└─────────────────────────────────────────────────────────────────┘

T_marshal_in       : Python str → Rust &str + tokenize       ≈ 200 µs
T_acquire          : py.allow_threads enter                  ≈ 5 µs
T_kernel           : swarm execution (= D25 stub ou D29 real) ≈ 30 ms (CPU stub)
T_release          : py.allow_threads exit                   ≈ 5 µs
T_marshal_out      : Rust Verdict → Python Verdict           ≈ 200 µs

Total CPU (stub D25)  ≈ 30 ms p50 — déjà sous le budget 40 ms
Total CPU (réel D29)  ≈ 100 ms p50 — proche du budget gRPC 200 ms ; speedup 2×
Total GPU (T4)        ≈ 5 ms p99 — bien sous le budget 40 ms ; speedup 40×
```

### 4.2 Étages observables (instruments Prometheus § 12)

| Étage | Instrument | Unit |
|---|---|---|
| `T_marshal_in` | `efe_ffi_marshal_in_seconds` | seconds Histogram |
| `T_kernel` | `efe_ffi_kernel_seconds` | seconds Histogram |
| `T_marshal_out` | `efe_ffi_marshal_out_seconds` | seconds Histogram |
| `T_inside_GIL` | `efe_ffi_gil_held_seconds` | seconds Histogram |
| `T_GIL_released` | `efe_ffi_gil_released_seconds` | seconds Histogram |
| Total | `efe_ffi_classify_seconds` | seconds Histogram |

### 4.3 Ratio GIL released (SLO F-8)

```text
gil_released_ratio = T_GIL_released / T_total ≥ 0.95
```

Mesure par `test_gil_release.py` (cf. § 13.3) : lance N classify
concurrents depuis 4 threads Python ; si le GIL était tenu ≥ 5 %, la
parallélisation flatlinerait. Le bench compare 1-thread vs 4-threads ;
le speedup doit être ≥ 3.5× (= ratio 0.95 = 4/(1+0.16)).

---

## § 5. Surface 1 : SLO formels (15 SLO YAML-parseable)

### 5.1 Fichier `bench_baselines/v0.6.0/ffi_slos.yaml`

Le fichier est consommé par `eval/scripts/check_slo.py` (= le même
parser que les autres SLO domaines). Format :

```yaml
version: v0.6.0
domain: ffi
description: |
  EFE PyO3 FFI SLO baselines — Φ-C-FFI freeze 2026-05-13.
host_fingerprint_baseline: "PLACEHOLDER_compute_at_freeze"
slos:
  - id: SLO-FFI-LATENCY-P99
    surface: latency
    threshold_seconds: 0.040
    direction: lower_or_equal
    measure_source: native_vs_grpc.json:p99
    host_fingerprint_required: false
  - id: SLO-FFI-SPEEDUP
    surface: latency
    threshold_ratio: 5.0
    direction: greater_or_equal
    measure_source: native_vs_grpc.json:speedup
    host_fingerprint_required: false
  # ... 13 autres SLO ...
```

### 5.2 Extension `check_slo.py`

Le parser doit accepter ces nouveaux champs (pre-existants pour la
plupart depuis Φ-D CUDA) :

* `threshold_seconds` — déjà supporté.
* `threshold_ratio` — déjà supporté.
* `threshold_count` — déjà supporté.
* `threshold_bytes` — déjà supporté.
* `threshold_pass_count` — **nouveau** (= "100/100 passes").

L'audit Φ-C-FFI level 1 vérifiera que `threshold_pass_count` n'est
pas silencieusement skippé (= pattern Φ-D A1).

### 5.3 Matrice de gating

| SLO | PR-gate `ffi-test.yml` | Nightly `ffi-bench.yml` | Tag-gate `ffi-publish.yml` |
|---|---|---|---|
| LATENCY-P99 | ⚠️ informational | ✅ enforced | ✅ enforced |
| SPEEDUP | ⏭️ skipped (no gRPC) | ✅ enforced | ✅ enforced |
| EQUIVALENCE | ✅ enforced (100/100) | ✅ enforced | ✅ enforced |
| GIL-RELEASED | ✅ enforced | ✅ enforced | ✅ enforced |
| WHEEL-SIZE | — | — | ✅ enforced |
| COLD-START | ⚠️ inf | ✅ enforced | ✅ enforced |
| DETERMINISM | ✅ enforced | ✅ enforced | ✅ enforced |
| MEMORY-DRIFT | — | ✅ enforced | ✅ enforced |
| ABI3 | — | — | ✅ enforced |
| STUB-SYNC | ✅ enforced | — | — |
| CROSS-PY | ✅ enforced | — | ✅ enforced |
| SBOM | — | — | ✅ enforced |
| SIGSTORE | — | — | ✅ enforced |
| DROPIN | ✅ enforced | — | ✅ enforced |
| WHEEL-COUNT | — | — | ✅ enforced |

---

## § 6. Surface 2 : Build & wheel matrix

### 6.1 Matrice de wheels finale

| # | OS | Arch | Python | Wheel filename |
|---|---|---|---|---|
| 1 | Linux | x86_64 | 3.10–3.13 (ABI3) | `efe-X.Y.Z-cp310-abi3-manylinux_2_17_x86_64.whl` |
| 2 | Linux | aarch64 | 3.10–3.13 (ABI3) | `efe-X.Y.Z-cp310-abi3-manylinux_2_17_aarch64.whl` |
| 3 | macOS | arm64 | 3.10–3.13 (ABI3) | `efe-X.Y.Z-cp310-abi3-macosx_11_0_arm64.whl` |
| 4 | macOS | x86_64 | 3.10–3.13 (ABI3) | `efe-X.Y.Z-cp310-abi3-macosx_10_15_x86_64.whl` |
| 5 | Windows | x86_64 | 3.10–3.13 (ABI3) | `efe-X.Y.Z-cp310-abi3-win_amd64.whl` |
| + | sdist | — | — | `efe-X.Y.Z.tar.gz` |

**Total** : 5 wheels + 1 sdist = 6 fichiers par release.

### 6.2 Pipeline tag-gate (extension de `ffi-publish.yml`)

Steps ajoutés par Φ-C-FFI :

```yaml
# 1. Wheel size check (per-wheel + total)
- name: Verify wheel size budget (SLO-FFI-WHEEL-SIZE)
  run: python scripts/bench/verify_wheel_size.py --max-mib 8 dist/

# 2. ABI3 symbol scanner
- name: Verify ABI3 strict (SLO-FFI-ABI3)
  run: python scripts/bench/verify_abi3.py dist/

# 3. SBOM CycloneDX
- name: Generate SBOM CycloneDX (SLO-FFI-SBOM)
  run: |
    pip install cyclonedx-bom
    cyclonedx-py environment -o dist/efe-${{ github.ref_name }}.cdx.json

# 4. Sigstore cosign signing (SLO-FFI-SIGSTORE)
- name: Sign wheels via Sigstore
  uses: sigstore/cosign-installer@v3
- run: |
    for f in dist/*.whl dist/*.tar.gz dist/*.cdx.json; do
      cosign sign-blob --yes "$f" \
        --output-signature "$f.sig" \
        --output-certificate "$f.pem"
    done

# 5. Rekor transparency log verification
- name: Verify Rekor entries
  run: |
    for f in dist/*.whl; do
      cosign verify-blob --certificate "$f.pem" --signature "$f.sig" "$f"
    done
```

### 6.3 Acceptance criteria publish

* Tous les 5 wheels présents (`SLO-FFI-WHEEL-COUNT`).
* Chaque wheel ≤ 8 MiB (`SLO-FFI-WHEEL-SIZE`).
* Chaque wheel passe ABI3 symbol scanner (`SLO-FFI-ABI3`).
* Chaque wheel a un `.sig` + `.pem` + entrée Rekor (`SLO-FFI-SIGSTORE`).
* Un SBOM CycloneDX accompagne (`SLO-FFI-SBOM`).

---

## § 7. Surface 3 : Numerical equivalence Python ↔ gRPC

### 7.1 Mission

SLO F-7 (= `SLO-FFI-EQUIVALENCE`) dit : "Verdict bit-identical vs gRPC
sur 100 fixtures". On code ça chirurgicalement :

* 100 fixtures `(text, catalog_id, seed)` figées dans
  `python/efe/tests/fixtures/numerical_equiv.json`.
* Pour chacune : 
  * Calcule `verdict_native = efe.classify_zero_copy(text, ...)`
  * Compare au snapshot pré-calculé `expected[i]`.
  * Le snapshot est aussi le verdict qu'aurait retourné un `EfeClient`
    gRPC sur la même fixture **dans le D25-freeze environment**.

### 7.2 Format fixture

```json
{
  "version": "v0.6.0",
  "fixtures": [
    {
      "id": "fixture-001",
      "text": "I want to dispute the charge on my account",
      "catalog_id": "router-v1",
      "seed": 42,
      "expected": {
        "label": "billing-dispute",
        "confidence": 0.9876,
        "cone_id": "cone-router-v1",
        "rounds_executed": 12,
        "swarm_id": "swarm-a1b2c3d4e5f60708",
        "catalog_version": "1.0.0"
      }
    },
    /* ... 99 more ... */
  ]
}
```

### 7.3 Procédure de freeze

Lors du gel Φ-C-FFI :

```bash
# 1. Build the FFI wheel from current HEAD
maturin develop --release -m crates/efe-ffi/Cargo.toml

# 2. Run the equivalence harness in CAPTURE mode
python scripts/bench/freeze_numerical_equiv.py \
    --out crates/efe-ffi/python/efe/tests/fixtures/numerical_equiv.json \
    --count 100 \
    --catalog crates/efe-ffi/python/efe/tests/fixtures/catalog.json

# 3. Verify pass rate is 100/100 (sanity)
python -m pytest crates/efe-ffi/python/efe/tests/test_numerical_equiv.py -v
```

### 7.4 SLO `SLO-FFI-EQUIVALENCE` (cf. § 5)

```yaml
- id: SLO-FFI-EQUIVALENCE
  surface: correctness
  threshold_pass_count: 100
  expected_count: 100
  direction: equal
  measure_source: numerical_equiv.json:passes
```

---

## § 8. Surface 4 : Memory safety harness

### 8.1 Mission

SLO `SLO-FFI-MEMORY-DRIFT` : après 10 000 classify, le RSS du process
Python n'augmente pas de plus de 5 % par rapport au RSS post-warmup.

### 8.2 Méthodologie

```python
# python/efe/tests/test_memory_leak.py (abrégé)

import gc, os, psutil, tracemalloc
import efe

def test_no_leak_after_10000_classify(bundle, catalog_path):
    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    proc = psutil.Process(os.getpid())

    # Warmup : 1000 calls pour stabiliser RSS
    for _ in range(1000):
        client.classify("warmup text")
    gc.collect()
    rss_baseline = proc.memory_info().rss

    # Mesure : 10 000 calls
    tracemalloc.start()
    for i in range(10_000):
        client.classify(f"text {i % 100}")
    gc.collect()
    rss_final = proc.memory_info().rss

    drift_pct = (rss_final - rss_baseline) / rss_baseline * 100
    snapshot = tracemalloc.take_snapshot()
    top = snapshot.statistics("lineno")[:10]

    print(f"\nRSS baseline={rss_baseline/2**20:.1f} MiB ; "
          f"final={rss_final/2**20:.1f} MiB ; drift={drift_pct:.2f} %")
    for stat in top:
        print(f"  {stat}")

    tracemalloc.stop()
    client.close()
    assert drift_pct < 5.0, (
        f"SLO-FFI-MEMORY-DRIFT violated : drift {drift_pct:.2f} % > 5 %"
    )
```

### 8.3 Refcount audit

En complément, un mini-harness vérifie qu'aucun objet PyO3 n'oublie
de `decref` :

```python
import sys, efe

def test_verdict_refcount_stable():
    bundle = efe.load_bundle("/etc/efe/bundles/active")
    client = efe.EfeNativeClient(bundle, "/etc/efe/catalogs/c.json")

    # Capture les refcount baselines
    rc_bundle = sys.getrefcount(bundle)
    rc_client = sys.getrefcount(client)

    # Run 100 classify ; chaque verdict est local au scope
    for _ in range(100):
        v = client.classify("hello")
        del v
    
    # Refcount des objets longs ne doit PAS dériver
    assert sys.getrefcount(bundle) == rc_bundle
    assert sys.getrefcount(client) == rc_client
    client.close()
```

---

## § 9. Surface 5 : Cold-start & import budget

### 9.1 Mission

SLO `SLO-FFI-COLD-START` : `import efe ; efe.version()` doit retourner
en ≤ 100 ms p95.

### 9.2 Méthode

```python
# python/efe/tests/test_cold_start.py

import subprocess, sys, time, json

ITERATIONS = 30
SCRIPT = (
    "import time, sys; "
    "t0 = time.perf_counter(); "
    "import efe; "
    "v = efe.version(); "
    "t1 = time.perf_counter(); "
    "sys.stdout.write(f'{(t1-t0)*1000:.3f}\\n')"
)

def test_cold_start_p95_under_100ms():
    latencies = []
    for _ in range(ITERATIONS):
        result = subprocess.run(
            [sys.executable, "-c", SCRIPT],
            check=True, capture_output=True, text=True,
        )
        latencies.append(float(result.stdout.strip()))
    latencies.sort()
    p95 = latencies[int(0.95 * ITERATIONS)]
    print(f"\ncold start latencies : {latencies}")
    print(f"p95 = {p95:.3f} ms")
    assert p95 < 100.0, f"cold start p95 {p95:.3f} ms > 100 ms"
```

### 9.3 Optimisations Φ-C-FFI pour atteindre ≤ 100 ms

* Pas d'import lourd au top-level `efe/__init__.py` (= déjà fait — le
  bundle ne se charge pas au boot).
* `try: import efe_tool except` : gardé mais le coût est ~5 ms.
* Compression LTO sur la wheel : reduit chargement disque (~10 ms).
* `strip = true` dans `[tool.maturin]` : reduit taille mappable.

---

## § 10. Surface 6 : Stub `.pyi` ↔ Rust API sync

### 10.1 Risque

Le fichier `python/efe/_native.pyi` est **hand-maintained**. Si un dev
ajoute une `#[pyfunction]` ou un `#[pyclass]` sans mettre à jour le
stub, les utilisateurs mypy ont une signature périmée — pas une erreur
de build, juste de la mauvaise DX.

### 10.2 Pin test

```python
# scripts/tests/test_efe_ffi_structure.py

import ast, re, importlib
from pathlib import Path

PYI = Path("crates/efe-ffi/python/efe/_native.pyi")

# Liste des symboles attendus dans le .pyi (= fonctions + classes
# publiques de efe._native, dérivé manuellement de lib.rs#L78-L92).
EXPECTED = {
    "functions": {"version", "load_bundle", "classify_zero_copy", "aclassify_zero_copy"},
    "classes": {
        "Verdict", "Bundle", "EpistemicLatentSpace",
        "EfeNativeClient", "SwarmController",
        "SwarmSpec", "SwarmSpecBuilder", "AlignmentCone",
    },
    "exceptions": {
        "EfeError", "BundleError", "CatalogError", "SwarmError",
        "AlignmentBreach", "NumericalError", "ResourceExhausted",
        "ConfigError", "TimeoutError", "FfiError",
    },
}

def parse_pyi():
    tree = ast.parse(PYI.read_text())
    funcs, classes = set(), set()
    for node in ast.iter_child_nodes(tree):
        if isinstance(node, ast.FunctionDef):
            funcs.add(node.name)
        elif isinstance(node, ast.ClassDef):
            classes.add(node.name)
    return funcs, classes

def test_stub_pyi_in_sync():
    funcs, classes = parse_pyi()
    missing_funcs = EXPECTED["functions"] - funcs
    missing_classes = (EXPECTED["classes"] | EXPECTED["exceptions"]) - classes
    assert not missing_funcs, f"missing from .pyi : {missing_funcs}"
    assert not missing_classes, f"missing from .pyi : {missing_classes}"
```

### 10.3 Manifest de symboles canoniques

Un fichier `crates/efe-ffi/src/_canonical_symbols.txt` liste **la
source de vérité** ; les deux côtés (Rust `lib.rs#fn efe_native` et
le `.pyi`) doivent matcher cette liste. Le test § 10.2 vérifie le côté
`.pyi` ; un grep statique sur `lib.rs` vérifie le côté Rust.

---

## § 11. Surface 7 : ABI3 verification

### 11.1 Mission

SLO `SLO-FFI-ABI3` : la wheel produite **n'utilise aucun symbole
Python-version-specific** (= elle est strictement ABI3). Sinon un
utilisateur Python 3.13 qui télécharge une wheel ABI3-py310 verrait
`undefined symbol: _Py_NotImplementedStruct` au runtime.

### 11.2 Scanner

```python
# scripts/bench/verify_abi3.py

import argparse, os, re, subprocess, sys, zipfile
from pathlib import Path

# Tout symbole CPython ABI3 commence par `Py_` ou `PyLong_` etc.
# La liste **interdite** = tout symbole CPython qui n'est PAS dans la
# section ABI3 limited API (PEP 384). Pour vérifier, on parcourt
# les `.so` / `.pyd` à l'intérieur du `.whl` et on vérifie qu'aucun
# symbole importé n'est dans la blacklist.

BLACKLIST_PATTERNS = [
    # Symboles version-specific connus :
    r"^_Py_NewRef$",           # Python 3.10+
    r"^_PyObject_FastCall$",   # internals
    r"^_PyEval_GetFrame$",     # internals
]

def extract_symbols(so_path: Path) -> set[str]:
    if sys.platform == "linux":
        out = subprocess.check_output(["nm", "-D", "--undefined-only", str(so_path)]).decode()
        return {m.group(1) for m in re.finditer(r"^\s*U\s+(\S+)", out, re.MULTILINE)}
    elif sys.platform == "darwin":
        out = subprocess.check_output(["nm", "-u", str(so_path)]).decode()
        return {m.group(1) for m in re.finditer(r"^\s*(\S+)", out, re.MULTILINE)}
    else:  # Windows
        out = subprocess.check_output(["dumpbin", "/imports", str(so_path)]).decode()
        return set(re.findall(r"\s+([A-Za-z_]\w+)$", out, re.MULTILINE))

def scan_wheel(whl: Path) -> list[str]:
    bad = []
    with zipfile.ZipFile(whl) as z:
        for name in z.namelist():
            if not name.endswith((".so", ".pyd", ".dylib")):
                continue
            # Extract to /tmp and scan
            with z.open(name) as f:
                tmp = Path(f"/tmp/{Path(name).name}")
                tmp.write_bytes(f.read())
                syms = extract_symbols(tmp)
                for s in syms:
                    for pat in BLACKLIST_PATTERNS:
                        if re.match(pat, s):
                            bad.append(f"{whl.name}:{name}:{s}")
    return bad

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("path", nargs="+", type=Path)
    args = ap.parse_args()
    all_bad = []
    for p in args.path:
        for whl in p.glob("*.whl") if p.is_dir() else [p]:
            print(f"scanning {whl} ...")
            all_bad += scan_wheel(whl)
    if all_bad:
        print("\nBLACKLISTED SYMBOLS DETECTED:")
        for b in all_bad:
            print(f"  {b}")
        sys.exit(1)
    print("ABI3 strict : PASS")

if __name__ == "__main__":
    main()
```

### 11.3 Quand l'enforce

* Tag-gate `ffi-publish.yml` (= post-build, pre-publish).
* PR-gate optional (= seulement si `crates/efe-ffi/Cargo.toml` change).

---

## § 12. Surface 8 : Observabilité in-process (8 instruments)

### 12.1 Instruments canoniques

| # | Instrument | Type | Labels | Mesure |
|---|---|---|---|---|
| 1 | `efe_ffi_classify_total` | Counter | `outcome` (ok/err) | Calls totaux |
| 2 | `efe_ffi_classify_seconds` | Histogram | `endpoint` (sync/async) | Latence end-to-end |
| 3 | `efe_ffi_gil_held_seconds` | Histogram | `op` | GIL acquis |
| 4 | `efe_ffi_gil_released_seconds` | Histogram | `op` | GIL relâché |
| 5 | `efe_ffi_marshal_in_seconds` | Histogram | `kind` (str/bytes/array) | Conversion Python→Rust |
| 6 | `efe_ffi_marshal_out_seconds` | Histogram | `kind` (verdict/els) | Conversion Rust→Python |
| 7 | `efe_ffi_bundle_cache_hits_total` | Counter | `outcome` (hit/miss/evict) | LRU bundle cache |
| 8 | `efe_ffi_wheel_load_seconds` | Histogram | — | Cold import (mesuré 1× au boot) |

### 12.2 Fichier `crates/efe-telemetry/src/metrics/ffi_instruments.rs`

```rust
//! FFI observability — 8 instruments Prometheus.
//! Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 12.

use std::sync::OnceLock;
use opentelemetry::metrics::{Counter, Histogram, Meter};
use opentelemetry::{global, KeyValue};

static INSTRUMENTS: OnceLock<FfiInstruments> = OnceLock::new();

pub struct FfiInstruments {
    classify_total: Counter<u64>,
    classify_seconds: Histogram<f64>,
    gil_held_seconds: Histogram<f64>,
    gil_released_seconds: Histogram<f64>,
    marshal_in_seconds: Histogram<f64>,
    marshal_out_seconds: Histogram<f64>,
    bundle_cache_hits_total: Counter<u64>,
    wheel_load_seconds: Histogram<f64>,
}

pub fn instruments() -> &'static FfiInstruments {
    INSTRUMENTS.get_or_init(|| {
        let meter = global::meter("efe.ffi");
        FfiInstruments {
            classify_total: meter
                .u64_counter("efe_ffi_classify_total")
                .with_description("Total efe-ffi classify calls")
                .init(),
            classify_seconds: meter
                .f64_histogram("efe_ffi_classify_seconds")
                .with_description("End-to-end classify latency seconds")
                .init(),
            // ... 6 autres ...
            wheel_load_seconds: meter
                .f64_histogram("efe_ffi_wheel_load_seconds")
                .with_description("Wheel cold load latency (boot-time)")
                .init(),
        }
    })
}

pub fn note_classify(outcome: &'static str, seconds: f64) {
    let inst = instruments();
    inst.classify_total.add(1, &[KeyValue::new("outcome", outcome)]);
    inst.classify_seconds.record(seconds, &[KeyValue::new("endpoint", "sync")]);
}

pub fn note_gil(held: f64, released: f64, op: &'static str) {
    let inst = instruments();
    inst.gil_held_seconds.record(held, &[KeyValue::new("op", op)]);
    inst.gil_released_seconds.record(released, &[KeyValue::new("op", op)]);
}

pub fn note_marshal_in(seconds: f64, kind: &'static str) { /* ... */ }
pub fn note_marshal_out(seconds: f64, kind: &'static str) { /* ... */ }
pub fn note_cache(outcome: &'static str) { /* hit/miss/evict */ }
pub fn note_wheel_load(seconds: f64) {
    instruments().wheel_load_seconds.record(seconds, &[]);
}
```

### 12.3 Production callers (D-FFI-2 enforcement)

| Instrument | Callers production |
|---|---|
| `classify_total` | `client.rs::classify()` (success + error branches) |
| `classify_seconds` | `client.rs::classify()` (toujours record final) |
| `gil_held_seconds` | `controller.rs::execute()` wrap |
| `gil_released_seconds` | `controller.rs::execute()` (= `py.allow_threads` exit) |
| `marshal_in_seconds` | `classify.rs` après tokenize |
| `marshal_out_seconds` | `verdict.rs::from_synthetic` |
| `bundle_cache_hits_total` | `cache.rs::get()` (hit/miss) + `cache.rs::evict()` |
| `wheel_load_seconds` | `lib.rs::efe_native` au `#[pymodule]` init |

---

## § 13. Surface 9 : Defensive

### 13.1 Garde-fous

| # | Garde-fou | Implémentation |
|---|---|---|
| DF1 | **Panic safety** : aucun panic Rust ne fait SIGABRT côté Python | `#[pyo3(catch_unwind = true)]` sur toutes les `#[pyfunction]` |
| DF2 | **GIL deadlock detection** : si une opération tient le GIL > 50 ms, log un WARN | wrap `py.allow_threads` + mesure |
| DF3 | **Empty input rejection** : tout `text=""` ou `bundle_path=""` retourne `ConfigError` | déjà fait classify.rs |
| DF4 | **Thread safety** : `PyEfeNativeClient` est `Send + Sync`, vérifié via static assert | `static_assertions::assert_impl_all!` |
| DF5 | **Drop ordering** : `Bundle` est libéré APRÈS `Client` qui le référence | strong refs via `Arc` |
| DF6 | **Buffer protocol contiguity** : `PyReadonlyArray*` rejette les arrays non-contiguës | déjà fait types.rs `as_slice()?` |
| DF7 | **Numerical guard** : NaN/Inf dans embedding rejected | déjà fait `efe-core` ; FFI propage |
| DF8 | **GIL release ratio min** : SLO F-8 enforce ≥ 95 % | `test_gil_release.py` |
| DF9 | **No `unsafe`** : crate compile avec `#![forbid(unsafe_code)]` | déjà appliqué |
| DF10 | **Wheel load timeout** : `import efe` > 1 s log un WARN visible | mesure `wheel_load_seconds` + alert |

### 13.2 Implémentation de `catch_unwind`

```rust
// Dans crates/efe-ffi/src/lib.rs (extension)
//
// PyO3 ≥ 0.21 transforme automatiquement un panic Rust en
// `PanicException` Python. On le configure explicitement et on log
// le panic via tracing pour ne pas perdre la stack trace.

use std::panic;

pub fn install_panic_hook() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        tracing::error!("efe-ffi rust panic: {info}");
        default(info);
    }));
}
```

L'appel est dans `efe_native()` `#[pymodule]` init.

### 13.3 `test_gil_release.py` strict (SLO F-8)

```python
import threading, time, efe

def test_gil_released_ratio_above_95pct(bundle, catalog_path):
    """4 threads concurrents devraient multiplier le throughput
    par ~ 3.5× si le GIL est relâché ≥ 95 % du temps de classify."""

    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    N = 100

    def measure_serial():
        t0 = time.perf_counter()
        for _ in range(N):
            client.classify("text")
        return time.perf_counter() - t0

    def measure_parallel(n_threads=4):
        def worker(): 
            for _ in range(N // n_threads):
                client.classify("text")
        t0 = time.perf_counter()
        threads = [threading.Thread(target=worker) for _ in range(n_threads)]
        for t in threads: t.start()
        for t in threads: t.join()
        return time.perf_counter() - t0

    serial = measure_serial()
    parallel = measure_parallel(4)
    speedup = serial / parallel
    print(f"\nserial={serial*1000:.2f}ms ; parallel={parallel*1000:.2f}ms ; "
          f"speedup={speedup:.2f}x")
    # 4 threads avec GIL release ratio R : speedup = 1 / (1/4 + (1-R)*3/4)
    # R=0.95 → speedup ≈ 3.6 ; R=0.50 → speedup ≈ 1.45.
    assert speedup >= 3.0, (
        f"SLO-FFI-GIL-RELEASED proxy : speedup {speedup:.2f}x < 3.0"
    )
    client.close()
```

---

## § 14. Surface 10 : Bench harness (Criterion Rust + pytest-benchmark)

### 14.1 Bench Criterion Rust (`crates/efe-ffi/benches/ffi_overhead.rs`)

```rust
//! Bench Criterion : overhead pur FFI (sans GIL, sans Python).
//!
//! Mesure :
//!   * `synthesise_verdict_only` — coût du stub D25 (= base ligne)
//!   * `pyverdict_from_synthetic` — coût de la construction PyVerdict
//!   * `pyverdict_to_dict` — coût de la sérialisation Rust→PyDict
//!
//! Output Criterion → `target/criterion/` → consommé par
//! `scripts/bench/criterion_ffi_to_slo.py`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use efe_ffi::controller::synthesise_verdict_for_bench;
// ... etc ...
```

### 14.2 Bench Python (`test_async_overhead.py`)

```python
import asyncio, time, efe

def test_async_overhead_under_10pct():
    """L'async path doit overhead < 10% du sync path."""
    bundle = efe.load_bundle(...)
    
    def sync_time():
        client = efe.EfeNativeClient(bundle, "...")
        t0 = time.perf_counter()
        for _ in range(100):
            client.classify("x")
        return time.perf_counter() - t0
    
    async def async_time():
        t0 = time.perf_counter()
        for _ in range(100):
            await efe.aclassify_zero_copy("x", ...)
        return time.perf_counter() - t0

    s = sync_time()
    a = asyncio.run(async_time())
    overhead = (a - s) / s
    assert overhead < 0.10, f"async overhead {overhead*100:.1f}% > 10%"
```

### 14.3 Bridge Criterion → SLO

```python
# scripts/bench/criterion_ffi_to_slo.py
# Lit target/criterion/<bench>/estimates.json, projette en SLO JSON
# parseable par check_slo.py.
```

---

## § 15. Surface 11 : Contractuelle

### 15.1 ABI stability promise (semver)

* **MAJOR** : tout changement qui casse la surface Python publique
  (= une signature de fonction change, une classe disparaît, un
  attribut frozen devient mutable).
* **MINOR** : ajout d'une nouvelle fonction / classe / attribut
  optionnel.
* **PATCH** : fix bug numerical ou perf sans breaking change.

Le pin test § 10 enforce la stabilité côté `.pyi`.

### 15.2 Wheel attestation

Chaque wheel publiée est :

* Signée Sigstore (= certificate court-vivant émis par Fulcio).
* Indexée Rekor (= transparency log public, vérifiable).
* SBOM joint (CycloneDX 1.5).
* Hash recorded dans `bench_baselines/v0.6.0/ffi_wheel_attestations.json`.

### 15.3 Drop-in migration

`EfeClient` (gRPC) → `EfeNativeClient` (PyO3) :

```python
# AVANT (D25-pre)
from efe_tool import EfeClient
client = EfeClient.from_env()
verdict = client.classify("text", catalog="router-v1")

# APRÈS (Φ-C-FFI)
import efe
bundle = efe.load_bundle("/etc/efe/bundles/active")
client = efe.EfeNativeClient(bundle, "/etc/efe/catalogs/router-v1.json")
verdict = client.classify("text")
```

Le test `test_dropin_migration.py` (§ 16) vérifie que le verdict est
**numerically equivalent** entre les deux clients.

---

## § 16. Matrice complète des artefacts Φ-C-FFI (30 entrées)

| # | Artefact | Chemin | Type | Sprint | SLO/Gap |
|---|---|---|---|---|---|
| 01 | SLO YAML | `bench_baselines/v0.6.0/ffi_slos.yaml` | config | D26 | G1 |
| 02 | Doc archi Φ-C-FFI | `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` | doc | D26 | — |
| 03 | FFI instruments | `crates/efe-telemetry/src/metrics/ffi_instruments.rs` | code | D26 | G11 |
| 04 | Telemetry re-export | `crates/efe-telemetry/src/metrics/mod.rs` | code | D26 | G11 |
| 05 | Bundle LRU cache | `crates/efe-ffi/src/cache.rs` | code | D26 | G17 |
| 06 | Bundle cache wiring | `crates/efe-ffi/src/classify.rs` (extension) | code | D26 | G17 |
| 07 | Criterion bench | `crates/efe-ffi/benches/ffi_overhead.rs` | code | D26 | G2 |
| 08 | Cargo.toml extension | `crates/efe-ffi/Cargo.toml` | config | D26 | G2 |
| 09 | Fixtures equiv | `crates/efe-ffi/python/efe/tests/fixtures/numerical_equiv.json` | data | D27 | G3 |
| 10 | Fixtures catalog | `crates/efe-ffi/python/efe/tests/fixtures/catalog.json` | data | D27 | G3 |
| 11 | Test equiv | `crates/efe-ffi/python/efe/tests/test_numerical_equiv.py` | test | D27 | G3 |
| 12 | Test memory leak | `crates/efe-ffi/python/efe/tests/test_memory_leak.py` | test | D27 | G4 |
| 13 | Test cold start | `crates/efe-ffi/python/efe/tests/test_cold_start.py` | test | D27 | G5 |
| 14 | Test determinism strict | `crates/efe-ffi/python/efe/tests/test_determinism_strict.py` | test | D27 | G19 |
| 15 | Test dropin migration | `crates/efe-ffi/python/efe/tests/test_dropin_migration.py` | test | D27 | G18 |
| 16 | Test async overhead | `crates/efe-ffi/python/efe/tests/test_async_overhead.py` | test | D27 | G16 |
| 17 | Test gil release strict | `crates/efe-ffi/python/efe/tests/test_gil_release.py` (extension) | test | D27 | G15 |
| 18 | Verify wheel size | `scripts/bench/verify_wheel_size.py` | tool | D28 | G6 |
| 19 | Verify ABI3 | `scripts/bench/verify_abi3.py` | tool | D28 | G7 |
| 20 | Stub pin test | `scripts/tests/test_efe_ffi_structure.py` | test | D28 | G8 |
| 21 | Criterion FFI bridge | `scripts/bench/criterion_ffi_to_slo.py` | tool | D28 | G20 |
| 22 | Freeze equiv | `scripts/bench/freeze_numerical_equiv.py` | tool | D28 | G3 |
| 23 | Workflow ffi-bench | `.github/workflows/ffi-bench.yml` | ci | D28 | G14 |
| 24 | Workflow ffi-publish ext | `.github/workflows/ffi-publish.yml` (extension) | ci | D28 | G9/G10 |
| 25 | Workflow ffi-test ext | `.github/workflows/ffi-test.yml` (extension) | ci | D28 | G14 |
| 26 | Runbook GIL contention | `docs/runbook/ffi_gil_contention.md` | doc | D28 | G12 |
| 27 | Runbook Python version drift | `docs/runbook/ffi_python_version_drift.md` | doc | D28 | G12 |
| 28 | Runbook wheel load failure | `docs/runbook/ffi_wheel_load_failure.md` | doc | D28 | G12 |
| 29 | Runbook perf regression | `docs/runbook/ffi_perf_regression.md` | doc | D28 | G12 |
| 30 | Stub .pyi extension | `crates/efe-ffi/python/efe/_native.pyi` (sync) | code | D28 | G8 |

---

## § 17. Plan d'implémentation D26 → D28 chirurgical

### 17.1 D26 (jour 1) — Surface SLO + Cache + Bench Rust

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–10:30 | SLO YAML drafting | 01 | 15 entries OK |
| 10:30–12:00 | FFI instruments + re-export | 03, 04 | `cargo build` OK |
| 12:00–13:00 | Production callers wiring | 03 (cont.) | grep ≥ 1 caller hors test |
| 14:00–16:00 | LRU bundle cache | 05, 06 | tests verts |
| 16:00–17:30 | Criterion bench + Cargo.toml | 07, 08 | `cargo bench --no-run` OK |
| 17:30–18:00 | Day-1 audit + commit | — | — |

**Go/No-Go D26 EOD** : `cargo test -p efe-ffi --lib` + `cargo bench --no-run -p efe-ffi` verts.

### 17.2 D27 (jour 2) — Surface tests Python

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–10:00 | Fixtures equiv + catalog | 09, 10 | 100 fixtures stables |
| 10:00–11:30 | `test_numerical_equiv.py` | 11 | 100/100 passes local |
| 11:30–12:30 | `test_memory_leak.py` | 12 | drift < 5 % |
| 14:00–15:00 | `test_cold_start.py` | 13 | p95 < 100 ms |
| 15:00–16:00 | `test_determinism_strict.py` | 14 | 100/100 |
| 16:00–17:00 | `test_dropin_migration.py` | 15 | skipped si no gRPC |
| 17:00–18:00 | Tests async / GIL extension | 16, 17 | verts |

**Go/No-Go D27 EOD** : `pytest python/efe/tests/` 100 % vert local.

### 17.3 D28 (jour 3) — Build pipeline + CI + Runbooks

| Heure | Tâche | Artefact | DoD |
|---|---|---|---|
| 09:00–10:00 | `verify_wheel_size.py` + `verify_abi3.py` | 18, 19 | self-test passes |
| 10:00–11:00 | Stub pin test + canonical_symbols | 20, 30 | sync OK |
| 11:00–12:00 | Criterion → SLO bridge | 21 | mock JSON OK |
| 14:00–15:00 | Freeze equiv tool | 22 | sortie 100 fixtures |
| 15:00–16:00 | Workflow `ffi-bench.yml` | 23 | yamllint OK |
| 16:00–17:00 | Workflow `ffi-publish.yml` extension | 24 | signing + SBOM OK |
| 17:00–17:30 | Workflow `ffi-test.yml` extension | 25 | matrix 3.10→3.13 OK |
| 17:30–18:30 | 4 runbooks | 26–29 | sections complètes |

**Go/No-Go D28 EOD = Φ-C-FFI go-live** :
* Tous les 30 artefacts livrés.
* `pytest` + `cargo test -p efe-ffi` + `cargo bench --no-run` verts.
* Au moins 1 simulation de `ffi-bench.yml` en local OK.

---

## § 18. Risques identifiés & mitigations (20 risques)

| # | Risque | Sévérité | Probabilité | Mitigation |
|---|---|---|---|---|
| R1 | PyO3 0.21 → 0.22 breaking changes | HIGH | LOW | Pin exact `pyo3 = "=0.21"` ; bump séparé |
| R2 | abi3-py310 ne supporte pas tous les patterns futurs | MED | MED | Document blacklist d'APIs |
| R3 | Sigstore Rekor outage bloque publish | MED | LOW | `--rekor-url` fallback offline mode |
| R4 | Wheel size > 8 MiB avec LTO off | MED | MED | LTO `thin` + strip enforce |
| R5 | Memory leak vrai (pas false positive) | HIGH | LOW | tracemalloc detail + smoketest pre-merge |
| R6 | GIL release ratio < 95 % sur stub D25 | HIGH | MED | Move all CPU work inside `py.allow_threads` |
| R7 | numerical equiv brisée par bump efe-core | HIGH | MED | Pin test cassure → freeze tool re-run |
| R8 | Cross-Python 3.13 wheel load failure | HIGH | LOW | CI matrix include 3.13 ; alert tôt |
| R9 | tokio runtime double-init panic | MED | LOW | `OnceLock` guard déjà présent |
| R10 | mac M1 cross-compile failure | MED | MED | macos-14 runner ; QEMU fallback |
| R11 | manylinux 2_17 outdated (glibc trop vieux) | LOW | MED | Bumpable à 2_28 si besoin (Φ-D) |
| R12 | SBOM CycloneDX scope inclut deps build-only | MED | MED | Filtre `--prod-only` |
| R13 | Stub `.pyi` désync (post-merge sans pin test) | HIGH | MED | Pin test D-FFI-4 enforce |
| R14 | LRU cache thread-unsafe → race | HIGH | LOW | `parking_lot::Mutex` + tests stress |
| R15 | Pin verify_abi3 false positive | MED | MED | Whitelist explicite |
| R16 | gRPC EfeClient deprecated avant DT-F-DROPIN-1 | HIGH | LOW | Co-existence stratégie déjà documentée |
| R17 | `aclassify` spawn_blocking thread-pool saturation | MED | MED | Tokio config 4 workers |
| R18 | Python 3.14 (= alpha) ABI3 cassure | LOW | LOW | Test informational, pas blocking |
| R19 | Cosign keyless ID drift (= cert expired entre signs) | LOW | LOW | Re-sign ≤ 1 h |
| R20 | Bundle cache eviction OOM si > 8 GiB | MED | LOW | LRU max 4 bundles |

---

## § 19. Décisions de conception clés (18)

| # | Décision | Rationale |
|---|---|---|
| D1 | PyO3 0.21 (pas 0.20, pas 0.22) | 0.21 = ABI3-py310 mature ; 0.22 bump séparé Φ-D |
| D2 | ABI3-py310 (pas py38) | Python 3.10 = floor industrial 2026 |
| D3 | maturin 1.7+ (pas setuptools-rust) | Maturin = standard de fait wheel Rust |
| D4 | numpy 0.21 (= PyO3 sibling) | Évite mismatch ABI |
| D5 | `pyo3-async-runtimes` (pas asyncio-rust) | Maintenu, supporte tokio multi-thread |
| D6 | 2 tokio workers (pas 1, pas N=cores) | Compromis CLI vs server |
| D7 | LRU `parking_lot::Mutex` (pas tokio::sync::Mutex) | Sync needed côté Python |
| D8 | 5 wheels (pas 1 per Python version) | ABI3 = 1 wheel par OS×arch |
| D9 | Pin .pyi hand-maintained (pas auto-gen) | Auto-gen pyo3-stub-gen pas mature ; pin test compense |
| D10 | Cosign keyless (pas key pair) | Pas de gestion de clé long terme |
| D11 | SBOM CycloneDX (pas SPDX) | Standard industrie 2026 |
| D12 | manylinux 2_17 (pas 2_28) | Compatibilité CentOS 7 demandée |
| D13 | strip + LTO thin | Wheel size budget tight |
| D14 | `#![forbid(unsafe_code)]` | Doctrine de sécurité crate-wide |
| D15 | Determinism via embedding hash (pas request_id) | Pin test D-FFI-5 |
| D16 | Bundle cache LRU 4 slots (pas 1, pas illimité) | OOM bound + hit rate ≥ 80 % |
| D17 | Numerical equiv 100 fixtures (pas 10, pas 1000) | Couverture vs CI time tradeoff |
| D18 | `pytest-benchmark` (pas hyperfine) | Intégré à pytest, output JSON parseable |

---

## § 20. Anti-régression vs gRPC client

### 20.1 Critère anti-régression

`EfeNativeClient` doit être **au moins aussi correct que `EfeClient`
gRPC** sur les 4 axes :

1. **Numerical** — verdict bit-identique (SLO-FFI-EQUIVALENCE).
2. **Determinism** — `(text, seed)` fixés → même verdict à chaque
   appel (SLO-FFI-DETERMINISM).
3. **Surface** — toutes les options gRPC sont exposées côté PyO3
   (timeout, retry config, tracing context).
4. **Error mapping** — toute erreur gRPC observable côté Python doit
   avoir un équivalent PyO3.

### 20.2 Test `test_dropin_migration.py`

```python
import efe, pytest

@pytest.mark.skipif(
    not __import__("importlib").util.find_spec("efe_tool"),
    reason="efe_tool gRPC client not installed",
)
def test_native_and_grpc_produce_same_verdict(bundle, catalog_path):
    from efe_tool import EfeClient

    text = "I want to dispute the charge"
    catalog = "router-v1"

    grpc = EfeClient.from_env()
    grpc_v = grpc.classify(text, catalog=catalog, seed=42)

    native = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        native_v = native.classify(text)
    finally:
        native.close()

    assert grpc_v.label == native_v.label
    assert abs(grpc_v.confidence - native_v.confidence) < 1e-6
    assert grpc_v.swarm_id == native_v.swarm_id
```

---

## § 21. Critères d'acceptation (DoD à 100 %)

### 21.1 DoD par surface

* **SLO YAML** : 15 entries parseable par `check_slo.py` ; aucun
  champ non-reconnu silencieusement skippé.
* **Telemetry** : 8 instruments avec ≥ 1 caller production each.
* **Numerical equiv** : 100/100 passes ; CI nightly catch dérive.
* **Memory** : 10 000 classify, RSS drift ≤ 5 %.
* **Cold start** : ≤ 100 ms p95 sur 30 runs.
* **Stub sync** : pin test rouge si .pyi désync vs `_canonical_symbols.txt`.
* **ABI3** : zero blacklisted symbols sur 5 wheels.
* **Sigstore** : 5 `.sig` + `.pem` + Rekor lookup OK.
* **SBOM** : 1 CycloneDX par release, schema 1.5 valid.
* **Wheel size** : 5 ≤ 8 MiB each.
* **Wheel count** : exactement 5 + 1 sdist.
* **Determinism** : 100/100 cross-OS/arch.
* **CI matrix** : 3 OS × 4 Python = 12 jobs verts.
* **Runbooks** : 4 docs, chaque section "Symptômes / Diagnostic /
  Mitigation" complète.

### 21.2 DoD agrégé

```text
Φ-C-FFI DoD =
   (15/15 SLO parseables)
 ∧ (8/8 instruments × ≥ 1 production caller)
 ∧ (30/30 artefacts livrés)
 ∧ (CI ffi-test + ffi-bench + ffi-publish verts)
 ∧ (0 scripts référencés-mais-absents)
 ∧ (0 lint warnings clippy/ruff/mypy)
 ∧ (0 audit findings level 1 unsolved)
```

---

## § 22. Hand-off & dépendances downstream

### 22.1 Consommateurs Φ-C-FFI

| Consommateur | Sprint | Use case |
|---|---|---|
| **Python SDK extension** | D29-D30 | `efe-python-sdk` pure-Python wrapper sur `efe._native` |
| **Notebook examples** | D29 | Jupyter notebooks démo `examples/` |
| **DT-F-WIRE-1** (real controller) | D29 | `efe._native.SwarmController.execute` câblé à `efe-core` |
| **DT-F-ELS-1** (zero-copy ELS) | D29 | `numpy.ndarray` partageant le buffer Rust |
| **DT-F-DROPIN-1** (gRPC shim) | D29 | `from efe import EfeClient` → `EfeNativeClient` |
| **EFE LangChain integration** | D31+ | LangChain `EfeRetriever` utilise `EfeNativeClient` |
| **EFE Java SDK** | M3+ | Inspiration de l'archi (JNI au lieu de PyO3) |

### 22.2 Dépendances amont gelées au Φ-C-FFI freeze

* `efe-core` (= API publique stable).
* `efe-kernels` (= optional dep ; cuda + nccl features).
* `efe-telemetry` (= ré-export FfiInstruments).
* PyO3 0.21 (= pinned exact).
* maturin 1.7+ (= range).
* numpy 0.21 (= sibling de pyo3).

---

## § 23. Annexes (A → H)

### Annexe A — Mapping SLO ↔ Test ↔ Workflow

| SLO | Test | Workflow |
|---|---|---|
| LATENCY-P99 | `bench/native_vs_grpc.py::test_native_p99_under_budget` | ffi-bench.yml |
| SPEEDUP | `bench/native_vs_grpc.py::test_speedup_vs_grpc` | ffi-bench.yml |
| EQUIVALENCE | `test_numerical_equiv.py` | ffi-test.yml |
| GIL-RELEASED | `test_gil_release.py` | ffi-test.yml |
| WHEEL-SIZE | `scripts/bench/verify_wheel_size.py` | ffi-publish.yml |
| COLD-START | `test_cold_start.py` | ffi-bench.yml |
| DETERMINISM | `test_determinism_strict.py` | ffi-test.yml |
| MEMORY-DRIFT | `test_memory_leak.py` | ffi-bench.yml |
| ABI3 | `scripts/bench/verify_abi3.py` | ffi-publish.yml |
| STUB-SYNC | `scripts/tests/test_efe_ffi_structure.py` | ffi-test.yml |
| CROSS-PY | matrix Python | ffi-test.yml |
| SBOM | step CI | ffi-publish.yml |
| SIGSTORE | step CI | ffi-publish.yml |
| DROPIN | `test_dropin_migration.py` | ffi-test.yml |
| WHEEL-COUNT | step CI | ffi-publish.yml |

### Annexe B — Instruments ↔ Production callers

(cf. § 12.3.)

### Annexe C — Commandes opérationnelles

```bash
# Build local
cd crates/efe-ffi && maturin develop --release

# Run all FFI tests (Φ-C-FFI scope)
pytest crates/efe-ffi/python/efe/tests/ -v

# Run bench nightly (locally simulated)
pytest crates/efe-ffi/python/efe/tests/bench/ -m bench -v

# Criterion bench Rust-side
cargo bench -p efe-ffi

# Verify ABI3
python scripts/bench/verify_abi3.py crates/efe-ffi/dist/

# Verify wheel size
python scripts/bench/verify_wheel_size.py --max-mib 8 crates/efe-ffi/dist/

# Freeze numerical equiv
python scripts/bench/freeze_numerical_equiv.py \
    --out crates/efe-ffi/python/efe/tests/fixtures/numerical_equiv.json
```

### Annexe D — Glossaire Sigstore

| Terme | Définition |
|---|---|
| Fulcio | CA Sigstore qui émet des certificats courts (10 min) |
| Rekor | Transparency log ; chaque signature y est indexée publiquement |
| cosign | CLI Sigstore (= équivalent gpg/pgp pour artefacts modernes) |
| OIDC | Identity provider (= GitHub Actions OIDC token utilisé pour keyless) |

### Annexe E — Hash de la wheel canonique (post-freeze)

À remplir post-Φ-C-FFI release :

```
efe-0.6.0-cp310-abi3-manylinux_2_17_x86_64.whl
  SHA256: <hash>
  Size:   <bytes>
  Rekor:  <log index>
```

### Annexe F — Format SBOM CycloneDX

```json
{
  "bomFormat": "CycloneDX",
  "specVersion": "1.5",
  "version": 1,
  "metadata": {
    "timestamp": "2026-05-13T12:00:00Z",
    "component": {
      "type": "library",
      "name": "efe",
      "version": "0.6.0"
    }
  },
  "components": [
    {"type": "library", "name": "pyo3", "version": "0.21.0", "purl": "pkg:cargo/pyo3@0.21.0"},
    {"type": "library", "name": "numpy", "version": "0.21.0", "purl": "pkg:cargo/numpy@0.21.0"},
    /* ... transitive deps ... */
  ]
}
```

### Annexe G — Erreurs courantes & diagnostic

| Erreur | Cause probable | Mitigation |
|---|---|---|
| `ImportError: libpython3.10.so not found` | Wheel pas ABI3 | Re-build avec `--features pyo3/abi3-py310` |
| `undefined symbol: _Py_NotImplementedStruct` | Wheel ABI3 cassée | `verify_abi3.py` aurait dû catch ; investigate maturin version |
| `PanicException: index out of bounds` | Rust panic propagé | Stack trace via `RUST_BACKTRACE=1` |
| `EfeError: bundle missing` | bundle_path incorrect | Vérifier `/etc/efe/bundles/active/` symlink |
| `RuntimeError: tokio runtime not initialised` | `aclassify` appelé avant `init_tokio_runtime` | Fix automatique au #[pymodule] init |

### Annexe H — Liens & références

* `EFE_FFI_ARCHITECTURE.md` — archi D25 baseline.
* `EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` — pattern de référence pour cette complétion.
* `bench_baselines/v0.6.0/cuda_slos.yaml` — modèle YAML SLO.
* `eval/scripts/check_slo.py` — parser SLO.
* `PRODUCT_COMPLETION_ARCHITECTURE.md` § Φ-C — feuille de route Φ-C globale.
* PyO3 docs : <https://pyo3.rs/v0.21.0/>
* maturin docs : <https://www.maturin.rs/>
* Sigstore cosign : <https://docs.sigstore.dev/cosign/overview/>
* PEP 384 (limited API) : <https://peps.python.org/pep-0384/>
* CycloneDX : <https://cyclonedx.org/specification/overview/>

---

**Fin du document Φ-C-FFI Complétion.**

Document préparé pour exécution chirurgicale D26 → D28 par l'équipe
EFE. Toute déviation vs ce plan doit être tracée dans le journal de
sprint et auditée Level 1 + Level 2.
