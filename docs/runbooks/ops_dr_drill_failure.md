# Runbook — DR drill failure / cadence breach

> **Severity** : major (Sev2 if a drill fails ; Sev3 if a cadence breach
> ≤ 100 days ; Sev2 if cadence > 120 days)
> **Owner** : SRE
> **Linked SLOs** : `SLO-OPS-DR-DRILL-CADENCE`
> **Alerts firing** : `OpsDrDrillSoon` (Sev4) → `OpsDrDrillOverdue` (Sev2)

---

## 0. TL;DR

Either (a) the quarterly DR drill exceeded the **30 min RTO** budget,
or (b) the cadence (≤ 100 days between drills) is about to breach.
Both are major incidents because DR is a regulator-visible control
(GDPR Art. 32, AI Act Art. 9).

---

## 1. Cadence-only breach (no failed drill)

```bash
# Schedule the next drill.
date -u -d "+0 days"
date -u -d "+7 days"     # propose a calendar block
```

1. Open a Linear ticket `EFE-DR-DRILL-<YYYY-MM>`.
2. Calendar-block 2 h with SRE + DPO + comms.
3. Update `bench_baselines/v0.9.0/dr_cadence.yaml`.
4. Run the dry mode to warm caches :
   `python3 scripts/ops/dr_drill_simulator.py --mode dry`.

No customer impact ; no postmortem.

---

## 2. Failed staging drill (over-budget)

The simulator output flags one or more steps as `over_budget`. For each :

| Step over budget | Likely root cause | Mitigation |
|---|---|---|
| `verify_primary_unhealthy` | Slow health endpoints | Add `--readiness-probe-interval=5s` |
| `confirm_two_eyes_approval` | Approval tooling slow | Pre-register approvers in PagerDuty schedules |
| `promote_dr_postgres_replica` | Replication lag spike | Pin `synchronous_commit = remote_apply` for tenant Postgres |
| `redirect_dns_to_dr` | DNS TTL too high | Cap TTL ≤ 60 s in CloudFlare config |
| `scale_dr_workers_to_target` | Cold start | Pre-warm replicas via `pdb-replica-warmer` cronjob |
| `invalidate_edge_caches` | CDN purge slow | Use stale-while-revalidate ≤ 30 s |
| `warm_dr_classify_endpoint` | GPU pre-load too slow | Bake images with embedding cache ; reduce model load time |
| `verify_audit_chain_resumes` | Audit signing key fetch slow | Pre-cache HMAC key in pod startup |
| `post_status_page_update` | Statuspage API rate-limit | Batch announcements |
| `notify_stakeholders` | Email IDP / Slack hop | Use the failover Slack-incoming-webhook |

Patch + re-run the drill. RTO must come back under 1800 s with **all**
steps within their per-step budget.

---

## 3. Failed prod drill

A prod drill that fails RTO is functionally an outage of the secondary
region. Page Sev1, treat as outage. Postmortem within 24 h.

---

## 4. Audit chain continuity check

After **any** DR drill (staging or prod), confirm the audit chain
remains continuous across the failover :

```sql
WITH chain AS (
    SELECT
        chain_position,
        prev_hmac,
        current_hmac
    FROM audit_events
    WHERE tenant_id = 'efe-platform-self'
      AND emitted_at > now() - INTERVAL '2 hours'
)
SELECT chain_position, prev_hmac IS NOT NULL AS has_prev
FROM chain
ORDER BY chain_position;
```

A gap (= `chain_position` jumps) is **Sev1** regardless of RTO. Fix
chain continuity first ; then continue the postmortem.

---

## 5. Quarterly drill checklist

- [ ] T-2 weeks : announce internally, request DPO observer
- [ ] T-1 week  : pre-flight (`--mode dry`)
- [ ] T-0       : drill (`--mode staging` or `--mode prod`)
- [ ] T+1 day   : postmortem regardless of outcome
- [ ] T+1 week  : action items closed or DT'd
- [ ] T+1 month : trust center page refreshed with drill summary

---

## 6. Cross-references

- [`scripts/ops/dr_drill_simulator.py`](../../scripts/ops/dr_drill_simulator.py)
- [`docs/runbooks/M26_DR_PLAYBOOK.md`](M26_DR_PLAYBOOK.md)
- [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md) § 16
- [`bench_baselines/v0.9.0/ops_master_slos.yaml`](../../bench_baselines/v0.9.0/ops_master_slos.yaml) (= `SLO-OPS-DR-DRILL-CADENCE`)
