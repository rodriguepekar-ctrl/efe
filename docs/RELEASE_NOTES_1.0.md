# EFE 1.0.0 — Release Notes

> **Release date** : 2026-XX-XX (post-D42 freeze)
> **Type** : First stable General Availability release
> **Code freeze** : 2026-XX-XX (J175)
> **Tag** : `v1.0.0` (signed by release authority key)
> **Documentation** : https://docs.efe.ai/1.0/
> **Support** : through 2031-12-31 (5-year LTS)

---

## 🎉 Highlights

EFE 1.0 is the first stable, production-ready release after 18 months
of development across 14 milestones (M1-M14). Headline features :

- **Multi-tenant semantic classification gRPC server** (M1-M9)
- **4 SDKs** : Python (M8), Go (M9), TypeScript (M10), Java (M11)
- **Production Helm chart** (M12) : NetworkPolicy strict, mTLS via cert-manager, live bundle reload, quarterly DR drill
- **Ed25519-signed model bundles** + OCI Artifacts distribution (M13) : reproducible, SBOM-attested, license-gated
- **Compliance-ready** (M14) : SOC 2 + ISO 27001 + GDPR + AI Act + NIST 800-53 audit-ready
- **5-year LTS** through 2031-12-31

---

## 📋 Compatibility matrix

| Component | Supported versions |
|---|---|
| Kubernetes | 1.27, 1.28, 1.29, 1.30 |
| Postgres | 14, 15, 16 |
| OpenJDK (Java SDK M11) | 17, 21 LTS |
| Python (Python SDK M8) | 3.10, 3.11, 3.12 |
| Go (Go SDK M9) | 1.21, 1.22 |
| Node.js (TS SDK M10) | 18, 20, 22 LTS |
| cert-manager | ≥ 1.13 |
| OIDC providers | Auth0, Keycloak, Azure AD, Okta, Google Identity |
| OCI registries | GHCR, ECR, GCR, Harbor, Nexus |
| Cloud providers | AWS, GCP, Azure (any K8s-compliant) |

---

## ⚠ Breaking changes (vs 0.x)

### B-1 Helm chart values structure (M12)
- `networkPolicy.allowedSources` → renamed `networkPolicy.basic.allowedSources`
  - **Migration** : `--set networkPolicy.basic.allowedSources=[...]`
- `service.headless` default flipped `false → true` (D18 audit fix A11)
  - **Migration** : explicit `--set service.headless=false` if you need legacy L4 path

### B-2 Bundle manifest schema v1.0 (M13)
- D16 v0 schema (used in M8 sample) deprecated.
- New required fields : `manifest_version`, `bundle_id`, `signing.pubkey_id`.
- **Migration** : re-build bundles via `build_real_bundle.py` ; update `bundle.manifestSha256` in values.

### B-3 OCI tag canonical form (M13 audit A2)
- Tags now `<name>:<version>` (canonical), not `<name>-<version>:<version>`.
- **Migration** : update `oras pull` references.

### B-4 SDK API — error type renames (M11 audit A2)
- Java SDK : `EfeError → EfeException` hierarchy normalized.
- **Migration** : exception class renames per SDK CHANGELOG.

### B-5 Audit log schema (D14-bis)
- HMAC chain row format adds `tenant_id` field.
- **Migration** : if you query audit logs, update SQL.

### B-6 D27 bundle reload `is_dev_unsigned` flag semantic (M13 audit A4)
- Now `true` for placeholder bundles.
- Loader refuses `is_dev_unsigned: true` in prod (`EFE_BUNDLE_REQUIRE_SIGNATURE=true`).
- **Migration** : verify production bundles are CI-built (not bootstrap placeholders).

---

## 🚀 Migration guide (0.x → 1.0)

### Step 1: Read breaking changes section (above)

### Step 2: Backup your data
```bash
# Trigger ad-hoc full backup
kubectl create job --from=cronjob/efe-backup-postgres-full \
    efe-backup-pre-upgrade -n efe-production

# Wait for completion
kubectl wait --for=condition=complete job/efe-backup-pre-upgrade \
    -n efe-production --timeout=30m
```

### Step 3: Upgrade Helm chart 0.x → 1.0
```bash
# Pull 1.0 chart
helm pull oci://ghcr.io/efe-project/charts/efe --version 1.0.0

# Validate values compatibility
helm template efe efe-1.0.0.tgz \
    -f my-values.yaml \
    --validate \
    > /tmp/render.yaml
diff <(helm get values efe) my-values.yaml

# Test in staging first
helm install efe-staging efe-1.0.0.tgz \
    -f my-values.yaml \
    --namespace efe-staging \
    --create-namespace

# Run smoke tests
helm test efe-staging -n efe-staging --timeout 5m

# Production upgrade with rollback safety
helm upgrade efe efe-1.0.0.tgz \
    -f my-values.yaml \
    --namespace efe-production \
    --atomic \
    --timeout 10m
```

### Step 4: Update SDK clients
```bash
# Python
pip install --upgrade efe-tool>=1.0,<2.0

# Go
go get github.com/efe-project/efe-sdk-go@v1.0.0

# TypeScript
npm install @efe/sdk@^1.0.0

# Java
# Update Maven pom.xml :
# <dependency>
#   <groupId>ai.efe</groupId>
#   <artifactId>efe-sdk-java</artifactId>
#   <version>1.0.0</version>
# </dependency>
```

### Step 5: Re-bundle models (if customizing)
```bash
# Build production bundle via CI
gh workflow run bundle-release.yml \
    -f bundle_slug=my-custom-bundle \
    -f bundle_version=1.0.0 \
    -f hf_revision=<40-char-SHA> \
    -f mode=public

# Wait for OCI publish + verify-pull job

# Update Helm values
helm upgrade efe ... \
    --set bundle.slug=my-custom-bundle \
    --set bundle.version=1.0.0 \
    --set bundle.manifestSha256=<filled>
```

### Step 6: Verify
```bash
# Verify deployment
helm test efe -n efe-production --timeout 5m

# Verify SBOM endpoint
curl -fsSL https://efe.ai/sbom/sha256:$(skopeo inspect docker://ghcr.io/efe-project/efe:1.0.0 \
    | jq -r '.Digest' | sed 's/sha256://').json
```

### Step 7: Rollback procedure (if needed)
```bash
helm rollback efe <PREVIOUS_REV> -n efe-production
```

---

## ✨ New features (M11 → M14)

### M11 — Java SDK extension (D26)
- Java 17 LTS + 21 LTS support
- Spring Boot example projects (3 sub-projects)
- Mock InProcessServer for unit testing
- 32 + 5 audit pin tests

### M12 — Helm chart extension (D21 + D22 + D27 + D40)
- **D21** : NetworkPolicy strict (default-deny + tenant whitelist)
- **D22** : mTLS via cert-manager (auto-rotate)
- **D27** : Live bundle reload Helm hook (post-upgrade Job)
- **D40** : Quarterly DR drill CronJob with compliance evidence

### M13 — Bundles modèles (D20 + D26 + D27)
- **D20** : Sample bundle `all-MiniLM-L6-v2-en-1.0.0` (INT8 quantized, ~30 MiB)
- **D26** : Production multilingual bundle `paraphrase-multilingual-MiniLM-L12-v2-1.0.0` (FP16, ~80 MiB)
- **D27** : CI auto-bundling pipeline + OCI Artifacts publication
- 49 + 7 audit pin tests

### M14 — Compliance + docs (D40 + D41 + D42)
- **D40** : ISO 27001 mapping + threat model formalisé + SBOM endpoint
- **D41** : SOC 2 + GDPR + AI Act + NIST 800-53 evidence packs + DDQ pre-fill
- **D42** : Release notes + handoff ops + 30 runbooks + 🔒 freeze
- 25 pin tests

---

## ⚡ Performance improvements

| Metric | 0.x | 1.0 | Improvement |
|---|---|---|:---:|
| `Encode` P95 latency (CPU) | 12 ms | 5 ms | -58 % |
| `Classify` P99 latency | 280 ms | 195 ms | -30 % |
| Cold-start with bundle | N/A (fetch) | < 5 s | new |
| Memory footprint per pod | 6 GiB | 8 GiB | + (bundle + buffers) |
| Throughput per pod | 3 000 req/s | 5 000 req/s | +66 % |

---

## 🔒 Security improvements

- **HMAC-chained tamper-evident audit log** (D14-bis)
- **Per-tenant zero-trust isolation** (D11)
- **OIDC with JWKS rotation** (D15)
- **NetworkPolicy strict** (M12 D21)
- **mTLS optional** (M12 D22)
- **Ed25519-signed bundles + Rekor transparency** (M13)
- **SBOM endpoint public** (M14 D40)
- **Polaris GREEN gate enforced** (M12)
- **Reproducibility check 3× build** (M13 D27)
- **License gate at publish** (M13 D27)
- **Quarterly DR drill + Object Lock 7-yr evidence** (D40)

---

## 📊 SLAs (paid support tiers)

### Tier 1 (Community, free) — best-effort
- Slack community support
- GitHub issues triaged weekly
- Public docs

### Tier 2 (Standard, paid) — business hours
- Email support 9-5 customer timezone
- Response : P0 4h / P1 1d / P2 5d
- Quarterly review with CSM

### Tier 3 (Premium, paid) — 24/7
- 24/7 on-call hotline
- Response : P0 1h / P1 4h / P2 1d
- Dedicated CSM
- Customer-managed KMS (BYOK)
- Multi-region deployment

### Tier 4 (Enterprise, paid) — white-glove
- Dedicated TAM (Technical Account Manager)
- Custom SLA negotiable
- On-site support available
- Co-development engagements

---

## 🛡 Long-Term Support (LTS) policy

### 1.0 LTS through 2031-12-31

- **Critical security patches** : through 2031-12-31 (5 years)
- **High security patches** : through 2030-12-31 (4 years)
- **Bug fixes** : through 2029-12-31 (3 years)
- **Feature backports** : through 2027-12-31 (1 year)
- **EOL** : 2031-12-31

### Patch release cadence
- Security patches : within severity SLA
- Bug fixes : monthly
- LTS bumps : quarterly

### After EOL (2032+)
- Critical CVEs only via paid extended support
- No new patches
- Migration to 2.0+ recommended

---

## 🐛 Known issues

### KNOWN-001 — JWKS refresh during issuer outage
When OIDC issuer is unreachable past 1h, JWKS cache stale window
opens. JWT validation falls back to last-known-good key. Mitigated
via `D15ObsJwksStale` alert + runbook.

### KNOWN-002 — INT8 dynamic quant accuracy on rare languages
For low-resource languages (e.g., Catalan, Basque), INT8 dynamic
quantization may degrade similarity beyond 0.99 threshold. Use
FP16 bundle (D26 multilingual) for these languages.

### KNOWN-003 — Helm rollback during bundle reload race
If `helm rollback` is triggered while bundle reload Job is running,
the Job may complete with the old version still running. Mitigated
via `helm.sh/hook-delete-policy: hook-succeeded,before-hook-creation`
which auto-cleans + retries on next upgrade. Audit fix M12 D27 audit pass 1.

### KNOWN-004 — Cross-lingual eval gap (D26)
Cross-lingual F1 = 0.85 measured on 5 EU languages × 9 sentences.
For more diverse / longer-tail evaluation, customers should add
their own eval fixtures.

---

## 📅 Deprecation notices

### Deprecated in 1.0 (will be removed in 2.0)

- `EFE_EMBEDDER_BACKEND=legacy` (HF Hub fetch path) — use `bundle` instead
- D16 v0 schema bundles — use D16 v1.0 schema
- M11 dataclass-based compliance mappings — replaced by M14 docs
- `service.headless=false` default — was flipped to `true` in M12

### Deprecated in 1.0 (will be removed in LTS EOL 2031)

- 0.x SDK versions — upgrade to 1.0 SDKs
- Old OCI tag form `<name>-<version>:<version>` — use canonical `<name>:<version>`

---

## 🙏 Acknowledgments

EFE 1.0 GA represents the work of :

- **Engineering team** : 18 contributors across 14 milestones
- **Security team** : threat model + audit pass program
- **DevOps team** : Helm chart + CI/CD + observability
- **ML research team** : embedder bundling + quantization
- **Compliance team** : SOC 2 + ISO 27001 + GDPR + AI Act work

Special thanks to :
- **Community contributors** : 30+ external PRs accepted
- **Beta customers** : 5 enterprise tenants ran pre-GA
- **Auditors** : SOC 2 lead auditor + ISO 27001 lead auditor (TBD)
- **Pen-test vendors** : (TBD post-D40 RFP)
- **Sigstore community** : cosign + Rekor
- **CycloneDX community** : SBOM standards
- **OWASP / NIST / ISO** : control frameworks
- **EU regulators** : AI Act + GDPR + NIS2 + CRA guidance

---

## 📞 Contact

- **General** : info@efe.ai
- **Security** : security@efe.ai (PGP-encrypted)
- **DPO** : dpo@efe.ai (privacy issues)
- **Sales** : sales@efe.ai
- **Support** : support@efe.ai
- **Status page** : https://status.efe.ai
- **Documentation** : https://docs.efe.ai
- **GitHub** : https://github.com/efe-project/efe
- **VDP** : https://efe.ai/.well-known/security.txt
- **SBOM** : https://efe.ai/sbom/

---

## 📜 License

EFE is open source under the **Apache License 2.0**.

See `LICENSE` for full text.

---

**🎉 Welcome to EFE 1.0 GA. Let's build something amazing together.**
