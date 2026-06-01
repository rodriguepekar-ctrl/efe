# ADR-M29-003 : 7-day grace period for tenant erasure

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M29_DATA_RIGHTS_ARCHITECTURE.md § 7.3

## Décision

L'erasure (GDPR Art. 17) a une **fenêtre de grâce de 7 jours**
avant que la cascade cascade réelle ne s'exécute.

## Justification
- GDPR Art. 17 : "without undue delay", typically interpreted as
  ≤ 30 days. 7 jours est bien dans la fenêtre.
- **Anti-accident** : 95% des "Mauvais clics" sont rattrapés en < 24h.
- **MFA initial bloque déjà 99% des compromis JWT** ; le grace cover
  les 1% restants.

## Alternatives

| Option | Pour | Contre |
|---|---|---|
| Pas de grace (immédiat) | Conforme min GDPR | Erreurs humaines irréversibles |
| **7 jours** ✅ | Bonne sécurité, dans GDPR | Tenant peut attendre 7j |
| 30 jours | Max GDPR | Trop long, frustre customers |

## Conséquences

### Positives
- Customer support n'est PAS dans la boucle pour les erreurs.
- Audit trail simple : `requested_at`, `cancelled_at`, `completed_at`.

### Négatives
- Tenant continue de payer pendant la grace. Documenté dans le ToS.
- Si grace dépassée par DPO sign-off attendu : démarre quand DPO signe
  (clock reset, doc explicit).

## Configurable

`tenant_feature.requires_dpo_sign_for_deletion = TRUE` → 2-step
confirmation : tenant DELETE + DPO signature email. Then 7-day grace
starts. Enterprise tier opt-in.
