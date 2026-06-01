# ADR-M30-001 : SigLIP-base-patch16-224 over CLIP / ViT / DINOv2

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M30_MULTIMODAL_ARCHITECTURE.md § 5

## Décision

SigLIP-base-patch16-224 comme encodeur vision M30.

## Justification

1. **License Apache 2.0** (Google) — commercial-friendly, pas de
   restriction d'usage contrairement à CLIP (OpenAI MIT mais avec
   clause "research only" parfois invoquée).
2. **Sigmoid loss vs softmax** — bien meilleur sur des batches
   inférieurs à 1k, ce qui est notre régime d'inférence (batch 1-8).
3. **Dimension 768** — match natif avec notre classifier head fine-
   tuné en M21, pas de projection supplémentaire.
4. **Latence Triton** — ~ 70 ms p99 sur A100 MIG 1g.10gb pour batch
   1, vs ~ 95 ms pour CLIP ViT-L/14.
5. **Multilingue par design** — `mSigLIP` (M30.1) ajoutera FR + EN
   sans changer d'architecture.

vs **DINOv2** : DINOv2 est self-supervised mais sans text-alignment,
donc inutile pour les use-cases image-to-text de l'agent runner.
vs **ViT-L/14 brut** : ViT non-aligné nécessiterait un fine-tune
chez nous pour aligner avec l'espace textuel.

## Conséquences

### Positives
- Modèle 397 M params → fits in 1g.10gb MIG slice (10 GB VRAM).
- Token throughput Triton : ~ 120 req/s par MIG slice à 50 % util.

### Négatives
- Pas de support natif pour les modalités "video" — il faudra un
  encoder dédié (M31 Video).
- SigLIP-large-patch14-384 serait 2 × meilleur sur certains tests
  internes mais 4 × le coût VRAM ; rejeté pour le MVP M30.
