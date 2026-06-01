# RB-07 — Latence p99 classify > 2× SLO sur 30 min

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P2** |
| MTTR cible | 1 h |
| Fréquence attendue | 2-5 / mois |
| Last reviewed | 2026-05-09 |
| Owner primaire | Perf Ops |
| Owner backup | SRE |

## Symptômes

- Alerte Prometheus : `EfeClassifyP99HighLatency`
- Grafana panel « p99 Latency » → > 500 ms (vs 250 ms baseline)
- Logs efe-server : `request took 1234ms` répétés
- Tenant complaints emails : « API très lente »

## Diagnostic (≤ 5 min)

### 1. Identifier la dimension dégradée

```bash
kubectl exec -n efe deploy/efe-server -- \
  curl -s http://localhost:9090/metrics \
  | grep -E "efe_classify_duration_ms_(bucket|sum|count)" | head -20
```

### 2. Repository des slow queries Postgres

```sql
SELECT query, calls, mean_exec_time, max_exec_time
  FROM pg_stat_statements
 WHERE mean_exec_time > 100
 ORDER BY mean_exec_time DESC
 LIMIT 10;
```

### 3. Vérifier la pression mémoire pods

```bash
kubectl top pods -n efe --sort-by=memory
```

### 4. Vérifier le hit ratio cache

```bash
kubectl exec -n efe deploy/efe-server -- \
  curl -s http://localhost:9090/metrics | grep efe_cache_hit_ratio
```

## Procédure de résolution

### Étape 1 : Stabiliser

Forcer scale-up manuel :

```bash
kubectl scale -n efe deploy/efe-server --replicas=12
```

### Étape 2 : Identifier la cause racine

| Cause probable | Indicateur | Solution |
|----------------|------------|----------|
| LLM provider lent | provider p99 > 2× baseline | Cf. RB-02 (breaker) |
| DB requêtes slow | `pg_stat_statements` top | Add index ou `ANALYZE` table |
| Cache Redis miss | hit ratio < 70 % | Investigate cache invalidation |
| Tenant noisy-neighbor | concentration RPS sur 1 tenant | Augment per-tenant rate-limit |
| Garbage collection | mem RSS > 80 % limit | Bump memory limit |
| Insufficient nodes | HPA at maxReplicas | Increase node group max |

### Étape 3 : Corriger

#### Cas DB

```sql
ANALYZE llm_call_audit;
CREATE INDEX CONCURRENTLY IF NOT EXISTS ix_audit_tenant_ts
  ON llm_call_audit (tenant, ts DESC);
```

#### Cas cache

```bash
kubectl exec -n efe deploy/efe-server -- \
  curl -X POST http://localhost:8081/internal/cache/stats
# If miss ratio is high : check for invalidation storms.
```

#### Cas noisy-neighbor

```bash
efe-admin quotas set <tenant> --max-rps 50 --yes
```

#### Cas memory pressure

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --set efe-server.resources.limits.memory=8Gi --reuse-values
```

### Étape 4 : Vérifier

Attendre 10 min ; p99 doit revenir < SLO :

```bash
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,sum(rate(efe_classify_duration_ms_bucket[5m]))%20by%20(le))"
```

### Étape 5 : Communiquer

- Statuspage si > 30 min : « Performance degradation investigated »
- Slack `#efe-incidents`
- Email tenants impactés si > 1 h

## Post-incident

- Si pattern récurrent (même cause) → ajout permanent capacité ou fix code
- RCA dans 48 h pour P2 récurrent

## Procédure de validation (Game Day trimestriel)

Force latence en staging :

```bash
kubectl exec -n efe-staging deploy/efe-server -- \
  curl -X POST http://localhost:8081/internal/inject-latency \
  -d '{"ms":600,"duration":"5m"}'
```

Doit déclencher l'alerte Prometheus dans ≤ 10 min.

**Test trimestriel** : premier mardi du trimestre, owner Perf Ops.

## Annexes

- Architecture cache Redis : `docs/EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` § Cache
- Dashboard Grafana : `EFE Overview` → panel « Latency distribution »
