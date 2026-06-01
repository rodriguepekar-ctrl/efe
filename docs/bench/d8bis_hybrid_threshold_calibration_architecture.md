# D8-bis — Architecture : automated calibration sweep for the hybrid decoder thresholds

> **Statut** : architecture détaillée, prête à implémenter.
> **Sprint origine** : D8 finalization (DT-D8.2 P3).
> **Date** : 2026-04-27.
> **Pré-requis** : avoir lu
> [`d8_hybrid_realdata_architecture.md`](d8_hybrid_realdata_architecture.md) et
> [`d7_genesis_fusion_bias_remediation_architecture.md`](d7_genesis_fusion_bias_remediation_architecture.md).
> **Outil de référence** : `eval/scripts/calibrate_mode1.py`
> (607 lignes, sweep Mode-1 fusion β/α en D7).

---

## 1. Mission

Automatiser la sélection des **4 seuils** du
[`HybridClassifier`](../../clients/python/efe_tool/integrations/hybrid_decoder.py)
sur la même méthodologie que le sweep D7 Mode-1 :

| Paramètre | Default actuel (rev D8) | Plage de sweep proposée |
|-----------|------------------------:|--------------------------|
| `raw_high_margin` | 0.55 | `{0.45, 0.50, 0.55, 0.60, 0.65}` |
| `swarm_high_confidence` | 0.85 | `{0.75, 0.80, 0.85, 0.90}` |
| `swarm_low_confidence` | 0.70 | `{0.55, 0.65, 0.70, 0.75}` (contrainte : `< swarm_high_confidence`) |
| `raw_top_gap` | 0.04 | `{0.02, 0.04, 0.06, 0.08}` |

Espace brut : 5 × 4 × 4 × 4 = **320 points**. Avec la contrainte
`swarm_low < swarm_high` : ~220 points effectifs. Sub-grid raisonnable
3 × 3 × 3 × 3 = 81 points autour du current-default tuple
(avec contrainte ~64 points effectifs).

**Question scientifique** : existe-t-il un tuple
`(rh*, sh*, sl*, gap*)` dans cet espace qui maximise
`aggregate_macro_f1` sur les 3 catalogs synthétiques sans dégrader
`min_per_catalog_macro_f1` (anti-overfit, comme les SLO `D7-MC-*`) ?

**Hypothèse nulle** : les seuils calibrés à la main
`(0.55, 0.85, 0.70, 0.04)` sont déjà sur le pareto-optimum dans
l'espace de recherche — le sweep ne livrera pas un gain > 0.005
aggregate_macro_f1.

---

## 2. Hors périmètre

- **Real-data calibration** : nécessite données partenaire pilote
  (axe 2 D8 SKIPPED). Non couvert ici.
- **Per-catalog tuning** : on cherche un tuple unique optimal sur
  l'agrégat. Une variante per-catalog viendrait après mesure
  empirique.
- **Online A/B** : suivi en DT-D8.3 (P4, gated trafic réel).
- **Rebuild Rust** : le `HybridClassifier` est en Python pur, donc
  le sweep n'a pas besoin de `cargo build` (contraste avec D7
  qui rebuild `efe-server` à chaque tuple).

---

## 3. Pré-condition : étendre `run_eval_hybrid.py` aux flags seuils

### 3.1 État actuel

`eval/scripts/run_eval_hybrid.py` ligne 140 instancie
`HybridClassifier(client=client, catalog=cat)` **sans paramètres**.
Le sweep ne peut donc pas piloter les seuils.

### 3.2 Modification minimale

Ajouter 4 flags CLI :

```python
parser.add_argument("--raw-high-margin", type=float, default=None)
parser.add_argument("--swarm-high-confidence", type=float, default=None)
parser.add_argument("--swarm-low-confidence", type=float, default=None)
parser.add_argument("--raw-top-gap", type=float, default=None)
```

Et propager :

```python
hybrid_kwargs = {}
if args.raw_high_margin is not None:
    hybrid_kwargs["raw_high_margin"] = args.raw_high_margin
# ... idem pour les 3 autres ...
hybrid = HybridClassifier(client=client, catalog=cat, **hybrid_kwargs)
```

**Fail-fast** : `HybridClassifier.__post_init__` vérifie déjà
`swarm_low < swarm_high` ; le sweep doit catch `ValueError` et
skip ce point (cf. `BindTimeoutError` pattern dans
`calibrate_mode1.py`).

### 3.3 Anti-régression

* Test Python : `scripts/tests/test_d8bis_run_eval_hybrid_thresholds.py`
  vérifie que les flags CLI sont propagés correctement (parse →
  `HybridClassifier.kwargs`).
* Test pin existant `test_d8_hybrid_eval.py` reste vert (les
  defaults non touchés).

---

## 4. Architecture du sweep — `eval/scripts/calibrate_hybrid.py`

### 4.1 Topologie modules

```
eval/scripts/calibrate_hybrid.py            (NEW, ~600 lignes — sibling de calibrate_mode1.py)
├── GridPoint            (dataclass : 4 floats)
├── GridResult           (dataclass : aggregate metrics + per-catalog)
├── SUB_GRID / FULL_GRID constants
├── evaluate_point       (no-spawn : invoque run_eval_hybrid.py directement)
├── rank_results         (sort by aggregate_macro_f1, tie-break min_per_catalog)
└── main                 (CLI : --catalog/--golden flags, --output-dir, --full-grid)
```

### 4.2 Différence vs `calibrate_mode1.py`

| Aspect | `calibrate_mode1.py` | `calibrate_hybrid.py` |
|--------|---------------------|-----------------------|
| Sweeps | β₀ × λ_β × α₀ (3 dims) | rh × sh × sl × gap (4 dims) |
| Server | spawn `efe-server --release` per tuple | **pas de spawn** : un seul server tournant, on change juste les flags `run_eval_hybrid.py` |
| Rebuild | `cargo build` éventuel | aucun (Python pur) |
| Durée full-grid (estim.) | ~80 × 90 s = 2 h | ~220 × 30 s = 1.8 h |
| Persistence winner | hardcode dans `mode1.rs::build_mode1_fusion_config` | hardcode dans `hybrid_decoder.py::DEFAULT_*` constants |
| Anti-overfit gate | `min_per_catalog_macro_f1` (D7-MC-3) | mirror : nouveau SLO `D8bis-CAL-*` |

### 4.3 GridPoint dataclass

```python
@dataclass(frozen=True)
class HybridGridPoint:
    raw_high_margin: float
    swarm_high_confidence: float
    swarm_low_confidence: float
    raw_top_gap: float

    def is_valid(self) -> bool:
        # Mirror of HybridClassifier.__post_init__ check.
        return self.swarm_low_confidence < self.swarm_high_confidence

    def as_cli_flags(self) -> list[str]:
        return [
            "--raw-high-margin", f"{self.raw_high_margin}",
            "--swarm-high-confidence", f"{self.swarm_high_confidence}",
            "--swarm-low-confidence", f"{self.swarm_low_confidence}",
            "--raw-top-gap", f"{self.raw_top_gap}",
        ]

    def label(self) -> str:
        return (
            f"rh{self.raw_high_margin:.2f}"
            f"_sh{self.swarm_high_confidence:.2f}"
            f"_sl{self.swarm_low_confidence:.2f}"
            f"_gap{self.raw_top_gap:.3f}"
        )
```

### 4.4 Sweep grids

```python
# Sub-grid (81 points raw, ~64 effective) — centré sur le default D8.
SUB_GRID_RAW_HIGH_MARGIN     = [0.50, 0.55, 0.60]
SUB_GRID_SWARM_HIGH_CONF     = [0.80, 0.85, 0.90]
SUB_GRID_SWARM_LOW_CONF      = [0.65, 0.70, 0.75]
SUB_GRID_RAW_TOP_GAP         = [0.03, 0.04, 0.05]

# Full grid (320 raw, ~220 effective) — exhaustif, annexe B.
FULL_GRID_RAW_HIGH_MARGIN    = [0.45, 0.50, 0.55, 0.60, 0.65]
FULL_GRID_SWARM_HIGH_CONF    = [0.75, 0.80, 0.85, 0.90]
FULL_GRID_SWARM_LOW_CONF     = [0.55, 0.65, 0.70, 0.75]
FULL_GRID_RAW_TOP_GAP        = [0.02, 0.04, 0.06, 0.08]

def sub_grid() -> Iterable[HybridGridPoint]:
    for rh in SUB_GRID_RAW_HIGH_MARGIN:
        for sh in SUB_GRID_SWARM_HIGH_CONF:
            for sl in SUB_GRID_SWARM_LOW_CONF:
                for gap in SUB_GRID_RAW_TOP_GAP:
                    p = HybridGridPoint(rh, sh, sl, gap)
                    if p.is_valid():
                        yield p
```

### 4.5 Eval driver — single-server pattern

Contrairement au D7 mode1 sweep, on garde **un seul `efe-server`
tournant** et on change uniquement les flags du runner Python :

```python
def evaluate_point_no_spawn(
    point: HybridGridPoint,
    catalogs: list[str],
    goldens: list[str],
    endpoint: str,
    output_path: Path,
    quiet: bool,
) -> GridResult:
    cmd: list[str] = [
        sys.executable, str(RUN_EVAL_HYBRID),
        "--multi-catalog",
        "--endpoint", endpoint,
        "--output", str(output_path),
        *point.as_cli_flags(),
    ]
    for cat, gold in zip(catalogs, goldens):
        cmd.extend(["--catalog", cat, "--golden", gold])
    res = subprocess.run(cmd, cwd=ROOT, ...)
    # rc=1 = "gates failed" → on lit quand même les metrics.
    if res.returncode not in (0, 1):
        return GridResult(point=point, error=f"crash rc={res.returncode}",
                          aggregate_macro_f1=0.0, ...)
    report = json.loads(output_path.read_text())
    return GridResult(point=point,
                      aggregate_macro_f1=report["aggregate"]["macro_f1"],
                      min_per_catalog_macro_f1=report["aggregate"]["min_per_catalog_macro_f1"],
                      aggregate_accuracy_top1=report["aggregate"]["accuracy_top1"],
                      per_catalog=report["per_catalog"])
```

### 4.6 Pré-flight : vérifier qu'un server tourne déjà

Le sweep démarre avec un check `wait_for_port` rapide (timeout 2s)
sur l'endpoint donné. Si le port n'accepte pas, **fail-fast** avec
un message clair invitant l'opérateur à démarrer un `efe-server`.
Anti-régression : le sweep ne *spawn* pas de server lui-même
(différence vs D7) — le fait de partager un server unique évite le
biais de cold-start sur les premiers tuples.

### 4.7 Ranking + tie-break

```python
def rank_key(r: GridResult) -> tuple[float, float, float]:
    return (
        r.aggregate_macro_f1,                # primary : agrégat
        r.min_per_catalog_macro_f1,          # tie-break #1 : anti-overfit
        r.aggregate_accuracy_top1,           # tie-break #2 : top1
    )
results.sort(key=rank_key, reverse=True)
winner = results[0]
```

### 4.8 Émission des artefacts

```
target/calibration-d8bis/
├── winner.json              # tuple gagnant + ses métriques
├── full.json                # liste de tous les GridResult
└── grid.csv                 # tabular pour analyse externe
```

`winner.json` schema :

```json
{
  "winner": {
    "raw_high_margin": 0.50,
    "swarm_high_confidence": 0.85,
    "swarm_low_confidence": 0.65,
    "raw_top_gap": 0.04,
    "aggregate_macro_f1": 0.872,
    "min_per_catalog_macro_f1": 0.748,
    "aggregate_accuracy_top1": 0.92,
    "per_catalog": [...]
  },
  "delta_vs_default": {
    "aggregate_macro_f1": 0.015,
    "label": "rh0.50_sh0.85_sl0.65_gap0.040 vs DEFAULT (rh0.55_sh0.85_sl0.70_gap0.040)"
  },
  "n_points_evaluated": 64,
  "n_points_skipped": 17,
  "duration_seconds": 1820,
  "host": "M3 Pro 14-core (M9 reference machine)"
}
```

---

## 5. SLOs ajoutés en D8-bis

| ID | Description | Cible | Source |
|----|-------------|-------|--------|
| **D8bis-CAL-1** | `aggregate_macro_f1` du winner ≥ default + 0.005 | OBSERVATIONNEL — pas de gate (le default reste autorisé même si le sweep trouve "marginal" mieux) | `winner.json::delta_vs_default` |
| **D8bis-CAL-2** | `min_per_catalog_macro_f1` du winner ≥ default | GATE | anti-régression : on refuse un winner qui sacrifie un catalog |
| **D8bis-CAL-3** | Durée full-grid sweep ≤ 3 h sur M3 Pro | OBSERVATIONNEL | `winner.json::duration_seconds` |

**Note** : à la différence du D7 sweep, on N'IMPOSE PAS un gain
minimum — l'hypothèse nulle (default déjà optimal) est défendable.
Le sweep est un **outil de découverte** ; si le gain est < 0.005,
on fige le default tel quel et on documente "calibration confirme
le hand-tuning".

---

## 6. Procédure de freeze post-sweep

Si le winner est ≠ default ET D8bis-CAL-2 vert :

1. Mettre à jour `clients/python/efe_tool/integrations/hybrid_decoder.py`
   ligne 102-113 (`DEFAULT_RAW_HIGH_MARGIN` etc.) avec les valeurs
   winner.
2. Re-run `cargo test` (no impact, hybrid est Python pur, mais
   les tests Python pinnés doivent rester verts).
3. Mettre à jour les tests `scripts/tests/test_d8_hybrid_eval.py`
   si les nouvelles thresholds changent les assertions.
4. Commit + tag éventuel `v0.1.13.x` (pas un sprint majeur, juste un
   patch de calibration).

Si le winner == default (hypothèse nulle confirmée) :

1. Émettre `target/calibration-d8bis/null_hypothesis_confirmed.txt`
   contenant le ranking complet pour audit.
2. **Aucun changement de code**. Mettre à jour le commentaire
   ligne 102-113 pour mentionner que la calibration empirique a été
   réalisée le `<date>` et a confirmé les valeurs hand-tuned.
3. Marquer DT-D8.2 comme **résolue par mesure** dans la closure
   doc.

---

## 7. Anti-régression D8-bis

| Vérification | Outil |
|--------------|-------|
| `run_eval_hybrid.py` accepte les 4 nouveaux flags | `test_d8bis_run_eval_hybrid_thresholds.py` |
| `HybridClassifier` honore les kwargs | tests unitaires existants |
| `calibrate_hybrid.py` produit `winner.json` valide | nouveau `test_d8bis_calibrate_hybrid_smoke.py` |
| Aucune régression D8 hand-tuned | `pytest scripts/tests/test_d8_*.py` |

---

## 8. Plan wallclock

| Phase | Durée estimée | Output |
|-------|--------------:|--------|
| **J1** : extension `run_eval_hybrid.py` (4 flags + tests) | 0.5 j | PR 1 mergeable indépendamment |
| **J2** : `calibrate_hybrid.py` (~600 lignes, dataclasses, sub-grid) | 1.0 j | PR 2 — squelette + tests smoke |
| **J3** : exécution full-grid sur M3 Pro + analyse | 0.5 j | `target/calibration-d8bis/winner.json` |
| **J4** : décision freeze/null + closure DT-D8.2 | 0.25 j | closure doc rev2 |
| **Total** | **2.25 j wallclock** | DT-D8.2 RÉSOLUE par mesure ou par freeze |

---

## 9. Risques

- **R-D8bis-1 (LOW)** : le full-grid (220 points × 30s = 1.8 h) peut
  drift si le binaire `efe-server` cold-start introduit un overhead.
  Mitigation : warmup explicite avant le sweep (3 requêtes sur le
  premier catalog, jeter les résultats).
- **R-D8bis-2 (LOW)** : la grille proposée est centrée sur les
  defaults. Si l'optimum est *en dehors* (e.g. `raw_high_margin =
  0.40` ou `0.70`), le sweep sub-grid le manquera. Mitigation :
  toujours lancer en mode `--full-grid` pour la décision, garder le
  sub-grid pour les itérations rapides en dev.
- **R-D8bis-3 (LOW)** : le `HybridClassifier` peut ajouter des
  paramètres en D9-D14. Le sweep doit être structuré pour qu'ajouter
  une 5e dimension reste un changement local.

---

## 10. Hand-off

Cette architecture est **prête à implémenter** sans blocages
externes. Tous les composants existent déjà :

* `HybridClassifier` accepte des kwargs : ✅ (D8.5)
* `run_eval_hybrid.py --multi-catalog` produit `aggregate.macro_f1` :
  ✅ (D8.1)
* Pattern reference `calibrate_mode1.py` : ✅ (D7)
* Golden datasets 100 cas, 3 catalogs : ✅ (D7-D8)
* Job CI artifact `eval-hybrid-multi-bench.json` : ✅ (D8.6)

Reste : 2.25 j de codage Python + 1 sweep run.

> Cette architecture est référencée par `docs/audits/d8_sprint_closure.md`
> § 5.1 DT-D8.2.
