# CCPA + CPRA — EFE controls mapping

> **Sprint** : Φ-D-COMPLIANCE D86
> **Audience** : California residents, California Privacy Protection Agency (CPPA)
> **Region** : US — California
> **Statutes** : Cal. Civ. Code § 1798.100-1798.199 (CCPA + CPRA)
> **Last updated** : 2026-04-15
> **Next review** : 2027-04-15

---

## 1. Statement of Applicability — Summary

| CCPA right | # Controls | Implemented |
|---|---:|---:|
| Right to know | 2 | 2 |
| Right to delete | 1 | 1 |
| Right to opt-out (sale/sharing) | 2 | 2 |
| Right to correct | 1 | 1 |
| Right to limit use of sensitive PI | 1 | 1 |
| Right to non-discrimination | 1 | 1 |
| **Total** | **8** | **8** |

> **Note** : CCPA + CPRA rights overlap significantly with GDPR (Art.
> 13/15/17/20/22). The 8 controls below map to GDPR controls in the
> `gdpr.md` file and re-use the same evidence scripts.

---

## 2. Controls catalog (8 controls)

### CCPA-1798.100 — Right to Know (Categories)
- **Description** : Consumer can request the categories of PI collected.
- **EFE artefacts** : ROPA per tenant + `transparency_notice` per catalog
- **Evidence script** : `comp_ropa_freshness_evidence` + `comp_transparency_notice_evidence`
- **Cross-ref GDPR** : Art. 13/14

### CCPA-1798.110 — Right to Know (Specific Pieces)
- **Description** : Consumer can request specific pieces of PI.
- **EFE artefacts** : Portability export (= JSON ZIP, M29 § 8)
- **Evidence script** : `comp_portability_30d_evidence`
- **Cross-ref GDPR** : Art. 15 + Art. 20

### CCPA-1798.105 — Right to Delete
- **Description** : Consumer can request deletion of PI.
- **EFE artefacts** : Erasure cascade (M29 § 7)
- **Evidence script** : `comp_erasure_30d_evidence`
- **Cross-ref GDPR** : Art. 17

### CCPA-1798.120 — Right to Opt-Out of Sale or Sharing
- **Description** : Consumer can opt-out of sale/share of PI.
- **EFE artefacts** : 4-axis consent (= `third_party_sharing` axis)
- **Evidence script** : `comp_consent_granularity_evidence`
- **Cross-ref GDPR** : Art. 7

### CCPA-1798.121 — Right to Limit Use/Disclosure of Sensitive PI
- **Description** : Consumer can limit use of sensitive PI.
- **EFE artefacts** : Sensitive-data filter at ingestion + audit
- **Evidence script** : `comp_consent_granularity_evidence`
- **Cross-ref GDPR** : Art. 9 (special categories)

### CCPA-1798.106 — Right to Correct
- **Description** : Consumer can request correction of inaccurate PI.
- **EFE artefacts** : EditAndConfirmGoldLabel (AL) + admin correction RPC (M29)
- **Evidence script** : `comp_evidence_aggregator` (cross-ref AL)
- **Cross-ref GDPR** : Art. 16

### CCPA-1798.125 — Right to Non-Discrimination
- **Description** : No service degradation for exercising privacy rights.
- **EFE artefacts** : Documented in `docs/compliance/dpa_template.md`
- **Evidence script** : N/A (= policy)

### CCPA-1798.130.a.5 — Notice at Collection
- **Description** : Inform consumer at or before collection.
- **EFE artefacts** : `transparency_notice` per catalog + tenant onboarding
- **Evidence script** : `comp_transparency_notice_evidence`
- **Cross-ref GDPR** : Art. 13

---

## 3. Consumer rights request workflow

```
[Request submitted]
   │
   ▼
[Identity verification]   ←── DT-COMP-LIVE-CCPA-IDV (US-specific KYC)
   │
   ▼
[Request triage]
   │
   ├── Know → portability export (≤ 30 days, GDPR Art. 20 satisfies)
   ├── Delete → erasure cascade (≤ 30 days, GDPR Art. 17 satisfies)
   ├── Opt-out → consent record update (axis third_party_sharing = false)
   └── Correct → label edit + audit
   │
   ▼
[Audit log + response]
```

---

## 4. CCPA-specific extensions vs GDPR

| Aspect | CCPA | GDPR | EFE |
|---|---|---|---|
| Verification | "Reasonable" identity verification | Strict identity proof | EFE uses MFA + email re-confirm |
| Deadline | 45 days (extendable +45) | 30 days (extendable +60) | EFE applies stricter = 30 days |
| Sale opt-out | Required link "Do Not Sell" | N/A explicit | EFE = 4-axis consent UI |
| Children | Opt-in for < 16 | Opt-in for < 16 (or country-specific) | EFE blocks < 16 by default |

---

## 5. Cross-references

- GDPR : same rights `gdpr.md` Art. 13/15/16/17/20
- Φ-D-COMPLIANCE : doctrine D-COMP-5 (erasure 30d) covers CCPA delete
- Φ-D-COMPLIANCE : doctrine D-COMP-9 (consent granularité 4 axes) covers CCPA opt-out

---

**End of CCPA + CPRA control mapping for EFE.**
