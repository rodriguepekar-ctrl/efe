# ADR-M31-006 : Retries idempotents sur les opérations Stripe

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M31_FINOPS_BILLING_ARCHITECTURE.md § 6

## Décision

Chaque appel mutant à Stripe (create_customer, create_invoice,
create_invoice_item, finalize_invoice, create_portal_session)
passe un **idempotency key** dérivé du contexte business.

Format : `efe-m31:{action}:{tenant_id}:{period}:{sequence}`.

Exemples :
- Création de customer : `efe-m31:cust:acme:bootstrap:1`.
- Création d'invoice item du mois : `efe-m31:invitem:acme:2026-05:classify_requests`.
- Création de l'invoice mensuelle : `efe-m31:inv:acme:2026-05:0`.

## Justification

1. **Network retries safe** — un timeout côté EFE puis retry du cron
   ne crée pas deux invoices. Stripe renvoie la réponse précédente.
2. **Cron idempotency** — la cron mensuelle peut tourner deux fois
   par accident (replay manuel via `scripts/m31/invoice_runner.sh`)
   sans facturer 2× les clients.
3. **Audit trail clair** — chaque idempotency key identifie de manière
   unique l'opération business. Trace facile entre logs EFE et
   dashboard Stripe.

## Conséquences

### Positives
- Pas de duplicate-invoice cauchemar (vu chez d'anciens clients
  Stripe sans cette discipline).
- Stripe webhook replay safe : le handler peut être relancé sans
  bug.

### Négatives
- Si la clé est mal formée (sequence number qui change quand on
  re-run), on peut accidentellement créer une opération en double.
  Mitigation : tests unitaires figent le format ; runbook
  `RB_M31_STRIPE_API_DEGRADED` documente la regen.
- Stripe garde les idempotency keys 24h ; après 24h une retry
  pourrait passer. Mitigation : la cron mensuelle tourne en < 1h,
  bien à l'intérieur de la fenêtre.
