# RB_M30_DR_DRILL — Quarterly Disaster Recovery drill

**Severity** : N/A (planned). Calendar reminder owns this.
**Trigger** : `M30DrillOverdue` or quarterly calendar.

## 1. Scheduling
- Quarterly cadence : end of Q1, Q2, Q3, Q4.
- Off-business hours preferred (Saturday 04:00 UTC) to minimize
  customer impact in case the drill goes sideways.
- Notify ALL enterprise customers 7 days before via the contract
  channel (most contracts contain an opt-out clause for the drill
  window).

## 2. Pre-flight
- Open Slack channel `#m30-drill-<date>`.
- Verify both regions are in steady state — no active incident.
- Verify replication lag < 5 s on the active.
- Take a baseline screenshot of the M30 Grafana dashboard.

## 3. Execute
```bash
EFE_API_URL="https://api.efe.ai" \
EFE_OPERATOR_TOKEN="$DRILL_TOKEN" \
EVIDENCE_S3_BUCKET="efe-evidence-eu-west-1" \
EFE_INITIAL_ACTIVE="eu-west-1" \
EFE_FAILOVER_TARGET="eu-west-3" \
scripts/m30/dr_drill_runner.sh
```

The runner :
1. Initiates the drill via `POST /v1/region/drill`.
2. Waits for the `DrillReport` (max 5 min).
3. Uploads the JSON report to S3 evidence vault.
4. Exits non-zero if verdict != "pass".

## 4. Verify
- The new active answers writes (smoke-test classify endpoint).
- Replication slot is healthy in the reverse direction.
- Cloudflare Geosteer shifted traffic correctly.
- No customer-visible 5xx spike (Grafana dashboard).

## 5. Failback
- Same procedure but reversed (target = eu-west-1).
- `force=true` is allowed for failback since the source is
  read-only.

## 6. Archive evidence
The S3 upload happens automatically. Also:
- Print the `DrillReport` JSON to a PDF and attach to the SOC 2
  evidence ticket.
- Update `docs/compliance/dr_drill_history.md` with the verdict +
  duration.

## 7. Post-drill
- File a P3 retrospective ticket.
- Identify any steps that needed manual creativity → update this
  runbook.
- Update the `M30DrillOverdue` alert's last-success timestamp.
