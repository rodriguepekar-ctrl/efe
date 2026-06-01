# ADR-M29-001 : Microsoft Presidio over spaCy / cloud APIs

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M29_DATA_RIGHTS_ARCHITECTURE.md § 11

## Décision

Presidio comme PII analyzer.

## Justification
1. **Open-source Apache 2.0** — pas de vendor-lock-in.
2. **FR + EN multi-langue** — packs spaCy bundled.
3. **On-prem deploy** — sidecar K8s ; pas d'aller-retour vers le cloud.
4. **Custom recognizers** — pattern + transformer-based, per-tenant.
5. **Data residency safe** — input ne quitte jamais le cluster.

vs spaCy seul : Presidio = spaCy + 30+ recognizers prêts à l'emploi.
vs cloud APIs (AWS Comprehend, GCP DLP) : data residency + cost.

## Conséquences

### Positives
- 2 répliques Presidio suffisent pour 1k req/s sustained.
- Custom recognizers ajoutables sans rebuild.

### Négatives
- Image Microsoft (~2 GB) — pull lent au cold start.
- Mises à jour Presidio = check-listing manuel des nouveaux
  recognizers (certains FP).
