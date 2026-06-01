# HIPAA-ready (Baseline) — EFE controls mapping

> **Sprint** : Φ-D-COMPLIANCE D86
> **Audience** : US healthcare prospects, BAA (Business Associate Agreement) due-diligence
> **Region** : US
> **Status** : **Baseline only** — full HIPAA covered entity / BA designation
> requires a dedicated sprint + signed BAA + targeted controls
> hardening. This document attests to the **baseline** controls available
> in EFE 1.0 GA that align with HIPAA Security + Privacy Rules.
> **Last updated** : 2026-04-15

---

## 1. Scope

HIPAA-ready = controls in place such that EFE **can** be deployed for
PHI-touching workloads provided that :

1. A signed BAA exists between EFE and the covered entity.
2. The PHI workload uses Mode-3 (= human oversight + audit).
3. Sub-processor list excludes non-BAA-signed providers.

**Out of scope for HIPAA-ready** :
- ePHI-specific encryption key controls (= dedicated KMS sprint)
- Full breach notification timer per § 164.404 (= 60 days, integrated with our 72h GDPR mechanism)
- Patient-portal access controls (= EFE is B2B, not direct patient-facing)

---

## 2. Statement of Applicability — Summary

| HIPAA section | # Controls | Baseline-implemented | Compensating | N/A |
|---|---:|---:|---:|---:|
| § 164.308 Administrative safeguards | 6 | 6 | 0 | 0 |
| § 164.310 Physical safeguards | 4 | 0 | 4 | 0 |
| § 164.312 Technical safeguards | 6 | 6 | 0 | 0 |
| § 164.404 Breach notification | 2 | 2 | 0 | 0 |
| **Total** | **18** | **14** | **4** | **0** |

> Physical safeguards (§ 164.310) are 100 % compensating because EFE
> is fully cloud-hosted with no physical office. The cloud provider
> (AWS / GCP / Azure) covers the physical controls via their HIPAA-
> eligible BAA.

---

## 3. Controls catalog (18 controls)

### § 164.308 Administrative safeguards (6)

#### HIPAA-164.308.a.1.i — Security Management Process
- **EFE artefacts** : Risk register, threat model M14, SOC 2 CC mapping
- **Evidence script** : `comp_framework_mapping_audit`

#### HIPAA-164.308.a.1.ii.B — Risk Management
- **EFE artefacts** : Quarterly risk review (D-COMP-10)
- **Evidence script** : `comp_framework_mapping_audit`

#### HIPAA-164.308.a.1.ii.D — Information System Activity Review
- **EFE artefacts** : `audit_events` HMAC chain + daily replay
- **Evidence script** : `comp_hmac_chain_evidence`

#### HIPAA-164.308.a.3.i — Workforce Security
- **EFE artefacts** : RBAC scopes (D22 mTLS), audit log
- **Evidence script** : `comp_tenant_isolation_evidence`

#### HIPAA-164.308.a.4.i — Access Management
- **EFE artefacts** : JWT scopes, tenant isolation
- **Evidence script** : `comp_tenant_isolation_evidence`

#### HIPAA-164.308.a.6.i — Security Incident Procedures
- **EFE artefacts** : Breach response runbook + 72h notification
- **Evidence script** : `comp_breach_72h_evidence`

---

### § 164.310 Physical safeguards (4) — Compensated by cloud BAA

#### HIPAA-164.310.a.1 — Facility Access Controls
- **Compensating** : Cloud provider HIPAA BAA (AWS / GCP / Azure)
- **Evidence** : Cloud provider attestation (referenced)

#### HIPAA-164.310.b — Workstation Use
- **Compensating** : Remote-only workforce, MDM-managed devices
- **Evidence** : Internal MDM policy

#### HIPAA-164.310.c — Workstation Security
- **Compensating** : Same as above

#### HIPAA-164.310.d.1 — Device and Media Controls
- **Compensating** : Cloud provider key management

---

### § 164.312 Technical safeguards (6)

#### HIPAA-164.312.a.1 — Access Control
- **EFE artefacts** : `tenant_id` filter strict + RBAC
- **Evidence script** : `comp_tenant_isolation_evidence`

#### HIPAA-164.312.a.2.iv — Encryption and Decryption
- **EFE artefacts** : AES-256-GCM at rest (Mode-3) + TLS 1.3 in transit
- **Evidence script** : `comp_encryption_at_rest_evidence`

#### HIPAA-164.312.b — Audit Controls
- **EFE artefacts** : `audit_events` table + HMAC chain
- **Evidence script** : `comp_hmac_chain_evidence`

#### HIPAA-164.312.c.1 — Integrity
- **EFE artefacts** : HMAC chain on every event; prev_signature_hex chained
- **Evidence script** : `comp_hmac_chain_evidence`

#### HIPAA-164.312.d — Person or Entity Authentication
- **EFE artefacts** : JWT bearer + mTLS (D22)
- **Evidence script** : `comp_tenant_isolation_evidence`

#### HIPAA-164.312.e.1 — Transmission Security
- **EFE artefacts** : TLS 1.3 enforced; cert pinning option
- **Evidence script** : `comp_encryption_at_rest_evidence` (covers both rest + transit)

---

### § 164.404 Breach Notification (2)

#### HIPAA-164.404.b — Timeliness
- **EFE artefacts** : 72h DPO notification (= integrated, GDPR + HIPAA both satisfied)
- **Evidence script** : `comp_breach_72h_evidence`
- **Note** : HIPAA requires 60 days to individual ; our 72h DPO trigger starts the cascade well within.

#### HIPAA-164.530.c — Safeguards
- **EFE artefacts** : Multi-layer compensating + technical safeguards combined
- **Evidence script** : `comp_erasure_30d_evidence`

---

## 4. Path to full HIPAA designation

Beyond HIPAA-ready baseline, full deployment requires :

1. **Signed BAA** between EFE and covered entity (legal).
2. **Sub-processor BAA** with each sub-processor in the chain (OpenAI/Anthropic etc. HIPAA-eligible tier).
3. **De-identification scope** : ensure PHI is minimised at ingest (= comp_data_minimization_evidence).
4. **Risk Analysis (§ 164.308.a.1.ii.A)** : annual PHI-specific risk analysis (= extension of DPIA).
5. **Patient access** : if EFE serves patient-portal use cases, add direct access endpoints.

---

## 5. Cross-references

- GDPR : Many HIPAA controls have GDPR equivalents in `gdpr.md`
- ISO 27001 : Annex A.5.34 = "Privacy + PII protection" — directly maps to HIPAA
- AI Act : if PHI-touching AI deployment, AI Act Art. 6 applicability triggers
- Φ-D-COMPLIANCE : doctrine D-COMP-4 (breach 72h) covers HIPAA breach timeliness

---

**End of HIPAA-ready baseline mapping for EFE.**
