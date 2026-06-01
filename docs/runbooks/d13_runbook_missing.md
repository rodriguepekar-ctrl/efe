# d13_runbook_missing — Runbook : alert without runbook_url

> **Severity** : warning (process gap).
> **Alert** : `D13ObsRunbookMissing`.
> **Owner** : engineering manager.

## 1. Alert summary

A D13-OBS alert is firing without a `runbook_url` annotation.
This means either :
- A new alert was added without a runbook (CI should have
  caught this).
- A Helm-overridden alert has empty annotations.

## 2. First 5 minutes

1. Identify the offending alert :
   ```promql
   ALERTS{family="D13-OBS"}
     unless on (alertname)
   ALERTS{family="D13-OBS", runbook_url!=""}
   ```
2. Check the rule definition :
   ```bash
   kubectl get prometheusrule efe-acme-d13 \
       -o yaml | grep -A5 "alert: <ALERTNAME>"
   ```
3. Check the values.yaml for an override that might have
   stripped the annotation.

## 3. Mitigation

- Add the missing annotation :
  ```yaml
  observability:
    alerts:
      overrides:
        <ALERTNAME>:
          annotations:
            runbook_url: https://docs.efe.ai/runbooks/<file>
  ```
- Helm upgrade.

## 4. Resolution

- Write the actual runbook (use the template in
  `architecture D13 § 10.2`).
- Re-run CI test
  `scripts/tests/test_d13_alert_runbook_links.py` to confirm.

## 5. Escalation

If the alert was added without a runbook and bypassed CI,
investigate CI bypass.
