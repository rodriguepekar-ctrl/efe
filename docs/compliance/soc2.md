# SOC 2 Trust Services Criteria — EFE controls index

> **Sprint** : Φ-D-COMPLIANCE D86
> **Audience** : SOC 2 auditor (CPA firm), customer trust review teams
> **Status** : Type I self-attestation. Type II = DT-COMP-SOC2-TYPE2-AUDIT.
> **Last updated** : 2026-04-15
> **Audit period** : 2026-04-01 → 2027-03-31

---

## 1. Statement of Applicability — Summary

| TSC category | # Controls | Implemented | N/A |
|---|---:|---:|---:|
| CC1 Control Environment | 5 | 5 | 0 |
| CC2 Communication + Information | 3 | 3 | 0 |
| CC3 Risk Assessment | 4 | 4 | 0 |
| CC4 Monitoring | 2 | 2 | 0 |
| CC5 Control Activities | 3 | 3 | 0 |
| CC6 Logical + Physical Access | 8 | 6 | 2 (= no physical office, see below) |
| CC7 System Operations | 5 | 5 | 0 |
| CC8 Change Management | 1 | 1 | 0 |
| CC9 Risk Mitigation | 2 | 2 | 0 |
| A1 Availability | 3 | 3 | 0 |
| C1 Confidentiality | 2 | 2 | 0 |
| PI1 Processing Integrity | 5 | 5 | 0 |
| P1 Privacy | 11 | 11 | 0 |
| P2-P8 Privacy detailed | 12 | 8 | 4 (= consumer-portal aspects not applicable to B2B) |
| **Total** | **64** | **58** | **6** |

> **Not applicable rationale** :
> - CC6.4-CC6.6 (physical security) = no physical office, cloud BAA covers.
> - P3.1, P5.1, P6.4, P7.1 = consumer-facing portals not in EFE B2B scope.

---

## 2. Controls catalog — Trust Service Criteria

### CC1 Control Environment (5)
- **CC1.1** : Commitment to integrity + ethics — `SECURITY.md`, Code of Conduct
- **CC1.2** : Board oversight of internal control — Quarterly compliance review
- **CC1.3** : Management establishes structures + reporting — Org chart + RACI
- **CC1.4** : Demonstrates commitment to attract / retain competent people — Hiring + L&D records
- **CC1.5** : Holds people accountable — Performance reviews

### CC2 Communication + Information (3)
- **CC2.1** : Information to support functioning — Internal docs + ROPA
  - Evidence : `comp_ropa_freshness_evidence`
- **CC2.2** : Communicates information internally — Slack + Confluence + standups
- **CC2.3** : Communicates externally — Trust center + status page

### CC3 Risk Assessment (4)
- **CC3.1** : Specifies objectives — Annual planning
- **CC3.2** : Identifies + analyzes risk — `docs/compliance/threat_model.md`
- **CC3.3** : Considers fraud risk — Risk register
- **CC3.4** : Identifies + assesses change — Change management process

### CC4 Monitoring (2)
- **CC4.1** : Conducts ongoing + separate evaluations — Daily compliance audit + annual review
  - Evidence : `comp_audit_chain_daily`
- **CC4.2** : Evaluates + communicates deficiencies — Incident response

### CC5 Control Activities (3)
- **CC5.1** : Selects + develops control activities — Per Annex A mapping
- **CC5.2** : Selects + develops general technology controls — RBAC + audit chain
  - Evidence : `comp_tenant_isolation_evidence`
- **CC5.3** : Deploys via policies + procedures — Documented SOPs

### CC6 Logical + Physical Access (8 — 2 N/A)
- **CC6.1** : Implements logical access controls — JWT scopes + RBAC
  - Evidence : `comp_tenant_isolation_evidence`
- **CC6.2** : Authorization, modification, removal — IAM workflow
  - Evidence : `comp_tenant_isolation_evidence`
- **CC6.3** : Restricts user access — Tenant isolation
- **CC6.4** : (N/A) Physical access — Cloud BAA
- **CC6.5** : (N/A) Logical/physical asset disposal — Cloud-managed
- **CC6.6** : (N/A) Protects against external threats — Cloud BAA + WAF
- **CC6.7** : Transmission + disposal of confidential data — TLS 1.3 + encryption at rest
  - Evidence : `comp_encryption_at_rest_evidence`
- **CC6.8** : Implements controls to prevent + detect — IDS + audit chain
  - Evidence : `comp_hmac_chain_evidence`

### CC7 System Operations (5)
- **CC7.1** : Detection of security events — `audit_events` + Prometheus alerts
  - Evidence : `comp_hmac_chain_evidence`
- **CC7.2** : Monitoring of security events — Daily replay
  - Evidence : `comp_audit_chain_daily`
- **CC7.3** : Evaluates security events — Triage runbook
- **CC7.4** : Responds to + recovers from incidents — Breach response (`docs/runbook/comp_breach_response.md`)
  - Evidence : `comp_breach_72h_evidence` + `comp_breach_24h_containment_evidence`
- **CC7.5** : Tests + identifies + addresses system vulnerabilities — Pentest + SBOM

### CC8 Change Management (1)
- **CC8.1** : Authorizes, designs, develops, tests changes — Git PR review + CI gates

### CC9 Risk Mitigation (2)
- **CC9.1** : Identifies + mitigates risks of business disruptions — DR drill quarterly
- **CC9.2** : Assesses + manages risks of vendors + business partners — Sub-processor list
  - Evidence : `comp_subprocessor_inventory_evidence`

### A1 Availability (3)
- **A1.1** : Monitors capacity + demand — Capacity planning
- **A1.2** : Authorizes, designs, develops + tests recovery infrastructure — DR drill
- **A1.3** : Tests recovery plan — Quarterly DR drill

### C1 Confidentiality (2)
- **C1.1** : Identifies + maintains confidential information — Data residency + tenant isolation
  - Evidence : `comp_data_residency_evidence` + `comp_tenant_isolation_evidence`
- **C1.2** : Disposes of confidential information — Erasure cascade
  - Evidence : `comp_erasure_30d_evidence`

### PI1 Processing Integrity (5)
- **PI1.1** : Obtains + uses information to meet processing objectives — AL gold queue
- **PI1.2** : Implements policies + procedures over inputs — Schema validation
- **PI1.3** : Implements policies + procedures over processing — Mode-1/2/3 deterministic
- **PI1.4** : Implements policies + procedures over outputs — Verdict + audit trail
  - Evidence : `comp_hmac_chain_evidence`
- **PI1.5** : Stores inputs, outputs, processing — Audit retention policy

### P1 Privacy (11) — All implemented via GDPR/CCPA mappings
- P1.1 Notice of objectives — Transparency notice
- P1.2 Notice of practices — DPA
- P1.3 Consent obtained — `comp_consent_granularity_evidence`
- P2.1 Notice of purpose — DPA + ROPA
- P3.1 (N/A consumer portal)
- P4.1 Collection limited — Data minimization
- P4.2 Use limited — Purpose limitation
- P4.3 Retention — Retention policy
- P5.1 (N/A consumer portal)
- P6.1 Access requests — Portability + access (M29)
- P6.2 Correction requests — AL edit
- P6.3 Deletion requests — Erasure (M29)
- P6.4 (N/A consumer portal)
- P7.1 (N/A consumer portal)
- P8.1 Inquiries + complaints — DPO contact

---

## 3. Cross-references

- `iso27001.md` — ISO 27001 Annex A mapping (substantial overlap with CC6/CC7)
- `gdpr.md` — GDPR controls (substantial overlap with P1-P8)
- `soc2_evidence.md` — M14 evidence template (existing)
- `SOC2_EVIDENCE_TEMPLATE.md` — M14 template (existing)
- Φ-D-COMPLIANCE archi § 14 — SLO list

---

## 4. Path to SOC 2 Type II

- DT-COMP-SOC2-TYPE2-AUDIT — Engage SOC 2 auditor for 6-12 month observation period.
- Pre-requisite : All Type I controls demonstrably operating + evidence collected for the audit period.

---

**End of SOC 2 TSC control index for EFE.**
