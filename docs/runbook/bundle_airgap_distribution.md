# Runbook — Bundle Air-Gapped Distribution

> **Audience** : Customer Success + Solutions Engineering.
> **Cas d'usage** : Enterprise clients déployant EFE dans un
> environnement sans accès Internet (= government, defense,
> finance régulée, healthcare HIPAA-strict).
> **Sprint d'origine** : Φ-C D27.
> **Référence architecture** : `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md`
> § 8.3.

---

## TL;DR

Un déploiement air-gapped est composé de 2 environnements :
- **Jump host** : machine intermédiaire avec accès Internet ET
  accès SSH au réseau isolé du client.
- **Réseau isolé** : où tournent les replicas EFE ; aucun accès
  Internet sortant.

La procédure pull-transfer-push se fait via le jump host avec
`skopeo` (= outil canonique CNCF pour OCI images sans daemon).

---

## 1. Pré-requis client

### 1.1 Hardware

- **Jump host** : Linux x86_64, ≥ 4 GiB RAM, ≥ 50 GiB disk
- **Réseau interne** : OCI registry interne (= Harbor, JFrog
  Artifactory, ECR managed, etc.) accessible depuis les replicas
  EFE
- **SSH** : key-based auth entre jump host et les replicas

### 1.2 Software

- `skopeo` ≥ 1.13 sur le jump host
- `oras` ≥ 1.2 sur le jump host (= alternative `skopeo` pour OCI Artifacts)
- `cosign` ≥ 2.2 pour la vérification de signature
- Credentials :
  - Read-only access au registry EFE primary (= `registry.efe.ai`)
  - Push access au registry client interne

### 1.3 Trust store custom

Pour les déploiements air-gapped, le trust store **ne peut PAS**
vérifier en ligne Rekor. Solution : le client reçoit une copie
**hors-bande** (= USB ou email signé) du `trust-store.toml` +
des entrées Rekor pré-archivées.

```bash
# Reçoit du jump host par scp :
$ scp jump-host:/data/efe-trust-store.toml /etc/efe/
$ scp jump-host:/data/efe-rekor-cache.json /etc/efe/
```

Le `trust-store.toml` air-gapped contient :

```toml
# /etc/efe/trust-store.toml (air-gapped variant)
[[trusted_keys]]
fingerprint = "<fingerprint EFE prod>"
description = "EFE Production Signing — air-gap distribution 2026-07"
added_at = "2026-07-15T00:00:00Z"
revoked = false

# Cache des entrées Rekor consultées (= snapshot hors-bande)
[rekor_cache]
mode = "snapshot"
snapshot_date = "2026-07-15T00:00:00Z"
snapshot_path = "/etc/efe/efe-rekor-cache.json"
```

---

## 2. Procédure de distribution (= 1 fois par release)

### 2.1 Étape 1 : Pull depuis registry EFE (= jump host en ligne)

```bash
# Sur le jump host avec Internet :
$ skopeo login registry.efe.ai \
    --username "${CLIENT_OCI_USERNAME}" \
    --password-stdin < /etc/efe/registry-token

$ BUNDLE_ID=paraphrase-multilingual-MiniLM-L12-v2
$ BUNDLE_VERSION=1.0.0

$ skopeo copy --src-tls-verify=true \
    docker://registry.efe.ai/bundles/${BUNDLE_ID}:${BUNDLE_VERSION} \
    docker-archive:/data/efe-bundle-${BUNDLE_VERSION}.tar:efe-bundle:${BUNDLE_VERSION}

# Vérifier le bundle est complet
$ ls -lh /data/efe-bundle-${BUNDLE_VERSION}.tar
-rw-r--r-- 78M  /data/efe-bundle-1.0.0.tar
```

### 2.2 Étape 2 : Vérification cosign + Rekor

```bash
# Récupère le bundle cosign + cert depuis le registry (= mêmes URLs)
$ skopeo inspect --raw \
    docker://registry.efe.ai/bundles/${BUNDLE_ID}:${BUNDLE_VERSION} \
    > /data/efe-bundle-${BUNDLE_VERSION}.manifest.json

# Vérifie cosign keyless via Rekor (= dernier check ONLINE)
$ cosign verify-blob \
    --bundle /data/efe-bundle-${BUNDLE_VERSION}.cosign.bundle \
    --certificate-identity-regexp "https://github.com/efe-ai/efe/.+" \
    --certificate-oidc-issuer-regexp "https://token.actions.githubusercontent.com" \
    --rekor-url https://rekor.sigstore.dev \
    /data/efe-bundle-${BUNDLE_VERSION}.tar

# Snapshot l'entrée Rekor (= pour usage offline plus tard)
$ curl -s "https://rekor.sigstore.dev/api/v1/log/entries/?logIndex=<index>" \
    > /data/efe-rekor-snapshot-${BUNDLE_VERSION}.json
```

### 2.3 Étape 3 : Transfer vers le réseau isolé

Options de transfer :

**Option A : SCP via jump host** (= classique, simple)
```bash
$ scp /data/efe-bundle-${BUNDLE_VERSION}.tar customer-jump:/data/
$ scp /data/efe-rekor-snapshot-${BUNDLE_VERSION}.json customer-jump:/data/
$ scp /data/efe-bundle-${BUNDLE_VERSION}.cosign.bundle customer-jump:/data/
```

**Option B : Physique** (= sneakernet pour les ultra-secure)
```bash
# Burn à un dvd ou USB chiffré ; transport physique
$ dd if=/data/efe-bundle-${BUNDLE_VERSION}.tar of=/dev/usb1 bs=1M
```

**Option C : Cross-region replication** (= si même org mais
sub-account isolé)
```bash
$ aws s3 cp /data/efe-bundle-${BUNDLE_VERSION}.tar \
    s3://customer-isolated-bucket/efe-bundles/ \
    --sse AES256 --region eu-restricted-1
```

### 2.4 Étape 4 : Push vers registry interne du client

```bash
# Sur le jump host du client (= accès au registry interne) :
$ skopeo login internal-registry.customer.local \
    --username admin --password-stdin < customer-token

$ skopeo copy --dest-tls-verify=true \
    docker-archive:/data/efe-bundle-${BUNDLE_VERSION}.tar:efe-bundle:${BUNDLE_VERSION} \
    docker://internal-registry.customer.local/efe/bundles/${BUNDLE_ID}:${BUNDLE_VERSION}

# Vérifier que le push a marché
$ skopeo inspect docker://internal-registry.customer.local/efe/bundles/${BUNDLE_ID}:${BUNDLE_VERSION}
```

### 2.5 Étape 5 : Configuration EFE helm chart

```yaml
# values-customer.yaml (= override le default)
bundle:
  registry: internal-registry.customer.local
  id: paraphrase-multilingual-MiniLM-L12-v2
  version: 1.0.0
  pull_policy: IfNotPresent
  trust_store_path: /etc/efe/trust-store.toml
  rekor_offline_cache: /etc/efe/efe-rekor-cache.json
```

```bash
# Deploy depuis le jump host (= dans le réseau isolé)
$ helm upgrade efe efe/efe-stack \
    --values values-customer.yaml \
    --set bundle.registry=internal-registry.customer.local
```

---

## 3. Vérification post-deployment

### 3.1 Boot du replica

```bash
$ kubectl logs -n efe efe-server-0 | grep BUNDLE

[INFO] BUNDLE_VERIFY : 9 checks ...
[INFO] BUNDLE_VERIFY : pass (= manifest_sha256 match trust store)
[INFO] BUNDLE_VERIFY : rekor offline cache used (= air-gap mode)
[INFO] BUNDLE_PREWARM : 100 inferences p99=42ms
[INFO] BUNDLE_LOADED_AT_BOOT = 1
[INFO] BUNDLE_FINGERPRINT_INFO bundle_id=paraphrase-multilingual-MiniLM-L12-v2 version=1.0.0
```

### 3.2 Sanity test

```bash
$ kubectl exec -n efe efe-server-0 -- /efe/bin/efe-bundle verify \
    /efe/bundle/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/

[PASS] manifest_valid
[PASS] components_hashed_ok
[PASS] signature_valid
[SKIP] cosign_rekor_valid (= offline mode, using snapshot)
[PASS] license_present
[PASS] onnx_loadable
[PASS] tokenizer_loadable
[PASS] reproducibility_attested
[PASS] recall_smoke_test_passed
```

### 3.3 Premier classify

```bash
$ grpcurl -d '{"text": "test"}' \
    internal-efe.customer.local:50051 efe.v1.SwarmService.Execute
{
  "verdict": {
    "label": "support_triage",
    "confidence": 0.892
  }
}
```

---

## 4. Rotation en air-gap

À chaque rotation 6 mois (cf. `bundle_rotation.md`), le client doit
répéter les étapes 2.1 → 2.5. Pour automatiser, le client peut
développer un **cron interne** sur le jump host :

```bash
# /etc/cron.d/efe-bundle-rotation
0 4 1 */6 * /usr/local/bin/efe-airgap-rotate.sh
```

Le script `efe-airgap-rotate.sh` :
1. Pull la dernière version depuis `registry.efe.ai`
2. Verify cosign en ligne
3. Push vers internal registry
4. Email customer ops pour confirmer la rotation

---

## 5. Troubleshooting

| Symptôme | Cause probable | Fix |
|----------|----------------|-----|
| `cosign verify-blob` fail offline | Rekor cache expiré | Re-download cache depuis jump host |
| `BUNDLE_VERIFY rekor_missing` | Pas de cache local | Configurer `rekor_offline_cache` dans values.yaml |
| `kubectl rollout` timeout | Registry interne unreachable | Vérifier DNS internal-registry.customer.local |
| Hash mismatch | Bundle corrompu pendant transfer | Re-transfer + checksum SHA-256 |
| Signature invalid | Trust store pas updated | Push new trust-store.toml dans image |

---

## 6. Liens utiles

- Architecture : `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 8.3
- skopeo doc : https://github.com/containers/skopeo
- cosign offline : https://docs.sigstore.dev/cosign/verifying/verify/#offline-verification
- Customer DPA template : `docs/legal/dpa_template.md`
