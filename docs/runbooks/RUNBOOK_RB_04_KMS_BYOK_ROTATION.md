# RB-04 — Rotation forcée clé BYOK (KMS compromise)

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P0** |
| MTTR cible | 4 h |
| Fréquence attendue | < 1 / 3 ans |
| Last reviewed | 2026-05-09 |
| Owner primaire | Security Lead |
| Owner backup | Compliance Engineer |

## Déclencheurs

- Email AWS KMS Security : « Suspicious activity on key X »
- Alerte CloudTrail : `kms:Decrypt` from unusual IP
- Alerte Prometheus `EfeKmsByokAbnormalActivity`
- Audit interne révélant fuite de clé
- Demande explicite tenant (DPO right-to-rectify)

## Procédure

### Étape 1 : Préparer (≤ 30 min)

1. **Page CTO + Security Lead + DPO** impactés
2. **Geler les écritures du tenant** :

   ```bash
   efe-admin tenant suspend acme --reason "KMS-COMPROMISE-ROTATION" --yes
   ```

3. Lister toutes les ressources chiffrées avec la clé compromise :
   - Audit chain rows (BYOK envelope)
   - S3 audit bucket objects
   - Secrets Manager secrets

### Étape 2 : Provisionner nouvelle clé (≤ 30 min)

```bash
NEW_KEY_ID=$(aws kms create-key \
  --description "EFE acme BYOK v2 (post-RB-04 $(date +%Y%m%d))" \
  --key-usage ENCRYPT_DECRYPT \
  --query 'KeyMetadata.KeyId' --output text)

aws kms create-alias \
  --alias-name alias/efe-acme-byok-v2 \
  --target-key-id $NEW_KEY_ID

NEW_KEY_ARN=$(aws kms describe-key \
  --key-id $NEW_KEY_ID \
  --query 'KeyMetadata.Arn' --output text)
echo "New BYOK ARN: $NEW_KEY_ARN"
```

### Étape 3 : Re-key les données (≤ 2 h)

#### A. Audit chain (rows in `llm_call_audit`)

```bash
kubectl exec -n efe deploy/efe-server -- /app/bin/efe-rekey-audit \
  --tenant acme \
  --from-key $OLD_KEY_ARN \
  --to-key $NEW_KEY_ARN
```

#### B. S3 audit bucket

```bash
aws s3 cp s3://efe-acme-audit/ s3://efe-acme-audit-v2/ \
  --recursive \
  --sse aws:kms \
  --sse-kms-key-id $NEW_KEY_ID
```

#### C. Secrets Manager

```bash
for s in $(aws secretsmanager list-secrets \
  --filters Key=tag-value,Values=acme \
  --query 'SecretList[*].Name' --output text); do
  aws secretsmanager rotate-secret --secret-id $s
done
```

### Étape 4 : Switch & vérifier

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --set efe-server.byok.awsKmsKeyId=$NEW_KEY_ARN \
  --reuse-values

efe-admin smoke-test acme
```

### Étape 5 : Schedule destruction de l'ancienne clé

```bash
aws kms schedule-key-deletion \
  --key-id $OLD_KEY_ID \
  --pending-window-in-days 30
```

### Étape 6 : Communication

- **Email tenant : notification GDPR Art. 33 + Art. 34** (DPO obligation)
- Page DPO + Legal
- Statuspage : downtime documenté
- Notification autorités de contrôle si breach matériel (CNIL en France, ICO UK, etc.)

## Post-incident

- Postmortem détaillé
- Audit AWS CloudTrail des 90 derniers jours sur l'ancienne clé
- Mise à jour du runbook si nouveaux apprentissages

## Procédure de validation

Test trimestriel **NON-DESTRUCTIF** :

```bash
kubectl exec -n efe-staging deploy/efe-server -- /app/bin/efe-rekey-audit \
  --tenant staging \
  --dry-run
```

Simule la rotation sans modifier de données. Doit produire un rapport listant les bytes à re-chiffrer.

## Annexes

- AWS KMS rotation docs : https://docs.aws.amazon.com/kms/latest/developerguide/rotate-keys.html
- GDPR Art. 33 / Art. 34 : breach notification (72 h)
- Architecture BYOK : `docs/EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md` § BYOK
