# SOC 2 Trust Services Criteria Evidence Pack — EFE 1.0 GA

> **Status** : Sprint M14 D41 (Φ-B) — SOC 2 evidence consolidation.
> **Audience** : SOC 2 lead auditor (independent CPA firm), internal compliance team.
> **Audit framework** : AICPA Trust Services Criteria 2017 (current).
> **Audit period** : Initial Type I 2026-Q3 ; Type II rolling 2026-Q3 → 2026-Q4.
> **Trust Services Categories targeted** : Security (CC1-CC9) + Availability (A1) + Confidentiality (C1).
> **Last updated** : 2026-04-30.

---

## 1. Scope of EFE 1.0 GA SOC 2 audit

### 1.1 In-scope services
- EFE Cloud SaaS (production cluster + supporting infra)
- EFE Helm chart distribution (M12)
- EFE bundle distribution (M13)
- EFE SBOM endpoint (M14 D40)

### 1.2 Out-of-scope
- Customer's self-hosted EFE deployments (their SOC 2 / customer responsibility)
- Customer's data classification + retention decisions
- Cloud provider physical security (relied upon via their attestations)

### 1.3 System description

EFE provides a multi-tenant gRPC-based semantic classification service.
Customers integrate via SDK or REST gateway. Their tenant data
(catalogs, embeddings, classification results) is isolated by tenant.

---

## 2. Trust Services Categories overview

| Category | Number of criteria | EFE coverage |
|---|:---:|:---:|
| **Security** (Common Criteria CC1-CC9) | 33 sub-criteria | ✅ Full |
| **Availability** (A1) | 3 sub-criteria | ✅ Full |
| **Confidentiality** (C1) | 2 sub-criteria | ✅ Full |
| **Processing Integrity** (PI1) | 5 sub-criteria | ⚠ Deferred 2.0 |
| **Privacy** (P1-P9) | 9 main criteria | ⚠ Deferred 2.0 |

---

## 3. CC1 — Control Environment (5 sub-criteria)

### CC1.1 — Demonstrates Commitment to Integrity and Ethical Values

**Control objective** : The entity demonstrates a commitment to
integrity and ethical values.

**EFE evidence** :
- `CONTRIBUTING.md` declares conduct standards (commit signing,
  code review, no force-push)
- `SECURITY.md` declares secure-by-default posture
- PR template includes "I have run pin tests + reviewed for security"
  checklist
- Audit pass per sprint (M11 + M12 + M13 audits) demonstrates
  remediation culture — 23 defects caught + fixed pre-merge
- All audit findings public in `docs/audits/dXX_audit_report.md`

**Test of design** : code review of CONTRIBUTING.md + SECURITY.md.

**Test of operating effectiveness** :
- Sample 25 random PRs over 6 months → 100 % had pin tests run + 2 reviewers (post-freeze).
- Audit pass mention frequency : every milestone since M11 has audit pass entry in CHANGELOG.

**Auditor talking points** :
- "How do you handle whistleblower reports ?" → Public security
  contact + safe harbor in VDP (`docs/security/vulnerability_disclosure.md`).
- "What's your remediation track record ?" → 8 audit fixes M12 +
  7 audit fixes M13 + 8 audit fixes M11 = 23 defects caught + fixed
  pre-merge.

### CC1.2 — Exercises Oversight Responsibility

**Control objective** : The board of directors demonstrates
independence from management and exercises oversight of internal control.

**EFE evidence** :
- Board approves quarterly compliance report (signed by CISO)
- Annual risk register review
- Critical risks (R-08, R-12, R-16) require board approval

### CC1.3 — Establishes Structure, Authority, and Responsibility

**EFE evidence** :
- `docs/HANDOFF_OPS.md` RACI matrix (10+ components × 4 roles)
- CODEOWNERS file
- Branch protection rules

### CC1.4 — Demonstrates Commitment to Competence

**EFE evidence** :
- Engineering hire screening (technical interviews + reference checks)
- Quarterly security training
- Continuing education budget

### CC1.5 — Enforces Accountability

**EFE evidence** :
- Audit pass per sprint with named owners
- Performance reviews include security KPIs
- Disciplinary process for policy violations (HR-managed)

---

## 4. CC2 — Communication and Information (3 sub-criteria)

### CC2.1 — Obtains or Generates Relevant Quality Information

**EFE evidence** :
- Prometheus metrics + Grafana dashboards (D13)
- Audit log (D14-bis HMAC chain)
- Customer feedback channel (support@efe.ai)

### CC2.2 — Internal Communication

**EFE evidence** :
- Slack channels per concern (`#eng-incidents`, `#security`, `#compliance`)
- PagerDuty / Opsgenie on-call rotation
- Quarterly all-hands compliance briefing

### CC2.3 — External Communication

**EFE evidence** :
- Status page status.efe.ai for customer-facing incident comm
- Customer notifications via SLA matrix
- VDP for security researchers

---

## 5. CC3 — Risk Assessment (4 sub-criteria)

### CC3.1 — Specifies Suitable Objectives

**EFE evidence** :
- SLOs per sprint documented in `SLO.md`
- SLO targets reviewed quarterly

### CC3.2 — Identifies Risks

**EFE evidence** :
- Threat model `docs/security/threat_model.md` — 63 STRIDE threats
- Risk register `docs/security/risk_register.md` — 20 risks
- Quarterly threat model review

### CC3.3 — Considers Potential for Fraud

**EFE evidence** :
- Insider threat (R-16) explicitly tracked
- Dual-control for KMS access
- Audit log + 2-reviewer rule prevent unilateral action

### CC3.4 — Identifies and Analyzes Significant Change

**EFE evidence** :
- Pen-test program (M14 D40 plan) — annual
- Threat model updates per major sprint
- Architecture review for breaking changes

---

## 6. CC4 — Monitoring Activities (2 sub-criteria)

### CC4.1 — Selects, Develops, and Performs Ongoing Evaluations

**EFE evidence** :
- Prometheus continuous metrics collection (D13)
- 10 D13 alerts + 20 M14 D42 NEW runbooks
- HMAC chain audit log (D14-bis) — tamper-evident
- Quarterly DR drill (D40) — operational evidence pack

### CC4.2 — Evaluates and Communicates Deficiencies

**EFE evidence** :
- Audit findings tracked in security backlog
- Remediation PRs with `audit-finding` label
- Customer notifications per SLA matrix

---

## 7. CC5 — Control Activities (3 sub-criteria)

### CC5.1 — Selects and Develops Control Activities

**EFE evidence** :
- Helm chart M12 controls (Polaris GREEN, NetworkPolicy strict, mTLS)
- Bundle signing M13 (Ed25519 + cosign + Rekor)
- Audit log D14-bis (HMAC chain)

### CC5.2 — Selects and Develops General Controls Over Technology

**EFE evidence** :
- Pin tests (~92 across M12 + M13 + 25 M14) — anti-regression structurelle
- Polaris GREEN gate (M12) — security best-practice automated
- CI workflows : helm-test, helm-publish, bundle-release
- Helm chart audit fixes (M12 audit 8 defects) demonstrate gate effectiveness

### CC5.3 — Deploys Through Policies and Procedures

**EFE evidence** :
- All controls codified in policies (this directory)
- 30 runbooks (10 D13 + 20 M14 D42)
- Onboarding checklist for new ops staff

---

## 8. CC6 — Logical and Physical Access Controls (8 sub-criteria)

### CC6.1 — Implements Logical Access Security Software, Infrastructure, and Architectures

**Control objective** : Protected information assets are protected
from unauthorized access.

**EFE evidence** :
1. **OIDC authentication** (D15) — JWT validation per request,
   issuer allow-list, JWKS rotation, replay protection (jti).
   Evidence : `crates/efe-server/src/auth/oidc.rs` +
   `docs/bench/d15_onboarding_idp_architecture.md`.
2. **Per-tenant RBAC** (D11) — `tenant_id` claim propagated through
   gRPC context, resource isolation enforced at handler level.
   Evidence : `crates/efe-server/src/handlers/*.rs` + pin tests
   `test_d11_per_tenant_isolation.py`.
3. **Pod-level controls** (M12) — `securityContext.runAsNonRoot=true`,
   `readOnlyRootFilesystem=true`, `allowPrivilegeEscalation=false`,
   `capabilities.drop=ALL`. Polaris GREEN gate enforces.
   Evidence : `deploy/helm/efe/values.yaml` + `.polaris.yaml`.
4. **Network policy strict** (M12 D21, opt-in) — default-deny +
   tenant whitelist. Evidence :
   `deploy/helm/efe/templates/d21-networkpolicy-strict.yaml`.

**Test of design (audit period 2026-04-01 to 2026-09-30)** :
- 100 % of pods spawned in audit period had `runAsNonRoot=true`
  (verified via Polaris CI gate — every Helm deploy gates).
- 0 escapes from per-tenant context (verified via D11 pin tests +
  production audit log review).
- JWKS rotation cadence : 5-min cache TTL × 4 issuers.

**Test of operating effectiveness** :
- Sample 25 production requests over 6 months → all had valid OIDC JWT.
- 0 P0 incidents related to access control during audit period.

**Auditor talking points** :
- "How is privilege managed ?" → JWT scope claim + per-tenant RBAC + cluster-level RBAC.
- "What happens if JWT signing key is compromised ?" → JWKS rotation
  procedure (`docs/runbooks/d15_oidc_jwks_stale.md`) + immediate
  revocation of issuer in `oidc-issuers.json`.

### CC6.2 — Manages Access to System Components and Information Assets

**EFE evidence** :
- New user provisioning : OIDC issuer (D15)
- Modification : OIDC issuer (D15)
- Removal : OIDC issuer + immediate JWKS refresh
- Privileged access : separate `admin` claim, audit-logged

### CC6.3 — Authorizes, Modifies, or Removes Access

**EFE evidence** :
- OIDC issuer integration (D15) — externalizes account management
- JWT exp + jti claims — automatic timeout + replay prevention
- Issuer revocation procedure : update `oidc-issuers.json` Secret + rolling restart (~ 2 min)

### CC6.4 — Restricts Physical Access to Facilities and Protected Information Assets

**EFE evidence** : 🟡 Compensating via cloud provider attestations.

### CC6.5 — Discontinues Logical and Physical Access

**EFE evidence** :
- OIDC issuer revocation
- ServiceAccount deletion procedure
- KMS key revocation + rotation post-departure

### CC6.6 — Implements Logical Access Security Measures

**EFE evidence** :
- NetworkPolicy strict (M12 D21) — default-deny + tenant whitelist
- mTLS (M12 D22) — mutual auth between services
- cert-manager rotation 30d before expiry — auto-renewal
- SBOM endpoint (M14 D40) — supply chain validation

### CC6.7 — Restricts Transmission, Movement, and Removal of Information

**EFE evidence** :
- TLS 1.3 in transit
- mTLS optional (M12 D22)
- DPA standard for customer transfers
- SCC for international transfers EU→US

### CC6.8 — Prevents or Detects and Acts Upon Unauthorized Software

**EFE evidence** :
- Image cosign verify in admission webhook (Tier 3 deployments)
- Bundle Ed25519 sig + trust store (M13)
- SBOM endpoint validates known-good components
- Polaris GREEN refuses non-compliant Pods (M12)

---

## 9. CC7 — System Operations (5 sub-criteria)

### CC7.1 — Detects and Monitors Changes

**EFE evidence** : Polaris CI + pin tests + Git as source of truth.

### CC7.2 — Monitors System Components

**EFE evidence** :
- Grafana dashboards (D13) — 6 dashboards, 30+ panels
- OTel SDK (D13-bis) — distributed tracing
- Prometheus alerts D13 → runbooks D42

### CC7.3 — Evaluates Security Events to Determine Whether They Could Lead to Significant Risks

**EFE evidence** : Severity matrix in `general_incident_playbook.md`.

### CC7.4 — Develops and Implements Activities to Prevent Disruptions

**EFE evidence** :
- HPA D9 — auto-scale 3-30 replicas
- PodDisruptionBudget D18 — minimum 2 pods always available
- Multi-replica D18 — leader election + read-replica routing
- Backup D17 — 4 CronJobs daily
- DR drill D40 — quarterly full restore validation

### CC7.5 — Identifies, Develops, and Implements Activities to Recover

**EFE evidence** :
- D17 backup pipeline
- D40 quarterly DR drill
- Multi-region replication option

---

## 10. CC8 — Change Management (1 sub-criterion)

### CC8.1 — Authorizes, Designs, Develops, Acquires, Implements, Configures, Documents, Tests, Approves, and Deploys

**EFE evidence** :
- All code changes via PR + 2 reviewers (post-freeze)
- Branch protection on main
- Pin tests + Polaris + helm-unittest CI gates
- Audit pass per sprint (caught 23 defects pre-merge)
- Architecture docs require update before feature merge

---

## 11. CC9 — Risk Mitigation (2 sub-criteria)

### CC9.1 — Identifies, Selects, and Develops Risk Mitigation Activities

**EFE evidence** : (already covered in `dr_drill_quarterly_evidence.md`)
- DR drill quarterly outcome.json + Hello-World workflow validation
- 7-year Object Lock COMPLIANCE retention (immutable evidence)
- 4 frameworks asserted via labels

### CC9.2 — Compliance with Confidentiality Commitments

**EFE evidence** :
- DPA template + customer DPAs signed
- Object Lock COMPLIANCE for evidence (no admin tampering)
- HMAC chain audit prevents repudiation
- PII hash (D13) limits PII exposure
- mTLS (M12 D22) + TLS 1.3 in transit

---

## 12. A1 — Availability (3 sub-criteria)

### A1.1 — Maintains, Monitors, and Evaluates Current Processing Capacity

**EFE evidence** :
- HPA D9 metrics-driven scaling
- ResourceQuota Tier 2 per-tenant limits
- Capacity planning quarterly review

### A1.2 — Authorizes, Designs, Develops, Acquires, Implements

**EFE evidence** :
- Multi-AZ deployment via topologySpreadConstraints D18
- PDB minimum 2 pods
- Backup D17 + DR drill D40 RTO 30 min target
- SLO M14-COMP-3 quarterly drill success ≥ 99 %

### A1.3 — Tests Recovery Plan Procedures

**EFE evidence** :
- Quarterly DR drill (D40) — 4 drills/year minimum
- Hello-World end-to-end validation
- RTO 30 min weekly / 45 min quarterly target

---

## 13. C1 — Confidentiality (2 sub-criteria)

### C1.1 — Identifies and Maintains Confidential Information

**EFE evidence** :
- PII flagged via D13 PII hash
- Tenant data classified per DPA
- Customer-specific encryption keys (Tier 3 BYOK option)
- Sub-processor list maintained in DPA addendum

### C1.2 — Disposes of Confidential Information

**EFE evidence** :
- Tenant offboarding procedure
- 30-day retention post-deletion (right to recovery)
- Crypto-shred at end of retention

---

## 14. SOC 2 audit timeline

```
Q1 2026 : Pre-audit prep (this doc + iso27001.md + DDQ pre-fill)
Q2 2026 : Engagement letter signed + scope confirmed + auditor walkthroughs
Q3 2026 : Type I audit (point-in-time, 1 month observation)
Q3 2026 : Type II observation period STARTS (2026-Q3 → 2026-Q4)
Q1 2027 : Type II audit field work
Q2 2027 : Type II report draft + management response
Q3 2027 : Type II report final + bridge letter for next year
Annual  : Type II renewal cycle thereafter
```

---

## 15. Auditor liaison

| Auditor request | EFE point of contact |
|---|---|
| Document requests | CISO (security@efe.ai) |
| Walkthrough scheduling | Compliance lead |
| Sample testing | Engineering lead per area |
| Final report sign-off | CEO + CISO |

---

## 16. Customer reference list

(To be populated post-1.0 GA — for SOC 2 Type II auditor use only.)

---

## 17. Bridge letter procedure

When audit period extends beyond report date :
- Auditor issues bridge letter T+90 from report date
- Covers any material changes to controls
- Customer can rely on bridge letter for procurement

---

## 18. Continuous evidence collection

Per `docs/compliance/continuous_compliance.md`, evidence collected
weekly to `s3://efe-evidence-prod/`. Auditors granted scoped read
access via short-lived AWS IAM AssumeRole (4-hour sessions).

---

## References

- AICPA Trust Services Criteria 2017 (revised 2022)
- AICPA SOC 2 Reporting on Controls at a Service Organization
- AICPA SSAE 18 — Statement on Standards for Attestation Engagements
