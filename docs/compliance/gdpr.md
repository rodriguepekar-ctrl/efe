# GDPR (Regulation EU 2016/679) — EFE controls index

> **Sprint** : Φ-D-COMPLIANCE D86
> **Audience** : DPO, supervisory authorities (CNIL, ICO, BfDI, AEPD, ...)
> **Region** : EU + EEA + UK adequacy
> **Status** : Annual review v1
> **Last updated** : 2026-04-15
> **Next review** : 2027-04-15
> **Cross-reference** : This file is the INDEX of GDPR controls. Detailed
> implementations live in `gdpr_art32.md` (security), `GDPR_ART30_REGISTER.md`
> (RoPA), `gdpr_dpia_template.md` (Art. 35), `dpa_template.md` (Art. 28).

---

## 1. Statement of Applicability — Summary

| Chapter | # Controls | Implemented | Compensating | N/A |
|---|---:|---:|---:|---:|
| Ch. I (Art. 1-4) Definitions | 4 | 4 | 0 | 0 |
| Ch. II (Art. 5-11) Principles | 7 | 7 | 0 | 0 |
| Ch. III (Art. 12-23) Rights | 12 | 12 | 0 | 0 |
| Ch. IV (Art. 24-43) Controller / Processor | 14 | 14 | 0 | 0 |
| Ch. V (Art. 44-50) Transfers | 4 | 4 | 0 | 0 |
| Ch. VI (Art. 51-59) Supervisory Authorities | 2 | 2 | 0 | 0 |
| Ch. VIII (Art. 77-84) Remedies + Penalties | 4 | 4 | 0 | 0 |
| **Total** | **47** | **47** | **0** | **0** |

---

## 2. Controls catalog (47 controls)

### Chapter I — Definitions (4 controls)

#### GDPR-Art4.1 — Definition of personal data
- **EFE artefacts** : ROPA per tenant lists PI categories
- **Evidence** : `comp_ropa_freshness_evidence`

#### GDPR-Art4.2 — Definition of processing
- **EFE artefacts** : Audit events action taxonomy
- **Evidence** : `comp_hmac_chain_evidence`

#### GDPR-Art4.7 — Definition of controller
- **EFE artefacts** : DPA template (= EFE is processor, tenant is controller)
- **Evidence** : N/A (= legal posture)

#### GDPR-Art4.8 — Definition of processor
- **EFE artefacts** : DPA template
- **Evidence** : `comp_subprocessor_inventory_evidence`

---

### Chapter II — Principles (7 controls)

#### GDPR-Art5.1.a — Lawfulness, fairness, transparency
- **EFE artefacts** : Consent record + transparency notices
- **Evidence** : `comp_consent_granularity_evidence` + `comp_transparency_notice_evidence`

#### GDPR-Art5.1.b — Purpose limitation
- **EFE artefacts** : Per-catalog purpose statement
- **Evidence** : `comp_evidence_aggregator` (cross-ref)

#### GDPR-Art5.1.c — Data minimisation
- **EFE artefacts** : Mode-2/3 only forwards query_text (not PII unless consented)
- **Evidence** : `comp_evidence_aggregator`

#### GDPR-Art5.1.d — Accuracy
- **EFE artefacts** : AL EditAndConfirmGoldLabel for corrections
- **Evidence** : `comp_evidence_aggregator`

#### GDPR-Art5.1.e — Storage limitation
- **EFE artefacts** : Retention policy per category (7 days raw, 7 years audit)
- **Evidence** : `comp_ropa_freshness_evidence`

#### GDPR-Art6 — Lawfulness of processing
- **EFE artefacts** : 4-axis consent + DPA legal basis
- **Evidence** : `comp_consent_granularity_evidence`

#### GDPR-Art7 — Conditions for consent
- **EFE artefacts** : Granular opt-in/opt-out; version tracker
- **Evidence** : `comp_consent_granularity_evidence`

---

### Chapter III — Rights of the data subject (12 controls)

| Article | Subject | Evidence script |
|---|---|---|
| Art. 12 | Transparent information | `comp_transparency_notice_evidence` |
| Art. 13 | Information at collection | `comp_transparency_notice_evidence` |
| Art. 14 | Information not from subject | `comp_transparency_notice_evidence` |
| Art. 15 | Right of access | `comp_portability_30d_evidence` (= same data export) |
| Art. 16 | Right to rectification | `comp_evidence_aggregator` (= AL edit) |
| Art. 17 | Right to erasure | `comp_erasure_30d_evidence` |
| Art. 18 | Right to restriction | M29 quarantine endpoint |
| Art. 19 | Notification re rectification/erasure | Audit chain |
| Art. 20 | Right to data portability | `comp_portability_30d_evidence` |
| Art. 21 | Right to object | `comp_consent_granularity_evidence` |
| Art. 22 | Automated decision-making | Mode-3 forced + AL human review |
| Art. 23 | Restrictions | DT-COMP-LIVE-CONSENT-PIPELINE |

---

### Chapter IV — Controller + Processor (14 controls)

| Article | Subject | Evidence |
|---|---|---|
| Art. 24 | Responsibility of controller | `comp_framework_mapping_audit` |
| Art. 25 | DP by design / by default | `comp_tenant_isolation_evidence` |
| Art. 26 | Joint controllers | DPA template |
| Art. 27 | Representatives in EU | (= EFE established in EU) |
| Art. 28 | Processor | `comp_subprocessor_inventory_evidence` |
| Art. 30 | Records of processing activities | `comp_ropa_freshness_evidence` |
| Art. 32 | Security of processing | `comp_encryption_at_rest_evidence` + `comp_hmac_chain_evidence` |
| Art. 33 | Notification of breach to authority | `comp_breach_72h_evidence` |
| Art. 34 | Communication of breach to subject | `comp_breach_72h_evidence` |
| Art. 35 | DPIA | `comp_dpia_freshness_evidence` |
| Art. 36 | Prior consultation | DT-COMP-LIVE-AUTHORITY |
| Art. 37 | Designation of DPO | `comp_ai_act_dpo_signoff_evidence` |
| Art. 38 | Position of DPO | (= contractual) |
| Art. 39 | Tasks of DPO | `comp_ai_act_dpo_signoff_evidence` |

---

### Chapter V — Transfers (4 controls)

| Article | Subject | Evidence |
|---|---|---|
| Art. 44 | General principle | `comp_data_residency_evidence` |
| Art. 46 | Appropriate safeguards (SCC) | `comp_data_residency_evidence` |
| Art. 47 | Binding corporate rules | `comp_subprocessor_inventory_evidence` |
| Art. 49 | Derogations | (= not used by EFE) |

---

### Chapter VI — Supervisory Authorities (2 controls)

| Article | Subject | Evidence |
|---|---|---|
| Art. 51 | Independent SA | (= EU regulatory landscape) |
| Art. 58 | SA powers | Breach response procedure |

---

### Chapter VIII — Remedies + Penalties (4 controls)

| Article | Subject | Evidence |
|---|---|---|
| Art. 77 | Right to lodge complaint | (= consumer-facing notice) |
| Art. 79 | Right to effective judicial remedy | (= legal posture) |
| Art. 82 | Right to compensation + liability | Insurance + DPA template |
| Art. 83 | General conditions for imposing fines | Risk register entry |

---

## 3. Cross-references

- `gdpr_art32.md` — detailed Art. 32 security implementation (M14)
- `GDPR_ART30_REGISTER.md` — ROPA template (M14)
- `gdpr_dpia_template.md` — DPIA template (M14)
- `dpa_template.md` — DPA template (M14)
- `ccpa.md` — California equivalent rights
- Φ-D-COMPLIANCE archi § 14 — full SLO list

---

**End of GDPR control index for EFE.**
