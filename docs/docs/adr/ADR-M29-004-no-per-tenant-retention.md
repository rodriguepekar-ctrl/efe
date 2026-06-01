# ADR-M29-004 : Per-tenant retention policy NOT supported in M29

- **Status** : Accepted (limitation explicit) · **Date** : 2026-05-11

## Décision

M29 conserve le **retention global 7 ans** sur `agent_audit`
(hérité de M25). Pas de surface API pour configurer une retention
custom per-tenant.

## Justification
- AI Act Art. 9 + ISO 27001 + SOC 2 demandent un **minimum** de
  retention ; pas un maximum. Tenant ne peut pas couper avant
  l'expiration légale.
- Tenants qui veulent **plus** que 7 ans peuvent utiliser leur
  audit export sink (S3) avec leur propre lifecycle policy.
- Implémenter per-tenant retention demande une logique de cascade
  delete sélective non-triviale qui aurait dérive le scope M29.

## Conséquences

### Positives
- Scope M29 reste 3-4 semaines.
- Single source of truth : tous les tenants ont le même retention.

### Négatives
- Tenant ne peut pas reduce. Mitigation : `DELETE /v1/tenants/:slug`
  (Component B) supprime tout incl. l'audit (au-delà du retention
  légal qui doit être conservé selon la régulation locale, qui dans
  l'UE permet la pseudonymisation post-deletion → couvert).

## Pivot

M30+ : si paying customers le demandent et que les obligations
légales le permettent, on ajoute `tenant_feature.audit_retention_days`
+ migration cleanup function.
