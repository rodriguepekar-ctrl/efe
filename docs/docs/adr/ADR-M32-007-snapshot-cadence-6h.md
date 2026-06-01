# ADR-M32-007 : Cadence snapshot 6h + replay NATS pour le tail

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md § 8

## Décision

Snapshots S3 toutes les 6 h. Au redémarrage d'un pod, le replay
charge le dernier snapshot + replay les events NATS
`vector.upsert` / `vector.delete` depuis le timestamp du snapshot.

## Justification

1. **6 h = trade-off RPO / coût** :
   - RPO ~ 6 h max (event log NATS ferme le gap).
   - 4 snapshots/jour × 1 M docs × 4 KB/doc = 16 GB/jour S3.
     Acceptable à $0.023/GB-mois.
2. **NATS retention** — `vector.*` topics retiennent 24 h
   d'historique ⇒ tail de replay borné.
3. **Pod restart** — typiquement < 1 fois/semaine en steady state ;
   replay 60 s sur 1 M docs reste invisible côté SLO.

vs **snapshot après chaque insert** : trop coûteux S3 + lifecycle.
vs **snapshot 24 h** : RPO trop long pour SOC 2 BCDR.

## Conséquences

### Positives
- Storage cost prévisible.
- Recovery rapide.

### Négatives
- En cas de NATS down + pod restart, on perd la queue du dernier
  snapshot (≤ 6 h de updates). Couvre une situation rare.
