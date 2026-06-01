# ADR-M29-005 : Two-key BYOK rotation window

- **Status** : Accepted · **Date** : 2026-05-11
- **Related** : EFE_M29_DATA_RIGHTS_ARCHITECTURE.md § 10.3

## Décision

Pendant 90 jours après une BYOK rotation, **deux clés sont valides
en simultané** :
- Reads : try new first, fallback old.
- Writes : new only.

## Justification

Atomic flip (single key) :
- Avantage : pas de complexity dual-key.
- Risque : re-encrypt mid-flight d'un million de rows = fenêtre de
  ~30 min où les lectures peuvent voir un mix old/new. Race condition.

Two-key window :
- Avantage : zero-downtime ; reads jamais bloquées.
- Coût : storage des deux clés + complexity un peu plus haute.

## Conséquences

### Positives
- **Zero downtime** rotation.
- **Recovery** : si la new key échoue (bug, KMS down), tenant ne perd
  pas l'accès à son data (old key still works).

### Négatives
- KMS storage : 2 keys × all tenants × 90j window = surface
  d'attaque doublée.
- Code complexity : every decrypt-path tries new then old. Mitigation
  : centralisé dans `efe-crypto::decrypt` helper.

## Rotation timeline

```
t=0   : rotation triggered, state=preparing.
t=0+1m: state=reencrypting. Workers begin column-by-column re-encrypt
         using new key.
t=X   : last column done, state=committing.
t=X+5s: state=done. NEW key is "primary write key".
        OLD key still in KMS, still works for reads.
t=X+90d: OLD key deleted from KMS. Reads encrypting under old fail
         past this point (already re-encrypted under new).
```
