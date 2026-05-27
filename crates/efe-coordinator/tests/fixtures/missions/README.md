# tests/fixtures/missions/ — Fixtures de requêtes utilisateur

Ces fichiers JSON sont des **requêtes utilisateur représentatives** consommées
par les tests live Phase 3 R2 (`tests/live_missions.rs`).

Conçues pour être **réutilisables d'un sprint à l'autre** : S2 ajoute les
queries de base, S3-S4 enrichissent avec des champs métier (`expected_compression_ratio`,
`expected_verifier_confidence`, …), S9 utilise toute la collection pour
mesurer le gain de pré-mâchage.

---

## Format JSON

```json
{
  "name": "<kebab-or-snake-case identifier>",
  "task_class": "FactualQa|Analysis|Code|Creative|Reasoning|Translation",
  "query": "<le texte exact envoyé à Mission A / B / Speculative>",
  "expected_keyword": "<token attendu dans final_text après lowercase()>",
  "expected_lang": "fr|en|...",
  "comment": "<note humaine sur l'objectif de cette fixture>"
}
```

### Champs requis (S2)

| Champ | Type | Justification |
|---|---|---|
| `name` | string | Identifiant lisible. Match le nom de fichier sans extension |
| `task_class` | enum | Catégorie sémantique (cf. `efe_nano_app_coordinator::types::router::TaskClass`) |
| `query` | string | Le texte exact envoyé via `MissionARequest.user_text` |
| `expected_keyword` | string (lowercase) | Asserté dans `final_text.to_lowercase().contains(...)` (A2 du test S2) |
| `expected_lang` | string | Langue attendue (info documentaire, pas asserté en S2) |
| `comment` | string | Note interne pour le mainteneur |

### Champs à venir (S3-S9)

```jsonc
{
  // ... champs S2 ci-dessus ...

  // S3 — verifier nano-robot
  "expected_verifier_verdict": "SUPPORTED",
  "expected_verifier_confidence_min": 0.7,

  // S4 — compressor + métriques
  "expected_compression_ratio_max": 0.7,    // ≤ 30 % réduction
  "expected_pre_chew_tokens_max": 50,

  // S6 — router
  "expected_route": "claude_claude-sonnet-4-6",
  "expected_route_justification_keyword": "factual",

  // S9 — savings report
  "baseline_min_tokens": 80,
  "baseline_min_cost_cents": 1
}
```

Les anciennes fixtures restent compatibles : les champs absents sont simplement
non vérifiés.

---

## Convention de nommage

Fichiers numérotés `NN_<name>.json` pour ordre prédictif :

| # | Fichier | Sprint d'origine | Task class | Objectif |
|---|---|---|---|---|
| 01 | `01_factual_simple.json` | S2 | FactualQa | Hello-World — réponse factuelle 1-token |
| 02 | (à venir) `02_factual_with_numbers.json` | S3 | FactualQa | Forcer chiffres dans la réponse (vitesse lumière) |
| 03 | (à venir) `03_code_gen_rust.json` | S6 | Code | Test du router → Claude pour code |
| 04 | (à venir) `04_summary_short.json` | S9 | Analysis | Compresseur testé |
| 05 | (à venir) `05_long_reasoning.json` | S9 | Reasoning | Multi-step, Claude Opus typiquement |
| 06 | (à venir) `06_multi_step.json` | S9 | Reasoning | Transitivité logique |
| 07 | (à venir) `07_ambiguous.json` | S4 | FactualQa | Force reformulator à se déclencher |
| 08 | (à venir) `08_pii_heavy.json` | S8 | FactualQa | Stress PII redactor (I15) |

---

## Comment ajouter une nouvelle fixture

1. Choisir un numéro libre (cf. tableau ci-dessus).
2. Créer `tests/fixtures/missions/NN_<name>.json`.
3. Mettre à jour ce README (tableau + champs si nouveaux).
4. Référencer la fixture dans le test correspondant
   (`tests/live_missions.rs` ou `tests/workflow_mission_*.rs`).
5. Si tu ajoutes un champ : documenter ici + mettre à jour le `build_*_request`
   helper concerné dans `live_missions.rs`.

---

## Vérification d'intégrité

```bash
for f in crates/efe-coordinator/tests/fixtures/missions/*.json; do
    python3 -m json.tool < "$f" >/dev/null && echo "✅ $(basename "$f")" || echo "❌ $(basename "$f")"
done
```
