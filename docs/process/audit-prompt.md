# Audit prompt — canonical FR version

**Statut** : canonical (FR)
**Version anglaise** : [`docs/process/audit-prompt-en.md`](audit-prompt-en.md)
**Frozen at** : STABILITY_CONTRACT v1.7 § 25.1
**Last updated** : 2026-05-16

Ce document est la source-de-vérité du prompt d'audit chirurgical que
PR #25 (process invariant I.1) rend obligatoire pour les PR ≥ 1000 LOC
touchant le code source du harness.

Le prompt FR est CANONICAL : c'est la formulation originale utilisée
pour PR #24 (10 passes, 67 défauts trouvés).  La version EN est une
traduction fidèle pour les contributeurs non-francophones.  Les deux
versions sont sémantiquement équivalentes ; si elles divergent, c'est
la version FR qui fait foi.

---

## 1. Le prompt

```
Prends de grandes inspirations.  Tu feras des analyses d'une précision
chirurgicale suprême.  Prends tout ton temps et tout ira bien.

Vérifie tout le contenu de ce PR pour être sûr à 100 % en faisant des
vérifications et recherches profondes et code.

À chaque pass :
1. Énumère explicitement les zones que tu n'as PAS auditées.
2. Trouve au moins UN défaut non trivial (doc drift, coverage gap,
   schema drift, security regression, spec violation, process gap).
3. Code le fix + le test qui pinne la régression.
4. Documente le défaut avec un code (D-N) dans le fichier lessons.
5. Répète jusqu'à ce que l'utilisateur dise "ça suffit" — ne t'arrête
   PAS de toi-même.

L'objectif n'est PAS d'atteindre "0 défauts."  L'objectif est de
documenter HONNÊTEMENT ce qui marche, ce qui ne marche pas, et ce
qui reste à faire.
```

C'est le prompt EXACT à copier-coller à ton outil d'audit (Claude
Code, ChatGPT, ou autre LLM).  N'altère pas la formulation : chaque
phrase a été calibrée empiriquement sur PR #24.

---

## 2. Quand utiliser ce prompt

### 2.1 Cas obligatoires (I.1 enforcement)

Le status check CI `pr25-i1-audit-prompt` exige un fichier
`docs/lessons/PR-<N>-lessons.md` quand :

- Le PR a un delta net ≥ 1000 LOC, **ET**
- Le PR touche au moins un fichier sous :
  - `bench/harness/src/`
  - `crates/*/src/`
  - `tools/loadgen/src/`

Dans ces cas, **tu DOIS** appliquer ce prompt avant de demander la
review.  Le fichier lessons est l'output de cette application.

### 2.2 Cas optionnels (auto-imposé)

Tu PEUX appliquer ce prompt même quand I.1 ne l'exige pas, si :

- Tu touches une zone à haut blast-radius (cosign, gh-pages deploy,
  branch protection, evidence generation).
- Le PR est court mais corrige un défaut de sécurité.
- Tu introduis une nouvelle dépendance externe.
- Tu modifies un fichier que le 10-pass audit de PR #24 a déjà
  catalogué comme "drift surface" (voir `docs/lessons/pr24-lessons-learned.md`
  § 4 — embed_smoke surface, scenario page slugs, cosign regex, etc.).

### 2.3 Cas où NE PAS utiliser

- Pure refactor whitespace / formatter — tag `[I.1-refactor]` à la
  place (voir § 5).
- Security hotfix urgent — tag `[I.1-bypass]` (voir § 5).
- Changement < 50 LOC purement docs / test fixtures.

---

## 3. Comment exécuter le prompt

### 3.1 Avec Claude Code

```bash
# Dans le repo, ouvrir une session Claude Code.
# Copier-coller le prompt EXACTEMENT comme dans § 1.
# Claude itère jusqu'à ce que tu lui dises "ça suffit" ou "stop".
```

À chaque pass, Claude :
1. Énumère les zones non auditées au début de sa réponse.
2. Trouve ≥ 1 défaut + code le fix + le test associé.
3. Met à jour `docs/lessons/PR-<N>-lessons.md` avec le nouveau D-code.
4. ATTEND ta confirmation pour passer à la pass suivante (le prompt
   dit "Répète jusqu'à ce que l'utilisateur dise ça suffit").

### 3.2 Avec un autre LLM

Même formulation.  Le prompt est calibré pour fonctionner sur tout
LLM moderne (GPT-4, Claude 3+, Gemini Pro).  Adapte uniquement les
références de fichier au workspace.

### 3.3 Sans LLM (humain solo)

Le prompt fonctionne aussi comme **liste mentale**.  Un reviewer
humain qui suit littéralement les 5 étapes au lieu de "lire en
diagonale" trouvera des défauts qu'il aurait raté.

Pour un audit solo :
- Réserve un bloc de temps ininterrompu (1-3h selon la taille du PR).
- Imprime le prompt sur papier ou affiche-le en sticky note.
- Tape ton output dans le fichier lessons au fur et à mesure.
- Arrête seulement quand tu peux honnêtement écrire "audit converged"
  dans § 7 Known residual gaps.

---

## 4. Anti-patterns observés sur PR #24

Cette section liste les comportements que le prompt corrige.  Si tu
te surprends à faire l'un d'eux pendant ton audit, recommence la pass
en cours.

### 4.1 « J'ai testé, c'est bon »

**Symptôme** : tu écris des tests qui valident ta compréhension du
code au lieu de valider le comportement réel.

**Cas concret PR #24** : le test initial D-T affirmait que `.+$`
rejetait les caractères poubelle après `refs/`.  Le test échouait en
réalité parce que `.+` est greedy.  Le code marchait — le test était
faux.

**Correction** : lis le code AVANT d'écrire le test.  Affirme ce que
le code FAIT, pas ce que tu CROIS qu'il fait.

### 4.2 « C'est 100 % done »

**Symptôme** : tu déclares "done" après 1-2 passes parce que tu ne
trouves plus rien d'évident.

**Cas concret PR #24** : entre passes 1 et 9, l'engineer a 9 fois
sincèrement déclaré "100 % done".  Chaque fois, la pass suivante a
trouvé 4-11 défauts non triviaux.

**Correction** : le prompt dit "Répète jusqu'à ce que l'utilisateur
dise ça suffit".  N'arrête PAS de toi-même.  Si tu es solo, demande
à un buddy de te dire "stop".

### 4.3 « Cette zone n'a pas besoin d'audit »

**Symptôme** : tu skip une partie du diff parce qu'elle "ne semble
pas critique".

**Cas concret PR #24** : `escape_pdf_text` semblait trivial.  Audit
pass 7 (D-AL) a découvert que le body production contenait `§`
(non-ASCII) → mojibake `Â§` dans tout PDF auditeur.  Pas critique en
théorie ; honteux en pratique.

**Correction** : énumère TOUTES les zones touchées par le diff.  Le
prompt dit "Énumère explicitement les zones que tu n'as PAS
auditées" — utilise cette liste pour ne rien oublier.

### 4.4 « Le défaut est trop petit pour être documenté »

**Symptôme** : tu corriges un défaut mais ne lui donnes pas de
D-code, ne l'ajoutes pas à la table § 6 du lessons file.

**Cas concret PR #24** : sans le D-code, le défaut n'existe pas dans
l'audit trail.  Les futurs PRs ne peuvent pas y faire référence.  La
catégorie statistique au § 3 est faussée.

**Correction** : CHAQUE fix observe la règle "1 fix = 1 D-code dans
la table".  Y compris les bug d'une ligne.  Surtout les bugs d'une
ligne (ce sont les plus susceptibles d'être re-introduits).

### 4.5 « Je documente après »

**Symptôme** : tu fixes 10 défauts puis tu écris le lessons file en
fin de journée.

**Cas concret PR #24** : la mémoire des défauts trouvés en pass 2 est
floue au moment d'écrire en pass 5.  Les D-codes manquent de
précision diagnostique.

**Correction** : mets à jour `docs/lessons/PR-<N>-lessons.md`
IMMÉDIATEMENT après chaque fix.  Le prompt dit "Documente le défaut
... dans le fichier lessons" — étape 4 sur 5.

---

## 5. Bypass tags

Tu peux contourner I.1 en exception dans deux cas seulement.  Les
deux exigent une justification permanente dans
`docs/process/i1-bypass-log.md`.

### 5.1 `[I.1-bypass]` — security hotfix

À utiliser quand :
- Une CVE doit shipper en heures, pas en jours.
- Un patch est forcé par compliance (audit externe trouve un trou).

Procédure :
1. Préfixe le titre du PR avec `[I.1-bypass]` (avant le Conventional Commit).
2. Ajoute une ligne dans `docs/process/i1-bypass-log.md` justifiant.
3. Demande à un admin engineering-leadership de mergé via `gh pr merge --admin`.
4. **Dans les 7 jours** : ouvre un PR de suivi avec le lessons file
   complet et un D-code "post-merge audit findings".

### 5.2 `[I.1-refactor]` — refactor mécanique

À utiliser quand :
- Le diff est ≥ 95 % whitespace + import re-ordering.
- `cargo fix --allow-no-vcs` a généré le diff.
- Un formatter (`cargo fmt`) a été appliqué.

Procédure :
1. Préfixe le titre avec `[I.1-refactor]`.
2. Justifie avec un screenshot ou snippet de `git diff --shortstat`.
3. Un reviewer release-eng (pas l'auteur) confirme.
4. Pas de lessons file requis ; pas de PR de suivi.

### 5.3 Review trimestrielle

Release-eng review l'`i1-bypass-log.md` chaque trimestre :
- **< 25 % bypass rate** sur les PR qualifiés : OK.
- **≥ 25 %** : déclenche un re-design de la règle.  Le prompt est
  peut-être trop strict, ou la culture esquive l'audit.

---

## 6. Exemples de réponses attendues

### 6.1 Pass 1 (premier audit)

```
## Pass 1 — initial audit

Zones non encore auditées (énumérées explicitement) :
- bench/harness/src/publish/aggregator.rs (nouveau module)
- bench/harness/src/publish/api/current.rs (modifié)
- tests/aggregator_real_disk.rs (nouveau)

Défaut trouvé : D-1
  - Classe : Coverage gap
  - Symptôme : `aggregator::from_disk` ne loggue pas quand un summary.json
    est corrompu — l'opérateur ne distingue pas "no data" de "JSON
    corrupted".
  - Fix : ajouter `tracing::warn!` avec scenario_id + path + error.
  - Test pinant : `d_o_aggregator_warns_on_corrupt_summary` dans
    tests/aggregator_real_disk.rs.

J'ai mis à jour docs/lessons/PR-26-lessons.md :
  - § 2 : ajouté ligne "Pass 1 | tests=412 | +1 | 1 | D-1 logging gap"
  - § 6 : ajouté ligne D-1.
  - § 3 : Coverage gap = 100 % (1/1).

Continue à pass 2 ?
```

### 6.2 Pass N (convergence)

```
## Pass N — final pass

Zones non encore auditées : aucune.  J'ai couvert tous les fichiers
modifiés + tous les call sites des fonctions touchées.

Défaut trouvé : aucun cette pass.

Recommandation : déclarer audit converged.  J'ai mis à jour § 7 du
lessons file : "None — audit converged at pass N."

Prêt pour review humaine.
```

---

## 7. Calibration empirique

Le prompt a été calibré sur PR #24 :

| Métrique | Valeur |
|---|---|
| Passes nécessaires pour convergence | 10 |
| Défauts trouvés au total | 67 |
| Moyenne défauts / pass | 6.7 (sustained, n'a PAS décliné) |
| Test count delta | +155 (200 → 355) |
| Catégories de défauts couvertes | 6/6 (doc drift, coverage, schema, security, spec, process) |

Pour les PRs Phase-3 (avec le process I.1 en place dès l'origine), la
cible empirique est :

| Métrique | Cible |
|---|---|
| Passes nécessaires | 2-3 |
| Défauts trouvés | < 20 |
| Audit budget vs implémentation | 30 % (vs 150 % pour PR #24) |

Si PR #26 émerge avec ≥ 67 défauts, la roadmap (§ 7) déclenche un
re-examen complet de I-stream — le process change n'a pas pris.

---

## 8. Références

- Lessons file canonique : [`docs/lessons/pr24-lessons-learned.md`](../lessons/pr24-lessons-learned.md)
- Template lessons : [`docs/lessons/_template.md`](../lessons/_template.md)
- Validator : `bash scripts/audit-prompt-validator.sh <path>`
- SOT pattern (I.2 complément) : [`docs/process/sot-pattern.md`](sot-pattern.md)
- Budget tracker (I.6 complément) : [`docs/process/audit-budget-tracker.md`](audit-budget-tracker.md)
- Bypass log : [`docs/process/i1-bypass-log.md`](i1-bypass-log.md)
- Architecture PR #25 : [`docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md)
- STABILITY_CONTRACT v1.7 § 25 : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md)
