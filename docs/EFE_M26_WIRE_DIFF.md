# EFE M26 Wire Format Diff (vs M25)

**Status** : informative. M26 is a semver MINOR ; all wire changes are
**additive** unless explicitly noted.

---

## REST `/v1/agent/*`

### New endpoints (24-25)

| # | Method | Path |
|---|---|---|
| 22 | POST | `/v1/agent/graduate/check` |
| 23 | POST | `/v1/agent/graduate/confirm` |
| 24 | POST | `/v1/agent/graduate/revoke` |
| 25 | GET | `/v1/agent/post-market/quarterly` |

### New endpoint (streaming)

| # | Method | Path | Notes |
|---|---|---|---|
| (8) | GET | `/v1/agent/runs/:id/steps/stream` | SSE — `Accept: text/event-stream` |

### Existing endpoints (unchanged)

All M25 endpoints retain their request/response shape. New optional
fields are added but legacy clients that ignore unknown fields work as
before.

## CRD `AgentRun` (efe.ai/v1)

### New optional field

```yaml
spec:
  minChannelRequired: STABLE  # NEW : optional, default = EXPERIMENTAL
```

### Status (unchanged)

## CRD `AgentRunTemplate` (efe.ai/v1)

### Status (unchanged ; same fields as M25)

## SQL — `agent_run` table

| Field | M25 | M26 |
|---|---|---|
| `template_id` | `TEXT NOT NULL` (storing slug) | renamed to `template_slug` (same column data) |

A view `agent_run_v25` aliases `template_slug → template_id` for
read-only backward compat. SELECTs against the view continue to work.
INSERTs MUST use the new column name.

## SQL — `tenant_feature` table (additions)

```sql
m26_graduation_acknowledged_at TIMESTAMPTZ
m26_graduation_revoked_at TIMESTAMPTZ
m26_graduation_acknowledgement_hash TEXT
m26_graduation_revoked_reason TEXT
m26_daily_budget_cents BIGINT
m26_monthly_budget_cents BIGINT
```

## SQL — new tables

- `compliance_pack_run` — R-100 lineage.
- `dpo_review` — DPO quarterly review log.
- `ai_act_post_market_report` — Art. 26 quarterly export tracker.
- `pen_test_run` + `pen_test_finding` — pen test execution + findings.
- `m26_runtime_kpi_snapshot` — per-minute KPI snapshots for the
  post-market report.

## SQL — new view

- `v_tenant_channel` — resolves the current channel
  (`DISABLED` / `EXPERIMENTAL` / `STABLE`) per tenant.

## gRPC `efe.agent_runtime.v1`

### Unchanged

The M25 protobuf service stays identical. M26 surfaces are purely REST.
Internal mesh calls (controller ↔ efe-server) keep using gRPC over mTLS.

## Headers added

| Header | Purpose |
|---|---|
| `X-EFE-Tenant` | Sent on every REST call ; redundant with JWT claim but useful for log correlation |
| `X-EFE-User-Email` | Set by portal-admin server actions for audit |
| `X-EFE-Run-Id` | Set by relay → mcp-server / LLM router |
| `X-EFE-Step` | Step index for the call |

## Metric additions (Prometheus)

| Metric | Cardinality |
|---|---|
| `efe_agent_firecracker_api_duration_seconds{api,status}` | low |
| `efe_agent_microvm_leak_recovered_total` | n/a |
| `efe_agent_microvm_reaped_total{tenant}` | tenant cardinality |
| `efe_agent_audit_hmac_verify_failures_total` | n/a (P0 alert) |
| `efe_agent_cost_cap_terminate_duration_seconds` | low |
| `efe_agent_gdpr_cascade_oldest_pending_seconds{tenant}` | tenant cardinality |
| `efe_agent_relay_request_duration_seconds{class,status}` | low |
| `efe_agent_relay_rate_limited_total{class}` | low |

## Helm chart — `values.yaml`

| Key | M25 | M26 |
|---|---|---|
| `admissionWebhook.enabled` | `false` | `true` |
| `vsockRelayer.enabled` | n/a | `true` |
| `vsockRelayer.image.*` | n/a | repo + tag |
| `slo.enabled` | n/a | `true` |
| `alertmanager.enabled` | n/a | `true` |
| `controller.replicas` | `1` | `2` |
| `controller.image.tag` | `"2.0.0"` | `"2.1.0"` |

Old M25 values continue to work ; new keys default to safe values.

## CLI `efe-admin agent`

### New subcommand : `graduate`

- `efe-admin agent graduate check --tenant <s>`
- `efe-admin agent graduate confirm --tenant <s> --acknowledgement-hash sha256:<hex> --confirm`
- `efe-admin agent graduate revoke --tenant <s> --reason "..." --confirm`
- `efe-admin agent graduate status --tenant <s>`

### Existing subcommands

Unchanged.

## Deprecations introduced in M26 (removed in M27)

| Item | Replacement |
|---|---|
| `AgentRun.spec.image` field | use template's image |
| `efe-admin agent template upsert --legacy` flag | (removed) |
| Pre-cosign-Rekor signature verification | always Rekor-verified at boot |
| `agent_run_v25` view | use `agent_run` directly with `template_slug` |
