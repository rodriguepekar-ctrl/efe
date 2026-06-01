# Runbook : General Incident Response Playbook

> **Severity** : applies to all P0 / P1
> **ETA to resolution** : per severity SLA
> **Sprint origin** : M14 D42
> **Last updated** : 2026-04-30
> **Owner** : Security lead (backup : DevOps lead)

---

## 1. Symptom

Any P0 or P1 alert / customer report / external notification.

## 2. Why this matters (impact)

- Customer impact + SLA breach
- Regulatory notification obligations (GDPR Art. 33, AI Act Art. 73, NIS2)
- Reputation + trust

## 3. Diagnostics

### 3.1 Triage (first 15 min)

```bash
# 1. Acknowledge alert in PagerDuty / Opsgenie
# 2. Assess severity (P0 / P1 / P2 / P3)
# 3. Open incident channel #incident-<YYYY-MM-DD-N>
# 4. Page secondary if uncertain
```

### 3.2 Severity classification

| Symptom | Severity |
|---|---|
| Data loss / corruption | P0 |
| Production outage | P0 |
| Authentication completely broken | P0 |
| Tenant isolation breach (cross-tenant data) | P0 |
| Backup or DR system non-functional | P0 |
| Active security incident (breach in progress) | P0 |
| Significant degraded service (P95 violations) | P1 |
| Authentication intermittent | P1 |
| Single-tenant outage | P1 |
| Functional bug (non-blocking) | P2 |
| Documentation issue | P3 |

## 4. Mitigation steps

### 4.1 Containment (first 30 min for P0)

1. Stop the bleeding (e.g., disable affected endpoint, rollback recent change)
2. Preserve forensic state (snapshot logs, metrics, K8s events)
3. Notify primary stakeholders (eng lead + CISO)

### 4.2 Communication (first 1h for P0)

1. Status page update (status.efe.ai)
2. Customer notification draft prepared
3. Internal Slack `#eng-incidents` updated hourly
4. Slack `#exec-bridge` for P0 leadership awareness

### 4.3 Resolution

1. Identify root cause
2. Apply fix (cherry-pick to release branch if patch needed)
3. Verify fix in staging
4. Roll out to production
5. Monitor for 1h post-fix
6. Customer notification of resolution

### 4.4 Regulatory notification

| Trigger | Authority | Timeline |
|---|---|---|
| Personal data confirmed exposed | EU DPA (CNIL/etc.) | 72h (Art. 33 GDPR) |
| Personal data exposed + high risk | Affected customers | 72h (Art. 34 GDPR) |
| AI system serious incident | EU AI Office | 15 days (Art. 73 AI Act) |
| NIS2 critical entity incident | National CSIRT | 24h initial + 72h full report |
| US customer breach | State AGs (varies) | per state law |

## 5. Resolution criteria

- Service restored to normal SLOs for 1h
- 0 P0 alerts firing
- Customer notifications sent
- Post-incident review scheduled

## 6. Post-incident actions

- [ ] Post-mortem within 5 business days (template in HANDOFF_OPS §10)
- [ ] Action items tracked in security backlog
- [ ] Anti-regression pin tests added if root cause is code
- [ ] Threat model updated if new attack vector
- [ ] Risk register updated
- [ ] Customer-facing post-mortem (sanitized) for material incidents

## 7. Related runbooks

- All sprint-specific runbooks (`d14_*`, `d15_*`, etc.)
- `general_breach_notification.md`
- `general_secret_rotation_playbook.md`
