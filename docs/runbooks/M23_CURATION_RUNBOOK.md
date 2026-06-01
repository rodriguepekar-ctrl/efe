# M23 Curation Runbook

**Audience** : tenant operators, curation reviewers, SRE on-call.
**Last reviewed** : 2027-Q2 (M23 GA).

---

## 1. Quick reference

| Task | Command / URL |
|---|---|
| Enable M23 for a tenant | `efe-admin tenant enable-m23 <slug>` |
| List pending proposals | `efe-admin curation list --catalog <id>` |
| Approve a proposal | `efe-admin curation approve <proposal_id> --reason "..."` |
| Promote (start canary) | `efe-admin curation promote <proposal_id>` |
| Roll back a version | `efe-admin curation rollback <version_id> --reason "..."` |
| Drift events feed | `https://portal.efe.example.com/<slug>/curation/drift` |
| Cost dashboard | `https://portal.efe.example.com/<slug>/curation/cost` |
| Verify HMAC chain | `efe-admin audit verify <slug> --module=curation` |

---

## 2. Onboarding a new tenant to M23

1. **Verify dependencies**
   - Server-wide flags : `EFE_FEATURE_M23_CURATION_ENABLED=true` set in
     `efe-server` env.
   - If using internal LLM mode : `EFE_FEATURE_M21_LLMROUTER=true` AND
     `EFE_FEATURE_M23_USE_INTERNAL_LLM=true`.
2. **Sign DPA addendum** (required) — the M23 DPA addendum lists the LLM
   sub-processors that may be invoked when `internal_llm=true`.
3. **Set tenant flags**
   ```sql
   UPDATE tenant_feature SET
     m23_curation_enabled = true,
     m23_curation_allow_internal_llm = false,        -- recommended default
     m23_drift_detection_enabled = true,
     m23_curation_require_4_eyes = true,             -- recommended for enterprise
     m23_curation_budget_cents_per_day = 5000        -- $50/day
   WHERE id = '<tenant_id>';
   ```
4. **Bootstrap initial taxonomy_version** (v3.0.0) from current ACTIVE
   bundle :
   ```bash
   efe-admin curation init --tenant <slug> --catalog <id>
   ```
5. **Notify operator team** : welcome email + link to `/curation/queue`.

---

## 3. Daily reviewer workflow

1. **Check the queue** at `/<slug>/curation/queue`.
2. For each PENDING proposal :
   - Read rationale + diff preview on the card.
   - Click **View Full Diff** for the Sankey / heatmap.
   - **Approve** or **Reject** with a rationale (≥ 10 chars).
3. APPROVED proposals require a separate **Promote** action (typically
   by a `curation_promoter` or `taxonomy_admin`).
4. Promote starts a **24h canary**. Auto-rollback fires if metrics regress.

---

## 4. SLA targets

| Tier | Review SLA P99 | Notify after |
|---|:---:|:---:|
| Enterprise | 24 h | 4 h |
| Pro | 48 h | 8 h |
| Community | 7 d | 24 h |

PagerDuty escalation triggers if a HIGH-severity drift event is
unacknowledged for > 12 h.

---

## 5. Common incidents

### 5.1 Reviewer fatigue / queue backlog

**Symptoms** : > 20 PENDING proposals for a single tenant.

**Action** :
1. Aggregate drift events to identify root cause class.
2. Adjust `m23_drift_kl_threshold` upward by 0.05 (less sensitive).
3. Allocate a reviewer rotation : 1 hour / day dedicated.

### 5.2 Canary auto-rollback storm

**Symptoms** : 3+ consecutive promotes auto-rollback in 24h.

**Action** :
1. Check Prometheus : `efe_curation_promotions_total{outcome="rolled_back"}`.
2. Inspect `taxonomy_change_proposal.diff_json` of the rolled-back proposals.
3. Likely cause : LLM proposing too-aggressive splits ; tighten prompt
   eval gates, freeze further promotes, escalate to ML team.
4. Temporarily set `m23_canary_traffic_percent = 0.01` to slow rollouts.

### 5.3 Audit chain HMAC failure

**Symptoms** : `efe-admin audit verify` returns `AuditHmacFailure(<row_id>)`.

**Action** :
1. **CRITICAL** : freeze all promotes (`UPDATE tenant_feature SET m23_curation_enabled=false`).
2. Open a security incident (P1).
3. Investigate : DB row was modified outside the application path?
   Master HMAC key rotated mid-write?
4. Restore from backup if confirmed tampering ; otherwise re-key + replay
   from last known-good chain marker.

### 5.4 LLM provider unavailable

**Symptoms** : `efe-curation` returns `gRPC UNAVAILABLE`.

**Action** :
1. Check LLMRouter status (M21 dashboards).
2. Verify provider chain fallback : if Anthropic down, OpenAI fallback active?
3. Tenants in INTERNAL_LLM mode : tools return `UNAVAILABLE` ; tenant
   should switch to SAMPLING_DELEGATION temporarily.

### 5.5 Budget exceeded mid-day

**Symptoms** : `efe_curation_budget_exceeded_total` > 0 for tenant.

**Action** :
1. Verify `m23_curation_budget_cents_per_day` setting.
2. Communicate with tenant : raise budget OR pause proposals until tomorrow.
3. Daily reconciliation runs at 00:00 UTC.

---

## 6. Disaster recovery

### 6.1 Lost taxonomy_version row

The `anchors_json` is stored in `taxonomy_version`. If a row is lost,
restore from :
1. Postgres point-in-time recovery (PITR).
2. The cosign-signed taxonomy artifact published to GHCR.

```bash
# Pull from GHCR + verify signature
cosign verify ghcr.io/efe-ai/taxonomies/<tenant>/<catalog>:<semver>
docker pull ghcr.io/efe-ai/taxonomies/<tenant>/<catalog>:<semver>
```

### 6.2 Audit chain corruption

Run `efe-admin audit verify <slug>` quarterly as compliance gate. If
broken, the FIRST broken row defines the recovery point.

---

## 7. Compliance handoff

The compliance pack regeneration cron runs at 02:00 UTC nightly. M23
adds 5 files to the pack (cf. § 21.5 of the architecture doc).
Auditors should look for :
- `M23_CURATION_LINEAGE.json` — full provenance per promoted version
- `M23_REVIEWER_LIST.json` — reviewer history
- `M23_LLM_USAGE_REPORT.json` — cost + provider breakdown
- `M23_DRIFT_EVENTS.json` — drift events history
- `M23_PROMPT_REGISTRY.json` — prompt template versions used

---

## 8. References

- Architecture : [`docs/EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`](../EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md)
- Migration : [`crates/efe-server/migrations/0014_taxonomy_curation.sql`](../../crates/efe-server/migrations/0014_taxonomy_curation.sql)
- Roadmap : [`docs/EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md`](../EFE_LLM_INTEGRATION_ROADMAP_M20_M25.md) § 9
- Compliance : [`docs/release/COMPLIANCE_PACKAGE.md`](../release/COMPLIANCE_PACKAGE.md)
- AI Act Art. 14 hooks : M19 production hardening doc § 5
