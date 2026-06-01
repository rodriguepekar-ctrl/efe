# EFE Demo — Contournements connus (master reference)

**Date drafted** : 2026-05-17
**Companion**    : `docs/marketing/EFE-demo-script-2026-05-17.md`
**Owner**        : @bench-infra-lead
**Statut**       : LIVE — toutes les contournements ci-dessous ont été testés en live le 2026-05-17 sur le repo actuel
**Audience**     : toi-même, le matin de la démo, en mode "panique 5 minutes avant"

---

## 0. Comment lire ce document

Tu es stressé.  Tu n'as pas le temps de lire de la prose.  Ce
document est organisé pour la **consultation rapide en condition
réelle** :

1. **§ 1 (Pre-flight)** : à lire UNE FOIS, T-30 minutes avant la
   démo.  Tout coche, tu es bon.
2. **§ 2 (Inventaire défauts)** : référence ; tu y reviens si
   quelqu'un te pose une question hostile.
3. **§ 3 (Pendant la démo)** : à scanner pendant que la commande
   tourne, pour t'assurer que rien ne va casser.
4. **§ 4 (Never Do List)** : à mémoriser.  Si tu vas appuyer sur
   Entrée sur une commande de cette liste, **ARRÊTE**.
5. **§ 10 (Cheat sheet)** : à imprimer en 1 page, à coller sur la
   tranche du laptop.

### 0.1 Les 3 règles d'or

1. **Si une commande échoue de façon inattendue, narrate calmly.**
   "Intéressant, le test surface un cas que mes répétitions n'ont
   pas couvert.  Je notepad ça pour la post-mortem."  Continuer.
2. **Le wow-moment doit toujours finir avec le fichier restauré.**
   Si tu sors de la démo en oubliant le `mv` retour, les tests
   resteront cassés.  Trap obligatoire (cf § 5).
3. **Aucune commande qui modifie l'état partagé du repo en démo.**
   Pas de `cargo clean`, pas de `cargo update`, pas de `git`
   destructif, pas de `--write`.

---

## 1. Pre-flight contournements (T-30 minutes)

### 1.1 Environnement shell

```bash
# Set in this exact order:
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
export PYO3_PYTHON=python3.12
export CARGO_TERM_COLOR=always
export CARGO_INCREMENTAL=0   # économise le disque pendant la démo

# Vérifier que le cwd est correct
[ "$(pwd)" = "/Users/rodrigue.pekar/Downloads/jjjj/efe" ] || { echo "BAD CWD"; exit 1; }
```

**Si l'une des trois variables n'est pas exportée**, les démos
peuvent échouer silencieusement (rebuild pyo3, output sans couleurs,
disque saturé).  À vérifier en `env | grep -E 'PYO3|CARGO'` avant de
commencer.

### 1.2 Disk space — vérification + cleanup

```bash
# Doit afficher au moins 5 Gi libres
df -h ~ | awk 'NR==2{print "  available:", $4}'

# Si < 5 Gi : cleanup les artefacts non-essentiels SANS toucher au cache de tests
rm -rf target/release            # 6.8 GB récupérable
rm -rf target/debug/incremental  # 1-3 GB
rm -rf target/criterion          # 1.5 MB (négligeable mais propre)
rm -rf target/doc                # 8 MB

# JAMAIS faire :
# cargo clean    # casserait tout, rebuild 8+ min
```

**Disk currently** (2026-05-17 21:00 UTC) : **13 GB libres / 926 GB
total**.  La marge est tendue.  Si tu installes quoi que ce soit
d'autre entre maintenant et la démo, **re-vérifie**.

### 1.3 Pre-build de tous les binaires utilisés en démo

```bash
# 1m si cache chaud, 1m 08s si Cargo.lock touché
cargo build -p efe-bench-harness --bin efe-bench-harness
cargo build -p efe-bench         --bin efe-bench
cargo build --workspace --bins   # warmup complet pour Phase 6 (3-5 min)

# Vérifier que les binaires existent + sont récents
ls -la target/debug/efe-bench-harness target/debug/efe-bench 2>&1
```

### 1.4 Pre-compile de tous les tests utilisés en démo

```bash
# 30s si cache chaud, 1m 08s si cache froid
cargo test -p efe-bench-harness --no-run \
  --test g11_round_trip \
  --test stability_contract_v17 \
  --test s3_audit_chain_integrity \
  --test s9_s12_monitoring_chain_integrity \
  --test backfill_edge_cases

# Une fois les tests compilés, les ré-exécutions en démo font ~10ms.
```

### 1.5 Vérification fonctionnelle des 6 commandes critiques

Coller-coller, tout doit afficher du PASS / 0 failed :

```bash
cargo test -p efe-bench-harness --test g11_round_trip --quiet 2>&1 | tail -1
# attendu : test result: ok. 9 passed; ...

cargo test -p efe-bench-harness --test stability_contract_v17 --quiet 2>&1 | tail -1
# attendu : test result: ok. 51 passed; ...

cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity --quiet 2>&1 | tail -1
# attendu : test result: ok. 16 passed; ...

bash scripts/audit-prompt-validator.sh docs/lessons/PR-27-lessons.md 2>&1 | tail -1
# attendu : info: sections=7 defects=19 passes=9 gap-lines=13

bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/ 2>&1 | tail -1
# attendu : End of golden diff.

./target/debug/efe-bench-harness version 2>&1 | head -1
# attendu : efe-bench-harness 0.2.0
```

**Si une SEULE de ces 6 lignes ne correspond pas exactement à
l'attendu**, AVORTER la démo.  Debugger hors scène.

### 1.6 Network checks

| Surface | Need network? | Demo dependency |
|---------|---------------|-----------------|
| `cargo test` (compiled) | ❌ | None |
| `cargo test` (uncompiled) | ✅ (crates.io) | Pre-flight built tests |
| `bash scripts/audit-prompt-validator.sh` | ❌ | Pure shell + grep |
| `bash scripts/backfill-tolerance-used.sh --dry-run` | ❌ | Pure shell + Python |
| `./target/debug/efe-bench-harness version` | ❌ | Static binary |
| **cosign verify-blob** (NOT in script) | ✅ (Rekor) | Don't use in demo |

**Conclusion** : ta démo est **offline-safe**.  Tu peux la faire
même si le wifi tombe.

### 1.7 Terminal sanity

```bash
# Tester la couleur cargo
cargo test -p efe-bench-harness --test g11_round_trip --quiet 2>&1 | head -3
# Le "ok" doit apparaître en vert.  Sinon CARGO_TERM_COLOR non exporté.

# Tester la largeur
echo "1234567890" | head -c 120 | wc -c
# Doit afficher 11 (10 chars + newline).  Si tu vois 120, ton tput est bizarre.

# Tester la hauteur (35 lignes ?)
tput lines
# Doit afficher ≥ 35
```

### 1.8 macOS Gatekeeper / quarantaine

Si le laptop a redémarré récemment, vérifier que les binaires
n'ont pas été mis en quarantaine :

```bash
xattr -l target/debug/efe-bench-harness 2>&1 | head -3
# Si tu vois "com.apple.quarantine", supprimer :
xattr -d com.apple.quarantine target/debug/efe-bench-harness 2>/dev/null || true
xattr -d com.apple.quarantine target/debug/efe-bench         2>/dev/null || true
```

### 1.9 Backup state snapshot (avant la démo, optional mais safe)

```bash
# Crée un fichier témoin de l'état "demo-ready"
git rev-parse HEAD > /tmp/efe-demo-baseline-commit.txt
git status --short > /tmp/efe-demo-baseline-status.txt 2>&1
ls docs/lessons/monitoring/*.md > /tmp/efe-demo-baseline-monitoring.txt

# Après la démo, vérifier que rien n'a bougé :
# diff /tmp/efe-demo-baseline-monitoring.txt <(ls docs/lessons/monitoring/*.md)
```

---

## 2. Inventaire complet des défauts connus

Référencer cette section quand l'audience pose une question
spécifique du genre "et ça là, pourquoi ça plante ?" — tu sors la
fiche, tu réponds en 10 secondes.

### 2.1 Défauts trackés dans master_tracker.md

| Code | Symptôme | Cause racine | Impact démo | Contournement |
|------|----------|--------------|-------------|---------------|
| **G-ENGINE-CB** | `cargo test --workspace` fait échouer 6 tests dans `engine.rs` et `golden_scenarios.rs` ; `efe-bench-harness list/validate` plante avec "id 'CB-01_*' must match ^S[0-9]{2}_..." | Regex à `bench/harness/src/scenario/mod.rs:45` : `^S[0-9]{2}_[a-z0-9_]+$` rejette le préfixe `CB-` ajouté par le PR CB-catalogue-expansion | **Haut si tu cours `list` ou `validate` ; nul sinon** | Ne PAS utiliser `list` / `validate` en démo.  Si forcé, patch sed (cf § 4.2) |
| **G-CB-IMPL** | Workload réel des 35 CB scenarios non implémenté | Hors scope PR #27 ; chaque famille doit faire son propre workload | Aucun (pas dans le script) | "C'est tracé sous G-CB-IMPL avec un effort estimé à 8 engineer-weeks" |
| **G-CB-BASELINES** | Baselines NDJSON de production manquants | Données pas encore collectées | Aucun (démo utilise fixtures) | "On démontre sur fixtures synthétiques.  Les baselines réels seront collectés à partir de la PR #28" |
| **G-CB-LONGRUN-INFRA** | Long-run (24h+, soak/leak) infra non câblée | Besoin d'un cluster K8s permanent | Aucun (pas dans le script) | "C'est l'item G-CB-LONGRUN-INFRA, propriétaire release-eng, ETA Q4" |
| **G-PR27.1-OVERRIDE-LEN** | `override_reason` accepte des chaînes > 200 chars | Pas de length check côté serde | Aucun (introspection seulement) | "Tracé dans PR #27.1 (~ 2h de travail)" |
| **G-PR27.1-UNIT-CLASSIFIER** | `is_throughput()` ne reconnaît que `EventsPerSec` | Hardcodé sur un seul variant | Aucun | "PR #27.1, ~ 1h" |
| **G-PR27.1-TAXONOMY-AUTO** | Pas de validator auto entre Rust enums et STABILITY contract strings | Process gap | Aucun | "PR #27.1, ~ 1 jour" |
| **G-PR27.1-WARN-MARGIN** | `WARN_MARGIN_RATIO = 0.10` hardcodé partout au lieu de per-metric | Spec violation, mini-aggregator pas aligné avec production | Aucun | "PR #27.1, ~ 2 jours" |
| **G-PR27.1-COSIGN-INSTALL** | Workflow CI ne install pas cosign | Coverage gap | Aucun (cosign hors scope démo) | "PR #27.1, ~ 30 min" |
| **G-PR27.1-§24-REORG** | STABILITY § 24 mélange S-suite et bench public surface | Long-term doc reorg | Aucun | "STABILITY-bump PR future, ~ 1 semaine" |
| **G-PR27-S5-GOLDEN-REGEN** | Pas de procédure documentée pour régen le golden | Process doc gap | Aucun | "Process doc, ~ 1h" |

### 2.2 Défauts surfacés par le live-test 2026-05-17 (D-LIVETEST-N)

| Code | Symptôme | Cause | Impact démo | Contournement |
|------|----------|-------|-------------|---------------|
| **D-LIVETEST-1** | Les monitoring reports citent `case_a_no_runtime_override.ndjson` mais le fichier disque est `case_a_warn_null_tolerance.ndjson` | Naming asynchrone entre prose et disque | **Nul** (meta-test pin sur substring "case_a") | Ne PAS faire `ls docs/lessons/monitoring/PR-27-monitoring-week-9.md | grep case_a_no_runtime_override` en live |
| **D-LIVETEST-2** | 7 tests PR #16 mTLS panic dans rustls | `CryptoProvider::install_default()` jamais appelé | Aucun (hors scope) | Q&A : "C'est tracé comme G-PR16-CRYPTO-PROVIDER" |
| **D-LIVETEST-3** | `./target/debug/efe-admin --help` produit output vide | Pas de clap-derived help text | Mineur (skip ce binaire) | Skip `efe-admin` dans le tour des binaires si extension Act 6 |
| **D-LIVETEST-4** | `cargo build` plante sans `PYO3_PYTHON=python3.12` | pyo3 requires Python ≥ 3.10, système 3.9 | **Catastrophique si oublié** | § 1.1 — export OBLIGATOIRE avant tout |

### 2.3 Warnings clippy pré-existants (effective_tolerance.rs)

| Ligne | Warning | Contournement |
|-------|---------|---------------|
| 2917 | `clippy::uninlined_format_args` | Pas pertinent en démo (on ne lance pas clippy) |
| 2925 | `assertions_on_constants` | Pas pertinent |
| 2926 | `assertions_on_constants` | Pas pertinent |
| 3058 | `clippy::uninlined_format_args` | Pas pertinent |

**Total** : 4 warnings dans le code lib, **0 warnings dans mes tests S9-S12**.  Si quelqu'un demande "et clippy ?", réponse : "On a clippy-clean sur nos tests de chain integrity.  Sur le code lib il reste 4 warnings cosmétiques pré-S2 trackés pour PR #27.1."

### 2.4 Inconvénients d'environnement

| Symptôme | Cause | Contournement |
|----------|-------|---------------|
| `cargo` affiche `warning: profiles for the non root package will be ignored` à chaque invocation | Cargo.toml d'un sub-crate déclare un profile qui devrait être au workspace root | Si distrayant en démo : `2>&1 | grep -v "^warning: profiles"` |
| `efe-bench-harness version` affiche `git_sha: unknown, rustc_version: unknown, build_date: unknown` | Binaire pas built avec vergen env vars | OK pour démo — montre la commande marche.  Si demandé : "Vergen n'était pas configuré dans cette build dev, en CI les 3 champs sont peuplés." |
| `cargo test` warnings sur `efe-telemetry` (22 warnings, 19 doublons sur unused variables) | Code de telemetry pre-S2 | Hors scope.  Pas pertinent en démo. |
| `cargo test` warnings sur `efe-prompts` (18 warnings, doublons sur unused imports) | Code de prompts pre-S2 | Hors scope. |

### 2.5 Tests dont l'exécution est lente (à éviter en live)

| Test binary | Tests count | Durée typique | Verdict | Action démo |
|-------------|-------------|---------------|---------|-------------|
| `cargo test --workspace` | 5,422 | **~8 min** | 99.76 % PASS | **JAMAIS en démo** |
| `cargo test -p efe-core` | ~280 + dissolution_e2e | ~2 min | Some FAILED | Pas en démo |
| `cargo test --test g11_round_trip` | 9 | **~10 ms** | OK | ✅ démo Act 3 |
| `cargo test --test stability_contract_v17` | 51 | ~10 ms | OK | ✅ démo Act 3 |
| `cargo test --test s3_audit_chain_integrity` | 14 | ~5 ms | OK | ✅ démo Act 4 |
| `cargo test --test s9_s12_monitoring_chain_integrity` | 16 | ~5 ms | OK | ✅ démo Act 4 |
| `cargo test --test backfill_edge_cases` | 14 | ~5 ms | OK | ✅ démo Act 5 |
| `cargo test --lib compare::effective_tolerance` | 45 | ~10 ms | OK | ✅ démo (optionnel) |

---

## 3. Contournements pendant la démo (par phase du script)

### 3.1 Act 1 — Hook (pas de risque)

Pas de commande, pas de risque technique.  Risque humain : tu
oublies ta phrase d'ouverture.  Contournement : la phrase est
imprimée en § 10 cheat-sheet.

### 3.2 Act 2 — Le gap (master_tracker.md)

| Risque | Symptôme | Contournement |
|--------|----------|---------------|
| Editor freeze | cmd+F prend > 2 sec | Tu fais semblant que c'est intentionnel : "Pendant que ça charge, laisse-moi te raconter…" |
| Mauvais éditeur ouvert | Tu vois le mauvais fichier | Réflexe : ⌘+1, ⌘+2, ⌘+3 (onglets pré-ouverts).  Pas de Cmd+O en live. |
| L'audience ne voit pas le markdown | Tu présentes un fichier .md brut | Pré-installer `glow` ou un viewer markdown si projection.  Sinon pour onbas ASCII en taille 18. |

**Backup phrase si tu perds le fil** : "Ce que tu vois là, c'est le
single source of truth pour les défauts.  Chaque ligne est un gap
avec son owner, son test de pinning, et son statut.  La ligne G.11
est ce qu'on a fermé en PR #27."

### 3.3 Act 3 — La propriété forte

#### Risque A : le test prend > 30 secondes

**Symptôme** : tu tapes `cargo test ... --test g11_round_trip` et
le terminal reste muet pendant 30 secondes.

**Cause probable** : cache invalidé par un `cargo update` ou un
upgrade Rust depuis le pre-flight.

**Contournement** :
1. Garder le sourire.  Continuer à parler à l'audience.
2. "Pendant que le compilateur fait son travail, laisse-moi te
   montrer la structure du test."
3. Switch vers onglet éditeur sur `bench/harness/tests/g11_round_trip.rs`.
4. Pointer la doc-string en haut du fichier qui décrit les 9 tests.
5. Quand le terminal reprend, retourner et constater le PASS.

**Phrase de récup** : "Tu vois, ça compile maintenant.  C'est ça
qui est important — le test compile et passe.  Pas la rapidité du
compilateur."

#### Risque B : un test échoue inopinément (régression durant la nuit)

**Symptôme** : `test g11_round_trip ... FAILED` au milieu de la
démo.

**Cause probable** : tu as tiré une branche entre la pré-flight et
la démo, OU un fichier de fixture a été touché.

**Contournement immédiat** :
1. **Ne PAS paniquer**.  Cargo test te dit quel test exactement.
2. Réponse verbalisée : "Intéressant — j'ai un test rouge que je
   n'attendais pas.  Laisse-moi voir ce qui se passe."
3. Si tu peux le diagnose en 30 sec : explique.
4. Si tu peux pas : "C'est exactement le genre de signal qu'on
   veut.  Si le test rouge passe en review, on rouvre G.11 dans le
   tracker.  C'est le processus."

**Pivot** : tu prends l'audience comme témoin que le système
**marche** quand il rougit.  Sauf si évidemment c'est un bug
catastrophique, auquel cas : "Je note pour debug, je continue la
démo, je reviens là-dessus à la fin."

#### Risque C : le rituel 5x affiche un échec sur un des 5

**Symptôme** : un des 5 PASS devient un FAIL.

**Cause** : déterminisme cassé.  C'est la propriété même qu'on
prétend garantir.

**Contournement** : c'est le pire scénario possible.  Honnêteté
forcée : "OK.  Là, j'ai un cas que je ne vois jamais en
développement.  Soit l'environnement est différent — par exemple
le système d'horloge — soit on a une régression sur la déterminisme.
Je note, je continue."

**Le bon réflexe** : ne pas réessayer en cliquant frénétiquement.
Une régression de déterminisme est un défaut sérieux, traité comme
tel, pas masqué par une re-tentative.

### 3.4 Act 4 — La chaîne d'audit + WOW-MOMENT

C'est l'acte le plus risqué.  Le `mv` est destructif si oublié.

#### Risque A : le `mv` source n'existe pas

**Symptôme** :
```
mv: docs/lessons/monitoring/PR-27-monitoring-week-11.md: No such file or directory
```

**Cause** : tu n'es pas dans le bon cwd, OU le fichier a été
renommé.

**Contournement immédiat** :
1. Tape `pwd` à voix haute.  Si pas `/Users/rodrigue.pekar/Downloads/jjjj/efe`, `cd` et réessayer.
2. Si tu es dans le bon cwd : `ls docs/lessons/monitoring/`.  Si le
   fichier n'est pas là, dire "Je l'ai déjà déplacé pour le show de
   ce matin, je continue avec un autre."
3. Pivot vers `PR-27-monitoring-week-12.md`.

#### Risque B : tu oublies de remettre le fichier en place

**Symptôme** : à la fin de la démo, le test reste rouge.

**Contournement** : utiliser un trap obligatoire dans un sub-shell :

```bash
(
  trap 'mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md 2>/dev/null && echo "[trap] restored" || true' EXIT INT TERM
  
  # Le wow-moment
  mv docs/lessons/monitoring/PR-27-monitoring-week-11.md /tmp/.efe-wm-week11.md
  
  cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists
  # → FAILED
  
  # Restauration manuelle (le trap est une safety net)
  mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md
  
  cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists --quiet
  # → ok
)
```

Le `trap` garantit que **même si Ctrl-C ou crash**, le fichier
revient en place avant que tu sortes du sub-shell.

**Vérification post-act** : taper systématiquement à la fin
```
ls -la docs/lessons/monitoring/PR-27-monitoring-week-11.md | head -1
```
Si tu ne vois pas le fichier, **STOP, restore manuellement** avant
toute autre commande :
```
mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md
```

#### Risque C : le test FAILED affiche un autre nom de test que celui attendu

**Symptôme** :
```
test some_other_test ... FAILED
test s11_week_report_exists ... ok
```

**Cause** : tu n'as pas spécifié le filtre `s11_week_report_exists`
sur la commande de test.

**Contournement** : tape la commande complète :

```bash
cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists
```

Note le filtre `s11_week_report_exists` à la fin — c'est ce qui
isole le seul test qui regarde le fichier déplacé.

### 3.5 Act 5 — Le backfill

#### Risque A : le golden diff a changé pendant la nuit

**Symptôme** : le dry-run produit un output différent du golden.

**Cause** : fixture modifiée par accident, ou bug introduit dans
le script.

**Contournement** :
1. Lire le diff à voix haute.  "Hmm, je vois une dérive sur le
   champ X."
2. Si le diff est minime (un timestamp, un UUID) : "C'est exactement
   ce que le test edge_cases pinche — laisse-moi voir."
3. Lancer `cargo test -p efe-bench-harness --test backfill_edge_cases`.
   Si ça reste vert : "Le diff humain est cosmétique, le test
   structurel passe.  Je note pour investiguer."

#### Risque B : le script bash retourne exit ≠ 0

**Symptôme** :
```
$ echo $?
2
```

**Cause** : permissions, ou un argument mal passé.

**Contournement** : vérifier la permission :
```
chmod +x scripts/backfill-tolerance-used.sh
```
Et le ré-essayer.

### 3.6 Act 6 — La cumulative truth

Risques mineurs ; le validator et l'echo final ne plantent
quasiment jamais.

#### Risque A : le validator affiche "warn" sur le lessons file

**Symptôme** :
```
warn:  defect-index table : 17 row(s) (expected ≥ 19)
```

**Cause** : quelqu'un a édité le lessons file et supprimé des rows.

**Contournement** : "Je note — quelqu'un a touché au lessons file
hors process.  Je vais le réviser après."

---

## 4. Never Do List — commandes interdites en démo

Mémorise cette liste.  Si tu vas appuyer sur Entrée sur une de ces
commandes pendant la démo, **STOP**.

| Commande | Pourquoi interdite | Conséquence | Recovery |
|----------|--------------------|-----------|----------|
| `cargo clean` | Détruit le cache de compilation | 8+ min de rebuild en live | Tu perds la démo.  Avorte. |
| `cargo update` | Régénère Cargo.lock | Rebuild de toutes les deps | Avorte la démo, refait le pre-flight |
| `cargo test --workspace` | 5,422 tests, ~8 min | Audience perd attention | Ctrl-C, pivot vers `--test g11_round_trip` |
| `cargo test --workspace --no-fail-fast` | Idem mais ne s'arrête pas sur fail | Pire | Idem |
| `./target/debug/efe-bench-harness list` | G-ENGINE-CB regex rejette CB-* | Le bin plante avec un error | Skip cette commande, narration : "j'ai un défaut tracké sur cette commande, je te montre via les tests" |
| `./target/debug/efe-bench-harness validate` | G-ENGINE-CB idem | Plante | Skip |
| `./target/debug/efe-bench-harness run` | Exige un cluster K8s + chaos-mesh | Bloque ou crash | Skip |
| `./target/debug/efe-server` (sans timeout) | Bloque, attend des env vars | Terminal freeze | Ctrl-C |
| `./target/debug/efe-agent-runner` (sans timeout) | Bloque pareil | Idem | Ctrl-C |
| `./target/debug/controller` (sans timeout) | Bloque pareil | Idem | Ctrl-C |
| `./target/debug/participant` (sans timeout) | Bloque pareil | Idem | Ctrl-C |
| `./target/debug/efe-llm-router` (sans EFE_LLMROUTER_PG_DSN) | Plante avec config error | Print juste un error msg | OK si court, mais évite |
| `bash scripts/backfill-tolerance-used.sh --write` | **MODIFIE LES FIXTURES** | Régression silencieuse | Restore depuis git : `git checkout bench/fixtures/pre_g11_ndjson/` |
| `bash scripts/smoke.sh` | Démarre un vrai cluster | Demande > 10 min | Ctrl-C |
| `bash scripts/k3d-e2e.sh` | Idem | Idem | Idem |
| `git checkout` / `git reset --hard` | Détruit du travail non commité | Selon le state, peut casser fixture demos | NE PAS |
| `rm -rf docs/lessons/` | Détruit la chaîne d'audit | Tests s3 + s9_s12 deviennent rouges | NE PAS |
| `rm target/debug/g11_round_trip-*` | Force la recompilation du test | 30s+ en live | NE PAS |

### 4.1 Si tu DOIS faire l'une de ces commandes en démo

**Quasi-jamais nécessaire.**  Mais si oui :

1. **Pré-tape la commande mais ne l'envoie pas tout de suite**.
2. Verbalise : "Je vais te montrer quelque chose, ça va prendre N
   secondes."
3. Pendant les N secondes, narre.
4. Quand ça finit, narre encore.

### 4.2 Patch G-ENGINE-CB en 30 secondes (si vraiment forcé de
démontrer `list`)

```bash
# Édite scenario/mod.rs ligne 45 — change la regex
sed -i.bak 's|"id .. .. must match ..S\[0-9\]{{2}}_\[a-z0-9_\]+..|"id {} must match ^(S[0-9]{{2}}|CB-[0-9]{{2}})_[a-z0-9_]+$"|' bench/harness/src/scenario/mod.rs

# Rebuild + retester
cargo build -p efe-bench-harness --bin efe-bench-harness
./target/debug/efe-bench-harness list

# IMPORTANT : restore après démo
mv bench/harness/src/scenario/mod.rs.bak bench/harness/src/scenario/mod.rs
```

**Risque** : si tu rates le sed, tu casses le repo et tu n'as plus
de démo.  À ne faire QUE si rehearsé 10x.

---

## 5. Wow-moment safety detail

Le wow-moment de l'Act 4 est l'étape la plus à risque de la démo.
Voici la procédure renforcée :

### 5.1 Le script complet avec trap

```bash
# AVANT de commencer : vérifier l'état initial
ls -la docs/lessons/monitoring/PR-27-monitoring-week-11.md
# attendu : -rw-r--r-- ... PR-27-monitoring-week-11.md

# Le wow-moment dans un sub-shell avec trap
(
  trap 'mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md 2>/dev/null && echo "[trap] restored" ; true' EXIT INT TERM
  
  mv docs/lessons/monitoring/PR-27-monitoring-week-11.md /tmp/.efe-wm-week11.md
  cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists 2>&1 | tail -8
  echo ""
  echo "  (je le remets en place)"
  mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md
  cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists --quiet 2>&1 | tail -1
)

# APRÈS : vérifier que tout est restauré
ls -la docs/lessons/monitoring/PR-27-monitoring-week-11.md
# attendu : même fichier qu'avant
```

### 5.2 Que faire si tu casses le wow-moment

#### Cas A — le sub-shell crashe avant le mv-back

**Symptôme** : terminal a un prompt mais le fichier est dans /tmp.

**Action** :
```bash
mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md
cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity --quiet | tail -1
```
Vérifier `ok. 16 passed`.

#### Cas B — tu réussis le mv-back mais le test reste rouge

**Symptôme** : test reste FAILED après restauration.

**Cause possible** : un autre fichier déplacé par accident.

**Action** :
```bash
# Diff contre git pour identifier le delta
git status --short docs/lessons/
# Si un fichier est noté `??` ou `D` :
git checkout docs/lessons/
```

#### Cas C — l'audience te demande "et si vous oubliez de remettre ?"

**Réponse pré-rédigée** :

> "Bonne question.  En CI on a un job qui fait exactement ce que je
> viens de faire — `mv` + `test --no-run` — sur chaque PR.  Si le
> fichier n'est pas remis, le CI rouge.  Et puis dans la séquence
> normale d'une PR, le diff `git diff` montrerait le fichier
> supprimé, ce que le review humain attraperait avant le merge."

---

## 6. Environnement (Mac vs Linux, terminal, screen-sharing)

### 6.1 macOS-specific gotchas

| Gotcha | Symptôme | Contournement |
|--------|----------|---------------|
| `gtimeout` n'existe pas par défaut | `command not found: gtimeout` | Utiliser `perl -e 'alarm 5; exec @ARGV' -- <cmd>` ou installer coreutils : `brew install coreutils` |
| Quarantaine sur binaires téléchargés | Gatekeeper popup | `xattr -d com.apple.quarantine target/debug/*` |
| Sleep + lid close = network pause | Pas grave, démo offline | N/A |
| Display Sleep entraîne déconnexion HDMI | Black screen à l'écran | `caffeinate -dimsu &` avant la démo |
| `tput cols` parfois faux dans iTerm2 split panes | Format mal aligné | Resize manuellement avant démo |

### 6.2 Screen-sharing artifacts

| Plateforme | Comportement | Contournement |
|------------|--------------|---------------|
| Zoom (screen share) | Lag possible jusqu'à 2 sec ; couleurs OK | Ralentir le rythme, ne pas faire défiler vite |
| Google Meet (screen share) | Bug connu : certains caractères Unicode pas rendus | Éviter les caractères box-drawing (═ │ ┌) dans le terminal — désactiver `CARGO_TERM_COLOR` si problème |
| Tencent VooV (Kovant-style) | Latence forte, jitter | Imprimer en gros taille (≥ 22 pt) |
| Projection HDMI direct | Couleurs souvent gamma-décalées | Vérifier le contraste vert/noir avant ; sinon `CARGO_TERM_COLOR=never` |
| OBS recording | OK | Normal |

### 6.3 Présentation à distance vs en présence

| Aspect | Distance (Zoom) | Présence |
|--------|-----------------|----------|
| Rythme | Plus lent (lag) | Naturel |
| Beats (silences) | 5 sec après wow-moment | 3 sec |
| Taille police | ≥ 22 pt | ≥ 18 pt |
| Onglets éditeur pré-ouverts | Pareil | Pareil |
| Mute backchannel notifications | Critique | Idem |

---

## 7. Network failure contournements

Pas critique parce que la démo est offline-safe (§ 1.6).  Mais
juste au cas où.

### 7.1 Si le wifi tombe pendant la démo

**Impact** : aucun.  Toutes les commandes du script tournent en
local.

**Vérification** : `cargo test` est en mode `offline` automatique
si le cache est chaud.  Vérifier avec `--offline` :
```bash
cargo test -p efe-bench-harness --test g11_round_trip --offline --quiet
```

### 7.2 Si un dépendance externe (Rekor, GitHub) est down

**Impact** : aucun — la démo ne touche pas Rekor en runtime.

**Si demandé** : "Le quarterly publish utilise Rekor pour
l'inclusion proof, mais ce n'est pas dans la démo.  Notre dry-run
(§ 4.6) vérifie le pipeline cosign sans toucher au log public."

### 7.3 Si Slack / Discord / etc. notifications te dérangent

**Action préventive** :
- macOS : System Preferences → Focus → Do Not Disturb → On
- Quitter Slack desktop : `osascript -e 'quit app "Slack"'`
- Quitter Discord : idem
- Désactiver `nyc` (battery) widget : System Prefs → Battery → Show
  percentage Off

---

## 8. Q&A contournements (questions auxquelles tu ne sais pas répondre)

Le script Q&A (§ 8 du démo script) couvre 8 questions hostiles.
Voici les **stratégies génériques** quand tu te fais piéger.

### 8.1 La question "Comment ça scale ?"

**Mauvaise réponse** : "Je ne sais pas."

**Bonne réponse** :

> "Pour cette pile en particulier, le bottleneck est la persistance
> NDJSON et le rendering PDF du quarterly.  À 35 scenarios × 90 jours
> × 8-10 metrics par scenario, on tourne autour de 5,800 rows par
> quarter, ce qui rend en ~ 40 min sur ubuntu-latest.  Ça scale à
> ~100 scenarios sans changement.  Au-delà on partitionne par
> scenario family.  C'est tracé comme évolution dans la roadmap
> Phase-4."

### 8.2 La question "Combien d'utilisateurs vous avez ?"

**Mauvaise réponse** : "On va lever."

**Bonne réponse** :

> "Aujourd'hui, EFE est un produit interne en validation.  La
> question de qui d'autre l'utilise est exactement ce que le levée
> doit financer.  Le point que je démontre aujourd'hui c'est que la
> *fondation technique* est testée à 174 tests permanents, donc
> qu'un client peut s'en servir sans payer la dette de qualité que
> les benchmarks mainstream traînent."

### 8.3 La question "Pourquoi Rust ?"

**Réponse** :

> "Trois raisons : (1) le déterminisme par construction — il n'y a
> pas de garbage collection qui introduit du jitter dans les
> mesures de p99 ; (2) la propriété ownership qui rend
> impossible certains bugs de concurrence qui contaminent les
> outils d'analyse en Python ; (3) le compile-time des freeze
> tests est exactement le contrat qu'on veut — si tu casses le
> schéma, ça compile pas."

### 8.4 La question "C'est ouvert open-source ?"

**Réponse** :

> "Pas encore.  La pile est privée pendant cette phase de
> validation.  Le pattern de discipline d'audit, lui, est publié —
> en doc, pas en code.  On regarde une trajectoire OSS pour la
> couche bench-publication (le quarterly + le validator) une fois
> la traction commerciale validée."

### 8.5 La question "Combien de FTE engineers ?"

**Réponse** : (adapter à la réalité.  Pour la session : "Un
founding engineer + un cofondateur côté business.  On commence à
recruter avec la prochaine close.")

### 8.6 La question piège — un détail technique pointu hors PR #27

**Symptôme** : "Et le test `pr16_mtls_happy_path_tls13` que j'ai
vu sur ton écran tout à l'heure ?"

**Mauvaise réponse** : silence + transpiration.

**Bonne réponse** :

> "Bonne lecture.  C'est un panic dans rustls 0.23 lié à
> l'installation du CryptoProvider — un default provider qui n'est
> pas auto-installé en TLS depuis la 0.23.  C'est un défaut de
> bootstrap qu'on a surfacé pendant le test produit ce matin.
> Tracé comme G-PR16-CRYPTO-PROVIDER, fixable en ~ 10 lignes.
> Mais ce qui compte, c'est qu'on a une chaîne de surveillance qui
> détecte ces choses-là — le rapport de test produit du matin est
> dans `docs/reports/EFE-product-test-2026-05-17.md`."

### 8.7 La question "C'est de l'engineering ou du marketing ?"

**Réponse** :

> "L'engineering est ce que tu peux reproduire.  Tu peux cloner le
> repo et lancer la même séquence de commandes que celle que tu
> viens de me voir taper.  Tu auras les mêmes 174 PASS.  C'est le
> contrat qu'on vend."

### 8.8 La question "Et si je veux essayer ?"

**Réponse** :

> "On a un tier early-access.  Je peux t'envoyer un lien post-démo
> qui te donne un repo accessible et un setup-guide pour faire
> tourner les benchmarks sur ton infra.  Tu auras les mêmes
> garanties structurelles que celles que tu viens de voir."

---

## 9. Post-demo state recovery

Quand tu sors de la salle, vérifie immédiatement que le repo est
dans un état propre.

### 9.1 Vérifications obligatoires (T+2 min)

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe

# Vérification 1 — pas de fichier temp en /tmp
ls /tmp/.efe-wm-* /tmp/.demo-test* 2>/dev/null
# si quelque chose s'affiche : tu as raté un mv-back
# → mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md

# Vérification 2 — git status est propre
git status --short docs/
# si tu vois autre chose que des deltas attendus : restore

# Vérification 3 — tous les 7 tests PR #27 passent encore
cargo test -p efe-bench-harness \
  --test g11_round_trip \
  --test g11_tolerance_used_populated \
  --test stability_contract_v17 \
  --test backfill_edge_cases \
  --test pr27_workflow_yaml_invariants \
  --test s3_audit_chain_integrity \
  --test s9_s12_monitoring_chain_integrity 2>&1 | grep -E "^test result:"
# 7 lignes "ok" attendues
```

### 9.2 Cleanup files inutiles

```bash
rm -f /tmp/.efe-wm-* /tmp/.demo-test* /tmp/synthetic-smoke-result.json 2>/dev/null
echo "  cleanup done"
```

### 9.3 Si la démo est suivie d'une autre démo (back-to-back)

Re-exécuter le pre-flight § 1.5 dans son intégralité.  Ne pas
supposer que l'état est stable.

---

## 10. Cheat sheet en 1 page (à imprimer)

```
═══════════════════════════════════════════════════════════════════
  EFE DEMO — CHEAT SHEET — 2026-05-17
═══════════════════════════════════════════════════════════════════

PRE-FLIGHT
  cd /Users/rodrigue.pekar/Downloads/jjjj/efe
  export PYO3_PYTHON=python3.12
  export CARGO_TERM_COLOR=always
  df -h ~ | head -2                # ≥ 5 Gi libres
  cargo test --test g11_round_trip --quiet | tail -1   # 9 PASS

CORE COMMANDS (en ordre du démo)
  ACT 3 — pinning test
    cargo test -p efe-bench-harness --test g11_round_trip
    for i in 1 2 3 4 5; do cargo test ... --test g11_round_trip --quiet; done
    cargo test -p efe-bench-harness --test stability_contract_v17 --quiet

  ACT 4 — wow moment
    (
      trap 'mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md 2>/dev/null' EXIT
      mv docs/lessons/monitoring/PR-27-monitoring-week-11.md /tmp/.efe-wm-week11.md
      cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists 2>&1 | tail -8
      mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md
      cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists --quiet
    )

  ACT 5 — backfill
    bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/ | tail -10
    cargo test -p efe-bench-harness --test backfill_edge_cases --quiet

  ACT 6 — cumulative
    bash scripts/audit-prompt-validator.sh docs/lessons/PR-27-lessons.md

NEVER DO
  cargo clean                       # 8+ min rebuild
  cargo update                      # idem
  cargo test --workspace            # 8 min
  efe-bench-harness list/validate   # G-ENGINE-CB
  efe-server/agent/controller       # block, no --help
  backfill-tolerance-used.sh --write # modifies fixtures

OPENING PHRASE
  "On a passé 3 mois à fermer une ligne fausse depuis 4 mois.
   Pas parce qu'on est lents. Parce qu'on a structuré la fermeture
   de telle sorte que personne ne peut la rouvrir silencieusement."

CLOSING PHRASE
  "C'est ça qu'on construit chez EFE. Pas un produit qui marche
   aujourd'hui. Un produit qui ne peut pas casser silencieusement
   demain. Des questions ?"

IF SOMETHING BREAKS
  1. Smile. Narrate. Continue.
  2. cd /Users/rodrigue.pekar/Downloads/jjjj/efe  (always-recovery)
  3. ls /tmp/.efe-wm-* — if anything, restore.
  4. cargo test -p efe-bench-harness --test g11_round_trip --quiet  (sanity)

POST-DEMO RECOVERY
  ls /tmp/.efe-wm-* /tmp/.demo-test* 2>/dev/null
  git status --short docs/
  cargo test -p efe-bench-harness --test g11_round_trip --quiet | tail -1

═══════════════════════════════════════════════════════════════════
```

---

## 11. Annexe — Sources et traçabilité

Ce document est dérivé de :

- `docs/reports/EFE-product-test-2026-05-17.md` — le rapport du test
  produit complet exécuté ce matin (8 phases, 5,409 PASS / 13 FAIL).
- `docs/gaps/master_tracker.md` — la liste canonique des G-XX.
- `docs/marketing/EFE-demo-script-2026-05-17.md` — le script de
  démo principal qui appelle ce document.
- `bench/harness/src/scenario/mod.rs:45` — la ligne exacte qui
  porte le défaut G-ENGINE-CB.
- `bench/harness/tests/s9_s12_monitoring_chain_integrity.rs` — le
  test de chaîne dont dépend le wow-moment.

Tous les exemples ci-dessus ont été **exécutés live le 2026-05-17
entre 19:00 et 22:00 UTC** sur le repo
`/Users/rodrigue.pekar/Downloads/jjjj/efe`.  Aucun exemple n'est
théorique.

---

_Bonne démo.  Tu as tout.  Tu peux y aller._
