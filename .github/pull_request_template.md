<!--
═══════════════════════════════════════════════════════════════════════
  EFE-A1 PR template — superseded by PR #25 (process changes I.1 + I.2 + I.6)

  Rules in 1 line each :
    I.1  Audit prompt + lessons file required if PR ≥ 1000 LOC AND
         touches harness src/.  See "Lessons file" section.
    I.2  Const lists of paths / filenames / flags MUST be single
         source of truth.  CI lint enforces.
    I.6  Audit budget section (below) is REQUIRED ; leaving it blank
         fails the pr25-i6-audit-budget status check.

  Full architecture : docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md
  Lessons template  : docs/lessons/_template.md
  Audit prompt      : docs/process/audit-prompt.md  (FR canonical)
                      docs/process/audit-prompt-en.md  (EN translation)
═══════════════════════════════════════════════════════════════════════
-->

## Résumé
<!-- 1-3 phrases : quoi + pourquoi.  L'audience est un reviewer qui n'a
     pas le contexte (lit ça dans 3 mois pour comprendre l'incident
     ouvert ce jour). -->

## Changes
<!-- Bullet list des changements principaux (fichiers majeurs, design
     decisions notables).  Évite "tweaks misc" — sois précis. -->
-
-

## Audit budget (I.6 — requis pour tout PR ≥ 1000 LOC)
<!-- Estimation à ouverture du PR.  Recompute + remplir les "actual"
     au moment du merge.  Tracker cumulatif :
     docs/process/audit-budget-tracker.md
     Cible Phase-3 steady-state : 30 % (PR #24 outlier 150 %). -->

- Implementation : __ days
- Audit (per I.1) : __ days (__ % of impl)
- Buffer for unforeseen depth : __ days
- **Total estimate** : __ days
- Actual at merge time (filled by author at merge) : __ impl / __ audit / __ % ratio

## Lessons file (I.1 — requis pour PR ≥ 1000 LOC touchant harness src)
<!-- Le fichier docs/lessons/PR-<N>-lessons.md DOIT être committé
     dans ce PR.  Suit le schéma de docs/lessons/_template.md.
     Validateur local : `bash scripts/audit-prompt-validator.sh
     docs/lessons/PR-<N>-lessons.md`.
     Status-check CI : pr25-i1-audit-prompt. -->

- [ ] `docs/lessons/PR-<N>-lessons.md` est présent dans ce diff.
- [ ] Le fichier suit le schéma à `docs/lessons/_template.md` (7 sections).
- [ ] La table "Defect index" (§ 6) a ≥ 1 ligne.
- [ ] La section "Known residual gaps" (§ 7) est présente (même si "None").
- [ ] `bash scripts/audit-prompt-validator.sh docs/lessons/PR-<N>-lessons.md` passe localement.

**Si exempt** (LOC < 1000 OU pas de fichier sous `*/src/`), cocher :
- [ ] Exempt — raison : <une phrase>
- [ ] OU `[I.1-bypass]` tag dans le titre + justification dans
      `docs/process/i1-bypass-log.md` (uniquement security hotfix).
- [ ] OU `[I.1-refactor]` tag + diff confirmée ≥ 95 % whitespace /
      imports par reviewer (release-eng).

## SOT — Single source of truth (I.2 — requis si nouveau const list)
<!-- Toute liste de paths / filenames / flags / endpoints / scenario IDs
     avec ≥ 5 éléments DOIT être SOT.  Pas de duplication.
     CI test : `cargo test --test process_invariants`.
     Auditor : `bash scripts/sot-audit.sh`. -->

- [ ] Aucun nouveau `const NAME: &[&str]` (ou shape équivalent) de
      longueur ≥ 5 dans ce PR
- [ ] **OU** la nouvelle liste est à `<path>` et référencée par tous
      les consumers : <listing>
- [ ] **OU** whitelist explicite ajoutée à `tests/process_invariants.rs`
      avec commentaire de justification.
- [ ] `cargo test --test process_invariants` passe localement.

## Tests
- [ ] Unit tests ajoutés / modifiés (couvre le happy path AVANT
      les error paths — bias-correction post PR #24).
- [ ] Integration tests (si user-facing).
- [ ] Benchmarks (si hot path).
- [ ] Tests manuels documentés ci-dessous.

## Risks
<!-- Risques identifiés (perf, sécurité, compat, contract surface).
     Référence les threat-codes de PR #24 § 20 (T100, T116, T118,
     T121, T122, T125) si applicable. -->

## Deploy notes
- [ ] Migration requise : oui / non
- [ ] Config change : oui / non
- [ ] Rollback strategy : <décrire>
- [ ] Runbook impact :
  - [ ] `docs/runbooks/bench-publish-incident-response.md` à jour si nouvelle classe d'incident
  - [ ] STABILITY_CONTRACT § X à jour si surface contractuelle modifiée
  - [ ] Drill cadence (`docs/drills/`) inchangée ; sinon expliquer

## Pre-merge checklist
- [ ] Titre PR suit Conventional Commits (`type(scope): summary`)
- [ ] `cargo test -p efe-bench-harness` passe
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] CI verte (incl. `pr25-i1-audit-prompt`, `pr25-i2-sot-invariants`,
      `pr25-i6-audit-budget`)
- [ ] Self-review fait (lis ton propre diff comme un reviewer
      hostile)
- [ ] CHANGELOG.md entry ajouté
- [ ] Documentation à jour (rustdoc + docs/)

## Test plan
<!-- Étapes manuelles que le reviewer peut suivre pour valider
     l'impact utilisateur.  Inclure les commandes exactes. -->
- [ ]
- [ ]
- [ ]

---

🤖 Si ce PR a été assisté par Claude Code, mentionne-le ici.

Closes #
