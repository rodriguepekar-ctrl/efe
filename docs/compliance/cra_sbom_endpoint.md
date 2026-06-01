# EU Cyber Resilience Act (CRA) — SBOM Endpoint Architecture

> **Status** : Sprint M14 D40 (Φ-A) — CRA-aligned SBOM endpoint specification.
> **Audience** : EU regulators, customers, security researchers.
> **Regulation** : EU Regulation 2024/2847 (CRA). Effective 2027-12.
> **Last updated** : 2026-04-30.

---

## 1. CRA alignment

EU CRA 2024 requires for products with digital elements :
- **Article 13** : SBOM published machine-readable
- **Article 11** : Vulnerability handling (covered in `vulnerability_disclosure.md`)
- **Article 14** : Security update mechanism (covered via Helm rolling upgrade)
- **Annex I §2** : 5-year support window post-shipment (covered in RELEASE_NOTES_1.0)

---

## 2. Endpoint contract

### 2.1 Per-artifact SBOM

```
GET https://efe.ai/sbom/<algorithm>:<digest>.json

Response 200 :
  Content-Type: application/vnd.cyclonedx+json
  ETag: "<digest>"
  Cache-Control: public, max-age=31536000, immutable
  X-EFE-Sbom-Spec: cyclonedx-1.5
  X-EFE-Sbom-Type: image | bundle | helm-chart
  Body: <CycloneDX 1.5 JSON>

Response 404 :
  Content-Type: application/json
  Body: {"error":"sbom_not_found", "digest":"<input>"}

Response 410 :
  Content-Type: application/json
  Body: {"error":"sbom_recalled", "advisory_url":"https://efe.ai/security/advisory/<id>"}
```

### 2.2 Index endpoint

```
GET https://efe.ai/sbom/index.json

Response 200 :
  {
    "version": "1.0",
    "updated_at": "2026-04-30T12:00:00Z",
    "artifacts": [
      {
        "type": "image",
        "name": "efe-server",
        "version": "1.0.0",
        "digest": "sha256:abc...",
        "sbom_url": "https://efe.ai/sbom/sha256:abc....json",
        "signed_url": "https://efe.ai/sbom/sha256:abc....json.sig",
        "license": "Apache-2.0",
        "uploaded_at": "2026-04-30T11:55:00Z"
      },
      ...
    ]
  }
```

### 2.3 Search endpoint (post-1.0 GA)

```
GET https://efe.ai/sbom/search?component=<purl>&affected_by=<cve>

Returns artifacts that contain the queried component or are affected by the CVE.
```

---

## 3. Supported digest algorithms

- `sha256:` — primary, all artifacts (default)
- `sha512:` — future, for FIPS-conformant deployments (post-1.0 GA)

---

## 4. Storage architecture

```
┌────────────────────────────┐
│   CI workflow              │
│   (release.yml /           │
│    bundle-release.yml /    │
│    helm-publish.yml)       │
└─────────┬──────────────────┘
          │
          │  on each release :
          │   1. compute artifact digest
          │   2. upload SBOM to S3
          │
          ▼
┌────────────────────────────┐
│   AWS S3 bucket            │
│   efe-sbom-prod            │
│   - Object Lock COMPLIANCE │
│   - 7-year retention       │
│   - public-read via CDN    │
└─────────┬──────────────────┘
          │
          ▼
┌────────────────────────────┐
│   CloudFront CDN           │
│   efe.ai/sbom/*            │
│   - 1-year cache TTL       │
│   - geo-distributed        │
│   - HTTPS-only + HSTS      │
└────────────────────────────┘
```

---

## 5. CI workflow integration

3 workflows upload to the S3 bucket :
- `release.yml` (efe-server image)
- `bundle-release.yml` (M13 bundles)
- `helm-publish.yml` (M12 chart)

Each step :

```yaml
- name: Upload SBOM to public CDN
  env:
    AWS_ROLE_ARN: ${{ secrets.AWS_SBOM_PUBLISHER_ROLE }}
  run: |
    set -euo pipefail
    DIGEST=$(cosign verify --output-attestations <ref> | jq -r '...')
    aws s3 cp ./SBOM.json \
        "s3://efe-sbom-prod/sha256/${DIGEST}.json" \
        --content-type "application/vnd.cyclonedx+json" \
        --cache-control "public, max-age=31536000, immutable" \
        --metadata "spec=cyclonedx-1.5,uploaded_at=$(date -u +%FT%TZ)"
    aws s3 cp ./SBOM.json.sig \
        "s3://efe-sbom-prod/sha256/${DIGEST}.json.sig" \
        --content-type "application/octet-stream"
    aws cloudfront create-invalidation \
        --distribution-id ${{ secrets.CDN_DIST_ID }} \
        --paths "/sbom/sha256/${DIGEST}.json" "/sbom/sha256/${DIGEST}.json.sig"
```

---

## 6. Index regeneration

Daily CronJob `regenerate-sbom-index.yml` runs at 02:00 UTC :

```yaml
name: regenerate-sbom-index
on:
  schedule:
    - cron: "0 2 * * *"
  workflow_dispatch:

jobs:
  regenerate:
    runs-on: ubuntu-22.04
    steps:
      - uses: aws-actions/configure-aws-credentials@v4
        with:
          role-to-assume: ${{ secrets.AWS_SBOM_PUBLISHER_ROLE }}
          aws-region: us-east-1
      - run: |
          aws s3api list-objects-v2 \
              --bucket efe-sbom-prod \
              --prefix sha256/ \
              --query 'Contents[].[Key,LastModified,Size]' \
              --output json > _objects.json
          python3 scripts/release/build_sbom_index.py \
              --objects-json _objects.json \
              --output _index.json
          aws s3 cp _index.json s3://efe-sbom-prod/index.json \
              --cache-control "public, max-age=86400" \
              --content-type "application/json"
          aws cloudfront create-invalidation \
              --distribution-id ${{ secrets.CDN_DIST_ID }} \
              --paths "/sbom/index.json"
```

---

## 7. Security posture

### 7.1 Threat mitigations

| Threat | Mitigation |
|---|---|
| Attacker uploads malicious SBOM at known digest | S3 IAM Policy : write only via CI OIDC (no human IAM) ; Object Lock COMPLIANCE prevents overwrite |
| MITM on SBOM fetch | TLS 1.3 + HSTS + Public Key Pinning (CT log monitored) |
| SBOM signature stripped | Companion `.json.sig` file (cosign) referenced in INDEX |
| DDoS on endpoint | CloudFront edge caching + AWS WAF rate limit |
| Cache poisoning at CDN | Immutable cache keys = digest itself ; can't poison without controlling S3 |
| S3 bucket takeover | KMS-encrypted + IAM Role assume rate-limited + alerting |

### 7.2 Audit log

All S3 writes logged via CloudTrail with `eventCategory=DataEvent` ;
retained 7 years per Object Lock COMPLIANCE.

---

## 8. CRA-specific requirements

| CRA Article | Requirement | EFE response |
|---|---|---|
| Art. 13(1) | SBOM machine-readable | ✅ CycloneDX 1.5 JSON |
| Art. 13(2) | SBOM updated on each new version | ✅ Per-artifact via CI |
| Art. 13(3) | SBOM accessible to authorities | ✅ Public endpoint |
| Art. 11(2) | Vulnerability handling for supported lifetime | ✅ 5-year LTS through 2031 |
| Art. 14 | Security updates | ✅ Helm rolling upgrade |
| Annex I §2(3) | Default secure config | ✅ Helm prod values + Polaris |

---

## 9. Cost analysis

| Component | Monthly cost (estimated) |
|---|---|
| S3 storage (1 TB at 10K artifacts × 100 KB) | $25 |
| CloudFront (10 TB/month outbound) | $850 |
| CloudFront requests (100M/month) | $80 |
| Total | **~$1 000/month** |

---

## 10. Roadmap

- **1.0 GA** : per-digest GET + index endpoint + cosign signatures
- **1.1** : SBOM search by component (purl)
- **1.2** : SBOM diff endpoint (compare 2 artifacts)
- **2.0** : VEX (Vulnerability Exploitability eXchange) integration

---

## References

- EU Regulation 2024/2847 (CRA)
- NTIA SBOM Minimum Elements
- CycloneDX 1.5 Specification
- Sigstore Cosign documentation
