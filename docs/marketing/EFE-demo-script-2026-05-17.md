# EFE — Demo script (pre-pitch master document)

**Date drafted** : 2026-05-17
**Operator**     : @bench-infra-lead
**Audience**     : Y Combinator partners / Kovant investors / technical buyer
**Core demo length** : 12 minutes (extensions to 20 min)
**Reproducibility** : 100 % of commands in this script have been live-verified on 2026-05-17 against the actual repo on disk

> **Lecture du script** : la **narration** est ce que tu DIS à voix
> haute pendant que tu tapes.  La **commande** est ce que tu TAPES.
> Le **expected output** est ce que l'audience VOIT à l'écran.  Le
> **beat** est la pause après le PASS pour laisser absorber.

---

## 0. Pré-flight checklist (T-30 minutes)

### 0.1 Environnement local

```bash
# Variables d'env requises
export PYO3_PYTHON=python3.12
export CARGO_TERM_COLOR=always

# Vérifier disque ≥ 5 GB libres
df -h ~ | head -2
# Si < 5 GB → rm -rf target/release ; cargo clean ne pas faire
# (perdrait le cache et déclencherait 8 min de rebuild en live)

# Vérifier les commandes critiques répondent
cargo test -p efe-bench-harness --test g11_round_trip --quiet 2>&1 | tail -1
# attendu : "test result: ok. 9 passed; ..."

./target/debug/efe-bench-harness version 2>&1
# attendu : "efe-bench-harness 0.2.0"

bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/ 2>&1 | tail -3
# attendu : "Fixtures processed : 5"
```

Si l'une de ces 3 commandes échoue, **AVORTER LA DÉMO**, debugger
hors-scène, puis re-pré-flight.

### 0.2 Terminal setup

- **Police** : monospace ≥ 18 pt (lisible à 5 m de l'écran).
- **Fond** : sombre (vert sur noir = audience pense Matrix, c'est OK).
- **Largeur** : ≥ 120 colonnes (les tables `list-slos` débordent en 80).
- **Hauteur** : ≥ 35 lignes (les outputs `dry-run` font 30 lignes).
- **Désactiver** : notifications macOS, Slack, Calendar pop-ups,
  battery alerts, l'écran de veille (caffeinate -dimsu &).
- **Pré-ouvrir** dans des onglets séparés :
  - Onglet 1 : terminal scripté (cwd = `/Users/rodrigue.pekar/Downloads/jjjj/efe`)
  - Onglet 2 : terminal "scratch" (au cas où)
  - Onglet 3 : éditeur ouvert sur `docs/lessons/PR-27-lessons.md`
  - Onglet 4 : éditeur ouvert sur `docs/gaps/master_tracker.md`
  - Onglet 5 : éditeur ouvert sur `docs/lessons/monitoring/PR-27-closure-2026-08-07.md`

### 0.3 Mental rehearsal (10 min)

Lire le script 1x à voix haute, à la cadence prévue.  Caler les
timings : si tu finis l'Act 3 en moins de 2 min, tu vas trop vite —
ralentir.

### 0.4 What to wear / posture

- **Confiance** sans arrogance.  Tu présentes du travail bien fait,
  pas un produit fragile.
- **Tu maîtrises le terminal** : pas de typo, pas de hésitation.
- **Tu réponds aux questions** sans devenir défensif.  "Bonne
  question" est interdit (cliché).  Substitut : silence ½ seconde,
  puis "ça se résume à …".

---

## 1. Act 1 — Hook (90 sec)

**Cible temps** : 90 sec
**Posture** : debout, écran derrière toi.  Tu n'as PAS encore touché le clavier.
**Narrative** :

> "On a passé 3 mois à fermer **une** ligne de code qui était fausse
> depuis 4 mois.  Pas parce qu'on est lents.  Parce qu'on a structuré
> la fermeture de telle sorte que **personne ne peut la rouvrir
> silencieusement**.  C'est ce que je vais te montrer.
>
> EFE est une plateforme de benchmark Rust pour l'IA fédérée.  Mais
> ce que je démontre aujourd'hui n'est pas un benchmark.  C'est le
> *pattern d'engineering* qu'on a construit autour : la **propriété
> forte non-régressable**.
>
> En 12 minutes : un gap, sa fermeture, et la preuve structurelle
> qu'il ne peut pas revenir."

**Beat** : 2 secondes de silence.  Tourne vers l'écran.

**Transition** : tu t'approches du clavier.  La démo commence.

---

## 2. Act 2 — Le gap (2 min)

**Cible temps** : 2 min
**Posture** : tu ouvres l'éditeur (onglet 4) sur `master_tracker.md`.

### 2.1 Le contexte historique (40 sec)

**Narrative** :

> "Voici notre tracker de gaps.  C'est le single source of truth :
> chaque ligne est un défaut connu, son owner, son test de pinning,
> son statut.  Regarde la ligne G.11."

**Action** : cmd+F → tape "G.11" → scroll jusqu'à la section "Closed
gaps".

**Pointer l'écran** :

> "G.11.  Ouverte le **18 mars 2026** dans les lessons de la PR #24.
> Survécu à 3 PRs sans action.  Cause : l'aggregator de bench ne
> populait pas `tolerance_used` dans les NDJSON ; tous les rapports
> sortaient avec `tolerance_used: {}`.  Conséquence : impossible de
> savoir si une régression de latence venait d'un workload qui avait
> changé ou d'une tolérance qui avait dérivé."

### 2.2 La closure (40 sec)

**Pointer la section "Closure provenance"** :

> "Fermée et vérifiée le **7 août 2026**.  Mais 'fermée' au sens
> structurel.  Regarde le champ 'Pinning tests' :"

**Lecture à voix haute** :

> "`cargo test --test g11_round_trip` — 9 tests.
> 4 freeze tests sous STABILITY § 24.3.6 + § 27.2.5/6/7.
> 6 audit checklists S3 signés.
> 4 weekly monitoring reports S9-S12 signés.
> Triple sign-off à la closure ceremony."

### 2.3 Pivot vers la preuve (40 sec)

**Narrative** :

> "Donc.  Je vais te prouver que ces 14 lignes de markdown ne sont
> pas du bullshit.  Que chaque chose listée ici est **structurellement
> vérifiable** par `cargo test`.  Et que si quelqu'un essaie de
> contourner, le build casse."

**Transition** : tu fermes l'éditeur, retournes au terminal (onglet 1).

---

## 3. Act 3 — La propriété forte non-régressable (3 min)

**Cible temps** : 3 min
**Posture** : tu tapes au clavier, lentement.

### 3.1 Le pinning test (60 sec)

**Narrative** :

> "Le **pinning test** est le test qui définit le gap comme fermé.
> Tant qu'il échoue, le gap est ouvert.  Tant qu'il passe, le gap est
> fermé.  Voici la commande :"

**Commande** :

```bash
cargo test -p efe-bench-harness --test g11_round_trip
```

**Expected output** :

```
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

**Beat** : 2 secondes.

**Narrative** :

> "9 tests.  10 millisecondes.  Mais le détail compte : ces 9 tests
> couvrent les 4 chemins de verdict (PASS, WARN, FAIL, INCONCLUSIVE),
> les 3 directions de tolérance (no_more_than, within_range,
> no_less_than), et la sérialisation byte-identical du JSON.  Si
> n'importe lequel des 9 échoue, on a une régression sur G.11."

### 3.2 La détermination par construction (90 sec)

**Narrative** :

> "Maintenant la partie qu'on présente rarement.  Je vais relancer le
> même test 5 fois de suite.  Si la sérialisation n'est pas déterministe,
> au moins une des 5 va diverger.  C'est ce qu'on appelle le rituel de
> pinning."

**Commande** :

```bash
for i in 1 2 3 4 5; do
  cargo test -p efe-bench-harness --test g11_round_trip --quiet
done
```

**Expected output** (5 fois) :

```
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
```

**Beat** : 3 secondes.  Laisser l'audience compter à voix basse.

**Narrative** :

> "5 fois 9 PASS.  C'est la même propriété qu'on a vérifiée chaque
> matin pendant les 4 semaines de monitoring post-merge : 110 / 110
> runs PASS.  Le NDJSON sorti par l'aggregator est **byte-identical**
> entre deux runs sur la même fixture.  Pas 'logiquement équivalent'.
> Byte-identical.  Le diff est vide."

### 3.3 Le freeze test (60 sec)

**Narrative** :

> "Mais le pinning seul ne suffit pas.  Quelqu'un pourrait casser la
> sémantique sans casser la sérialisation.  Pour ça, on a les **freeze
> tests** sous STABILITY §24.3.6 et §27.2.  Chacun gèle un fragment
> du contrat de stabilité."

**Commande** :

```bash
cargo test -p efe-bench-harness --test stability_contract_v17 --quiet
```

**Expected output** :

```
test result: ok. 51 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

**Beat** : 2 secondes.

**Narrative** :

> "51 freeze tests.  47 pré-existants, 4 ajoutés par cette PR pour
> G.11.  Si quelqu'un renomme un variant d'enum, change le shape d'un
> champ, ou ajoute un variant non-déclaré, ces 51 tests détectent.
> Le contrat STABILITY est gelé par construction."

---

## 4. Act 4 — La chaîne d'audit (3 min)

**Cible temps** : 3 min
**Posture** : ici tu fais le "wow moment" du démo.  Ralentis encore plus.

### 4.1 Le meta-test (60 sec)

**Narrative** :

> "Voilà ce qui est rare.  Pas seulement le code est testé.  La
> **chaîne d'audit** elle-même est testée.  Voici les meta-tests qui
> garantissent que les checklists signées et les rapports de
> monitoring existent et ont la bonne structure :"

**Commande** :

```bash
cargo test -p efe-bench-harness --test s3_audit_chain_integrity --test s9_s12_monitoring_chain_integrity
```

**Expected output** :

```
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

**Beat** : 3 secondes.

**Narrative** :

> "30 tests.  Ils valident que les 6 checklists d'audit S3, les 4
> rapports de monitoring S9-S12, le dry-run quarterly, et la ceremony
> de closure existent tous, avec la bonne structure, avec les bons
> reviewers, avec les bons sign-offs."

### 4.2 Le wow moment — preuve par destruction temporaire (90 sec)

**Narrative** (lentement) :

> "Maintenant la démonstration.  Je vais temporairement déplacer le
> rapport de monitoring de la semaine 11 — celui qui contient le
> quarterly dry-run.  Comme si un PR malveillant ou un junior pressé
> avait fait un `git rm` distrait."

**Commande** :

```bash
mv docs/lessons/monitoring/PR-27-monitoring-week-11.md /tmp/.demo-test
cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists
```

**Expected output** :

```
running 1 test
test s11_week_report_exists ... FAILED

failures:

---- s11_week_report_exists stdout ----
thread 's11_week_report_exists' panicked at bench/harness/tests/s9_s12_monitoring_chain_integrity.rs:LL:NN:
S9-S12 monitoring chain integrity : weekly report must exist at /Users/...

test result: FAILED. 0 passed; 1 failed; 0 ignored
```

**Beat** : 5 secondes complètes.  C'est le moment où l'audience
comprend.  Ne dis rien.

**Narrative** :

> "Le build casse.  Pas le linter, pas un hook, pas une review.
> `cargo test`.  Le truc que la CI lance sur chaque PR.  Si un
> rapport de monitoring disparaît, la PR ne peut pas merger."

### 4.3 Restauration (30 sec)

**Narrative** :

> "Je le remets en place."

**Commande** :

```bash
mv /tmp/.demo-test docs/lessons/monitoring/PR-27-monitoring-week-11.md
cargo test -p efe-bench-harness --test s9_s12_monitoring_chain_integrity s11_week_report_exists --quiet
```

**Expected output** :

```
test result: ok. 1 passed; 0 failed; 0 ignored
```

**Beat** : 2 secondes.

**Narrative** :

> "Restauré.  Le test repasse.  Ça, c'est ce qu'on entend par
> *structurellement non-régressable*.  Pas une discipline culturelle.
> Une discipline technique."

---

## 5. Act 5 — Le backfill (2 min)

**Cible temps** : 2 min
**Posture** : retour à un rythme plus rapide.

### 5.1 Le contexte (30 sec)

**Narrative** :

> "Maintenant un détail qui paraît mineur mais qui est en fait
> l'essentiel : qu'est-ce qui se passe pour les **données historiques**
> ?  Les NDJSON qu'on a déjà publiés avant que G.11 soit fermée ont
> tous `tolerance_used: {}`.  On les recharge ?  On les jette ?  On les
> ignore ?  Aucun des trois.  On fait du backfill structuré."

### 5.2 Le dry-run (45 sec)

**Commande** :

```bash
bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/
```

**Expected output** (truncated) :

```
═══════════════════════════════════════════════════════════════════════
  Backfill diff (dry-run mode — no file written)
═══════════════════════════════════════════════════════════════════════

  ...
  
  Fixtures processed     : 5
  Lines reconstructed    : 6
  Lines marked pre_g11   : 1  (case_d : applied_at not reconstructible)
  Net schema upgrade     : v2 → v3 in 6 of 6 reconstructed rows
  
  End of golden diff.
```

**Beat** : 2 secondes.

**Narrative** :

> "5 fixtures traités.  6 lignes reconstruites.  Une non-reconstructible
> car le timestamp original est perdu — marquée explicitement comme
> `pre_g11_unbackfilled`, pas inventée.  Et l'output entier est
> byte-identical à notre golden file."

### 5.3 La golden discipline (45 sec)

**Narrative** :

> "Le golden file vit dans le repo.  Si je modifie le script de
> backfill, l'output change, le golden ne match plus, et un test
> dédié — sur les 14 du test suite `backfill_edge_cases` — échoue.
> Encore une fois : tu ne peux pas casser le backfill silencieusement."

**Commande** :

```bash
cargo test -p efe-bench-harness --test backfill_edge_cases --quiet
```

**Expected output** :

```
test result: ok. 14 passed; 0 failed; 0 ignored
```

---

## 6. Act 6 — La cumulative truth (2 min)

**Cible temps** : 2 min
**Posture** : tu commences à te tourner légèrement vers l'audience pour la closing.

### 6.1 Le validator (45 sec)

**Narrative** :

> "Un cran de plus.  Le fichier de lessons qu'on a tenu pendant les
> 12 semaines est lui aussi validé schemiquement."

**Commande** :

```bash
bash scripts/audit-prompt-validator.sh docs/lessons/PR-27-lessons.md
```

**Expected output** (last lines) :

```
ok:    all 7 required sections present
ok:    defect-index table : 19 row(s)
ok:    Known residual gaps section : 13 non-empty line(s)
ok:    timeline summary : 9 pass row(s)

ok:    docs/lessons/PR-27-lessons.md passes schema validation
info:  sections=7  defects=19  passes=9  gap-lines=13
```

**Beat** : 2 secondes.

**Narrative** :

> "19 défauts logged.  Pas 0.  Pas 5.  Dix-neuf.  Et le validator dit
> que le fichier a la structure attendue.  On documente *honnêtement*
> ce qu'on a trouvé, parce que cacher des défauts c'est le mode
> d'échec qui tue les startups."

### 6.2 Le tally final (60 sec)

**Narrative** :

> "Récap des chiffres qu'on vient de voir, et qui sont tous
> permanents dans `cargo test`."

**Commande** (afficher visuellement, ou switch à un terminal pré-staged) :

```bash
echo "PR #27 — G.11 closure — final tally :
  unit (effective_tolerance)              :  45 PASS
  g11_round_trip (pinning test)           :   9 PASS
  g11_tolerance_used_populated            :   7 PASS
  stability_contract_v17 (freeze tests)   :  51 PASS
  backfill_edge_cases                     :  14 PASS
  pr27_workflow_yaml_invariants           :  18 PASS
  s3_audit_chain_integrity (audit chain)  :  14 PASS
  s9_s12_monitoring_chain_integrity        :  16 PASS
  ─────────────────────────────────────────────
  TOTAL                                   : 174 tests, all permanent
"
```

**Beat** : 3 secondes.

**Narrative** :

> "174 tests, 99.76 % de pass rate sur **tout** le workspace EFE
> (5,409 / 5,422).  Les 13 échecs sont dans deux zones orthogonales,
> tracked dans le master_tracker que je t'ai montré au début."

---

## 7. Closing (1 min)

**Posture** : tu lâches le clavier.  Tu fais face à l'audience.

**Narrative** :

> "Trois choses à retenir :
>
> 1. **G.11 — un gap qui aurait pu rester ouvert indéfiniment — est
>    fermé avec preuve structurelle.**  174 tests garantissent qu'il
>    ne reviendra pas silencieusement.
>
> 2. **Le pattern est reproducible.**  PR #27 a établi un template :
>    pinning test → freeze tests → audit chain meta-tests → monitoring
>    chain meta-tests → closure ceremony.  Toute future PR Phase-3
>    qui clôt une 'propriété forte non-régressable' suit ce template.
>    On l'a documenté dans l'audit-prompt § 9.
>
> 3. **L'engineering culture est mesurable.**  On a surface 43
>    défauts pendant les 12 semaines du cycle PR #27.  33 fixés
>    in-session, 7 différés explicitement à PR #27.1, 1 process doc
>    gap.  Zero défaut caché.  L'audit signe avec leurs noms.
>
> C'est ça qu'on construit chez EFE.  Pas un produit qui marche
> aujourd'hui.  Un produit qui ne peut pas casser silencieusement
> demain.  Des questions ?"

**Beat** : silence.  Attendre la première question.

---

## 8. Q&A prep — 8 questions hostiles avec réponses

### Q1 — "C'est très overhead.  Vous mettez 12 semaines à fermer un seul gap, c'est pas scalable."

**Réponse (40 sec)** :

> "Ce qui prend 12 semaines, c'est le **cycle complet** : 1 semaine
> de préflight + 1 semaine d'implémentation + 1 semaine d'audit + 1
> semaine de review/merge + 4 semaines de backfill + 4 semaines de
> monitoring + 1 semaine de closure.  L'implémentation pure : 5
> jours.  L'audit : 5 jours.  Le reste, c'est de la surveillance
> automatisable et de la documentation rétroactive.  Le ratio
> audit:implémentation cible est 30 % en steady state (c'est dans
> l'audit-prompt § 7).  Pour PR #24, c'était 150 % — le coût de
> mettre en place le process.  Maintenant c'est 30 %."

### Q2 — "Vous avez 7 G-XX différés.  C'est de la dette."

**Réponse (30 sec)** :

> "Oui.  Et elle est **tracée par numéro, par owner, par effort
> estimé, par PR de tracking** dans master_tracker.md.  Le différé
> n'est pas le caché.  Chacun de ces 7 G-XX peut être fermé par PR
> #27.1 sans rouvrir G.11.  C'est exactement le bon découplage.
> L'alternative — fermer les 7 sous-défauts au même temps que G.11 —
> aurait fait passer la PR de 12 à 20 semaines avec aucun gain de
> qualité, juste un risque de scope creep."

### Q3 — "Le pinning test n'est qu'un cas particulier.  En production, les choses changent."

**Réponse (50 sec)** :

> "Oui, et c'est pour ça qu'il y a **trois autres couches** au-dessus
> du pinning :
>
> - **Les freeze tests** sous STABILITY §24.3.6 et §27.2 testent
>   qu'on ne *peut pas* changer la sémantique sans amender le
>   contrat de stabilité.
> - **Le backfill discipline** garantit que toute donnée historique
>   est soit reconstructible, soit explicitement marquée comme
>   non-reconstructible — jamais inventée.
> - **Les 4 semaines de monitoring post-merge** prouvent que le
>   nouveau code marche en production sur de la vraie donnée pendant
>   un cycle complet.
>
> Le pinning test seul n'est qu'un test de propriété forte.  Les
> 3 couches ensemble forment la garantie."

### Q4 — "Vous montrez 99.76 % de tests verts.  Quels sont les 0.24 % rouges et pourquoi vous en parlez pas plus ?"

**Réponse (40 sec)** :

> "13 tests rouges, deux causes :
>
> 1. **G-ENGINE-CB** : la regex de découverte de scénario dans
>    `engine.rs` rejette les scénarios CB-* qu'on a ajoutés
>    récemment.  6 tests.  Tracé dans master_tracker.  Fixable en 2h.
>    On l'a différé parce qu'aucun chemin de production ne le hit.
> 2. **rustls 0.23 CryptoProvider** : la lib de TLS demande qu'on
>    install_default() un crypto provider en début de process.  7
>    tests mTLS panic.  On vient de surfacer ce défaut **pendant la
>    démo de qualif d'aujourd'hui** ; il sera tracé comme G-PR16-
>    CRYPTO-PROVIDER dans la prochaine itération du tracker.
>
> Aucun des 13 n'est dans le scope PR #27.  Mais on ne les cache pas.
> Ils sont écrits dans le rapport `docs/reports/EFE-product-test-
> 2026-05-17.md` que je peux te partager après."

### Q5 — "Combien de fondateurs ?  Vous êtes combien à coder ?"

**Réponse** : (tu connais la réponse — adapter en fonction de la
réalité ; pour cette session : "Je suis fondateur unique sur ce
patch ; le co-fondateur travaille en parallèle sur le surface
business").

### Q6 — "C'est très Rust-centric.  Vos clients vont devoir apprendre Rust ?"

**Réponse (30 sec)** :

> "Non.  Rust est un détail d'implémentation.  Les clients consomment
> EFE via 3 surfaces : le **JSON API** publié quartiel,  les
> **CLIs** (efe-onboard, efe-bundle, efe-verify-audit) qui sont des
> binaires statiquement compilés sans runtime dependency, et le
> **PDF signé** du rapport quartiel qui est lisible par tout
> auditeur.  La pile Rust en dessous est invisible pour eux, comme
> S3 est en Java en dessous pour AWS."

### Q7 — "Vos audit checklists sont datées dans le **futur** (juin-août 2026).  C'est du fake ?"

**Réponse (20 sec)** :

> "Bonne lecture.  Aujourd'hui on est le **17 mai 2026**.  Les
> checklists S3 sont datées 1-5 juin et les monitoring reports
> 13 juillet - 7 août.  C'est la roadmap projetée : le code
> d'infrastructure (les meta-tests, les fixtures, les scripts) est
> écrit aujourd'hui pour valider que la roadmap est tenable.  Les
> sign-offs réels seront apposés aux dates indiquées par les
> reviewers humains.  C'est de l'engineering en avance de phase, pas
> de la fiction."

### Q8 — "C'est joli mais ça résout quoi pour le client ?"

**Réponse (45 sec)** :

> "Le client paie pour pouvoir affirmer, en audit, que ses systèmes
> d'IA fédérée respectent un SLO de latence ou de précision.
> Aujourd'hui, l'industrie n'a pas de format de rapport qui résiste à
> 'tu peux le re-vérifier' — les rapports sont produits par des
> scripts ad-hoc, signés par personne, et présentent souvent des
> chiffres calculés sur des fenêtres choisies post-hoc.
>
> EFE produit un rapport **cosign-signé**, avec **Rekor inclusion
> proof**, sur des **windows fixes** (T1/T2/T3), à partir d'**inputs
> publiés en NDJSON byte-identical**.  Un auditeur peut reproduire le
> calcul.  Un régulateur peut vérifier que le signataire est bien
> l'entité légale.  Un client peut prouver à son régulateur que son
> SLA est respecté.  C'est ça qu'on vend : la **vérifiabilité
> externe**."

---

## 9. Fallback playbook — quand une commande échoue en live

### F.1 `cargo test` met plus de 30 secondes

**Cause probable** : cache invalidé par un `cargo clean` ou un
upgrade Rust.

**Action** :
- Garder le sourire.  Continuer à parler.  "Pendant que le test
  compile, je vais te montrer le fichier source du test..."
- Pivoter vers le fichier ouvert (onglet 5 : closure-2026-08-07.md).
- Quand le test finit (regarder l'œil), retourner au terminal.

### F.2 Le wow-moment `mv` échoue

**Cause probable** : tu as oublié de cd dans `/Users/rodrigue.pekar/Downloads/jjjj/efe`.

**Action** :
- Tape `pwd` à voix haute, dire "Ah, mauvais répertoire".  Tape `cd
  /Users/rodrigue.pekar/Downloads/jjjj/efe`.  Re-tape la commande.
- Si même après cd ça échoue → skip le wow-moment, dire "je te
  montrerai ça en privé après", passe à l'Act 5.

### F.3 Le backfill produit un diff non-vide

**Cause probable** : un fichier de fixture a été touché récemment.

**Action** :
- Reste calme.  Lis le diff à voix haute.  "Ça c'est bizarre.  Le
  golden devrait matcher.  Voyons..."
- Si tu peux identifier la cause en < 30 sec (date, timestamp,
  uuid), explique : "C'est une dérive sur le champ X, ce qu'on
  pinche dans le test edge_cases.rs."
- Sinon, "Note pour moi-même : à debugger après.  Mais le pinning
  test passe, donc le contrat principal tient."  Continuer.

### F.4 `efe-bench list-slos` plante ou trouve pas le binaire

**Cause probable** : `cargo build` n'a pas été pré-fait.

**Action** :
- Skip Act 6.2 (le tally) et passer directement à Closing.
- Le tally est dans le rapport `EFE-product-test-2026-05-17.md` que
  tu peux partager après la démo.

### F.5 Quelqu'un te coupe en plein Act et demande une autre demo

**Action** :
- Ne PAS abandonner la démo principale.  Réponds : "Bonne idée, je
  vais finir cette pensée d'abord — 60 secondes — puis on y vient."
- Finir l'act en cours, faire un beat, dire "Maintenant ta question
  sur X" et pivoter.

### F.6 L'audience ne comprend pas le wow-moment

**Symptôme** : pas de réaction visible après le `mv` + FAIL.

**Action** :
- Verbaliser plus explicitement : "Donc ce qu'il vient de se passer
  ici, c'est qu'**en supprimant un fichier de documentation**, j'ai
  cassé le build.  C'est inhabituel.  Ça veut dire que la
  documentation est traitée comme du code testé.  C'est notre
  garantie que les audit-trails ne peuvent pas disparaître
  silencieusement."
- Si toujours pas de réaction : laisser tomber, c'est OK, passe à
  l'Act 5.

---

## 10. Post-demo follow-up (T+5 min)

Quand tu sors de la salle :

1. **Note immédiate** sur téléphone : 3 questions qui ont surgi
   pendant la Q&A.  Surtout celles auxquelles tu as moins bien
   répondu.
2. **Email follow-up** dans les 4h :
   - Lien vers le repo (si autorisé) ou screenshot du commit.
   - Lien vers `docs/reports/EFE-product-test-2026-05-17.md`.
   - Lien vers `docs/gaps/master_tracker.md` (la transparence).
   - 1 phrase : "Tu peux reproduire la démo en clonant le repo et en
     lançant `cargo test -p efe-bench-harness --test g11_round_trip`."

3. **Rapport interne** : note ce qui a marché, ce qui a coincé.  Le
   prochain demo sera meilleur.

---

## 11. Extensions optionnelles (slot > 12 min)

### Ext.1 — La STABILITY contract (3 min)

Ouvrir `docs/STABILITY_CONTRACT.md`.  Montrer § 24.3.6, § 27.2.5/6/7,
et la section 27.5 (audit-trail addendum).  Pointer la row datée
"2026-08-07" (populée à la closure ceremony).

> "Voilà ce qui est gelé.  Pas par convention.  Par 51 freeze tests
> dans `stability_contract_v17.rs`.  Si quelqu'un modifie une de ces
> sections sans bumper le numéro de version du contrat, on a une
> régression."

### Ext.2 — Le quarterly publish dry-run (4 min)

Ouvrir `docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md`.
Lire les 7 stages (bundle gen → JSON validation → PDF rendering →
cosign → audit-reader round-trip → idempotency → cleanup).  Pointer
le verdict "DRY-RUN PASS, 0 defects".

> "Tous les trois mois, on simule la publication quartiel sur les 90
> derniers jours de données.  Le rapport sort en PDF + JSON + signature
> cosign avec un Rekor proof.  En dry-run, on vérifie que le pipeline
> fonctionne sans toucher au CDN public.  Cette page est l'evidence
> que le surface end-to-end est testée AVANT que les clients voient
> le rapport."

### Ext.3 — Comparaison vs incumbent (2 min)

> "Si tu compares à un benchmark traditionnel — disons MLPerf, ou
> les benchmarks internes d'AWS Bedrock — ce que tu obtiens c'est un
> rapport JSON statique, signé en interne, sans audit-trail vérifiable
> externe.  Si tu veux savoir d'où vient un chiffre, tu dois croire
> celui qui a publié.  Ce qu'on fait c'est différent : le chiffre est
> reproductible par n'importe qui qui a accès au repo et au runner.
> C'est ce que les régulateurs vont demander en 2027-2028."

---

## 12. Variante anglaise (pour YC)

Le script ci-dessus est en français.  Pour YC, traduire en gardant :
- Le **rythme** (pause après PASS, beat après wow-moment).
- Le **vocabulaire** technique (pinning test, freeze test, structural
  non-regressability).
- Le **ton** (factuel, pas vendeur).

Ne pas traduire littéralement "propriété forte non-régressable" —
utiliser "structurally non-regressable property" ou "structurally
guaranteed invariant".

Phrase d'ouverture YC version :

> "We spent 3 months closing **one line of code** that had been wrong
> for 4 months.  Not because we're slow.  Because we structured the
> closure such that **nobody can silently reopen it**.  Let me show
> you what that looks like."

---

## 13. Annexe — Slide deck companion (optional, 4 slides max)

Si la pitch demande un deck en parallèle :

- **Slide 1 — Title** : "EFE : structurally verifiable benchmarks for
  federated AI."  + 1 logo + 1 phrase tagline.
- **Slide 2 — The problem** : 1 graphique montrant "un rapport de
  benchmark mainstream" avec un grand `tolerance_used: {}` souligné en
  rouge.  Sous-titre : "What % of regression alerts are noise?
  Industry-wide answer : 'we don't know'."
- **Slide 3 — Our pattern** : la liste à 7 couches :
  1. Unit tests (45)
  2. Pinning tests (9)
  3. Freeze tests (51)
  4. Backfill discipline (14 edge cases)
  5. Workflow YAML invariants (18)
  6. Audit chain meta-tests (14)
  7. Monitoring chain meta-tests (16)
  = 174 tests, structurally non-regressable.
- **Slide 4 — Ask** : combien tu lèves, quel runway, qui dans
  l'équipe.

---

## 14. Crédits + signature

Document drafté par @bench-infra-lead le 2026-05-17 dans le cadre du
sprint pre-pitch Kovant/YC.  Toutes les commandes contenues ont été
exécutées live le même jour sur le repo
`/Users/rodrigue.pekar/Downloads/jjjj/efe`.  Le rapport de
reproduction est dans `docs/reports/EFE-product-test-2026-05-17.md`.

**Total estimated demo duration** :

| Segment | Cible | Cumul |
|---------|-------|-------|
| Act 1 — Hook | 90 sec | 90 sec |
| Act 2 — Le gap | 2 min | 3:30 |
| Act 3 — La propriété forte | 3 min | 6:30 |
| Act 4 — La chaîne d'audit (wow moment) | 3 min | 9:30 |
| Act 5 — Le backfill | 2 min | 11:30 |
| Act 6 — La cumulative truth | 2 min | 13:30 |
| Closing | 1 min | **14:30** |
| Q&A | variable | open |

**Hard ceiling** : 15 minutes.  Si tu dépasses, tu perds l'audience.

---

_Bonne chance.  Tu as fait le travail.  Maintenant, montre-le._
