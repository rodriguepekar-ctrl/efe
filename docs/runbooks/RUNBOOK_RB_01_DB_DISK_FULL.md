# RB-01 — Disque Postgres saturé (> 85 %)

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P1** |
| MTTR cible | 30 min |
| Fréquence attendue | 1-2 / an / tenant production |
| Last reviewed | 2026-05-09 |
| Owner primaire | DBA on-call |
| Owner backup | SRE on-call |

## Symptômes

- Alerte Prometheus : `EfePostgresDiskFull`
- Métrique Grafana : panel « Postgres disk usage » > 85 %
- Log pattern : `WARNING: disk-only sort:` ou `ERROR: could not extend file`
- Page on-call : **OUI**, page DBA + SRE

## Diagnostic (≤ 5 min)

### 1. Identifier le cluster + l'instance impactés

```bash
kubectl get pvc -n efe -o json | jq -r '
  .items[] | select(.metadata.name|test("data-efe-postgres")) |
    "\(.metadata.name)\t\(.status.capacity.storage)"'
```

### 2. Calculer l'occupation par table (top 10)

```bash
kubectl exec -n efe efe-postgres-instance1-0 -c database -- \
  psql -U postgres -d efe -c "
    SELECT relname,
           pg_size_pretty(pg_total_relation_size(C.oid)) AS size
      FROM pg_class C
      LEFT JOIN pg_namespace N ON N.oid = C.relnamespace
     WHERE nspname = 'public'
     ORDER BY pg_total_relation_size(C.oid) DESC
     LIMIT 10;
  "
```

### 3. Vérifier l'autovacuum lag

```sql
SELECT relname, n_dead_tup, n_live_tup,
       round(100 * n_dead_tup / NULLIF(n_live_tup, 0), 1) AS dead_pct
  FROM pg_stat_user_tables
 WHERE n_dead_tup > 1000
 ORDER BY dead_pct DESC NULLS LAST
 LIMIT 10;
```

## Procédure de résolution

### Étape 1 : Stabiliser (≤ 5 min)

1. **Geler les inserts massifs** côté tenant impacté :
   ```bash
   efe-admin tenant suspend acme \
     --reason "RB-01: emergency disk pressure - bulk operations frozen" --yes
   ```
   ⚠️ Cela bloque uniquement les classify ; les SELECT continuent (lecture).

2. Snapshot RDS immédiat (sécurité) :
   ```bash
   aws rds create-db-snapshot \
     --db-instance-identifier efe-acme-postgres \
     --db-snapshot-identifier efe-acme-rb01-$(date +%s)
   ```

### Étape 2 : Diagnostiquer (≤ 10 min)

Identifier la cause racine probable parmi 3 cas :

- **Cas A** : croissance organique du `llm_call_audit` (M21+). Solution : ajouter du stockage.
- **Cas B** : autovacuum lag accumulant des dead tuples. Solution : VACUUM FULL ciblé.
- **Cas C** : bug applicatif émettant des doublons. Solution : pause + investigation.

### Étape 3 : Corriger

#### Cas A — étendre le storage RDS (no-downtime)

```bash
CURRENT=$(aws rds describe-db-instances \
  --db-instance-identifier efe-acme-postgres \
  --query 'DBInstances[0].AllocatedStorage' --output text)
NEW=$((CURRENT * 3 / 2))

aws rds modify-db-instance \
  --db-instance-identifier efe-acme-postgres \
  --allocated-storage $NEW \
  --apply-immediately
```

Postgres reprend le service en 5-10 min après l'extension.

#### Cas B — VACUUM FULL ciblé (durant maintenance window)

```sql
VACUUM FULL VERBOSE llm_call_audit;
```

Cette opération **lock la table** pendant son exécution (5-30 min selon volume).

#### Cas C — escalation SRE + dev

Mettre l'application en read-only mode :

```bash
kubectl patch -n efe deploy/efe-server --type merge \
  -p '{"spec":{"template":{"spec":{"containers":[{"name":"efe-server","env":[{"name":"EFE_READ_ONLY_MODE","value":"true"}]}]}}}}'
```

### Étape 4 : Vérifier

```bash
kubectl exec -n efe efe-postgres-instance1-0 -c database -- df -h /pgdata
```

Cible : **< 70 %** post-correction.

Réactiver les écritures :

```bash
efe-admin tenant update acme --max-rps 100 --yes
```

### Étape 5 : Communiquer

- Slack `#efe-incidents` : durée + RCA preview
- Statuspage : update si > 30 min
- Email tenant Acme : si suspended pendant > 15 min

## Post-incident

- Postmortem si MTTR > 1 h
- Action items :
  - Tuner `autovacuum_vacuum_scale_factor` à 0.05 (default 0.2) sur tables M21+
  - Augmenter alerte à 75 % (vs 85 %) pour anticiper

## Procédure de validation (Game Day trimestriel)

Reproduire en staging :

```bash
kubectl exec -n efe-staging efe-postgres-instance1-0 -c database -- \
  psql -U postgres -d efe -c "
    INSERT INTO llm_call_audit (tenant, mode, sentence_hash,
                                provider_used, total_tokens, cost_usd,
                                duration_ms, label, confidence,
                                anchors_used, seq, prev_hmac, hmac)
    SELECT 'staging', 'MODE_2', md5(random()::text),
           'openai', 100, 0.001,
           50, 'test', 0.85,
           '{}'::text[], i, '0', md5(random()::text)
      FROM generate_series(1, 50000000) AS i;
  "
```

Doit déclencher l'alerte Prometheus dans ≤ 5 min.

**Test trimestriel** : premier mardi du trimestre, owner DBA Lead.

## Annexes

- Dashboard Grafana : `EFE Overview` → panel « Postgres disk usage »
- Documentation PGO : https://access.crunchydata.com/documentation/postgres-operator/v5/
- AWS RDS storage scaling : https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/USER_PIOPS.StorageTypes.html
