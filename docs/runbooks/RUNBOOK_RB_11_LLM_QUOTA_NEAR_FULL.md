# RB-11 — Quota tokens LLM tenant à 95 % du daily cap

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P3** (proactive notification) |
| MTTR cible | 4 h (préventif) |
| Fréquence attendue | 5-10 / mois |
| Last reviewed | 2026-05-09 |
| Owner primaire | Account Executive |
| Owner backup | LLM Ops |

## Symptômes

- Métrique : `efe_tenant_daily_tokens_used / efe_tenant_daily_tokens_cap > 0.95`
- Email tenant : « Approaching daily token limit »
- Alerte Prometheus `EfeTenantQuotaNearFull`

## Diagnostic

### 1. État budget tenant

```bash
efe-admin quotas get <tenant> --format json
```

### 2. Pattern de consommation

```sql
SELECT date_trunc('hour', ts) AS h,
       sum(total_tokens) AS tokens,
       count(*) AS calls
  FROM llm_call_audit
 WHERE tenant = '<tenant>'
   AND ts > now() - interval '24 hours'
 GROUP BY 1 ORDER BY 1 DESC LIMIT 24;
```

## Procédure

### Étape 1 : Notifier proactive (≤ 30 min)

Account Executive contacte tenant :

> Subject : EFE quota notification — daily token limit approaching
>
> Hi [contact],
>
> We've detected that your tenant `<tenant>` has used 95 % of its daily
> token cap (X / Y tokens). At current rate, the cap will be hit in ~Z
> hours.
>
> Options :
>   1. Upcap for 24h : we increase the cap, you continue. Pro-rated billing.
>   2. Throttle : the API will gracefully error with `BudgetExhausted`
>      until midnight UTC.
>
> Please reply to confirm your preference.

### Étape 2 : Si confirmation tenant pour up-cap

```bash
# Document the current cap before changing.
ORIG=$(efe-admin quotas get <tenant> --format json | jq .max_tokens_per_day)
echo "Original cap : $ORIG"

# Apply new cap.
efe-admin quotas set <tenant> --max-tokens-per-day <new-cap> --yes
```

Suivez avec un email de confirmation et trackez l'incident dans le CRM (upsell opportunity).

### Étape 3 : Si throttle

Pas d'action nécessaire — l'app auto-throttle quand cap hit (Mode-2/3 errors gracefully avec `BudgetExhausted`).

Documentation tenant :

> Your Mode-2 / Mode-3 calls will return `code: budget_exhausted` until
> midnight UTC. Mode-1 (anchor-only classify) continues to work normally.

### Étape 4 : Restore (J+1)

À midnight UTC, le compteur reset automatiquement. Restaurer le cap original si modifié temporairement :

```bash
efe-admin quotas set <tenant> --max-tokens-per-day "$ORIG" --yes
```

## Post-incident

- Pattern récurrent (> 3 fois / mois) → revue contractuelle avec tenant
- Long terme : alerter à 80 % au lieu de 95 % pour donner plus de temps de réaction
- Considérer cap hourly (vs daily) pour smoothing

## Procédure de validation

En staging, force cap=0.01 puis exécute 100 classify Mode-2 :

```bash
efe-admin quotas set staging-tenant --monthly-cap-usd 0.01 --yes
for i in {1..100}; do
  efe-admin classify-test staging-tenant --mode 2 || true
done
```

Doit déclencher l'alerte ≤ 10 min.

**Test trimestriel** : owner LLM Ops.

## Annexes

- Architecture cost governance : `docs/EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` § Cost
- Dashboard Grafana : `EFE Overview` → panel « Cost today (USD per tenant) »
