# RB-12 — Failover Postgres déclenché

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P1** |
| MTTR cible | 30 min |
| Fréquence attendue | < 1 / an / cluster |
| Last reviewed | 2026-05-09 |
| Owner primaire | DBA |
| Owner backup | SRE |

## Symptômes

- Logs efe-server : `connection refused` ou `read-only transaction`
- AWS RDS event : `Multi-AZ failover completed`
- GCP Cloud SQL event : `failover from primary to standby`
- Azure PG event : `HA failover completed`
- Pods efe-server temporairement en CrashLoopBackOff puis recovery

## Diagnostic

### AWS

```bash
aws rds describe-events \
  --source-identifier efe-acme-postgres \
  --source-type db-instance \
  --duration 60 | jq '.Events[]'
```

### GCP

```bash
gcloud sql operations list --instance=efe-acme-postgres --limit=10
```

### Azure

```bash
az postgres flexible-server show \
  --resource-group efe-acme-rg \
  --name efe-acme-postgres \
  --query 'highAvailability'
```

## Procédure

### Étape 1 : Vérifier que les apps reconnectent

```bash
kubectl logs -n efe deploy/efe-server --tail=50 \
  | grep -E "(reconnect|connection|connected)"
```

Si pods en CrashLoopBackOff > 2 min → restart :

```bash
kubectl rollout restart -n efe deploy/efe-server deploy/mcp-server
```

### Étape 2 : Vérifier intégrité audit chain

```bash
kubectl exec -n efe deploy/efe-server -- \
  /app/bin/efe-verify-audit-chain --tenant=<tenant>
```

Doit retourner OK. Si broken → cf. RB-03 immédiatement (P0 escalation).

### Étape 3 : Plan de failback

| Cloud | Comportement |
|-------|--------------|
| AWS Multi-AZ | Auto-réplication standby ↔ primary, pas d'action |
| GCP HA Regional | Auto-failback quand zone primaire reviendra |
| Azure ZoneRedundant | Auto-failback |

Pas d'action manuelle nécessaire dans tous les cas.

### Étape 4 : Vérifier post-failover

#### Latency check

```bash
curl -s "http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,sum(rate(efe_classify_duration_ms_bucket[5m]))%20by%20(le))"
```

Cible : retour à baseline en ≤ 5 min.

#### Replication lag (si applicable)

##### AWS

```bash
aws rds describe-db-instances \
  --db-instance-identifier efe-acme-postgres \
  --query 'DBInstances[0].StatusInfos'
```

##### GCP

```sql
SELECT * FROM pg_stat_replication;
```

##### Azure

```bash
az monitor metrics list \
  --resource $(az postgres flexible-server show \
    --resource-group efe-acme-rg --name efe-acme-postgres --query id -o tsv) \
  --metric replica_lag_in_seconds
```

### Étape 5 : Communication

- Statuspage : « Database failover completed at HH:MM, service restored » (typique downtime < 1 min)
- Slack `#efe-incidents`
- Pas d'email tenant si downtime < 5 min

## Post-incident

- Postmortem requis si downtime > 5 min
- Vérifier qu'aucune écriture n'a été perdue (compare audit_id séquence pre / post)
- Documenter cause racine (zone outage, hardware fail, planned maintenance)

## Procédure de validation (Game Day annuel)

### AWS

```bash
aws rds reboot-db-instance \
  --db-instance-identifier efe-staging-postgres \
  --force-failover
```

### GCP

```bash
gcloud sql instances failover efe-staging-postgres
```

### Azure

```bash
az postgres flexible-server restart \
  --resource-group efe-staging-rg \
  --name efe-staging-postgres \
  --failover Forced
```

Doit déclencher reconnexion automatique des pods en ≤ 60 s.

**Test annuel** : owner DBA. Programmé hors heures de bureau.

## Annexes

- AWS RDS Multi-AZ : <https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/Concepts.MultiAZ.html>
- GCP Cloud SQL HA : <https://cloud.google.com/sql/docs/postgres/high-availability>
- Azure PG HA : <https://learn.microsoft.com/en-us/azure/postgresql/flexible-server/concepts-high-availability>
- Architecture audit chain : `docs/EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` § Audit
