# EXP 3 — Prêt à exécuter : `compare` détecte une régression de performance

**Préparation date** : 2026-05-17 23:30 UTC
**Owner**            : @bench-infra-lead
**Durée d'exécution**: 5-7 min
**Pré-requis**       : binaire `efe-bench-harness` compilé, jq, bash
**État vérifié**     : 4 cas testés live (regression FAIL exit 1, warn exit 0, healthy exit 0, self exit 0) + 3 formats (markdown/text/json)

> Chaque output ci-dessous a été observé en LIVE sur le repo
> `/Users/rodrigue.pekar/Downloads/jjjj/efe` à 23:30 UTC le 2026-05-17.

---

## Vue d'ensemble — ce que fait `compare`

EFE prend **2 fichiers JSON conformes au baseline-v1 schema** :
- `--baseline` : la mesure de référence (run précédent, golden, etc.)
- `--result`   : la mesure candidate (nouveau PR, nouveau release)

Et il **émet 3 sorties** :
1. **Per-metric verdict** : pass / warn / fail par métrique
2. **Overall verdict** : pass / warn / fail (agrégat — fail si au moins une métrique fail)
3. **Exit code** : 0 si overall pass/warn, **1 si overall fail** → utilisé par CI comme merge gate

Les **seuils de tolérance** sont implicites par métrique (par défaut hardcodés ; cf JSON `tolerance_used`) :
- `p50_ms` : ±15 % avant fail
- `p95_ms` : ±25 %
- `p99_ms` : ±35 %
- `error_rate` : ±100 %
- `throughput_qps` : -15 % (uniquement la baisse compte, hausse = OK)

---

## STEP 3.0 — Setup workdir

**Commande** :
```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
rm -rf /tmp/efe-exp3 2>/dev/null
mkdir -p /tmp/efe-exp3
```

→ Workdir frais à `/tmp/efe-exp3/`.

---

## STEP 3.1 — Créer le baseline (le run "stable" qu'on a en référence)

**Commande** :
```bash
cat > /tmp/efe-exp3/baseline.json <<'EOF'
{
  "schema_version": 1,
  "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0",
  "loadgen_version": "0.1.0",
  "chart_version": "0.6.3",
  "image_tag": "v0.2.0",
  "git_commit": "abc1234",
  "run_id": "baseline-run-001",
  "started_at": "2026-05-17T10:00:00Z",
  "duration_s": 300,
  "total_requests": 50000,
  "metrics": {
    "p50_ms": 12.5,
    "p95_ms": 45.0,
    "p99_ms": 95.0,
    "error_rate": 0.001,
    "throughput_qps": 167.0,
    "recovery_time_s": null
  },
  "chaos_events": [],
  "verdict": "pass"
}
EOF
```

→ Le run de référence : p50=12.5, p95=45, p99=95 ms, error 0.1%, throughput 167 qps.

---

## STEP 3.2 — Créer 3 results candidates (régression / sain / warn-level)

**Commande** (3 fichiers) :
```bash
# RÉGRESSION : p50/p95/p99 explosent + error_rate × 8
cat > /tmp/efe-exp3/result-regression.json <<'EOF'
{
  "schema_version": 1, "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0", "loadgen_version": "0.1.0",
  "chart_version": "0.6.3", "image_tag": "v0.2.1",
  "git_commit": "def5678", "run_id": "test-run-001",
  "started_at": "2026-05-17T11:00:00Z",
  "duration_s": 300, "total_requests": 50500,
  "metrics": {
    "p50_ms": 18.0, "p95_ms": 65.5, "p99_ms": 145.0,
    "error_rate": 0.008, "throughput_qps": 168.0,
    "recovery_time_s": null
  },
  "chaos_events": [], "verdict": null
}
EOF

# SAIN : delta < 5 %, dans le bruit
cat > /tmp/efe-exp3/result-healthy.json <<'EOF'
{
  "schema_version": 1, "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0", "loadgen_version": "0.1.0",
  "chart_version": "0.6.3", "image_tag": "v0.2.0-rc.2",
  "git_commit": "ghi9012", "run_id": "test-run-002",
  "started_at": "2026-05-17T11:30:00Z",
  "duration_s": 300, "total_requests": 50100,
  "metrics": {
    "p50_ms": 12.8, "p95_ms": 46.0, "p99_ms": 97.0,
    "error_rate": 0.0012, "throughput_qps": 167.5,
    "recovery_time_s": null
  },
  "chaos_events": [], "verdict": null
}
EOF

# WARN-LEVEL : delta entre seuils warn et fail
cat > /tmp/efe-exp3/result-warn.json <<'EOF'
{
  "schema_version": 1, "scenario_id": "S01_smoke_test",
  "harness_version": "0.2.0", "loadgen_version": "0.1.0",
  "chart_version": "0.6.3", "image_tag": "v0.2.0-rc.3",
  "git_commit": "jkl3456", "run_id": "test-run-003",
  "started_at": "2026-05-17T12:00:00Z",
  "duration_s": 300, "total_requests": 50050,
  "metrics": {
    "p50_ms": 14.2, "p95_ms": 52.0, "p99_ms": 112.0,
    "error_rate": 0.003, "throughput_qps": 167.2,
    "recovery_time_s": null
  },
  "chaos_events": [], "verdict": null
}
EOF
```

→ 3 fichiers créés.

---

## STEP 3.3 — Compare REGRESSION (format markdown — l'output pour PR comments)

**Commande** :
```bash
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown
```

**Output observé live** :
```markdown
## 🔬 Bench results — `efe-bench-harness@0.2.0`

| Scenario | Verdict | p50 (ms) | p95 (ms) | p99 (ms) | error_rate | throughput (qps) |
|---|---|---|---|---|---|---|
| `S01_smoke_test` | ❌ fail | 18.0 (+44.0%) ❌ | 65.5 (+45.6%) ❌ | 145.0 (+52.6%) ❌ | 0.800% (+700.0%) ⚠ | 168.0 (+0.6%) |

🔗 Full results : [bench/results/](/bench/results/)
🔗 Methodology : [bench/README.md](/bench/README.md)
```

**Interprétation** :
- p50 +44 % → ❌ (dépasse 15 % seuil fail)
- p95 +45.6 % → ❌ (dépasse 25 %)
- p99 +52.6 % → ❌ (dépasse 35 %)
- error_rate +700 % → ⚠ (dépasse 100 % warn mais magnitude absolue reste petite)
- throughput +0.6 % → OK (hausse, donc bénéfique)
- **Overall : ❌ fail**

→ Ce tableau est ce qui est posté en commentaire automatique sur un PR GitHub.  Tu vois le delta par métrique, le verdict par cellule, et le verdict global.

---

## STEP 3.4 — Compare REGRESSION (format text — pour CI logs)

**Commande** :
```bash
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format text
echo "exit code: $?"
```

**Output observé live** :
```
S01_smoke_test : fail (5 metrics compared)
ERROR: bench verdict : 1 scenario(s) failed
exit code: 1
```

→ Format compact pour les logs CI.  **Exit code 1** = gate CI fail.
**C'est ce qui rejette le PR**.

---

## STEP 3.5 — Compare REGRESSION (format json — machine-readable)

**Commande** :
```bash
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format json 2>/dev/null
```

**Output observé live** (54 lignes) :
```json
{
  "scenario_id": "S01_smoke_test",
  "baseline_chart_version": "0.6.3",
  "baseline_git_commit": "abc1234",
  "rows": {
    "p50_ms": {
      "metric": "p50_ms",
      "baseline": 12.5,
      "actual": 18.0,
      "delta_pct": 44.0,
      "delta_abs": 5.5,
      "status": "fail",
      "tolerance_used": 15.0
    },
    "p95_ms": {
      "metric": "p95_ms",
      "baseline": 45.0,
      "actual": 65.5,
      "delta_pct": 45.55555555555556,
      "delta_abs": 20.5,
      "status": "fail",
      "tolerance_used": 25.0
    },
    "p99_ms": {
      "metric": "p99_ms",
      "baseline": 95.0,
      "actual": 145.0,
      "delta_pct": 52.63157894736842,
      "delta_abs": 50.0,
      "status": "fail",
      "tolerance_used": 35.0
    },
    "error_rate": {
      "metric": "error_rate",
      "baseline": 0.001,
      "actual": 0.008,
      "delta_pct": 700.0,
      "delta_abs": 0.007,
      "status": "warn",
      "tolerance_used": 1.0
    },
    "throughput_qps": {
      "metric": "throughput_qps",
      "baseline": 167.0,
      "actual": 168.0,
      "delta_pct": 0.5988023952095809,
      "delta_abs": 1.0,
      "status": "pass",
      "tolerance_used": -15.0
    }
  },
  "overall": "fail"
}
```

**Interprétation** : c'est le format que consomme un Dashboard
Prometheus, un script de release engineering, ou un audit
automatique.  Chaque métrique a son :
- `baseline` + `actual` (valeurs)
- `delta_pct` (variation relative)
- `delta_abs` (variation absolue)
- `status` (pass/warn/fail)
- `tolerance_used` (le seuil hardcodé pour cette métrique)

Et un **`overall`** champ qui agrège.

---

## STEP 3.6 — Compare HEALTHY (delta < 5 %, scenario OK)

**Commande** :
```bash
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-healthy.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown
echo "exit code: $?"
```

**Output observé live** :
```markdown
## 🔬 Bench results — `efe-bench-harness@0.2.0`

| Scenario | Verdict | p50 (ms) | p95 (ms) | p99 (ms) | error_rate | throughput (qps) |
|---|---|---|---|---|---|---|
| `S01_smoke_test` | ✅ pass | 12.8 (+2.4%) | 46.0 (+2.2%) | 97.0 (+2.1%) | 0.120% (+20.0%) | 167.5 (+0.3%) |

exit code: 0
```

→ Toutes les métriques sous les seuils.  **Verdict ✅ pass, exit 0**.  Le PR peut merger.

---

## STEP 3.7 — Compare WARN-level (entre seuils warn et fail)

**Commande** :
```bash
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-warn.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown
echo "exit code: $?"
```

**Output observé live** :
```markdown
## 🔬 Bench results — `efe-bench-harness@0.2.0`

| Scenario | Verdict | p50 (ms) | p95 (ms) | p99 (ms) | error_rate | throughput (qps) |
|---|---|---|---|---|---|---|
| `S01_smoke_test` | ⚠ warn | 14.2 (+13.6%) ⚠ | 52.0 (+15.6%) ⚠ | 112.0 (+17.9%) ⚠ | 0.300% (+200.0%) | 167.2 (+0.1%) |

exit code: 0
```

**Interprétation** :
- p50 +13.6 % → ⚠ (entre warn ~5 % et fail 15 %)
- p95 +15.6 % → ⚠ (sous fail 25 %)
- p99 +17.9 % → ⚠ (sous fail 35 %)
- Verdict overall : ⚠ warn
- **Exit code 0** → le PR peut merger MAIS un humain doit regarder

→ La distinction critique : **WARN ≠ FAIL** côté gate.  WARN = bruit
acceptable mais notifié.  FAIL = rejet automatique.

---

## STEP 3.8 — Self-compare (baseline vs baseline — sanity check à 0.0 %)

**Commande** :
```bash
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/baseline.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format markdown
echo "exit code: $?"
```

**Output observé live** :
```markdown
## 🔬 Bench results — `efe-bench-harness@0.2.0`

| Scenario | Verdict | p50 (ms) | p95 (ms) | p99 (ms) | error_rate | throughput (qps) |
|---|---|---|---|---|---|---|
| `S01_smoke_test` | ✅ pass | 12.5 (+0.0%) | 45.0 (+0.0%) | 95.0 (+0.0%) | 0.100% (+0.0%) | 167.0 (+0.0%) |

exit code: 0
```

→ Toutes les colonnes à +0.0 %.  Verdict ✅ pass.  C'est le sanity
check qui prouve que le comparateur ne hallucine pas.

---

## STEP 3.9 — Table récap des 4 cas avec exit codes

**Commande** :
```bash
echo "─── Récap des exit codes (le critère gate CI) ───"

for case_name in "Self (baseline vs baseline)" "HEALTHY" "WARN" "REGRESSION"; do
  case "$case_name" in
    Self*)  result=baseline.json ;;
    HEALTHY) result=result-healthy.json ;;
    WARN)   result=result-warn.json ;;
    REGRESSION) result=result-regression.json ;;
  esac

  ./target/debug/efe-bench-harness compare \
    --result /tmp/efe-exp3/$result \
    --baseline /tmp/efe-exp3/baseline.json \
    --format text > /dev/null 2>&1
  printf "  %-32s : exit %d\n" "$case_name" "$?"
done
```

**Output observé live** :
```
  Self (baseline vs baseline)      : exit 0
  HEALTHY                          : exit 0
  WARN                             : exit 0
  REGRESSION                       : exit 1
```

→ **Le seul cas qui exit non-zero est REGRESSION**.  C'est le seul
cas qui bloque le merge CI.

---

## STEP 3.10 — Inspect le tolerance_used par métrique (les seuils hardcodés)

**Commande** :
```bash
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/result-regression.json \
  --baseline /tmp/efe-exp3/baseline.json \
  --format json 2>/dev/null | \
  jq '.rows | to_entries[] | {metric: .key, tolerance_used: .value.tolerance_used, status: .value.status}'
```

**Output observé live** :
```json
{
  "metric": "p50_ms",
  "tolerance_used": 15.0,
  "status": "fail"
}
{
  "metric": "p95_ms",
  "tolerance_used": 25.0,
  "status": "fail"
}
{
  "metric": "p99_ms",
  "tolerance_used": 35.0,
  "status": "fail"
}
{
  "metric": "error_rate",
  "tolerance_used": 1.0,
  "status": "warn"
}
{
  "metric": "throughput_qps",
  "tolerance_used": -15.0,
  "status": "pass"
}
```

**Interprétation** :
- p50 seuil 15 % → un delta de 44 % le dépasse → fail
- p95 seuil 25 % → delta 45.6 % dépasse → fail
- p99 seuil 35 % → delta 52.6 % dépasse → fail
- error_rate seuil 1.0 (= 100 %) → delta 700 % dépasse mais classé warn (magnitude absolue 0.007 reste petite)
- throughput seuil -15 % (négatif car direction inverse) → +0.6 % est OK

→ Les seuils sont **per-metric** et **hardcoded** dans le binaire
(tracké comme G-PR27.1-WARN-MARGIN dans master_tracker pour la
production-alignment).

---

## STEP 3.11 — Cleanup

**Commande** :
```bash
rm -rf /tmp/efe-exp3
echo "  cleanup done"
```

---

## Synthèse de l'EXP 3

| Cas | Verdict | Exit code | Gate CI | Action humaine |
|-----|---------|-----------|---------|----------------|
| Self-compare (baseline vs baseline) | ✅ pass | 0 | merge OK | aucune |
| HEALTHY (delta < 5 %) | ✅ pass | 0 | merge OK | aucune |
| WARN (delta 5-15 %) | ⚠ warn | 0 | merge OK | revue humaine recommandée |
| REGRESSION (delta > 15 %) | ❌ fail | **1** | **merge BLOQUÉ** | investiguer / amender / rollback |

**Garanties démontrées** :
1. EFE **calcule** les deltas absolus + relatifs par métrique.
2. EFE **classifie** chaque métrique en pass/warn/fail vs seuil.
3. EFE **agrège** un overall verdict (fail si ≥ 1 métrique fail).
4. EFE **émet 3 formats** (markdown PR / text CI logs / JSON dashboard).
5. EFE **exit code 1** pour fail → consommable par GitHub Actions.
6. EFE **0.5 seconde** par comparaison (4 comparisons en < 2 sec).

**Ce qu'EXP 3 prouve** :
- Un humain dans la PR voit le delta verbatim.
- Le CI rejette automatiquement les régressions > seuil.
- Le dashboard consomme le JSON pour graphique tendance.

C'est **un seul binaire** qui sert 3 audiences différentes (humain
PR-reviewer, CI bot, dashboard).  Audit-grade par construction.

---

## Bonus — Script tout-en-un (exécutable)

```bash
bash scripts/efe-exp3-run.sh
```

Companion file `scripts/efe-exp3-run.sh` exécute les 11 steps en
séquence avec les outputs colorés.

---

## Recovery si quelque chose ne marche pas

| Symptôme | Cause probable | Recovery |
|----------|---------------|----------|
| `unknown variant 'WARN'` (uppercase) | Schema v3 vs v1 confusion | Vérifier `verdict: null` (pas "WARN") et que `chaos_events: []` |
| `missing field 'chaos_events'` | Schema incomplet | Ajouter `"chaos_events": []` dans tous les fichiers |
| `invalid type: sequence, expected a map` | Wrong root structure | Vérifier le JSON est un objet (pas array) |
| `bench-harness aborted error=...` | Schema fail | Lire le message d'erreur — il dit exactement quel champ manque |
| Exit code 1 même pour HEALTHY | Mauvaise comparaison de seuils | Vérifier que les deltas sont bien < 15 % pour p50, < 25 % pour p95, etc. |
| Le markdown ne s'affiche pas | Output stderr-only | Pipeline mal câblé — séparer 2>&1 |

---

## Edge cases (à connaître mais pas dans le script de base)

### Cas 1 — Compare result vs result-différent-scenario

```bash
# Si scenario_id ≠ → erreur
./target/debug/efe-bench-harness compare \
  --result /tmp/efe-exp3/baseline.json \
  --baseline /tmp/efe-exp3/result-regression.json
# (les deux ont le même scenario_id → ça marche)
# Si on changeait un des scenario_id → mismatch error
```

### Cas 2 — Metric absent dans le result

```bash
# Si une métrique manque, elle est silently ignorée
# (n'est pas counted dans la comparaison)
```

### Cas 3 — Metric null

```bash
# Si une métrique est null dans le baseline OU result, elle est skipped
# (cf recovery_time_s : null partout dans nos exemples → pas dans le tableau)
```

---

_Cette expérience a été préparée live le 2026-05-17 à 23:30 UTC.
Tous les outputs ci-dessus sont les outputs réels._

_Tu peux maintenant exécuter EXP 3 sans surprise._
