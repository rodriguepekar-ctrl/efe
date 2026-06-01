# ADR-M29-002 : Per-tenant HMAC key for audit export chain

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M29_DATA_RIGHTS_ARCHITECTURE.md § 6.4

## Décision

Chaque tenant a un HMAC key dédié pour l'export audit chain. La clé
est délivrée **PGP-encrypted** au registration du sink (une seule fois),
et EFE n'en garde PAS de copie raw après livraison (uniquement le hash
SHA-256 stocké pour rotation tracking).

## Alternatives

| Option | Pour | Contre |
|---|---|---|
| **Per-tenant key** ✅ | Tenant peut vérifier sans confiance envers EFE | Plus de gestion de clés |
| Master key + per-tenant signature suffix | Plus simple côté EFE | Tenant doit faire confiance à EFE |
| Per-event PGP signature | Vérification asymétrique | Performance prohibitive (signing chaque event) |

## Conséquences

### Positives
- **Cryptographically isolation** : compromis EFE → tenant peut
  toujours détecter via le chain HMAC.
- **Tenant-controlled** : tenant peut révoquer la clé en rotation
  (nouvelle clé sur prochain rollup).

### Négatives
- **Key delivery** : exige PGP côté tenant. Pas tous les customers
  ont du PGP — fallback : portal-admin download avec MFA + auto-expire.
- **Key loss** : si le tenant perd la clé, il ne peut plus vérifier
  l'historique. La rotation est annuelle par défaut (mitigation).

## Rotation

Annuelle par défaut. Trigger : `POST /v1/audit/export-sink/:id/rotate-key`.
Génère nouvelle clé, encrypte PGP, livre une fois. Old key reste
valide pour les exports déjà émis.
