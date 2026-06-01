# EFE A1 — Roadmap (Q1 2026 → Q4 2027)

> **Document type** : Multi-phase roadmap for the `efe-meta-swarm`
> hierarchical multi-swarm coordinator.
> **Status** : Phase 1 ✅ shipped ; Phase 2 architecture frozen ;
> Phases 3-5 outlined.
> **Owners** : meta-swarm-team (lead) ; sre-team (operations) ;
> platform-team (deployment) ; sec-team (security review) ;
> product-team (Phase 5 SLA).
> **Last refreshed** : 2026-05-13.
> **Cadence** : refreshed at every Phase Gate (≤ L5).

---

## Table of contents

0. [Executive summary](#0-executive-summary)
1. [Maturity ladder L0 → L5](#1-maturity-ladder)
2. [Phase landscape (visual)](#2-phase-landscape)
3. [Phase 0 — Pre-A1 design (Jan-Feb 2026)](#3-phase-0--pre-a1-design)
4. [Phase 1 — In-process library (Mar-May 2026) ✅](#4-phase-1--in-process-library--shipped)
5. [Phase 2 — gRPC transport (Jul-Aug 2026)](#5-phase-2--grpc-transport)
6. [Phase 3 — Multi-tenant + cross-region (Oct-Dec 2026)](#6-phase-3--multi-tenant--cross-region)
7. [Phase 4 — Distributed consensus / Raft (Jan-Mar 2027)](#7-phase-4--distributed-consensus--raft)
8. [Phase 5 — Customer-facing SLA (Apr-Jun 2027)](#8-phase-5--customer-facing-sla)
9. [Cumulative deliverables](#9-cumulative-deliverables)
10. [Gates L0 → L5 — formal criteria](#10-gates-summary)
11. [Cross-phase dependencies](#11-cross-phase-dependencies)
12. [Critical path](#12-critical-path)
13. [Risk register (cross-phase)](#13-risk-register)
14. [Resource allocation (FTE-weeks)](#14-resource-allocation)
15. [Decision log + pivot points](#15-decision-log)
16. [Communication plan](#16-communication-plan)
17. [Open questions per phase](#17-open-questions)

---

## 0. Executive summary

```
                         L0    L1    L2    L3    L4    L5
                          │     │     │     │     │     │
Phase 0 — Pre-A1 design   │─────┤     │     │     │     │
Phase 1 — Library        ❘─────┘─────┤     │     │     │  ← we are here
Phase 2 — gRPC transport             │─────┤     │     │
Phase 3 — Multi-tenant               │     │─────┤     │
Phase 4 — Raft consensus             │     │     │─────┤
Phase 5 — SLA public                 │     │     │     │─────┐
                                                            GA
                                                            │
                       Jan'26 May'26 Aug'26 Dec'26 Mar'27 Jun'27 …
```

**Today (2026-05-13)** : L1 attained, Phase 1 shipped (PR #1-#4 +
bench tracking master-gate hook).  Phase 2 architecture frozen
([`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md)).

**Next milestone** : Phase 2 L2 graduation, target **2026-08-31**.

**End state** : Phase 5 customer-facing SLA, target **2027-06-30**.
By that date, `efe-meta-swarm` is a productised coordination layer
with a 99.95 % monthly availability SLA and ≥ 3 paying tenants on
multi-region deployments.

### Headline metrics at end of roadmap (2027-06-30)

- **619 → 1 200+** unit/integration tests on the meta-swarm path.
- **7 → 25+** opposable SLOs.
- **1 → 5** master rollups (SLO-OPS-A1-META-{SWARM, GRPC, MULTI-TENANT, RAFT, SLA}).
- **0 → 99.95 %** availability SLA.
- **1 → N** tenants with isolated meta-swarm instances.

---

## 1. Maturity ladder

The 6 levels are **opposable** — each has a single line that names
what it makes possible, and a single line of doctrine that is true at
that level.

| Level | Name | Capability unlocked | Doctrine guarantee |
|---|---|---|---|
| L0 | Design | Architecture frozen, threats catalogued. | We know what we are building and why. |
| L1 | Library | `MetaSwarm::iterate()` works in-process with deterministic, contractive semantics. | Bit-identical on reruns, 47-iter convergence on Kovant. |
| **L2** | **Transport** | **Multi-process gRPC; SIGHUP hot reload; Helm sub-chart.** | **Internal EFE-team production, 7-day soak clean.** |
| L3 | Multi-tenant | Per-tenant isolation, cross-region replication. | One tenant's meta-swarm never starves another's. |
| L4 | Consensus | Raft-based controller HA ; ≥ 5 replicas tolerate 2 failures. | No single point of coordinator failure. |
| L5 | SLA | Customer-facing SLA 99.95 %, public status page. | Refund if we miss the SLA. |

The transitions L*n* → L*n+1* are **gated by an opposable checklist**
of 5-10 criteria (cf. § 10).  Each gate is binary — passing it is
non-negotiable ; failing it defers the next phase by one sprint.

---

## 2. Phase landscape

### 2.1 Big-picture Gantt (compact)

```
        2026                            2027
        Q1   |  Q2   |  Q3   |  Q4   |  Q1   |  Q2   |  Q3
       ─────┼───────┼───────┼───────┼───────┼───────┼───────
P0 ▇▇▇▇▇▇▇  │       │       │       │       │       │
P1     ▇▇▇▇▇▇▇▇▇▇▇▇│       │       │       │       │
P2          │       │▇▇▇▇▇▇▇        │       │       │
P3          │       │       │▇▇▇▇▇▇▇│       │       │
P4          │       │       │       │▇▇▇▇▇▇▇│       │
P5          │       │       │       │       │▇▇▇▇▇▇▇│
        ─────┼───────┼───────┼───────┼───────┼───────┼───────
gate   L0    L1     L2      L3       L4       L5
```

### 2.2 Detailed phase windows

| Phase | Window | Weeks | Active sprints | Gate at end |
|---|---|---:|---:|---|
| P0 — Design | 2026-01-06 → 2026-02-28 | 8 | 4 | L0 |
| P1 — Library | 2026-03-02 → 2026-05-15 | 11 | 5 | L1 ✅ (2026-05-13) |
| (P1 → P2 lag) | 2026-05-16 → 2026-07-05 | 7 | 3 | — (slack, hardening, vacation) |
| P2 — gRPC | 2026-07-06 → 2026-08-30 | 8 | 4 | L2 (2026-08-31) |
| (P2 → P3 lag) | 2026-09-01 → 2026-10-04 | 5 | 2 | — |
| P3 — Multi-tenant | 2026-10-05 → 2026-12-20 | 11 | 5 | L3 (2026-12-22) |
| (P3 → P4 lag) | 2026-12-21 → 2027-01-11 | 3 | 1 | — (holidays) |
| P4 — Raft | 2027-01-12 → 2027-03-27 | 11 | 5 | L4 (2027-03-30) |
| (P4 → P5 lag) | 2027-03-28 → 2027-04-11 | 2 | 1 | — |
| P5 — SLA | 2027-04-12 → 2027-06-26 | 11 | 5 | L5 (2027-06-30) |

Total roadmap : **77 weeks** of active work + **17 weeks** of slack
across 18 months (Jan 2026 → Jun 2027).

---

## 3. Phase 0 — Pre-A1 design

**Window** : 2026-01-06 → 2026-02-28 (8 weeks, ✅ done).
**Gate**  : L0 — design frozen 2026-02-26.

### 3.1 Deliverables

| # | Artifact | Status |
|---|---|---|
| D0.1 | `docs/EFE_A1_PHASE1_ARCHITECTURE.md` | ✅ |
| D0.2 | Math derivation : contractive fixed-point, joint residual | ✅ |
| D0.3 | Public API freeze (`MetaSwarm`, `SwarmParticipant`, `MetaSpace`) | ✅ |
| D0.4 | Threat model v0 | ✅ |
| D0.5 | License carve-out R5-B1 (`UNLICENSED` until Phase 5) | ✅ |
| D0.6 | Initial SLO budget : 7 surface SLOs identified | ✅ |

### 3.2 L0 gate (passed 2026-02-26)

1. ✅ Public API has 0 `pub fn` that would break in Phase 2-5.
2. ✅ Fixed-point math reviewed by 2 senior engineers.
3. ✅ Threat model has 0 SEV-1 unmitigated entries.
4. ✅ Carve-out R5-B1 ratified by legal.

---

## 4. Phase 1 — In-process library  (shipped)

**Window** : 2026-03-02 → 2026-05-15 (11 weeks).
**Gate**  : L1 — bench-tracking master-gate hook merged 2026-05-13.

### 4.1 PRs shipped

| PR | Title | Merged |
|---|---|---|
| #1 | `feat(efe-meta-swarm): scaffold hierarchical multi-swarm crate` | 2026-03-20 |
| #2 | `feat(efe-meta-swarm): LocalParticipant<S: efe_core::Swarm> adapter` | 2026-04-03 |
| #3 | `test(efe-meta-swarm): 130+ proptest cases reaching the 200 target` | 2026-04-28 |
| #4 | `feat(efe-meta-swarm): bench baseline tracking + master-gate hook` | 2026-05-13 |
| #5 | `docs: Phase-1 end-of-sprint progress report` (planned) | 2026-05-15 |

### 4.2 Numbers at L1

```
Tests passing            : 619    (199 default + 209 attention-weighted + 202 core-integration + 8 smoke + 1 doc tests)
Surface SLOs A1          :   7    (LATENCY-P99/P50, CONVERGENCE-ITERS, RESIDUAL-FINAL, DETERMINISM, NON-REGRESSION, MEMORY-PEAK-MB)
Master rollups           :  19    (18 pre-existing + 1 SLO-OPS-A1-META-SWARM)
LOC efe-meta-swarm       : ~4 500
LOC tests/proptests      : ~6 000
Bench (Kovant, 4-vCPU)   :  12 ms p99 / 47 iters / residual 9.7e-5
Determinism              : ✅ bit-identical 3 reruns
```

### 4.3 L1 gate (passed 2026-05-13)

1. ✅ Bench `three_swarms_kovant_latency` produces 7 SLO values, all within budget.
2. ✅ Master rollup `SLO-OPS-A1-META-SWARM` exists, status ACTIVE.
3. ✅ Doctrine anti-Φ-D.A1 respected : 0 SILENT-SKIP on `ops-master-gate.yml`.
4. ✅ Determinism : 3 reruns produce bit-identical `(iters, residual.to_bits)`.
5. ✅ Cargo `default`, `attention-weighted`, `core-integration` feature sets compile + test.
6. ✅ efe-server / efe-core non-regression : 0 touched files in `crates/efe-server/`.

### 4.4 Post-L1 hardening (2026-05-16 → 2026-07-05, slack window)

- Soft-launch monitoring : 3 main runs PASS confirm baseline stability.
- Flip `continue-on-error: true → false` on `bench-meta-swarm-a1` job + "A1 meta-swarm master gate" step.
- Remove the 7 `deferred_ticket: DT-A1-META-SOFT-LAUNCH-3-RUNS` rows from `bench_baselines/v0.10.0/meta_swarm_slos.yaml`.
- Remove the smoke test `baseline_yaml_soft_launch_dt_present`.
- Conduct Phase-2 design review with sec-team + platform-team.

---

## 5. Phase 2 — gRPC transport

**Window** : 2026-07-06 → 2026-08-30 (8 weeks, 4 sprints).
**Gate**  : L2 — multi-process production, target 2026-08-31.
**Full architecture** : [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md).

### 5.1 Sprint-by-sprint plan

| Sprint | Dates | Theme | PRs |
|---|---|---|---|
| A1.P2.1 | 07-06 → 07-19 | Protos + bare gRPC servers | #5-#10 |
| A1.P2.2 | 07-20 → 08-02 | env var + SIGHUP + Helm scaffold | #11-#16 |
| A1.P2.3 | 08-03 → 08-16 | Observability + start of 7-day soak | #17-#21 |
| A1.P2.4 | 08-17 → 08-30 | L2 graduation | #22-#25 |

### 5.2 Headline deliverables

- **2 new crates** : `efe-meta-swarm-proto`, `efe-meta-swarm-server`
- **2 gRPC services** : `MetaSwarmService` (7 RPCs), `MetaParticipantService` (6 RPCs)
- **3 topologies** : T1 in-process / T2 single-node-3-processes / T3 K8s pod-per-participant
- **18 env vars** + SIGHUP atomic hot-reload (≤ 200 ms p99)
- **Helm sub-chart** `efe-meta-swarm` default-disabled
- **14 Prometheus metrics + 8 OTel span types** + 2 Grafana dashboards + 4 alert rules
- **7-day internal soak** with 10 pass criteria
- **Public bench** `meta-swarm vs mono-swarm` (5 workloads × 3 modes)

### 5.3 L2 gate (target 2026-08-31)

10 criteria (G1-G10, detailed in [Phase 2 architecture § 12](EFE_A1_PHASE2_ARCHITECTURE.md#12-gate-2--3-l2-attained-criteria)).  Headline :

- 7 surface A1 + 4 surface A2 SLOs GREEN on 7/7 nightly runs.
- Master rollups #19 + #20 both PASS on 7/7 nightly runs.
- 7-day soak passes all 10 criteria.
- Public bench published.
- Helm chart lint passes.
- Security review signed off.
- Runbook + 2 SRE rehearsals.

### 5.4 Numbers expected at L2

```
Tests passing            : ~ 850    (+ ~230 from Phase 2)
Surface SLOs A1+A2       :  11      (+ 4 Phase 2)
Master rollups           :  20      (+ 1 SLO-OPS-A1-META-GRPC)
LOC efe-meta-swarm*      : ~ 9 000  (+ 2 new crates)
Phase 2 p99 (4-vCPU)     :  ≤ 100 ms (Phase 1 ~12 ms + gRPC overhead)
Soak incidents Sev1/2    :   0
```

---

## 6. Phase 3 — Multi-tenant + cross-region

**Window** : 2026-10-05 → 2026-12-20 (11 weeks, 5 sprints).
**Gate**  : L3 — multi-tenant isolation, target 2026-12-22.
**Architecture** : `docs/EFE_A1_PHASE3_ARCHITECTURE.md` (to be drafted Aug 2026).

### 6.1 Scope (planned)

1. **Per-tenant isolation** : each tenant gets a dedicated meta-swarm
   instance, identified by a `tenant_id` claim in the JWT.  Resource
   quotas (cpu/mem/iters-per-min) enforced via K8s ResourceQuotas +
   token-bucket at the controller.
2. **Cross-region replication** : meta-swarm controllers in 2 regions
   (us-east-1, eu-west-1).  Active-active for read RPCs (`Health`,
   `Info`), active-passive for write RPCs (`Iterate`, `ReloadConfig`).
   Region selection via geo-aware DNS.
3. **Persistent state** : optional `PersistentMetaSwarm` that
   checkpoints the meta-consensus every N iterations to S3.  Allows
   warm-start after pod restart.
4. **f64 wire format option** : tenants that need higher precision opt
   in via the `IterationConfig.precision = F64` field.  Trades 2×
   bandwidth for numerical headroom.
5. **Auto-rotation of mTLS certs** : Phase-2 manual rotation → cert-manager
   reconciles every 30 days.
6. **SLO additions** :
   - `SLO-A3-TENANT-ISOLATION-P99` : 99-th percentile of cross-tenant
     interference (measured by latency variance per tenant).
   - `SLO-A3-REGION-FAILOVER-P99` : ≤ 30 s to fail over between regions.
   - `SLO-A3-CHECKPOINT-LAG-P99` : ≤ 5 s lag between in-memory state
     and S3 checkpoint.

### 6.2 New crates / modules

```
crates/
├── efe-meta-swarm-tenant/        # NEW — tenant resolver + quotas
│   ├── src/jwt_claims.rs
│   ├── src/quotas.rs
│   └── src/registry.rs
└── efe-meta-swarm-server/
    └── src/
        ├── persistence/
        │   ├── s3_checkpoint.rs    # NEW
        │   └── warm_start.rs       # NEW
        └── region/
            ├── geo_dns.rs           # NEW
            └── failover.rs          # NEW
```

### 6.3 L3 gate (target 2026-12-22)

| # | Criterion |
|---|---|
| L3-G1 | 3 tenant configurations run concurrently with measured isolation ≥ 99 %. |
| L3-G2 | Cross-region failover drill : us-east-1 → eu-west-1 in ≤ 30 s. |
| L3-G3 | Warm-start from S3 checkpoint resumes within 10 s. |
| L3-G4 | All Phase-1 + Phase-2 + Phase-3 SLOs GREEN on 14 consecutive nightly runs (2 weeks). |
| L3-G5 | Auto-rotation of mTLS works (sample : 5 rotations observed in soak). |
| L3-G6 | f64 mode produces correct results on the precision test suite. |
| L3-G7 | Tenant quota enforcement : DoS test with one tenant attempting 10 000 req/s does not affect other tenants' p99. |
| L3-G8 | Updated threat model `PHASE3_THREAT_MODEL.md` signed off. |

### 6.4 Numbers expected at L3

```
Tests passing            : ~ 1 100
Surface SLOs A1+A2+A3    :  14
Master rollups           :  21    (+ 1 SLO-OPS-A1-META-MULTI-TENANT)
LOC efe-meta-swarm*      : ~ 13 000
Tenants concurrent       :  3     (test : 10 for stress)
Regions                  :  2
```

---

## 7. Phase 4 — Distributed consensus / Raft

**Window** : 2027-01-12 → 2027-03-27 (11 weeks, 5 sprints).
**Gate**  : L4 — Raft-based controller HA, target 2027-03-30.

### 7.1 Scope (planned)

1. **Raft consensus on the controller layer** : replace Phase-2's
   K8s-lease-based active-passive with a real Raft implementation
   (`raft-rs` or in-house).  ≥ 5 controller replicas tolerate ≤ 2
   simultaneous failures.
2. **Log-shipping of `ReloadConfig`** : every config change is a Raft
   log entry, applied in order on every replica.  Eliminates
   split-brain.
3. **Snapshot compaction** : every 10 000 entries the leader snapshots
   the manifest + meta-consensus.  Snapshots stored in the same S3
   bucket as Phase-3 checkpoints.
4. **Quorum-replicated `Iterate` results** : optional ; tenants can opt
   in to "every Iterate result is replicated to ≥ ⌈N/2⌉ followers
   before returning".  Trades latency for durability.
5. **SLO additions** :
   - `SLO-A4-RAFT-COMMIT-P99` : commit latency ≤ 50 ms.
   - `SLO-A4-LEADER-ELECTION-P99` : new leader within 5 s of old leader failure.
   - `SLO-A4-SNAPSHOT-LAG-P99` : snapshot age ≤ 10 000 entries.

### 7.2 Risks specific to Phase 4

- **Risk** : Raft adds 2-3× latency on the write path.  **Mitigation** :
  the bulk of traffic (`Iterate`) is read-only relative to the controller's
  manifest state — only `ReloadConfig` goes through Raft.
- **Risk** : implementation complexity → bugs.  **Mitigation** : use
  `raft-rs` (battle-tested in TiKV), not a roll-your-own implementation.

### 7.3 L4 gate (target 2027-03-30)

| # | Criterion |
|---|---|
| L4-G1 | Chaos drill : kill 2 of 5 controllers ; cluster recovers within 5 s ; no `Iterate` failures. |
| L4-G2 | Split-brain test : network partition between 2 controllers ; both lose quorum ; cluster blocks correctly. |
| L4-G3 | Snapshot + restore : kill all controllers ; restart from snapshot ; cluster resumes in ≤ 30 s. |
| L4-G4 | 14-day soak with 3 simulated leader failures per day. |
| L4-G5 | All Phase 1-4 SLOs GREEN on 14 consecutive nightly runs. |
| L4-G6 | Documentation `docs/runbooks/META_SWARM_RAFT.md` published. |

### 7.4 Numbers expected at L4

```
Tests passing            : ~ 1 350
Surface SLOs A1..A4      :  17
Master rollups           :  22
Controller replicas      :  5     (Raft quorum = 3)
Failure tolerance        :  2     (any 2 of 5 can fail without service loss)
```

---

## 8. Phase 5 — Customer-facing SLA

**Window** : 2027-04-12 → 2027-06-26 (11 weeks, 5 sprints).
**Gate**  : L5 — public SLA + status page + first paying tenant, target 2027-06-30.

### 8.1 Scope (planned)

1. **License flip** : `efe-meta-swarm` crate moves from `UNLICENSED`
   to **Apache 2.0** (carve-out R5-B1 of
   `EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` activates).
2. **Customer-facing SLA** : 99.95 % monthly availability with refund
   schedule.  Tracked by Phase-3's `SLO-A3-TENANT-ISOLATION-P99` +
   Phase-2's `SLO-A1-META-LATENCY-P99` rolled up into a customer-
   visible compound `SLA-A5-AVAILABILITY-30D`.
3. **Public status page** : `status.efe.io/meta-swarm` shows per-region,
   per-tenant SLA.  Updated every 60 s from the Prometheus pipeline.
4. **Postmortem velocity SLA** : Sev1/2 postmortems within 48 h
   (audited by `postmortem_velocity_audit.py`).  Already exists for the
   workspace ; Phase 5 extends it to the meta-swarm surface.
5. **First paying tenant** : ≥ 1 tenant signs a contract referencing
   `SLA-A5-AVAILABILITY-30D` and starts producing real traffic.
6. **SLO additions** :
   - `SLA-A5-AVAILABILITY-30D` : 99.95 % rolling 30-day window.
   - `SLA-A5-BREACH-CREDIT-OWED` : USD owed in service credits if SLA missed.

### 8.2 L5 gate (target 2027-06-30)

| # | Criterion |
|---|---|
| L5-G1 | License flipped to Apache 2.0 ; legal sign-off recorded. |
| L5-G2 | Status page live, scraped by an external monitor (e.g. UptimeRobot) for cross-validation. |
| L5-G3 | First paying tenant signed ; their traffic visible in dashboards. |
| L5-G4 | SLA breach simulation : inject a 30-min outage ; verify status page reflects it within 60 s ; verify credit math is correct. |
| L5-G5 | 30 consecutive days of measured 99.95 %+ availability prior to L5. |
| L5-G6 | All Phase 1-5 SLOs GREEN on 30 consecutive nightly runs. |
| L5-G7 | Customer-facing documentation (`docs/customer/META_SWARM_INTEGRATION.md`) complete. |
| L5-G8 | Support runbook (`docs/runbooks/META_SWARM_CUSTOMER_SUPPORT.md`) ; on-call rotation includes meta-swarm. |

### 8.3 Numbers expected at L5

```
Tests passing            : ~ 1 500
Surface SLOs total       :  19+
Customer-facing SLAs     :   2     (availability + breach credit)
Master rollups           :  23
Tenants paying           :   ≥ 1
Regions                  :   ≥ 2
Availability             :   99.95 % monthly (SLA-A5)
```

---

## 9. Cumulative deliverables

A condensed view, what exists at the end of each phase :

| Artifact | L0 | L1 | L2 | L3 | L4 | L5 |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| `crates/efe-meta-swarm` (lib) | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `crates/efe-meta-swarm-proto` | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ |
| `crates/efe-meta-swarm-server` | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ |
| `crates/efe-meta-swarm-tenant` | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| `charts/efe/charts/efe-meta-swarm` | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ |
| 7-day soak passed | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ |
| 14-day soak passed | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| 30-day prod observation | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
| Public bench published | ❌ | ❌ | ✅ | ✅ | ✅ | ✅ |
| mTLS auto-rotation | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| Multi-region failover | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ |
| Raft controller HA | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ |
| Customer-facing SLA | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
| Apache-2.0 license | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
| Status page | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |
| ≥ 1 paying tenant | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ |

---

## 10. Gates summary

Each gate is **opposable** : every criterion has a single verification
artifact and a single owner.  No criterion is "soft" — failing one
defers the gate.

| Gate | Criteria | Pass = | Owner |
|---|:---:|---|---|
| L0 | 4 | Design frozen, API safe for L1-L5 | meta-swarm-team |
| **L1** | **6** | **In-process library production-ready, 7 SLOs GREEN** | **meta-swarm-team (✅ 2026-05-13)** |
| L2 | 10 | gRPC transport, 7-day soak, public bench | meta-swarm-team + sre-team |
| L3 | 8 | Multi-tenant, cross-region, mTLS auto-rotation | meta-swarm-team + platform-team |
| L4 | 6 | Raft HA, ≥ 5 replicas tolerate ≤ 2 failures | meta-swarm-team + sre-team |
| L5 | 8 | Customer SLA, status page, first paying tenant | meta-swarm-team + product-team |

Detailed criteria :
- L1 → [`EFE_A1_PHASE2_ARCHITECTURE.md § 12`](EFE_A1_PHASE2_ARCHITECTURE.md#12-gate-2--3-l2-attained-criteria) (G1-G10)
- L3-L5 → in the respective phase architecture documents (drafted at each phase kickoff)

---

## 11. Cross-phase dependencies

```
P0 (design) ──────► P1 (lib) ─────────────► P2 (gRPC)
                       │                       │
                       │                       ├─────► P3 (multi-tenant)
                       │                       │         │
                       │                       │         │
                       │                       │         └────► P4 (Raft)
                       │                       │                  │
                       │                       │                  └──► P5 (SLA)
                       │                       │
                       └─soft launch 3 runs────┘
                       └─license carve-out R5-B1 ─────────────────────► P5 flips Apache 2.0
```

### 11.1 Hard dependencies (block downstream if missed)

| Dep | Source | Target | Why |
|---|---|---|---|
| HD-1 | P1 SLO baseline frozen | P2 NON-REGRESSION | NON-REGRESSION reads `last_p99_observed_ms` from the P1 baseline. |
| HD-2 | P2 mTLS infra | P3 multi-tenant | P3 tenant isolation rides on the mTLS cert SAN. |
| HD-3 | P2 SIGHUP hot reload | P3 cross-region | Regions reload manifests via SIGHUP on failover. |
| HD-4 | P3 cross-region | P4 Raft | Raft assumes inter-region network primitives. |
| HD-5 | P4 Raft | P5 SLA | 99.95 % availability requires Raft HA. |
| HD-6 | P0 license carve-out R5-B1 | P5 license flip | Carve-out is the legal basis for flipping to Apache 2.0. |

### 11.2 Soft dependencies (degrade if missed)

| Dep | Source | Target | Degradation |
|---|---|---|---|
| SD-1 | P1 OTel skeleton | P2 OTel spans | If P1 OTel is absent, P2 ships its own — same outcome, +1 week. |
| SD-2 | P2 public bench | P5 marketing | P5 can ship without if P2 bench is delayed ; we re-run at P5. |
| SD-3 | P3 f64 mode | P5 high-precision tenants | If P3 f64 is delayed, P5 launches without high-precision tenant tier. |

---

## 12. Critical path

The critical path (Gantt-style longest sequence) is :

```
P0 design (8w) → P1 library (11w) → P2 gRPC (8w) → P3 multi-tenant (11w) → P4 Raft (11w) → P5 SLA (11w)
                                                                                                       │
                                                                                              60 weeks  total active
                                                                                              + 17w slack
                                                                                              ─────────
                                                                                              77 weeks
                                                                                              (≈ 18 months)
```

The 17 weeks of slack are intentional :
- 7 weeks between P1 and P2 (vacation, P1 soft-launch, P2 architecture)
- 5 weeks P2 → P3 (P2 stabilisation in prod)
- 3 weeks P3 → P4 (holidays)
- 2 weeks P4 → P5 (light)

If any phase slips by > 4 weeks, the L5 target moves to **Q3 2027**.
If two phases slip, replan.  The team's commitment is to **L2 (P2)
hitting on time**, after which downstream slack absorbs minor delays.

### 12.1 Burndown rules

- Slip > 1 week :  PM notified, no re-planning.
- Slip > 2 weeks : engineering lead notified, mid-phase re-baseline.
- Slip > 4 weeks : reschedule downstream, update this doc.
- Slip > 8 weeks : phase merged with next (e.g. P3+P4 combined).

---

## 13. Risk register

Risks are scored on **Likelihood × Impact** (L/M/H × L/M/H).  Owners
are responsible for monitoring + reporting at every Phase Gate.

| ID | Risk | L | I | Owner | Mitigation |
|---|---|:---:|:---:|---|---|
| RR-1 | gRPC overhead pushes P2 p99 over budget | M | H | meta-swarm-team | StepStream + 150 ms budget headroom |
| RR-2 | Helm chart breaks Phase-1 prod deployments | L | H | platform-team | Default-disabled + golden manifest diff CI |
| RR-3 | P3 cross-region replication has split-brain bug | M | H | meta-swarm-team | Use K8s leases until P4 Raft ; chaos drills monthly |
| RR-4 | Customer doesn't sign by L5 (P5 G3) | M | M | product-team | Start customer discovery in P3, not P5 |
| RR-5 | License flip blocked by legal | L | H | sec-team | Carve-out R5-B1 ratified at L0 ; ratification stands |
| RR-6 | Sandbox / TCC kills dev productivity (mac) | M | L | meta-swarm-team | Recommend devs use `~/Projects/`, not `~/Downloads/` |
| RR-7 | Determinism breaks across rustc versions | M | H | meta-swarm-team | Pin rustc 1.88 ; CI matrix on rustc 1.88 + nightly |
| RR-8 | OTel cardinality blows up Grafana | L | M | sre-team | Bounded labels, sampling 5 % |
| RR-9 | Raft impl bugs (P4) | M | H | meta-swarm-team | Use `raft-rs` (TiKV-tested) ; do not roll our own |
| RR-10 | Customer SLA breach in first month of P5 | M | H | sre-team + product-team | 30-day pre-launch observation (L5-G5) + canary tenant |
| RR-11 | BLAS / GPU vector requirement appears in P3+ | M | M | meta-swarm-team | DT-A1-BLAS-PHASE3 tracker ; budget +1 sprint if needed |
| RR-12 | Compliance / data-residency requires data localisation | L | H | sec-team | Phase 3 cross-region already gives per-region isolation |

---

## 14. Resource allocation

FTE-weeks per phase per team.  A FTE-week = one full-time engineer for one week.

| Team | P0 | P1 | P2 | P3 | P4 | P5 | Total |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| meta-swarm-team | 8 | 33 | 24 | 33 | 33 | 22 | **153** |
| sre-team | 1 | 2 | 8 | 11 | 11 | 22 | **55** |
| platform-team | 0 | 1 | 8 | 11 | 5 | 5 | **30** |
| sec-team | 4 | 2 | 4 | 5 | 4 | 8 | **27** |
| product-team | 0 | 0 | 0 | 5 | 0 | 22 | **27** |
| eng-leadership (review) | 2 | 3 | 3 | 3 | 3 | 3 | **17** |
| **Total** | **15** | **41** | **47** | **68** | **56** | **82** | **309** |

Assuming ~6 FTE meta-swarm-team capacity, the 153 FTE-weeks fits in
77 calendar weeks comfortably (theoretical max 462 FTE-weeks).

### 14.1 Hiring needs

- Phase 3 (Q4 2026) : +1 platform engineer to own the Helm + K8s
  surface as it grows.
- Phase 5 (Q2 2027) : +1 product engineer to own customer integration
  + status page.

---

## 15. Decision log

Major architectural decisions, by phase :

### L0 (Design)
- **D-L0-1** : Public API is `MetaSwarm` + `SwarmParticipant`, both crate-public, no `pub(crate)` for L1-L5 callers.
- **D-L0-2** : Fixed-point math is contractive (joint Picard iteration).  Rejected : variance-weighted aggregation (too dependent on per-participant noise model).
- **D-L0-3** : f32 default, f64 opt-in (P3+).
- **D-L0-4** : Carve-out R5-B1 : `UNLICENSED` until P5.

### L1 (Library)
- **D-L1-1** : `iter_batched` not `iter_with_setup` for benches — Criterion contract guarantees setup cost is excluded.
- **D-L1-2** : Dual encoding `target+operator` AND `comparator+threshold_*` in baseline YAML, alignment deferred to `DT-A2-SLO-PARSER-UNIFY`.
- **D-L1-3** : Master rollup `SLO-OPS-A1-META-SWARM` added in PR #4 (not deferred), because the title "+ master-gate hook" demanded a real gate.
- **D-L1-4** : Soft-launch `deferred_ticket: DT-A1-META-SOFT-LAUNCH-3-RUNS` on all 7 surface SLOs ; removed after 3 main runs PASS.

### L2 (gRPC)
- **D-L2-1** (planned) : f32 only on the wire ; f64 deferred to P3.
- **D-L2-2** (planned) : Single controller in Phase 2 (K8s lease) ; Raft in P4.
- **D-L2-3** (planned) : `panic = "unwind"` for `efe-meta-swarm-server` ; one participant crash must not kill the controller.

### Later phases : decisions logged at each respective gate.

---

## 16. Communication plan

| Audience | Cadence | Channel | Format |
|---|---|---|---|
| meta-swarm-team | daily | #meta-swarm | Slack standup |
| eng-leadership | weekly | #eng-leadership | 1-line status |
| sre-team | per-PR + soak start | #sre | PR review + ad-hoc |
| platform-team | per-phase-kickoff + Helm PRs | #platform | Doc review |
| product-team | monthly during P3-P5 | sync meeting | Slides |
| All EFE | per-gate (L1-L5) | #announcements | Announce post |
| External (customer) | per-L5 milestone | status.efe.io | Status update + blog post |

### 16.1 Phase gate announcement template

```
[L<n> ATTAINED] efe-meta-swarm Phase <n> shipped

Highlights :
  - <2-3 bullets>

Numbers :
  - Tests passing : <count>
  - Surface SLOs  : <count>
  - Master rollups : <count>

Next : Phase <n+1>, target <date>.

Detailed report : <link to doc>
```

---

## 17. Open questions

Per-phase open questions that need answers before that phase starts.
This is the **forward queue** of design questions.

### Phase 2 (already in [phase 2 doc § 21](EFE_A1_PHASE2_ARCHITECTURE.md#21-open-questions))
1. f32 vs f64 on the wire
2. StepStream mandatory or optional
3. TLS-only vs plaintext-dev
4. Helm major/minor bump
5. Soak start date
6. Sigstore signing of bench HTML
7. mTLS auto-rotation in P2 or defer to P3
8. Single controller vs HA in P2

### Phase 3 (resolve by 2026-09-30)
9. Tenant quota model : per-tenant CPU+mem, or per-tenant `iters/sec` only ?
10. Cross-region active-active for `Iterate` (with conflict resolution), or active-passive only ?
11. Checkpoint frequency : every N iters, every T seconds, or hybrid ?
12. S3 bucket layout : per-tenant prefix or shared with object-level ACLs ?
13. f64 mode : 100 % opt-in, or default for tenants with `precision=high` claim ?
14. Multi-region failover trigger : automatic (latency probe) or manual (operator) ?

### Phase 4 (resolve by 2026-12-31)
15. `raft-rs` vs `openraft` vs in-house ?
16. Quorum size : 3, 5, or 7 ?
17. Snapshot storage : same S3 bucket as P3 checkpoints, or dedicated ?
18. Should `Iterate` results be replicated by Raft (durable) or kept best-effort (low-latency) ?

### Phase 5 (resolve by 2027-03-31)
19. SLA breach credit schedule : flat % refund, or tiered ?
20. Status page : self-hosted or Statuspage.io ?
21. First-tenant onboarding flow : self-serve via UI, or white-glove ?
22. Apache-2.0 vs MIT for the license flip ?
23. Do we offer a free tier or paid-only ?

---

## Appendix A — Cross-references

- Phase 1 architecture : *(to be added when `EFE_A1_PHASE1_ARCHITECTURE.md` is finalised)*
- Phase 2 architecture : [`EFE_A1_PHASE2_ARCHITECTURE.md`](EFE_A1_PHASE2_ARCHITECTURE.md)
- Phase 1 bench tracking : [`EFE_A1_BENCH_TRACKING.md`](EFE_A1_BENCH_TRACKING.md)
- Operational doctrine : [`EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md)
- Master gate runbook : [`runbooks/ops_master_gate_fail.md`](runbooks/ops_master_gate_fail.md)
- PR #4 description : [`../crates/efe-meta-swarm/PR4_DESCRIPTION.md`](../crates/efe-meta-swarm/PR4_DESCRIPTION.md)

## Appendix B — Refresh cadence

This document is refreshed :

- After **every phase gate** (L1 → L5).
- After **every quarterly leadership review**.
- When a **risk fires** (RR-* event).
- When a **decision is made** that contradicts an existing § 15 entry.

The "last refreshed" stamp at the top of this file is the source of
truth.  A diff between two versions of this doc is the cleanest record
of how the roadmap evolved.
