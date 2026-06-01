# RB_M30_FAILOVER_PLAYBOOK — Manual region failover

**Severity** : P1 (active region degraded ; flip to passive required).
**Trigger** : `M30RegionPeerDown` + on-call decision OR planned drill.

## 0. Pre-flight (MUST do before flipping)

1. **Confirm the active region is actually degraded.**
   - Check `/v1/region/health` from a third location (mobile network,
     not the office VPN).
   - Confirm with a second operator over PagerDuty.

2. **Check replication lag.**
   - `SELECT now() - pg_last_xact_replay_timestamp() FROM pg_stat_replication;`
   - If lag > 30 s, **prefer waiting** unless the active is hard-down.
     Use `force=true` only if the active is unreachable for > 10 min.

3. **Open a war-room.** Slack channel `#m30-failover-<date>` ; pin
   the runbook URL.

## 1. Quiesce the (still-up?) active region
If the active is reachable, drain in-flight work :
```bash
curl -X POST "$ACTIVE_URL/v1/region/quiesce" \
  -H "Authorization: Bearer $OP_TOKEN" \
  -d '{"reason":"failover planned"}'
```
Wait 60 s for batch + webhook workers to settle.

## 2. Initiate the failover
```bash
curl -X POST "$ACTIVE_URL/v1/region/failover" \
  -H "Authorization: Bearer $OP_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"target_region":"eu-west-3","reason":"<incident-id>","force":false}'
```
Expected response :
```json
{"status":"succeeded","previous_active":"eu-west-1","new_active":"eu-west-3","duration_ms":4200}
```
If `force=false` is refused with `region.failover_refused`, re-read
§ 0 — replication is lagging or the target is not ready.

## 3. Verify the new active
```bash
# Should answer 200 within 10 s.
curl -i "$TARGET_URL/v1/region/health"

# Smoke-test a write.
curl -X POST "$TARGET_URL/v1/classify/text" \
  -H "Authorization: Bearer $SMOKE_TOKEN" \
  -d '{"text":"hello","mode":"mode1"}'
```

## 4. Update Cloudflare Geosteer (already automatic ; verify)
The Geosteer pool monitor detects the active region's health and
shifts traffic within ~ 60 s. Verify :
```bash
curl -s "https://api.cloudflare.com/client/v4/zones/$CF_ZONE/load_balancers" \
  -H "Authorization: Bearer $CF_TOKEN" | jq '.result[0].default_pool_ids'
```
Should list `eu-west-3` first if `eu-west-1` is down.

## 5. Communicate
- Status page : `Region eu-west-1 failed over to eu-west-3`.
- Email enterprise tenants per the contract NDA.
- Post in `#announcements`.

## 6. Schedule the failback
The failback is **planned**, not urgent. Once eu-west-1 is healthy :
1. Reverse-promote eu-west-1 from a fresh backup of eu-west-3.
2. Run `efe-region drill --to=eu-west-1 --force=true` to validate.
3. Schedule the flip during a low-traffic window.

## 7. Post-incident
- File P1 incident.
- Compute RTO + RPO from the audit chain (entries
  `RegionFailoverInitiated` and `RegionFailoverCompleted`).
- Upload report to `s3://efe-evidence/dr_drill/`.
- Update this runbook if any step needed manual creativity.
