# RB-05 — Tenant `BudgetExhausted` non récupérable

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P3** |
| MTTR cible | 4 h |
| Fréquence attendue | 2-5 / mois |
| Last reviewed | 2026-05-09 |
| Owner primaire | Account Executive |
| Owner backup | LLM Ops |

## Symptômes

- Alerte Prometheus : `EfeTenantBudgetExhausted`
- Logs efe-server : `BudgetExhausted { kind: "monthly_cap" }` répétés
- Email tenant : « Mode-2/3 calls failing »

## Diagnostic

### 1. État budget tenant

```bash
efe-admin quotas get acme --format json
```

### 2. Consommation réelle (30j)

```sql
SELECT date_trunc('day', ts) AS d, sum(cost_usd) AS spent
  FROM llm_call_audit
 WHERE tenant = 'acme'
   AND ts > now() - interval '30 days'
 GROUP BY 1 ORDER BY 1 DESC;
```

## Procédure

### Étape 1 : Comprendre

Comparer consommation vs cap mensuel :

| Cas | Diagnostic | Action |
|-----|------------|--------|
| Consommation > 95 % cap | Limite normale atteinte | Account Executive — upsell |
| Consommation < 95 % mais erreurs | Bug | Investigation |

### Étape 2 : Cas normal (consommation ≈ cap)

1. Account Executive contacte tenant pour upsell ou augmentation cap.
2. Une fois validé contractuellement :

   ```bash
   efe-admin quotas set acme \
     --monthly-cap-usd 5000 \
     --max-tokens-per-day 5000000 \
     --yes
   ```

### Étape 3 : Cas bug

#### A. Vérifier les locks Postgres orphelins

```sql
SELECT * FROM pg_locks WHERE NOT granted;
```

Si locks suspects : restart efe-server pods.

```bash
kubectl rollout restart -n efe deploy/efe-server
```

#### B. Vérifier intégrité de la table de tracking

```sql
SELECT * FROM tenant_llm_budget WHERE tenant_id = 'acme';
```

Si corrompue : recalculer depuis `llm_call_audit`.

```sql
WITH actual_spend AS (
  SELECT tenant, sum(cost_usd) AS spent
    FROM llm_call_audit
   WHERE ts >= date_trunc('month', now())
     AND tenant = 'acme'
   GROUP BY tenant
)
UPDATE tenant_llm_budget tlb
   SET monthly_spent_usd = a.spent,
       updated_at = now()
  FROM actual_spend a
 WHERE tlb.tenant_id = a.tenant;
```

## Communication

- Email tenant si Mode-2/3 down > 1 h
- Pas d'escalation automatique (P3)

## Procédure de validation

En staging, force `monthly_cap = 0.01` puis exécute 100 classify Mode-2 :

```bash
efe-admin quotas set acme-staging --monthly-cap-usd 0.01 --yes
for i in {1..100}; do
  efe-admin classify-test acme-staging --mode 2 || true
done
```

Doit déclencher l'alerte ≤ 10 min.

## Annexes

- Architecture cost governance : `docs/EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` § Cost
- Dashboard Grafana : `EFE Overview` → panel « Cost today (USD per tenant) »
