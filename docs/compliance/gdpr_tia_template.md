# Transfer Impact Assessment (TIA) Template

> **Status** : Sprint M14 D41 (Φ-B) audit fix M14-A2 — TIA template
> for international transfers per GDPR Chapter V + Schrems II.
> **Audience** : Customer DPOs, EFE DPO.
> **When required** : For transfers to non-adequacy-recognized countries.
> **Last updated** : 2026-04-30.

---

## TIA — `<Customer Tenant>` ←→ `<Destination Country>`

### 1. Transfer scenario
- **From** : `<EU country>`
- **To** : `<destination country>`
- **Adequacy decision exists ?** : Yes (UK, Switzerland, Japan, etc.) / No
- **If No, applicable safeguard** :
  - [ ] SCC (EU 2021/914 Module 2 / 3 / 4)
  - [ ] BCR (Binding Corporate Rules)
  - [ ] Article 49 derogation

### 2. Categories of data transferred
- `<list>`

### 3. Recipient
- **Recipient** : `<entity>`
- **Country** : `<country>`
- **Sector** : `<sector>`

### 4. Local law assessment
- **National security access laws** : `<assessment>`
- **Surveillance laws (e.g., FISA 702 if US)** : `<assessment>`
- **Effective remedies for data subjects** : `<assessment>`

### 5. Risk assessment
- **Risk of access by public authorities** : Low / Medium / High
- **Risk of mass surveillance** : Low / Medium / High
- **Risk of bulk data collection** : Low / Medium / High

### 6. Supplementary measures
- [ ] Encryption at rest (KMS keys held in EU)
- [ ] Encryption in transit (TLS 1.3)
- [ ] Pseudonymization (PII hash D13)
- [ ] Customer-managed KMS (Tier 3 BYOK)
- [ ] Logical access controls (RBAC + audit)
- [ ] Other : `<specify>`

### 7. Outcome
- **Transfer permitted** : Yes / No (with conditions)
- **Conditions** : `<list>`
- **Re-assessment frequency** : Annual

### 8. Sign-off
- Controller DPO : `<name + signature>`
- Processor DPO (consulted) : `<name + signature>`
- Date : `<date>`

---

## Reference
- GDPR Chapter V (transfers to third countries)
- EU Standard Contractual Clauses 2021/914
- Schrems II decision (CJEU)
- EDPB Recommendations 01/2020 on supplementary measures
