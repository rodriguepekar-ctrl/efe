# RB_M31_INVOICE_RECONCILIATION_FAILED — Local mirror drifted from Stripe

**Severity** : P2 (financial integrity).
**Trigger** : `M31WebhookProcessingErrors` or manual reconciler exit-code non-zero.

## 1. Identify the drift
```bash
kubectl -n efe-system logs job/<last-reconciler-job> --tail=500
```

The log lists, per tenant, the `ReconciliationOutcome` enum value :
- `MissingLocal {stripe_invoice_id}` — Stripe has an invoice we don't.
- `MissingRemote {local_invoice_id}` — we have an invoice Stripe doesn't.
- `AmountDrift {…}` — totals differ.
- `StatusDrift {…}` — statuses differ.

## 2. Triage

### 2.1 MissingLocal
- Cause : webhook for `invoice.created` was lost.
- Fix : the reconciler should auto-insert. If not :
  ```sql
  -- Manual upsert from Stripe API ; verify the data first via
  -- the Stripe Dashboard https://dashboard.stripe.com/invoices/<id>
  ```

### 2.2 MissingRemote
- Cause : a Stripe operator voided + deleted the invoice (rare).
- Fix : mark the local invoice `void` + email the tenant. NEVER
  silently delete the local row — keep for audit.

### 2.3 AmountDrift
- Cause : invoice was edited in Stripe Dashboard (humans).
- Fix : update the local mirror to match Stripe ; log who edited
  + when.

### 2.4 StatusDrift
- Cause : webhook for `invoice.paid` was lost.
- Fix : the reconciler should auto-update. If it didn't, check
  webhook dedup table :
  ```sql
  SELECT * FROM stripe_webhook_event
   WHERE event_type LIKE 'invoice.%' AND processing_status='failed'
   ORDER BY received_at DESC LIMIT 20;
  ```

## 3. Audit trail
Any manual fix MUST be written to the HMAC audit chain with reason +
operator. Use the audit CLI :
```bash
efe-verify-audit insert --event InvoiceFinalized --tenant <id> --reason "reconcile fix"
```

## 4. Post-incident
- File P2 incident.
- If recurring : root-cause the webhook delivery loss (NATS bus
  saturation, signing-secret rotation drift).
