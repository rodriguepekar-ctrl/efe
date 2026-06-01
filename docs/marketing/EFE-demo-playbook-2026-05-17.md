# EFE Demo — Playbook séquentiel (master action list)

**Date drafted** : 2026-05-17
**Owner**        : @bench-infra-lead (toi)
**Statut**       : LIVE
**Position**     : ce doc est la **synthèse opérationnelle** des 4
                   autres docs.  À consulter quand tu veux savoir
                   "qu'est-ce que je fais maintenant ?".

**Pack démo total** :

| Doc | Rôle | Quand |
|-----|------|-------|
| 📒 `EFE-demo-script-2026-05-17.md` (857 l) | Narration + 6 acts | Pendant démo |
| 🛟 `EFE-demo-contournements-2026-05-17.md` (897 l) | Recovery | Si X casse |
| ⚙️ `EFE-demo-environnement-2026-05-17.md` (1043 l) | Setup | Avant démo |
| 🤖 `scripts/efe-demo-preflight.sh` (270 l) | Auto-check | T-30 min |
| 📋 **Ce doc** (~ 400 l) | **Quoi faire et quand** | **Maintenant** |

---

## 0. Vue d'ensemble — les 7 phases

```
═══════════════════════════════════════════════════════════════════
  EFE DEMO — TIMELINE
═══════════════════════════════════════════════════════════════════

  Phase 1   Cette semaine          (~ 4-6 h cumulées)
  Phase 2   Veille (T-24h)         (~ 1 h)
  Phase 3   Matin J (T-day → T-2h) (~ 1 h)
  Phase 4   Sur site (T-60 → T-0)  (~ 1 h)
  ─────────────────────────────────────────────
  Phase 5   PENDANT démo (T-0 → T+15)
  ─────────────────────────────────────────────
  Phase 6   Immédiat après (T+5 → T+30)
  Phase 7   Lendemain (T+24h)

═══════════════════════════════════════════════════════════════════
```

### 0.1 Le test d'arrêt absolu

À n'importe quel moment, si une de ces 3 conditions devient fausse,
**AVORTE la démo et reschedule** :

1. `bash scripts/efe-demo-preflight.sh` retourne PRE-FLIGHT FAIL.
2. Les 7 tests PR #27 scope ne sont pas tous verts.
3. Tu ne te rappelles plus de la phrase d'ouverture.

Reschedule, debug hors scène, recommence le pre-flight.

---

## Phase 1 — Cette semaine

### 1.1 DÉCIDER (15 min, à faire EN PREMIER)

- [ ] **Public** : YC partners / Kovant / customer / demo day ?
- [ ] **Format** : en présence ou distance (Zoom/Meet/VooV) ?
- [ ] **Slot** : 10 min / 15 min / 30 min / 45 min ?
- [ ] **Date cible** : préciser jour + heure UTC
- [ ] **Backup date** : prévoir +7 jours au cas où

**Implication des choix** :

| Slot | Adaptation démo |
|------|-----------------|
| 5 min (demo day) | Acts 3 + 4 only, skip Act 2 + 5 + 6 |
| 10 min (YC call) | Hook réduit (45 s) + Acts 3 + 4 + closing |
| 15 min (Kovant) | Full 14:30 + 1 min Q&A buffer |
| 30 min | Full + Extensions § 11 (STABILITY, quarterly) + 15 min Q&A |
| 45 min | + Founder narrative en intro |

### 1.2 RÉPÉTITION 1 (60 min, T-week)

- [ ] Lire les 3 docs (script + contournements + environnement) **en entier**
- [ ] Apprendre par cœur :
  - [ ] La phrase d'ouverture
  - [ ] La phrase de closing
  - [ ] Les 8 réponses Q&A du contournements § 8
- [ ] Faire la démo seul, à voix haute, sans script — chronométrer
- [ ] Si tu finis en > 16 min : couper.  Si < 12 min : ralentir.

### 1.3 RÉPÉTITION 2 (60 min, T-week)

- [ ] Refaire la démo, **filmer avec téléphone**
- [ ] Te regarder : tics verbaux, posture, regards
- [ ] Corriger les transitions buguées
- [ ] Re-chronométrer

### 1.4 RÉPÉTITION 3 (45 min, idéalement avec un témoin)

- [ ] Demander à un ami (technique mais pas tech-EFE) d'écouter
- [ ] Lui poser des questions à toi, comme un investisseur
- [ ] Identifier quels passages perdent l'audience
- [ ] Noter les nouvelles questions Q&A surgissantes

### 1.5 ARTEFACTS COMPLÉMENTAIRES (optionnel, selon besoin)

Pour chacun, demande-moi de le produire si tu veux :

- [ ] **Slide deck companion** (4 slides Title / Problem / Pattern / Ask) — si on te demande un deck
- [ ] **Follow-up email template** — pour l'envoi T+4h
- [ ] **One-pager EFE** — à laisser après la démo
- [ ] **Founder narrative structuré** — pour insertion en Q&A
- [ ] **FAQ étendue** (20+ questions au-delà des 8) — si tu veux te blinder davantage

### 1.6 LOGISTIQUE PRÉ-DÉMO (selon où elle a lieu)

- [ ] **En présence** : confirmer adresse + salle + horaire avec l'host
- [ ] **En présence** : prévoir transport + 30 min de buffer à l'arrivée
- [ ] **Distance Zoom** : tester la salle de réunion 24h avant avec ami
- [ ] **Distance Zoom** : envoyer le lien Zoom à toutes parties prenantes
- [ ] **Imprimer** : 3 cheat sheets (§ 24 de chaque doc, soit 3 pages A4)

---

## Phase 2 — Veille (T-24 h)

### 2.1 Pre-build complet (20 min)

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
export PYO3_PYTHON=python3.12

# Pull la dernière version stable (skip si pas de remote)
git pull --ff-only 2>/dev/null || echo "  (no remote pull)"

# Pre-build tous les binaires + tests utilisés en démo
cargo build -p efe-bench-harness --bin efe-bench-harness
cargo build -p efe-bench         --bin efe-bench

cargo test -p efe-bench-harness --no-run \
  --test g11_round_trip \
  --test g11_tolerance_used_populated \
  --test stability_contract_v17 \
  --test backfill_edge_cases \
  --test pr27_workflow_yaml_invariants \
  --test s3_audit_chain_integrity \
  --test s9_s12_monitoring_chain_integrity
```

- [ ] Pre-build OK
- [ ] Pre-compile tests OK

### 2.2 Pre-flight script complet (5 min)

```bash
bash scripts/efe-demo-preflight.sh
```

- [ ] PRE-FLIGHT PASS (ou seulement WARN, jamais FAIL)
- [ ] Si FAIL : debug ce soir, ne pas attendre demain matin

### 2.3 Hardware checklist veille (10 min)

- [ ] Laptop chargé à 100 %
- [ ] Téléphone chargé à 100 %
- [ ] Chargeur USB-C 96W dans le sac
- [ ] Adaptateur HDMI/USB-C dans le sac
- [ ] Câble HDMI ≥ 2 m dans le sac
- [ ] Backup laptop préparé (si applicable, cf environnement doc § 19)
- [ ] Power bank dans le sac

### 2.4 Mental rehearsal final (15 min)

- [ ] Re-lire le script à voix basse une fois
- [ ] Mémoriser :
  - [ ] Phrase d'ouverture
  - [ ] Phrase de closing
  - [ ] Comment lancer le wow-moment (Act 4)
  - [ ] La sortie attendue de chaque commande (tu dois savoir avant que ça apparaisse)
- [ ] Imprimer les 3 cheat sheets si pas déjà fait

### 2.5 Préparer le retour à la maison

- [ ] Petit déjeuner planifié (suffisant mais pas lourd)
- [ ] Réveil 2h avant le départ pour la démo
- [ ] Habits choisis et préparés

---

## Phase 3 — Matin J (T-day → T-2 h)

### 3.1 Réveil + routine (T-2 h 30 min)

- [ ] Réveil sans snooze
- [ ] Hydratation : 500 ml d'eau avant café
- [ ] Petit déjeuner léger
- [ ] **PAS** plus de café que d'habitude (anxiété)

### 3.2 Dernier dry-run dans la chambre (T-2 h)

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
export PYO3_PYTHON=python3.12 CARGO_TERM_COLOR=always CARGO_INCREMENTAL=0
bash scripts/efe-demo-preflight.sh
```

- [ ] PRE-FLIGHT PASS
- [ ] Lancer la démo complète en miroir mental, sans le terminal
- [ ] Vérifier le timing chrono : 12-14 min

### 3.3 Last checks (T-90 min)

- [ ] Battery laptop ≥ 90 %
- [ ] Téléphone ≥ 90 %
- [ ] Sac complet (cf 2.3)
- [ ] Cheat sheets imprimés dans le sac
- [ ] Itinéraire / lien Zoom vérifié

---

## Phase 4 — Sur site (T-60 → T-0)

### 4.1 T-60 minutes — Arrivée

**En présence** :
- [ ] Arriver à T-60 min sur place
- [ ] Saluer l'host, vérifier la salle est libre
- [ ] Repérer la prise électrique
- [ ] Brancher le laptop immédiatement
- [ ] Tester la projection :
  - [ ] Brancher HDMI
  - [ ] Vérifier "Mirror Displays"
  - [ ] Tester la lisibilité d'un `cargo test --quiet` à 5 m

**Distance (Zoom/Meet/VooV)** :
- [ ] Lancer le call à T-15 min en avance
- [ ] Tester audio + video avec backstage participant
- [ ] Vérifier l'écran qu'on va partager

### 4.2 T-30 minutes — Environnement

```bash
cd /Users/rodrigue.pekar/Downloads/jjjj/efe
export PYO3_PYTHON=python3.12
export CARGO_TERM_COLOR=always
export CARGO_INCREMENTAL=0
export RUST_BACKTRACE=1
export LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8

caffeinate -dimsu &
CAFFEINATE_PID=$!
echo "caffeinate PID: $CAFFEINATE_PID"

bash scripts/efe-demo-preflight.sh
```

- [ ] Env vars exportées
- [ ] caffeinate lancé
- [ ] Pre-flight PASS (ou WARN bénin)
- [ ] Focus mode "Demo" activé (Control Center)
- [ ] Slack/Discord/Mail/Calendar/Messages quittés (cf environnement doc § 3.2)

### 4.3 T-15 minutes — Editor + Terminal

- [ ] Ouvrir VS Code/Cursor avec les 5 fichiers :

```bash
code docs/gaps/master_tracker.md \
     docs/lessons/PR-27-lessons.md \
     docs/lessons/monitoring/PR-27-closure-2026-08-07.md \
     bench/harness/tests/g11_round_trip.rs \
     bench/harness/tests/s9_s12_monitoring_chain_integrity.rs
```

- [ ] Sur `master_tracker.md` : Cmd+F → "G.11"
- [ ] Désactiver Copilot autocomplete (extensions)
- [ ] Désactiver auto-save (settings)
- [ ] Terminal iTerm2 :
  - [ ] Profile "Demo" actif
  - [ ] Police ≥ 18 pt (22 pt si projection lointaine)
  - [ ] 120 col × 35 lines
  - [ ] `clear` puis vérifier vide

- [ ] Cheat sheets sortis du sac, posés près du laptop

### 4.4 T-5 minutes — Mental + audience

- [ ] Vérifier `ls docs/lessons/monitoring/PR-27-monitoring-week-11.md` → présent (wow-moment ready)
- [ ] Respirer 4-7-8 trois fois :
  - 4 sec inhale
  - 7 sec hold
  - 8 sec exhale
- [ ] Une gorgée d'eau
- [ ] Sourire à l'audience qui s'installe
- [ ] Présentation rapide : "On va parler de fermeture de gap d'engineering en 15 minutes."

### 4.5 T-0 — GO

Lance Act 1.  Tu connais la phrase d'ouverture :

> **"On a passé 3 mois à fermer une ligne de code qui était fausse depuis 4 mois.  Pas parce qu'on est lents.  Parce qu'on a structuré la fermeture de telle sorte que personne ne peut la rouvrir silencieusement.  C'est ce que je vais te montrer."**

---

## Phase 5 — PENDANT la démo (T-0 → T+15)

### 5.1 Séquence des 6 acts (réf script doc § 1-7)

| Time | Act | Action terminal |
|------|-----|-----------------|
| T+0:00 | Hook (90 s) | Aucune — debout, mains libres |
| T+1:30 | Le gap (2 min) | Cmd+1 (master_tracker.md), Cmd+F "G.11" |
| T+3:30 | Pinning + freeze (3 min) | `cargo test --test g11_round_trip` + boucle 5x + freeze |
| T+6:30 | ★ WOW MOMENT (3 min) | Trap-wrapped mv + test fail + restore |
| T+9:30 | Backfill (2 min) | `bash scripts/backfill-tolerance-used.sh --dry-run` |
| T+11:30 | Cumulative truth (2 min) | `bash scripts/audit-prompt-validator.sh` + tally |
| T+13:30 | Closing (1 min) | Lâcher le clavier, face audience |
| T+14:30 | Q&A | Aucune commande pré-tapée |

### 5.2 Le wow-moment (Act 4, le plus risqué)

**À taper exactement** :

```bash
(
  trap 'mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md 2>/dev/null && echo "[trap] restored"' EXIT INT TERM
  
  mv docs/lessons/monitoring/PR-27-monitoring-week-11.md /tmp/.efe-wm-week11.md
  cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists 2>&1 | tail -8
  
  echo ""
  echo "  (je le remets en place)"
  mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md
  cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists --quiet | tail -1
)
```

**Vérification post-act** : `ls -la docs/lessons/monitoring/PR-27-monitoring-week-11.md`

Si le fichier n'apparaît pas : **STOP, restore manuellement** :
```bash
mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md
```

### 5.3 Si une commande échoue inopinément

**Référence rapide** : contournements doc § 3.

Règle universelle :
1. **Sourire**.  Continuer à parler.
2. Si tu peux diagnose en 30 sec : explique.
3. Sinon : "C'est intéressant — note pour debug.  Je continue."
4. Pivot vers la phase suivante.

### 5.4 Q&A — 8 questions préparées

Référence : contournements doc § 8.

- Q1 : "Vous mettez 12 semaines à fermer un gap, c'est pas scalable."
- Q2 : "Vous avez 7 G-XX différés.  C'est de la dette."
- Q3 : "Le pinning test n'est qu'un cas particulier."
- Q4 : "Quels sont les 0.24 % rouges et pourquoi tu en parles pas plus ?"
- Q5 : "Combien de fondateurs ?"
- Q6 : "Vos clients vont devoir apprendre Rust ?"
- Q7 : "Vos checklists sont datées dans le futur — c'est du fake ?"
- Q8 : "Ça résout quoi pour le client ?"

Les réponses sont préparées verbatim.  Garde la cheat-sheet
contournements à proximité.

---

## Phase 6 — Immédiatement après (T+5 → T+30)

### 6.1 T+2 — Cleanup (encore dans la salle)

```bash
# Vérifier qu'aucun temp file en /tmp
ls /tmp/.efe-wm-* /tmp/.demo-test* 2>/dev/null

# Si quelque chose s'affiche :
mv /tmp/.efe-wm-week11.md docs/lessons/monitoring/PR-27-monitoring-week-11.md 2>/dev/null

# Re-vérifier que les 7 tests passent
cargo test -p efe-bench-harness --test g11_round_trip --quiet | tail -1
# attendu : ok. 9 passed
```

- [ ] Aucun temp file resté
- [ ] Tests PR #27 toujours verts

### 6.2 T+5 — Sortir de la salle

- [ ] Kill caffeinate : `kill $CAFFEINATE_PID`
- [ ] Désactiver Focus "Demo"
- [ ] Débrancher HDMI + ranger câbles
- [ ] Remercier l'host

### 6.3 T+30 — Note debrief (sur téléphone, dans un café)

- [ ] **3 questions hostiles** qu'on t'a posées (verbatim) :
  - 1.
  - 2.
  - 3.
- [ ] **Ta moins bonne réponse** :
- [ ] **3 trucs qui ont marché** :
- [ ] **3 trucs qui ont coincé** :
- [ ] **Signal d'intérêt** (de 1 à 10, et pourquoi) :
- [ ] **Next step demandé / suggéré** par l'audience :

### 6.4 T+2 h à T+4 h — Email de follow-up

À envoyer dans les 4 h après la démo, à toutes les personnes
présentes.  Inclure :

- Lien repo (si autorisé) ou screenshot du commit
- Lien vers `docs/reports/EFE-product-test-2026-05-17.md`
- Lien vers `docs/gaps/master_tracker.md`
- Phrase : "Tu peux reproduire la démo en clonant le repo et en
  lançant `cargo test -p efe-bench-harness --test g11_round_trip`."
- Réponse à toute question restée sans réponse pendant la Q&A
- CTA clair : "Veux-tu un follow-up call ?" / "Tu veux que je te
  partage [X spécifique demandé] ?"

Si tu veux que je rédige un template d'email, dis-moi.

---

## Phase 7 — Lendemain (T+24 h)

### 7.1 Debrief approfondi

- [ ] Re-lire tes notes de la salle
- [ ] Identifier 3 améliorations concrètes pour la prochaine démo
- [ ] Mettre à jour le contournements doc § 8 avec les nouvelles
      questions hostiles surgies
- [ ] Si un défaut a été surfacé par l'audience : créer une row
      dans master_tracker.md

### 7.2 Suivi business

- [ ] Si réponse positive : planifier le next step (term sheet,
      diligence, customer trial, etc.)
- [ ] Si réponse négative : tirer les leçons, NE PAS prendre
      personnel
- [ ] Si pas de réponse à T+72h : envoyer un soft follow-up
- [ ] Mettre à jour le pipeline CRM / pitch tracker

### 7.3 Préparer la prochaine démo

- [ ] Si une autre démo prévue dans la semaine : refaire ce playbook
- [ ] Si pas : noter ce qu'il faudra refaire la veille de la
      prochaine

---

## Annexe A — Decision trees

### A.1 Démo en présence vs distance

```
Présence ?
├── OUI → Suivre Phase 4.1 (en présence)
│         + Phase 4.3 (projection avec Mirror Displays)
│         + Posture debout, laptop sur table
│
└── NON → Suivre Phase 4.1 (distance)
          + Tester audio/video à T-15
          + Lancer le call à T-15
          + Caméra à hauteur des yeux
          + Background neutre
```

### A.2 Slot durée

```
Combien de temps ?
├── 5 min  → Acts 3 + 4 only, skip 2 + 5 + 6
├── 10 min → Hook 45s + Acts 3 + 4 + Closing
├── 15 min → Full démo 14:30 + buffer 30s
├── 30 min → Full + Extensions § 11 + Q&A 15 min
└── 45 min → + Founder narrative en intro + Full + Q&A
```

### A.3 Si pre-flight FAIL juste avant la démo

```
PRE-FLIGHT FAIL ?
├── Sur Battery only (warn) → Brancher AC, relancer pre-flight
├── Sur 7 tests PR#27 (FAIL) → AVORTER + reschedule
├── Sur 21 critical files (FAIL) → AVORTER + restore depuis git
├── Sur disk (FAIL) → Cleanup target/release, relancer
├── Sur cwd (FAIL) → cd /Users/rodrigue.pekar/Downloads/jjjj/efe
└── Sur env vars (FAIL) → re-export, relancer
```

### A.4 Si la machine primaire meurt en démo

```
Switch backup laptop (90 sec)
├── Annoncer "Pardon — je passe sur mon laptop backup."
├── Débrancher HDMI primaire
├── Rebrancher HDMI sur backup (déjà ouvert, déjà signé in)
├── Cmd+1 sur Terminal
├── cd /Users/rodrigue.pekar/efe-backup
└── Reprendre où tu étais
```

(Pré-requis : avoir fait le setup backup la veille, cf
environnement doc § 19.)

---

## Annexe B — Mini-checklist 1 page (à imprimer)

```
═════════════════════════════════════════════════════════════════════
  EFE DEMO PLAYBOOK — MINI-CHECKLIST
═════════════════════════════════════════════════════════════════════

  PHASE 1 — Cette semaine
  [ ] Décider public / format / date
  [ ] Répéter 2-3 fois (chronométrer + filmer)
  [ ] (Optionnel) demander slide deck / email template

  PHASE 2 — Veille
  [ ] cargo build + cargo test --no-run (~ 20 min)
  [ ] bash scripts/efe-demo-preflight.sh → PASS
  [ ] Hardware sac : charger, HDMI, câble, backup
  [ ] Mental rehearsal + imprimer cheat sheets

  PHASE 3 — Matin J
  [ ] Hydratation + petit déjeuner
  [ ] Dry-run final dans la chambre
  [ ] Battery ≥ 90 %

  PHASE 4 — Sur site (T-60 → T-0)
  T-60 : Arriver + brancher + tester projection
  T-30 : Export env vars + caffeinate + pre-flight script
  T-15 : Ouvrir 5 tabs éditeur + iTerm2 profile Demo
  T-5  : Vérifier wow-moment file + 4-7-8 breathing
  T-0  : GO

  PHASE 5 — Pendant (12-14 min)
  Hook → Gap → Pinning → ★ WOW ★ → Backfill → Cumul → Closing → Q&A

  PHASE 6 — Immédiat après (T+5 → T+30)
  [ ] ls /tmp/.efe-wm-* — restore si présent
  [ ] cargo test ... g11_round_trip — sanity check
  [ ] kill caffeinate + Focus off
  [ ] Notes debrief sur téléphone
  [ ] Email follow-up dans les 4h

  PHASE 7 — Lendemain
  [ ] Debrief 3 améliorations
  [ ] Mettre à jour contournements doc avec nouvelles Q&A
  [ ] Suivi business : next step

═════════════════════════════════════════════════════════════════════
```

---

## Annexe C — Référence rapide aux 4 docs

| Tu as besoin de... | Va dans... |
|---|---|
| Savoir quoi dire à l'Act 4 | `EFE-demo-script` § 4 |
| Restaurer le wow-moment file | `EFE-demo-contournements` § 5 |
| Configurer iTerm2 | `EFE-demo-environnement` § 10 |
| Vérifier que tout marche | `bash scripts/efe-demo-preflight.sh` |
| Répondre à "C'est de l'engineering ou du marketing ?" | `EFE-demo-script` § 8 Q7 |
| Patch G-ENGINE-CB si forcé | `EFE-demo-contournements` § 4.2 |
| Recovery battery faible | `EFE-demo-environnement` § 2.4 |
| Switch backup laptop | `EFE-demo-environnement` § 19 |
| Liste des 14 binaires testés | `docs/reports/EFE-product-test-2026-05-17.md` |
| État de G.11 | `docs/gaps/master_tracker.md` |
| Quoi rapporter sur l'audit | `docs/lessons/PR-27-lessons.md` |

---

_C'est ce playbook qui te dit, à n'importe quel moment de la
préparation, "qu'est-ce que je fais maintenant"._

_Bonne démo._
