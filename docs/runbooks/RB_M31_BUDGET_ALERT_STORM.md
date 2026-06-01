# RB_M31_BUDGET_ALERT_STORM — Budget alert false-positive cascade

**Severity** : P3 (noise ; customer trust at risk).
**Trigger** : `M31BudgetExceededStorm` for ≥ 5 min.

## 1. Identify the offending alerts
```sql
SELECT tenant_id, severity, COUNT(*) AS n
  FROM budget_alert ba
  JOIN budget_alert_event bae ON bae.alert_id = ba.id
 WHERE bae.fired_at > NOW() - INTERVAL '1 hour'
 GROUP BY 1, 2
 ORDER BY n DESC
 LIMIT 20;
```

## 2. Triage

### 2.1 Mis-configured threshold
- Common cause : tenant set threshold in dollars instead of cents
  ($100 → 100 cents = $1.00).
- Email the tenant + offer to update via the portal.

### 2.2 Pricing rebase regression
- A new product price + a stale rollup row creates apparent overrun.
- Cross-check `usage_rollup_day` vs `pricing_plan_meter` for the
  affected period.

### 2.3 Burst usage (legitimate)
- The customer is genuinely over budget. Mitigation : suggest plan
  upgrade.

## 3. Mitigation

### 3.1 Bulk dismiss false positives
```sql
UPDATE budget_alert
   SET dismissed_until = NOW() + INTERVAL '24 hour'
 WHERE id IN (<list>);
```

### 3.2 Snooze the alert rule
If the storm is happening across many tenants and is clearly
synthetic, snooze the Prometheus alert via the Alertmanager UI for
1h while investigating.

## 4. Post-incident
- File P3 ticket.
- If the cause was pricing-rebase regression : add a test to
  `bench/m31_pricing_calibration.py`.
- If the cause was customer mis-config : add inline validation in
  the portal UI / SDK to prevent unit confusion.
