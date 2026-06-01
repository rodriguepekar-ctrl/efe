# RB-02 — Circuit breaker LLM ouvert > 5 min

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P2** |
| MTTR cible | 15 min |
| Fréquence attendue | 5-10 / mois (provider issues, rate limits) |
| Last reviewed | 2026-05-09 |
| Owner primaire | LLM Ops |
| Owner backup | SRE |

## Symptômes

- Alerte Prometheus : `EfeLlmBreakerOpen`
- Métrique Grafana : panel « LLM Provider Health » → un ou plusieurs providers en `OPEN`
- Logs efe-server : `breaker tripped: 5 consecutive failures from anthropic-sonnet`
- Impact tenant : Mode-2 + Mode-3 calls fail with `degraded_reason: provider_breaker_open`

## Diagnostic (≤ 5 min)

### 1. Identifier le(s) provider(s) impacté(s)

```bash
kubectl exec -n efe deploy/efe-server -- \
  curl -s http://localhost:8081/internal/breakers \
  | jq '.[] | select(.state=="OPEN")'
```

### 2. Vérifier les status pages

- https://status.anthropic.com
- https://status.openai.com
- https://status.aws.amazon.com (Bedrock)
- https://status.azure.com

### 3. Lister les tenants à exposition Mode-2/3

```sql
SELECT tenant_id FROM tenant_feature_mode2 WHERE enabled=TRUE
UNION
SELECT tenant_id FROM tenant_feature_mode3 WHERE enabled=TRUE;
```

## Procédure de résolution

### Étape 1 : Stabiliser

#### Vérifier les fallback providers

```bash
efe-admin classify-test acme --mode 2 --provider openai-gpt4
```

#### Cas critique : tenant single-provider

```sql
SELECT tenant_id, providers
  FROM tenant_llm_config
 WHERE 'anthropic-sonnet' = ANY(providers)
   AND array_length(providers, 1) = 1;
```

Si non-vide → **escalation P1**.

### Étape 2 : Diagnostiquer

Si breaker OPEN > 10 min ET status page provider est GREEN :

#### A. Vérifier les credentials

```bash
kubectl get secret -n efe efe-llm-credentials -o json \
  | jq -r '.data."anthropic-key"' | base64 -d | head -c 20
```

#### B. Vérifier les quotas API

```bash
curl -H "x-api-key: $ANTHROPIC_KEY" https://api.anthropic.com/v1/usage
```

### Étape 3 : Corriger

#### Cas A : credentials expirés

```bash
efe-admin keys rotate-llm acme --provider anthropic --yes
```

#### Cas B : provider rate-limit

```bash
efe-admin quotas set acme --max-llm-rps 5 --yes
```

#### Cas C : provider en panne

```bash
efe-admin llm-config set acme --primary-provider openai --yes
```

#### Reset manuel du breaker

Quand le provider est rétabli :

```bash
kubectl exec -n efe deploy/efe-server -- \
  curl -X POST http://localhost:8081/internal/breakers/anthropic-sonnet/reset
```

### Étape 4 : Vérifier

```bash
efe-admin classify-test acme --mode 2
```

### Étape 5 : Communiquer

- Statuspage : update « Mode-2 / Mode-3 calls degraded due to provider X » (si > 10 min)
- Slack `#efe-incidents`
- Email tenants Mode-2/3 si > 30 min

## Post-incident

- Si pattern récurrent (même provider) : revue contractuelle SLA provider
- RCA dans 48 h

## Procédure de validation

Forcer un breaker open en staging :

```bash
kubectl exec -n efe-staging deploy/efe-server -- \
  curl -X POST http://localhost:8081/internal/breakers/test-provider/force-open
```

Doit déclencher l'alerte Prometheus dans ≤ 5 min.

## Annexes

- Architecture LLM Router : `docs/EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md`
- Dashboard Grafana : `EFE Mode-3` → panel « LLM Provider Health »
