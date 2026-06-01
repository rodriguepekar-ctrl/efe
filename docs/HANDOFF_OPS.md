# EFE 1.0 GA — Operations Handoff Document

> **Status** : Sprint M14 D42 (Φ-C) — build → operate transition.
> **Audience** : ops team, on-call engineers, incident responders.
> **Effective** : post-J175 freeze (2026-XX-XX).
> **Last updated** : 2026-04-30.

---

## 1. Mission

Document the transition from build mode (M1-M14 active development)
to operate mode (post-1.0 GA freeze).

---

## 2. Ownership matrix (RACI)

| Component | Owner (R) | Backup (R) | Reviewer (C) | Decision (A) |
|---|---|---|---|---|
| Helm chart M12 | DevOps lead | Eng lead | Security lead | Eng VP |
| Bundle pipeline M13 | ML lead | DevOps lead | Security lead | Eng VP |
| OIDC D15 | Security lead | DevOps lead | Eng lead | CISO |
| Postgres backend D14 | Data eng | DevOps lead | Eng lead | Eng VP |
| Multi-tenant D11 | Eng lead | Security lead | DevOps lead | Eng VP |
| Backup D17 | DevOps lead | Eng lead | Security lead | Eng VP + DPO |
| DR drill D40 | DevOps lead | Compliance lead | CISO | CEO |
| SBOM endpoint M14-D40 | Security lead | DevOps lead | Eng lead | CISO |
| Threat model M14-D40 | Security lead | Eng lead | CISO | CISO |
| Pen-test M14-D40 | Security lead | Eng VP | CISO | CEO + CISO |
| Audit log D14-bis | Security lead | Eng lead | DPO | CISO |
| Observability D13 | DevOps lead | SRE | Eng lead | Eng VP |
| HPA D9 | DevOps lead | SRE | Eng lead | Eng VP |
| Replica D18 | DevOps lead | SRE | Eng lead | Eng VP |
| Compliance docs M14 | Compliance lead | CISO | DPO + Legal | CEO |
| Customer support | Support eng | CSM | Sales eng | Customer Success VP |
| Sales engineering | Sales eng | Sales lead | Marketing | Sales VP |
| Marketing / docs | Marketing lead | Tech writer | Eng VP | CMO |

---

## 3. On-call rotation

### 3.1 Schedule

```
Week 1 : Primary (eng-1) + Secondary (eng-2)
Week 2 : Primary (eng-2) + Secondary (eng-3)
Week 3 : Primary (eng-3) + Secondary (eng-4)
Week 4 : Primary (eng-4) + Secondary (eng-1)
[rotates 4-week cycle]
```

### 3.2 Coverage

- 24/7 on-call for Tier 3+ paying customers
- Business-hours for Tier 1-2 (community + standard)
- Holiday / vacation backup arranged 4 weeks in advance

### 3.3 PagerDuty / Opsgenie config

- P0 : page primary immediately ; escalate to secondary if unack 15 min
- P1 : page primary ; escalate after 30 min
- P2 : email + Slack ; escalate during business hours
- P3 : Slack only ; tracked in backlog

### 3.4 Escalation tree

```
                  L1 — On-call engineer
                        │
                        │ (15-min escalate if no response)
                        ▼
                  L2 — Eng lead
                        │
                        │ (30-min escalate if P0 not contained)
                        ▼
                  L3 — CTO / Eng VP
                        │
                        │ (60-min if critical breach / regulatory)
                        ▼
                  L4 — CEO + CISO + DPO + Legal
                        │
                        │ (Customer notification approval)
                        ▼
                  External : Customers + EU DPA + Regulators
```

---

## 4. Critical credentials inventory

### 4.1 Secrets that MUST be rotated periodically

| Secret | Owner | Cadence | Last rotated | Next rotation |
|---|---|---|---|---|
| `EFE_BUNDLE_PRIVKEY_PEM` (M13) | Security lead | Quarterly | 2026-Q1 | 2026-Q2 |
| OIDC issuer secrets (D15) | Security lead | Annual | 2025-Q4 | 2026-Q4 |
| HMAC keys (D14-bis) | Security lead | Quarterly | 2026-Q1 | 2026-Q2 |
| TLS certificates (M12 D22) | DevOps lead | Auto-rotate cert-manager (30d before expiry) | rolling | rolling |
| Cloud IAM (S3, KMS) | DevOps lead | Annual | 2025-Q4 | 2026-Q4 |
| GHCR PAT | DevOps lead | Annual | 2025-Q4 | 2026-Q4 |
| Slack webhook | DevOps lead | Ad-hoc | 2025-Q3 | as-needed |
| AWS_SBOM_PUBLISHER_ROLE | DevOps lead | Annual | 2025-Q4 | 2026-Q4 |
| AWS_EVIDENCE_ROLE | Security lead | Annual | 2025-Q4 | 2026-Q4 |
| Cosign keypair | Security lead | Annual | 2025-Q4 | 2026-Q4 |

### 4.2 Storage locations

- KMS HSM-backed : bundle privkeys, HMAC keys
- AWS Secrets Manager : OIDC issuer secrets, DB passwords
- GH Encrypted Secrets : `EFE_BUNDLE_PRIVKEY_PEM`, registry creds
- Vault (Tier 3) : customer-managed
- 1Password / similar (operator personal) : never for production

### 4.3 Rotation procedure

For each secret type, see corresponding runbook :
- `m13_signing_key_rotation.md` (bundle keys)
- `d15_oidc_jwks_stale.md` (OIDC issuer rotation)
- `d22_mtls_cert_renewal_failed.md` (TLS rotation)
- `general_secret_rotation_playbook.md` (general procedure)

---

## 5. Production environment overview

### 5.1 Cluster topology

```
- Region : <customer-chosen> (typically us-east-1 / eu-west-1)
- AZs : 3 minimum
- Worker pools :
  - efe-server : 3-30 pods (HPA D9)
  - efe-pg : 1-3 pods (PG operator)
  - cert-manager : 1 pod
  - cron-jobs : ephemeral (D17 + D40)
- Ingress : ingress-nginx + cert-manager + WAF
- Service mesh : optional (linkerd / istio)
- Storage classes : efe-ssd-gp3 (block), efe-fs-efs (NFS)
```

### 5.2 Network architecture

- VPC private subnets only for pods
- Public subnet for ingress LB only
- NetworkPolicy strict (M12 D21) per default
- mTLS between services (M12 D22) optional
- Egress allowlist : kube-dns + audit aggregator + S3 backup + OIDC

### 5.3 DNS records authoritative source

- `efe.ai` : Route 53 (or customer DNS)
- `*.efe.ai` : managed via Terraform `efe-dns-records.tf`
- Status page : status.efe.ai (Statuspage / customer-managed)
- Docs : docs.efe.ai
- SBOM endpoint : efe.ai/sbom (CloudFront)

---

## 6. Build pipeline ownership

| Pipeline | Workflow | Owner |
|---|---|---|
| efe-server release | `.github/workflows/release.yml` | DevOps lead |
| Helm chart release | `.github/workflows/helm-publish.yml` | DevOps lead |
| Bundle release (M13 D27) | `.github/workflows/bundle-release.yml` | ML lead + Security lead |
| Helm chart test | `.github/workflows/helm-test.yml` | DevOps lead |
| Bench | `.github/workflows/bench.yml` | Eng lead |
| SDK Go publish | `.github/workflows/sdk-go-publish.yml` | Eng lead |
| SDK TS publish | `.github/workflows/sdk-typescript-publish.yml` | Eng lead |
| SDK Java publish | `.github/workflows/sdk-java-publish.yml` | Eng lead |
| Reproducibility check | `.github/workflows/reproducibility.yml` | Security lead |
| Evidence collection (M14) | `.github/workflows/evidence-collection.yml` | Security lead |
| Docs site | `.github/workflows/docs.yml` | Tech writer |

---

## 7. Customer escalation procedure

### 7.1 Customer-reported P0/P1 (data loss / outage)

1. Customer reports via support@efe.ai or PagerDuty integration
2. CSM acknowledges within 15 min (Tier 3) / 1h (Tier 2)
3. CSM escalates to on-call engineer
4. On-call follows incident playbook
5. Hourly customer updates until resolved
6. Post-mortem within 5 business days

### 7.2 Customer-reported P2/P3

1. Support ticket created
2. Triaged within 24h (Tier 2-3) / 5d (Tier 1)
3. Assigned to engineering team
4. SLA per severity matrix (RELEASE_NOTES §SLA)

### 7.3 Compliance / privacy / security questions

- Privacy → DPO (dpo@efe.ai)
- Security → security@efe.ai
- Compliance → compliance@efe.ai
- General legal → legal@efe.ai

---

## 8. Vendor contacts

### 8.1 Cloud providers (sub-processors)

| Vendor | Contact | TAM |
|---|---|---|
| AWS | aws-support (case via console) | TAM-name@aws |
| GCP | google-cloud-support | TAM-name@google |
| Azure | azure-support | TAM-name@microsoft |

### 8.2 Critical vendors

| Vendor | Service | Contact | SLA |
|---|---|---|---|
| Auth0 | OIDC issuer (optional) | support@auth0.com | 24h Tier 3 |
| Datadog | Observability (Tier 3) | support@datadoghq.com | 24h |
| Cloudflare | CDN / WAF | support@cloudflare.com | 24h |
| HackerOne | Bug bounty (post-1.0) | support@hackerone.com | 24h |
| Sigstore | cosign + Rekor (community) | community@sigstore.dev | best-effort |

### 8.3 Audit / certification vendors

| Vendor | Service | Engagement |
|---|---|---|
| SOC 2 auditor (TBD) | Annual Type II | Engaged Q1 2026 |
| ISO 27001 auditor (TBD) | Annual surveillance | Engaged Q3 2026 |
| Pen-test vendor (TBD) | Annual external | Engaged Q3 2026 |

---

## 9. Compliance officer contacts

| Role | Contact | Responsibility |
|---|---|---|
| CISO | ciso@efe.ai | Overall security posture |
| DPO | dpo@efe.ai | GDPR + privacy |
| Compliance lead | compliance@efe.ai | Audit liaison |
| Legal counsel | legal@efe.ai | Contract review + IP |
| Privacy counsel | privacy@efe.ai | Privacy contracts |

---

## 10. Post-incident review template

```markdown
# Post-Incident Review : <Incident Title>

## Summary
- Date/Time UTC : 
- Duration : 
- Severity : P0/P1/P2/P3
- Customers impacted : 
- Data impact : 

## Timeline
- T+0 : 
- T+30min : 
- T+1h : 
- T+resolution : 

## Root cause
- Technical : 
- Process : 

## Impact
- SLA breach : Yes/No
- Customer notifications sent : 
- Regulator notifications : 

## Mitigation deployed
- Short-term : 
- Long-term : 

## Action items
- [ ] 
- [ ] 
- [ ] 

## Lessons learned
- 
- 

## Sign-off
- Eng lead : 
- CISO : 
- DPO (if privacy) : 
- CEO (if material) : 
```

---

## 11. Operational metrics dashboards

### 11.1 Production health (Grafana)

- `prod/efe-overview` : RPS, error rate, latency P99
- `prod/efe-tenants` : per-tenant breakdown
- `prod/efe-postgres` : connection pool, slow queries
- `prod/efe-bundle` : load durations, signature verification
- `prod/efe-audit` : HMAC chain health
- `prod/compliance-health` : pin tests + Polaris status

### 11.2 Business metrics (Grafana / Looker)

- DAU/WAU/MAU per tenant
- Customer satisfaction (NPS)
- SLA compliance rates
- Audit findings open

---

## 12. Change management post-freeze

See `docs/CONTRIBUTING.md` post-freeze update.

Allowed PR types :
- Security patches (CVE response, pen-test findings)
- Critical bug-fixes (P0 outage causes)
- Compliance updates
- Documentation clarifications

PRs require :
- 2 reviewers (post-freeze)
- CODEOWNERS approval
- All status checks green
- Signed commits

---

## 13. Release procedure post-freeze

```bash
# Patch release (1.0.x)
# 1. Cherry-pick fix to release branch
git checkout release-1.0
git cherry-pick <commit-sha>

# 2. Bump version
echo "1.0.1" > VERSION
git add VERSION
git commit -m "Release 1.0.1"

# 3. Tag (signed)
git tag -s v1.0.1 -m "EFE 1.0.1 — security patch"
git push origin v1.0.1

# 4. Trigger release workflow (auto-pushes to OCI)
# (release.yml workflow runs on tag push)

# 5. Update RELEASE_NOTES_1.0.md changelog section
# 6. Notify customers via status.efe.ai
```

---

## 14. Onboarding new ops engineer

### 14.1 Day 1
- Access provisioning : VPN + GH + cloud IAM + Slack
- Read CONTRIBUTING + SECURITY + this doc
- Pair with current on-call

### 14.2 Week 1
- Shadow incidents (read-only)
- Run through 5 runbooks in staging
- Read all `docs/EFE_*_EXTENSION_ARCHITECTURE.md`

### 14.3 Month 1
- Solo P3 incidents
- Backup on-call

### 14.4 Month 3
- Primary on-call rotation
- Author runbook updates

---

## 15. Operational tooling

| Tool | Purpose | Access |
|---|---|---|
| `kubectl` | K8s management | RBAC scoped per env |
| `helm` | Chart management | RBAC scoped |
| `oras` | OCI artifact mgmt | OIDC |
| `cosign` | Sig + verify | OIDC |
| `cyclonedx-cli` | SBOM validate | local |
| `aws cli` | AWS ops | IAM AssumeRole |
| `gh cli` | GH workflow trigger | PAT scoped |
| `cmctl` | cert-manager ops | RBAC scoped |
| `kubectl logs` | log inspection | RBAC scoped |
| Grafana / Loki | observability | OIDC |
| Slack | comms | SSO |
| PagerDuty | on-call | SSO |

---

## 16. Off-boarding ops engineer

```bash
# Per termination checklist
1. Revoke OIDC access (issuer admin)
2. Remove from GH org / repo
3. Revoke cloud IAM AssumeRole entries
4. Rotate any keys they had personal copies of
5. Remove from PagerDuty rotation
6. Archive Slack DM history
7. Final equipment return / wipe
8. Exit interview
```

---

## 17. Continuous improvement

- Quarterly handoff doc review
- Post-incident reviews update this doc
- New runbooks added as scenarios emerge

---

## 18. Frequently consulted external resources

- ISO 27001:2022 standard (purchased)
- NIST SP 800-53 r5 (free)
- AICPA Trust Services Criteria (purchased)
- AWS / GCP / Azure security best practices
- CIS Kubernetes Benchmark
- OWASP Top 10
- ENISA Threat Landscape reports
