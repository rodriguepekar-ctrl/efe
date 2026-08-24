# Archive — mai 2026

**⚠ Ces documents ne décrivent PAS l'état courant du dépôt.** Ils datent des 23–26 mai 2026 et
sont antérieurs à tout ce que M0→M9, M1.b, M1.c, MZ et P1 ont changé. Les lire comme
documentation vivante induirait en erreur ; le canon actuel est `docs/architecture/`.

Ils vivaient hors du dépôt, dans le dossier parent, et n'existaient nulle part ailleurs.
Ce répertoire les met à l'abri **en tant qu'archive**, pas en tant que référence.

## Ce qu'il y a, et ce qui est régénérable

| Fichier | Source dans le dépôt ? |
|---|---|
| `ARCHITECTURE.md`, `ARCHITECTURE_GLOBALE.md` | — (uniques) |
| `EFE_ANALYSE_ET_PLAN_FINITION.md`, `EFE_CAPACITES_AUJOURDHUI.md`, `RESEARCH.md` | — (uniques) |
| `ARCHITECTURE_GLOBALE.pdf`, `EFE_CAPACITES_AUJOURDHUI.pdf` | régénérables depuis le `.md` ci-dessus par `scripts/_md_to_pdf.py` |
| `EFE_MVP_Sous_Modules.pdf` | régénérable par `generate_mvp_submodules_pdf.py` (suivi) |
| `EFE_Architecture_Complete.pdf`, `EFE_Coding_Roadmap.pdf`, `EFE_Investor_Brief.pdf`, `EFE_Roadmap_Complete.pdf` | **AUCUNE** — exemplaires uniques, non reconstructibles |

Ces quatre derniers sont la raison de l'exception `!docs/archive-2026-05/*.pdf` dans le
`.gitignore` : la politique du dépôt est de ne pas suivre les PDF (on suit la source), mais une
politique qui ferait perdre un artefact sans source se retournerait contre son propre but.

## Le patch

`0001-fix-phase-1-grammars-compatibles-llama.cpp-b9297-bum.patch` est **DÉJÀ APPLIQUÉ** : son
commit `eb63412` est un ancêtre de `HEAD` — vérifié par `git merge-base --is-ancestor`.
Il est conservé comme trace, **pas** comme travail en attente. Ne pas le ré-appliquer.

## Portée

`ARCHITECTURE_GLOBALE.md` couvre EFE **et** « Scout & Synthesize », une application qui consomme
la plateforme. S&S n'est mentionné que dans deux fichiers suivis (`README_POC.md`,
`crates/efe-nano-app-coordinator/README.md`) — c'est donc ici que sa description la plus complète
se trouve.

## Les deux fichiers de permissions Claude Code

`claude-permissions.vrai.local.json` et `claude-permissions.jjjj-copie.local.json` sont les
`.claude/settings.local.json` des deux répertoires parents, archivés **à plat et renommés**.

Ils ne sont **pas** placés dans un `.claude/` du dépôt, et c'est délibéré : le suffixe
`.local.json` désigne le fichier que Claude Code n'attend justement pas en partage. Actif dans le
dépôt, il ferait hériter tout clone d'autorisations d'exécution que son propriétaire n'a jamais
accordées. Ici ils sont conservés comme trace, inertes.
