# Continuous Compliance — EFE 1.0 GA

> **Status** : Sprint M14 D42 (Φ-C) — continuous compliance program.
> **Audience** : CISO, DPO, audit committee, customers.
> **Cadence** : monthly + quarterly + annual cycles.
> **Last updated** : 2026-04-30.

---

## 1. Mission

Maintain compliance posture between formal audits via continuous
evidence collection, monitoring, and review.

---

## 2. Cadence overview

```
Daily      : SBOM index regeneration + CDN invalidation
Weekly     : Evidence pack collection (CI workflow)
Monthly    : KPI review by security team
Quarterly  : Compliance review by CISO + DPO + Eng VP
             - Threat model re-evaluation
             - Risk register update
             - DR drill outcome.json review
             - Pen-test findings status
             - SoA review
Annual     : External audits (SOC 2 + ISO 27001 surveillance)
             - Pen-test report
             - DPO compliance attestation
             - SBOM endpoint health review
3-year     : ISO 27001 recertification
```

---

## 3. Weekly evidence collection

CI workflow `.github/workflows/evidence-collection.yml` runs every
Monday 04:00 UTC, writing to `s3://efe-evidence-prod/<DATE>/`.

Outputs collected :
- Pin tests JUnit XML
- Polaris audit JSON (3 profiles)
- SBOM index snapshot
- SLO metrics (Prometheus query)
- Audit log integrity check (sample 100 random rows, HMAC validate)
- Helm chart Polaris GREEN status
- Cosign signature verifications

Storage : Object Lock COMPLIANCE 7-year retention.

---

## 4. Monthly KPI review

| Metric | Target | Source | Owner |
|---|---|---|---|
| Pin tests GREEN rate | 100 % | CI | Eng lead |
| Polaris GREEN rate | 100 % | CI | DevOps lead |
| SBOM endpoint uptime | ≥ 99.9 % | CDN metrics | DevOps lead |
| Audit log HMAC integrity | 100 % | Weekly check | Security lead |
| DR drill quarterly success | ≥ 99 % within RTO | D40 outcome.json | DevOps lead |
| Pen-test critical findings | 0 open > 7d | Security tracker | Security lead |
| Customer DDQ response time | < 5 business days | Sales tracker | Sales eng |
| GDPR DSR response time | < 30 days | DPO log | DPO |
| Sub-processor list current | 100 % | DPO review | DPO |
| Vulnerability CVE patching | < 30 days for High+ | Dependabot tracker | Security lead |

---

## 5. Quarterly compliance review

### 5.1 Inputs reviewed

- DR drill outcome (per `dr_drill_quarterly_evidence.md`)
- Threat model updates (additions, removals, residual changes)
- SOC 2 evidence collection (continuous)
- GDPR data subject requests log
- Pen-test findings status
- SBOM index health (size / age / coverage)
- Risk register (new risks, closed risks, re-rated risks)
- Customer compliance complaints
- Sub-processor changes
- Regulatory updates (AI Act implementing acts, NIS2 transposition, etc.)

### 5.2 Outputs

- **Quarterly compliance report** (signed by CISO) — published internally
- **Updated SoA** if controls drift
- **Risk register update**
- **Threat model update** if new threats identified
- **Customer notification** if any change material

### 5.3 Sign-off chain

```
Engineering ↦ CISO ↦ DPO ↦ Eng VP ↦ CTO ↦ CEO
                      └──→ Board (quarterly summary)
```

---

## 6. Annual SOC 2 Type II audit

```
Q1     : Engagement letter + scope letter + auditor selected
Q2     : Audit kickoff + walkthroughs (5-10 sessions)
Q3     : Field work + sample testing (auditor visit + remote)
Q4     : Report draft + management response
Q1+1   : Final report + bridge letter to next year
Q1+1   : Public availability for customers (under NDA)
```

---

## 7. Annual ISO 27001 surveillance

```
Year 1 (2026)  : Stage 1 + Stage 2 audit (initial certification)
Year 2 (2027)  : Surveillance audit
Year 3 (2028)  : Surveillance audit
Year 4 (2029)  : Recertification audit (3-year cycle)
```

---

## 8. Compliance metrics dashboard

Grafana dashboard `Compliance Health` (post-D42 plan) :
- Real-time SoA coverage % (ISO 27001 + NIST + SOC 2)
- Open audit findings by severity
- Open pen-test findings by severity
- Days since last DR drill
- Days since last pen-test
- Days since last threat model update
- Days since last risk register update

---

## 9. Auditor read-access provisioning

External auditors (SOC 2, ISO 27001, FedRAMP 3PAO) receive scoped
read-access via short-lived AWS IAM AssumeRole sessions :

```bash
# Issued by CISO at audit kickoff
aws sts assume-role \
    --role-arn arn:aws:iam::<ACCT>:role/AuditorReadOnly \
    --role-session-name auditor-soc2-2026-q3 \
    --duration-seconds 14400  # 4-hour session
```

Auditor role permissions :
- `s3:GetObject` on `efe-evidence-prod/*`
- `s3:GetObject` on `efe-sbom-prod/*`
- No write/delete anywhere
- CloudTrail logged + DPO notified per session

---

## 10. Customer audit right (Art. 28 GDPR)

Customers may request audit once per year, with reasonable notice
(30 days). EFE provides :
- SOC 2 Type II report (under NDA)
- ISO 27001 certificate + SoA
- Pen-test report (sanitized)
- DPA + sub-processor list
- Read access to relevant evidence pack (4-hour session)

Customer audit rights are limited to documentation review + scoped
data subject right inquiries — no direct access to other customers'
data.

---

## 11. Continuous improvement loop

```
Detect (monitoring + alerts)
   ↓
Triage (severity classification)
   ↓
Mitigate (runbook execution)
   ↓
Document (audit log + post-mortem)
   ↓
Learn (root cause + pin test added)
   ↓
Update (compliance docs + threat model + risk register)
```

Per ISO 27001 Clause 10 (Improvement) and SOC 2 CC4.

---

## 12. Compliance roadmap

| Quarter | Focus |
|---|---|
| 2026-Q3 | First SOC 2 Type I + ISO 27001 Stage 1 |
| 2026-Q4 | SOC 2 Type II observation period START + ISO Stage 2 |
| 2027-Q1 | First pen-test + SOC 2 Type II audit field work |
| 2027-Q2 | SOC 2 Type II report + bug bounty public launch |
| 2027-Q3 | First annual surveillance |
| 2027-Q4 | AI Act conformity assessment (post-effective-date) |
| 2028-Q1 | Customer DDQ portal launch |
| 2028-Q4 | NIS2 critical entity self-assessment |
| 2029-Q4 | ISO 27001 recertification |

---

## 13. Communication

- **Customer status** : status.efe.ai
- **Internal** : `#compliance` Slack channel
- **External** : compliance@efe.ai
- **DPA inquiries** : dpo@efe.ai
- **Security inquiries** : security@efe.ai

---

## References

- ISO/IEC 27001:2022 Clause 10 (Improvement)
- AICPA SOC 2 Common Criteria CC4 (Monitoring Activities)
- NIST SP 800-137 (Continuous Monitoring)
- NIST AI RMF 1.0 (AI Risk Management Framework)
