# Quarterly DR Drill — Compliance Evidence Mapping (D40)

> **Status** : Sprint D40 (Φ-F) — formal compliance evidence
> document for the EFE quarterly DR drill.
> **Schema versions** :
>   * `dr-drill-outcome-2.0.0` ([schema](dr-drill-outcome-2.0.0.schema.json))
>   * `helloworld-1.0.0` ([schema](helloworld-1.0.0.schema.json))
>
> **Audience** : SOC 2 / ISO 27001 / NIS2 / RGPD external auditors,
> internal DPO, internal compliance officer, security ops team.
>
> **Frameworks covered** :
>   * SOC 2 Trust Services Criteria — **CC9.1** Risk Mitigation +
>     **CC9.2** Confidentiality Commitments + **A1.2** Availability.
>   * ISO 27001:2022 Annex A — **A.17.1.3** Verify continuity +
>     **A.17.2.1** Availability + **A.5.30** ICT Readiness.
>   * NIS2 Directive (EU 2022/2555) — **Art. 21.2(c)** Business
>     continuity, backup management, disaster recovery, crisis
>     management.
>   * RGPD / GDPR — **Art. 32.1(c)** Restoration capability +
>     **Art. 32.1(d)** Regular testing procedure.

---

## 1. Scope

This document binds each field of the quarterly DR drill outcome
artefact (`outcome.json` schema v2) to one or more compliance
control requirements, so an external auditor can validate that
every required control has documented evidence.

The pack lives in `.ci-evidence/dr-drill-quarterly/<YYYY>-Q<N>/`
(per-quarter immutable archive — see
[`.ci-evidence/dr-drill-quarterly/README.md`](../../.ci-evidence/dr-drill-quarterly/README.md)).

## 2. Compliance frameworks reference

### 2.1 SOC 2 Trust Services Criteria

| Criterion | Title | EFE control evidence |
|-----------|-------|----------------------|
| **CC9.1** | *Risk Mitigation* — "Identifies, selects, and develops risk mitigation activities for risks arising from potential business disruptions." | The quarterly drill is the **operational evidence** that the disaster-recovery plan is exercisable. Each `outcome.json` documents (a) the drill happened, (b) the recovery time, (c) the validation suite. Auditor reads `success_overall == true` + `within_rto == true` + every `verifiers[].ok == true`. |
| **CC9.2** | *Compliance with Confidentiality Commitments* | The evidence pack lives behind S3 Object Lock in `COMPLIANCE` mode for 7 years — proof that the auditor sees the **same** evidence the system did when it was written, with no possibility of post-hoc tampering by an admin. |
| **A1.2** | *Availability* | The drill measures the recovery time objective (RTO) via `duration_secs` ≤ `rto_target_secs`. Drives SLO `D40-DR-1`. Trend reports per quarter demonstrate availability commitments are testable + tested. |

### 2.2 ISO 27001:2022 Annex A

| Control | Title | EFE control evidence |
|---------|-------|----------------------|
| **A.17.1.3** | *Verify, review and evaluate information security continuity* | The quarterly drill **is** the verification activity. Documented runbook (`dr/run_quarterly_drill.sh`) + automated execution via Helm CronJob + immutable evidence pack ≡ quarterly audit trail. |
| **A.17.2.1** | *Availability of information processing facilities* | Hello-World workflow validates the **business path** (not just binaries). Auditor reads `helloworld.all_steps_ok == true` + `execute.verdict ∈ {CONVERGED, CONTINUE}`. |
| **A.5.30** | *ICT Readiness for business continuity* | Quarterly cadence + automated execution + 7-year retention = continuous readiness signal. Failed drills produce evidence too (success OR failure, both archived). |

### 2.3 NIS2 Directive (EU 2022/2555)

| Article | Subject | EFE control evidence |
|---------|---------|----------------------|
| **Art. 21.2(c)** | *Business continuity, such as backup management and disaster recovery, and crisis management* | NIS2 mandates entities essential and important have backup + DR procedures **documented and tested regularly**. The D40 quarterly drill provides the formal testing evidence ; per-quarter pack provides the documentation. |

### 2.4 RGPD / GDPR (EU 2016/679)

| Article | Subject | EFE control evidence |
|---------|---------|----------------------|
| **Art. 32.1(c)** | *Capacité de rétablir la disponibilité des données à caractère personnel et l'accès à celles-ci dans des délais appropriés en cas d'incident physique ou technique* | RTO 45 min + Hello-World workflow validation = formal capability evidence. |
| **Art. 32.1(d)** | *Procédure visant à tester, à analyser et à évaluer régulièrement l'efficacité des mesures techniques et organisationnelles* | Quarterly cadence (4× per year minimum) + 7-year archive = "régulièrement" satisfied. |

---

## 3. Field-to-control mapping (outcome.json v2)

The table below maps each field of the canonical `outcome.json`
(schema v2) to the controls it demonstrates evidence for.

| Field | Type | Frameworks | Control evidence demonstrated |
|-------|------|------------|-------------------------------|
| `schema_version` | string (1.0.0) | all | Schema version pinning ; auditor verifies schema integrity (anti-version-drift). |
| `drill_kind` | enum | all | "quarterly" demonstrates compliance cadence (vs ad-hoc weekly drills). |
| `drill_label` | string | all | Human-readable identifier ; cross-references the audit trail. |
| `started_at` / `finished_at` | ISO-8601 | SOC2-A1.2, ISO-A.17.1.3, NIS2 | Timestamping the drill provides the **when** evidence. |
| `duration_secs` | integer | SOC2-A1.2, RGPD-32.1c | RTO measurement (≡ recovery time evidence). |
| `rto_target_secs` | integer | SOC2-A1.2, RGPD-32.1c | RTO **target** declared in advance ; immutable. |
| `within_rto` | boolean | SOC2-A1.2, RGPD-32.1c | Drives SLO `D40-DR-1`. Auditor checks distribution over 4 quarters. |
| `phases.*` | integers | ISO-A.5.30 | Per-phase breakdown shows readiness, not just completion. |
| `verifiers[*].kind` | enum | SOC2-CC9.1, ISO-A.17.1.3 | Audit chain, bundle sigs, OIDC, catalog smoke, execute smoke = 5 essential post-restore validations. |
| `verifiers[*].ok` | boolean | SOC2-CC9.1, ISO-A.17.1.3 | `false` triggers the post-mortem flow ; auditor checks **all** `ok == true`. |
| `helloworld.*` | object | ISO-A.17.2.1, RGPD-32.1c | Business path validation (not just binaries up). Critical evidence the **service is genuinely usable** post-restore. |
| `helloworld.all_steps_ok` | boolean | ISO-A.17.2.1 | Top-line Hello-World assertion. |
| `helloworld.execute.verdict` | enum | ISO-A.17.2.1 | Verdict ∈ {CONVERGED, CONTINUE} = healthy swarm. |
| `helloworld.elapsed_secs` | integer | SOC2-A1.2, RGPD-32.1c | Hello-World latency ≡ SLO `D40-DR-2` (≤ 60 s). |
| `helloworld.hmac_chain_post_execute_ok` | boolean | RGPD-32.1c | Post-restore audit chain integrity (RGPD breach risk if false). |
| `compliance_frameworks` | array | all | Self-asserted frameworks the drill claims to satisfy. Auditor cross-checks with this document. |
| `attestation_signature` | base64 | all | 4-eyes Ed25519 signature ; binds the outcome to a verified-identity attestor pair. |
| `attestation_pubkey_id` | string | all | Public key identifier in `EFE_ATTESTATION_TRUST_STORE_PATH` for signature verification. |
| `success_overall` | boolean | all | Top-line drill outcome ; SLO `D40-DR-3` is computed as the rolling 4-quarter average of this field. |

---

## 4. SLO contract reference

| SLO | Target | Evidence field | Frameworks |
|-----|--------|----------------|------------|
| `D40-DR-1` | Quarterly drill RTO ≤ 45 min | `within_rto == true` | SOC2-CC9.1, ISO-A.17.1.3, NIS2-Art21, RGPD-32.1c |
| `D40-DR-2` | Hello-World latency ≤ 60 s | `helloworld.elapsed_secs <= 60` | ISO-A.17.2.1, NIS2-Art21 |
| `D40-DR-3` | Drill success rate ≥ 95 % over 4 last quarters | rolling avg of `success_overall` | SOC2-CC9.1, ISO-A.17.1.3, NIS2-Art21, RGPD-32.1c, RGPD-32.1d |
| `D40-DR-4` | Drill freshness ≤ 90 days since last `success_overall == true` | latest `finished_at` within 90 d | SOC2-CC9.1, ISO-A.17.1.3, NIS2-Art21, RGPD-32.1d |

---

## 5. External auditor walkthrough

### Step 1 — Inventory the evidence pack

```bash
# Auditor lists each quarter in the audit window.
$ aws s3 ls s3://efe-compliance-evidence/dr-drill-quarterly/ --recursive
2026-04-01 04:00:00  drill-2025-Q4-20260101T040000Z/outcome.json
2026-04-01 04:00:00  drill-2025-Q4-20260101T040000Z/attestation.txt
2026-04-01 04:00:00  drill-2026-Q1-20260401T040000Z/outcome.json
...
```

### Step 2 — Verify Object Lock + retention

```bash
$ aws s3api get-object-lock-configuration --bucket efe-compliance-evidence
{
  "ObjectLockConfiguration": {
    "ObjectLockEnabled": "Enabled",
    "Rule": {
      "DefaultRetention": {
        "Mode": "COMPLIANCE",
        "Years": 7
      }
    }
  }
}
```

### Step 3 — Validate each `outcome.json` against the schema

```bash
$ python3 docs/compliance/validate_outcome.py \
    --schema docs/compliance/dr-drill-outcome-2.0.0.schema.json \
    --instance .ci-evidence/dr-drill-quarterly/2026-Q1/outcome.json
OK : outcome conforms to schema v2.0.0
```

### Step 4 — Verify the 4-eyes attestation

```bash
$ python3 docs/compliance/verify_attestation.py \
    --instance .ci-evidence/dr-drill-quarterly/2026-Q1/outcome.json
OK : attestation valid (signed by drill-attestation-2026)
```

### Step 5 — Compute SLOs over the audit window

```bash
$ python3 docs/compliance/slo_summary.py \
    --window-days 365 \
    --evidence-dir .ci-evidence/dr-drill-quarterly/

D40-DR-1 (RTO ≤ 45min) : 4/4 PASS (100.0%)
D40-DR-2 (HelloWorld ≤ 60s) : 4/4 PASS (100.0%)
D40-DR-3 (Success rate ≥ 95% rolling 4Q) : PASS (100.0%)
D40-DR-4 (Freshness ≤ 90d) : PASS (last drill 47 days ago)
```

### Step 6 — Cross-reference per-control

The auditor reads §3 of this document, picks each control of
interest, and verifies the corresponding outcome.json field has
the expected value across the 4 quarters of the audit window.

---

## 6. Failed-drill handling

A **failed drill** still produces an evidence pack — auditors
need the failure record as much as the success record (a missing
quarter is a worse signal than a failed quarter, because it
suggests the test wasn't even run).

When `success_overall == false` :

* The drill `outcome.json` has the failure documented (which
  verifier failed, or `helloworld.all_steps_ok == false`).
* A **post-mortem** is required within 7 days, archived alongside
  the outcome at `<YYYY>-Q<N>/post-mortem.md`.
* A **remediation drill** is scheduled within 14 days ; its
  outcome is archived at `<YYYY>-Q<N>-remediation/` (NOT
  replacing the original failure record).
* PagerDuty alert `D40DrillQuarterlyFailed` fires immediately ;
  on-call ops responds within 4 h.
* DPO is notified within 24 h (RGPD breach-risk assessment).
* If 96 h pass without a successful remediation drill, the SOC 2
  auditor receives a heads-up notification and the issue is
  documented in the next audit cycle's incident report.

---

## 7. Document maintenance

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0.0 | 2026-04-29 | rodrigue.pekar | Initial release with sprint D40. |

This document is referenced by the pin test
`scripts/tests/test_dr_drill_runbook.py::test_compliance_doc_lists_frameworks`
which fails CI if this document drifts away from the canonical
4-framework mapping.

---

## 8. References

* [`docs/EFE_BACKUP_EXTENSION_ARCHITECTURE.md`](../EFE_BACKUP_EXTENSION_ARCHITECTURE.md)
  — sprint D40 architecture document (1 970 LOC, §17 compliance mapping).
* [`docs/bench/d17_backup_dr_architecture.md`](../bench/d17_backup_dr_architecture.md)
  — D17 baseline architecture (1 875 LOC).
* [`docs/audits/d17_backup_dr_audit_report.md`](../audits/d17_backup_dr_audit_report.md)
  — D17 audit report (10 fixes shipped pre-D40).
* [`docs/docs/howto/backup-and-restore.md`](../docs/howto/backup-and-restore.md)
  — operator runbook (extended with D40 quarterly section).
* [`.ci-evidence/dr-drill-quarterly/README.md`](../../.ci-evidence/dr-drill-quarterly/README.md)
  — evidence pack layout reference.
* [`dr/run_quarterly_drill.sh`](../../dr/run_quarterly_drill.sh)
  — the script that emits this evidence.
* [`deploy/helm/efe/templates/d40-dr-drill-quarterly-cronjob.yaml`](../../deploy/helm/efe/templates/d40-dr-drill-quarterly-cronjob.yaml)
  — production CronJob schedule.
