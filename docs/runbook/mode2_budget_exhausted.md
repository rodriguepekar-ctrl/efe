# Runbook — Mode-2 Budget Exhausted

> Sprint **Φ-D-MODE2 Completion (D68)** — artefact #33.
> Cf. `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md` § 7.

## Symptômes

| Indicateur | Implication |
|---|---|
| `efe_mode2_budget_hard_stop_total{tenant="X"}` ≥ 1 | Tenant X budget exhausted |
| `efe_mode2_tenant_budget_used_ratio{tenant="X"} ≥ 1.0` | Tenant at or above cap |
| Customer reports : "responses degraded today" | Mode-1 fallback active |
| Email alert "budget 80 %" earlier in the day | Soft warning preceded |

## 3 scenarios + mitigation

### Scenario A : Legitimate usage growth

Customer's actual usage outgrew their budget.

```bash
# Identify the tenant.
curl -s "http://prometheus/api/v1/query?query=efe_mode2_tenant_budget_used_ratio%3E%3D1.0" | jq

# Get tenant's recent spend pattern (= 7-day rolling).
curl -s "http://prometheus/api/v1/query?query=increase(efe_llm_cost_usd_total%7Btenant%3D%22X%22%7D%5B7d%5D)"
```

Mitigation :
1. Reach out to customer + propose budget bump.
2. Apply temporary 24h override via `efe-admin` CLI :
   ```bash
   efe-admin budget set --tenant X --monthly-cap-usd 5000 --until 2026-06-01
   ```
3. Document the override in the audit log + tenant notes.

### Scenario B : Cost spike from a runaway prompt

A new prompt version produces verbose outputs that 10× cost.

```bash
# Check per-prompt token usage.
curl -s "http://prometheus/api/v1/query?query=sum%20by%20(prompt_id)(rate(efe_llm_tokens_total%7Bkind%3D%22output%22%7D%5B1h%5D))"

# Identify recently bumped prompts.
git log --since="48 hours ago" -- crates/efe-prompts/templates/
```

Mitigation :
1. Rollback the offending prompt via `efe-admin prompts rollback`.
2. Notify `#efe-prompts` of the regression.
3. Verify SLO-MODE2-COST-P99 returns to normal within 1h.

### Scenario C : Adversarial cost amplification attack

A malicious / compromised tenant generates many high-cost prompts to drain the budget.

```bash
# Look for unusual request volume spike.
curl -s "http://prometheus/api/v1/query?query=rate(efe_mode2_classify_total%7Btenant%3D%22X%22%7D%5B10m%5D)"

# Cross-reference with safety filter rate.
curl -s "http://prometheus/api/v1/query?query=rate(efe_mode2_safety_filter_blocks_total%7Btenant%3D%22X%22%7D%5B10m%5D)"
```

Mitigation :
1. Apply emergency rate limit :
   ```bash
   efe-admin ratelimit set --tenant X --requests-per-min 10 --emergency
   ```
2. Open security incident.
3. Audit recent prompts via :
   ```bash
   python3 scripts/mode2/replay_audit_chain.py --tenant X --since $(date -u -d '24 hours ago' +%FT%TZ)
   ```

## Reconciliation post-incident

After resolving, run cost reconciliation to verify accuracy :

```bash
python3 scripts/mode2/reconcile_costs.py \
    --governor-state /tmp/governor.json \
    --provider-billing /tmp/billing.json
```

Drift > 5 % → escalate to billing team.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| 1 tenant exhausted, < 1 % traffic | self-resolve | 24h |
| Multiple tenants exhausted | `#efe-billing` | 4h |
| Cost amplification suspected | `#efe-security` | 30min |
| Cost reconciliation drift > 10 % | PagerDuty | 1h |

## Liens

* SLO : `SLO-MODE2-BUDGET-HARD-STOP`, `SLO-MODE2-COST-P99`
* Archi § 7 : `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`
* Tool : `scripts/mode2/reconcile_costs.py`
