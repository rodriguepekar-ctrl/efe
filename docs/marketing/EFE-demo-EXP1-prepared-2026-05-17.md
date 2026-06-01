# EXP 1 — Prêt à exécuter : pre vs post G.11 NDJSON

**Préparation date** : 2026-05-17 23:15 UTC
**Owner**            : @bench-infra-lead
**Durée d'exécution**: 5 min
**Pré-requis**       : aucun spécial, jq + bash, fixtures sur disque
**État vérifié**     : 5 fixtures pre-G.11 + 5 fixtures post-G.11 présents, jq 1.6 disponible

> Chaque output ci-dessous a été observé en LIVE sur le repo
> `/Users/rodrigue.pekar/Downloads/jjjj/efe` à 23:15 UTC le 2026-05-17.
> Ce n'est pas du préfabriqué.

---

## 0. Setup (10 sec)

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
clear
```

**Vérifier** que les fixtures sont bien là :

```bash
ls bench/fixtures/pre_g11_ndjson/ bench/fixtures/post_g11_ndjson/
```

**Tu dois voir** : 5 + 5 fichiers .ndjson.

---

## STEP 1.1 — Inspect pre-G.11 (un row historique cassé)

**Commande** :
```bash
head -1 bench/fixtures/pre_g11_ndjson/case_a_warn_null_tolerance.ndjson \
  | jq '{schema_version, scenario_id, verdict, tolerance_used, "tolerance.source": .tolerance.source}'
```

**Output observé live** :
```json
{
  "schema_version": 2,
  "scenario_id": "CB-09_llm_router_decision_latency",
  "verdict": "WARN",
  "tolerance_used": null,
  "tolerance.source": null
}
```

**Interprétation** :
- `schema_version: 2` → NDJSON v2 (avant le fix G.11)
- `verdict: "WARN"` → un WARN a été émis
- `tolerance_used: null` → **on ne sait pas pourquoi** le WARN est émis
- `tolerance.source: null` → pas non plus de trace dans le sub-objet

> **Ce que ça prouve** : la row historique est **opaque**.  Un audit
> ne peut pas reconstruire la sémantique du WARN.

---

## STEP 1.2 — Inspect post-G.11 (un row traçable)

**Commande** :
```bash
head -1 bench/fixtures/post_g11_ndjson/CB-01.ndjson \
  | jq '{schema_version, scenario_id, verdict, tolerance_used}'
```

**Output observé live** :
```json
{
  "schema_version": 3,
  "scenario_id": "CB-01_events_append_throughput",
  "verdict": "PASS",
  "tolerance_used": {
    "value": 5,
    "unit": "ms",
    "direction": "no_more_than",
    "source": "yaml",
    "applied_at": "2026-04-15T03:14:22Z"
  }
}
```

**Interprétation** :
- `schema_version: 3` → NDJSON v3 (après G.11)
- `verdict: "PASS"` → c'est un PASS
- `tolerance_used` n'est PLUS un scalaire null — c'est un **objet structuré** avec 5 champs :
  - `value: 5` — la valeur seuil
  - `unit: "ms"` — l'unité (membre de la closed taxonomy)
  - `direction: "no_more_than"` — la direction (≤)
  - `source: "yaml"` — origine = config statique du scenario
  - `applied_at: "2026-04-15T03:14:22Z"` — timestamp UTC

> **Ce que ça prouve** : la row post-G.11 est **traçable**.  Un audit
> peut reconstruire : "le PASS est parce que la mesure était ≤ 5 ms,
> seuil défini dans le YAML, appliqué à 03:14:22 UTC le 15 avril 2026."

---

## STEP 1.3 — Agréger toutes les rows pre-G.11 (7 rows / 5 fichiers)

**Commande** :
```bash
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  jq -r '"\(.scenario_id) verdict=\(.verdict) tolerance_used=\(.tolerance_used)"' "$f"
done
```

**Output observé live** :
```
CB-09_llm_router_decision_latency verdict=WARN tolerance_used=null
CB-09_llm_router_decision_latency verdict=INCONCLUSIVE tolerance_used=null
CB-20_kernels_cuda_throughput verdict=PASS tolerance_used=50000
CB-01_events_append_throughput verdict=PASS tolerance_used=50
CB-01_events_append_throughput verdict=PASS tolerance_used=null
CB-01_events_append_throughput verdict=PASS tolerance_used=null
CB-01_events_append_throughput verdict=PASS tolerance_used=null
```

**Interprétation** :
- **7 rows** au total dans 5 fichiers (case_e en contient 3).
- Sur les 7 : **5 ont `tolerance_used=null`** (opaque) et **2 ont un nombre** (50, 50000) mais **sans unité, direction, ou source** — donc presque aussi opaque.
- Le schéma v2 stocke `tolerance_used` comme un scalaire (nombre ou null) ; il **ne porte pas la sémantique**.

> **Insight critique** : ce n'est pas que tout est `null`.  C'est que
> même quand il y a un nombre, on ne sait pas si c'est des
> millisecondes, des bytes, ou un ratio — et on ne sait pas d'où
> vient la valeur.  G.11 a résolu ce problème en **structurant le
> champ**.

---

## STEP 1.4 — Agréger toutes les rows post-G.11 (8 rows / 5 fichiers)

**Commande** :
```bash
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '"\(.scenario_id) verdict=\(.verdict) tolerance_used.value=\(.tolerance_used.value) source=\(.tolerance_used.source)"' "$f"
done
```

**Output observé live** :
```
CB-01_events_append_throughput verdict=PASS tolerance_used.value=5 source=yaml
CB-01_events_append_throughput verdict=PASS tolerance_used.value=50 source=yaml
CB-01_events_append_throughput verdict=PASS tolerance_used.value=50000 source=yaml
CB-01_events_append_throughput verdict=PASS tolerance_used.value=null source=no_tolerance_defined
CB-09_llm_router_decision_latency verdict=WARN tolerance_used.value=200 source=yaml
CB-09_llm_router_decision_latency verdict=INCONCLUSIVE tolerance_used.value=200 source=yaml
CB-20_kernels_cuda_throughput verdict=PASS tolerance_used.value=100000 source=gpu_runner_override
CB-01_events_append_throughput verdict=PASS tolerance_used.value=50 source=yaml
```

**Interprétation** :
- **8 rows** au total dans 5 fichiers.
- Chaque row a maintenant **value + source** explicites.
- Sources observées : `yaml` (6 rows), `gpu_runner_override` (1 row),
  `no_tolerance_defined` (1 row).
- Le row `no_tolerance_defined` correspond au cas-limite où **aucun
  seuil n'est défini** — explicité par le code, pas par null implicite.

> **Insight critique** : 100 % des rows sont auditables.  Même la
> row "no_tolerance_defined" l'est : l'absence de seuil est elle-même
> documentée.

---

## STEP 1.5 — Bonus 1 : distribution des verdicts (avant / après)

**Commande** :
```bash
echo "▸ Verdicts pre-G.11 :"
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  jq -r '.verdict' "$f"
done | sort | uniq -c

echo ""
echo "▸ Verdicts post-G.11 :"
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '.verdict' "$f"
done | sort | uniq -c
```

**Output observé live** :
```
▸ Verdicts pre-G.11 :
   1 INCONCLUSIVE
   5 PASS
   1 WARN

▸ Verdicts post-G.11 :
   1 INCONCLUSIVE
   6 PASS
   1 WARN
```

**Interprétation** :
- Les distributions sont **similaires** (1 INCONCLUSIVE, ~5-6 PASS, 1 WARN).
- C'est attendu : les fixtures sont conçues pour exercer les mêmes
  branches du verdict aggregator.  La PR #27 a changé la
  **représentation** du `tolerance_used`, pas la logique du verdict.

---

## STEP 1.6 — Bonus 2 : distribution des `source` post-G.11

**Commande** :
```bash
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '.tolerance_used.source // "UNSPECIFIED"' "$f"
done | sort | uniq -c
```

**Output observé live** :
```
   1 gpu_runner_override
   1 no_tolerance_defined
   6 yaml
```

**Interprétation** :
- **6 rows** ont un seuil défini dans le YAML scenario.
- **1 row** vient d'un override GPU runner (cas spécifique aux jobs CUDA).
- **1 row** a pas de tolérance définie (cas explicite).
- La closed taxonomy `ToleranceSource` a 8 variants — les fixtures
  en exercent 3 (yaml, gpu_runner_override, no_tolerance_defined),
  le reste est exercé par les 5 D-S3 fixtures backfillées + tests
  unitaires.

---

## STEP 1.7 — Bonus 3 : taille totale par mode

**Commande** :
```bash
pre_count=0
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  pre_count=$((pre_count + $(wc -l < "$f")))
done
post_count=0
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  post_count=$((post_count + $(wc -l < "$f")))
done
echo "  pre-G.11  : $pre_count rows across $(ls bench/fixtures/pre_g11_ndjson/*.ndjson | wc -l) fichiers"
echo "  post-G.11 : $post_count rows across $(ls bench/fixtures/post_g11_ndjson/*.ndjson | wc -l) fichiers"
```

**Output observé live** :
```
  pre-G.11  : 7 rows across        5 fichiers
  post-G.11 : 8 rows across        5 fichiers
```

→ Tailles comparables (7 vs 8), les fixtures sont équilibrées.

---

## 8. Conclusion de l'EXP 1

| Aspect | pre-G.11 (v2) | post-G.11 (v3) |
|--------|---------------|----------------|
| `schema_version` | 2 | 3 |
| `tolerance_used` | scalaire (number) ou null | **objet structuré** {value, unit, direction, source, applied_at} |
| Provenance (source) | absente | explicite (closed taxonomy 8 variants) |
| Unité | implicite (devine) | explicite (closed taxonomy 8 variants) |
| Direction | implicite | explicite (closed taxonomy 4 variants) |
| Timestamp | parfois | systématique |
| Auditabilité | NULLE | TOTALE |

**Ce que EXP 1 a démontré** :
1. Les données historiques (v2) sont **opaques** : même un nombre
   sans unité n'est pas exploitable en audit.
2. Les données nouvelles (v3) sont **structurées** : chaque row
   porte sa propre justification.
3. La transformation est **constatable en 5 commandes jq** —
   pas en analysant le code, mais en regardant les données réelles.

---

## Bonus — Tout en un script (copy-paste exécutable)

```bash
#!/bin/bash
# EFE EXP 1 — pre vs post G.11 NDJSON analysis
# Run from /Users/rodrigue.pekar/Downloads/jjjj/efe

cd /Users/rodrigue.pekar/Downloads/jjjj/efe || exit 1

echo "═══════════════════════════════════════════════════════════════"
echo "  EXP 1 — Pre vs Post G.11 NDJSON"
echo "═══════════════════════════════════════════════════════════════"

echo ""
echo "── Step 1.1 — One pre-G.11 row (opaque) ──"
head -1 bench/fixtures/pre_g11_ndjson/case_a_warn_null_tolerance.ndjson \
  | jq '{schema_version, scenario_id, verdict, tolerance_used, "tolerance.source": .tolerance.source}'

echo ""
echo "── Step 1.2 — One post-G.11 row (traceable) ──"
head -1 bench/fixtures/post_g11_ndjson/CB-01.ndjson \
  | jq '{schema_version, scenario_id, verdict, tolerance_used}'

echo ""
echo "── Step 1.3 — Aggregate pre-G.11 rows ──"
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  jq -r '"\(.scenario_id) verdict=\(.verdict) tolerance_used=\(.tolerance_used)"' "$f"
done

echo ""
echo "── Step 1.4 — Aggregate post-G.11 rows ──"
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '"\(.scenario_id) verdict=\(.verdict) tolerance_used.value=\(.tolerance_used.value) source=\(.tolerance_used.source)"' "$f"
done

echo ""
echo "── Step 1.5 — Verdict distributions ──"
echo "▸ pre-G.11 :"
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  jq -r '.verdict' "$f"
done | sort | uniq -c

echo "▸ post-G.11 :"
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '.verdict' "$f"
done | sort | uniq -c

echo ""
echo "── Step 1.6 — Source taxonomy distribution (post-G.11) ──"
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  jq -r '.tolerance_used.source // "UNSPECIFIED"' "$f"
done | sort | uniq -c

echo ""
echo "── Step 1.7 — Row counts ──"
pre_count=0
for f in bench/fixtures/pre_g11_ndjson/*.ndjson; do
  pre_count=$((pre_count + $(wc -l < "$f")))
done
post_count=0
for f in bench/fixtures/post_g11_ndjson/*.ndjson; do
  post_count=$((post_count + $(wc -l < "$f")))
done
echo "  pre-G.11  : $pre_count rows / $(ls bench/fixtures/pre_g11_ndjson/*.ndjson | wc -l) files"
echo "  post-G.11 : $post_count rows / $(ls bench/fixtures/post_g11_ndjson/*.ndjson | wc -l) files"

echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "  EXP 1 complete."
echo "═══════════════════════════════════════════════════════════════"
```

Si tu veux, lance ce bloc d'un coup et tu auras les 7 outputs
attendus.  L'exécution prend < 5 secondes.

---

## Si quelque chose ne marche pas

| Symptôme | Cause probable | Recovery |
|----------|---------------|----------|
| `jq: command not found` | jq pas installé | `brew install jq` |
| `No such file or directory: bench/fixtures/...` | Pas dans le bon cwd | `cd /Users/rodrigue.pekar/Downloads/jjjj/efe` |
| `parse error: Invalid numeric literal` | Le fichier NDJSON est corrompu | `git checkout bench/fixtures/` |
| Aucun output sur le `for` loop | Pas de fichiers .ndjson | Vérifier `ls bench/fixtures/pre_g11_ndjson/*.ndjson` |

---

_Cette expérience a été préparée live le 2026-05-17 à 23:15 UTC.
Tous les outputs ci-dessus sont les outputs réels._

_Tu peux maintenant exécuter EXP 1 sans surprise._
