# GDPR Article 32 — Security of Processing — EFE 1.0 GA

> **Status** : Sprint M14 D41 (Φ-B) — GDPR Art. 32 evidence + DPIA support.
> **Audience** : EU DPAs (CNIL France, BfDI Germany, ICO UK, etc.), DPO,
> compliance team, customer DPOs.
> **Regulation** : Regulation (EU) 2016/679 (GDPR) Art. 32.
> **Last updated** : 2026-04-30.

---

## 1. Scope

This document attests EFE 1.0 GA's compliance with GDPR Article 32(1) :

> "Taking into account the state of the art, the costs of implementation
> and the nature, scope, context and purposes of processing as well as
> the risk of varying likelihood and severity for the rights and freedoms
> of natural persons, the controller and the processor shall implement
> appropriate technical and organisational measures to ensure a level of
> security appropriate to the risk."

EFE acts as a **processor** (Art. 4(8)) on behalf of customer
**controllers** (Art. 4(7)). DPA (Art. 28) governs the relationship.

---

## 2. Article 32(1) — 4 measures + EFE controls

### 2.1 (a) Pseudonymisation and encryption of personal data

#### 2.1.1 Pseudonymisation

**EFE control** : `EFE_PII_HASH=true` (D13) — hashes potential PII
fields (emails, SSN-shaped strings, phone numbers) before storing in
audit logs or sending to OTel traces.

**Implementation** : `crates/efe-server/src/observability/pii_hash.rs`.

**Evidence** :
- Pin test `test_d13_pii_hash_applies_to_audit_log.py`
- Pin test `test_d13_pii_hash_applies_to_otel_spans.py`

#### 2.1.2 Encryption in transit

**EFE control** :
- TLS 1.3 minimum for all gRPC + REST endpoints
- mTLS optional (M12 D22 cert-manager-managed)
- HSTS header enforced

**Evidence** :
- Helm values default `tls.minVersion: TLSv1.3`
- M12 D22 Certificate template

#### 2.1.3 Encryption at rest

**EFE control** :
- K8s Secret AES-256 (cloud provider managed)
- Postgres TDE (Tier 3 deployments)
- S3 SSE-KMS (backup + evidence + SBOM buckets)
- Customer-managed KMS keys (Tier 3 BYOK)

**Evidence** :
- D14 Postgres values reference KMS-encrypted secrets
- D17 backup S3 buckets have `--sse aws:kms` enforced
- M14 D40 SBOM bucket S3 SSE-KMS

### 2.2 (b) Confidentiality, integrity, availability, resilience

#### 2.2.1 Confidentiality

**EFE controls** :
- OIDC D15 (auth)
- Per-tenant RBAC D11 (authz)
- NetworkPolicy strict M12 D21 (network isolation)
- mTLS M12 D22 (transport)
- PII hash D13 (data minimization)

#### 2.2.2 Integrity

**EFE controls** :
- HMAC chain audit D14-bis (tamper-evident)
- Bundle Ed25519 sig M13 (supply chain)
- Cosign image (image integrity)
- SBOM endpoint M14 D40 (verifiable provenance)

#### 2.2.3 Availability

**EFE controls** :
- HPA D9 (3-30 replicas auto-scale)
- Multi-replica D18 + leader election + PDB
- DR drill D40 (quarterly)
- SLO P99 < 200 ms (M11-RPC-3)

#### 2.2.4 Resilience

**EFE controls** :
- D17 4-CronJob backup pipeline (full + diff + WAL + bundle)
- D40 quarterly drill from-scratch restore
- Multi-region backup option
- 30-min RTO weekly / 45-min RTO quarterly target

### 2.3 (c) Restoration capability

**EFE controls** :
- D17 backup pipeline + retention policy (4 weeks full + 14 days diff
  + 14 days WAL + 30 days vault + 4 weeks bundle)
- Restore procedure documented in `dr_drill_quarterly_evidence.md`
- Hello-World end-to-end validation per drill
- Object Lock COMPLIANCE 7-yr (immutable evidence)

**Test cadence** :
- Weekly drill (D17) — RTO 30 min
- Quarterly drill (D40) — RTO 45 min, full Hello-World workflow
- Compliance evidence pack per quarter

### 2.4 (d) Process for regularly testing, assessing and evaluating

**EFE controls** :
- Quarterly DR drill (operational test)
- Pin tests CI gate (each PR)
- Polaris GREEN gate (each Helm deploy)
- Annual pen-test (post-1.0 GA)
- Annual SOC 2 Type II audit
- Annual ISO 27001 surveillance
- Quarterly compliance review (`continuous_compliance.md`)

---

## 3. DPIA — Data Protection Impact Assessment

### 3.1 When DPIA is required (Art. 35)

A DPIA is required when processing is likely to result in high risk
to natural persons' rights and freedoms, particularly :
- Systematic + extensive evaluation including profiling
- Special categories of data (Art. 9) at large scale
- Systematic monitoring of public areas at large scale

**EFE assessment** : Standard EFE deployment does NOT require a DPIA.
However, customers using EFE for :
- Profiling-style classification (job applicant scoring, credit decisions)
- Special-category inference (health, religion, sexual orientation)
must conduct a DPIA per Art. 35.

### 3.2 DPIA template

See `docs/compliance/gdpr_dpia_template.md` for customer-fillable template.

---

## 4. Data Subject Rights (Articles 15-22)

### 4.1 Right of access (Art. 15)

**EFE support** :
- Audit log query by data subject (HMAC chain D14-bis)
- Customer's responsibility to surface access UI
- 30-day SLA for access requests

### 4.2 Right to rectification (Art. 16)

**EFE support** : Catalog versions D12 — operator can publish corrected version ; old version retained for audit.

### 4.3 Right to erasure (Art. 17)

**EFE support** :
- Tenant offboarding procedure (full data deletion)
- 30-day retention post-deletion (right to recovery)
- Crypto-shred at end of retention
- Audit log preserved per legal obligation (Art. 17(3)(e))

### 4.4 Right to restriction of processing (Art. 18)

**EFE support** : Tenant data freeze procedure (read-only mode for the tenant).

### 4.5 Right to data portability (Art. 20)

**EFE support** :
- Audit log export in JSON / NDJSON
- Catalog export via gRPC `Export` RPC (D12)
- Bundle artifact download (M13)

### 4.6 Right to object (Art. 21)

**EFE support** : Customer's responsibility to surface objection UI ; EFE provides the data freeze API.

### 4.7 Automated individual decision-making (Art. 22)

**EFE control** : EFE classifications are inputs to customer's
decision processes ; not stand-alone decisions. EFE provides
confidence scores so customers can implement human review for
borderline cases.

---

## 5. International transfers (Chapter V)

### 5.1 Standard Contractual Clauses (SCC)

EU customers transferring data to non-EU regions :
- SCC 2021/914 (Module 2 : controller-to-processor) included in DPA
- Schrems II compliance via additional safeguards :
  - Encryption at rest with EU-only KMS
  - Customer-managed KMS keys (Tier 3 BYOK option)
  - Logical access restrictions (no US-side admin access by default)

### 5.2 Adequacy decisions

EFE deployments in adequacy-recognized countries (UK, Switzerland,
Japan, etc.) do not require SCC but maintain DPA.

### 5.3 Transfer Impact Assessment (TIA)

Required for transfers to non-adequate countries. Template :
`gdpr_tia_template.md` (post-D42 plan).

---

## 6. Sub-processors (Art. 28)

### 6.1 Current sub-processors

| Sub-processor | Service | Region | DPA in place |
|---|---|---|:---:|
| AWS | Compute + storage | Customer-chosen region | ✅ |
| GCP | Compute + storage | Customer-chosen region | ✅ |
| Azure | Compute + storage | Customer-chosen region | ✅ |
| Auth0 | OIDC issuer (optional) | Customer-chosen region | ✅ |
| Datadog | Observability (Tier 3 optional) | Customer-chosen region | ✅ |
| Stripe | Billing (SaaS only) | EU + US | ✅ |

### 6.2 Sub-processor change notification

30-day prior notice via email to customer DPO + status.efe.ai notice.

### 6.3 Customer right to object

If customer objects within 30 days, EFE will :
1. Propose alternative
2. If no alternative, customer may terminate (no penalty)

---

## 7. Article 33 — Breach notification

### 7.1 Timeline

```
T+0      : Incident detected (alert / customer report)
T+1h     : SOC severity assessed (P0/P1)
T+4h     : DPO notified
T+24h    : Customer notification draft prepared
T+72h    : EU DPA notification sent (Art. 33 deadline)
T+1d     : Customer notification sent (Art. 34 if high risk)
T+7d     : Public statement on status.efe.ai
T+14d    : Post-mortem published (sanitized)
T+30d    : Remediation report to DPA
```

### 7.2 Notification template

Template `breach_notification_template.md` (DPA-ready).

### 7.3 Triage criteria

- **Personal data confirmed exposed** → Art. 33 + Art. 34 notifications
- **Personal data potentially exposed** → Art. 33 only (DPA notified)
- **No personal data involved** → internal incident only

---

## 8. Customer DPO support

EFE DPO contact : **dpo@efe.ai** (PGP-encrypted available).

EFE DPO responsibilities :
- Quarterly review of EFE's GDPR posture
- Customer DPO liaison on technical questions
- DPA review for new customers
- Sub-processor DPA review

---

## 9. Records of processing activities (Art. 30)

EFE maintains a Record of Processing Activities (RoPA) :
- Categories of data subjects (customer-controlled)
- Categories of personal data (customer-classified)
- Recipients (customer-controlled)
- Transfers + safeguards
- Retention schedules
- Security measures (this document)

Customer's RoPA is their responsibility ; EFE provides RoPA template
in `gdpr_ropa_template.md`.

---

## 10. Audit + certification

- Annual SOC 2 Type II (covers GDPR Art. 32 by reference)
- Annual ISO 27001 surveillance
- Optional ISO 27018 audit (PII protection in cloud)
- Customer audit right (Art. 28(3)(h)) — once per year, with notice

---

## References

- Regulation (EU) 2016/679 (General Data Protection Regulation)
- EDPB Guidelines on data protection by design (Art. 25)
- ENISA Handbook on Security of Personal Data Processing (2017)
- ICO Code of Practice for Data Protection (UK)
- CNIL Guide RGPD pour les développeurs (France)
