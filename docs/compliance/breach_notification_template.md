# Personal Data Breach Notification Template

> **Status** : Sprint M14 D41 (Φ-B) audit fix M14-A2 — DPA-ready breach
> notification template per GDPR Art. 33.
> **Audience** : DPO, EU DPA, customer DPOs.
> **Last updated** : 2026-04-30.

---

## To: <Lead Supervisory Authority>
## From: EFE Project DPO (dpo@efe.ai)
## Subject: Personal Data Breach Notification — Reference <INCIDENT-ID>

---

### 1. Notification metadata
- **Notification timestamp** : `<UTC ISO 8601>`
- **Hours since detection** : `<hours since T+0>`
- **Reference ID** : `<INCIDENT-ID>`
- **Breach status** : Confirmed | Suspected | Under investigation
- **Severity** : P0 | P1
- **Initial / Update / Final** : Initial

### 2. Nature of the personal data breach (Art. 33(3)(a))
- **What happened** :
  `<Description of the breach event>`
- **When** :
  `<UTC timestamp of breach event T+0>`
- **How discovered** :
  `<Detection mechanism>`
- **Categories of personal data concerned** :
  - [ ] Identification data (name, email)
  - [ ] Contact details (phone, address)
  - [ ] Authentication data (hashed credentials)
  - [ ] Special categories (Art. 9) — `<specify>`
  - [ ] Other — `<specify>`
- **Categories of data subjects concerned** :
  - [ ] EFE customers (B2B)
  - [ ] End-users of customer applications
  - [ ] Other — `<specify>`
- **Approximate number of data subjects affected** :
  `<integer or range>`
- **Approximate number of data records affected** :
  `<integer or range>`

### 3. DPO contact (Art. 33(3)(b))
- **DPO name** : `<name>`
- **DPO email** : dpo@efe.ai
- **DPO phone** : `<phone>`
- **PGP fingerprint** : `<fingerprint>`

### 4. Likely consequences (Art. 33(3)(c))
- **Risk to data subjects** : High | Medium | Low
- **Specific consequences** :
  `<e.g., financial loss, identity theft, reputational damage,
   discrimination, restriction of rights, loss of confidentiality of
   pseudonymised data, etc.>`

### 5. Mitigation measures (Art. 33(3)(d))
- **Already taken** :
  - `<Mitigation 1>`
  - `<Mitigation 2>`
- **Planned within 7 days** :
  - `<Planned mitigation>`

### 6. Notification to data subjects (Art. 34)
- **Required ?** : Yes (high risk) | No (mitigated)
- **If required, when** : `<planned date>`
- **Communication method** : Email + status.efe.ai post

### 7. Cross-border aspects
- **Other DPAs involved** : `<list>`
- **Lead supervisory authority** : `<authority name>`

### 8. Annexes
- [ ] Forensic timeline
- [ ] Affected data sample (hashed)
- [ ] Mitigation evidence
- [ ] Customer notification draft (if Art. 34 applies)

---

## Customer notification template (Art. 34, if required)

```
Subject: Important security notice regarding your EFE deployment

Dear <Customer DPO>,

On <DATE>, we detected an incident affecting <SCOPE>. We are notifying
you proactively per GDPR Art. 34 because the risk to your data
subjects' rights and freedoms may be high.

Summary :
- What happened : <description>
- When : <timestamp>
- Affected scope : <tenants / data categories / data subjects approximate count>
- Mitigation already deployed : <list>
- Required actions on your side : <if any>

We have notified <DPA> per Art. 33 on <DATE>.

For full details, please contact dpo@efe.ai.

The EFE Security team
```

---

## Public notice template (status.efe.ai)

```
🛡 [POST-MORTEM] Security incident <ID> — <DATE>

We experienced an incident affecting <SCOPE> from <TIME> to <TIME> UTC
on <DATE>. Affected data has been contained, mitigated, and customers
notified per applicable data protection laws.

Full sanitized post-mortem : <URL>
```

---

## Reference

- GDPR Art. 33 (Notification of a personal data breach to the supervisory authority)
- GDPR Art. 34 (Communication of a personal data breach to the data subject)
- EDPB Guidelines 9/2022 on personal data breach notification
- WP29 Guidelines on Personal data breach notification
