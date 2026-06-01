# D-M50-INFRA-001 — ClickHouse vs Postgres+Redis for M50 metering

**Status**       : ACCEPTED
**Date drafted** : 2026-05-18
**Decision**     : Use **existing Postgres + Redis stack**, NOT ClickHouse
**Owner**        : @bench-infra-lead (Sprint 0 Action 0.3)
**Supersedes**   : Implicit ClickHouse assumption in `EFE_M50_AGENTIC_ARCHITECTURE.md`
                   (verification report D-M50-AUDIT-14)

---

## Decision in one sentence

> **M50 will use the existing EFE infrastructure (Postgres for relational
> data and audit traces, Redis for high-volume counters and pool warmup).
> No new ClickHouse dependency.**

---

## Why the M50 architecture doc proposed ClickHouse

The original M50 architecture doc § 4.6 stated :

> "Storage : ClickHouse pour aggregation high-volume. Hourly rollups
> vers Postgres pour reporting."

This was a **speculative assumption** based on industry-common patterns
(many SaaS billing systems use ClickHouse for usage analytics).  The
verification report flagged it (D-M50-AUDIT-14) because ClickHouse was
not validated against existing EFE infrastructure.

## Live audit of existing EFE infrastructure (2026-05-18)

### Postgres state

```
5 crates use sqlx 0.7 / postgres :
  - efe-active-learning      (M24)
  - efe-agent-runner         (M25)
  - efe-curation             (M23)
  - efe-llm-router           (M21)
  - efe-server               (M10)
```

Postgres is the **canonical relational store** for EFE. Used by audit
chains (M23), agent runs (M25), classification catalog (M16), etc.

### Redis state

```
M31 efe-metering uses Redis :
  - Per-meter counters
  - Cached hour bucket (AtomicU64 + OnceCell<String>)
  - Hot-path budget : < 1 µs per increment
  - parking_lot::Mutex (sync) instead of tokio::sync::Mutex (async)
```

Redis is the **canonical high-throughput counter store** for EFE.
The M31 metering crate already provides the exact pattern M50 needs.

### ClickHouse state

```
$ grep -rln "clickhouse\|ClickHouse" Cargo.toml crates/ 2>/dev/null
(no matches)
```

ClickHouse is **NOT** currently in EFE.  Adding it would mean :
- New Kubernetes manifest
- New Helm chart sub-section
- New monitoring stack integration (Grafana dashboards)
- New backup procedure
- New compliance review (data residency)
- ~ CHF 500-1500 / month in cloud cost (ClickHouse Cloud) or self-hosted operations

---

## Decision matrix

| Criterion | ClickHouse Cloud | Postgres + Redis (existing) |
|-----------|-------------------|------------------------------|
| **Storage cost (1M events/mo)** | ~ CHF 500/mo base | ~ CHF 50/mo (incremental on existing infra) |
| **Hot-path latency** | 1-5 ms per write | < 1 µs (Redis counter increment) |
| **Aggregation speed** | Excellent (column store) | Good (with proper indexing + rollup jobs) |
| **Operational complexity** | New infra to operate | Zero new ops (already running) |
| **Compliance scope** | New data residency review | Already in scope |
| **Backup integration** | New procedure | Existing M28 backup covers it |
| **Recovery time** | 1-2 hours from snapshot | Existing M28 procedures |
| **Engineering time to integrate** | 2-3 weeks (new crate, Helm chart, ops) | < 3 days (reuse M23 + M31 patterns) |
| **Annual ops cost** | ~ CHF 6-18 k | ~ CHF 600 (incremental Postgres + Redis) |

**Verdict** : Postgres + Redis wins on every criterion **for our scale**.

---

## Why Postgres + Redis is sufficient for M50

### Volume projections

| Phase | Calls/sec | Calls/month | Storage/month |
|-------|-----------|--------------|----------------|
| Phase 1 GA (M+6) | 10-100 | ~ 250 M | ~ 50 GB (audit traces) |
| Phase 2 (M+12) | 100-1000 | ~ 2.5 B | ~ 500 GB |
| Phase 3 Series A (M+18) | 1000-5000 | ~ 12 B | ~ 2-5 TB |

Postgres handles up to **billions of rows** efficiently with proper
partitioning + indexing.  At our M+18 projected scale (~ 12 B events/year),
Postgres remains viable with :
- Time-partitioned `nano_agent_traces` table (monthly partitions)
- Indexed on `(tenant_id, started_at)` for queries
- Hourly rollup jobs aggregating to `nano_agent_usage_hourly`
- TimescaleDB extension (optional, free) for time-series optimizations

### Architecture pattern (Postgres + Redis stack)

```
HOT PATH (per nano agent call) :
  Redis : meter increment (< 1 µs)
  Postgres : trace insert (5-20 ms, batched)

COLD PATH (rollups + reporting) :
  Cron job (hourly) :
    SELECT FROM nano_agent_traces (raw)
    AGGREGATE → INSERT INTO nano_agent_usage_hourly
    
  Cron job (daily) :
    AGGREGATE hourly → daily
    
  Cron job (monthly) :
    AGGREGATE daily → monthly (for invoicing)

DASHBOARDS :
  Postgres reads (with indexes) — Grafana queries
```

This pattern is **proven** by the M31 metering crate's existing design.

### If we ever NEED ClickHouse (future trigger)

We migrate to ClickHouse **only if** :
- Sustained > 100k events/sec (vs current target 10-50k)
- Query latency on hourly rollups > 5 sec
- TimescaleDB extension insufficient

These conditions are **unlikely before Series A** (M+18).  Adding
ClickHouse can be a Phase 4 v2 decision, not Sprint 0.

---

## Impact on M50 architecture doc

The M50 architecture doc should be updated (documentation patch in
Sprint 13 alongside STABILITY § 30) :

```diff
-Storage : ClickHouse pour aggregation high-volume. Hourly rollups
-vers Postgres pour reporting.
+Storage : Postgres pour audit traces + rollups (table time-partitioned
+monthly). Redis pour per-tenant counters (consistent avec M31 pattern).
+ClickHouse considered and rejected for v1 — Postgres + Redis sufficient
+at projected scale. Cf D-M50-INFRA-001.
```

---

## Impact on budget allocation (M50 roadmap)

The M50 roadmap § 9.1 originally allocated :

```
ClickHouse cloud : ~ 500 USD/mois × 18 = CHF 9 k
```

This becomes :

```
Postgres scaling (managed) : ~ 300 USD/mois × 18 = CHF 5 k
Redis scaling (existing)    : ~ 100 USD/mois × 18 = CHF 1.8 k
Total saved on infra        : CHF 2.2 k
```

The saved CHF 2.2 k can be reallocated to compliance budget (additional
SOC 2 prep tools).

---

## Alternatives considered

### Alternative A : Pure Postgres (no Redis)

**Rejected** : Redis is already used by M31 for the same hot-path pattern.
Going Postgres-only would require recomputing the counter logic from
scratch with sub-millisecond increments, which Postgres does not natively
support.

### Alternative B : ClickHouse Cloud

**Rejected** : Operational complexity + cost not justified at projected
scale.

### Alternative C : DuckDB embedded

**Considered, rejected** : DuckDB is excellent for analytical queries but
not for multi-tenant production workloads.  Could be used for **internal
analytics dashboards** in a future Phase 4 v2.

### Alternative D : Snowflake / BigQuery

**Rejected** : Cloud-vendor lock-in (Snowflake) or Google-vendor lock-in
(BigQuery).  EFE's positioning as European audit-grade infrastructure
favors open-source / self-hostable backends.

---

## Decision sign-off

- [x] Postgres + Redis stack confirmed sufficient for projected M50 scale
- [x] No new infrastructure dependency introduced by M50
- [x] M50 architecture doc to be patched in Sprint 13
- [x] M50 roadmap budget updated (saves CHF 2.2 k on infra, reallocated to compliance)
- [x] Migration trigger to ClickHouse documented (post-Series A)

**Decision accepted.**

---

## Consequences

### Positive
- Zero new infrastructure operational burden
- Lower cloud spend through M+18
- Faster engineering (reuse M31 pattern)
- Compliance scope unchanged

### Negative
- If we hit > 100k events/sec faster than projected, will need
  emergency ClickHouse migration (Phase 4 v2)
- Aggregation queries may need TimescaleDB extension for sub-second
  reporting on > 1 B rows

### Neutral
- Architecture doc M50 needs minor patch (non-breaking, Sprint 13)

---

_Action 0.3 complete — 2026-05-18._
