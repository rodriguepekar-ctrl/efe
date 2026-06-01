# M25 Agent Runtime Runbook

**Audience** : SRE on-call, security on-call, tenant operators.
**Last reviewed** : 2027-Q4 (M25 GA pilot).
**Status** : EXPERIMENTAL. Single design-partner pilot. Sunset clause
if < 2 tenants at 12 months.

---

## 0. Severity & escalation

| Symptom | Severity | Page |
|---|:---:|:---:|
| `efe_agent_sandbox_escape_attempts_total > 0` | **P0** | PagerDuty `efe-security` |
| Audit chain HMAC verify fails | P0 | `efe-security` |
| > 3 cost-cap hits / hour for one tenant | P1 | `efe-sre` |
| Approval expired without response > SLA | P2 | tenant ops |
| microVM provisioning P99 > 5 s | P3 | `efe-sre` |

---

## 1. Quick reference

| Task | Command |
|---|---|
| Sign DPIA for tenant | `efe-admin agent dpia sign --tenant <slug> --addendum-hash sha256:<hex> --confirm` |
| Enable feature flag | `kubectl set env deploy/efe-agent-runner-controller EFE_FEATURE_AGENT_RUNTIME_ENABLED=true` |
| List active runs | `efe-admin agent list --tenant <slug>` |
| Tail run | `efe-admin agent tail <run_id>` |
| Approve step | `efe-admin agent approve <approval_id> --reason "..." --confirm` |
| Force terminate | `efe-admin agent terminate <run_id> --reason "..." --confirm` |
| Verify audit chain | `efe-admin audit verify <slug> --module=agent_runtime` |
| Daily cost | `efe-admin agent cost daily --tenant <slug> --days 7` |

---

## 2. Onboarding tenant to M25

1. **Verify dependencies** :
   - `EFE_FEATURE_AGENT_RUNTIME_ENABLED=true`
   - `EFE_FEATURE_M21_LLMROUTER=true`
   - Helm chart `efe-agent-runner` deployed in `efe-system`
   - Cluster nodes have `/dev/kvm`

2. **Sign DPO DPIA addendum** (M25-specific) — covers :
   - Autonomous LLM-driven agent as new high-risk AI system (AI Act Art. 9)
   - Data sources accessed via tools (mcp-server proxy)
   - 12-month re-sign requirement
   - Right to erasure cascade scope

3. **Set tenant flags** :
   ```sql
   UPDATE tenant_feature SET
     m25_agent_runtime_enabled = TRUE,
     m25_dpia_signed_at = NOW(),
     m25_dpia_expires_at = NOW() + INTERVAL '365 days',
     m25_dpia_addendum_hash = 'sha256:<hex>',
     m25_cost_cap_per_run_usd = 5.00,
     m25_timeout_minutes_max = 60,
     m25_allowed_profiles = ARRAY['research_lite']::text[],
     m25_allowed_providers = ARRAY['openai']::text[],
     m25_memory_enabled = FALSE,
     m25_microvm_pool_size = 2,
     m25_max_concurrent_runs = 5
   WHERE id = '<tenant_id>';
   ```

4. **Upload AgentRunTemplate** :
   ```bash
   efe-admin agent template upsert --file template-research.yaml --confirm
   ```

5. **Smoke test** : create a noop run, watch it complete, verify
   audit chain (`efe-admin audit verify <slug> --module=agent_runtime`).

---

## 3. Daily operator workflow

1. Open `/agent` in portal-admin → check active runs.
2. Review pending approvals (`/agent/runs/[id]` → orange banner).
3. Watch cost dashboard `/agent/cost` → flag any > 80 % daily budget.
4. Check on-call PagerDuty for any P0 alerts (escape / HMAC).

---

## 4. SLA targets

| Tier | Provisioning P99 | Approval response | Cost-cap response |
|---|:---:|:---:|:---:|
| Pilot (single design partner) | 5 s | 30 min | ≤ 10 s |

---

## 5. Common incidents

### 5.1 Sandbox escape attempt detected

**Symptoms** : `efe_agent_sandbox_escape_attempts_total{tenant=…} > 0`.
PagerDuty P0 fires immediately.

**Action** :
1. **Freeze the tenant** :
   ```bash
   kubectl annotate agentrun -l efe.ai/tenant=<slug> efe.ai/freeze=true
   ```
2. Open security incident (P0).
3. Pull the audit row :
   ```sql
   SELECT * FROM agent_audit
   WHERE tenant_id='<slug>' AND event_type='sandbox_escape_attempt_detected'
   ORDER BY created_at DESC LIMIT 10;
   ```
4. Dump microVM forensics (the host node-daemon stores the last-known
   syscall trace at `/var/lib/efe/agent-runner/forensics/<vm_id>/`).
5. Disable M25 for the tenant pending root cause :
   ```sql
   UPDATE tenant_feature SET m25_agent_runtime_enabled=FALSE WHERE id='<slug>';
   ```

### 5.2 Audit chain HMAC verify fails

**Symptoms** : `efe-admin audit verify` returns `AuditHmacFailure`.

**Action** :
1. **CRITICAL P0** : freeze ALL M25 activity (server flag OFF).
2. Open security incident.
3. Investigate : DB row modified outside the application path?
4. Restore from backup if confirmed tampering ; otherwise re-key + replay.

### 5.3 Cost-cap storm

**Symptoms** : 3+ runs hit COST_CAPPED in one hour for the same tenant.

**Action** :
1. Inspect the failing template / prompts — usually adversarial input
   pushing the LLM into long loops.
2. Reduce `m25_cost_cap_per_run_usd` for the tenant temporarily.
3. Notify the tenant operator.

### 5.4 Approval expired without response

**Symptoms** : `efe_agent_approval_expired_total` increments.

**Action** :
1. The run will auto-fail. No SRE action required unless > 3 in 24 h
   for one tenant ; then page the tenant operator.

### 5.5 microVM provisioning slow / failing

**Symptoms** : `efe_agent_microvm_provision_duration_seconds` P99 > 5 s,
or `outcome="error"` rate > 1 %.

**Action** :
1. Check node-daemon health on every worker (`kubectl get pods -n efe-system -l app=efe-agent-runner-node`).
2. `/dev/kvm` permission denied? → DaemonSet privileged flag missing.
3. Disk pressure? → check `df -h` on nodes.
4. If multi-node : drain affected node and let pool recover.

### 5.6 Pending approval expired during incident

If an approval expired *because* the operator was paging us during an
incident (unrelated), they can re-create the approval via :
```bash
efe-admin agent get <run_id>     # find new pending approval id
efe-admin agent approve <id> --reason "operator was paged ..."  --confirm
```

---

## 6. Disaster recovery

### 6.1 Lost agent_state (checkpoint)

Checkpoints are KMS-encrypted ; if the row is lost the run cannot be
resumed. Mark the run as FAILED and ask the operator to restart.

### 6.2 Lost agent_audit rows

The HMAC chain detects loss (`AuditChainBroken`). PITR restore is the
only path ; otherwise re-key + freeze the tenant pending DPO review.

### 6.3 Lost AgentRunTemplate

Templates are versioned ; recover from `git` (operators are encouraged
to commit `template.yaml` files alongside infra-as-code).

---

## 7. Compliance handoff

The compliance pack regen cron (R-100) adds for M25 :
- `M25_DPIA_LOG.json` — DPIA signatures + hashes + expiries
- `M25_AGENT_RUN_AUDIT.json` — last 90 days of agent runs
- `M25_TOOL_REJECTION_LOG.json` — whitelist rejections (last 90 days)
- `M25_APPROVAL_LOG.json` — Art. 14 approvals (last 90 days)

External auditors verify via Rekor (cosign signatures) +
`efe-admin audit verify <tenant> --module=agent_runtime`.

---

## 8. Sunset checklist (T+12 months)

If < 2 active tenants at the steering committee review :
1. Announce sunset in CHANGELOG with 90-day deprecation window.
2. Disable feature flag for new tenants ; let existing finish in flight.
3. Drop `agent_run` (etc.) tables in a follow-up migration.
4. Remove Helm chart from charts repo.
5. Archive `crates/efe-agent-runner` + `agent-core/` to a frozen branch.

---

## 9. References

- Architecture : [`../EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md`](../EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md)
- Migration : [`../../crates/efe-server/migrations/0016_agent_runtime.sql`](../../crates/efe-server/migrations/0016_agent_runtime.sql)
- Helm chart : [`../../charts/efe-agent-runner/`](../../charts/efe-agent-runner/)
- DPIA template : [`../release/DPIA_M25_TEMPLATE.md`](../release/DPIA_M25_TEMPLATE.md)
- Profile config : [`../../config/agent-profiles.yaml`](../../config/agent-profiles.yaml)
- AI Act Art. 9 / 14 / 26 hooks : architecture doc § 9
