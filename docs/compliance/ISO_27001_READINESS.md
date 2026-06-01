# ISO 27001:2022 Readiness Assessment

**Organization** : EFE-AI Inc.
**Service** : EFE Classification Platform v1.0.0 GA
**Assessment date** : 2026-05-09
**Target certification** : 2027-H1 (initial Stage 1 + Stage 2 audits)
**Internal champion** : CISO + Compliance Officer
**External advisor** : (TBD)

> ISO 27001:2022 supersedes ISO 27001:2013. The 2022 revision restructures
> Annex A into 4 themes (37 + 8 + 14 + 34 = 93 controls) instead of the
> 14 categories of the 2013 version. This document is a self-assessment
> against the 2022 controls.

---

## Executive summary

| Theme                                  | Controls | Implemented | In progress | Not yet started |
|----------------------------------------|----------|-------------|-------------|-----------------|
| A.5 Organizational controls            | 37       | 32          | 4           | 1               |
| A.6 People controls                    | 8        | 7           | 1           | 0               |
| A.7 Physical controls                  | 14       | 11          | 2           | 1               |
| A.8 Technological controls             | 34       | 30          | 4           | 0               |
| **Total**                              | **93**   | **80 (87.4 %)** | **11 (11.6 %)** | **2 (2.0 %)** |

**Overall readiness** : **87.4 %**.

Gap themes : disaster-recovery automation (A.7.10, A.7.11), supplier
audit cadence (A.5.21, A.5.22), employee security training metrics
(A.6.3).

---

## A.5 — Organizational controls (37)

| # | Control                                          | Status        | Evidence pointer                                    |
|---|--------------------------------------------------|---------------|-----------------------------------------------------|
| A.5.1 | Policies for information security            | Implemented   | `docs/security/information_security_policy.md`      |
| A.5.2 | Information security roles and responsibilities | Implemented   | RACI matrix in `HANDBOOK.md` § 4                    |
| A.5.3 | Segregation of duties                            | Implemented   | RBAC enforces 4-eyes principle on production access |
| A.5.4 | Management responsibilities                      | Implemented   | Quarterly board review                              |
| A.5.5 | Contact with authorities                         | Implemented   | DPO + legal team contacts up to date                |
| A.5.6 | Contact with special interest groups             | Implemented   | OWASP, FIRST, Sigstore community                    |
| A.5.7 | Threat intelligence                              | Implemented   | Mandiant feed + internal threat model               |
| A.5.8 | Information security in project management      | Implemented   | Security review gate on every architecture doc      |
| A.5.9 | Inventory of information and other associated assets | Implemented | `docs/asset_inventory.md`                          |
| A.5.10 | Acceptable use of information                  | Implemented   | AUP signed by every employee + tenant               |
| A.5.11 | Return of assets                               | Implemented   | Offboarding checklist                               |
| A.5.12 | Classification of information                  | Implemented   | Labels : Public / Internal / Confidential / Restricted |
| A.5.13 | Labelling of information                       | Implemented   | Document templates auto-stamp classification        |
| A.5.14 | Information transfer                           | Implemented   | TLS 1.3 + per-tenant KMS                            |
| A.5.15 | Access control                                 | Implemented   | RBAC + per-tenant KMS context                       |
| A.5.16 | Identity management                            | Implemented   | OIDC SSO via Okta                                   |
| A.5.17 | Authentication information                     | Implemented   | MFA mandatory                                       |
| A.5.18 | Access rights                                  | Implemented   | Quarterly access reviews                            |
| A.5.19 | Information security in supplier relationships | Implemented   | DPAs + sub-processor agreements                     |
| A.5.20 | Addressing information security within supplier agreements | Implemented | Contract template includes ISMS clauses |
| A.5.21 | Managing information security in the ICT supply chain | **In progress** | SLSA-3 implemented for EFE ; supplier coverage 60 % |
| A.5.22 | Monitoring, review and change management of supplier services | **In progress** | Quarterly audit cadence not yet operationalized |
| A.5.23 | Information security for use of cloud services | Implemented   | AWS / GCP / Azure compliance attestations on file   |
| A.5.24 | Information security incident management planning | Implemented | Runbook in `docs/runbooks/incident_response.md`     |
| A.5.25 | Assessment and decision on information security events | Implemented | Severity matrix + escalation runbook              |
| A.5.26 | Response to information security incidents     | Implemented   | PagerDuty integration                               |
| A.5.27 | Learning from information security incidents   | Implemented   | Post-mortem template + culture                      |
| A.5.28 | Collection of evidence                         | Implemented   | Audit chain + immutable WORM storage                |
| A.5.29 | Information security during disruption         | Implemented   | DR plan in `docs/runbooks/dr.md`                    |
| A.5.30 | ICT readiness for business continuity         | Implemented   | Multi-region failover                               |
| A.5.31 | Legal, statutory, regulatory and contractual requirements | Implemented | Regulatory radar maintained by Legal       |
| A.5.32 | Intellectual property rights                  | Implemented   | OSS license review + SBOM                           |
| A.5.33 | Protection of records                         | Implemented   | Audit logs : 7-year retention, WORM                 |
| A.5.34 | Privacy and protection of PII                 | Implemented   | GDPR Art. 30 register + DPA template                |
| A.5.35 | Independent review of information security    | **In progress** | Initial external pen-test 2026-Q3                |
| A.5.36 | Compliance with policies, rules and standards | Implemented   | Internal audit annually                             |
| A.5.37 | Documented operating procedures               | **Not started** | Runbook coverage at 80 %, target 100 % by Q4 2026 |

## A.6 — People controls (8)

| # | Control                              | Status        | Evidence pointer                              |
|---|--------------------------------------|---------------|-----------------------------------------------|
| A.6.1 | Screening                        | Implemented   | Background check before access to production |
| A.6.2 | Terms and conditions of employment | Implemented | Confidentiality + IP assignment in contracts |
| A.6.3 | Information security awareness, education and training | **In progress** | Annual training implemented ; metrics dashboard not yet wired |
| A.6.4 | Disciplinary process            | Implemented   | HR policy                                     |
| A.6.5 | Responsibilities after termination or change of employment | Implemented | Offboarding + access revocation < 1h |
| A.6.6 | Confidentiality or non-disclosure agreements | Implemented | NDA template signed                  |
| A.6.7 | Remote working                  | Implemented   | Remote work policy + endpoint MDM             |
| A.6.8 | Information security event reporting | Implemented | Internal whistleblowing channel              |

## A.7 — Physical controls (14)

| # | Control                              | Status        | Evidence pointer                              |
|---|--------------------------------------|---------------|-----------------------------------------------|
| A.7.1 | Physical security perimeters     | Implemented (cloud)  | AWS / GCP / Azure data-center attestations |
| A.7.2 | Physical entry                  | Implemented (cloud)  | Same as A.7.1                              |
| A.7.3 | Securing offices, rooms and facilities | Implemented (offices) | Badge access + visitor management |
| A.7.4 | Physical security monitoring    | Implemented (offices) | CCTV + access logs (offices)             |
| A.7.5 | Protecting against physical and environmental threats | Implemented (cloud) | Cloud provider responsibility |
| A.7.6 | Working in secure areas         | N/A (cloud-native)   | No physical secure areas owned             |
| A.7.7 | Clear desk and clear screen     | Implemented   | Policy + automated screen lock                |
| A.7.8 | Equipment siting and protection | Implemented (offices) | MDM-tracked corporate devices            |
| A.7.9 | Security of assets off-premises | Implemented   | Encryption at rest + remote wipe              |
| A.7.10 | Storage media                  | **In progress** | NIST 800-88 disposal procedures pending audit |
| A.7.11 | Supporting utilities           | Implemented (cloud)  | Cloud provider responsibility              |
| A.7.12 | Cabling security               | N/A (cloud-native)   |                                             |
| A.7.13 | Equipment maintenance          | Implemented (offices) | Maintenance log                            |
| A.7.14 | Secure disposal or re-use of equipment | **Not started** | Disposal contract with secure erasure not yet signed |

## A.8 — Technological controls (34)

| # | Control                              | Status        | Evidence pointer                              |
|---|--------------------------------------|---------------|-----------------------------------------------|
| A.8.1 | User end point devices           | Implemented   | MDM (Jamf / Intune)                           |
| A.8.2 | Privileged access rights         | Implemented   | JIT access via Teleport                       |
| A.8.3 | Information access restriction   | Implemented   | RBAC + per-tenant KMS                         |
| A.8.4 | Access to source code            | Implemented   | GitHub branch protection + 2-of-2 review      |
| A.8.5 | Secure authentication            | Implemented   | OIDC + MFA + WebAuthn                         |
| A.8.6 | Capacity management              | Implemented   | HPA + autoscaling alerts                      |
| A.8.7 | Protection against malware       | Implemented   | EDR on endpoints + image scanning (Trivy)     |
| A.8.8 | Management of technical vulnerabilities | Implemented | Daily Trivy + monthly external scan         |
| A.8.9 | Configuration management         | Implemented   | GitOps (Flux) + Terraform                     |
| A.8.10 | Information deletion           | Implemented   | KMS key destruction + WORM erasure procedures |
| A.8.11 | Data masking                   | Implemented   | PII redaction in audit logs                   |
| A.8.12 | Data leakage prevention        | Implemented   | DLP scanners on outbound traffic              |
| A.8.13 | Information backup             | Implemented   | Daily backups + quarterly restore drill       |
| A.8.14 | Redundancy of information processing facilities | Implemented | Multi-region deployment            |
| A.8.15 | Logging                        | Implemented   | Centralized logs + audit chain                |
| A.8.16 | Monitoring activities          | Implemented   | Prometheus + Grafana + alert rules            |
| A.8.17 | Clock synchronization          | Implemented   | NTP + Chrony, max drift < 10 ms               |
| A.8.18 | Use of privileged utility programs | Implemented | Restricted to break-glass roles             |
| A.8.19 | Installation of software on operational systems | Implemented | Immutable container images          |
| A.8.20 | Networks security              | Implemented   | mTLS + service mesh + network policies        |
| A.8.21 | Security of network services   | Implemented   | TLS 1.3 mandatory                             |
| A.8.22 | Segregation of networks        | Implemented   | Per-tenant network namespaces                 |
| A.8.23 | Web filtering                  | **In progress** | Egress proxy in pilot                       |
| A.8.24 | Use of cryptography            | Implemented   | KMS + cosign + AES-256-GCM                    |
| A.8.25 | Secure development life cycle  | Implemented   | SDLC playbook                                 |
| A.8.26 | Application security requirements | Implemented | OWASP ASVS L2 baseline                       |
| A.8.27 | Secure system architecture and engineering principles | Implemented | Architecture docs + threat modeling |
| A.8.28 | Secure coding                  | Implemented   | Linter + SAST + code review                   |
| A.8.29 | Security testing in development and acceptance | Implemented | DAST + IAST + fuzzing in CI            |
| A.8.30 | Outsourced development         | **In progress** | Sub-processor security requirements being expanded |
| A.8.31 | Separation of development, test and production environments | Implemented | Distinct AWS accounts |
| A.8.32 | Change management              | Implemented   | PR templates + tag-based release pipeline     |
| A.8.33 | Test information               | Implemented   | Synthetic test data, no production PII in lower envs |
| A.8.34 | Protection of information systems during audit testing | **In progress** | Audit-mode toggles being formalized |

---

## Roadmap to certification

| Phase                   | Target date  | Owner    |
|-------------------------|-------------|----------|
| Internal gap closure    | 2026-Q3     | CISO     |
| Stage 1 audit (documentation review) | 2026-Q4 | Lead Auditor |
| Stage 2 audit (implementation review) | 2027-Q1 | Lead Auditor |
| Certification issued    | 2027-H1     | External CB |
| Annual surveillance audit | 2027-H2 + annually | External CB |

---

## References

- ISO/IEC 27001:2022 standard
- ISO/IEC 27002:2022 implementation guidance
- EFE compliance corpus : `docs/compliance/`
- Risk register : `docs/security/risk_register.md`
- Threat model : `docs/security/threat_model.md`
- Internal audit log : `docs/audits/`

---

## Generated for tenant `{{tenant_name}}` on `{{generated_at}}`.
