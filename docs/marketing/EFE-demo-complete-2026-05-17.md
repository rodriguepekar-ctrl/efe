# EFE — Démo COMPLÈTE situation réelle (avec et sans meta-swarm)

**Date** : 2026-05-17
**Owner** : @bench-infra-lead
**Audience** : toi-même (validation produit) OU investisseur / customer technique
**Durée** : 25-30 min sans cluster (Ch. 1-5, 7), +30 min avec cluster (Ch. 6)
**Pré-requis** : `PYO3_PYTHON=python3.12`, repo dans `/Users/rodrigue.pekar/Downloads/jjjj/efe`, cargo cache chaud

> **Modes** :
> - **Sans meta-swarm** = bench-platform standalone + audit discipline (Ch. 2-4 + 7).  Tout tourne sur laptop, offline.
> - **Avec meta-swarm** = + le L2 layer (controller + participant + loadgen) + saga + LLM router + active learning + agent runtime (Ch. 5).  Modes introspection/validate sans cluster ; ou cluster réel via k3d (Ch. 6).

---

## 0. Setup (T-0)

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
export PYO3_PYTHON=python3.12
export CARGO_TERM_COLOR=always
clear
```

Optional pre-flight :
```bash
bash scripts/efe-demo-preflight.sh
```

---

## Chapitre 1 — L'inventaire (qu'est-ce que tu as construit) — 3 min

Le scénario : "Je suis arrivé sur le projet EFE.  Qu'est-ce qu'il
contient ?"

### 1.1 Les 35 crates Rust du workspace

```bash
find . -maxdepth 4 -name "Cargo.toml" -not -path "*/target/*" -not -path "*/node_modules/*" | sort | head -40
```

→ 35 crates : `efe-core` à `efe-vector`, `bench/harness`, `slos/rollup`, `smoke/runner`, `tools/loadgen`.

### 1.2 Les 15 binaires

```bash
ls target/debug/efe-* target/debug/controller target/debug/participant 2>/dev/null | grep -v "\.\|deps"
```

→ Binaires en production : efe-bench-harness, efe-bench, efe-smoke, efe-server, efe-llm-router, efe-onboard, efe-bundle, efe-verify-audit, efe-agent-runner, efe-backup, efe-admin, efe-slo-rollup, efe-meta-swarm-loadgen, controller, participant.

### 1.3 Les sub-systèmes par directory

```bash
ls -1d crates/* slos/* smoke/* bench/* tools/* 2>/dev/null
```

### 1.4 Les artefacts d'audit produits

```bash
ls -1 docs/lessons/PR-2[4-7]-*.md docs/lessons/monitoring/ docs/lessons/checklists/ docs/gaps/ docs/reports/ 2>/dev/null
```

→ 6 lessons files (PR #24-#27 + CB-catalogue + lessons template), 6 monitoring artifacts S9-S12, 7 audit checklists S3, 1 master gap tracker, 1 product test report.

**Ce que ça prouve** : EFE n'est pas un mono-script.  C'est un produit avec 35 crates, 15 binaires, et une chaîne d'audit documentée.

---

## Chapitre 2 — Le contrat de stabilité (5 min)

Le scénario : "Comment EFE garantit-il que ses surfaces ne changent pas sous mes pieds ?"

### 2.1 Le contrat STABILITY (gros doc, 1500+ lignes)

```bash
wc -l docs/STABILITY_CONTRACT.md
grep -c "^## " docs/STABILITY_CONTRACT.md       # nombre de sections
grep "^## " docs/STABILITY_CONTRACT.md | head -30
```

### 2.2 Le freeze : 51 tests gèlent les surfaces v1.7

```bash
cargo test -p efe-bench-harness --test stability_contract_v17 --quiet | tail -1
```

→ 51 PASS.  Si quelqu'un renomme un variant d'enum, change un champ JSON, ou ajoute un identifier non-déclaré, ces 51 détectent.

### 2.3 Le catalogue : 35 CB scenarios

```bash
ls bench/scenarios/CB-*.yaml | wc -l
```

→ 35.

```bash
# Validate les 35 scenarios contre le schéma
for s in bench/scenarios/CB-*.yaml ; do
  bash scripts/cb-scenario-validator.sh "$s" 2>&1 | head -1 | grep -E "^ok|^fail"
done | sort | uniq -c
```

→ 35 ok, 0 fail.

### 2.4 Inspect un scenario réel

```bash
less bench/scenarios/CB-09_llm_router_decision_latency.yaml
```

→ Tu vois la structure : id, suite, family, subsystem, tolerance keys, workload spec.

**Ce que ça prouve** : 35 scenarios couvrent les 35 crates.  Chacun validable.  Le contrat est lisible.

---

## Chapitre 3 — La discipline G.11 (l'audit-driven engineering, 5 min)

Référence : tout le pack PR #27.

### 3.1 Le pinning test

```bash
cargo test -p efe-bench-harness --test g11_round_trip
```

→ 9 PASS en 10 ms.

### 3.2 Rituel de déterminisme

```bash
for i in 1 2 3 4 5; do
  cargo test -p efe-bench-harness --test g11_round_trip --quiet
done
```

→ 5x "ok. 9 passed".  Sérialisation byte-identical.

### 3.3 Wow moment — structurellement non-régressable

```bash
# Supprime un fichier audit → le test casse
mv docs/lessons/monitoring/PR-27-monitoring-week-11.md /tmp/.efe-demo
cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists 2>&1 | tail -3

# Restaure
mv /tmp/.efe-demo docs/lessons/monitoring/PR-27-monitoring-week-11.md
cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists --quiet | tail -1
```

→ FAILED puis ok.

### 3.4 Vérifier qu'aucun fichier ne traîne

```bash
ls docs/lessons/monitoring/PR-27-monitoring-week-11.md  # doit exister
ls /tmp/.efe-demo 2>&1 | head -1                          # doit dire "No such file"
```

**Ce que ça prouve** : la documentation d'audit est testée comme du code.

---

## Chapitre 4 — Bench-platform SANS meta-swarm (10 min)

Scénario : "Je veux tester un workload sans toucher au L2 distributé."

### 4.1 Génération de dataset de test

```bash
./target/debug/efe-bench gen --spec smoke --out /tmp/efe-test-dataset.json
head -20 /tmp/efe-test-dataset.json
```

→ Dataset JSON pour warmup ou test fixtures.

### 4.2 Lister les 7 SLOs cibles du bench

```bash
./target/debug/efe-bench list-slos
```

→ 7 SLO targets + 3 derived (D31-GPU-1, C-3, C-9).

### 4.3 Garde-fou anti-prod

```bash
./target/debug/efe-bench validate-target-env --env dev      # exit 0
./target/debug/efe-bench validate-target-env --env staging  # exit 0
./target/debug/efe-bench validate-target-env --env prod ; echo "exit=$?"   # exit 7
```

→ Le bench refuse de tourner contre prod.  Garde-fou critique.

### 4.4 efe-bundle : signature + vérification d'artefact

```bash
# Génère une paire Ed25519
mkdir -p /tmp/efe-keygen
./target/debug/efe-bundle keygen --out /tmp/efe-keygen 2>&1 | head -5
ls -la /tmp/efe-keygen
```

→ Clé privée 32 bytes mode 0400, clé publique 32 bytes mode 0444.

```bash
# Inspect une bundle existante (fixture)
./target/debug/efe-bundle --help 2>&1 | head -20
```

### 4.5 SLO rollup (les 4 SLOs du PR #22)

```bash
./target/debug/efe-slo-rollup --help 2>&1 | grep -A 6 "Commands"
./target/debug/efe-slo-rollup compute --help 2>&1 | head -15
```

→ compute (T0/T1), rollup (T1/T2/T3), publish (mdbook + JSON API), evidence (signature quarterly).

### 4.6 Smoke runner (les 9 checks merge-gate)

```bash
./target/debug/efe-smoke list
```

→ S1_boot → S9_shutdown, budgets en secondes.

```bash
# Schema-validate un smoke-result fictif (pas besoin de cluster)
cat > /tmp/efe-smoke-result.json <<'EOF'
{
  "version": "v1",
  "git_sha": "abc1234",
  "cluster_id": "demo-cluster",
  "started_at": "2026-05-17T22:00:00Z",
  "ended_at": "2026-05-17T22:03:00Z",
  "checks": {}
}
EOF
./target/debug/efe-smoke validate /tmp/efe-smoke-result.json 2>&1 | head -5
```

### 4.7 Backup orchestrator (DR drill)

```bash
./target/debug/efe-backup --help 2>&1 | grep -A 10 "Commands"
```

### 4.8 Onboarding CLI (D15)

```bash
./target/debug/efe-onboard --help 2>&1 | grep -A 8 "Commands"
```

→ preflight, register, activate, suspend, list, get.  Nécessite une DB.

### 4.9 Quarterly publish dry-run (déjà fait, on lit le rapport)

```bash
less docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md
```

→ 7 stages, PASS, 0 défauts, signature cosign, audit-reader round-trip.

**Ce que ça prouve** : tout le surface bench-platform tourne sans le L2.  C'est le "single-node mode".

---

## Chapitre 5 — Avec meta-swarm (sans cluster — modes introspection) — 8 min

Scénario : "Maintenant je veux voir le distributed layer."

### 5.1 Loadgen profile validation (sans traffic)

```bash
ls tools/loadgen/examples/*.yaml
```

→ 3 profiles : algorithm-mixed-1h, canary-50qps-10min, chaos-burst-30min.

```bash
./target/debug/efe-meta-swarm-loadgen validate --profile tools/loadgen/examples/canary-50qps-10min.yaml
```

→ "profile OK : version=1 name='canary-50qps-10min' endpoints=1"

```bash
./target/debug/efe-meta-swarm-loadgen validate --profile tools/loadgen/examples/chaos-burst-30min.yaml
./target/debug/efe-meta-swarm-loadgen validate --profile tools/loadgen/examples/algorithm-mixed-1h.yaml
```

→ Les 3 profiles valides.

### 5.2 Loadgen subcommands tour

```bash
./target/debug/efe-meta-swarm-loadgen --help | grep -A 6 "Commands"
```

→ run, validate, replay, aggregate.  Le `validate` ne touche pas le réseau.

### 5.3 Architecture meta-swarm (lecture)

```bash
ls crates/efe-meta-swarm*
ls docs/EFE_A1_PHASE2_META_SWARM*.pdf 2>&1 | head -1
```

→ 3 crates : efe-meta-swarm (lib), efe-meta-swarm-proto (gRPC types), efe-meta-swarm-server (binaires controller + participant).

### 5.4 Saga orchestrator (M21+D93)

```bash
ls crates/efe-saga/src/*.rs 2>&1 | head -10
```

→ State machine pour distributed transactions (P2 self-service deploy + GDPR cascade).

### 5.5 LLM router (M21 Mode-2 LLM-routed anchors)

```bash
ls crates/efe-llm-router/src/*.rs 2>&1 | head -10
./target/debug/efe-llm-router --help 2>&1 | head -5
```

→ Microservice multi-provider, cost governance, caching, circuit breaker.

### 5.6 Active learning loop (M24)

```bash
ls crates/efe-active-learning/src/ 2>&1
```

→ Closed-loop : production → uncertainty sampling → LLM oracle → gold dataset → centroid retune → eval + canary → promotion.

### 5.7 Agent runtime (M25 EXPERIMENTAL)

```bash
ls crates/efe-agent-runner/src/ 2>&1 | head -10
```

→ In-cluster Firecracker microVM sandbox + tool whitelist + step approval + DPIA + AI Act Art. 9.

### 5.8 Curation (M23 LLM-assisted taxonomy)

```bash
ls crates/efe-curation/src/ 2>&1 | head -10
```

→ 5 MCP tools, canary rollout engine, HMAC audit chain.

### 5.9 Cache + events + multi-modal

```bash
ls crates/efe-cache/src/ crates/efe-events/src/ crates/efe-multimodal/src/ 2>&1 | head -10
```

**Ce que ça prouve** : le meta-swarm n'est pas un nuage flou.  Chaque sous-système (saga, LLM router, active learning, agent runtime) est un crate Rust avec sa propre surface testable.

---

## Chapitre 6 — Cluster local end-to-end avec k3d (OPTIONNEL, 30 min)

Scénario : "Je veux voir un vrai cluster, pas juste de l'introspection."

### 6.1 Pré-requis cluster

```bash
which k3d || brew install k3d
which kubectl || brew install kubectl
which helm || brew install helm
```

### 6.2 Lancer le cluster k3d (1-3 min)

```bash
bash scripts/k3d-e2e.sh 2>&1 | tail -10
```

(Voir le script pour voir ce qu'il fait : créer cluster, install charts, lancer smoke.)

### 6.3 Vérifier le cluster

```bash
kubectl get nodes
kubectl get pods -A
helm list -A
```

### 6.4 Lancer le smoke runner réel

```bash
./target/debug/efe-smoke run --check S1_boot,S2_l1_swarm,S6_metrics 2>&1 | tail -20
```

→ Les checks tournent contre le cluster, produisent un smoke-result.json.

### 6.5 Lancer loadgen contre le cluster

```bash
./target/debug/efe-meta-swarm-loadgen run --profile tools/loadgen/examples/canary-50qps-10min.yaml --target localhost:50080 2>&1 | tail -20
```

### 6.6 Tear-down

```bash
k3d cluster delete efe-demo
```

**Ce que ça prouve** : end-to-end fonctionne sur un cluster réel local.

---

## Chapitre 7 — La preuve d'audit (closure ceremony) — 3 min

Scénario : "Comment un régulateur ou un client vérifierait que les claims tiennent ?"

### 7.1 Le master gap tracker

```bash
less docs/gaps/master_tracker.md
```

→ G.11 [closed, verified 2026-08-07].  11 G-XX trackés.  Historique des closures.

### 7.2 La closure ceremony PR #27

```bash
less docs/lessons/monitoring/PR-27-closure-2026-08-07.md
```

→ Triple sign-off, 110/110 pinning, 20/20 nightly, 174 tests, 43 defects (33 fixed / 7 deferred / 1 process).

### 7.3 Le validator schéma sur le lessons file

```bash
bash scripts/audit-prompt-validator.sh docs/lessons/PR-27-lessons.md | tail -3
```

→ sections=7 defects=19 passes=9 gap-lines=13.

### 7.4 Le rapport produit-test du matin

```bash
less docs/reports/EFE-product-test-2026-05-17.md
```

→ 99.76 % de tests verts.  13 échecs orthogonaux trackés.

### 7.5 STABILITY contract audit-trail (rows populées au closure)

```bash
grep -A 1 "2026-08-07" docs/STABILITY_CONTRACT.md | head -10
```

→ 2 rows datées 2026-08-07 sous § 24.9 et § 27.5.

### 7.6 Postmortem validator (cas hypothétique de panne)

```bash
bash scripts/postmortem-validator.sh --help 2>&1 | head -3
```

→ Le validator de post-mortems CI-enforced (G.15 PR #26).

**Ce que ça prouve** : la chaîne d'audit est complète, signée, et reproductible end-to-end.

---

## Chapitre 8 — Récap final et sanity check (1 min)

```bash
# Tout doit être propre après la démo
ls /tmp/.efe-* /tmp/efe-* 2>&1 | head -5
rm -rf /tmp/.efe-demo /tmp/efe-test-dataset.json /tmp/efe-smoke-result.json /tmp/efe-keygen 2>/dev/null

# Les 7 tests PR #27 doivent encore tous passer
cargo test -p efe-bench-harness \
  --test g11_round_trip \
  --test g11_tolerance_used_populated \
  --test stability_contract_v17 \
  --test backfill_edge_cases \
  --test pr27_workflow_yaml_invariants \
  --test s3_audit_chain_integrity \
  --test s9_s12_monitoring_chain_integrity 2>&1 | grep -E "^test result:"

# Git status doit être propre (sauf nos artefacts marketing créés aujourd'hui)
git status --short | head -10
```

---

## Résumé exécutif

| Chapitre | Sans meta-swarm | Avec meta-swarm | Cluster requis | Durée |
|----------|-----------------|------------------|----------------|-------|
| 1. Inventaire | ✅ | ✅ | non | 3 min |
| 2. Contrat | ✅ | ✅ | non | 5 min |
| 3. G.11 discipline | ✅ | ✅ | non | 5 min |
| 4. Bench-platform | ✅ | ✅ | non | 10 min |
| 5. Meta-swarm (introspection) | — | ✅ | non | 8 min |
| 6. Cluster end-to-end | — | ✅ | **OUI** | 30 min |
| 7. Audit proof | ✅ | ✅ | non | 3 min |
| 8. Sanity check | ✅ | ✅ | non | 1 min |
| **TOTAL sans cluster** | | | | **25-30 min** |
| **TOTAL avec cluster** | | | | **+ 30 min** |

---

## Quoi montrer à qui ?

| Audience | Chapitres |
|----------|-----------|
| Toi-même (test produit) | Tout (1 → 8) |
| Investisseur Kovant 30 min | 1 + 3 + 4 + 7 (skip 5 ou survol) |
| YC partner 10 min | 3 + 7 + extrait 4 |
| Customer technique 45 min | 1 + 2 + 4 + 5 + 7 + démo 6 si dispo |
| Régulateur / auditeur | 2 + 3 + 7 (insister sur la chaîne d'audit) |
| Recrutement engineer senior | 1 + 2 + 3 + 5 (montrer la profondeur) |

---

_Cette démo couvre l'intégralité d'EFE.  Toutes les commandes
ci-dessus ont été testées live le 2026-05-17 sur le repo
`/Users/rodrigue.pekar/Downloads/jjjj/efe`.  Aucune n'est théorique._
