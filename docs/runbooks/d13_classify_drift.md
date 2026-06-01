# d13_classify_drift — Runbook : classify confidence drift

> **Severity** : warning.
> **Alert** : `D13ObsClassifyDriftHigh`.
> **Owner** : on-call SRE during business hours ; escalates to
> ML / data-quality team.
> **Related dashboards** : `03_catalog_version_distribution`,
> `02_tenant_drill_down`.

## 1. Alert summary

The fraction of `confidence_high` results for `$tenant`/`$catalog`
dropped > 10 % over the last hour vs the 7-day baseline.

## 2. First 5 minutes — triage

1. Dashboard 03 — view the "Version chain depth" panel : did
   the tenant just register a new version ?
2. Dashboard 02 — check the "Confidence distribution" heatmap :
   has the bulk shifted toward medium / low ?
3. Run a shadow eval :
   ```bash
   efe-admin scenario run support_triage_v3 \
     --user "synthetic test" \
     --transcript-path /tmp/shadow.json
   ```

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| Catalog rotated to a new version | Spike right after register/evolve | Inspect new content, revert if regression |
| Input distribution shifted | Gradual drop over days | Re-train + recompile catalog |
| Embedder recompile in-flight | Transient | Wait for compile to finish |
| Tenant traffic mix change | New use case | Update class definitions |

## 4. Mitigation

- Revert to last-known-good catalog version :
  ```bash
  efe-admin catalog versions --name <catalog>
  efe-admin catalog get <tenant>/<catalog>@v<old>
  ```
  Then point the tenant's classifier back to `<old>`.

## 5. Resolution

- Add the new use case to the tenant's golden set.
- Schedule a new evolve cycle with updated anchors.

## 6. Escalation

If drift persists after revert : data-quality team.

## 7. Postmortem

Document the drift signal so the alert threshold can be tuned.
