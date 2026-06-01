# ADR-M30-003 : Triton HTTP, gRPC reporté à M30.1

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M30_MULTIMODAL_ARCHITECTURE.md § 6.3

## Décision

M30 embarque uniquement le client **HTTP** vers Triton. Le client
gRPC est reporté à M30.1.

## Justification

1. **Volume MVP** — ≤ 50 RPS par région à la commercial release.
   HTTP/1.1 keep-alive suffit ; le gain gRPC (~ 2 × throughput
   par-pod) est non-critique.
2. **Pas de protobuf dep dans `efe-multimodal`** — éviter d'ajouter
   `prost`/`tonic` au crate qui est volontairement minimal.
3. **Plus simple à debug** — `curl /v2/models/.../infer` marche en
   shell. Inestimable pour le on-call.
4. **JSON inline tensors** — acceptable pour 768-dim vision /
   384-dim audio embeddings (< 5 KB body). Sera remplacé par
   binary attachments en M30.1.

## Conséquences

### Positives
- Crate `efe-multimodal` = ~ 1 500 LOC, builds en 1 s isolation.
- Mock testing trivial — `wiremock` peut simuler Triton.

### Négatives
- ~ 30 % overhead p99 vs gRPC pour les vision encodings dûr le
  JSON tensor inline.
- M30.1 devra refactorer `triton_client.rs` — mais l'interface
  publique (`infer`) reste stable.

## Migration M30.1

Le swap HTTP → gRPC sera **interne** au crate (impl detail). Les
appels `client.infer(model, modality, &req)` ne changent pas. Le
swap pourra se faire par feature flag :
`features = ["grpc"]`.
