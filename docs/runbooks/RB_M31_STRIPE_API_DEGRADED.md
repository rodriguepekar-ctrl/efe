# RB_M31_STRIPE_API_DEGRADED — Stripe API 5xx storm

**Severity** : P2 ; P1 if it crosses a month-end window.
**Trigger** : `M31StripeApiDegraded` for ≥ 10 min.

## 1. Diagnose

Check Stripe status page first :
- https://status.stripe.com

Then our side :
```bash
kubectl -n efe-system logs deploy/efe-stack-stripe-receiver --tail=200
kubectl -n efe-system logs cronjob/efe-stack-stripe-invoicer --tail=200
```

## 2. Customer impact

| Surface | Behaviour during outage |
|---|---|
| `/v1/billing/portal` | 503 ; portal link unavailable. |
| `/v1/billing/invoices` | Reads from local mirror — still works. |
| `/v1/billing/payment_method PUT` | 503 ; user retries. |
| Monthly invoicer | Items deferred ; cron retries every hour up to 24h. |
| Webhook receiver | Stripe will retry for up to 3 days per their SLA. We are passive. |

## 3. Mitigation

### 3.1 If Stripe is up but we are degraded
- Likely a credential issue. Verify `efe-stripe-secrets` exists and
  the API key is correct :
  ```bash
  kubectl -n efe-system get secret efe-stripe-secrets -o json | jq '.data | keys'
  ```
- Rotate via external-secrets if compromised.

### 3.2 If Stripe is down
- Wait. Our retry logic is bounded ; the queue will drain when
  Stripe recovers.
- Communicate via status page if customer-visible (`/v1/billing/portal`
  returns 503).

### 3.3 Manual invoice rerun
After Stripe recovers, re-trigger the invoicer for the affected
period :
```bash
scripts/m31/invoice_runner.sh
```

## 4. Verify reconciliation
The 6-hourly reconciler will pick up any drift between local mirror
and Stripe. Force a one-shot run :
```bash
kubectl -n efe-system create job --from=cronjob/efe-stack-stripe-reconciler \
    manual-reconcile-$(date +%s)
```

## 5. Post-incident
- File P2 incident.
- If month-end was affected : email enterprise tenants explaining
  the 1-2 day invoice delay.
- Compare `efe_stripe_api_errors_total` 7-day trend.
