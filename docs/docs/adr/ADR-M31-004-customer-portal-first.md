# ADR-M31-004 : Stripe Customer Portal first (pas d'UI billing maison)

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M31_FINOPS_BILLING_ARCHITECTURE.md § 4.3

## Décision

Les clients gèrent leurs payment methods, abonnements, factures
**uniquement via le Stripe Customer Portal**. Notre interface
EFE expose un seul bouton "Manage billing" qui crée une session
portal et redirige.

## Justification

1. **Coût d'opportunité dev** — construire un UI billing complet (
   ajouter / retirer cartes, mandate SEPA, gestion des taxes, modes
   de paiement bancaires) = ~ 8 semaines de frontend + back-end.
2. **PCI scope** — un UI maison qui handle des `setupIntent` Stripe
   nécessite un audit PCI partiel. Le portal Stripe nous garde en
   SAQ-A (rien sensible chez nous).
3. **i18n + accessibility** — Stripe portal supporte ~30 langues
   et est WCAG-AA. On ne pourrait pas faire mieux à coût raisonnable.
4. **Stripe Tax + 3DS** — gérés automatiquement par le portal.

vs **Custom UI** : permet du branding plus poussé mais coût élevé.
M31 reporte cette décision à M32 si les retours clients l'exigent.

## Conséquences

### Positives
- Time-to-market accéléré.
- Pas de dette UI billing à maintenir.
- Conformité Stripe + portails déjà certifiés.

### Négatives
- Redirection externe (le client quitte efe.ai). Mitigation : la
  page retour mentionne "Vous reviendrez sur EFE après cette
  étape".
- Customization brand limitée à logo + colors. Acceptable pour
  M31 GA EXPERIMENTAL.
- Pas de tracking analytics dans le portail (Stripe ne le fournit
  pas). On loggue à l'ouverture de session côté EFE.
