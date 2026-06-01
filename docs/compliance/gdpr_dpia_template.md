# DPIA — Data Protection Impact Assessment Template

> **Status** : Sprint M14 D41 (Φ-B) audit fix M14-A2 — DPIA template
> per GDPR Art. 35.
> **Audience** : Customer DPOs, EFE DPO consultation.
> **When required** : Per Art. 35(3) — large-scale processing of
> special categories, systematic + extensive evaluation, systematic
> monitoring of public areas at large scale.
> **Last updated** : 2026-04-30.

---

## DPIA Document — `<Customer Tenant Name>`

### 1. Description of the processing
- **Controller** : `<Customer entity legal name>`
- **Processor** : EFE Project (via SaaS / self-hosted)
- **Joint controllers / sub-controllers** : `<list>`
- **Description of processing operations** :
  `<e.g., automated semantic classification of customer support tickets>`
- **Data flow** :
  ```
  End-user → Customer app → EFE gRPC API → Embedding → Class label → Customer app
  ```
- **Geographic scope** : EU / EEA / Other

### 2. Purpose of processing (legitimate basis)
- **Legal basis (Art. 6)** :
  - [ ] Consent (Art. 6(1)(a))
  - [ ] Contract performance (Art. 6(1)(b))
  - [ ] Legal obligation (Art. 6(1)(c))
  - [ ] Vital interest (Art. 6(1)(d))
  - [ ] Public task (Art. 6(1)(e))
  - [ ] Legitimate interest (Art. 6(1)(f))
- **For special categories (Art. 9)** :
  - [ ] Explicit consent (Art. 9(2)(a))
  - [ ] Other — `<specify>`

### 3. Categories of data subjects + data processed
- **Data subjects** : `<categories>`
- **Personal data processed** : `<list>`
- **Special categories (Art. 9)** : `<list, if any>`
- **Data minimization assessment** :
  `<Justification that processing is limited to what is necessary>`

### 4. Necessity + proportionality assessment
- **Why necessary ?** : `<rationale>`
- **Less intrusive alternatives considered** : `<list>`
- **Why those alternatives rejected** : `<rationale>`

### 5. Risks to data subjects' rights and freedoms

| Risk | Likelihood | Impact | Severity |
|---|:---:|:---:|:---:|
| <Risk 1> | L/M/H | L/M/H | L/M/H |
| <Risk 2> | L/M/H | L/M/H | L/M/H |
| <Risk 3> | L/M/H | L/M/H | L/M/H |

### 6. Mitigation measures
| Risk | Mitigation | Owner | Effective date |
|---|---|---|---|
| <Risk 1> | <Mitigation> | <Owner> | <Date> |

### 7. Consultation
- **DPO consulted** : Yes | No
- **DPO opinion** : `<DPO comment>`
- **Data subjects consulted ?** : Yes | No
- **External experts consulted** : `<list>`

### 8. Final decision
- **Outcome** : Proceed | Proceed with modifications | Defer | Abandon
- **If proceed, conditions** : `<list>`
- **Approval** : `<approver>`
- **Date** : `<date>`

### 9. Review schedule
- **Annual review** : Yes
- **Triggered review on** : new data category / new sub-processor / new use case

### 10. Sign-off
- **Customer DPO** : `<name + signature>`
- **EFE DPO (consulted)** : `<name + signature>`
- **Customer Privacy counsel** : `<name + signature>`

---

## Reference
- GDPR Art. 35 (DPIA)
- WP29 Guidelines on DPIA
- ICO DPIA template
- CNIL DPIA methodology
