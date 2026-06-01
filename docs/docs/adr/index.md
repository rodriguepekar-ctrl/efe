# Architecture Decision Records

Les ADRs capturent les décisions d'architecture majeures avec leur
contexte, options considérées, décision et conséquences.

## Format

Inspiré de [Michael Nygard](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions).
Chaque ADR a les sections : Status, Date, Deciders, Contexte, Options,
Décision, Conséquences, Révision.

## Index

| # | Titre | Statut | Date |
|---|-------|--------|------|
| 001 | [Choix de la stack technique](ADR-001-tech-stack.md) | Accepted | 2026-06-29 |
| D93-001 | [Bump workspace rustc to 1.88](ADR-D93-001-rustc-1.88-bump.md) | Accepted | 2026-05-10 |
| D93-002 | [Defer pen-test `_test/*` endpoints to chaos-cluster sprint](ADR-D93-002-pen-test-deferral.md) | Accepted | 2026-05-10 |
| D93-003 | [Mount `/v1/agent/*` via efe-server, not a separate gateway pod](ADR-D93-003-rest-mount-strategy.md) | Accepted | 2026-05-10 |

## Nouveau ADR

Pour proposer un nouvel ADR :

1. Créer un PR avec `docs/docs/adr/ADR-XXX-titre.md`.
2. Statut initial : `Proposed`.
3. Discussion sur le PR (≥ 3 reviewers).
4. Après merge : statut `Accepted`.
5. Si renversé : statut `Superseded by ADR-YYY`.
