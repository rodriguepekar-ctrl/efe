# Runbook — Mode-3 Human Review Queue Full

> Sprint **Φ-D-MODE3 Completion (D73)** — artefact #32.
> Cf. `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` § 9.

## Symptômes

| Indicateur | Implication |
|---|---|
| `efe_mode3_human_review_pending` gauge ≥ 100 | Queue backing up |
| `efe_mode3_review_latency_seconds` p99 > 5 min 30 s | SLA breach |
| Customer complaints about delayed verdicts | Visible UX impact |

## Diagnostic

```bash
# Per-trigger backlog.
curl -s "http://prometheus/api/v1/query?query=sum%20by%20(escalation_trigger)(efe_mode3_human_review_pending)" | jq

# Reviewer activity over past hour.
curl -s "http://prometheus/api/v1/query?query=rate(efe_mode3_review_decisions_total%5B1h%5D)" | jq
```

## 3 scenarios

### A : Surge in escalations
Recent spike (e.g. prompt deployment causes more `low_confidence` triggers).

Mitigation :
* Page on-call reviewer team.
* Rollback offending prompt if identified.
* Scale reviewer pool temporarily.

### B : Reviewer shortage
Insufficient staff for nominal queue rate.

Mitigation :
* Activate auto-approve for low-risk triggers (= `tenant_policy` flag only).
* Page additional reviewers.
* Capacity planning review : adjust headcount.

### C : Routing bug
Items not appearing in reviewer UI despite being queued.

Mitigation :
* Check `efe-server::mode3::human_review` worker logs.
* Restart worker if stuck.
* Verify Postgres `mode3_review_queue` SELECT works.

## Backpressure

If queue ≥ 1000 pending items, **temporarily disable** triggering for
the lowest-priority category (= typically `tenant_policy`) to relieve
pressure. Re-enable when queue < 200.

## Escalation

| Queue size | Action | Délai |
|---|---|---|
| < 100 | monitor | n/a |
| 100-500 | `#efe-mode3` Slack alert | 1h |
| 500-1000 | PagerDuty SEV3 | 30 min |
| ≥ 1000 | PagerDuty SEV2 + executive notify | 15 min |

## Liens

* SLO : `SLO-MODE3-HUMAN-REVIEW-LATENCY-P99`
* Archi § 9
* Workflow drill : `.github/workflows/mode3-human-review-drill.yml`
