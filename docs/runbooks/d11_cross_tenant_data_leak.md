# Runbook : d11_cross_tenant_data_leak

> **Severity** : P0 (CRITICAL — multi-tenant isolation breach)
> **ETA to resolution** : Immediate forensic preservation
> **Sprint origin** : D11 (per-tenant isolation)
> **Last updated** : 2026-04-30
> **Owner** : Security lead + DPO + CISO

---

## 1. Symptom

- Customer reports : "I see another tenant's data"
- Audit log shows cross-tenant query unexpectedly succeeded
- Test pin `test_d11_per_tenant_isolation.py` fails in production

## 2. Why this matters

- **CRITICAL** — multi-tenant data leakage = breach of GDPR Art. 32
- Compliance impact : SOC 2 CC6.1 + ISO A.5.15 + GDPR Art. 32(1)(b) + AI Act Art. 15
- Customer trust + legal liability + potential GDPR Art. 33/34 notification

## 3. Diagnostics

```bash
# Check audit log for cross-tenant queries
kubectl exec -n efe-pg efe-pg-0 -- psql -c "
SELECT tenant_id, action, resource, timestamp
FROM audit_events
WHERE tenant_id != extracted_jwt_tenant_id
ORDER BY timestamp DESC LIMIT 50;
"

# Check NetworkPolicy strict in place
kubectl get networkpolicy -n efe-production efe-server-strict -o yaml

# Check JWT validation logs
kubectl logs -n efe-production -l app=efe-server --since 1h | grep "tenant_id"
```

## 4. Mitigation (CRITICAL response)

### Immediate (forensic preservation)
1. **STOP further reads/writes** for affected tenants :
   ```bash
   kubectl scale deployment efe-server --replicas=0 -n efe-production
   ```
2. **Snapshot state** for forensics :
   ```bash
   kubectl exec -n efe-pg efe-pg-0 -- pg_dump --schema-only audit_events > /tmp/forensic-$(date +%s).sql
   ```
3. **Notify** : security@efe.ai + dpo@efe.ai + ciso@efe.ai + legal@efe.ai

### Investigation
4. Identify scope :
   - How many tenants affected
   - What data was exposed
   - Time window of exposure
   - Root cause (RBAC bug / config drift / NetworkPolicy gap)

### Containment
5. If RBAC bug : pin to known-good revision via `helm rollback`
6. If config drift : reset Helm values to canonical
7. If NetworkPolicy gap : enable strict NP if disabled

### Notification
8. **GDPR Art. 33** : EU DPA notification within 72h
9. **GDPR Art. 34** : affected customer DPOs within 72h
10. **AI Act Art. 73** : EU AI Office within 15 days (if AI-related)
11. **Customer SLA** : per Tier 3 contractual obligations

## 5. Resolution criteria

- Root cause identified + fixed
- Pin tests `test_d11_per_tenant_isolation.py` pass in production
- All affected customers notified
- All required regulatory notifications sent
- Post-mortem published (sanitized public version)

## 6. Post-incident actions

- [ ] Update threat model with new attack vector
- [ ] Add anti-regression pin tests
- [ ] Customer compliance report update
- [ ] Possible regulator follow-up
- [ ] Insurance claim filed if applicable
- [ ] Lessons learned documented

## 7. Related runbooks

- `general_breach_notification.md`
- `general_incident_playbook.md`
- `d14bis_audit_chain_broken.md`
