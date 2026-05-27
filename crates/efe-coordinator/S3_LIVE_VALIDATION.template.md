# Phase 3 R2 — Sprint S3 : Live Validation (TEMPLATE)

> **Ceci est un template.** À remplir avec les valeurs OBSERVÉES après exécution
> réussie de `live_mission_a_with_analyzer_verifier`.
> Une fois rempli, renommer en `S3_LIVE_VALIDATION.md` (sans `.template`).
> Cohérence avec `S1_LIVE_VALIDATION.md` et `S2_LIVE_VALIDATION.md`.

---

**Date de validation** : YYYY-MM-DD (HH:MM UTC)
**Hardware** : <Mac M-series / Linux x86_64 / …>
**llama-server** : <build 9297+>, daemons analyzer (:8090) + verifier (:8093) UP
**Claude model** : `claude-sonnet-4-6` (DEFAULT_CLAUDE_MODEL)
**Phase 1 SLMs actifs** : 2 sur 4 (analyzer SmolLM2-360M Q5_K_M, verifier Phi-4-mini Q4_K_M)

---

## ✅ Résultat final — 16/16 assertions PASS attendu

```text
running 2 tests
=== Mission A naked === (S2, déjà validé)
test live_mission_a_naked_no_slm_just_claude ... ok

=== Mission A with analyzer + verifier ===
  query          : Quelle est la capitale de la France ? Réponds en une seule phrase courte.
  llm_id         : claude_claude-sonnet-4-6
  analyzer_url   : http://127.0.0.1:8090
  verifier_url   : http://127.0.0.1:8093
  candidate_srcs : 2 (["wp_paris_fr", "wp_france_admin"])
  cost_cents     : <X>
  latency_ms     : <Y> (mission_a) / <Z> (wall-clock)
  audit_hash     : <abc...def> (64 chars)
  final_text     : "<La capitale de la France est Paris.>"
  verifier:
    verdict      : SUPPORTED
    severity     : INFO
    similarity   : <0.78>
    best_source  : Some("<wp_paris_fr>")
  events_seen (<N>) : ["SWARM_STARTED", "ROBOT_STARTED", "PHEROMONE_DEPOSITED",
                        "ROBOT_FINISHED", "LLM_STARTED", "LLM_TOKEN_STREAMED",
                        "LLM_FINISHED", "VERDICT_READY", "SWARM_FINISHED", ...]
test live_mission_a_with_analyzer_verifier ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## Métriques observées vs cibles

| Métrique | Cible (architecture §4.3) | Valeur observée | Statut |
|---|---|---|---|
| `final_text` non vide | B1 | `"<…>"` | <✅/❌> |
| `final_text.to_lowercase()` contient "paris" | B2 | `"<…paris…>"` | <✅/❌> |
| `llm_used == claude_id` | B3 | `claude_claude-sonnet-4-6` | <✅/❌> |
| `cost_cents` | B4 ≤ 5 | <X> ¢ | <✅/❌> |
| `latency_ms` | B5 < 60 000 | <Y> ms | <✅/❌> |
| `latency_ms` | B6 ≥ 100 | <Y> ms | <✅/❌> |
| `audit_chain_hash.len()` | B7 == 64 | 64 | <✅/❌> |
| Wall-clock | B9 < 60 000 | <Z> ms | <✅/❌> |
| `resp.verdicts.len()` | B10 ≥ 1 | <1> | <✅/❌> |
| `verdicts[0].verdict.to_upper()` | B11 == "SUPPORTED" | `<SUPPORTED>` | <✅/❌> |
| `verdicts[0].similarity` | B12 ≥ 0.3 | <0.78> | <✅/❌> |
| Event `ROBOT_STARTED` (analyzer) | B13a | présent | <✅/❌> |
| Event `ROBOT_FINISHED` (analyzer) | B13b | présent | <✅/❌> |
| Event `PHEROMONE_DEPOSITED` (≥1) | B14 | <N> ≥ 1 | <✅/❌> |
| Event `VERDICT_READY` (verifier) | B15 | présent | <✅/❌> |
| `latency_ms` cap S3 | B16 < 10 000 | <Y> ms | <✅/❌> |

**Score attendu : 16/16 assertions vertes.**

---

## Observations bonus à documenter

1. **Latence SLM** : analyzer (~150-300 ms) + verifier (~400-800 ms) — confirmé Metal actif
2. **Streaming** : `LLM_TOKEN_STREAMED` × <N> events observés (cohérent avec R1b)
3. **Entités extraites** : analyzer a déposé <N> phéromones (au moins "France")
4. **Verdict cohérent** : claim "Paris" + sources "Paris" → SUPPORTED avec similarity haute
5. **Coût** : <X> ¢ pour la mission complète (analyzer + verifier gratuits, Claude seul payant)

---

## Bugs / surprises pendant la validation

<à remplir post-run>

---

## Critère d'acceptation S3 — État final

- [x] `crates/efe-coordinator/tests/live_missions.rs` contient le test ✅
- [x] `tests/fixtures/missions/02_factual_with_sources.json` existe ✅
- [x] `cargo build --features all-providers --tests` clean ✅
- [x] `cargo clippy --tests --no-deps` 0 warning S3 ✅
- [x] Skip propre × 3 cas (pas LLM / pas SLM / SLM down) ✅
- [<>] Test live PASS avec env + 2 daemons UP
- [<>] 16/16 assertions B1-B16 PASS
- [<>] Coût observé ≤ 3 ¢
- [<>] Latence observée < 10 000 ms
- [<>] Verdict observé == `SUPPORTED`
- [<>] Similarity observée ≥ 0.3
- [x] 0 régression suite offline (396/396 PASS) ✅
- [<>] `S3_LIVE_VALIDATION.md` scellé (ce fichier renommé)
- [x] Commit `feat(phase-3-r2-s3): …` ✅ (3811867)

---

## Commits R2 cumulés au moment du scellement S3

```text
<HHHHHHH> docs(phase-3-r2-s3): scellement live validation S3 16/16 PASS
<HHHHHHH> feat(phase-3-r2-s3-patch): combler N gaps chirurgicaux S3 (Justfile, template, doc)
3811867 feat(phase-3-r2-s3): Mission A + analyzer + verifier live (16 assertions)
b62821f docs(phase-3-r2-s3): architecture chirurgicale analyzer + verifier
45f3669 docs(phase-3-r2-s2): scellement live validation S2 12/12 PASS
38d5205 feat(phase-3-r2-s2-patch): combler 6 gaps chirurgicaux S2
c61be00 feat(phase-3-r2-s2): Mission A naked — Coordinator + Claude live
1fa7c19 docs(phase-3-r2-s2): architecture chirurgicale Mission A Naked
2f9c918 docs(phase-3-r2-s1): scellement live validation S1 4/4 daemons Metal
+ commits S1 cumulés
```

---

## Liens

- **Architecture S3** : `docs/architecture/EFE_A1_PHASE3_R2_S3_ANALYZER_VERIFIER_ARCHITECTURE.md`
- **Test** : `crates/efe-coordinator/tests/live_missions.rs` (test `live_mission_a_with_analyzer_verifier`)
- **Fixture** : `crates/efe-coordinator/tests/fixtures/missions/02_factual_with_sources.json`
- **Helpers réutilisables S4+** : `analyzer_endpoint`, `verifier_endpoint`,
  `live_slm_enabled`, `analyzer_and_verifier_alive`,
  `build_factual_request_with_sources`, `make_coord_with_analyzer_verifier`

---

## Conclusion S3

Sprint S3 validé end-to-end. **La collaboration nano-robot Phase 1 (SLM)
↔ gros LLM Phase 3 (Claude) ↔ nano-robot Phase 1** fonctionne en mission live :

1. analyzer extrait `<N>` entités + intent + langue
2. Phéromones déposées dans la blackboard (preuve via events `PHEROMONE_DEPOSITED`)
3. Claude génère sur le `user_text` brut (pas encore compressé en S3)
4. verifier fact-check la réponse contre les sources Wikipédia → `SUPPORTED`
5. Audit chain BLAKE3 intègre + bus SSE complet (11+ events)

**Prochaine étape** : Sprint S4 — Mission A complète avec les 4 nano-robots
restants (compressor, reformulator, filter, router) + `ChewingMetrics` qui
mesurera le gain de pré-mâchage en tokens d'entrée du LLM.
