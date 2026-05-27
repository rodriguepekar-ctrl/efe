# Phase 3 R2 — Sprint S2 : Live Validation (TEMPLATE)

> **Ceci est un template.** À remplir avec les valeurs OBSERVÉES après
> exécution réussie de `live_mission_a_naked_no_slm_just_claude`.
> Une fois rempli, renommer en `S2_LIVE_VALIDATION.md` (sans `.template`).

---

**Date de validation** : YYYY-MM-DD (HH:MM UTC)
**Hardware** : <Mac M-series / Linux x86_64 / …>
**llama-server** : <pas utilisé en S2 — Mission A naked, aucun nano-robot>
**Claude model** : `claude-sonnet-4-6` (DEFAULT_CLAUDE_MODEL)

---

## Résultat final

✅ **Mission A naked validée live (Coordinator + Claude, 0 nano-robot)**

```text
=== Mission A naked ===
  query    : Quelle est la capitale de la France ? Réponds en une seule phrase courte.
  llm_id   : claude_claude-sonnet-4-6
  cost_cents  : <X>
  latency_ms  : <Y> (mission_a internal) / <Z> (wall-clock)
  audit_hash  : <abc...def> (64 chars)
  final_text  : "<Paris est la capitale de la France.>"
  events_seen : ["SWARM_STARTED", ..., "LLM_FINISHED", ..., "SWARM_FINISHED"]
test live_mission_a_naked_no_slm_just_claude ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## Métriques observées

| Métrique | Cible (architecture §4.3) | Valeur observée | Statut |
|---|---|---|---|
| `final_text` non vide | A1 | <…> | <✅/❌> |
| `final_text` contient "paris" | A2 | <…> | <✅/❌> |
| `llm_used == claude_id` | A3 | <claude_claude-sonnet-4-6> | <✅/❌> |
| `cost_cents` | A4 ≤ 5 | <X> ¢ | <✅/❌> |
| `latency_ms` | A5 < 30 000 | <Y> ms | <✅/❌> |
| `latency_ms` | A6 ≥ 100 | <Y> ms | <✅/❌> |
| `audit_chain_hash.len()` | A7 == 64 | 64 | <✅/❌> |
| `verdicts.len()` | A8 == 0 | 0 | <✅/❌> |
| Wall-clock | A9 < 35 000 | <Z> ms | <✅/❌> |
| Events `SWARM_STARTED` + `SWARM_FINISHED` | A10 | présents | <✅/❌> |
| Event `LLM_FINISHED` | A11 | présent | <✅/❌> |

---

## Liens

- **Architecture S2** : `docs/architecture/EFE_A1_PHASE3_R2_S2_MISSION_A_NAKED_ARCHITECTURE.md`
- **Test** : `crates/efe-coordinator/tests/live_missions.rs`
- **Fixture** : `crates/efe-coordinator/tests/fixtures/missions/01_factual_simple.json`
- **Commit feature** : `c61be00`

---

## Commits S2 cumulés (à compléter)

```text
<HHHHHHH> feat(phase-3-r2-s2-patch): combler les 6 gaps chirurgicaux S2
c61be00 feat(phase-3-r2-s2): Mission A naked — Coordinator + Claude live
1fa7c19 docs(phase-3-r2-s2): architecture chirurgicale Mission A Naked
```

---

## Conclusion

Sprint S2 validé end-to-end. La fondation horizontale (Coordinator ↔
Provider) est saine sous conditions réelles. On peut empiler S3-S9 en
toute confiance.

**Prochaine étape** : Sprint S3 — Mission A avec analyzer + verifier
nano-robots Phase 1.
