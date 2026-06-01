# RB-10 — Certificat TLS expirant (< 30 j)

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P2** (downtime risk) |
| MTTR cible | 2 h |
| Fréquence attendue | 1-2 / an / cluster (renewal failure) |
| Last reviewed | 2026-05-09 |
| Owner primaire | K8s Ops |
| Owner backup | Security Lead |

## Symptômes

- cert-manager event : `Certificate <name> renewal failed`
- Browser warning : « certificate will expire in X days »
- Métrique : `certmanager_certificate_expiration_timestamp_seconds < now() + 30d`
- Alerte Prometheus `EfeTlsCertExpiring`

## Diagnostic

### 1. Lister les certificats du namespace

```bash
kubectl get certs -A | grep -i efe
kubectl describe cert -n efe portal-admin-tls
```

### 2. Logs cert-manager

```bash
kubectl logs -n cert-manager deploy/cert-manager --tail=200 \
  | grep -i "portal-admin-tls\|error"
```

### 3. État du Challenge

```bash
kubectl get challenges -A
kubectl describe challenge <name>
```

## Procédure de résolution

### Étape 1 : Identifier la cause

| Cause | Indicateur |
|-------|------------|
| **A** : DNS-01 challenge fail | record `_acme-challenge.<domain>` manquant |
| **B** : HTTP-01 challenge fail | endpoint inaccessible (firewall, ingress) |
| **C** : Issuer disabled | `ClusterIssuer` status `Ready=False` |
| **D** : Rate limit Let's Encrypt | log `tooManyRegistrations` ou `rateLimited` |

### Étape 2 : Corriger

#### Cas A : DNS-01

```bash
# Verify DNS record exists.
dig +short TXT _acme-challenge.portal.acme.com

# Force renewal with manual annotation.
kubectl annotate cert -n efe portal-admin-tls \
  cert-manager.io/issue-temporary-certificate="true"
kubectl delete cert -n efe portal-admin-tls
helm upgrade efe-stack ./charts/efe-stack --reuse-values
```

#### Cas B : HTTP-01

Vérifier que l'ingress accepte `/.well-known/acme-challenge/*` :

```bash
curl -I https://portal.acme.com/.well-known/acme-challenge/test
# Expect 200 (returns nothing) or 404 (no current challenge), NOT 403/timeout.
```

#### Cas C : ClusterIssuer

```bash
kubectl get clusterissuer letsencrypt-prod -o yaml
# Look for `status.conditions[].type=Ready,status=True`.

# If False : reset.
kubectl annotate clusterissuer letsencrypt-prod cert-manager.io/restart="$(date +%s)" --overwrite
```

#### Cas D : Rate limit Let's Encrypt

Wait 1 week pour reset OR switch to staging issuer temporairement :

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --set portal-admin.ingress.tls.issuerRef.name=letsencrypt-staging \
  --reuse-values
```

(Browsers will show « not trusted » — acceptable for internal staging only.)

### Étape 3 : Vérifier

```bash
kubectl get cert -n efe portal-admin-tls -o yaml | grep -A 5 status
# Status: Ready=True, NotAfter > now() + 60d
```

```bash
# Browser sanity.
echo | openssl s_client -servername portal.acme.com -connect portal.acme.com:443 \
  | openssl x509 -noout -dates
# notBefore/notAfter should be > 60 days in the future.
```

### Étape 4 : Communiquer

- Statuspage si downtime > 5 min
- Slack `#efe-incidents`
- Email DPO si downtime > 30 min

## Post-incident

- Si renewal a échoué silencieusement → améliorer alerting (alerte à 60d, pas seulement 30d)
- Postmortem si downtime > 1 h

## Procédure de validation

Force expiry simulation en staging :

```bash
# Renew NOW (forces a fresh challenge).
kubectl cert-manager renew -n efe-staging test-cert
```

OU :

```bash
kubectl annotate cert -n efe-staging test-cert \
  cert-manager.io/issue-temporary-certificate="true" --overwrite
kubectl delete cert -n efe-staging test-cert
# Re-applied via Helm.
```

Doit déclencher renewal succès dans ≤ 5 min (DNS-01 propagation).

**Test trimestriel** : owner K8s Ops.

## Annexes

- cert-manager docs : <https://cert-manager.io/docs/>
- Helm ingress config : `charts/efe-stack/charts/portal-admin/templates/ingress.yaml`
- Let's Encrypt rate limits : <https://letsencrypt.org/docs/rate-limits/>
