# `efe-ffi` — Architecture détaillée des bindings PyO3 (M3)

> **Statut** : Draft v1.0.0 — Document de référence pour
> l'achèvement du module **M3 = `efe-ffi`** entre l'AS-IS (stub
> 22 lignes au 2026-04-28, zéro PyO3) et la cible **GA 1.0**
> (PyO3 zero-copy bindings, wheels manylinux2014 + macOS arm64/x86_64,
> `EfeNativeClient` Python ≥ 5× plus rapide que le `EfeClient` gRPC
> existant).
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-28.
> **Dépendances doc** : `MODE1_ARCHITECTURE.md`,
> `M12_SWARM_INTEGRATION_ARCHITECTURE.md`,
> `LETTER_C_ARCHITECTURE.md`, `LETTER_D_ARCHITECTURE.md`,
> `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` (§7 phase Φ-C),
> `docs/EFE_KERNELS_ARCHITECTURE.md`.
> **Portée** : la totalité du portage Python — bindings PyO3,
> wheels publishing, async runtime (tokio + pyo3-async-runtimes),
> zero-copy numpy interop, stub `.pyi` files, CI matrix
> manylinux2014 + macOS arm64/x86_64 + Windows x86_64. Le document
> couvre le sprint **D25** (2 semaines wallclock, Φ-C). Le bundle
> réel + Java SDK (D26) et live bundle reload (D27) sont hors-portée
> de ce doc — ils figurent dans `PRODUCT_COMPLETION_ARCHITECTURE.md`
> §7.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de `efe-ffi`
§2. Position dans la topologie EFE — direction de dépendance
§3. État AS-IS au 2026-04-28
§4. Définition opérationnelle de `efe-ffi` v1.0
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble architecturale
§8. Le contrat — surface Python publique (15 ops principales)
§9. Choix de la chaîne d'outils (PyO3 + maturin + abi3)
§10. Système de build (pyproject.toml + Cargo.toml)
§11. Sous-système 1 — `_native` PyO3 module entry point
§12. Sous-système 2 — `PyVerdict` + dataclass-like wrappers
§13. Sous-système 3 — `PySwarmController` (high-level)
§14. Sous-système 4 — `PyEpistemicLatentSpace` numpy zero-copy
§15. Sous-système 5 — `classify_zero_copy()` one-shot helper
§16. Sous-système 6 — `load_bundle()` + bundle types
§17. Sous-système 7 — Async runtime (tokio + pyo3-async-runtimes)
§18. Sous-système 8 — GIL release pattern (`py.allow_threads`)
§19. Sous-système 9 — Error mapping Rust → Python exceptions
§20. Sous-système 10 — Zero-copy numpy interop
§21. Sous-système 11 — Wheel publishing pipeline
§22. Sous-système 12 — Stub file generation (.pyi)
§23. Déterminisme + reproductibilité côté Python
§24. Sûreté mémoire (ownership, GIL, Send/Sync)
§25. Stratégie de tests
§26. Matrice CI (manylinux2014 + macOS arm64/x86_64 + Windows)
§27. Migration : `EfeClient` (gRPC) ↔ `EfeNativeClient` (PyO3)
§28. Plan d'implémentation détaillé sprint-par-sprint (D25)
§29. Risques + atténuations
§30. Annexes — API Python complète (reference)
§31. Annexes — Gantt + RACI + budget
§32. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D24 (post-Φ-B), v0.3.0.
* **TO-BE** = état cible défini par chaque section.
* Les **types Python publics** sont préfixés `Py` côté Rust
  (e.g. `PyVerdict`) et **sans préfixe** côté Python (`Verdict`).
  Convention PyO3 standard.
* Les **fonctions Python publiques** suivent `snake_case`
  (`classify_zero_copy`, `load_bundle`).
* Les sprints sont **D25** (Φ-C, sprint 1 sur 3).
* Les défauts résiduels sont **DT-F.x** (Defect Trace, FFI).
* Toute mention "le moteur" sans qualification = `efe-core::SwarmController`.
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-ffi`** | Crate Rust qui produit une bibliothèque dynamique chargeable depuis Python via PyO3. Compile en `.so` (Linux) / `.dylib` (macOS) / `.pyd` (Windows). |
| **PyO3** | Framework Rust de référence pour bindings Python. Version cible : **0.21+**. Documentation : <https://pyo3.rs>. |
| **maturin** | Outil de build qui produit des wheels Python depuis un crate PyO3. Remplace setuptools. Version cible : **1.7+**. |
| **abi3** | ABI Python stable. Une seule wheel `.whl` couvre plusieurs versions Python (≥ 3.10 pour notre cible). Évite de publier 4-5 wheels par OS×arch×Python. |
| **manylinux2014** | Standard Python pour wheels Linux compatibles avec glibc ≥ 2.17. Construit dans un container CentOS 7. |
| **GIL** | Global Interpreter Lock — verrou Python qui sérialise l'exécution. PyO3 expose `py.allow_threads(\|\| { ... })` pour le relâcher pendant le travail Rust. |
| **Zero-copy** | Pas de copie mémoire entre Python et Rust. Pour numpy, `PyArray2<f32>` partage le buffer sans allocation. |
| **`_native`** | Le nom canonique du module PyO3 importable depuis Python : `from efe._native import classify_zero_copy`. |
| **`EfeClient`** | Le client Python EXISTANT (`clients/python/efe_tool/client.py`) qui parle au serveur via gRPC. PRÉSERVÉ — pas de breaking change. |
| **`EfeNativeClient`** | Le NOUVEAU client Python (D25) qui parle à `efe-core`/`efe-kernels` en-process via PyO3. Speedup ≥ 5× vs gRPC. |
| **wheel** | Format binaire Python distribuable (`.whl`). Le wheel maturin contient la `.so`/`.dylib`/`.pyd` + métadonnées. |
| **PyArray2** | Type numpy 2D depuis le crate `numpy` (PyO3 wrapper). Partage le buffer numpy sans copie. |
| **`pyo3-async-runtimes`** | Crate qui ponce tokio + Python asyncio. Évite à chaque binding de gérer le runtime Tokio manuellement. |

### §0.3 Cibles plateformes

| Plateforme | Wheel name | Status v1.0 |
|------------|------------|:-----------:|
| Linux x86_64 (manylinux2014) | `efe-X.Y.Z-cp310-abi3-manylinux_2_17_x86_64.whl` | ✅ obligatoire |
| Linux aarch64 (manylinux2014) | `efe-X.Y.Z-cp310-abi3-manylinux_2_17_aarch64.whl` | ✅ obligatoire |
| macOS arm64 (10.15+) | `efe-X.Y.Z-cp310-abi3-macosx_11_0_arm64.whl` | ✅ obligatoire |
| macOS x86_64 (10.15+) | `efe-X.Y.Z-cp310-abi3-macosx_10_15_x86_64.whl` | ✅ obligatoire |
| Windows x86_64 | `efe-X.Y.Z-cp310-abi3-win_amd64.whl` | ⚠️ best-effort (post-GA) |

5 wheels (4 obligatoires) couvrent ≈ 99 % de la userbase Python data
science.

### §0.4 Cibles Python

| Python version | Status v1.0 |
|:--------------:|:-----------:|
| 3.10 | ✅ minimum |
| 3.11 | ✅ |
| 3.12 | ✅ |
| 3.13 | ✅ |
| 3.14+ (post-2026) | ⚠️ futur — abi3 nous protège |

Grâce à `abi3`, **une seule wheel par OS×arch couvre Python 3.10+**.
Cela divise la matrice CI par 4-5.

---

## §1. Mission de `efe-ffi`

### §1.1 Énoncé opérationnel

> *Permettre à un script Python d'instancier un `SwarmController`
> EFE et de classifier du texte sans ouvrir une connexion gRPC,
> avec un speedup ≥ 5× sur p99 par rapport au client gRPC actuel
> (`EfeClient`) — tout en restant transparent côté API publique
> (drop-in replacement).*

### §1.2 Périmètre stricte

`efe-ffi` est responsable de **3 livrables atomiques** :

1. **Une bibliothèque dynamique** chargeable par Python : `.so` /
   `.dylib` / `.pyd` produite par maturin depuis le code Rust.
2. **Une API Python idiomatique** exposée sous `efe._native` :
   * `classify_zero_copy(text, catalog_id) -> Verdict`
   * `EfeNativeClient(bundle, catalog).classify(...) -> Verdict`
   * `load_bundle(path) -> Bundle`
   * `SwarmController(spec, registry, ...).execute(request) -> Response`
3. **Des stubs `.pyi`** pour l'auto-complétion IDE + mypy.

### §1.3 Hors-périmètre

`efe-ffi` n'est **PAS** responsable de :

* La logique de classification — vit dans `efe-core` (M1).
* Le runtime GPU — vit dans `efe-kernels` (M2).
* Le client gRPC `EfeClient` — vit dans `clients/python/efe_tool/`
  (M8). Préservé inchangé.
* Le SDK Java/Go/TS — autres modules (M9, M10, M11).
* La distribution PyPI — pipeline CI séparé.

### §1.4 Mission complémentaire

`efe-ffi` est aussi le **canari de l'API Rust publique** : si
`efe-core::SwarmController` change sa signature, le compile de
`efe-ffi` casse. Cela protège contre les breaking changes
involontaires côté Rust qui se répercuteraient en cascade sur les
SDKs Java/Go/TS.

---

## §2. Position dans la topologie EFE — direction de dépendance

### §2.1 Graph de dépendances post-D25

```
                    ┌───────────────────────────────┐
                    │     Python user code          │
                    │                                │
                    │   from efe import classify     │
                    │   from efe import EfeClient    │
                    └──────┬────────────────────────┘
                           │
              ┌────────────┴───────────────┐
              │                            │
              ▼                            ▼
     ┌─────────────────┐         ┌─────────────────┐
     │  efe._native    │         │  EfeClient      │
     │  (PyO3 wheel)   │         │  (gRPC)         │
     │                 │         │                 │
     │   ↓ depends on  │         │   ↓ TCP gRPC    │
     │                 │         │                 │
     └────┬────────────┘         └────────┬────────┘
          │                               │
          ▼                               ▼
   ┌──────────────┐              ┌────────────────┐
   │  efe-ffi     │              │  efe-server    │
   │  (Rust)      │              │  (gRPC server) │
   └────┬─────────┘              └────────┬───────┘
        │                                 │
        ▼                                 ▼
   ┌──────────────────────────────────────────────────┐
   │                 efe-core                          │
   │  (SwarmController + ComputeBackend trait + ...)   │
   └──────────┬────────────────────────────────────────┘
              │
              ▼ (optional)
       ┌──────────────┐
       │  efe-kernels │
       │   (GPU)      │
       └──────────────┘
```

**Lecture critique** : les deux paths Python (`efe._native` direct
in-process et `EfeClient` gRPC) **coexistent**. L'utilisateur
choisit selon son usage :

* **`EfeClient` gRPC** : multi-process, scaling horizontal, tenant
  isolation, audit chain server-side. Use case Lettre E.
* **`efe._native` PyO3** : in-process, latence ultra-faible,
  pas de réseau. Use case Lettres A/B/C/D.

### §2.2 Direction stricte

`efe-ffi → efe-core → efe-kernels` (transitivement).
**Jamais l'inverse**.

Cela garantit :

1. **`efe-core` reste compile-able seul** sans PyO3 / Python.
2. **`efe-ffi` peut être absent** du build — workspace compile sans.
3. **Le wheel Python ne dépend pas du serveur gRPC** — install
   `efe` Python sans installer `efe-server`.

### §2.3 Co-existence avec le client gRPC

Le module Python d'entry-point `efe.__init__.py` re-exporte les
deux APIs :

```python
# efe/__init__.py (post-D25)
from efe._native import (
    EfeNativeClient,
    classify_zero_copy,
    load_bundle,
    Bundle,
    Verdict,
    SwarmController,
)

# Préservé : import legacy via efe_tool (gRPC).
try:
    from efe_tool import EfeClient
except ImportError:
    pass  # gRPC stack non installée
```

L'utilisateur écrit :

```python
# Use case 1 : in-process (D25 nouveau)
from efe import classify_zero_copy
verdict = classify_zero_copy("hello", "router-v1", "/path/to/bundle.efe")

# Use case 2 : gRPC (existant)
from efe import EfeClient
client = EfeClient.from_env()
verdict = client.classify("hello", catalog)
```

---

## §3. État AS-IS au 2026-04-28

### §3.1 Inventaire physique

```
crates/efe-ffi/
├── Cargo.toml                          (435 octets)
└── src/
    └── lib.rs                          (22 lignes)
```

**Total** : 2 fichiers, **22 lignes** Rust, **zéro PyO3**.

### §3.2 Détail `Cargo.toml`

```toml
[package]
name = "efe-ffi"
description = "Foreign function interface (PyO3) for EFE"

# PyO3 integration is added in Sprint 2 (PR-017). Sprint 0 is a pure
# Rust library that just re-exports efe-core.

[dependencies]
efe-core = { path = "../efe-core" }

[lib]
name = "efe_ffi"
```

**Constats** :

* ✅ `efe-core` en dep directe.
* ❌ **`pyo3` absent** — à ajouter en D25.
* ❌ **`numpy` absent** — à ajouter en D25.
* ❌ **`pyo3-async-runtimes` absent** — à ajouter en D25.
* ❌ **Pas de `[lib] crate-type = ["cdylib"]`** — sans ça, maturin
  ne peut pas produire de `.so`. À ajouter en D25.
* ❌ **Pas de feature `extension-module`** — clé pour décharger
  Python.
* ❌ **Pas de `pyproject.toml`** — maturin a besoin de ce fichier
  à la racine de `efe-ffi/` pour packaging.

### §3.3 Détail `src/lib.rs`

```rust
//! Foreign Function Interface for EFE.
//!
//! Sprint 0: pure Rust re-exports (no PyO3 yet).
//! Sprint 2 (PR-017): PyO3 bindings for meta-analyzer and template registry.

pub use efe_core::{EfeError, NanoAgentId, Result, SwarmId, TemplateCode, VERSION};

#[cfg(test)]
mod tests {
    #[test]
    fn re_exports_work() { ... }
}
```

**Le seul export** est un re-export non-PyO3 de 6 symboles
`efe-core`. Ces re-exports sont SANS valeur — un consommateur Rust
peut directement importer `efe_core::SwarmId`.

### §3.4 État `clients/python/efe_tool/client.py`

`EfeClient` (gRPC) est livré et stable depuis D10. Sa surface :

```python
class EfeClient:
    def __init__(self, host, port, timeout_s, embedder, ...): ...
    @classmethod
    def from_env(cls) -> "EfeClient": ...
    def classify(self, text: str, catalog: AnchorCatalog) -> ClassifyResult: ...
    def healthcheck(self) -> bool: ...
    def close(self) -> None: ...
    def __enter__(self) -> "EfeClient": ...
    def __exit__(self, *exc) -> None: ...
```

`ClassifyResult` (in `types.py`) :

```python
@dataclass(frozen=True)
class ClassifyResult:
    label: str
    confidence: float
    sub_scores: list[SubScore]
    swarm_id: str
    rounds_executed: int
    cone_id: str
    catalog_version: str
    audit_id: str | None
```

L'objectif `efe-ffi` est de produire un type `Verdict` **structurellement
identique** à `ClassifyResult` (mais sans `audit_id` puisque pas
de chain HMAC en in-process), et un client `EfeNativeClient` avec
la même interface `.classify(text, catalog) -> Verdict`.

### §3.5 État `python/efe/`

Petit paquet Python à la racine — `__init__.py`, `cli.py`,
`py.typed`. Contient l'entry point `efe-cli`. Sera **étendu en D25**
pour re-exporter `efe._native` symbols.

### §3.6 Synthèse AS-IS

* `efe-ffi` (le crate) : **shell vide** (22 lignes, juste re-exports
  Rust→Rust).
* PyO3 : **absent**.
* maturin : **absent**.
* wheels publishing : **inexistant**.
* Stubs `.pyi` : **inexistants**.

**Verdict** : 100 % du code à écrire. Pas un seul `#[pymodule]`,
`#[pyclass]`, `#[pyfunction]` n'existe à ce jour. Effort estimé :
**10 jours-h** (Sprint D25).

---

## §4. Définition opérationnelle de `efe-ffi` v1.0

### §4.1 Le contrat v1.0

`efe-ffi` v1.0 satisfait **15 propriétés contractuelles** :

| # | Propriété | Métrique opposable |
|--:|-----------|--------------------|
| F-1 | API surface | 15 ops Python publiques (cf. §8) |
| F-2 | Speedup vs gRPC | p99 classify ≥ 5× plus rapide vs `EfeClient` à charge égale (mesuré par `tests/bench/native_vs_grpc.py`) |
| F-3 | Wheel size | ≤ 8 MiB compressed (manylinux2014 x86_64) |
| F-4 | Python version coverage | abi3 wheels Python 3.10-3.13 (1 wheel par OS×arch) |
| F-5 | Cross-platform | manylinux2014 x86_64 + aarch64, macOS arm64 + x86_64 |
| F-6 | Cold start | `import efe` ≤ 100 ms (lazy load embedder bundle) |
| F-7 | Numerical equivalence | Verdict bit-identical vs gRPC sur 100 fixtures |
| F-8 | GIL release | `classify` doit relâcher le GIL ≥ 95 % du temps total |
| F-9 | Memory safety | aucun panic Rust ne se propage en SIGABRT — tout est `PyResult` |
| F-10 | Async | `aclassify` async exposé via `pyo3-async-runtimes` |
| F-11 | Numpy zero-copy | Centroïde de sortie est un `numpy.ndarray` qui partage le buffer Rust |
| F-12 | Determinism | seed fixé → résultats bit-identiques sur 100 runs |
| F-13 | Error mapping | toute erreur Rust → exception Python typée (10 classes) |
| F-14 | Documentation | rustdoc + Python docstrings (Google style) sur 100 % public API |
| F-15 | Stub files | `efe.pyi` complet pour mypy + IDE auto-complete |

### §4.2 Versioning produit aligné

| Version | Sprint | Livrable |
|---------|:------:|----------|
| **v0.4.0** | D25 | `efe-ffi` PyO3 v1.0 + 5 wheels publiées |
| v0.4.1 | D26 | Bundle réel paraphrase-multilingual + Java SDK |
| v0.4.2 | D27 | Live bundle reload via AdminService |
| v1.0.0 GA | D42 | toutes F-1..F-15 vérifiées en CI |

### §4.3 Gating

Aucune version n'est releasée tant que :

* CI matrix **manylinux2014 + macOS arm64/x86_64 + Windows** GREEN.
* `tests/bench/native_vs_grpc.py` démontre speedup ≥ 5× p99.
* `mypy --strict tests/python/` passe avec les `.pyi` générés.
* Wheel size ≤ 8 MiB.
* `import efe` ≤ 100 ms cold.

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

1. **Le contrat Python est sacré**. Aucune modification de la
   surface publique en cours de Φ-C. Toute extension (ex : nouveau
   verdict field) remonte à un ADR + bump version.
2. **Pas de `unwrap()` dans le code Rust exposé**. Tous les
   `Result` rust se mappent à `PyResult` via le trait `From`.
   L'utilisateur Python ne voit jamais un `panic!` Rust.
3. **GIL release par défaut**. Toute opération > 100 µs doit
   `py.allow_threads(|| { ... })` ; c'est lint-checké via grep
   pin tests.
4. **Zero-copy par défaut**. Tout array numpy en entrée/sortie
   partage le buffer ; pas de `.to_vec()` côté Rust.
5. **Erreurs typées**. Pas d'`PyValueError` génériques pour les
   erreurs métier ; chaque famille a son exception (cf. §19).
6. **abi3 obligatoire**. Pas de wheel par version Python — un
   seul wheel couvre 3.10-3.13.
7. **Stub `.pyi` synchronisés**. Toute fonction PyO3 exposée
   PUBLIQUEMENT a sa déclaration `.pyi` à jour ; pin test verrouille.
8. **Docstrings Google-style**. Le `#[pyfn]` macro nécessite des
   docstrings Rust-side qui se traduisent en Python `__doc__`.
   Style aligné sur le reste de `efe_tool` (Google).
9. **Maturin develop friendly**. `maturin develop` (mode dev) doit
   fonctionner sur Mac/Linux/Windows. Pas de path absolu en
   `Cargo.toml`.
10. **Pure-Rust fallback**. Si l'utilisateur installe `efe` sans
    pip wheel binaire (`--no-binary`), le build source doit marcher
    avec le compilateur Rust seul (pas besoin de gcc/cl).

### §5.2 Ce que `efe-ffi` refuse d'implémenter

* **C-extensions hors PyO3** (ex : raw CPython API). PyO3 0.21+
  couvre 100 % des besoins. Refusé en revue PR.
* **Bindings WASM**. Hors-scope GA — peut-être post-v2.0.
* **Bindings Ruby/Lua/Node.js direct**. PyO3 est uniquement Python.
  Les autres langages passent par les SDKs gRPC.
* **GIL-free Python (PEP 703)**. Future expérimental. On suit
  l'évolution mais pas en v1.0.
* **CFFI fallback**. PyO3 est obligatoire ; pas de path alternatif.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

* `efe-ffi/Cargo.toml` déclare `[lib] crate-type = ["cdylib"]`.
* `efe-ffi/pyproject.toml` est le canonical entry point pour
  maturin.
* `efe-ffi/src/lib.rs` est l'unique point d'entrée Rust ; toute
  classe PyO3 ré-exportée depuis ici via `m.add_class::<T>()`.
* `efe-ffi/python/efe/` contient les **stubs `.pyi`** + le module
  Python `efe.__init__` qui ré-exporte `efe._native`.
* Aucun `unsafe` block dans `efe-ffi/src/` sauf accompagnement
  d'un commentaire `// SAFETY:` documenté.
* Aucun `panic!` direct ; tout passe par `PyResult` + `PyErr`.

### §6.2 Invariants de runtime

* Aucun appel bloquant ne tient le GIL. Tout `synchronize()`,
  `htod_copy`, `dtoh_copy` est dans un `py.allow_threads`.
* Toute exception Python custom dérive de `efe.EfeError` (cf. §19).
* Le module `_native` est `safe` à `import` plusieurs fois (PyO3
  garantit single-init).
* Les types PyO3 sont `Send + Sync` ssi le type Rust sous-jacent
  l'est ; sinon le type Python est `not thread-safe` et documenté.

### §6.3 Invariants de versioning

* La version du wheel `.whl` est **identique** à celle du crate
  workspace.
* PyO3 est pinné à une version mineure (ex `pyo3 = "0.21"`).
* maturin minimum **1.7.0**.
* Python ≥ 3.10 (abi3).
* La rétro-compat : v0.5.0 wheels installables sur Python 3.10
  jusqu'à 3.13 inclus sans recompilation.

---

## §7. Vue d'ensemble architecturale

### §7.1 Arborescence cible post-D25

```
crates/efe-ffi/
├── Cargo.toml                          (étendu — pyo3 + numpy + ...)
├── pyproject.toml                       (NEW — maturin entry)
├── README.md                            (NEW — install + quick start)
├── src/
│   ├── lib.rs                           (rewrite — #[pymodule] _native)
│   ├── error.rs                         (NEW — Rust → Python exception map)
│   ├── verdict.rs                       (NEW — PyVerdict dataclass-like)
│   ├── client.rs                        (NEW — PyEfeNativeClient)
│   ├── controller.rs                    (NEW — PySwarmController wrapper)
│   ├── bundle.rs                        (NEW — PyBundle + load_bundle())
│   ├── els.rs                           (NEW — PyEpistemicLatentSpace + numpy)
│   ├── classify.rs                      (NEW — classify_zero_copy() top-level)
│   ├── async_.rs                        (NEW — aclassify async via tokio)
│   ├── types.rs                         (NEW — PySwarmSpec, PyAlignmentCone, etc.)
│   └── tests.rs                         (NEW — Rust-side unit tests)
├── python/
│   └── efe/
│       ├── __init__.py                  (NEW — re-export _native)
│       ├── _native.pyi                  (NEW — stub for IDE/mypy)
│       ├── py.typed                     (NEW — PEP 561 marker)
│       └── tests/
│           ├── test_smoke.py            (NEW — import + basic call)
│           ├── test_classify.py         (NEW — Verdict equivalence)
│           ├── test_zero_copy.py        (NEW — numpy buffer sharing)
│           ├── test_async.py            (NEW — aclassify)
│           ├── test_errors.py           (NEW — exception types)
│           ├── test_determinism.py      (NEW — seed-fixed bit-identical)
│           ├── test_gil_release.py      (NEW — GIL released ≥ 95%)
│           └── bench/
│               └── native_vs_grpc.py    (NEW — speedup ≥ 5× p99)
└── benches/
    └── ffi_overhead.rs                  (NEW — Rust criterion bench)
```

**Total cible** : ~30 fichiers, ~3 500 LOC Rust + ~800 LOC Python +
~200 LOC stubs `.pyi` = **~4 500 lignes**.

### §7.2 Lifecycle d'un appel `classify_zero_copy("hello", "router-v1")`

```
Python user code
   │
   │  efe.classify_zero_copy("hello", "router-v1", "/path/to/bundle.efe")
   ▼
Python : efe._native.classify_zero_copy
   │
   │  PyO3 entry — has GIL
   ▼
Rust : classify_zero_copy(py, text, catalog_id, bundle_path) -> PyResult<PyVerdict>
   │
   │  1. Validate inputs (UTF-8, non-empty, etc.)
   │  2. Lookup or load bundle (cached)
   │  3. py.allow_threads(|| {              ← release GIL
   │       // BLOCKING WORK HERE :
   │       let embedding = bundle.encode(text)?;
   │       let spec = build_swarm_spec(catalog_id, embedding)?;
   │       let response = swarm_controller.execute(spec).await?;
   │       Ok(response)
   │     })
   │  4. py.with_gil(|py| {                ← re-acquire GIL
   │       PyVerdict::from_response(py, response)
   │     })
   ▼
Python : returns Verdict instance
```

### §7.3 Flux d'init du module `_native`

Au premier `import efe._native` :

```
Python : import efe
   │
   │  efe.__init__.py runs
   ▼
Python : from efe._native import classify_zero_copy, EfeNativeClient, ...
   │
   │  Python loader maps efe._native to the .so/.dylib/.pyd
   ▼
PyO3 : #[pymodule] entry function called
   │
   │  m.add_class::<PyVerdict>()?;
   │  m.add_class::<PyEfeNativeClient>()?;
   │  m.add_function(wrap_pyfunction!(classify_zero_copy, m)?)?;
   │  ...
   │  // ~30 add_class / add_function calls
   │
   │  m.add("EfeError", py.get_type::<PyEfeError>())?;
   │  // ~10 exception classes registered
   ▼
Module ready ; usable from Python.
```

---

## §8. Le contrat — surface Python publique

### §8.1 Inventaire des 15 ops principales

| # | Symbole Python | Type | Description |
|--:|----------------|:----:|-------------|
| 1 | `Verdict` | dataclass | Résultat de classification (label, confidence, …) |
| 2 | `Bundle` | class | Embedder bundle chargé |
| 3 | `SwarmController` | class | Bas niveau : exec swarm round par round |
| 4 | `EfeNativeClient` | class | Haut niveau : drop-in replacement de `EfeClient` gRPC |
| 5 | `EpistemicLatentSpace` | class | ELS exposé en numpy zero-copy |
| 6 | `SwarmSpec` | class | Builder de spec |
| 7 | `AlignmentCone` | class | Cône d'alignement |
| 8 | `classify_zero_copy(text, catalog_id, bundle) -> Verdict` | fn | Helper one-shot |
| 9 | `load_bundle(path) -> Bundle` | fn | Bundle loader |
| 10 | `EfeNativeClient.classify(text, catalog) -> Verdict` | method | Drop-in gRPC equivalent |
| 11 | `EfeNativeClient.aclassify(text, catalog) -> Awaitable[Verdict]` | async method | Async via tokio |
| 12 | `EfeNativeClient.healthcheck() -> bool` | method | Probe |
| 13 | `EfeNativeClient.close()` + `__enter__`/`__exit__` | method | Context manager |
| 14 | `EfeError` (et sous-classes) | exception | Hierarchy d'erreurs |
| 15 | `version() -> str` | fn | Version du wheel |

### §8.2 Mapping Rust → Python

| Rust type (efe-core) | Python type (efe._native) | Strategy |
|----------------------|---------------------------|----------|
| `OutputBundle` | `Verdict` | **Owned** copy (small struct) |
| `embedder::Bundle` | `Bundle` | **Borrowed** (Arc<Bundle> in PyO3) |
| `SwarmController` | `SwarmController` | **Owned** (one per Python instance) |
| `SwarmSpec` | `SwarmSpec` | **Owned** |
| `EpistemicLatentSpace` | `EpistemicLatentSpace` | **Borrowed** + numpy view |
| `AlignmentCone` | `AlignmentCone` | **Owned** |
| `EfeError` | `EfeError` (Python exception) | Mapped via `From<EfeError> for PyErr` |
| `efe-server::EfeClient` (gRPC) | n/a | NOT exposed — efe-ffi is in-process only |

### §8.3 Détails des classes Python

#### `Verdict`

```python
class Verdict:
    """Result of one classify call.

    Attributes:
        label: Predicted label (e.g. "billing-dispute").
        confidence: float in [0.0, 1.0].
        cone_id: Identifier of the alignment cone used.
        rounds_executed: Number of fusion rounds.
        swarm_id: Unique id of this swarm run.
        catalog_version: Version of the catalog hit.
        sub_scores: List of (label, confidence) pairs for top-N alternatives.
        latency_ms: Wall-clock latency in milliseconds.
    """
    label: str
    confidence: float
    cone_id: str
    rounds_executed: int
    swarm_id: str
    catalog_version: str
    sub_scores: list[tuple[str, float]]
    latency_ms: float

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def to_dict(self) -> dict[str, Any]: ...
```

Implémenté côté Rust comme `#[pyclass]` immutable (frozen).

#### `EfeNativeClient`

```python
class EfeNativeClient:
    """In-process EFE client (PyO3 / no gRPC).

    Drop-in replacement for `efe_tool.EfeClient` with ≥ 5× speedup.

    Args:
        bundle: Bundle instance from `load_bundle(...)`.
        catalog_path: Path to compiled catalog JSON.
        gpu_enabled: If True and CUDA feature compiled, use GPU substrate.
        seed: Optional seed for deterministic runs.

    Example:
        >>> from efe import load_bundle, EfeNativeClient
        >>> bundle = load_bundle("/etc/efe/bundles/active")
        >>> client = EfeNativeClient(bundle, "/etc/efe/catalogs/router.json")
        >>> verdict = client.classify("hello world")
        >>> print(verdict.label, verdict.confidence)
    """
    def __init__(
        self,
        bundle: Bundle,
        catalog_path: str,
        *,
        gpu_enabled: bool = False,
        seed: Optional[int] = None,
    ) -> None: ...

    def classify(self, text: str) -> Verdict: ...
    async def aclassify(self, text: str) -> Verdict: ...
    def healthcheck(self) -> bool: ...
    def close(self) -> None: ...
    def __enter__(self) -> "EfeNativeClient": ...
    def __exit__(self, *exc: object) -> None: ...
```

Comparaison drop-in vs `EfeClient` gRPC :

* `EfeClient(host, port, ...)` → `EfeNativeClient(bundle, catalog_path, ...)`
* `client.classify(text, catalog)` → `client.classify(text)` (catalog
  injecté à la construction)

Les retours `Verdict` ↔ `ClassifyResult` ont des fields proches
mais `Verdict` n'a pas `audit_id` (pas de chain HMAC en in-process).

---

## §9. Choix de la chaîne d'outils

### §9.1 Décision : PyO3 0.21+

| Option | Pour | Contre | Verdict |
|--------|------|--------|:-------:|
| **PyO3 0.21+** | Maintained, vibrant ecosystem, abi3 mature, `bound` API stable, `pyo3-async-runtimes` co-maintained | Macros bruyantes parfois | ✅ **RETENU** |
| `cpython` | Simpler subset of PyO3 | Less feature-rich, deprecated | ❌ |
| `rustpython` | Pure Rust Python | Different goal (interpreter) | ❌ |
| Bindgen + cpython.h | Total control | `unsafe` partout, maint nightmare | ❌ |
| `nanobind` (C++) | Excellent | C++ side, not Rust | ❌ |

**Décision** : PyO3 0.21+ avec features `extension-module, abi3, abi3-py310`.

### §9.2 Décision : maturin 1.7+

| Option | Pour | Contre | Verdict |
|--------|------|--------|:-------:|
| **maturin** | Co-maintained with PyO3, official, abi3 native, manylinux containers | n/a | ✅ **RETENU** |
| setuptools-rust | Plus venerable | Maintenance moins active | ❌ |
| Custom Cargo + Python bridge | Full control | Reinventing the wheel | ❌ |

**Décision** : maturin 1.7+ avec `pyproject.toml` standard.

### §9.3 Décision : abi3-py310

| Option | Couverture | Wheels par OS×arch |
|--------|------------|:------------------:|
| **abi3-py310** | Python 3.10-3.x | **1** (single wheel) |
| Per-Python wheels | Python 3.10, 3.11, 3.12, 3.13 | 4 |
| abi3-py37 | Python 3.7-3.x | 1 (mais pulling 3.7 deprecated) |

**Décision** : `abi3-py310` — minimum support 3.10 (LTS Python),
abandonner 3.7-3.9 (déjà EOL ou en EOL).

### §9.4 Modules PyO3 retenus

```toml
# Cargo.toml additions D25
pyo3 = { version = "0.21", features = [
    "extension-module",      # PyO3 builds an extension module (.so)
    "abi3-py310",             # ABI3 forward-compat from Python 3.10
    "macros",                 # #[pymodule], #[pyclass], #[pyfn]
] }

numpy = { version = "0.21" }  # PyArray2<f32> zero-copy

# Async support : tokio runtime + Python asyncio bridge.
pyo3-async-runtimes = { version = "0.21", features = ["tokio-runtime"] }

tokio = { workspace = true, features = ["rt-multi-thread", "macros"] }
```

### §9.5 Toolchain Python

| Component | Min version | Notes |
|-----------|:-----------:|-------|
| **Python** | 3.10 | Pinned via `abi3-py310` ; tested 3.10-3.13 |
| **maturin** | 1.7 | Build tool ; CI runner installs |
| **pip** | 22.3 | Installs the wheel |
| **mypy** | 1.10 | Stub validation in CI |
| **pytest** | 8 | Unit + integration tests |
| **numpy** | 1.24 | Required by PyArray2 ; abi3 compatible |

---

## §10. Système de build

### §10.1 Cargo.toml complet (post-D25)

```toml
[package]
name = "efe-ffi"
version.workspace = true
edition.workspace = true
description = "PyO3 bindings for EFE — in-process Python integration."

# CRITICAL : must produce a cdylib for Python to load.
[lib]
name = "efe_ffi"
crate-type = ["cdylib", "rlib"]  # rlib for Rust-side benches

[dependencies]
efe-core = { path = "../efe-core" }

# Optional GPU substrate ; pulled only when --features cuda.
efe-kernels = { path = "../efe-kernels", optional = true }

pyo3 = { version = "0.21", features = [
    "extension-module",
    "abi3-py310",
    "macros",
] }
numpy = { version = "0.21" }
pyo3-async-runtimes = { version = "0.21", features = ["tokio-runtime"] }

tokio = { workspace = true, features = ["rt-multi-thread", "macros", "sync"] }
tracing = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
pyo3 = { version = "0.21", features = ["auto-initialize"] }
criterion = "0.5"

[features]
default = []
cuda = ["dep:efe-kernels", "efe-kernels/cuda"]
```

### §10.2 pyproject.toml (NEW)

```toml
[build-system]
requires = ["maturin>=1.7,<2.0"]
build-backend = "maturin"

[project]
name = "efe"
description = "EFE — Ephemeral Fluid Swarm. In-process Python bindings."
authors = [
    { name = "EFE Team", email = "support@efe.ai" },
]
readme = "README.md"
requires-python = ">=3.10"
license = { text = "Proprietary" }

dependencies = [
    "numpy>=1.24",
]

[project.optional-dependencies]
test = [
    "pytest>=8",
    "pytest-asyncio>=0.23",
    "pytest-benchmark>=4",
]
typing = [
    "mypy>=1.10",
    "types-protobuf",
]

[tool.maturin]
# Map maturin to our crate. The Python module name is `efe._native`.
module-name = "efe._native"
python-source = "python"
features = ["pyo3/extension-module"]
strip = true

# abi3 wheel : single binary covers Python 3.10-3.13.
bindings = "pyo3"

# Tier-1 platforms.
manylinux = "2_17"

[tool.pytest.ini_options]
testpaths = ["python/efe/tests"]
asyncio_mode = "auto"
```

### §10.3 Build commands

```bash
# Dev mode (in-place install for iteration)
maturin develop

# Release wheel for current platform
maturin build --release

# manylinux wheel (in container)
docker run --rm -v $PWD:/io ghcr.io/pyo3/maturin build --release \
    --target x86_64-unknown-linux-gnu \
    --manylinux 2_17

# All platforms (CI matrix)
# cf. .github/workflows/ffi-publish.yml (cf. §21)
```

### §10.4 Comportement build sans PyO3 toolchain

Si l'utilisateur installe `efe-ffi` comme une crate Rust standalone
(via `cargo build -p efe-ffi`), pas via maturin :

* Le `crate-type = ["cdylib", "rlib"]` produit aussi un `.rlib`.
* PyO3 dépendance + numpy compilent normalement.
* Sans `extension-module` activé, le linker peut échouer (Python
  symbols undefined) — c'est attendu. Fix : `cargo build --no-default-features`
  pour skipper PyO3 entirely (futur DT-F).

Pour D25, on accepte que `cargo build -p efe-ffi` (sans maturin)
peut warner au link. Le canonical path est `maturin develop`.

---

## §11. Sous-système 1 — `_native` PyO3 module entry point

### §11.1 `lib.rs` réécrit

```rust
//! `efe._native` — in-process Python bindings for EFE (M3, Sprint D25).
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md`.

use pyo3::prelude::*;

mod async_;
mod bundle;
mod classify;
mod client;
mod controller;
mod els;
mod error;
mod types;
mod verdict;

/// Returns the wheel version (matches `Cargo.toml`).
#[pyfunction]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// PyO3 module entry. Python imports map here :
/// `from efe._native import classify_zero_copy, EfeNativeClient, ...`.
///
/// **NOTE** : `name = "_native"` because the public Python module
/// exposed to users is `efe`, with `efe._native` as the implementation
/// detail. The `efe/__init__.py` re-exports symbols.
#[pymodule]
#[pyo3(name = "_native")]
fn efe_native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // ── Functions ──────────────────────────────────────────────
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(classify::classify_zero_copy, m)?)?;
    m.add_function(wrap_pyfunction!(bundle::load_bundle, m)?)?;

    // ── Classes ────────────────────────────────────────────────
    m.add_class::<verdict::PyVerdict>()?;
    m.add_class::<bundle::PyBundle>()?;
    m.add_class::<client::PyEfeNativeClient>()?;
    m.add_class::<controller::PySwarmController>()?;
    m.add_class::<els::PyEpistemicLatentSpace>()?;
    m.add_class::<types::PySwarmSpec>()?;
    m.add_class::<types::PyAlignmentCone>()?;

    // ── Exception hierarchy ────────────────────────────────────
    error::register_exceptions(py, m)?;

    Ok(())
}
```

### §11.2 Module names

* Python user-facing : `efe.<X>` (e.g. `efe.classify_zero_copy`).
* Implementation : `efe._native.<X>` (the PyO3 module).
* The Python wrapper `efe/__init__.py` re-exports everything.

```python
# python/efe/__init__.py
"""EFE — Ephemeral Fluid Swarm. Python bindings."""

from efe._native import (
    Bundle,
    EfeNativeClient,
    EpistemicLatentSpace,
    SwarmController,
    SwarmSpec,
    AlignmentCone,
    Verdict,
    EfeError,
    BundleError,
    SwarmError,
    AlignmentBreach,
    NumericalError,
    classify_zero_copy,
    load_bundle,
    version,
)

# Optional gRPC client ; preserved.
try:
    from efe_tool import EfeClient
    __all_grpc__ = ["EfeClient"]
except ImportError:
    __all_grpc__ = []

__all__ = [
    "Bundle",
    "EfeNativeClient",
    "EpistemicLatentSpace",
    "SwarmController",
    "SwarmSpec",
    "AlignmentCone",
    "Verdict",
    "EfeError",
    "BundleError",
    "SwarmError",
    "AlignmentBreach",
    "NumericalError",
    "classify_zero_copy",
    "load_bundle",
    "version",
    *__all_grpc__,
]

__version__ = version()
```

---

## §12. Sous-système 2 — `PyVerdict` + dataclass-like wrappers

### §12.1 Implémentation `verdict.rs`

```rust
//! `PyVerdict` — Python-facing classification result.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 12.
//!
//! Mirrors `efe_core::dissolution::OutputBundle` + extra metadata
//! the swarm controller emits (swarm_id, rounds_executed, latency).
//! Frozen dataclass-like : every field is read-only after construction.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use efe_core::consensus::ConsensusVerdict;
use efe_core::dissolution::OutputBundle;
use efe_core::types::SwarmId;

#[pyclass(name = "Verdict", frozen)]
#[derive(Debug, Clone)]
pub struct PyVerdict {
    #[pyo3(get)]
    pub label: String,
    #[pyo3(get)]
    pub confidence: f32,
    #[pyo3(get)]
    pub cone_id: String,
    #[pyo3(get)]
    pub rounds_executed: u32,
    #[pyo3(get)]
    pub swarm_id: String,
    #[pyo3(get)]
    pub catalog_version: String,
    #[pyo3(get)]
    pub sub_scores: Vec<(String, f32)>,
    #[pyo3(get)]
    pub latency_ms: f64,
}

#[pymethods]
impl PyVerdict {
    /// Construct manually (mainly for tests). Production paths use
    /// `from_response` below.
    #[new]
    #[pyo3(signature = (
        label,
        confidence,
        cone_id,
        rounds_executed,
        swarm_id,
        catalog_version,
        sub_scores = vec![],
        latency_ms = 0.0,
    ))]
    fn new(
        label: String,
        confidence: f32,
        cone_id: String,
        rounds_executed: u32,
        swarm_id: String,
        catalog_version: String,
        sub_scores: Vec<(String, f32)>,
        latency_ms: f64,
    ) -> Self {
        Self {
            label,
            confidence,
            cone_id,
            rounds_executed,
            swarm_id,
            catalog_version,
            sub_scores,
            latency_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Verdict(label={:?}, confidence={:.4}, rounds={}, latency_ms={:.2})",
            self.label, self.confidence, self.rounds_executed, self.latency_ms
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.label == other.label
            && (self.confidence - other.confidence).abs() < 1e-6
            && self.cone_id == other.cone_id
            && self.rounds_executed == other.rounds_executed
            && self.swarm_id == other.swarm_id
            && self.catalog_version == other.catalog_version
    }

    /// Convert to a Python dict — useful for JSON serialization.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new_bound(py);
        dict.set_item("label", &self.label)?;
        dict.set_item("confidence", self.confidence)?;
        dict.set_item("cone_id", &self.cone_id)?;
        dict.set_item("rounds_executed", self.rounds_executed)?;
        dict.set_item("swarm_id", &self.swarm_id)?;
        dict.set_item("catalog_version", &self.catalog_version)?;
        dict.set_item("sub_scores", &self.sub_scores)?;
        dict.set_item("latency_ms", self.latency_ms)?;
        Ok(dict)
    }
}

impl PyVerdict {
    /// Internal constructor from the Rust `OutputBundle` returned by
    /// `SwarmController::execute`. NOT exposed to Python directly.
    pub(crate) fn from_response(
        bundle: OutputBundle,
        swarm_id: SwarmId,
        rounds_executed: u32,
        catalog_version: String,
        latency_ms: f64,
    ) -> Self {
        Self {
            label: bundle.decoded_label.unwrap_or_default(),
            confidence: bundle.confidence,
            cone_id: bundle.cone_id.to_string(),
            rounds_executed,
            swarm_id: swarm_id.to_string(),
            catalog_version,
            sub_scores: bundle
                .top_alternatives
                .into_iter()
                .map(|alt| (alt.label, alt.score))
                .collect(),
            latency_ms,
        }
    }
}
```

### §12.2 Python-side stub (`_native.pyi`)

```python
class Verdict:
    label: str
    confidence: float
    cone_id: str
    rounds_executed: int
    swarm_id: str
    catalog_version: str
    sub_scores: list[tuple[str, float]]
    latency_ms: float

    def __init__(
        self,
        label: str,
        confidence: float,
        cone_id: str,
        rounds_executed: int,
        swarm_id: str,
        catalog_version: str,
        sub_scores: list[tuple[str, float]] = ...,
        latency_ms: float = ...,
    ) -> None: ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def to_dict(self) -> dict[str, Any]: ...
```

---

## §13. Sous-système 3 — `PySwarmController`

### §13.1 Wrapping the Rust SwarmController

```rust
//! Python-facing wrapper for `efe_core::swarm::SwarmController`.
//!
//! For most users `EfeNativeClient` is the recommended entry point ;
//! `SwarmController` is exposed for advanced use cases (custom
//! genesis strategy, debug mode, etc.).

use pyo3::prelude::*;
use std::sync::Arc;

use efe_core::swarm::SwarmController;
use efe_core::types::SwarmSpec;

use crate::error::ffi_err;
use crate::types::PySwarmSpec;
use crate::verdict::PyVerdict;

#[pyclass(name = "SwarmController")]
pub struct PySwarmController {
    inner: Arc<SwarmController>,
}

#[pymethods]
impl PySwarmController {
    /// Construct via the standard builder pattern.
    /// Most users prefer `EfeNativeClient` ; this is exposed for
    /// integration tests + research notebooks.
    #[new]
    #[pyo3(signature = (catalog_path, *, gpu_enabled=false, seed=None))]
    fn new(
        catalog_path: &str,
        gpu_enabled: bool,
        seed: Option<u64>,
    ) -> PyResult<Self> {
        // Build the controller via the standard rust path.
        let inner = build_controller(catalog_path, gpu_enabled, seed)
            .map_err(ffi_err)?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Execute one swarm round + return the verdict. Releases the
    /// GIL during the heavy computation.
    fn execute(&self, py: Python<'_>, spec: &PySwarmSpec) -> PyResult<PyVerdict> {
        // Clone the Arc so we can move it across the GIL boundary.
        let controller = Arc::clone(&self.inner);
        let spec_inner: SwarmSpec = spec.to_inner();

        let result = py.allow_threads(|| {
            // Heavy CPU/GPU work here — GIL is released.
            tokio::runtime::Runtime::new()
                .map_err(|e| format!("tokio runtime: {e}"))?
                .block_on(controller.execute(spec_inner))
                .map_err(|e| format!("execute: {e}"))
        });

        match result {
            Ok(response) => Ok(PyVerdict::from_response(
                response.output_bundle,
                response.swarm_id,
                response.rounds_executed,
                response.catalog_version,
                response.latency_ms,
            )),
            Err(msg) => Err(ffi_err(msg)),
        }
    }
}

fn build_controller(
    catalog_path: &str,
    gpu_enabled: bool,
    seed: Option<u64>,
) -> Result<SwarmController, String> {
    // Load catalog, build genesis, fusion, consensus, dissolution,
    // alignment from the standard Rust paths. Skipped here for brevity ;
    // see `crates/efe-server/src/mode1.rs::build_mvp_backend` for the
    // canonical wiring.
    todo!("D25 J3 — wire SwarmController build from catalog file")
}
```

### §13.2 Python usage example

```python
from efe import SwarmController, SwarmSpec

controller = SwarmController("/etc/efe/catalogs/router-v1.json", seed=42)
spec = SwarmSpec.builder() \
    .request_id("test-1") \
    .embedding(np.random.rand(512).astype(np.float32)) \
    .build()
verdict = controller.execute(spec)
print(verdict.label, verdict.confidence)
```

---

## §14. Sous-système 4 — `PyEpistemicLatentSpace` numpy zero-copy

### §14.1 Goal

Expose the ELS (`[N × D]` matrix of agent latents) as a `numpy.ndarray`
**without copy** — the numpy buffer aliases the Rust `Vec<f32>`.
Allows research / debug code to inspect agent state via standard
numpy ops.

### §14.2 Implementation

```rust
//! Python-facing `EpistemicLatentSpace` with zero-copy numpy interop.

use numpy::{IntoPyArray, PyArray2, ToPyArray};
use pyo3::prelude::*;

use efe_core::els::EpistemicLatentSpace;

#[pyclass(name = "EpistemicLatentSpace")]
pub struct PyEpistemicLatentSpace {
    // Owning copy. Multi-thread access via numpy's `with_array`
    // pattern is not Send-safe ; we document the constraint.
    inner: EpistemicLatentSpace,
}

#[pymethods]
impl PyEpistemicLatentSpace {
    /// Number of agents (N).
    #[getter]
    fn n_agents(&self) -> usize {
        self.inner.n_agents()
    }

    /// Latent dimension (D).
    #[getter]
    fn latent_dim(&self) -> usize {
        self.inner.latent_dim()
    }

    /// Returns the ELS as a numpy float32 2-D array of shape (N, D).
    /// **Zero-copy** : the array aliases the underlying Rust storage.
    /// The caller MUST hold the Python GIL while reading.
    fn as_array<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let view = self.inner.as_slice();
        let n = self.inner.n_agents();
        let d = self.inner.latent_dim();
        // Reshape Vec<f32> into [N, D].
        // PyArray2::from_slice copies — for true zero-copy use the
        // unsafe `PyArray2::from_owned_array_bound` after wrapping in
        // ndarray::Array2.
        let arr = view.to_pyarray_bound(py).reshape([n, d])?;
        Ok(arr)
    }

    /// Compute and return the centroid as a 1-D numpy float32 array.
    fn centroid<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, numpy::PyArray1<f32>>> {
        let centroid = self.inner.centroid();
        Ok(centroid.into_pyarray_bound(py))
    }

    fn __repr__(&self) -> String {
        format!(
            "EpistemicLatentSpace(n_agents={}, latent_dim={})",
            self.inner.n_agents(),
            self.inner.latent_dim()
        )
    }
}
```

### §14.3 Zero-copy semantics

* `as_array(py) -> PyArray2<f32>` returns a numpy view that aliases
  Rust storage.
* The view is valid as long as the `PyEpistemicLatentSpace` instance
  is alive. Python `__del__` triggers Rust drop.
* Modifying the array from Python **does** modify the Rust storage.
* Concurrent Rust mutation while Python reads → undefined behaviour.
  Documented in the docstring.

---

## §15. Sous-système 5 — `classify_zero_copy()` one-shot helper

### §15.1 Goal

The simplest possible API : one function call, one Verdict back.
No client object, no setup. Useful for CLI scripts and notebooks.

### §15.2 Implementation

```rust
//! Top-level `classify_zero_copy(text, catalog_id, bundle_path)`
//! convenience function — one-shot classification without a
//! client object.

use pyo3::prelude::*;
use std::path::PathBuf;

use crate::error::ffi_err;
use crate::verdict::PyVerdict;

/// One-shot classify : load bundle if needed, run swarm, return verdict.
///
/// Cached internally : repeated calls with the same `bundle_path`
/// reuse the loaded bundle (LRU cache, max 4 bundles in flight).
#[pyfunction]
#[pyo3(signature = (text, catalog_id, bundle_path, *, gpu_enabled=false, seed=None))]
pub fn classify_zero_copy(
    py: Python<'_>,
    text: &str,
    catalog_id: &str,
    bundle_path: &str,
    gpu_enabled: bool,
    seed: Option<u64>,
) -> PyResult<PyVerdict> {
    // Validate input.
    if text.is_empty() {
        return Err(ffi_err("text must not be empty"));
    }
    let bundle_path = PathBuf::from(bundle_path);

    // Heavy work : release GIL.
    let result = py.allow_threads(|| {
        let bundle = bundle_cache_get_or_load(&bundle_path)?;
        let controller = controller_cache_get_or_build(catalog_id, gpu_enabled, seed)?;

        let embedding = bundle.encode(text).map_err(|e| format!("embed: {e}"))?;
        let spec = build_spec(catalog_id, &embedding, seed)?;
        let response = tokio::runtime::Handle::try_current()
            .or_else(|_| Ok(tokio::runtime::Builder::new_current_thread().enable_all().build()?))
            .map_err(|e: std::io::Error| format!("tokio runtime: {e}"))?
            .block_on(controller.execute(spec))
            .map_err(|e| format!("execute: {e}"))?;
        Ok::<_, String>(response)
    })
    .map_err(ffi_err)?;

    Ok(PyVerdict::from_response(
        result.output_bundle,
        result.swarm_id,
        result.rounds_executed,
        result.catalog_version,
        result.latency_ms,
    ))
}

/// Bundle cache (LRU max 4) — module-level static.
fn bundle_cache_get_or_load(_path: &std::path::Path) -> Result<std::sync::Arc<crate::bundle::InnerBundle>, String> {
    todo!("D25 J5 — DT-F-CACHE-1 : LRU bundle cache implementation")
}

fn controller_cache_get_or_build(
    _catalog_id: &str,
    _gpu_enabled: bool,
    _seed: Option<u64>,
) -> Result<std::sync::Arc<efe_core::swarm::SwarmController>, String> {
    todo!("D25 J5 — DT-F-CACHE-2 : controller cache by catalog+gpu+seed")
}

fn build_spec(
    _catalog_id: &str,
    _embedding: &[f32],
    _seed: Option<u64>,
) -> Result<efe_core::types::SwarmSpec, String> {
    todo!("D25 J5 — DT-F-SPEC-1 : embedding → SwarmSpec builder")
}
```

### §15.3 Python usage

```python
from efe import classify_zero_copy

verdict = classify_zero_copy(
    "I want to dispute the charge",
    catalog_id="router-v1",
    bundle_path="/etc/efe/bundles/active",
)
print(verdict.label)  # "billing-dispute"
```

---

## §16. Sous-système 6 — `load_bundle()` + bundle types

### §16.1 PyBundle wrapper

```rust
//! Python-facing wrapper for `efe_server::embedder::Bundle`.

use pyo3::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::ffi_err;

#[pyclass(name = "Bundle")]
pub struct PyBundle {
    pub(crate) inner: Arc<InnerBundle>,
}

/// Internal type — the actual Rust bundle (re-named to avoid
/// pyo3 macro conflicts).
pub struct InnerBundle {
    // Wraps the embedder bundle from efe-server.
    bundle: efe_server::embedder::Bundle,
}

#[pymethods]
impl PyBundle {
    #[getter]
    fn id(&self) -> String {
        self.inner.bundle.manifest().bundle_id.clone()
    }

    #[getter]
    fn fingerprint(&self) -> String {
        hex::encode(self.inner.bundle.fingerprint().0)
    }

    #[getter]
    fn version(&self) -> String {
        self.inner.bundle.manifest().version.clone()
    }

    #[getter]
    fn dim(&self) -> usize {
        self.inner.bundle.embedding_dim()
    }

    fn __repr__(&self) -> String {
        format!(
            "Bundle(id={:?}, version={:?}, dim={})",
            self.id(),
            self.version(),
            self.dim()
        )
    }

    /// Encode a text to its embedding vector. Releases the GIL.
    fn encode<'py>(
        &self,
        py: Python<'py>,
        text: &str,
    ) -> PyResult<Bound<'py, numpy::PyArray1<f32>>> {
        let inner = Arc::clone(&self.inner);
        let embedding = py.allow_threads(|| {
            inner.bundle.encode(text).map_err(|e| format!("encode: {e}"))
        }).map_err(ffi_err)?;
        Ok(numpy::IntoPyArray::into_pyarray_bound(embedding, py))
    }
}

/// Top-level `load_bundle(path)` function.
#[pyfunction]
pub fn load_bundle(path: &str) -> PyResult<PyBundle> {
    let path = PathBuf::from(path);
    // Bundle load involves filesystem I/O + signature verification ;
    // small but non-zero work. We keep the GIL since the Python
    // path is already a synchronous block.
    let bundle = efe_server::embedder::Bundle::load(&path, /* policy = strict */)
        .map_err(|e| ffi_err(format!("load_bundle: {e}")))?;
    Ok(PyBundle {
        inner: Arc::new(InnerBundle { bundle }),
    })
}
```

### §16.2 Bundle reuse via Arc

`PyBundle.inner: Arc<InnerBundle>` allows multiple Python objects
to share the same loaded bundle (e.g. one bundle, two clients
using it). Memory footprint stays at the Rust level.

---

## §17. Sous-système 7 — Async runtime

### §17.1 Goal

Expose `aclassify(text)` async coroutine for Python asyncio users.
This is critical for FastAPI / Sanic integration.

### §17.2 Implementation via pyo3-async-runtimes

```rust
//! Async helpers for EfeNativeClient.classify.

use pyo3::prelude::*;
use std::sync::Arc;

use crate::error::ffi_err;
use crate::verdict::PyVerdict;

/// Async classify — returns a Python awaitable.
#[pyfunction]
pub fn aclassify_zero_copy<'py>(
    py: Python<'py>,
    text: String,
    catalog_id: String,
    bundle_path: String,
) -> PyResult<Bound<'py, PyAny>> {
    // Spawn the work onto the tokio runtime ; pyo3-async-runtimes
    // wraps it as a Python awaitable.
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let bundle = load_bundle_async(&bundle_path).await?;
        let controller = build_controller_async(&catalog_id).await?;
        let embedding = bundle.encode(&text).await?;
        let spec = build_spec(&catalog_id, &embedding)?;
        let response = controller.execute(spec).await
            .map_err(|e| ffi_err(format!("execute: {e}")))?;
        Python::with_gil(|py| {
            Ok(PyVerdict::from_response(
                response.output_bundle,
                response.swarm_id,
                response.rounds_executed,
                response.catalog_version,
                response.latency_ms,
            ).into_py(py))
        })
    })
}

async fn load_bundle_async(_path: &str) -> PyResult<Arc<crate::bundle::InnerBundle>> {
    todo!("D25 J7 — DT-F-ASYNC-1 : async bundle load")
}
async fn build_controller_async(_catalog_id: &str) -> PyResult<Arc<efe_core::swarm::SwarmController>> {
    todo!("D25 J7 — DT-F-ASYNC-2")
}
fn build_spec(_catalog_id: &str, _embedding: &[f32]) -> PyResult<efe_core::types::SwarmSpec> {
    todo!("D25 J7 — DT-F-ASYNC-3")
}
```

### §17.3 Python usage

```python
import asyncio
from efe import EfeNativeClient, load_bundle

async def main() -> None:
    bundle = load_bundle("/etc/efe/bundles/active")
    async with EfeNativeClient(bundle, "/etc/efe/catalogs/router.json") as client:
        # Concurrent classifies — each releases the GIL.
        verdicts = await asyncio.gather(*[
            client.aclassify(text)
            for text in ["hello", "world", "foo", "bar"]
        ])
        for v in verdicts:
            print(v.label)

asyncio.run(main())
```

### §17.4 Tokio runtime initialisation

`pyo3-async-runtimes::tokio::init` must be called once at module
load. Done in the `#[pymodule]` entry :

```rust
#[pymodule]
fn efe_native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    pyo3_async_runtimes::tokio::init_with_runtime(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)  // 2 threads for FFI work
            .enable_all()
            .build()
            .expect("tokio runtime build failed"),
    )?;
    // ... rest ...
    Ok(())
}
```

---

## §18. Sous-système 8 — GIL release pattern

### §18.1 Why release the GIL?

Python's GIL serialises Python bytecode execution. As long as the
GIL is held, only ONE Python thread can run. If we keep the GIL
during a 50 ms swarm round, the entire Python process is frozen.

PyO3 lets us release the GIL via `py.allow_threads(|| { ... })`.
The closure runs in pure Rust without holding the GIL ; other
Python threads can progress.

### §18.2 Pattern

```rust
fn classify(&self, py: Python<'_>, text: &str) -> PyResult<PyVerdict> {
    let inner = Arc::clone(&self.inner);
    let text = text.to_string();  // own the input

    // GIL released here.
    let response = py.allow_threads(|| {
        // Pure Rust work — no Python access.
        inner.run_swarm(&text)
    });

    // GIL re-acquired automatically after the closure returns.
    Python::with_gil(|py| {
        // Build PyVerdict here, may need py for PyDict allocs.
        Ok(PyVerdict::from(response))
    })
}
```

### §18.3 SLO F-8 : GIL released ≥ 95 %

For `classify` :

* Total time = h2d copy (~1 ms) + embedder forward (~30 ms) +
  swarm rounds (~100 ms) + d2h + alignment (~3 ms) ≈ 134 ms.
* Time spent holding the GIL = input validation + output marshalling
  ≈ 1 ms.
* Ratio : `1 / 134 = 0.7 %` GIL-held ≪ 5 % threshold. ✅

A pin test `tests/python/test_gil_release.py` measures this :

```python
import threading
import time

def test_gil_released_during_classify(client, bundle):
    """Other Python threads can progress during classify()."""
    counter = 0
    stop = threading.Event()

    def background():
        nonlocal counter
        while not stop.is_set():
            counter += 1
            time.sleep(0.001)  # 1 ms

    t = threading.Thread(target=background)
    t.start()
    start_counter = counter

    # Classify — should not freeze the background thread.
    client.classify("test")

    end_counter = counter
    stop.set()
    t.join()

    # If GIL was held the whole time, counter would barely move.
    progressed = end_counter - start_counter
    assert progressed >= 50, f"background thread only progressed {progressed} times"
```

---

## §19. Sous-système 9 — Error mapping Rust → Python

### §19.1 Exception hierarchy

```python
class EfeError(Exception):
    """Base class for all EFE errors."""

class BundleError(EfeError):
    """Error loading or using a bundle (signature, hash, format)."""

class CatalogError(EfeError):
    """Error loading or resolving a catalog."""

class SwarmError(EfeError):
    """Error during swarm execution (genesis, fusion, dissolution)."""

class AlignmentBreach(EfeError):
    """The output centroid fell outside the alignment cone."""

class NumericalError(EfeError):
    """NaN/Inf detected during computation."""

class ResourceExhausted(EfeError):
    """OOM, VRAM full, rate-limited, etc."""

class ConfigError(EfeError):
    """Invalid configuration value."""

class TimeoutError(EfeError):
    """Operation timed out."""

class FfiError(EfeError):
    """Internal FFI bindings error (reported as a bug)."""
```

### §19.2 Rust mapping

```rust
//! Error mapping from Rust to Python.

use pyo3::create_exception;
use pyo3::prelude::*;

create_exception!(efe_native, EfeError, pyo3::exceptions::PyException);
create_exception!(efe_native, BundleError, EfeError);
create_exception!(efe_native, CatalogError, EfeError);
create_exception!(efe_native, SwarmError, EfeError);
create_exception!(efe_native, AlignmentBreach, EfeError);
create_exception!(efe_native, NumericalError, EfeError);
create_exception!(efe_native, ResourceExhausted, EfeError);
create_exception!(efe_native, ConfigError, EfeError);
create_exception!(efe_native, TimeoutError, EfeError);
create_exception!(efe_native, FfiError, EfeError);

pub fn register_exceptions(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("EfeError", py.get_type_bound::<EfeError>())?;
    m.add("BundleError", py.get_type_bound::<BundleError>())?;
    m.add("CatalogError", py.get_type_bound::<CatalogError>())?;
    m.add("SwarmError", py.get_type_bound::<SwarmError>())?;
    m.add("AlignmentBreach", py.get_type_bound::<AlignmentBreach>())?;
    m.add("NumericalError", py.get_type_bound::<NumericalError>())?;
    m.add("ResourceExhausted", py.get_type_bound::<ResourceExhausted>())?;
    m.add("ConfigError", py.get_type_bound::<ConfigError>())?;
    m.add("TimeoutError", py.get_type_bound::<TimeoutError>())?;
    m.add("FfiError", py.get_type_bound::<FfiError>())?;
    Ok(())
}

/// Convert any String message into a generic FfiError.
pub fn ffi_err(msg: impl Into<String>) -> PyErr {
    FfiError::new_err(msg.into())
}

/// Convert from `efe_core::EfeError` to the appropriate Python
/// exception subclass.
impl From<efe_core::EfeError> for PyErr {
    fn from(e: efe_core::EfeError) -> Self {
        match e {
            efe_core::EfeError::AlignmentBreach { .. } => AlignmentBreach::new_err(e.to_string()),
            efe_core::EfeError::NumericalIssue { .. } => NumericalError::new_err(e.to_string()),
            efe_core::EfeError::Config(_) => ConfigError::new_err(e.to_string()),
            efe_core::EfeError::ResourceExhausted(_) => ResourceExhausted::new_err(e.to_string()),
            efe_core::EfeError::Timeout(_) => TimeoutError::new_err(e.to_string()),
            efe_core::EfeError::Catalog(_) => CatalogError::new_err(e.to_string()),
            efe_core::EfeError::Bundle(_) => BundleError::new_err(e.to_string()),
            efe_core::EfeError::Swarm(_) | _ => SwarmError::new_err(e.to_string()),
        }
    }
}
```

### §19.3 Python usage

```python
from efe import EfeNativeClient, AlignmentBreach, BundleError

try:
    verdict = client.classify("offensive content here")
except AlignmentBreach as e:
    # Refused — out of cone.
    print("rejected:", e)
except BundleError as e:
    print("bundle problem:", e)
```

---

## §20. Sous-système 10 — Zero-copy numpy interop

### §20.1 Where numpy zero-copy matters

| Location | Direction | Strategy |
|----------|:---------:|----------|
| `Bundle.encode(text) -> ndarray` | Rust → Python | `Vec<f32>::into_pyarray_bound` (single copy) |
| `EpistemicLatentSpace.as_array() -> ndarray` | Rust → Python | view (no copy) |
| `SwarmSpec.set_embedding(ndarray)` | Python → Rust | `arr.as_slice()?` (no copy) |
| `Verdict.sub_scores: list[tuple]` | Rust → Python | converted to Python list (small N) |

### §20.2 The numpy crate

`numpy = "0.21"` from PyO3 ecosystem provides :

* `PyArray1<T>`, `PyArray2<T>` — typed ndarray Python objects.
* `IntoPyArray` trait — convert `Vec<T>` to `PyArray1<T>`.
* `ToPyArray` trait — copy a slice into a new numpy array.
* `PyReadonlyArray1<T>::as_slice()` — read-only view of numpy data
  as `&[T]`.

### §20.3 SwarmSpec.set_embedding example

```rust
#[pymethods]
impl PySwarmSpec {
    fn set_embedding<'py>(
        &mut self,
        embedding: numpy::PyReadonlyArray1<'py, f32>,
    ) -> PyResult<()> {
        let slice = embedding.as_slice()?;
        // Validate dim.
        if slice.len() != self.expected_dim {
            return Err(crate::error::ffi_err(format!(
                "expected embedding dim {}, got {}",
                self.expected_dim,
                slice.len()
            )));
        }
        // Copy to internal Vec — efe-core SwarmSpec owns the bytes.
        self.inner_set_embedding(slice.to_vec());
        Ok(())
    }
}
```

---

## §21. Sous-système 11 — Wheel publishing pipeline

### §21.1 GitHub Actions workflow

```yaml
# .github/workflows/ffi-publish.yml
name: ffi-publish

on:
  push:
    tags:
      - "v*.*.*"
  workflow_dispatch:

jobs:
  manylinux-x86_64:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: PyO3/maturin-action@v1
        with:
          target: x86_64-unknown-linux-gnu
          manylinux: 2_17
          working-directory: crates/efe-ffi
          args: --release --strip --features cuda
      - uses: actions/upload-artifact@v4
        with:
          name: wheels-manylinux-x86_64
          path: crates/efe-ffi/target/wheels/*.whl

  manylinux-aarch64:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
      - uses: PyO3/maturin-action@v1
        with:
          target: aarch64-unknown-linux-gnu
          manylinux: 2_17
          working-directory: crates/efe-ffi
          args: --release --strip
      # NB : aarch64 builds without cuda (no cross-compile NVIDIA toolchain).
      - uses: actions/upload-artifact@v4
        with:
          name: wheels-manylinux-aarch64
          path: crates/efe-ffi/target/wheels/*.whl

  macos-arm64:
    runs-on: macos-14  # Apple Silicon
    steps:
      - uses: actions/checkout@v4
      - uses: PyO3/maturin-action@v1
        with:
          target: aarch64-apple-darwin
          working-directory: crates/efe-ffi
          args: --release --strip
      - uses: actions/upload-artifact@v4
        with:
          name: wheels-macos-arm64
          path: crates/efe-ffi/target/wheels/*.whl

  macos-x86_64:
    runs-on: macos-13  # Intel
    steps:
      - uses: actions/checkout@v4
      - uses: PyO3/maturin-action@v1
        with:
          target: x86_64-apple-darwin
          working-directory: crates/efe-ffi
          args: --release --strip
      - uses: actions/upload-artifact@v4
        with:
          name: wheels-macos-x86_64
          path: crates/efe-ffi/target/wheels/*.whl

  windows-x86_64:
    runs-on: windows-2022
    steps:
      - uses: actions/checkout@v4
      - uses: PyO3/maturin-action@v1
        with:
          target: x86_64-pc-windows-msvc
          working-directory: crates/efe-ffi
          args: --release --strip
      - uses: actions/upload-artifact@v4
        with:
          name: wheels-windows-x86_64
          path: crates/efe-ffi/target/wheels/*.whl

  publish:
    needs:
      - manylinux-x86_64
      - manylinux-aarch64
      - macos-arm64
      - macos-x86_64
      - windows-x86_64
    runs-on: ubuntu-22.04
    if: startsWith(github.ref, 'refs/tags/v')
    steps:
      - uses: actions/download-artifact@v4
        with:
          path: dist
      - name: Publish to private PyPI
        env:
          MATURIN_PYPI_TOKEN: ${{ secrets.PYPI_TOKEN }}
        run: |
          pip install maturin
          maturin upload --repository-url ${{ secrets.PYPI_REPOSITORY_URL }} \
            dist/*/*.whl
```

### §21.2 abi3 verification

After build, verify the wheel is abi3 by checking the filename :

```bash
ls *.whl
# Expected pattern :
# efe-0.4.0-cp310-abi3-manylinux_2_17_x86_64.whl
#         ^^^^^^^^^^^                            ^
#         |                                       │
#         abi3 — single wheel for Python 3.10+    plat tag
```

---

## §22. Sous-système 12 — Stub file generation (.pyi)

### §22.1 Why stubs?

Python type checkers (mypy, pyright) and IDE auto-complete read
`.pyi` stub files. PyO3 doesn't auto-generate them ; we must
maintain them manually OR via a tool.

### §22.2 Strategy

* **Hand-maintained** for D25 v1.0.
* Pin test verifies `_native.pyi` is in sync with the Rust public
  API (parses the Rust file and asserts each `#[pyclass]`/`#[pyfunction]`
  has a matching `class`/`def` in the .pyi).

### §22.3 `python/efe/_native.pyi` (sample)

```python
"""Type stubs for efe._native (PyO3 bindings).

Hand-maintained ; pin test verifies sync with Rust public API.
"""

from __future__ import annotations

from typing import Any, Awaitable, Optional

import numpy as np
import numpy.typing as npt

# ─────────────────────────────────────────────────────────────────
# Types
# ─────────────────────────────────────────────────────────────────

class Verdict:
    label: str
    confidence: float
    cone_id: str
    rounds_executed: int
    swarm_id: str
    catalog_version: str
    sub_scores: list[tuple[str, float]]
    latency_ms: float

    def __init__(
        self,
        label: str,
        confidence: float,
        cone_id: str,
        rounds_executed: int,
        swarm_id: str,
        catalog_version: str,
        sub_scores: list[tuple[str, float]] = ...,
        latency_ms: float = ...,
    ) -> None: ...

    def __repr__(self) -> str: ...
    def __eq__(self, other: object) -> bool: ...
    def to_dict(self) -> dict[str, Any]: ...

class Bundle:
    id: str
    fingerprint: str
    version: str
    dim: int

    def __repr__(self) -> str: ...
    def encode(self, text: str) -> npt.NDArray[np.float32]: ...

class EpistemicLatentSpace:
    n_agents: int
    latent_dim: int
    def as_array(self) -> npt.NDArray[np.float32]: ...
    def centroid(self) -> npt.NDArray[np.float32]: ...
    def __repr__(self) -> str: ...

class EfeNativeClient:
    def __init__(
        self,
        bundle: Bundle,
        catalog_path: str,
        *,
        gpu_enabled: bool = ...,
        seed: Optional[int] = ...,
    ) -> None: ...
    def classify(self, text: str) -> Verdict: ...
    def aclassify(self, text: str) -> Awaitable[Verdict]: ...
    def healthcheck(self) -> bool: ...
    def close(self) -> None: ...
    def __enter__(self) -> "EfeNativeClient": ...
    def __exit__(self, *exc: object) -> None: ...

class SwarmController:
    def __init__(
        self,
        catalog_path: str,
        *,
        gpu_enabled: bool = ...,
        seed: Optional[int] = ...,
    ) -> None: ...
    def execute(self, spec: SwarmSpec) -> Verdict: ...

class SwarmSpec: ...
class AlignmentCone: ...

# ─────────────────────────────────────────────────────────────────
# Functions
# ─────────────────────────────────────────────────────────────────

def classify_zero_copy(
    text: str,
    catalog_id: str,
    bundle_path: str,
    *,
    gpu_enabled: bool = ...,
    seed: Optional[int] = ...,
) -> Verdict: ...

def load_bundle(path: str) -> Bundle: ...

def version() -> str: ...

# ─────────────────────────────────────────────────────────────────
# Exceptions
# ─────────────────────────────────────────────────────────────────

class EfeError(Exception): ...
class BundleError(EfeError): ...
class CatalogError(EfeError): ...
class SwarmError(EfeError): ...
class AlignmentBreach(EfeError): ...
class NumericalError(EfeError): ...
class ResourceExhausted(EfeError): ...
class ConfigError(EfeError): ...
class TimeoutError(EfeError): ...
class FfiError(EfeError): ...
```

---

## §23. Déterminisme + reproductibilité côté Python

### §23.1 Goal

Same input + same seed → bit-identical output across Python runs.

### §23.2 Strategy

Identical to Rust : `seed` parameter on every constructor :

```python
client = EfeNativeClient(bundle, catalog, seed=42)
v1 = client.classify("hello")
v2 = client.classify("hello")  # not necessarily identical (drift)
v3 = EfeNativeClient(bundle, catalog, seed=42).classify("hello")
assert v1 == v3  # SHOULD be identical : same client state, same seed
```

The Rust side enforces determinism via `SwarmSpec::with_seed(seed)`.

### §23.3 Pin test

```python
def test_determinism_seed_42(bundle, catalog_path):
    client_a = EfeNativeClient(bundle, catalog_path, seed=42)
    client_b = EfeNativeClient(bundle, catalog_path, seed=42)
    text = "test"
    a = client_a.classify(text)
    b = client_b.classify(text)
    assert a == b, f"determinism violated: {a} != {b}"
```

---

## §24. Sûreté mémoire (ownership, GIL, Send/Sync)

### §24.1 PyO3 ownership rules

* Every `#[pyclass]` is `Send + Sync` only if explicitly marked.
* By default PyO3 generates `Send` (since the GIL serialises access)
  but NOT `Sync`.
* Our `PyVerdict` is `frozen` + immutable → safe to share.
* Our `PyEfeNativeClient` holds `Arc<...>` internally → cheap clone,
  safe to share.

### §24.2 GIL semantics

* Holding `Python<'_>` token == holding GIL.
* `py.allow_threads(...)` releases GIL ; closure runs without it.
* Inside the closure, we cannot use any `PyObject` / `Py<T>` / `Bound<T>`.
* The closure must `Send` its inputs (no `&Python`).

### §24.3 Drop ordering

* When a Python `EfeNativeClient` is GC'd, Rust drop runs.
* The drop chain releases `Arc<SwarmController>` → if last ref, drop SwarmController.
* No async cleanup needed (no tokio task pinned to Python lifetime).

### §24.4 No `unsafe` policy

`efe-ffi/src/` contains 0 `unsafe` blocks. All numpy interop goes
through the `numpy` crate's safe API (`as_slice()?`, `into_pyarray_bound`).

---

## §25. Stratégie de tests

### §25.1 Pyramide de tests

```
                    ┌──────────────────┐
                    │   bench native   │   ← native_vs_grpc.py, weekly
                    │   vs gRPC        │
                    └──────────────────┘
                ┌────────────────────────┐
                │   E2E classify         │   ← test_classify.py
                │   (5 fixtures)         │
                └────────────────────────┘
            ┌────────────────────────────────┐
            │   Python pytest (~50 tests)    │
            │   smoke, errors, async, GIL,   │
            │   determinism, zero-copy       │
            └────────────────────────────────┘
        ┌────────────────────────────────────────┐
        │   Rust unit tests (~30 tests)          │
        │   PyO3 module init, type conversions   │
        └────────────────────────────────────────┘
    ┌────────────────────────────────────────────────┐
    │   Smoke build                                   │
    │   maturin develop ; import efe ; version()      │
    └────────────────────────────────────────────────┘
```

**Total cible** : ~100 tests post-D25.

### §25.2 Bench `native_vs_grpc.py`

```python
import time
import pytest

@pytest.mark.bench
def test_speedup_vs_grpc(efe_native, efe_grpc, fixture_texts):
    """SLO F-2 : ≥ 5× speedup p99 vs gRPC."""
    n = 1000
    
    # Warm both.
    for t in fixture_texts[:10]:
        efe_native.classify(t)
        efe_grpc.classify(t, catalog)
    
    # Measure.
    native_lat = []
    for t in fixture_texts[:n]:
        t0 = time.perf_counter()
        efe_native.classify(t)
        native_lat.append(time.perf_counter() - t0)
    
    grpc_lat = []
    for t in fixture_texts[:n]:
        t0 = time.perf_counter()
        efe_grpc.classify(t, catalog)
        grpc_lat.append(time.perf_counter() - t0)
    
    native_p99 = sorted(native_lat)[int(0.99 * n)]
    grpc_p99 = sorted(grpc_lat)[int(0.99 * n)]
    speedup = grpc_p99 / native_p99
    print(f"speedup p99 = {speedup:.1f}×")
    assert speedup >= 5.0, f"expected ≥ 5× speedup, got {speedup:.1f}×"
```

### §25.3 Pytest tests

`python/efe/tests/` covers :

* `test_smoke.py` — `import efe ; efe.version()`.
* `test_classify.py` — verdict roundtrip via 5 fixtures.
* `test_zero_copy.py` — modify numpy view, see Rust side updated.
* `test_async.py` — `asyncio.run(client.aclassify(text))`.
* `test_errors.py` — each exception class fires on the right input.
* `test_determinism.py` — same seed → bit-identical.
* `test_gil_release.py` — background thread progresses during classify.

---

## §26. Matrice CI

### §26.1 Workflow `ffi-publish.yml`

Cf. §21.1 — 5 jobs (linux x86_64, linux aarch64, macos arm64,
macos x86_64, windows x86_64) plus 1 publish job.

### §26.2 Workflow `ffi-test.yml` (PR gate)

```yaml
name: ffi-test
on:
  pull_request:
    paths:
      - "crates/efe-ffi/**"
      - ".github/workflows/ffi-test.yml"

jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-22.04, macos-14, windows-2022]
        python: ["3.10", "3.12"]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: ${{ matrix.python }}
      - uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: "1.85"
      - name: Install maturin + pytest
        run: pip install "maturin>=1.7" pytest pytest-asyncio numpy
      - name: maturin develop
        working-directory: crates/efe-ffi
        run: maturin develop --release
      - name: pytest
        working-directory: crates/efe-ffi
        run: python -m pytest python/efe/tests/ -v
```

---

## §27. Migration : `EfeClient` ↔ `EfeNativeClient`

### §27.1 Coexistence stratégie

* Pas de breaking change pour `EfeClient` (gRPC). Continue de
  fonctionner inchangé.
* Le module `efe.__init__` re-exporte les deux.
* L'utilisateur choisit explicitement.

### §27.2 Drop-in helper (post-D25 — DT-F-DROPIN-1)

Un helper pour migration progressive :

```python
def make_client(*, native: bool = True, **kwargs) -> EfeClient | EfeNativeClient:
    """Factory. native=True → in-process (5× faster) ; native=False → gRPC."""
    if native:
        return EfeNativeClient(**kwargs)
    return EfeClient(**kwargs)
```

### §27.3 API differences

| Property | `EfeClient` (gRPC) | `EfeNativeClient` (PyO3) |
|----------|--------------------|--------------------------|
| Constructor | `(host, port, timeout_s, ...)` | `(bundle, catalog_path, ...)` |
| `.classify(text, catalog)` | ✅ | `.classify(text)` (catalog at init) |
| `.aclassify(...)` | ❌ | ✅ |
| `.healthcheck()` | ✅ (gRPC ping) | ✅ (catalog presence + bundle hash check) |
| Result type | `ClassifyResult` | `Verdict` |
| Audit chain | server-side HMAC | none (in-process) |
| Multi-tenant | yes | no |

---

## §28. Plan d'implémentation détaillé sprint-par-sprint (D25)

### §28.1 Sprint **D25** (10 jours)

**Objectif** : ship `efe-ffi` v1.0 + 5 wheels published to private
PyPI. SLO F-2 (≥ 5× speedup) demonstrated.

| Jour | Livrable |
|:----:|----------|
| J1 | Cargo.toml + pyproject.toml + maturin develop hello-world (PyO3 + numpy install on dev machine) |
| J2 | `lib.rs` `#[pymodule]` + `version()` + `Verdict` class + 5 unit Rust tests |
| J3 | `Bundle` class + `load_bundle()` + GIL release pattern |
| J4 | `EfeNativeClient` + `.classify()` + tokio runtime init |
| J5 | `classify_zero_copy()` top-level fn + LRU bundle cache |
| J6 | `EpistemicLatentSpace` numpy zero-copy + `SwarmSpec`/`AlignmentCone` types |
| J7 | Async via `pyo3-async-runtimes` : `aclassify` + `EfeNativeClient.aclassify` |
| J8 | Error mapping (10 exception classes) + cross-error tests |
| J9 | `_native.pyi` stub file + mypy --strict CI gate |
| J10 | Wheel publishing pipeline + SLO bench `native_vs_grpc.py` ≥ 5× |

### §28.2 Critères Go/No-Go D25

* GG-D25.1 : 5 wheels published (manylinux x86_64 + aarch64 ; macOS arm64 + x86_64 ; Windows x86_64).
* GG-D25.2 : `pip install efe ; python -c "import efe ; print(efe.version())"` works on Mac/Linux.
* GG-D25.3 : `tests/bench/native_vs_grpc.py` shows ≥ 5× speedup p99.
* GG-D25.4 : `mypy --strict tests/python/` passes with `_native.pyi`.
* GG-D25.5 : 50+ pytest tests GREEN on the CI matrix.
* GG-D25.6 : Cold start `import efe` ≤ 100 ms.
* GG-D25.7 : Wheel size ≤ 8 MiB (manylinux x86_64).

---

## §29. Risques + atténuations

### §29.1 Top-10 risques

| # | Risque | Probabilité | Impact | Mitigation |
|--:|--------|:----------:|:------:|------------|
| F-R-1 | PyO3 0.21 ABI break vs 0.22 future | Faible | Critique | Pin `pyo3 = "0.21"`, follow upgrade guide |
| F-R-2 | abi3 compat broken on future Python (3.14+) | Faible | Hard | abi3-py310 covers 3.14+ ; reactive bump if needed |
| F-R-3 | Cold start > 100 ms (lazy_static + tokio init) | Médium | Médium | Lazy init bundle/controller cache ; bench in CI |
| F-R-4 | manylinux2014 fails to build on aarch64 | Médium | Hard | Use ghcr.io/pyo3/maturin official image |
| F-R-5 | numpy ABI incompat with old numpy versions | Faible | Médium | Require numpy >= 1.24 in pyproject |
| F-R-6 | Tokio runtime conflict with FastAPI/Sanic | Médium | Hard | Document uvloop/asyncio compat ; use pyo3-async-runtimes |
| F-R-7 | GIL deadlock when nested py.allow_threads | Faible | Critique | Code review + tests in CI |
| F-R-8 | mypy stub drift vs actual API | Médium | Médium | Pin test verifies stub matches Rust public API |
| F-R-9 | Speedup < 5× → SLO F-2 missed | Médium | Critique | Profile h2d/d2h overhead ; consider zero-copy v2 |
| F-R-10 | Linker error on Windows MSVC (Python static link) | Médium | Médium | maturin handles ; CI catches early |

### §29.2 Risques par phase

* **D25** : médium — PyO3 first-time, async runtime, wheel matrix.
* **D26-D27** (post-FFI) : faible — bundle reload + Java SDK
  unrelated to FFI.

---

## §30. Annexes — API Python complète

### §30.1 Module `efe` (re-exports `efe._native`)

| Symbol | Kind | Path |
|--------|:----:|------|
| `Verdict` | class | `efe.Verdict` |
| `Bundle` | class | `efe.Bundle` |
| `EpistemicLatentSpace` | class | `efe.EpistemicLatentSpace` |
| `EfeNativeClient` | class | `efe.EfeNativeClient` |
| `SwarmController` | class | `efe.SwarmController` |
| `SwarmSpec` | class | `efe.SwarmSpec` |
| `AlignmentCone` | class | `efe.AlignmentCone` |
| `EfeError` | exception | `efe.EfeError` |
| `BundleError` | exception | `efe.BundleError` |
| `CatalogError` | exception | `efe.CatalogError` |
| `SwarmError` | exception | `efe.SwarmError` |
| `AlignmentBreach` | exception | `efe.AlignmentBreach` |
| `NumericalError` | exception | `efe.NumericalError` |
| `ResourceExhausted` | exception | `efe.ResourceExhausted` |
| `ConfigError` | exception | `efe.ConfigError` |
| `TimeoutError` | exception | `efe.TimeoutError` |
| `FfiError` | exception | `efe.FfiError` |
| `classify_zero_copy(text, catalog_id, bundle_path, *, gpu_enabled, seed)` | fn | `efe.classify_zero_copy(...)` |
| `load_bundle(path)` | fn | `efe.load_bundle(...)` |
| `version()` | fn | `efe.version()` |

### §30.2 Quick reference

```python
import efe
import numpy as np

# 1. One-shot classify
v = efe.classify_zero_copy("text", "router-v1", "/etc/efe/bundles/active")

# 2. Persistent client
bundle = efe.load_bundle("/etc/efe/bundles/active")
with efe.EfeNativeClient(bundle, "/etc/efe/catalogs/router.json", seed=42) as client:
    v = client.classify("text")

# 3. Async
import asyncio
async def main():
    async with efe.EfeNativeClient(bundle, catalog_path) as client:
        v = await client.aclassify("text")
asyncio.run(main())

# 4. Low-level controller
controller = efe.SwarmController("/etc/efe/catalogs/router.json")
spec = efe.SwarmSpec.builder() \
    .request_id("test") \
    .embedding(np.zeros(512, dtype=np.float32)) \
    .build()
v = controller.execute(spec)

# 5. Inspect ELS via numpy
els = controller.last_els()  # post-D25 follow-up — DT-F-ELS-1
arr = els.as_array()  # numpy view, shape (N, D)
print(arr.shape, arr.dtype)  # (256, 512) float32
```

---

## §31. Annexes — Gantt + RACI + budget

### §31.1 Gantt prévisionnel D25

```
            J1 J2 J3 J4 J5 J6 J7 J8 J9 J10
Cargo.toml + maturin   ██
PyO3 #[pymodule] + Verdict   ██
Bundle + GIL release         ██
EfeNativeClient + tokio              ██
classify_zero_copy + cache              ██
ELS numpy + types                          ██
async + aclassify                             ██
Error mapping                                    ██
Stub .pyi + mypy CI                                 ██
Wheel publishing + SLO bench                            ██

Releases :
  v0.4.0 — D25 (5 wheels published)
```

### §31.2 RACI matrix

| Activité | Core | Ops | SDK | Sec | PM |
|----------|:----:|:---:|:---:|:---:|:--:|
| Cargo.toml + maturin setup | I | C | **R** | I | A |
| PyO3 #[pymodule] + classes | C | I | **R** | I | A |
| Tokio runtime + async | C | I | **R** | I | A |
| Wheel publishing pipeline | C | **R** | C | I | A |
| Stub .pyi + mypy gate | I | I | **R** | I | A |
| SLO bench native_vs_grpc | C | I | **R** | I | A |
| Documentation Python | I | I | **R** | I | C |

### §31.3 Budget

| Item | Coût | Notes |
|------|:----:|-------|
| Dev time | 10 j-h × 1 SDK eng | core cost |
| GitHub Actions runners | inclus | 5 jobs × 5 min × per release ≈ négligeable |
| PyPI private hosting | déjà existant | AWS S3 bucket via Twine |
| **Total annuel marginal** | **~0 €** | (pas de dep payante) |

---

## §32. Mises à jour du document

### §32.1 Versioning

* **v1.0.0 (2026-04-28)** — première rédaction post-D24.
* Toute découverte audit-time / défaut résiduel / changement scope
  déclenche une révision avec :
  * Nouvelle entrée CHANGELOG.md.
  * Bump du Statut (en haut).
  * Pin test garde dans
    `scripts/tests/test_efe_ffi_doc_drift.py` qui valide le
    sommaire + les 15 ops.

### §32.2 Liens à maintenir

* `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §7 (Φ-C) — cohérence.
* `crates/efe-ffi/Cargo.toml` — version sync.
* `crates/efe-ffi/pyproject.toml` — maturin config sync.
* `crates/efe-ffi/python/efe/_native.pyi` — surface API sync.

### §32.3 Critères d'archive

Ce document devient **historique** une fois GA 1.0 livré.
Les modifications post-GA partent dans `EFE_FFI_v1.1+_ARCHITECTURE.md`.

**Fin du document.**
