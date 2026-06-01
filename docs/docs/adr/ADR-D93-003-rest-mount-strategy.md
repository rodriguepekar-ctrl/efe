# ADR-D93-003 : Mount `/v1/agent/*` via efe-server, not a separate gateway pod

- **Status** : Accepted
- **Date** : 2026-05-10
- **Deciders** : Eng lead, Backend lead, Platform architect
- **Consulted** : SRE, DevOps, Frontend lead
- **Supersedes** : —
- **Superseded by** : —
- **Related** : ADR-001 (tech stack), M26 architecture § 7 (REST gateway),
  M25 architecture § 5 (AgentRuntimeService gRPC contract)

## Contexte

Le code de la passerelle REST `/v1/agent/*` (M26) vit dans le crate
`efe-server` sous `crates/efe-server/src/v1/agent/` :

- `mod.rs` — router axum + `AgentApiState`
- `auth.rs` — extraction `AuthCtx` (OIDC ES256 middleware)
- `errors.rs` — `AgentApiError` + mapping HTTP status codes
- `adapter.rs` — `AgentRuntimeServiceAdapter` (impl de
  `AgentRuntimeFacade`, créé D93 sous C-1)
- 8 handlers : `runs`, `approvals`, `templates`, `cost`, `dpia`,
  `graduate`, `post_market`, etc.

Au moment d'intégrer ce code dans le binaire serveur, deux topologies
de déploiement étaient envisageables :

### Topologie A : sub-router intégré dans efe-server

- Le serveur `efe-server` expose **2 listeners** dans le même pod :
  - gRPC tonic sur `:8080` (existing, M0-M25)
  - REST axum `/v1/agent/*` sur `:8081` (nouveau, M26)
- Le router REST appelle `AgentRuntimeService` **dans le même
  processus** via la trait `AgentRuntimeFacade` implémentée par
  l'adapter.
- Une seule image Docker, une seule deployment Helm.

### Topologie B : pod gateway dédié

- Un nouveau crate binaire `efe-agent-gateway` qui ne contient QUE
  le REST router + adapter.
- Le pod gateway parle à `efe-agent-runner` (le controller M25)
  **via tonic gRPC mTLS** sur le mesh K8s.
- Deux images Docker, deux deployments Helm, deux SLO panels Grafana.

### Topologie C : sub-router intégré + appel gRPC vers efe-agent-runner

- Le router REST vit dans le binaire `efe-server` (donc co-deployé avec
  le coeur classification).
- Mais l'adapter ne fait PAS un appel in-process ; il fait un appel
  tonic gRPC mTLS vers `efe-agent-runner` (le controller M25 dans son
  propre pod).
- C'est un hybride entre A et B.

## Critères de décision

| Critère | Topologie A | Topologie B | Topologie C |
|---|:--:|:--:|:--:|
| Isolation crash agent-runtime ↔ classification | ❌ même pod | ✅ pods séparés | ✅ pods séparés |
| Scale-out indépendant | ❌ couplé | ✅ découplé | ✅ découplé |
| Auth chain (OIDC + mTLS) cohérent | ✅ unique | ✅ unique mais 2-hop | ✅ unique 2-hop |
| Cold-start latency | ✅ in-process | ❌ +1 RTT mesh | ❌ +1 RTT mesh |
| Coût opérationnel (1 pod vs 2) | ✅ minimal | ❌ +1 deployment | ✅ même qu'A |
| Helm chart complexity | ✅ idem M25 | ❌ +1 chart ou +sub-chart | ✅ idem M25 |
| Surface de test E2E | ✅ trivial | ❌ requires gRPC mesh up | ❌ requires gRPC mesh up |
| Compatible "STABLE channel" architecturale (séparation lecture/écriture) | ❌ couplé | ✅ idéal | ✅ idéal |
| Risque sandbox-escape leak vers REST | ✅ contrôlé via adapter map_err | ✅ contrôlé idem | ✅ contrôlé idem |

## Options considérées

### Option 1 : Topologie A (sub-router intégré, appel in-process)

- **Pour** : zéro RTT supplémentaire, une seule image, debug local
  simple.
- **Contre** : couple le crash de `efe-agent-runner` avec celui de
  `efe-server` (1 pod restart fait perdre LE quorum REST + gRPC
  classification). Anti-pattern de séparation des préoccupations
  AI Act Art. 14 / Art. 26 (l'agent runtime est high-risk ; le
  service classification ne l'est pas).

### Option 2 : Topologie B (pod gateway dédié)

- **Pour** : isolation maximale.
- **Contre** : 2 nouvelles images, 2 nouveaux deployments, complexité
  Helm explose (chart de chart de chart). Le binaire `efe-agent-gateway`
  serait composé à 90% de re-imports du code de `efe-server` —
  duplication pénible.

### Option 3 : Topologie C (sub-router intégré + appel gRPC mesh) ✅

- **Pour** : reste sur 1 binaire (efe-server) et 2 deployments (le
  controller agent-runner reste indépendant en pod séparé) ; isolation
  crash : si `efe-agent-runner` tombe, `efe-server` REST renvoie un
  503 avec corps opaque ; mTLS sur le mesh continue de garantir
  l'authentification ; debug local possible via `EFE_AGENT_RUNTIME_INPROCESS=true`
  qui force le bypass tonic et l'appel direct (utile pour tests
  d'intégration cargo).
- **Contre** : 1 RTT mesh supplémentaire par REST call. Atténué :
  mesh est intra-cluster (< 1 ms p95) ; les appels REST agent runtime
  sont **non-hot-path** par construction (cost cap, graduation
  workflow, DPIA — toutes opérations à fréquence basse).

## Décision

**Option 3 retenue : Topologie C — sub-router intégré dans efe-server
+ appel tonic gRPC mTLS vers efe-agent-runner**.

Mise en œuvre D93 :

1. Le crate `efe-server` déclare en `Cargo.toml` :
   ```toml
   efe-agent-runner = { path = "../efe-agent-runner" }
   ```
   uniquement pour **importer la trait `AgentRuntimeFacade`** et les
   types d'erreur publics ; PAS pour appeler le service en in-process
   en production.
2. L'adapter `AgentRuntimeServiceAdapter` accepte au constructeur
   un `Arc<AgentRuntimeService>` qui peut être :
   - soit construit in-process (pour tests cargo `--features inprocess`),
   - soit un client tonic gRPC qui implémente la même trait au-dessus
     du transport mesh (configuration production par défaut).
3. La sélection se fait via env var `EFE_AGENT_RUNTIME_TRANSPORT` :
   - `inprocess` — tests locaux + smoke
   - `grpc` (default prod) — appelle `agent-runtime-grpc.svc.cluster.local:8443`
4. Helm chart `efe-server` : nouvelle env var
   `EFE_AGENT_RUNTIME_TRANSPORT=grpc` + nouvelle env
   `EFE_AGENT_RUNTIME_GRPC_ADDR` pointée vers le service mesh du
   controller.
5. `efe-agent-runner` continue d'exposer son service tonic sur le
   même port qu'avant (M25) ; aucun changement côté controller.

## Conséquences

### Positives

- **Isolation crash** : si `efe-agent-runner` est OOM-killed, les
  endpoints `/v1/agent/*` renvoient 503 ; les endpoints
  classification (`/v1/classify`, `/v1/embed`, etc.) restent
  pleinement disponibles. Le SLO classification (99.99% uptime)
  n'est PAS dégradé par un incident agent-runtime.
- **Scale-out indépendant** : on peut scaler `efe-server` (8 répliques
  pour classification heavy) sans scaler le controller agent
  (`replicas: 2` HA + leader election dans `efe-agent-runner`).
- **Test local simple** : `cargo test --features inprocess` reste
  fonctionnel sans déployer un mesh.
- **Compatible "STABLE channel"** : l'architecture mesh sépare le
  high-risk runtime (DPIA, sandbox, cost cap) du low-risk
  classification, conformément au plan de gouvernance AI Act Art. 26.

### Négatives

- **+1 RTT mesh sur les REST calls** : compensé par le fait que
  ces calls sont non-hot-path (cost cap, graduation, DPIA, post-market
  — fréquence < 10/s p95 cluster-wide).
- **Couplage compile-time** : `efe-server` dépend en `Cargo.toml`
  de `efe-agent-runner` pour importer la trait. Si un futur sprint
  refait le découpage des crates, il faudra extraire la trait dans
  un crate `efe-agent-runtime-api` séparé. Documenté comme dette
  technique mineure ; à revoir M28.
- **mTLS mesh requis** : Linkerd ou Istio doit être installé.
  Documenté dans le runbook M26 § 1.5 (cluster prerequisites).

### Neutres

- L'observabilité reste cohérente : un trace OpenTelemetry traverse
  `efe-server` → `efe-agent-runner` avec un `traceparent` propagé.
- La séparation cosign-signature reste granulaire (cosign signe
  `efe-server` ET `efe-agent-runner` indépendamment).

## Alternatives envisagées en pivot

Si l'Option 3 dégrade la performance REST en deçà du SLO M26 (P95 < 500ms) :

- **Pivot mineur** : passer en `inprocess` pour les endpoints
  haute-fréquence (live tail SSE) tout en gardant gRPC pour le reste.
  Faisable via un mapping endpoint-par-endpoint dans l'adapter.
- **Pivot majeur** : rebasculer en Topologie A et accepter le risque
  de couplage crash. Reverter cet ADR. Documenter le pivot dans un
  ADR-D93-003-rev2.

## Liste exacte des changements (D93)

```
crates/efe-server/Cargo.toml             (dep efe-agent-runner = path)
crates/efe-server/src/v1/agent/mod.rs   (build_agent_state + build_agent_router)
crates/efe-server/src/v1/agent/adapter.rs (NEW — AgentRuntimeServiceAdapter)
charts/efe-server/templates/deployment.yaml  (env EFE_AGENT_RUNTIME_TRANSPORT)
charts/efe-server/values.yaml            (agentRuntimeTransport default "grpc")
docs/docs/adr/ADR-D93-003-rest-mount-strategy.md  (ce document)
```

## Critère de levée

L'ADR reste accepté tant que :

1. Le SLO REST `/v1/agent/*` reste sous P95 < 500ms.
2. Aucun incident production de type "couplage crash" entre
   `efe-server` et `efe-agent-runner` n'est observé.
3. Le test de chaos "kill controller pod" → "REST renvoie 503 clean,
   classification reste up" passe en DR drill trimestriel.

Si ces 3 critères tiennent à 12 mois, on peut considérer l'ADR
comme **éprouvée** et bloquer toute reversion vers la Topologie A.

## Révision

Revue à la **fin du sprint M27** (post-first-paying-customer). Critères :

- Latence REST `/v1/agent/*` mesurée en prod : P95 effective ?
- Combien d'incidents `efe-agent-runner` ont été contenus sans
  affecter `efe-server` ?
- Le déploiement Helm reste-t-il simple pour les nouveaux clusters ?

Si métriques conformes, fermer la revue. Sinon, ouvrir un ADR
successeur.

## Référence

- M26 architecture, § 7 — REST gateway design
- M25 architecture, § 5 — AgentRuntimeService gRPC contract
- M26 assembly verification architecture
  (`docs/EFE_M26_ASSEMBLY_VERIFICATION_ARCHITECTURE.md` § 10)
- AI Act Art. 14 (human oversight via approval workflow)
- AI Act Art. 26 (post-market monitoring isolation)
