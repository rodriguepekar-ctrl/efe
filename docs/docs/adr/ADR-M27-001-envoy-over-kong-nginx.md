# ADR-M27-001 : Envoy over Kong / NGINX+ as the in-cluster API gateway

- **Status** : Accepted
- **Date** : 2026-05-11
- **Deciders** : Eng lead, SRE lead, Platform architect
- **Consulted** : Security lead, Backend leads
- **Related** : EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md § 7.1

## Contexte

M27 introduit un API gateway in-cluster qui assume :
- TLS termination depuis Cloudflare Argo Tunnel,
- validation JWT au edge (offload depuis `efe-server`),
- rate-limit distribué via un service externe (gRPC),
- routing v1/v2 + transformations request/response,
- métriques + accès logs structurés,
- chaos injection (fault filter) pour les tests M27-pentest.

Trois candidats sérieux : **Envoy**, **Kong**, **NGINX+**.

## Options considérées

### Option 1 : Envoy ✅

- **Pour** :
  - gRPC + HTTP/1.1 + HTTP/2 + HTTP/3 natifs avec un seul code path.
  - xDS dynamic config (RDS pour routes per-tenant sans reload).
  - WASM filter (extensibilité long-terme M29 PII detection).
  - Adoption massive en prod (Istio, Lyft, AWS App Mesh, GCP).
  - Stats + access log JSON natif → Loki/Datadog en plug-and-play.
- **Contre** :
  - YAML config plus verbeuse que NGINX.
  - WASM toolchain encore en évolution (mitigation : Lua filter v1).

### Option 2 : Kong

- **Pour** : UI admin web, plugin marketplace large, communauté forte.
- **Contre** :
  - Postgres requis comme control plane → ajoute un point de panne.
  - Plugins haute-fréquence (rate-limit, JWT) en Lua → performance < Envoy.
  - gRPC support secondaire (HTTP/1.1 first-class).

### Option 3 : NGINX+ (paid)

- **Pour** : connaissance opérationnelle large, perf raw.
- **Contre** :
  - License payante.
  - Plugins en C ou JS modules ; gRPC second-class.
  - Pas de xDS dynamic config.

## Décision

**Option 1 retenue : Envoy** comme gateway in-cluster.

Justification :
1. Premier critère = gRPC + HTTP/2 natifs avec performance équivalente.
   Envoy gagne.
2. Deuxième critère = extensibilité future (M29 WAF + PII redaction
   sidecar, M28 webhooks delivery via Envoy lambda) → Envoy + WASM est
   le path standard.
3. Troisième critère = ops complexity. Envoy YAML est verbeux mais
   reproducible via Helm ; Kong PG control plane introduit un nouveau
   point de panne incompatible avec notre objectif de simplicité.

## Conséquences

### Positives
- Une seule technologie pour gRPC + REST + SSE.
- xDS permet route updates per-tenant sans reload (M29 forward-compat).
- Observabilité Prometheus + access log JSON natif.

### Négatives
- Le YAML Envoy est dense. Mitigation : Helm ConfigMap templating +
  schéma validé en CI via `envoy --mode validate`.
- WASM filter écosystème encore jeune. Mitigation : restant en Lua jusqu'à
  M29.

### Neutres
- Pas de support natif circuit-breaker per-tenant (Envoy le fait par
  cluster, pas par tenant). M27 accepte ce trade-off ; M29 ajoute un
  filter custom.

## Pivot scenarios

Si Envoy CPU footprint dépasse 50% par pod sur cluster prod :
- Pivot mineur : ajuster `concurrency` Envoy flag.
- Pivot majeur : revenir à Kong avec contraintes opérationnelles.

## Révision

Revue fin M30 (multi-region) — vérifier que Envoy xDS scale-out
fonctionne avec routing géo.
