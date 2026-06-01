# M26 Agent Runtime — Production Runbook

**Audience** : SRE on-call (primary), tenant operators (sections 3 + 4),
DPO (sections 5 + 7), security on-call (section 5).
**Last reviewed** : 2027-Q4 (M26 GA).
**Supersedes** : `M25_AGENT_RUNTIME_RUNBOOK.md` (M25 runbook stays valid
for tenants on EXPERIMENTAL channel).

---

## 0. Severity + escalation

| Symptom | Sev | Routing |
|---|:--:|---|
| `efe_agent_sandbox_escape_attempts_total > 0` | **P0** | PagerDuty `efe-agent-oncall-prod` (phone+SMS+push) |
| Audit chain HMAC verify fails | **P0** | PagerDuty + Security on-call |
| Admission webhook availability < 99 % over 5 min | P0 | PagerDuty |
| REST gateway availability < 99 % over 5 min | P1 | PagerDuty (push only) |
| `efe_agent_microvm_provision_duration_seconds` P99 > 5 s for 10 min | P1 | PagerDuty |
| Cost cap → SIGTERM P99 > 60 s | P1 | PagerDuty |
| Approval response P95 > 1 h for 1 h | P2 | PagerDuty business hours |
| GDPR Art. 17 cascade pending > 30 d | P1 | PagerDuty + DPO |
| Run failure rate > 2 % for 30 min | P2 | Slack `#efe-agent-runtime-alerts` |
| Pen test scenario failure (P0/P1) | P1 | Security on-call |
| Image whitelist rejection > 5/day | P2 | Slack |

---

## 1. Quick reference (ops cheat sheet)

| Task | Command |
|---|---|
| Sign DPIA | `efe-admin agent dpia sign --tenant <s> --addendum-hash sha256:<h> --confirm` |
| Check graduation criteria | `efe-admin agent graduate check --tenant <s>` |
| Confirm graduation | `efe-admin agent graduate confirm --tenant <s> --acknowledgement-hash sha256:<h> --confirm` |
| Revoke graduation | `efe-admin agent graduate revoke --tenant <s> --reason "..." --confirm` |
| List active runs | `efe-admin agent list --tenant <s>` |
| Tail run | `efe-admin agent tail <run_id>` |
| Approve step | `efe-admin agent approve <approval_id> --reason "..." --confirm` |
| Force terminate | `efe-admin agent terminate <run_id> --reason "..." --confirm` |
| Verify audit chain | `efe-admin audit verify <s> --module=agent_runtime` |
| Daily cost | `efe-admin agent cost daily --tenant <s> --days 7` |
| Submit Art. 26 report | `efe-admin agent post-market submit --tenant <s> --quarter 2027-Q4` |

---

## 1.5. Cluster prerequisites (D93)

Before installing the Helm chart on a fresh cluster :

### 1.5.1 Label KVM-capable nodes

The vsock-relayer DaemonSet targets nodes with the
`efe.ai/kvm-available=true` label. If no node has this label, microVMs
can never launch.

```bash
# Probe each node for /dev/kvm + label the ones that have it.
kubectl get nodes -o json | jq -r '.items[].metadata.name' | while read n; do
  if kubectl debug node/$n -it --image=busybox -- ls /host/dev/kvm 2>/dev/null >/dev/null; then
    echo "$n : KVM available"
    kubectl label node $n efe.ai/kvm-available=true --overwrite
  else
    echo "$n : KVM not available (skipping)"
  fi
done
```

If you skip this step, the controller emits a warning event :
`MicrovmNoKvmNodes — no nodes labelled efe.ai/kvm-available=true`.

### 1.5.2 cert-manager Issuer

Production : pre-install a ClusterIssuer that signs against your CA,
then override `Values.admissionWebhook.certManager.issuerRef` to its
name.

Local / dev / fresh clusters : set
`Values.admissionWebhook.certManager.bootstrap: true` to provision a
self-signed Issuer scoped to the release namespace.

### 1.5.3 HMAC secrets

The controller refuses to start without `EFE_AUDIT_HMAC_KEY` and
`EFE_APPROVAL_HMAC_KEY`. Generate :

```bash
kubectl create secret generic efe-audit-hmac-key \
  --from-literal=key="$(openssl rand -hex 32)" -n efe-system
kubectl create secret generic efe-approval-hmac-key \
  --from-literal=key="$(openssl rand -hex 32)" -n efe-system
```

---

## 2. Onboarding tenant to STABLE

### 2.1 Prerequisites

1. Tenant has run on EXPERIMENTAL for ≥ 30 days.
2. Tenant has run ≥ 100 successful runs.
3. Zero `sandbox_escape_attempt_detected` events in last 90 days.
4. DPIA signed within last 11 months.
5. R-100 compliance pack regen succeeded for the tenant.
6. DPO has reviewed previous quarter.
7. AI Act Art. 26 post-market report submitted for previous quarter.
8. M26 tenant graduation agreement signed (operator + DPO).

### 2.2 Cluster-level prerequisites

- `efe-agent-runner` Helm chart at version 0.3.0+.
- Admission webhook `failurePolicy: Fail` (default in M26).
- vsock-relayer DaemonSet healthy on every KVM-capable node.
- 14 SLOs deployed and burn-rate alerts validated.
- Pen test bundle last run : 0 P0/P1 unresolved.
- DR drill completed in current quarter.
- 6 stakeholder sign-offs in `docs/release/M26_GRADUATION_SIGN_OFF.md`.

### 2.3 Execution

```bash
# 1. Pre-flight check.
efe-admin agent graduate check --tenant acme

# 2. Iterate on blocking items until ready=true.

# 3. Have the tenant security lead + DPO sign the M26 graduation agreement
#    (docs/release/M26_TENANT_GRADUATION_AGREEMENT.md).

# 4. Compute the acknowledgement hash :
sha256sum signed-agreement.pdf

# 5. Confirm graduation :
efe-admin agent graduate confirm \
  --tenant acme \
  --acknowledgement-hash sha256:<hex> \
  --confirm

# 6. Verify channel is now STABLE :
efe-admin agent graduate status --tenant acme
```

---

## 3. Daily operator workflow

1. Open `/agent` in portal-admin → check active runs.
2. Review pending approvals → `/agent/runs/[id]` → orange banner.
3. Watch cost dashboard `/agent/cost` → flag any > 80 % daily budget.
4. Check `/agent/dpia` → ensure expiry > 60 days out (re-sign if not).
5. Check PagerDuty + Slack `#efe-agent-runtime-alerts` for P0/P1.

---

## 4. Common incidents

### 4.1 Sandbox escape attempt

**Symptom** : `efe_agent_sandbox_escape_attempts_total > 0` ; PagerDuty P0.

**Action** :
1. Freeze the tenant : `kubectl annotate agentrun -l efe.ai/tenant=<s> efe.ai/freeze=true`.
2. Open security incident (P0).
3. Pull the audit row :
   ```sql
   SELECT * FROM agent_audit
   WHERE tenant_id='<s>' AND event_type='sandbox_escape_attempt_detected'
   ORDER BY created_at DESC LIMIT 10;
   ```
4. Pull the forensics dump from the host node-daemon :
   `/var/lib/efe/agent-runner/forensics/<vm_id>/`.
5. If confirmed, revoke the tenant graduation immediately :
   `efe-admin agent graduate revoke --tenant <s> --reason "P0 sandbox escape <date>" --confirm`.
6. File a `pen_test_finding` row with severity=P0.
7. Schedule a CSC emergency meeting within 24 h.

### 4.2 Audit chain HMAC verify fails

**Symptom** : `efe-admin audit verify` returns AuditHmacFailure ; PagerDuty P0.

**Action** :
1. **Freeze ALL M26 activity** : flip `EFE_FEATURE_AGENT_RUNTIME_ENABLED=false`
   on the controller (rollout restart).
2. Open security incident.
3. Determine the scope :
   ```sql
   SELECT MIN(created_at), MAX(created_at) FROM agent_audit
   WHERE tenant_id='<s>'
     AND id = '<broken_chain_row_id>';
   ```
4. Restore from PITR backup if confirmed tampering.
5. Re-key per `M26_DR_PLAYBOOK.md` § 4.

### 4.3 Admission webhook down

**Symptom** : webhook availability < 99.9 % over 5 min ; PagerDuty P0.

**Action** :
1. Check controller pods : `kubectl get pods -n efe-system -l app.kubernetes.io/name=efe-agent-runner`.
2. If both replicas are unhealthy, check :
   - cert-manager cert validity : `kubectl describe certificate -n efe-system <name>`.
   - DB connectivity from controller pods.
   - Logs for `webhook server` errors.
3. **Do NOT** flip the webhook to `failurePolicy: Ignore` — this would
   allow invalid CRs into etcd. Instead, scale the controller to
   `replicas: 4` temporarily and investigate.

### 4.4 Cost cap response slow

**Symptom** : `cost_cap_response` SLO breach ; PagerDuty P1.

**Action** :
1. Check the watchdog loop is running : controller logs every 5 s.
2. Check DB pool saturation. If saturated, scale up Postgres.
3. If individual tenant's runs are over-spending, force-terminate :
   `efe-admin agent terminate <run_id> --reason "cost watchdog stalled" --confirm`.

### 4.5 Approval expiring without response

**Symptom** : `approval_expired_total / total > 5 %` for 1 h.

**Action** :
1. Identify which tenant : `SELECT tenant_id, COUNT(*) FROM agent_approval_request
   WHERE status = 'EXPIRED' AND decided_at > NOW() - INTERVAL '1 hour' GROUP BY 1;`
2. Page the tenant operator (per their on-call rotation).
3. If the tenant operator can't respond, EFE has no ability to approve
   on their behalf — the runs simply fail. Document in Slack.

### 4.6 GDPR Art. 17 cascade pending too long

**Symptom** : `efe_agent_gdpr_cascade_oldest_pending_seconds` > 30 days.

**Action** :
1. Identify the request :
   ```sql
   SELECT * FROM bundle_rebuild_request
   WHERE status IN ('PENDING','IN_PROGRESS')
     AND created_at < NOW() - INTERVAL '30 days';
   ```
2. Notify DPO + legal team.
3. If catastrophic urgency, force-trigger : `efe-admin learning retune trigger --catalog <id>`.

### 4.7 vsock-relayer pod crash

**Symptom** : DaemonSet readiness probe failing on a node.

**Action** :
1. `kubectl describe pod -n efe-system <relayer-pod>` — check error.
2. Common causes : cert-manager cert expired, mTLS upstream unreachable,
   vsock device permissions.
3. While the relayer is down, in-flight runs on that node will fail
   their LLM/MCP calls. The agent-core retries (tenacity) ; if not
   recovered within ~30 s, the run errors out.
4. Drain the node if needed : `kubectl drain <node>`.

---

## 5. Security incidents (DPO + Security on-call)

### 5.1 Pen test P0/P1 finding (post-GA)

If a pen test scenario fails post-GA :
1. File `pen_test_finding` row with severity.
2. Open security incident.
3. P0 : freeze graduation for new tenants ; revoke for existing if scope warrants.
4. P1 : fix in next sprint ; document mitigations.

### 5.2 cosign key compromise

Per `M26_DR_PLAYBOOK.md` § 5.

### 5.3 Bug bounty report

If an external researcher reports a vulnerability :
1. Acknowledge within 24 h.
2. Triage severity using the same rubric as pen test findings.
3. Fix per SLAs in the bounty program.
4. Document in the post-market quarterly report (Section 6 — security
   incidents).

---

## 6. Disaster recovery

See `M26_DR_PLAYBOOK.md` for the full DR procedures (4 game-day
scenarios + step-by-step recovery).

---

## 7. Compliance handoff

The R-100 compliance pack regen cron exports for M26 :

- `M25_DPIA_LOG.json`
- `M25_AGENT_RUN_AUDIT.json`
- `M25_TOOL_REJECTION_LOG.json`
- `M25_APPROVAL_LOG.json`
- `M25_INCIDENT_LOG.json`
- `M26_GRADUATION_LOG.json` (NEW)
- `M26_PEN_TEST_LOG.json` (NEW)
- `AI_ACT_POST_MARKET_QUARTERLY/<tenant>/<quarter>.json` (NEW)

External auditors verify via Rekor (cosign signatures) + lineage query
(architecture doc § 18).

---

## 8. Sunset checklist (T+24 months)

If at 24 months :
- < 3 STABLE tenants OR
- Aggregate run volume < 10K/quarter at 18 months

The CSC convenes to vote on phased deprecation :
- **Phase 1** : freeze new tenant onboarding.
- **Phase 2** : 12-month migration window for existing tenants.
- **Phase 3** : end-of-life ; archive crate + Helm chart.

---

## 9. References

- Architecture : [`../EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md`](../EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md)
- DR playbook : [`./M26_DR_PLAYBOOK.md`](./M26_DR_PLAYBOOK.md)
- DPIA addendum : [`../release/DPIA_M26_ADDENDUM.md`](../release/DPIA_M26_ADDENDUM.md)
- Tenant agreement : [`../release/M26_TENANT_GRADUATION_AGREEMENT.md`](../release/M26_TENANT_GRADUATION_AGREEMENT.md)
- M25 runbook (still valid for EXPERIMENTAL) : [`./M25_AGENT_RUNTIME_RUNBOOK.md`](./M25_AGENT_RUNTIME_RUNBOOK.md)
