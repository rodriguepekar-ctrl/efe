# M24 Active Learning Runbook

**Audience** : tenant operators, SRE on-call, compliance team.
**Last reviewed** : 2027-Q3 (M24 GA).

---

## 1. Quick reference

| Task | Command |
|---|---|
| Enable M24 for tenant | `efe-admin tenant enable-m24 <slug>` |
| List queue | `efe-admin learning queue list --catalog <id>` |
| Stats overview | `efe-admin learning stats overview` |
| Confirm proposal | `efe-admin learning proposal confirm <id> --reason "..."` |
| Trigger retune | `efe-admin learning retune trigger --catalog <id>` |
| Promote run | `efe-admin learning promote <run-id>` |
| Rollback | `efe-admin learning rollback <run-id> --reason "..."` |
| GDPR erasure | `efe-admin learning gdpr erase --catalog <id> --content-hashes <h1>,<h2> --reason "..."` |
| Verify HMAC chain | `efe-admin audit verify <slug> --module=active_learning` |

---

## 2. Onboarding tenant to M24

1. **Verify dependencies** :
   - `EFE_FEATURE_M24_ACTIVE_LEARNING_ENABLED=true`
   - `EFE_FEATURE_M21_LLMROUTER=true`
   - `EFE_FEATURE_M23_CURATION_ENABLED=true`

2. **Sign DPO addendum** (M24 specific) — covers :
   - LLM oracle as new automated decision-maker
   - Cascade rebuild on Art. 17 erasure
   - 30-day disclosure of new training data sources

3. **Set tenant flags** :
   ```sql
   UPDATE tenant_feature SET
     m24_active_learning_enabled = true,
     m24_human_review_required = true,        -- recommended default
     m24_auto_promotion_enabled = false,      -- opt-in later if low-stakes
     m24_active_learning_budget_cents_per_day = 1000  -- $10/day
   WHERE id = '<tenant_id>';
   ```

4. **Per-catalog enablement** :
   ```sql
   UPDATE catalog SET
     m24_al_enabled = true,
     m24_stakes_tier = 'MEDIUM'  -- or LOW for auto-promote eligible
   WHERE id = '<catalog_id>' AND tenant_id = '<tenant_id>';
   ```

5. **Verify pipeline** : after 1 hour, sampler should have enqueued items :
   `efe-admin learning queue list --catalog <id>`.

---

## 3. Daily operator workflow

1. **Check queue** : `/<slug>/learning/queue` — review pending labels.
2. **For each AWAITING_REVIEW** : confirm / edit / reject with rationale ≥ 10 chars.
3. **Watch retunes** : `/<slug>/learning/retunes` — flagged-for-promotion runs need approval.
4. **Promote when satisfied** : canary 24h auto-rollback if regress.

---

## 4. SLA targets

| Tier | Review SLA P99 | Notify after | Retune cadence |
|---|:---:|:---:|:---:|
| Enterprise | 24 h | 4 h | Weekly |
| Pro | 48 h | 8 h | Weekly |
| Community | 7 d | 24 h | Bi-weekly |

PagerDuty escalates if a HIGH-urgency GDPR Art. 17 request unacknowledged > 4 h.

---

## 5. Common incidents

### 5.1 Queue back-pressure (capacity reached)

**Symptoms** : `efe_al_queue_back_pressure_total` increments.

**Action** :
1. `efe-admin learning queue stats` — check counts by status.
2. If `awaiting_review` > 50 — reviewer fatigue ; rotate reviewers.
3. If `failed` > 10 — LLM provider issue ; check M21 status.
4. If `labeling` stuck > 1h — worker stuck ; restart efe-server.

### 5.2 Centroid drift abort

**Symptoms** : `efe_al_centroid_drift_aborted_total` > 0 ; retune status ABORTED.

**Action** :
1. **CRITICAL** : freeze the affected catalog.
2. Open security incident (P1).
3. Investigate gold dataset for poisoning :
   ```sql
   SELECT id, text, expected_class_id, source, created_at
   FROM gold_example WHERE created_at > NOW() - INTERVAL '7 days'
     AND gold_dataset_id IN (
       SELECT id FROM gold_dataset
       WHERE tenant_id = '<tenant>' AND catalog_id = '<catalog>'
     );
   ```
4. Tombstone suspicious rows.
5. Force-retune : `efe-admin learning retune trigger --catalog <id>`.

### 5.3 Promotion auto-rollback storm

**Symptoms** : 3+ canary auto-rollbacks in 24 h.

**Action** :
1. Inspect last retune `eval_metrics_json` for accuracy regressions.
2. Likely : adversarial pattern in production traffic OR LLM oracle drift.
3. Reduce `m24_canary_traffic_percent` to 1 % temporarily.
4. Pause retune scheduler : `UPDATE catalog SET m24_al_enabled = false`.

### 5.4 GDPR Art. 17 SLA breach risk

**Symptoms** : pending request > 80 % SLA consumed.

**Action** :
1. Verify rebuild_request status :
   `SELECT * FROM bundle_rebuild_request WHERE status IN ('PENDING','IN_PROGRESS')`.
2. If catastrophic urgency : trigger force-retune immediately.
3. Notify DPO + legal team.

### 5.5 Audit chain HMAC failure

**Symptoms** : `efe-admin audit verify` returns `AuditHmacFailure`.

**Action** :
1. **CRITICAL P0** : freeze all M24 activity (server flag OFF).
2. Open security incident.
3. Investigate : DB row modified outside the application path?
4. Restore from backup if confirmed tampering ; otherwise re-key + replay.

---

## 6. Disaster recovery

### 6.1 Lost gold_label_proposal

`gold_label_proposal` is reconstructible from `llm_call_audit` (the
LLM call audit captures input + output). PITR Postgres restore.

### 6.2 Lost retune run + bundle artifact

The bundle artifact is cosign-signed and stored in GHCR. Pull from
registry, verify signature, re-import.

### 6.3 Lost holdout split

Holdout is hash-based deterministic — recalculable from gold_example
table at any time (no state needed).

---

## 7. Compliance handoff

The compliance pack regen cron (R-100) adds for M24 :
- `M24_LINEAGE.json` — full lineage per promoted bundle
- `M24_GDPR_ART17_LOG.json` — Art. 17 requests + completion timing
- `M24_DPO_CONSENT.json` — current consent status + history
- `M24_RETUNE_HISTORY.json` — last 90 days

External auditors verify via Rekor (cosign signatures) + lineage query
(§ 26.3 of the architecture doc).

---

## 8. References

- Architecture : [`docs/EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md`](../EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md)
- Migration : [`crates/efe-server/migrations/0015_active_learning.sql`](../../crates/efe-server/migrations/0015_active_learning.sql)
- M23 runbook (curation) : [`M23_CURATION_RUNBOOK.md`](M23_CURATION_RUNBOOK.md)
- Compliance : [`docs/release/COMPLIANCE_PACKAGE.md`](../release/COMPLIANCE_PACKAGE.md)
- AI Act Art. 11/14/15 hooks : architecture doc § 26
