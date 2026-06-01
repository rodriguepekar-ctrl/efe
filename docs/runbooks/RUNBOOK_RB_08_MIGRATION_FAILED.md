# RB-08 — Migration DB échouée (sqlx migrate run failure)

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P1** (blocking deployment) |
| MTTR cible | 1 h |
| Fréquence attendue | < 1 / 6 mois (release-only) |
| Last reviewed | 2026-05-09 |
| Owner primaire | DBA on-call |
| Owner backup | Backend Lead |

## Symptômes

- Helm post-install hook Job `*-migrate` en `Failed`
- Logs : `sqlx migration X failed: <error>`
- Nouveaux pods efe-server en `CrashLoopBackOff` (DB schema mismatch)
- Helm release status : `failed`

## Diagnostic

### 1. Logs Job migrate

```bash
kubectl logs -n efe job/efe-stack-migrate
```

### 2. État table `_sqlx_migrations`

```sql
SELECT version, description, success, execution_time, installed_on
  FROM _sqlx_migrations
 ORDER BY version DESC LIMIT 10;
```

### 3. Migration appliquée partiellement ?

```sql
SELECT pid, query_start, query
  FROM pg_stat_activity
 WHERE state != 'idle'
   AND application_name LIKE '%sqlx%';
```

## Procédure de résolution

### Étape 1 : Stabiliser

Bloquer le rollout des nouveaux pods :

```bash
kubectl rollout pause -n efe deploy/efe-server
kubectl rollout pause -n efe deploy/mcp-server
```

### Étape 2 : Diagnostiquer la cause

| Cas | Symptôme |
|-----|----------|
| **A** : conflit data | NOT NULL ajouté avec data null pré-existante |
| **B** : SQL syntax | erreur dans le fichier migration |
| **C** : timeout | longue migration sur grosse table |
| **D** : permission | role `efe_admin` manque permission |

### Étape 3 : Corriger

#### Cas A — data-fix

```sql
-- Backfill nulls before NOT NULL constraint applies.
UPDATE big_table SET new_col = 'default' WHERE new_col IS NULL;
```

Puis re-run :

```bash
kubectl delete job -n efe efe-stack-migrate
helm upgrade efe-stack ./charts/efe-stack --reset-then-reuse-values
```

#### Cas B — revert via Helm rollback

```bash
helm history efe-stack
helm rollback efe-stack <previous-revision>
# Then : fix SQL upstream, push image, re-deploy.
```

#### Cas C — étendre les timeouts

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --set migrations.backoffLimit=10 \
  --set migrations.activeDeadlineSeconds=3600 \
  --reset-then-reuse-values
```

#### Cas D — grant permissions

```sql
GRANT CONNECT ON DATABASE efe TO efe_admin;
GRANT USAGE ON SCHEMA public TO efe_admin;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO efe_admin;
```

### Étape 4 : Vérifier

```bash
kubectl get job -n efe efe-stack-migrate
# Status : Complete (Succeeded=1)

kubectl rollout resume -n efe deploy/efe-server
kubectl rollout resume -n efe deploy/mcp-server
kubectl wait -n efe pod -l app.kubernetes.io/name=efe-server \
  --for=condition=Ready --timeout=120s
```

### Étape 5 : Communiquer

- Slack `#efe-incidents`
- Statuspage si downtime > 15 min
- Email tenants si downtime > 30 min

## Post-incident

- Test régression : ajouter le scenario data-fix dans pre-deploy CI
- Améliorer la migration : la rendre idempotente + supportant data null
- Postmortem requis si MTTR > 1 h

## Procédure de validation

En staging, simuler une migration échouée :

```bash
kubectl exec -n efe-staging postgres-0 -c database -- \
  psql -U postgres -d efe -c "
    -- Inject conflicting data before next migration runs.
    INSERT INTO _sqlx_migrations (version, description, success, ...)
    VALUES (9999, 'simulated_failure', FALSE, NULL, ...);
  "
```

Helm install nouveau release ; doit échouer avec message clair.

**Test semestriel** : owner DBA + Backend Lead.

## Annexes

- sqlx docs : <https://docs.rs/sqlx/latest/sqlx/migrate/>
- Migrations source : `crates/efe-server/migrations/`
- Helm chart : `charts/efe-stack/templates/migrations-job.yaml`
