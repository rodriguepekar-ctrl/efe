# Certificate of Completion — GDPR Art. 17 Right to Erasure

**To** : `{customer.name}` (`{customer.tenant_slug}`)
**From** : EFE AI — Data Protection Officer
**Date issued** : `{issued_at}`
**Reference** : `EFE-RGPD-17-{erasure_request_id}`

---

## 1. Statement

In accordance with Article 17 of Regulation (EU) 2016/679 (the "GDPR"),
EFE AI hereby certifies the **complete erasure** of personal data
associated with the tenant identifier `{customer.tenant_slug}` from
the EFE production systems, as per the request initiated on
`{requested_at}` by `{requested_by}` and verified by Multi-Factor
Authentication on `{mfa_verified_at}`.

## 2. Scope of erasure

The cascade encompassed every database table containing tenant-scoped
data : agent runs, classify call logs, audit chain rows (pseudonymised
per § 4 below), webhook subscriptions, batch jobs, BYOK key material,
event log mirror, sub-processor acknowledgements, and S3-sink
configurations.

A schema-traversal verification was executed by SRE on
`{verification_run_at}` using `scripts/m29/verify_erasure_complete.sql`
and returned the verdict :

> `PASS : erasure complete`

## 3. Records of processing retained

The following records are retained under our legal obligations
(article 17(3)(b) and (e)) :

- **Audit chain entries** — preserved with all identifying fields
  replaced by SHA-256 hashes (pseudonymisation per Art. 4(5)). The
  HMAC chain integrity is preserved.
- **Billing records** — fiscal retention period (varies by
  jurisdiction, typically 10 years).

The audit chain entries cannot be re-associated with the natural
person they originally pertained to without additional information
which EFE no longer holds.

## 4. Final audit chain export

A signed, immutable Parquet export of the (pseudonymised) audit chain
through `{erasure_started_at}` was pushed to your registered S3 sink
at :

- **URL** : `{final_audit_url}`
- **SHA-256** : `{audit_export_sha256}`
- **HMAC chain anchor** : `{final_hmac_curr}` (verifiable against the
  per-tenant HMAC key delivered at sink registration)

## 5. Numbers

| Metric | Value |
|---|---:|
| Tables in cascade | `{tables_count}` |
| Rows deleted | `{rows_deleted}` |
| Rows pseudonymised | `{rows_pseudonymised}` |
| Cascade wall-clock | `{cascade_duration}` |
| Grace window respected | `{grace_window}` |

## 6. Right to lodge a complaint

If you consider that the erasure was incomplete or that EFE has not
met its obligations, you have the right to lodge a complaint with your
local supervisory authority. In France : Commission Nationale de
l'Informatique et des Libertés (CNIL — https://www.cnil.fr).

## 7. Contact

For any clarification : `dpo@efe.ai`.

---

**Signed** : `/s/ {dpo_name}`
**Title** : Data Protection Officer, EFE AI
**Date** : `{issued_at}`

This certificate is **digitally signed** with EFE's DPO PGP key
(fingerprint `{dpo_pgp_fingerprint}`). The signature file is attached
as `EFE-RGPD-17-{erasure_request_id}.sig`.
