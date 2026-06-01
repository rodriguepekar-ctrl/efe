# Data Processing Agreement (DPA) Template — EFE 1.0

> **Status** : Sprint M14 D41 (Φ-B) audit fix M14-A2 — DPA template
> per GDPR Art. 28.
> **Parties** : Controller (Customer) + Processor (EFE Project).
> **Last updated** : 2026-04-30.
> **Note** : This is a TEMPLATE. Customer-specific DPAs are
> negotiated per engagement ; legal counsel review required before
> signature.

---

## 1. Definitions
Per GDPR Art. 4. "Personal Data", "Processing", "Controller",
"Processor", "Data Subject", "Sub-processor", "Personal Data Breach".

## 2. Subject matter + duration
- **Subject** : Processing of Personal Data on behalf of Controller
  by Processor for the purpose of providing the EFE Service.
- **Duration** : The duration of the EFE Service Agreement.

## 3. Nature + purpose of processing
- **Nature** : Semantic classification of text via embedding-based
  cluster matching.
- **Purpose** : Customer-specific use cases (support routing, content
  moderation, etc.) per the Service Agreement.

## 4. Categories of Personal Data + Data Subjects
- Customer specifies in onboarding form
- EFE provides PII pseudonymization (`EFE_PII_HASH=true`) by default

## 5. Obligations of Processor (per Art. 28(3))

### (a) Process only on documented instructions
EFE processes Personal Data only on Controller's documented
instructions, including with regard to transfers, unless required
by law to process otherwise.

### (b) Personnel confidentiality
Persons authorized to process Personal Data are committed to
confidentiality.

### (c) Security measures (Art. 32)
EFE implements appropriate technical and organizational measures
documented in `docs/compliance/gdpr_art32.md` :
- TLS 1.3 in transit
- Cloud KMS encryption at rest
- mTLS optional
- Per-tenant RBAC + isolation
- HMAC chain audit log
- Multi-region backup + DR drill quarterly
- 30-day SLA for data subject rights

### (d) Sub-processors
Listed in `gdpr_ropa_template.md`. 30-day prior notice for changes.
Customer right to object within 30 days.

### (e) Assist Controller with Data Subject Rights
EFE provides export RPCs for data portability (Art. 20), erasure
procedures (Art. 17), and access procedures (Art. 15).

### (f) Assist Controller with Articles 32-36
Notification of breaches (Art. 33) within 24h to Controller's DPO.

### (g) Delete or return Personal Data after end of services
30-day grace period post-termination + crypto-shred.

### (h) Audit support
Customer audit right once per year, 30-day notice. Documentation
review + scoped data subject right inquiries.

## 6. Sub-processors

Current sub-processors :

| Sub-processor | Service | Region |
|---|---|---|
| AWS | Compute + storage | Customer-chosen |
| GCP | Compute + storage | Customer-chosen |
| Azure | Compute + storage | Customer-chosen |
| Auth0 | OIDC issuer (optional) | Customer-chosen |
| Datadog | Observability (Tier 3 optional) | Customer-chosen |

Notification of changes : 30 days prior. Customer can object within
30 days ; if no alternative, customer may terminate without penalty.

## 7. International transfers

For transfers to non-adequacy-recognized countries :
- SCC 2021/914 Module 2 (controller-to-processor) included as
  Schedule 2
- TIA conducted per `gdpr_tia_template.md`
- Customer-managed KMS available (Tier 3 BYOK)

## 8. Schedule 1 — Description of processing
- Categories of data subjects
- Categories of Personal Data
- Sensitive categories (Art. 9, if any)
- Frequency of processing
- Retention schedule

## 9. Schedule 2 — SCC (EU 2021/914)
[Standard Contractual Clauses, Module 2]

## 10. Schedule 3 — Sub-processors
[Per § 6 above]

## 11. Schedule 4 — Technical and Organizational Measures
[Per `gdpr_art32.md`]

---

## Sign-off

| Role | Controller (Customer) | Processor (EFE Project) |
|---|---|---|
| Signatory name | `<name>` | `<name>` |
| Title | `<title>` | `<title>` |
| Date | `<date>` | `<date>` |
| Signature | `<signature>` | `<signature>` |

---

## Reference

- GDPR Art. 28 (Processor obligations)
- EU Standard Contractual Clauses 2021/914
- ICO Article 28 controller-processor contracts guidance
