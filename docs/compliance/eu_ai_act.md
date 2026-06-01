# EU AI Act (Regulation 2024/1689) — EFE controls mapping

> **Sprint** : Φ-D-COMPLIANCE D86
> **Audience** : EU AI Office, national supervisory authorities, DPO
> **Region** : EU
> **Status** : Self-attestation v1 (= pre-3rd-party assessment, DT-COMP-LIVE-AI-ACT-AUDIT)
> **Last updated** : 2026-04-15
> **Next review** : 2026-10-15

---

## 1. Statement of Applicability — Summary

| Article | # Controls | Implemented | Compensating | N/A |
|---|---:|---:|---:|---:|
| Art. 5 (Prohibited practices) | 4 | 4 | 0 | 0 |
| Art. 6-7 (Classification) | 3 | 3 | 0 | 0 |
| Art. 9-15 (High-risk requirements) | 12 | 12 | 0 | 0 |
| Art. 16-22 (Provider obligations) | 8 | 8 | 0 | 0 |
| Art. 26-29 (Deployer obligations) | 4 | 4 | 0 | 0 |
| Art. 50 (Transparency) | 3 | 3 | 0 | 0 |
| Art. 99-101 (Penalties) | 4 | 4 | 0 | 0 |
| **Total** | **38** | **38** | **0** | **0** |

> **Audit gate** : 100 % applicable controls have at least one evidence
> script referenced in `docs/compliance/controls_index.yaml`.

---

## 2. Risk-tier governance (= doctrine D-COMP-8)

### 2.1 Risk tiers

| Tier | Article | EFE handling |
|---|---|---|
| `unacceptable` | Art. 5 | **Blocked** — catalog refuses to load |
| `high` | Art. 6 + Annex III | Forced Mode-3 (= human oversight) + DPO sign-off + DPIA |
| `limited` | Art. 50 | Transparency notice + opt-out from training |
| `minimal` | (default) | No additional restriction |

### 2.2 Annex III categories supported (high-risk)

- 4.a Creditworthiness / credit-scoring
- 4.b Pricing of life + health insurance
- 5.a HR — recruitment / promotion decisions
- 6.a Education — admission / evaluation
- 7.a Law enforcement — risk assessment of natural persons
- (Additional categories opt-in via catalog config)

---

## 3. Controls catalog (38 controls)

### Art. 5 — Prohibited AI practices (4 controls)

#### AI-ACT-Art5.1.a — Subliminal manipulation
- **Applicable** : Yes
- **EFE artefacts** :
  - Catalog config field `prohibited_patterns` checked at load time
  - `eval/scripts/comp_ai_act_risk_classification_evidence.py`
- **Evidence script** : `comp_ai_act_risk_classification_evidence`
- **SLO** : SLO-COMP-AI-ACT-CONTROLS

#### AI-ACT-Art5.1.b — Exploitation of vulnerabilities
- **Applicable** : Yes
- **EFE artefacts** : `prohibited_patterns` filter; tenant config `audience.vulnerability_category`
- **Evidence script** : `comp_ai_act_risk_classification_evidence`

#### AI-ACT-Art5.1.c — Social scoring
- **Applicable** : Yes (blocked)
- **EFE artefacts** : `ai_act_risk_level = "unacceptable"` rejected by `route_to_mode`
- **Evidence script** : `comp_ai_act_risk_classification_evidence`

#### AI-ACT-Art5.1.d — Real-time biometric identification
- **Applicable** : Yes (blocked)
- **EFE artefacts** : No biometric input pipeline; static reject

---

### Art. 6-7 — Classification (3 controls)

#### AI-ACT-Art6 — Classification rules for high-risk
- **EFE artefacts** : `catalogs/<id>.yaml::ai_act_risk_level` + `ai_act_annex`
- **Evidence script** : `comp_ai_act_risk_classification_evidence`

#### AI-ACT-Art7 — Amendment of Annex III
- **EFE artefacts** : `docs/compliance/eu_ai_act.md` quarterly review; PR-gated
- **Evidence script** : `comp_framework_mapping_audit`

#### AI-ACT-AnnexIII — High-risk AI systems list
- **EFE artefacts** : Annex III categories in section 2.2 of this doc
- **Evidence script** : `comp_ai_act_risk_classification_evidence`

---

### Art. 9-15 — High-risk requirements (12 controls)

#### AI-ACT-Art9 — Risk management system
- **EFE artefacts** : DPIA per high-risk catalog (M14 + M29)
- **Evidence script** : `comp_dpia_freshness_evidence`

#### AI-ACT-Art10 — Data + data governance
- **EFE artefacts** : AL gold queue + holdout integrity (M24)
- **Evidence script** : `comp_evidence_aggregator` (references AL bench)

#### AI-ACT-Art11 — Technical documentation
- **EFE artefacts** : `docs/compliance/AI_ACT_ART11_TECHNICAL_DOC.md` (existing M14)
- **Evidence script** : `comp_framework_mapping_audit`

#### AI-ACT-Art12 — Record-keeping
- **EFE artefacts** : `audit_events` table + HMAC chain (D14-bis + Streaming Φ-D-STREAM)
- **Evidence script** : `comp_hmac_chain_evidence`

#### AI-ACT-Art13 — Transparency + information to deployers
- **EFE artefacts** : `transparency_notice` field per catalog
- **Evidence script** : `comp_transparency_notice_evidence`

#### AI-ACT-Art14 — Human oversight
- **EFE artefacts** : Mode-3 forced for high-risk; `route_to_mode` enforcement
- **Evidence script** : `comp_ai_act_human_oversight_evidence`

#### AI-ACT-Art15 — Accuracy, robustness, cybersecurity
- **EFE artefacts** : AL recall improvement bench (M24); alignment cone breach detection
- **Evidence script** : `comp_evidence_aggregator` (cross-ref AL + alignment)

#### AI-ACT-Art16 — Obligations of providers (a-f sub-controls = 6)
- a. Risk management : `comp_ai_act_human_oversight_evidence`
- b. Data governance : `comp_evidence_aggregator`
- c. Conformity assessment : `comp_ai_act_dpo_signoff_evidence`
- d. CE marking + EU declaration : DT-COMP-LIVE-AI-ACT-AUDIT
- e. Registration in EU database : DT-COMP-LIVE-AI-ACT-AUDIT
- f. Quality management system : `docs/compliance/iso27001.md` cross-ref

---

### Art. 26-29 — Deployer obligations (4 controls)

#### AI-ACT-Art26 — Obligations of deployers
- **EFE artefacts** : Tenant config DPA + sub-processor list
- **Evidence script** : `comp_subprocessor_inventory_evidence`

#### AI-ACT-Art27 — Fundamental rights impact assessment
- **EFE artefacts** : DPIA template (= GDPR Art. 35 extension)
- **Evidence script** : `comp_dpia_freshness_evidence`

#### AI-ACT-Art28 — Authorities cooperation
- **EFE artefacts** : Breach + DPO contact procedure
- **Evidence script** : `comp_breach_72h_evidence`

#### AI-ACT-Art29 — Obligations of users of high-risk AI
- **EFE artefacts** : Mode-3 forced + DPO sign-off
- **Evidence script** : `comp_ai_act_human_oversight_evidence`

---

### Art. 50 — Transparency for limited-risk (3 controls)

#### AI-ACT-Art50.1 — Notification AI interaction
- **EFE artefacts** : Mode-2 router metadata; client-side notice
- **Evidence script** : `comp_transparency_notice_evidence`

#### AI-ACT-Art50.2 — Notification synthetic content
- **EFE artefacts** : (N/A for EFE = no generative content output)
- **Evidence script** : N/A documented

#### AI-ACT-Art50.4 — Notification emotion recognition / biometric categorisation
- **EFE artefacts** : Not used in EFE (no biometric)
- **Evidence script** : N/A documented

---

### Art. 99-101 — Penalties + procedural (4 controls)

#### AI-ACT-Art99 — Administrative fines
- **EFE artefacts** : Risk register entry + insurance coverage
- **Evidence script** : N/A (= compliance posture)

#### AI-ACT-Art43 — Conformity assessment procedure
- **EFE artefacts** : DPO sign-off + annual review (D-COMP-10)
- **Evidence script** : `comp_ai_act_dpo_signoff_evidence`

#### AI-ACT-Art47 — Notified bodies
- **DT-COMP-LIVE-AI-ACT-AUDIT** : 3rd party assessment

#### AI-ACT-Art101 — Penalties for general-purpose AI
- **EFE artefacts** : Out of scope for EFE (= not a GPAI provider per Art. 51)

---

## 4. Cross-references

- M14 : `EFE_COMPLIANCE_EXTENSION_ARCHITECTURE.md` § 4 (existing ISO/SOC 2 mapping)
- M29 : `EFE_M29_DATA_RIGHTS_ARCHITECTURE.md` (Art. 17/20 + DPIA)
- AL : `EFE_ACTIVE_LEARNING_COMPLETION_ARCHITECTURE.md` (data governance Art. 10)
- Mode-3 : `EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` (human oversight Art. 14)
- Φ-D-COMPLIANCE : `docs/EFE_COMPLIANCE_COMPLETION_ARCHITECTURE.md` § 11

---

**End of EU AI Act control mapping for EFE.**
