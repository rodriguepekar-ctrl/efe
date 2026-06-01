# d13_auth_failure_spike — Runbook : auth failures > 10/sec

> **Severity** : warning (security-sensitive).
> **Alert** : `D13ObsAuthFailureSpike`.
> **Owner** : SRE on-call ; security team in CC.
> **Related dashboards** : `05_audit_stream`,
> `02_tenant_drill_down`.

## 1. Alert summary

Sustained burst of auth/authz failures (> 10 / sec for 5 min)
for tenant `$tenant`. Possible causes : credential leak,
misconfigured client, attacker probing scopes.

## 2. First 5 minutes — triage

1. Dashboard 05 — open "Top 10 reasons" panel (LogQL :
   ```logql
   sum by (reason) (rate({stream="security", level="ERROR"}[5m] | json))
   ```
   ).
2. Identify the dominant reason :
   - `jwt_expired` → client clock skew or stale token cache.
   - `jwt_signature` → wrong issuer, key rotation issue.
   - `scope_missing` → client requesting an unauthorised RPC.
   - `tenant_mismatch` → potential cross-tenant attempt.
3. Check origin IPs :
   ```logql
   topk(10, count by (client_ip) ({stream="security", level="ERROR"}[15m] | json))
   ```

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| JWT expired in client cache | Sudden burst, all `jwt_expired` | Client refresh token logic |
| Issuer rotated keys | All `jwt_signature` | Update server's JWKs |
| Scope misconfigured | All `scope_missing` for one scope | Re-issue tenant token with right scope |
| Brute-force attack | Many distinct IPs | Block via firewall / WAF |

## 4. Mitigation

- If brute-force : block source IP at ingress.
- If client misconfig : contact tenant ops.
- If issuer key rotation : roll out updated JWKs ASAP.

## 5. Resolution

- File a security incident if attack confirmed.
- Update the tenant's onboarding doc if misconfig pattern.

## 6. Escalation

If `tenant_mismatch` events appear : page security team
immediately.

## 7. Postmortem

Mandatory if security incident confirmed.
