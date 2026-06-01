# Runbook : General Breach Notification Procedure

> **Severity** : P0 (any confirmed breach)
> **ETA to resolution** : T+30 days
> **Sprint origin** : M14 D42
> **Last updated** : 2026-04-30
> **Owner** : DPO + CISO + Legal

---

## 1. Symptom

Confirmed personal data breach (Art. 4(12) GDPR) :
- Accidental or unlawful destruction
- Loss
- Alteration
- Unauthorized disclosure
- Unauthorized access

## 2. Why this matters (impact)

- GDPR Art. 33 : DPA notification within 72h
- GDPR Art. 34 : data subject notification (if high risk)
- AI Act Art. 73 : EU AI Office notification within 15d (if AI-related)
- NIS2 Art. 23 : CSIRT notification within 24h (critical entities)
- Reputation + trust

## 3. Diagnostics

```bash
# Confirm breach :
# 1. What data was exposed ? (PII categories per GDPR Art. 9)
# 2. How many data subjects affected ?
# 3. When did it happen ? (T+0 of breach)
# 4. Who has the data now ?
# 5. Is the breach contained ?
```

## 4. Mitigation steps

### T+0 — Detection
- Open incident channel
- Page DPO + CISO + Legal
- Begin forensic preservation

### T+1h — Severity assessment
- DPO + CISO classify (high / low risk)
- Forensic logs frozen
- Affected systems isolated

### T+4h — DPO formal engagement
- DPO drafts initial breach notification
- Legal review starts
- CSIRT preparation if NIS2 customer

### T+24h — NIS2 CSIRT (if applicable)
- Initial notification to national CSIRT
- Customer escalation if Tier 3

### T+72h — GDPR Art. 33 notification
- Full notification to lead supervisory authority (CNIL France / BfDI Germany / ICO UK / etc.)
- Use template `breach_notification_template.md`
- Include : nature, categories, approximate count, contact, consequences, mitigations

### T+1d — Customer notification (Art. 34 if high risk)
- Email to affected customer DPOs
- Status page update
- Customer call for Tier 3+

### T+7d — Public statement
- status.efe.ai notice
- Press release if material

### T+14d — Sanitized post-mortem
- Public version on docs.efe.ai
- Internal full version to board

### T+15d — AI Act Art. 73 (if AI-related serious incident)
- Notification to EU AI Office
- Use template per upcoming implementing acts

### T+30d — Remediation report
- Full remediation report to DPA
- Action items tracked + status
- Independent verification (post-mortem signed by external auditor optional)

## 5. Resolution criteria

- All required regulatory notifications sent
- Affected customers notified
- Public statement issued (for material breaches)
- Remediation deployed
- Risk register updated
- Threat model updated

## 6. Post-incident actions

- [ ] Update threat model with new attack vector
- [ ] Add anti-regression pin tests
- [ ] Update VDP if external researcher involved
- [ ] Customer renewals proactively addressed
- [ ] Insurance claim filed if applicable
- [ ] Lessons learned documented in compliance report

## 7. Templates

- `breach_notification_template.md` (DPA-ready)
- Customer notification email template
- Press release template
- Internal post-mortem template

## 8. Related runbooks

- `general_incident_playbook.md`
- `d11_cross_tenant_data_leak.md` (specific scenario)
- `d14bis_audit_chain_broken.md`
