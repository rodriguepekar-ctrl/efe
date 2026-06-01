# M30 — Known gaps & deferred items

> **Status** : Living document. Updated at the close of the M30 audit
> on `{date}` (see git history). All items are non-blocking for the
> EXPERIMENTAL M30 release ; each is tracked with an owner + target
> release.

## Convention

- **Status `deferred-M30.1`** : already accepted by ADR, scheduled for
  the next dot release.
- **Status `accepted-design`** : intentional ; will not change.
- **Status `audit-resolved`** : was a gap surfaced by the chirurgical
  audit and has been fixed in M30 final.

## Multi-modal plane

| # | Item | Status | Tracking |
|---|---|---|---|
| MM-1 | **gRPC client to Triton** (HTTP only in M30). | `deferred-M30.1` | ADR-M30-003 |
| MM-2 | **Binary tensor attachments** to Triton (M30 sends JSON-encoded f32 widened from u8). | `deferred-M30.1` | ADR-M30-003 |
| MM-3 | **Software audio resampler** (`rubato`) — M30 rejects WAV ≠ 16 kHz. | `deferred-M30.1` | ADR-M30-004 |
| MM-4 | **CLIP-style joint fusion** — `FusionStrategy::ClipJoint` returns `Internal { reason: "clip_joint deferred to M30.1" }`. | `deferred-M30.1` | ADR-M30-007 (referenced in fusion.rs ; ADR file pending) |
| MM-5 | **Real learned linear projection** for `ConcatLinear` — M30 ships a Hadamard-style sign-mix placeholder. | `deferred-M30.1` | fusion.rs |
| MM-6 | **Multi-image batching** in vision encoder. | `deferred-M30.1` | architecture § 5.4 |
| MM-7 | **Streaming ASR** for audio (currently full-file inference). | `deferred-M31` | architecture § 7.3 |
| MM-8 | **WAV < 44 bytes accepted by sniffer**. | `audit-resolved` | `input_guard.rs` (WAV_MIN_HEADER_BYTES) |
| MM-9 | **`fuse()` accepted `target_dim = 0`** (would divide by zero downstream). | `audit-resolved` | `fusion.rs` |
| MM-10 | **`dpia_signed_by` missing** from `TenantModalityPolicy`. | `audit-resolved` | `policy.rs` |

## Multi-region plane

| # | Item | Status | Tracking |
|---|---|---|---|
| RG-1 | **Automatic failover** (consensus-based). | `deferred-M31` | ADR-M30-006 |
| RG-2 | **PG replication-lag gauge had no writer.** | `audit-resolved` | `efe-region::replication_monitor` + `set_replication_lag_secs` |
| RG-3 | **`Quiescer` trait was dead** (no default impl). | `audit-resolved` | `NoopQuiescer` |
| RG-4 | **DR drill harness `failback` always reports `noop`** because the orchestrator's `CurrentRegion` snapshot is captured at construction. | `accepted-design` | `dr_drill.rs` — the harness is a planner only ; the actual flip lives in the REST layer. Documented in the runbook `RB_M30_DR_DRILL` § 5. |
| RG-5 | **Concrete sqlx-backed `RegionApiState` / `LagFetcher` impls** live in the runtime binary, not in this crate. | `accepted-design` | consistent with `events`, `webhooks`, `data_rights`, etc. — pattern established since M27 |
| RG-6 | **`region_failover_event.drill_run_id` had no FK** to `dr_drill_run`. | `audit-resolved` | migration 0024 § 6.bis |

## Infrastructure

| # | Item | Status | Tracking |
|---|---|---|---|
| IN-1 | **Helm NetworkPolicy blocked Prometheus scrape** of the Triton metrics port. | `audit-resolved` | `efe-triton/templates/triton.yaml` — second `from` rule for the monitoring namespace |
| IN-2 | **Terraform default `g5g.8xlarge`** (ARM Graviton + T4G) was incompatible with `ami_type = AL2_x86_64_GPU` and could not host the `1g.10gb` MIG profile. | `audit-resolved` | `terraform/m30_gpu_pool/variables.tf` default → `p4de.24xlarge` + validation rule |
| IN-3 | **Cloudflare Geosteer fallback monitor lacks per-region custom failure thresholds.** | `deferred-M31` | terraform/m30_multiregion/cloudflare_geosteer.tf — Cloudflare API limitation |

## Audit & observability

| # | Item | Status | Tracking |
|---|---|---|---|
| OB-1 | **`multimodal_inference_audit` table is created but no writer is in this crate.** Concrete writer lives in the binary that owns the PG pool. | `accepted-design` | consistent with `agent_audit`, `customer_event_log`, etc. |
| OB-2 | **`region_failover_event` row insert** — same as OB-1. | `accepted-design` | — |
| OB-3 | **`M30RegionHighReplicationLag` alert** would never fire because no code set the gauge. | `audit-resolved` | RG-2 above |

## Test coverage

| # | Item | Status | Tracking |
|---|---|---|---|
| TC-1 | **`v1/multimodal::handlers`** had no router-level test. | `audit-resolved` | 3 smoke tests added in `handlers.rs#tests` |
| TC-2 | **`v1/region::handlers`** had no router-level test. | `audit-resolved` | 5 smoke tests added in `handlers.rs#tests` |
| TC-3 | **`replication_monitor::run_lag_monitor`** had no resilience test. | `audit-resolved` | 1 test : monitor survives a fetcher error, gauge holds prior value |

## Pre-existing failures NOT in M30 scope

| # | Item | Owner |
|---|---|---|
| PRE-1 | `efe-curation::semantic_diff::detect_split_via_centroid_cosine` — M23 algorithm classifies a centroid-cosine split as `Renamed + Added` instead of `Split`. Test contract vs. impl divergence. | M23 follow-up sprint |

## Audit verdict

The M30 surface (≈ 60 files across 4 layers — Rust crates, REST
handlers, SQL, Helm/Terraform) is **operationally complete** for
EXPERIMENTAL release. Every audit-resolved item ships with a unit
test pinning the fix. Deferred items are tracked by ADRs and have
no surface in production until the relevant feature flag is flipped.
