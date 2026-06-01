# ADR-M28-004 : HMAC-SHA256 webhook signing vs JWS / mTLS

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M28_STREAMING_EVENTS_ARCHITECTURE.md § 6.3

## Contexte

Outbound webhooks transportent des données business sensibles
(`agent.run.completed` contient cost + tenant identifier). Le
récepteur doit pouvoir vérifier que (a) le payload vient de EFE,
(b) le payload n'a pas été modifié en transit.

Trois schémas envisagés :
- **HMAC-SHA256** signature header (Stripe-style).
- **JWS** (JSON Web Signature) : asymétrique, le récepteur valide via
  notre clé publique.
- **mTLS** : le récepteur valide notre certificat client X.509.

## Décision

**HMAC-SHA256** (`X-EFE-Webhook-Signature: t=<ts>,v1=<hex>`) en v1.
mTLS en option Enterprise post-GA.

Justification :
1. **Compat** : tout HTTP framework supporte la vérification HMAC en
   < 10 lignes. JWS demande une crypto-lib. mTLS demande config
   serveur côté récepteur.
2. **Per-subscription secret** : chaque webhook a son propre secret
   généré par EFE et révélé une seule fois. Compromise d'un secret
   n'affecte qu'un seul webhook.
3. **Replay defense** : timestamp dans le header ; receveur rejette si
   tolérance dépassée.
4. **Time-to-customer-onboarding** : un curl pour vérifier ; pas de
   PKI à mettre en place.

mTLS reste dispo pour les enterprise customers qui ont déjà du PKI
maison ; documenté dans le post-M28 catalog.

## Conséquences

### Positives
- Code-side simple : `hmac::Hmac<Sha256>` + constant-time compare.
- Documentation customer-facing trivial.
- Compatible avec Stripe pattern → developers familiers.

### Négatives
- Symétrique → si EFE est compromis, l'attaquant peut forger des
  webhook signatures. Mitigation : secret stocké hash-only en DB ;
  raw secret jamais persisté ; intercept attack window minimisée.
- Pas de non-répudiation forte (un JWS aurait permis le receveur de
  prouver à un tiers que le message vient de EFE). Si un customer
  enterprise demande, mTLS ou JWS opt-in.
