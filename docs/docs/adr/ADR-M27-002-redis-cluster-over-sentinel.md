# ADR-M27-002 : Redis Cluster sharded over Redis Sentinel

- **Status** : Accepted
- **Date** : 2026-05-11
- **Deciders** : Eng lead, SRE lead
- **Consulted** : Backend lead
- **Related** : EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md § 8

## Contexte

M27 introduit un cache plane partagé entre `efe-server` et
`efe-llm-router` pour :
- embedding cache (SHA256 → Vec<f32>, 7d TTL),
- LLM response cache (SHA256 → JSON, 1h TTL, HMAC-signed),
- per-tenant quota counters (INCR + EXPIRE par minute),
- Envoy ratelimit token bucket.

Deux topologies envisageables : Redis Sentinel (HA simple) vs Redis
Cluster (sharded + HA).

## Options considérées

### Option 1 : Redis Cluster (sharded 3 master + 3 replica) ✅

- **Pour** :
  - Throughput horizontal : 3× la capacité écriture de 1 master.
  - Mémoire horizontale : 3× la capacité de cache.
  - Failover automatique sans Sentinel séparé.
- **Contre** :
  - Re-balancing pendant l'ajout de noeud demande attention.
  - Scripts Lua cross-key impossibles (sauf si hash tags `{...}`).
  - Mitigation : tous nos scripts touchent une seule clé.

### Option 2 : Redis Sentinel (1 master + 2 replica + 3 sentinels)

- **Pour** :
  - Plus simple opérationnellement.
  - Pas de re-balancing.
  - Scripts Lua cross-key OK.
- **Contre** :
  - Throughput écriture plafonne au single master.
  - Mémoire plafonne au single master.
  - Sentinel = composant additionnel à monitorer.

### Option 3 : Managed Redis (ElastiCache / MemoryStore / Upstash)

- **Pour** : zéro ops.
- **Contre** :
  - Coût mensuel.
  - Lock-in cloud-provider.
  - Re-évaluer après M27 livré (P3 sprint).

## Décision

**Option 1 retenue : Redis Cluster sharded 3+3**.

Justification :
1. À 5+ design partners en production, throughput LLM cache ≈ 1000 req/s
   sustained ; Sentinel single-master saturerait vers 50k ops/s
   (Redis benchmarks).
2. Mémoire : embedding cache 384-f32 × 100k unique texts ≈ 153 MB ; en
   ajoutant LLM cache + quota counters → 1-2 GB par master = 6 GB cluster
   utilisable. 3× headroom.
3. Re-balancing rare (only at scale-up).

## Conséquences

### Positives
- Throughput scale-out à mesure que les tenants arrivent.
- Failover automatique sans Sentinel pod additionnel.

### Négatives
- Aucun script Lua cross-key. Mitigation : architecture clés single-key.
- Recovery from full cluster loss = restaurer 6 snapshots RDB. Acceptable.

### Neutres
- Connection logic dans `efe-cache::client` doit gérer redirections
  MOVED/ASK. Géré par `redis::cluster_async::ClusterClient`.

## Pivot scenarios

Si l'opérationnel Cluster s'avère trop lourd à < 10 tenants :
- Migrer à managed Upstash Redis (Cluster mode compatible).
- Code `efe-cache` inchangé grâce à l'abstraction `CacheClient`.

## Révision

Revue fin M30 — observable cluster slot-balance + RDB restore time
quarterly.
