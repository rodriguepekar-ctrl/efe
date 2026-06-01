# ADR-M30-005 : Budget cardinalité Prometheus pour les métriques M30

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M30_MULTIMODAL_ARCHITECTURE.md § 6.4

## Décision

Aucun label de métrique M30 ne porte des valeurs de cardinalité
illimitée (tenant_id, customer_id). Les labels sont :

- `modality` ∈ {vision, audio, long_text, fusion}                 — 4
- `outcome`  ∈ {ok, err}                                          — 2
- `region`   ∈ {eu-west-1, eu-west-3} (phase 3)                   — 2
- `model_name` ∈ {siglip_base_patch16_224, whisper_tiny_encoder}  — 2
- `kind` ∈ {transport, timeout, bad_response}                     — 3
- `reason` (rejections) ∈ {too_large, invalid_format, …}          — 6
- `strategy` ∈ {concat_linear, weighted_mean, clip_joint}         — 3

Budget per histogram : ≤ 16 séries actives. 15 histogrammes ×
16 = 240 séries totales. Bien dans le budget single-shard
Prometheus (1 M séries).

## Justification

1. **Cardinalité tenant** — interdite. Le mappage tenant → erreur
   passe par les logs structurés (Loki), pas par Prometheus.
2. **Cardinality bombs** historiques — Sprint D13 a vu un
   incident où `tenant_id` avait été ajouté en label, faisant
   exploser Prometheus à 12 M séries.
3. **PromQL ergonomique** — la plupart des dashboards M30 ne font
   pas de drill-down per tenant, ils filtrent par modalité.

## Conséquences

### Positives
- Le canari Prometheus a marge confortable (taux d'append ~ 100
  séries/sec, headroom 100 ×).
- Pas de risque d'overload cardinality au lancement commercial.

### Négatives
- Pour drill-down per-tenant : passer par Loki (logs) ou par PG
  (`multimodal_inference_audit`).
- Les dashboards Grafana ne montrent pas le tenant en hover, il
  faut cliquer "Open in Loki".
