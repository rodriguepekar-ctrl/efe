# Records of Processing Activities (RoPA) Template

> **Status** : Sprint M14 D41 (Φ-B) audit fix M14-A2 — RoPA template
> per GDPR Art. 30.
> **Audience** : Customer DPOs, supervisory authorities on request.
> **Maintained by** : Controller (typically customer) ; EFE provides
> as processor.
> **Last updated** : 2026-04-30.

---

## RoPA — `<Customer Tenant Name>`

### As Controller (Art. 30(1))

| Field | Value |
|---|---|
| Controller name + contact | `<Customer entity>` |
| DPO name + contact | `<DPO email>` |
| Categories of data subjects | `<list>` |
| Categories of personal data | `<list>` |
| Categories of recipients | EFE Project (processor) + sub-processors per DPA |
| International transfers (recipients in 3rd countries) | `<list + safeguards>` |
| Retention schedule | `<per data class>` |
| Security measures | TLS 1.3, OIDC auth, per-tenant RBAC, audit log, backup |

### As Processor — recorded by EFE (Art. 30(2))

| Field | Value |
|---|---|
| Processor name + contact | EFE Project ; dpo@efe.ai |
| Each controller on whose behalf processing | `<customer list>` |
| Categories of processing | Semantic classification, embeddings |
| International transfers | `<as configured per customer region>` |
| Security measures | Per `gdpr_art32.md` |

---

## Sub-processor inventory

| Sub-processor | Service | Region | DPA | Notice |
|---|---|---|:---:|---|
| AWS | Compute + storage | Customer-chosen | ✅ | 30d prior |
| GCP | Compute + storage | Customer-chosen | ✅ | 30d prior |
| Azure | Compute + storage | Customer-chosen | ✅ | 30d prior |
| Auth0 | OIDC issuer (optional) | Customer-chosen | ✅ | 30d prior |
| Datadog | Observability (Tier 3) | Customer-chosen | ✅ | 30d prior |

---

## Reference
- GDPR Art. 30 (Records of processing activities)
- WP29 Guidelines on RoPA
- CNIL RoPA template
