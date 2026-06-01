# ADR-M33-004 : Catalog fermé de prédicats (+ extension custom)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md § 3.2

## Décision

18 prédicats de base hard-codés. Les tenants peuvent ajouter des
prédicats custom via `POST /v1/kg/predicates` (slug
`[a-z0-9_]{1,32}`).

## Justification

1. **Open-domain RE difficile** — produire le bon prédicat de
   manière fiable demande un modèle de pointe ; complexité non
   justifiée pour M33.
2. **Couverture business** — les 18 prédicats couvrent les use
   cases CRM/support/RH les plus courants.
3. **Extensibilité contrôlée** — custom predicates sont
   slug-validated et ne polluent pas le catalog global.

## Conséquences
- Surface RE pas trop large ⇒ classifier accuracy élevée.
- Les tenants à besoin exotique ajoutent leurs propres prédicats.
- M33.1 ajoutera l'extraction open-domain via LLM.
