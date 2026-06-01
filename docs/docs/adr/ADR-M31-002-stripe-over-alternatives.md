# ADR-M31-002 : Stripe plutôt que Lago / OpenMeter / Recurly

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M31_FINOPS_BILLING_ARCHITECTURE.md § 4.2

## Décision

Stripe pour le billing engine (Customer + Subscription + Invoice +
Customer Portal + payment methods).

## Justification

1. **Maturité** — Stripe est utilisé par >$1T/an de paiements. Notre
   risque opérationnel est minimal.
2. **Customer Portal** — sortir-the-box, économise ~ 6 semaines de
   dev frontend.
3. **Self-service** — les clients enterprise gèrent leurs cartes /
   factures eux-mêmes ; nous n'avons pas à exposer un UI billing.
4. **Compliance** — PCI DSS niveau 1 ; nous restons SAQ-A (jamais
   de PAN chez nous).
5. **Stripe Tax** — taxes US + EU + UK out-of-the-box, sans intégrer
   Avalara ($30k/an).

vs **Lago** : open-source, séduisant, mais demande de l'opérer (DB,
upgrades, sécurité). Pas notre core competency.

vs **OpenMeter** : excellent pour le metering pure, mais pas un
billing engine. On utilise notre propre metering (`efe-metering`).

vs **Recurly / Chargebee** : prix plus élevé, intégrations moins
matures, customer portal moins joli.

## Conséquences

### Positives
- Time-to-market : 4 semaines de sprint M31 vs ~ 12 si on
  construisait le billing maison.
- Trust : "Powered by Stripe" rassure les buyers enterprise.

### Négatives
- Vendor lock-in. Stripe migration plan : nous mirorrons toutes les
  invoices localement (`billing_invoice` table) donc une bascule
  vers un concurrent reste possible (~ 8 semaines de migration
  d'après nos benchmarks internes).
- Stripe fees : 2.9 % + $0.30 par transaction US ; 1.4 % + €0.25 EU.
  Inclus dans notre marge unitaire.
- Pas de support multi-currency natif pour notre catalogue
  out-of-the-box — on passe par les Stripe Products avec un Price
  par currency. M31.1 ajoutera le routing.
