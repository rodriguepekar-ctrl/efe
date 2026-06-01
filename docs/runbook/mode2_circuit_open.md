# Runbook — Mode-2 Provider Circuit Open

> Sprint **Φ-D-MODE2 Completion (D68)** — artefact #32.
> Cf. `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md` § 9.

## Symptômes

| Indicateur | Implication |
|---|---|
| `efe_llm_circuit_breaker_state{provider="X"} = 1` | Provider X is rejecting requests |
| `efe_llm_circuit_open_transitions_total{provider="X"}` rate > 0 | Provider X flapping |
| Customer reports : "everything still works but slow" | Fallback chain in use ; primary down |
| `efe_mode2_anchor_gen_seconds{provider="X"}` flat | No more LLM calls reaching X |

## Diagnostic

### Étape 1 : identify the offending provider

```bash
# Get all open circuits.
curl -s "http://prometheus/api/v1/query?query=efe_llm_circuit_breaker_state%3D%3D1" | jq
```

### Étape 2 : determine the cause

```bash
# Recent failures distribution per provider.
curl -s "http://prometheus/api/v1/query?query=rate(efe_llm_call_seconds_count%7Boutcome%3D%22err%22%7D%5B5m%5D)" | jq

# Provider availability tracker.
curl -s "http://prometheus/api/v1/query?query=efe_mode2_provider_availability%7Bprovider%3D%22X%22%7D"
```

Typical causes :
* Provider API outage (= check status page).
* Rate limit hit (= 429 responses).
* Auth credentials expired (= 401).
* Region-specific connectivity issue.

### Étape 3 : check fallback chain is healthy

```bash
# Other premium providers' circuit states.
curl -s "http://prometheus/api/v1/query?query=efe_llm_circuit_breaker_state%7Bprovider%3D~%22anthropic%7Cvertex%22%7D"
```

If all premium providers are open → Mode-2 effectively dead, all calls fall back to Mode-1. Investigate immediately.

## Mitigation

### Cas 1 : Provider auth expired

```bash
# Renew API key for the affected provider.
# Update vault secret :
vault kv put secret/efe/llm/openai api_key=NEW_KEY

# Restart efe-llm-router to pick up new key.
kubectl rollout restart deployment/efe-llm-router

# Verify recovery.
sleep 60
curl -s "http://prometheus/api/v1/query?query=efe_llm_circuit_breaker_state%7Bprovider%3D%22openai%22%7D"
# Should return 0 (= closed) after 30 s probe.
```

### Cas 2 : Rate limit hit

```bash
# Check provider rate-limit headers (= recorded in trace).
# Increase backoff in efe-llm-router config :
EFE_LLM_OPENAI_BACKOFF_MS=2000  # default 100

# Restart efe-llm-router.
```

### Cas 3 : Provider outage

* Confirm via provider status page.
* No action needed except wait + monitor fallback chain.
* If outage > 4h, consider temporarily disabling provider :

```bash
# Disable provider via config.
EFE_LLM_OPENAI_ENABLED=false
kubectl set env deployment/efe-llm-router EFE_LLM_OPENAI_ENABLED=false
```

## Circuit breaker behavior reference

```text
States transitions :
  CLOSED → 5 consecutive failures → OPEN
  OPEN → after 30 s → HALF-OPEN
  HALF-OPEN → 1 probe succeeds → CLOSED
  HALF-OPEN → probe fails → OPEN (reset 30 s)
```

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| 1 provider down, fallback chain healthy | self-monitor | n/a |
| 2+ providers down | `#efe-mode2` Slack | 1h |
| All premium providers down | PagerDuty | 15min |
| Mode-1 also degrading | PagerDuty SEV1 | 5min |

## Liens

* SLO : `SLO-MODE2-CB-DETECTION`, `SLO-MODE2-PROVIDER-AVAIL`
* Archi § 9 : `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`
* Code : `crates/efe-llm-router/src/circuit_breaker.rs`
