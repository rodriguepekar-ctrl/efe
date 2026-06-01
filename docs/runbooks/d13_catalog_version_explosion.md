# d13_catalog_version_explosion — Runbook : catalog chain > 200 versions

> **Severity** : warning.
> **Alert** : `D13ObsCatalogVersionExplosion`.
> **Owner** : SRE / catalog curator.
> **Related dashboards** : `03_catalog_version_distribution`.

## 1. Alert summary

A `(tenant, name)` catalog chain has > 200 active versions
for at least 10 min. The hard cap is 1000 (D12 quota) — at
200, a cleanup is overdue.

## 2. First 5 minutes — triage

1. Dashboard 03 — sort the "Version chain depth" table by
   `Versions` desc.
2. Identify the offending `(tenant, name)`.
3. List the chain :
   ```bash
   efe-admin catalog versions \
       --tenant <tenant> --name <name> --pretty
   ```

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| Test pipeline registering on every CI run | Many versions/hour | Restrict test catalogs to `name=test_*` and clean post-run |
| Auto-recompile loop | Same content, different versions | Disable the loop, deduplicate by content_sha256 |
| No retention policy | Version count grows linearly | Set up D14 retention policy |

## 4. Mitigation

- Cleanup obsolete versions :
  ```bash
  for v in $(efe-admin catalog versions ...| jq -r '.keys[:-50]'); do
    efe-admin catalog delete $v --force
  done
  ```
  (Keep the last 50 ; cascade-delete the rest.)

## 5. Resolution

- Add a CI guard : test pipelines must clean up after themselves.
- Plan D14 retention policy.

## 6. Escalation

If approaching 1000 (the hard cap) and cleanup not feasible :
contact partner to acknowledge the issue.

## 7. Postmortem

Document the root cause to prevent regression.
