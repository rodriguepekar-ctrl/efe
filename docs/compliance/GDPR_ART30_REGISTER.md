# GDPR Article 30 — Record of Processing Activities

**Controller** : the EFE customer (data controller).
**Processor** : EFE-AI Inc. (the EFE service).
**Tenant** : `{{tenant_name}}`
**Generated** : `{{generated_at}}`
**Master template version** : v1.0.0 (release v1.0.0 GA)

> Article 30 GDPR (Regulation EU 2016/679) requires both controllers
> and processors to maintain a record of processing activities under
> their responsibility. This file is the EFE-side processor record for
> tenant `{{tenant_name}}` and is suitable for inclusion in the
> tenant's own controller record.

---

## 1. Identity and contact details

| Role           | Entity                                | Contact                          |
|----------------|---------------------------------------|----------------------------------|
| Processor      | EFE-AI Inc.                           | privacy@efe.example.com          |
| Representative (EU) | EFE-AI EU SAS, Paris (FR)        | eu-rep@efe.example.com           |
| Data Protection Officer | DPO, EFE-AI Inc.             | dpo@efe.example.com              |
| Controller     | `{{tenant_name}}`                     | (provided by tenant)             |

## 2. Categories of processing

EFE processes the following data categories on behalf of the controller :

| Category                        | Examples                                              | Sensitivity |
|---------------------------------|-------------------------------------------------------|-------------|
| Tenant identifiers              | tenant ID, organization name                          | Low         |
| User identifiers                | user UUID, role assignments                           | Low         |
| Authentication tokens           | OIDC access tokens (encrypted at rest)                | Medium      |
| Inference inputs                | text submitted to classification API                  | Variable    |
| Inference outputs               | classification labels, confidences                    | Low         |
| Audit logs                      | API call records (HMAC-chained)                       | Low         |
| Operational telemetry           | latency, error rates, capacity                        | Low         |
| Compliance evidence             | attestation status, audit chain roots                 | Low         |

EFE **does not** process special-category data (Art. 9) unless the
controller explicitly opts in via a dedicated subscription with
additional safeguards.

## 3. Categories of data subjects

- End-users of the controller's services whose data is submitted for
  classification.
- Employees of the controller who administer EFE on the controller's
  behalf (admin portal users).

## 4. Purposes of processing

- Provide the EFE classification API as agreed in the Master Services
  Agreement.
- Provide audit trails to meet the controller's regulatory obligations.
- Provide observability and incident response capabilities.
- Provide compliance evidence packages.

EFE does **not** use tenant data for :
- Training general-purpose models.
- Marketing or sales.
- Sub-processor advertising or profiling.

## 5. Categories of recipients

| Recipient                | Purpose                                          | Region              |
|--------------------------|--------------------------------------------------|---------------------|
| EFE engineers (need-to-know) | Incident response, data subject requests       | EU, US              |
| AWS (sub-processor)      | Cloud hosting (compute, storage, KMS)            | Configurable per tenant |
| Mailgun (sub-processor)  | Notification emails (no user content)            | EU                  |
| Sigstore                 | Signature transparency (no user data)            | Public              |
| Datadog (optional)       | APM telemetry (no user content)                  | EU                  |

The full sub-processor list is at `docs/compliance/dpa_template.md` and
in the tenant's signed DPA.

## 6. Transfers to third countries

For tenants on EU-resident clusters :

- Data **stays in the EU** (AWS Frankfurt or Paris by default).
- No transfer outside the EU/EEA except for incident response, in which
  case data is reviewed in-place by trained personnel.

For tenants on US-resident clusters :

- Standard Contractual Clauses (SCCs) apply between EFE-AI EU SAS and
  EFE-AI Inc.
- Transfer Impact Assessment (TIA) document available on request.
- No onward transfer to third countries.

## 7. Retention periods

| Data category            | Default retention      | Configurable |
|--------------------------|------------------------|--------------|
| Inference inputs         | 0 days (ephemeral)     | Yes (up to 365 days) |
| Inference outputs        | Per audit log retention | Yes |
| Audit logs               | 7 years (immutable)    | No (regulatory minimum) |
| Operational telemetry    | 90 days                | Yes |
| Backup snapshots         | 30 days rolling        | Yes |
| Compliance evidence      | 7 years                | No (audit requirement) |

Tenants can configure tighter retention via the portal-admin →
Settings → Privacy → Retention.

## 8. Technical and organizational measures (Art. 32)

| Measure                              | Implementation                                       |
|--------------------------------------|------------------------------------------------------|
| Pseudonymization                     | User UUIDs, no plaintext PII in audit logs           |
| Encryption at rest                   | AES-256-GCM via per-tenant KMS keys                  |
| Encryption in transit                | TLS 1.3 mandatory, mTLS for inter-service            |
| Access controls                      | RBAC + per-tenant KMS context                        |
| Logging and monitoring               | All access audit-logged with HMAC chain integrity    |
| Confidentiality, integrity, availability | SOC 2 controls (see `SOC2_EVIDENCE.md`)         |
| Resilience and recovery              | Quarterly DR drills, multi-region failover           |
| Regular testing                      | Penetration tests annually, internal reviews quarterly |

Detailed measures : `docs/compliance/gdpr_art32.md`.

## 9. Data subject rights implementation

| Right (Article)                              | Implementation                                       |
|----------------------------------------------|------------------------------------------------------|
| Information (Art. 13–14)                     | Privacy notice templates ; DPA Annex                 |
| Access (Art. 15)                             | `efe-admin tenant export <tenant> --user <uuid>`     |
| Rectification (Art. 16)                      | Portal-admin → Users → Edit                          |
| Erasure (Art. 17)                            | `efe-admin tenant erase-user <tenant> <uuid>`        |
| Restriction (Art. 18)                        | Portal-admin → Users → Restrict                      |
| Portability (Art. 20)                        | `efe-admin tenant export --format json`              |
| Objection (Art. 21)                          | Opt-out flag in user record                          |
| Automated decision-making (Art. 22)          | Human-oversight queue (AI Act Art. 14 hooks)         |

Erasure includes :
- Soft-tombstone of audit log rows (HMAC chain remains intact).
- Hard-delete of inference inputs (if retention > 0).
- KMS key rotation on tenant-wide erasure.

## 10. Breach notification

In the event of a personal data breach :

1. EFE notifies the controller within **24 hours** of confirmed
   discovery (faster than the 72-hour Art. 33 requirement).
2. Notification format : `docs/compliance/breach_notification_template.md`.
3. Severity-1 breaches trigger immediate live-call escalation.

---

## Generated for tenant `{{tenant_name}}` on `{{generated_at}}`.
