# ADR-M30-002 : Taxonomie d'erreurs typée pour le plan multimodal

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M30_MULTIMODAL_ARCHITECTURE.md § 6.2

## Décision

Chaque erreur du crate `efe-multimodal` est un variant explicite de
`MultimodalError`. Chaque variant expose deux accesseurs stables :

- `error_code()` → identifiant chaîne stable (ex.
  `multimodal.input_too_large`).
- `http_status()` → code HTTP recommandé (mappé en table).

Les handlers REST `v1/multimodal/*` ne font PAS de matching sur
`Debug` ; ils délèguent au `IntoResponse` qui consulte
`error_code()` + `http_status()`.

## Justification

1. **Stable wire-contract** — `error_code` est publié dans
   `openapi.yaml` et fait partie du contrat client. Renommer un
   variant Rust ne casse pas l'API.
2. **Audit-safe** — le `Display` d'un variant ne contient JAMAIS
   les bytes d'entrée (image, audio, transcript). Conformité M29
   PII redactor enforced à compile-time par le linter QA.
3. **HTTP mapping centralisé** — un seul endroit à modifier quand
   le mapping HTTP doit évoluer (ex. 422 → 400 par décision
   produit).

## Conséquences

### Positives
- Pas de double indirection (`MultimodalError → reqwest::Error → HTTP`).
- Tests : `MultimodalError::X.error_code()` figé par
  `tests::error_codes_are_stable`.

### Négatives
- 11 variants pour M30 — refactor coûteux si on doit en supprimer un
  (impact API public).
- Pas d'extensibilité — ajouter une variant = SemVer minor bump du
  crate (workspace v0.0.x mais à surveiller en post-1.0).
