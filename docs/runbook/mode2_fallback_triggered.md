# Runbook — Mode-2 Fallback Triggered

> Sprint **Φ-D-MODE2 Completion (D68)** — artefact #31.
> Cf. `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md` § 14.

## Symptômes

| Indicateur | Implication |
|---|---|
| `efe_mode2_fallback_total{reason}` rate > 0 sustained | Mode-2 dégradé, Mode-1 utilisé |
| Customer reports : "results suddenly less accurate" | Fallback silencieux trop fréquent |
| Cost dashboard shows Mode-2 spend dropping | Cache hits + fallbacks combinés |
| Grafana `efe-mode2.json` panel "fallback ratio" > 5 % | Investigate root cause |

## 5 trigger reasons (= identifier le bon path)

| `reason` label | Diagnostic | Mitigation |
|---|---|---|
| `budget_exhausted` | `efe_mode2_budget_hard_stop_total{tenant}` ≥ 1 | Renew tenant budget OR adjust soft threshold |
| `provider_down` | `efe_llm_circuit_breaker_state{provider} = 1` (= open) | Investigate provider outage ; check fallback chain |
| `invalid_response` | `efe_mode2_json_validation_failures_total{provider, reason}` rate up | Re-tune prompt or provider model version |
| `timeout` | `efe_llm_call_seconds{provider}` p99 > 60 s | Provider perf issue ; raise concern |
| `tenant_quota` | Per-tenant rate-limit table check | Adjust quota OR ask tenant to throttle |

## Diagnostic chirurgical

### Étape 1 : identifier le trigger dominant

```bash
# Aggregated fallback breakdown over the past hour.
curl -s "http://prometheus/api/v1/query?query=sum%20by%20(reason)%20(rate(efe_mode2_fallback_total%5B1h%5D))" | jq
```

Output expected :
```json
{
  "data": {
    "result": [
      {"metric": {"reason": "budget_exhausted"}, "value": [..., "0.5"]},
      {"metric": {"reason": "provider_down"}, "value": [..., "0.0"]},
      ...
    ]
  }
}
```

### Étape 2 : root cause per trigger

#### `budget_exhausted`

```bash
# Identify affected tenants.
curl -s "http://prometheus/api/v1/query?query=efe_mode2_tenant_budget_used_ratio" | jq '.data.result[] | select(.value[1] | tonumber > 0.95)'

# Renew budget OR escalate to billing team.
```

#### `provider_down`

```bash
# Check provider circuit breaker states.
curl -s "http://prometheus/api/v1/query?query=efe_llm_circuit_breaker_state" | jq

# Check provider status pages :
#   OpenAI    : https://status.openai.com
#   Anthropic : https://status.anthropic.com
#   ...
```

#### `invalid_response`

```bash
# Check JSON validation failure breakdown.
curl -s "http://prometheus/api/v1/query?query=rate(efe_mode2_json_validation_failures_total%5B1h%5D)" | jq

# Common causes :
#   1. Prompt version bump introduced ambiguity → revert prompt
#   2. Provider model version changed mid-period → pin model_version
#   3. LLM output verbosity exceeds maxItems → tighten prompt
```

### Étape 3 : verify Mode-1 still functional

```bash
# Make sure Mode-1 fallback path is healthy.
curl -X POST http://efe-server:50051/classify -d '{
  "text": "test",
  "catalog_id": "router-v1",
  "force_mode": 1
}'
```

If Mode-1 also fails → CRIT incident.

## Mitigation immédiate

1. Si fallback ratio < 10 % et SLO PASS : monitor uniquement, no action.
2. Si fallback ratio ≥ 10 % sustained 1h+ : Slack `#efe-mode2`.
3. Si fallback ratio ≥ 30 % OR Mode-1 also degrading : PagerDuty.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| < 5 % fallback rate | self-service | n/a |
| 5–10 % | `#efe-mode2` | 24h |
| 10–30 % | `#efe-incidents` | 4h |
| ≥ 30 % | PagerDuty SDK on-call | 15min |

## Liens

* Archi § 14 : `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-MODE2-FALLBACK-WORKS`, `SLO-MODE2-FALLBACK-LATENCY-P99`
* Instruments : `efe_mode2_fallback_total`
