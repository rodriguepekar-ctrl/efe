# EFE M50 — N16 Multi-Region Replay Architecture

> **Sprint** : 9 (Phase 3, day 26 – 30 of M50).
> **Status** : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` (Sprint 8 N11, shipped)
> * `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` (Sprint 8 N12, shipped)
> * `EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md` (Sprint 8 N13, shipped)
> * `EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md` (Sprint 8 N14, shipped)
> * `EFE_M50_N15_FAIRNESS_SCHEDULING_ARCHITECTURE.md` (Sprint 8 N15, shipped)
>
> **Anchors** : Sprint 6 `ReplayEngine` (single-region replay) is the algorithmic root.  N16 is the **multi-region extension** ; it does NOT replace Sprint 6 — it composes with it.

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2. [Six incident patterns N16 closes (P1–P15)](#2-six-incident-patterns-n16-closes-p1p15)
3. [Commitments — what ships in Sprint 9 (G1–G15)](#3-commitments--what-ships-in-sprint-9-g1g15)
4. [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5. [Failure modes — what we PROTECT against (F1–F16)](#5-failure-modes--what-we-protect-against-f1f16)
6. [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7. [Component architecture](#7-component-architecture)
8. [Module hierarchy & data model](#8-module-hierarchy--data-model)
9. [API surface (HTTP + internal Rust traits)](#9-api-surface-http--internal-rust-traits)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [N11 / N12 / N13 / N14 / N15 alignment](#13-n11--n12--n13--n14--n15-alignment)
14. [Sprint 9 ship vs Sprint 10/11 split](#14-sprint-9-ship-vs-sprint-1011-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The Problem.**  The Sprint 6 `ReplayEngine` reconstructs and re-dispatches a swarm
inside the SAME region that produced it.  Phase 3 of M50 introduces multi-region
deployment (us-east-1, us-west-2, eu-west-1, ap-southeast-1) and three classes
of cross-region replay become operational requirements :

1. **Disaster Recovery (DR)** — region us-east-1 is offline ; replay its swarms
   in us-west-2 to validate the standby region can serve.
2. **Compliance audit** — a regulator demands forensic replay of EU customer
   workloads.  Replay must happen IN the EU (GDPR data residency).
3. **Catalog-drift detection** — a `RolloutPolicy::Stable(v2)` was deployed to
   eu-west-1 but us-east-1 is still on `v1`.  Operators want to replay a v1
   swarm against v2 to detect output drift BEFORE rolling v2 worldwide.

Without N16, every cross-region replay attempt today violates at least one of :
data residency (raw PII transferred), audit-chain integrity (HMAC keys not
portable), catalog-version pinning (replay picks the wrong agent version),
fairness (replay traffic starves real customers), or attribution (cost billed
to the customer instead of EFE).

**The Fix.**  N16 introduces a **ReplayCoordinator** service that orchestrates
cross-region replay with five strict guarantees :

1. **Data residency enforced** — the coordinator validates the
   tenant's `ResidencyPolicy` BEFORE any chain bytes leave the source region.
2. **HMAC chain portability** — chains are signed with HKDF-derived per-tenant
   keys that are reproducible in any region with KMS access ; verification is
   region-agnostic.
3. **Catalog version pinning** — the replay request carries the original
   `(agent_id, version)` tuple from the swarm envelope ; target region rejects
   the replay if the version is absent in its catalog.
4. **Fairness via N15** — replay traffic is admitted through a dedicated
   `BackpressureKind::ReplayRateLimited` lane in the N15 admission gate,
   capped at 10 % of the global op pool per region.
5. **Audit-of-audit** — every replay is itself audited with a typed
   `ReplayDispatched` trace that references the original swarm's
   `envelope_trace_id` ; replays-of-replays are forbidden at the audit layer
   (`I20`).

**Six capabilities ship in Sprint 9 N16** :

1. **`RegionCode`** SDK enum and `ResidencyPolicy` per-tenant configuration.
2. **`ReplayRequest` + `ReplayJob` + `ReplayResult`** SDK types — the operator-
   facing wire format.
3. **`ReplayCoordinator`** runtime service — front door for all replays,
   validates compliance + admits via N15 + dispatches to source / target
   regions.
4. **`ChainShipmentClient` + `ChainShipmentServer`** — gRPC over mTLS,
   atomic cross-region chain transfer with hash verification.
5. **`MultiRegionReplayEngine`** — wraps Sprint 6 `ReplayEngine` with region
   context, agent version pinning, dispatcher selection, and result diff
   storage in the dedicated `replay.*` Postgres schema.
6. **Replay-of-audit chain** — separate HMAC-chained `ReplayDispatched` /
   `ReplayCompared` / `ReplayCompleted` trace types stored in
   `replay.replay_audit_chain`.

**Out of scope (deferred to Sprint 10 N17 / Sprint 11 N18)** :

* Active-active multi-region writes (Sprint 10 N17 — cross-region quorum
  consensus).
* Automated DR failover orchestration (Sprint 11 N18 — runbook automation).
* Replay-driven canary A/B testing harness (Sprint 11 N19 — needs N18
  failover semantics first).
* Replay PII redaction service (Sprint 12 — wraps N16 + M24 GDPR saga).

**Backward compatibility** : N16 changes are **strictly additive**.  Sprint 6
`ReplayEngine` keeps its single-region API ; multi-region capability is a
new wrapper layer.  Pre-N16 chains still verify ; new chains carry
`origin_region: Option<RegionCode>` which is `None` for legacy entries.

---

## 1. Context, motivation, sprint placement

### 1.1 The shape of the gap

Today's `ReplayEngine` (Sprint 6) operates on three implicit assumptions :

| Assumption | Reality in Phase 3 |
|---|---|
| The audit chain backing the replay is in the **local Postgres**. | Multi-region deployments fan out the chain to 4 regions — local DB has only the region's slice. |
| HMAC keys for chain verification are in the **local KMS / Vault**. | Per-region KMS instances are independent ; cross-region key access requires explicit configuration. |
| The catalog has every `(agent_id, version)` tuple the chain references. | Region us-east-1 ships v1 ; eu-west-1 might ship v2 ahead.  Catalog drift across regions is common during rollout. |

When any of these break, today's `ReplayEngine` returns `ReplayError::NotFound`
or `ReplayError::ChainBroken`.  Operators in our Sprint 8 hardening cycle
hit this three times in a row :

1. **Incident I-2026-04-12** — EU customer requested replay for GDPR audit.
   Replay attempted in us-east-1.  Chain verification failed because eu-west-1
   HMAC key was not present in us-east-1's Vault.  Customer's audit deadline
   slipped 9 days.

2. **Incident I-2026-04-23** — Canary in us-east-1 promoted to Stable.  Two
   days later operators wanted to replay the canary cohort in eu-west-1 to
   confirm consistent behaviour.  Replay returned 17 % mismatches.
   Post-mortem revealed eu-west-1 still served v1 from a stuck deployment ;
   the "regression" was actually catalog drift, not code drift.  6 hours of
   investigation wasted.

3. **Incident I-2026-05-04** — DR drill : us-east-1 marked OFFLINE for the
   drill.  Operators tried to replay 200 swarms in us-west-2 to validate
   standby.  Replay coordinator (not yet built) implicitly fetched chain
   from us-east-1 (offline) — entire drill failed with `Store(io: connection
   refused)`.  No actual region-failover validation happened.

These three patterns motivate **N16 as a Sprint 9 priority** : the operator
class needs first-class cross-region replay, with the same forensic
guarantees as Sprint 6's single-region replay (G2 verifiable HMAC chain,
G6 audit chain), now extended across region boundaries.

### 1.2 Why Sprint 9, not Sprint 8

Sprint 8 closed the **single-region** operational primitives :

* N11 Firecracker pool (compute substrate)
* N12 cache key hardening (idempotency correctness)
* N13 shadow / canary catalog (rollout safety)
* N14 swarm envelope trace (forensic anchor)
* N15 fairness scheduling (multi-tenant resource governance)

These five make a single region production-grade.  Sprint 9 turns to
**multi-region**.  The N16 ship is the foundation : every subsequent Phase 3
ship (N17 active-active, N18 DR automation, M28 events cross-region fan-out)
depends on the cross-region primitives N16 defines (`RegionCode`,
`ChainShipmentProtocol`, `ResidencyPolicy`).

### 1.3 Who depends on N16 once it ships

| Consumer | Use case | API touched |
|---|---|---|
| **Compliance team** | GDPR audit replays in EU | `ReplayRequest { target_region: EuWest1 }` |
| **SRE / on-call** | DR drills (replay X swarms in standby region) | bulk replay endpoint |
| **Platform team** | Cross-region catalog drift detection | replay with explicit `expected_agent_version` |
| **Customers (Enterprise)** | "Reproducible regulator-ready" attestations | signed `ReplayResult` returned |
| **M24 GDPR saga** | Replay against tombstoned data → 451 | `ReplayCoordinator` integrates with M24 erase journal |
| **M31 billing** | Replay cost attribution to operator account, not customer | `ReplayResult.cost_attribution: Operator` |

### 1.4 The 15 incident-class patterns (preview)

Section 2 enumerates 15 incident classes N16 closes.  They cluster into 4
families :

* **Compliance & data residency** (P1, P5, P9, P12) — 4 patterns
* **Chain & catalog integrity** (P2, P3, P4, P10) — 4 patterns
* **Operational safety** (P6, P7, P8, P14, P15) — 5 patterns
* **Determinism & comparison** (P11, P13) — 2 patterns

Each pattern has a matching commitment (G), invariant (I), and failure-mode
(F) row, structured for an audit-grade traceability matrix.

---

## 2. Fifteen incident patterns N16 closes (P1–P15)

> **Methodology** : every P-row is sourced from either (a) a real Sprint 8
> hardening incident with ticket reference, (b) a near-miss flagged in the
> Sprint 8 retro, or (c) a regulator-level requirement from EFE's design
> partners.  No speculative patterns.

### Family A — Compliance & data residency

**P1. Raw PII transferred across regions during replay.**
Customer's GDPR-tagged swarm ran in eu-west-1.  Operator triggered replay in
us-east-1 to leverage spare capacity.  Today's engine has no residency
check ; it would copy the chain (including `full_input` bodies containing
PII) into us-east-1's Postgres.  **GDPR Article 44 violation** : personal
data transferred outside the EEA without an adequacy decision or appropriate
safeguards.  Real near-miss : a draft script was reviewed and BLOCKED at
code review in Sprint 7.

**P5. Replay leaks PII into a region's local backup.**
Even when the active replay is consciously routed to eu-west-1, if the
target region's audit chain backups replicate to a US-based DR bucket, the
PII still leaks through the back door.  N16 must mark replay-chain entries
with explicit residency so backup policies can filter them.

**P9. Tenant-mixed PII via shared replay catalog.**
Two tenants in eu-west-1 ; operator runs bulk replay for tenant A.  The
replay catalog table (`replay.replay_results`) is mistakenly read by a
report generator for tenant B (cross-tenant query bug).  Even though
neither swarm crossed regions, the **multi-tenant replay schema** must
enforce tenant isolation at the row level just like the production audit
chain (`ChainStoreError::CrossTenant` already exists for production —
mirror it for replay).

**P12. KMS key inaccessible in target region.**
Source region uses `aws/kms/eu-west-1/tenant-prod` to encrypt
`full_input` body BLOBs.  Target region us-west-2 attempts to decrypt
during replay → `KMS::Decrypt` returns 403 (cross-region key not granted).
Today's `ReplayEngine` would surface this as a vague `Store(io: KMS error)`.
N16 must (i) check key accessibility BEFORE shipping the chain, (ii) return
a typed `KmsKeyNotAvailable { kms_arn, target_region }` error.

### Family B — Chain & catalog integrity

**P2. HMAC verification fails because target region lacks the per-tenant key.**
Sprint 6 derives per-tenant HMAC keys via HKDF from a master key.  In a
multi-region deployment, each region runs its own HKDF instance with its
own master key (security best practice — no cross-region master-key sharing).
A chain produced in eu-west-1 with HKDF(master_eu, tenant) does NOT verify
against HKDF(master_us, tenant).  N16 must ship the **per-tenant derived
key** alongside the chain (not the master), authenticated by the source
region's signing root.

**P3. Catalog drift causes false mismatches.**
A swarm referencing `summarize_strict@v1.2.3` ran in us-east-1.  Operator
attempts replay in eu-west-1, which has `summarize_strict@v1.2.4` as
`Stable`.  Replay runs against v1.2.4, output mismatches, "regression"
falsely flagged.  Investigation reveals it's catalog drift.  N16 must
**pin to the exact recorded version** at replay time and **fail-fast** if
the version is absent in target region.

**P4. Chain has gaps after cross-region transfer.**
Chain transfer over gRPC is interrupted at trace 47 of 113.  Sprint 6's
verify pass succeeds on traces 1-47 (HMAC consistent), but ops 48-113 are
never re-dispatched.  Replay completes with `partial: true` but the diff
report says "all 47 ops match" — looks like success.  N16 must verify the
**chain head hash** against the source region's expected head BEFORE
starting reconstruction.

**P10. Schema version drift between source and target.**
Source region runs SDK v0.3.2 (envelope format v3).  Target region runs
SDK v0.4.0 (envelope format v4).  Chain transfer succeeds (raw bytes),
but envelope deserialization fails.  N16 must (i) embed schema version
in the shipment header, (ii) refuse replay if the target's schema is
older than the source's (operator must upgrade), (iii) accept newer
schemas with a documented compat layer.

### Family C — Operational safety

**P6. Replay floods overwhelm the target region's dispatcher.**
Operator triggers bulk replay of 10 000 swarms in eu-west-1 to validate a
DR drill.  Without rate limiting, those 10 000 swarms hit the dispatcher
at line rate, consuming all `op_drr` slots, starving real customer
traffic.  Customers see 429s for the duration of the drill.  N16 must
**reuse the N15 admission gate** with a dedicated `ReplayRateLimited`
backpressure kind, capped per-region.

**P7. Replay loop : replay traces trigger more replays.**
A replay produces audit traces (the replayed ops).  If those traces are
considered "real" by another automation that auto-replays new traces of
the same swarm, an **infinite loop** ensues.  N16 must tag replay-derived
traces with a sentinel so downstream automation can filter
(`audit_tags: ["replay", "no_recursion"]`).

**P8. Network partition during chain transfer.**
gRPC stream from eu-west-1 to us-west-2 fails mid-stream.  Today's code
would leave a half-loaded chain in memory and return Err.  N16 must use
an **atomic-shipment protocol** — either the entire chain is staged in a
holding area and atomically committed, or nothing.

**P14. Replay request without operator attribution.**
A replay is triggered.  Two months later forensics ask "who did this?".
There's no ticket / operator name / reason.  N16 mandates every
`ReplayRequest` carry `actor + ticket_id + reason` triple ; the
coordinator refuses replays without all three.

**P15. Concurrent replays of the same swarm race.**
Operators A and B both trigger replay for swarm `S` simultaneously.  Today
both would proceed in parallel, double the cost, possibly diverging
results.  N16 coordinates via a per-`(tenant, swarm_request_id)` lease in
the replay schema ; the second call gets `409 Conflict` with the in-flight
job's ID.

### Family D — Determinism & comparison

**P11. Mode C non-determinism inflates false-regression count.**
LLM outputs are inherently non-deterministic (sampling + temperature).
Comparing replay vs original output byte-by-byte for Mode C will show
near-100 % mismatch.  N16 must (i) classify per-mode tolerance,
(ii) use N13 `ComparisonStrategy::Semantic` for Mode C, (iii) report
mismatch ratio explicitly so operators interpret correctly.

**P13. Time-skew causes deterministic Mode A to falsely mismatch.**
A Mode A agent reads `now()` and embeds it in the output.  Replay
happens at T+30 days, "now()" differs by 30 days, output differs.  This
is a known limitation of Mode A determinism — the agent contract says
"agents must not read external state, including clock".  N16 must :
(i) document that Mode A determinism assumes no `now()` reads,
(ii) provide a **virtual clock** facility in the replay context so
agents that legitimately need `now()` get the original timestamp.

---

## 3. Commitments — what ships in Sprint 9 (G1–G15)

> **Read convention** : each G-row has the form `G-N : commitment / impl
> surface / acceptance test`.  Acceptance tests are tagged
> `phase_3_n16_*` and ship in the integration test suite.

### Compliance-class commitments

**G1. Data-residency pre-flight.**  Every `ReplayCoordinator.submit()` runs
`ResidencyPolicy::validate(tenant, target_region)` BEFORE any byte of chain
leaves the source region.  On violation it returns `Err(ReplayError::
ResidencyViolation { policy, target_region })`.  Acceptance test :
`phase_3_n16_residency_blocks_eu_to_us`.

**G5. PII in `full_input` is redacted before cross-region transfer if
`ResidencyPolicy::RedactOnTransfer` is set.**  Redaction reuses Sprint 5
`PiiRedactor` ; redacted body carries the `redaction_token_set` to verify
match later.  Acceptance : `phase_3_n16_redaction_pre_shipment`.

**G9. Per-tenant replay schema isolation.**  The `replay.replay_results`
table is partitioned by `tenant_id` ; every query in `MultiRegionReplay
Engine` includes `WHERE tenant_id = $1`.  Cross-tenant access returns
`ReplayError::CrossTenant`.  Acceptance : `phase_3_n16_cross_tenant_read_
blocked`.

**G12. KMS key availability check pre-shipment.**  Before chain shipment,
the coordinator calls `KmsKeyResolver::check_access(target_region, kms_arn)`
for every distinct key the chain references.  If any returns
`InsufficientPermissions`, the entire replay is rejected with
`KmsKeyNotAvailable { keys: Vec<_> }`.  Acceptance : `phase_3_n16_kms_
unavailable_short_circuits`.

### Integrity-class commitments

**G2. HMAC chain portable across regions.**  The chain shipment payload
includes the per-tenant HKDF-derived key, signed by the source region's
signing root.  Target region verifies the signature against a registry of
trusted signing roots, then uses the derived key to validate the chain.
Acceptance : `phase_3_n16_cross_region_chain_verifies`.

**G3. Catalog version pinning enforced.**  `ReplayRequest` MUST include
`expected_agent_versions: HashMap<NanoAgentId, NanoAgentVersion>` extracted
from the original swarm envelope.  Target region's catalog is queried for
each ; missing version → fail-fast with `AgentVersionMissing { agent_id,
version, region }`.  Acceptance : `phase_3_n16_version_drift_rejected`.

**G4. Atomic chain shipment.**  Shipment uses a "staging + commit" two-phase
protocol : (1) source sends chain bytes + head hash, (2) target verifies
head hash, (3) target acknowledges commit, (4) source confirms.  Network
failures at any phase leave the target staging area empty (rolled back).
Acceptance : `phase_3_n16_shipment_atomic_on_partition`.

**G10. Schema version negotiation.**  Shipment header carries
`sdk_schema_version: u32`.  Target rejects if its `MAX_SUPPORTED_SCHEMA`
is below the source's.  Compat layer migrates from older to newer schemas
in `MultiRegionReplayEngine`.  Acceptance : `phase_3_n16_schema_too_new_
rejected`.

### Operational-safety commitments

**G6. Replay traffic gated by N15.**  `MultiRegionReplayEngine::dispatch`
calls `admission_gate.admit_swarm` with op_count from the original
envelope.  Replay-specific bucket : `BackpressureKind::ReplayRateLimited`
caps at `10 % × global_op_drr_capacity` per region.  Acceptance :
`phase_3_n16_replay_does_not_starve_real_traffic`.

**G7. No replay-of-replay.**  The audit-trace check : if the swarm's
envelope `audit_tags` contains `"replay"`, the coordinator returns
`ReplayError::NestedReplayForbidden`.  Acceptance : `phase_3_n16_no_
nested_replays`.

**G8. Bounded per-swarm replay duration.**  Default cap : 10 minutes per
swarm.  Configurable via `ReplayRequest.timeout_ms`, hard ceiling at
60 minutes.  Acceptance : `phase_3_n16_replay_respects_timeout`.

**G14. Mandatory attribution.**  `ReplayRequest { actor, ticket_id,
reason }` are all required ; `actor.is_empty()` → `400 Bad Request`.
Acceptance : `phase_3_n16_missing_attribution_rejected`.

**G15. Atomic concurrent-replay lease.**  At submit, the coordinator
inserts `(tenant_id, swarm_request_id, job_id, leased_at)` into
`replay.replay_lease`.  UNIQUE constraint on `(tenant_id, swarm_request_id)
WHERE leased_at > NOW() - INTERVAL '30 minutes'`.  Concurrent submits
hit the constraint → `409 Conflict { existing_job_id }`.  Acceptance :
`phase_3_n16_concurrent_replay_rejected`.

### Determinism-class commitments

**G11. Mode-classified comparison thresholds.**  Replay result includes
per-op `mode_classification: ExecutionMode` and `match_outcome:
{Exact|Tolerated|Mismatch}`.  Thresholds per architecture § 7.5.
Acceptance : `phase_3_n16_mode_c_tolerance_applied`.

**G13. Virtual-clock injection.**  `ReplayCallContext` carries a
`virtual_clock: SystemTime` matching the original swarm's `started_at`.
Agents that need `now()` MUST read from the context, not `std::time`.
For Mode A determinism enforcement, the runtime sandboxes `std::time` calls
in a wrapper that asserts on use.  Acceptance : `phase_3_n16_virtual_clock_
deterministic_replay`.

---

## 4. Invariants — non-negotiables (I1–I20)

> **Reading convention** : every I-row is a *system-wide invariant* that
> MUST hold across any execution interleaving, any failure mode, any
> tenant.  Violations are bugs, not edge cases.

### Region & residency

**I1. Region-tagged chain entries.**  Every `NanoAgentTrace` written
post-N16 has `origin_region: Option<RegionCode>` ; legacy entries (pre-N16)
have `None` and are treated as `Unknown` for residency checks (default
deny for cross-region).

**I2. Chain verification stable across regions.**  Mathematically :
`verify(chain, key_source_region_A) == verify(chain, key_source_region_B)`
provided both key sources resolve the same per-tenant key.  The HKDF
salt is **tenant-scoped, not region-scoped**, so derivation is
region-invariant given the master key.  N16 ships master keys to target
regions via the trusted-signing-root mechanism (ADR-3).

**I3. Replay never modifies the original audit chain.**  All replay-
generated traces (`ReplayDispatched`, `ReplayCompared`, replay op traces)
go to `replay.replay_audit_chain` ; the production
`m50.audit_chain` table is **read-only** during replay.

**I9. Replay traces carry sentinel tags.**  Every replay-of-X trace has
`audit_tags = ["replay", "origin_region:<R>", "origin_swarm_id:<S>",
"replay_job_id:<J>"]`.  Filterable by `tag LIKE 'replay'` so reporting
queries can exclude.

**I15. Compliance check first.**  In `ReplayCoordinator.submit()`, the
order MUST be : (1) authn/authz, (2) attribution check (G14), (3)
residency check (G1), (4) nested-replay check (G7), (5) catalog version
check (G3), (6) KMS access check (G12), THEN (7) enqueue.  Any earlier
check failing skips all subsequent checks.

### Chain integrity

**I4. Replay pins agent version.**  `ReplayExecutionEngine.dispatch` calls
`catalog.get` with `NanoAgentRef::new_pinned(id, version)` (a new method
introduced by N16) which returns `CatalogError::VersionNotFound` if the
exact version is absent, regardless of `RolloutPolicy` state.

**I17. Atomic chain shipment.**  The shipment protocol is :
```
1. POST /chain/stage   { swarm_request_id, chain_bytes, head_hash }
2. GET  /chain/verify  { staging_id } → { verified: bool }
3. POST /chain/commit  { staging_id }
4. POST /chain/abort   { staging_id }   ← idempotent rollback
```
If any step (1)-(3) fails or times out, the staging area is GC'd after
5 minutes ; no replay ever reads from a staged-but-uncommitted chain.

**I18. Chain head hash verified pre-replay.**  Before reconstruction
begins, target computes `Blake3(concat(chain_row_hashes))` and compares
to the head hash sent in the shipment payload.  Mismatch → `ChainHead
Mismatch` error.

**I19. Replay envelope schema versioned.**  Shipment payload carries
`sdk_schema_version: u32` matching `efe_nano_sdk::SCHEMA_VERSION` (a new
const).  Target compares to its `MAX_SUPPORTED_SCHEMA` ; reject if
source > target.

### Catalog & dispatch

**I7. Replay never bills the customer.**  `ReplayDispatchContext.cost_
attribution = CostAttribution::Operator(operator_id)`.  M31 billing
integration : Mode C LLM calls during replay are charged to operator's
EFE-internal account, not the customer's.

**I8. Replay never triggers shadow scheduler.**  The dispatcher's
`shadow_scheduler` field is unset when `request.audit_tags` contains
`"replay"`.  N13 rollout state is read-only during replay.

**I20. No nested replays.**  See G7.  Audit-tag-based check at the
coordinator front door + at the dispatcher level (defense in depth).

### Resource limits & fairness

**I11. Replay rate-limited per-tenant.**  N15 admission gate is wired
with `replay_op_drr` capacity = 10 % of `op_drr` capacity ; per-tenant
weight = `tier.drr_weight / 10`.  Effective rate : Enterprise tenant can
replay 100 ops/sec ; Free tenant : 1 op/sec.

**I12. Replay queue is persistent.**  `replay.replay_jobs` table backs
the queue ; process restarts resume from `WHERE status = 'pending' OR
status = 'in_progress'`.  At-most-once execution via the lease.

**I16. Replay duration bounded.**  `request.timeout_ms ≤ 3 600 000`
(60 minutes hard ceiling).  Coordinator validates at submit ; engine
enforces with `tokio::time::timeout`.

### Multi-tenancy & access

**I13. Multi-tenant isolation in replay storage.**  Postgres RLS
(Row-Level Security) policies on `replay.replay_jobs`, `replay.replay_
results`, `replay.replay_diffs` : every query gets `WHERE tenant_id =
current_setting('app.current_tenant_id')::uuid`.  Defense in depth :
trait-level `assert_tenant_match(query_tenant, ctx_tenant)`.

**I10. Operator JWT required.**  Replay endpoint requires bearer token
with `replay:execute` scope.  `OperatorClaims { sub, scope, ticket_id,
expires_at }` validated by `JwtValidator`.

### Mode determinism

**I5. Mode A replay bit-exact.**  Pure Rust handler outputs `==`
byte-for-byte to the original ; any difference is treated as
`MatchOutcome::Mismatch` and surfaces to operators.

**I6. Mode B/C replay tolerated.**  Per-mode threshold (configurable
per-agent, defaults below).  Tolerated mismatches counted but NOT
flagged red :
* Mode B : 99.9 % structural match required.
* Mode C : 95 % semantic match required (N13 `ComparisonStrategy::
  Semantic`).

### Replay audit chain

**I14. KMS keys tagged for replay accessibility.**  Every `full_input`
body BLOB stored in `m50.body_store` carries `kms_arn: String` metadata.
The shipment pre-flight enumerates distinct ARNs and verifies access in
target region.

---

## 5. Failure modes — what we PROTECT against (F1–F16)

> **Read convention** : every F-row has `cause / detection / response /
> recovery`.  The system MUST be deterministic about every F — random
> behaviour ("eh, just retry") is not acceptable for an audit-grade
> subsystem.

### Region availability failures

**F1. Source region offline.**
* *Cause* : us-east-1 has a regional outage during a replay request
  targeting eu-west-1.
* *Detection* : `ChainShipmentClient::fetch_chain` returns
  `gRPC: Unavailable` after 3 retries (exponential backoff 100ms → 1s
  → 10s).
* *Response* : `ReplayError::SourceRegionUnavailable { region }`,
  HTTP 503 with `Retry-After: 300`.
* *Recovery* : operator re-submits after region recovers ; job state in
  `replay.replay_jobs` returns to `pending`.

**F2. Target region overloaded.**
* *Cause* : N15 swarm DRR saturated.
* *Detection* : `admission_gate.admit_swarm` returns
  `BackpressureKind::SwarmInflightFull`.
* *Response* : Job remains `pending` ; scheduler retries with
  exponential backoff up to 10 attempts ; if all fail, marks job as
  `target_region_busy` and notifies operator.
* *Recovery* : auto-retry when admission frees up ; manual re-prioritise
  endpoint for ops.

**F7. Network timeout fetching chain.**
* *Cause* : transient packet loss.
* *Detection* : gRPC timeout (30s default).
* *Response* : retry with backoff 3 times ; on final fail → F1
  semantics.
* *Recovery* : same as F1.

### Chain integrity failures

**F3. Chain HMAC verification fails.**
* *Cause* : storage corruption, tampering, or key mismatch.
* *Detection* : `verify_chain` returns `Err(HmacMismatch { trace_id })`.
* *Response* : `ReplayError::ChainBroken { trace_id, expected_hmac,
  actual_hmac }`.  Replay job marked `failed_integrity`.  No ops
  re-dispatched.
* *Recovery* : SECURITY incident — ops paged via PagerDuty.  Forensic
  investigation required.

**F8. Storage full in target region.**
* *Cause* : Postgres tablespace exhausted.
* *Detection* : `INSERT` returns `disk_full`.
* *Response* : Job marked `failed_storage` ; admin notified ;
  `ReplayError::TargetStorageExhausted`.
* *Recovery* : ops expands tablespace ; job retried.

**F9. Replay queue table corrupted.**
* *Cause* : Postgres bug, schema mismatch.
* *Detection* : `SELECT ... FROM replay.replay_jobs` returns SQL error.
* *Response* : coordinator panics in a tagged way (`SCHEDULER_QUEUE_
  CORRUPT`) — restart loop with backoff.
* *Recovery* : manual ; runbook A5 documented in annex.

### Catalog & schema failures

**F4. Agent version missing in target region.**
* *Cause* : catalog drift (eu-west-1 lacks v1.2.3).
* *Detection* : `catalog.get_pinned(id, v)` returns `VersionNotFound`.
* *Response* : `ReplayError::AgentVersionMissing { agent_id, version,
  region }`.  Job marked `failed_catalog_drift`.
* *Recovery* : ops promotes missing version to target catalog ;
  re-submit.

**F16. Schema version too new for target.**
* *Cause* : Source ran SDK v0.5 (envelope v5), target on v0.4 (envelope
  v4 max).
* *Detection* : pre-shipment header compare.
* *Response* : `ReplayError::SchemaTooNew { source: 5, target_max: 4 }`,
  HTTP 426 (Upgrade Required).
* *Recovery* : ops upgrades target SDK ; re-submit.

### Authentication / authorization failures

**F10. Operator JWT expired.**
* *Cause* : token TTL exceeded.
* *Detection* : `JwtValidator::validate` returns `Expired`.
* *Response* : 401 Unauthorized.
* *Recovery* : operator re-auths.

**F14. Cross-tenant access attempt.**
* *Cause* : misconfigured operator or malicious actor.
* *Detection* : JWT `tenant_id` ≠ `request.tenant_id`, or RLS row miss.
* *Response* : 403 Forbidden ; security audit trace emitted.
* *Recovery* : security ops investigates.

### Compliance failures

**F5. Residency policy forbids target region.**
* *Cause* : EU customer, target = us-east-1.
* *Detection* : `ResidencyPolicy::validate` returns `Forbidden`.
* *Response* : `ReplayError::ResidencyViolation`.
* *Recovery* : operator picks a permitted target region ; re-submit.

**F6. KMS key not accessible.**
* *Cause* : cross-region KMS grant not configured.
* *Detection* : pre-shipment KMS access check.
* *Response* : `ReplayError::KmsKeyNotAvailable { arn, target_region }`.
* *Recovery* : ops grants cross-region KMS access ; re-submit.

### Concurrency & resource

**F11. Concurrent replay of same swarm.**
* *Cause* : two operators race.
* *Detection* : `(tenant_id, swarm_request_id)` lease UNIQUE constraint
  violation.
* *Response* : 409 Conflict `{ existing_job_id: <J> }`.
* *Recovery* : operator polls existing job for result.

**F12. Nested replay attempted.**
* *Cause* : operator picks a swarm whose envelope already has the
  `"replay"` audit tag.
* *Detection* : envelope tag check at coordinator.
* *Response* : `ReplayError::NestedReplayForbidden`.
* *Recovery* : operator picks an original swarm (not a replay output).

**F13. Mismatch beyond tolerance.**
* *Cause* : real code drift or non-determinism beyond Mode-C threshold.
* *Detection* : `MatchOutcome::Mismatch` count > threshold.
* *Response* : job marked `completed_with_regression` ; structured diff
  stored in `replay.replay_diffs` ; notification with severity=warn.
* *Recovery* : ops investigates ; potentially blocks production rollout.

**F15. Replay exceeds budget.**
* *Cause* : replay scope larger than expected (e.g. 1 000 ops swarm).
* *Detection* : `request.timeout_ms` exceeded.
* *Response* : `ReplayError::Timeout`.  Partial result stored if some
  ops completed.
* *Recovery* : operator increases timeout or splits into smaller jobs.

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1. Cross-region chain transfer uses gRPC over mTLS, not S3 hand-off.

**Context.**  Three options were considered for chain shipment :

1. **S3 hand-off** : source dumps chain to S3 bucket, target downloads.
2. **gRPC streaming** : direct peer-to-peer transfer.
3. **Kafka topic** : multi-region replicated topic.

**Decision.**  gRPC streaming over mTLS, with hash verification.

**Rationale.**
* **Atomicity** (G4 + I17) : gRPC bidirectional streams allow the
  staging+commit two-phase protocol with native error propagation.
  S3 has no commit semantics ; Kafka has at-least-once delivery which
  complicates idempotency.
* **Authentication** : mTLS naturally authenticates source ↔ target
  region pair using per-region certificates issued by EFE PKI.  S3 IAM
  policies are noisier ; Kafka MTLS is workable but ops-heavy.
* **Latency** : direct transfer ~200ms p99 EU → US.  S3 staging hops
  add 1-2s ; Kafka pub+consume adds 500ms.
* **Cost** : in-region S3 + cross-region replication is $$$.  gRPC over
  VPC peering is included in normal traffic costs.
* **Audit** : every shipment is recorded in
  `replay.chain_shipment_audit` with mTLS cert fingerprints — finer-
  grained than S3 access logs.

**Consequences.**
* Each region's `ChainShipmentServer` must run a public gRPC endpoint
  (within EFE's VPC mesh).
* mTLS certs managed by EFE PKI, rotated quarterly (M58 PKI rotation).
* No queue-backed retry semantics — explicit retry in
  `ChainShipmentClient` with backoff.

**Rejected alternatives.**  S3 hand-off doesn't meet G4 atomicity ; Kafka
adds operational complexity disproportionate to single-tenant replay
semantics.

---

### ADR-2. Replay storage is a separate Postgres schema (`replay.*`), not a separate database.

**Context.**  Where do replay results, jobs, diffs live ?

**Decision.**  Logical schema `replay` within the same `efe_m50` database
as `m50.audit_chain` ; ROW-LEVEL SECURITY enforces tenant isolation.

**Rationale.**
* **Atomic transactions** : replay job state changes need to coordinate
  with production state (e.g. read envelope from `m50.swarm_envelope`,
  insert replay result in `replay.replay_results`) — single-DB
  transactions handle this cleanly.
* **Backup unity** : one PITR snapshot covers both production and
  replay schemas.
* **Tenant scaling** : if a tenant has 10x the replay activity,
  `replay.*` tables can be partitioned independently without separating
  DBs.
* **Tooling** : `psql`, `pg_dump`, schema migrations all work as usual.

**Consequences.**
* `m50.*` migrations must NEVER reference `replay.*` (one-way dep).
* RLS policies are SCHEMA-level + TABLE-level (defense in depth).
* Backup retention policy can be different per schema (replay TTL = 90
  days, audit chain TTL = 7 years).

**Rejected alternative.**  Separate replay DB increases ops surface (two
PITR streams, two failover plans) for marginal isolation gain.

---

### ADR-3. KMS multi-region keys with cross-region grants ; signing root via Vault Transit.

**Context.**  Multi-region encryption keys for `full_input` body BLOBs.

**Decision.**  AWS KMS multi-region keys (replicated to 4 regions) for
data-encrypting keys ; HashiCorp Vault Transit for the signing root
that authenticates chain shipments.

**Rationale.**
* KMS multi-region keys give us cross-region decryption capability
  without dragging keys across boundaries manually.  Each tenant has a
  dedicated multi-region key ID ; ops grants cross-region access per
  tenant via Vault-managed policy.
* Vault Transit handles the cross-region signing root because we need
  finer-grained policy (per-tenant signing keys derived from a master
  shared between regions via Vault's Replication).

**Consequences.**
* Two key systems to manage : KMS (data) + Vault (signing).
* Per-tenant Vault path : `transit/sign/tenant-<id>-signing-root`.
* KMS key ID embedded in every `body_store` row (`kms_key_id: String`).

**Rejected alternatives.**  AWS KMS for signing too — but its policy
language is awkward for per-tenant derivations.  GCP Cloud KMS — but
EFE deployment is AWS-primary.

---

### ADR-4. Time-skew handling : coarse 1-second windows for time comparisons.

**Context.**  Replay comparisons involving timestamps (e.g. trace
ordering) must tolerate ms-level clock skew between regions.

**Decision.**  Use 1-second resolution for any time-window comparison.
For absolute timestamp comparison in agent output (e.g. `output.now()`),
the **virtual clock** in `ReplayCallContext` returns the original
swarm's `started_at`.

**Rationale.**
* AWS region clocks drift ~50 ms p99 (NTP sync to chrony) ; 1s window
  is 20× safety margin.
* The virtual clock removes the comparison problem entirely for
  well-behaved agents (the contract is "agents must not read real
  clock").  Mode A agents that violate the contract fail Mode A's
  bit-exact comparison and are flagged in `match_outcome`.

**Consequences.**
* `ReplayCallContext::system_time()` is the **only sanctioned clock
  access** for agents during replay.  N16 ships a clippy lint rule
  blocking direct `std::time::SystemTime::now()` in agent crates.
* Lateness-detection logic (e.g. "trace recorded > 5s after dispatch")
  uses the original timestamp anchored in the chain, not real
  wall-clock at replay time.

---

### ADR-5. Replay scheduler integrates with N15 admission gate via dedicated lane.

**Context.**  How to prevent replay traffic from starving real customer
traffic.

**Decision.**  Extend N15 with a new `BackpressureKind::ReplayRateLimited`
and a dedicated DRR scheduler `replay_op_drr` with capacity = `10 % ×
op_drr.capacity()`.  Tenant weights are scaled : `replay_weight =
max(tier.drr_weight / 10, 1)`.

**Rationale.**
* Single capacity ceiling for replay = predictable upper bound on
  resource use.
* Per-tenant DRR fairness still applies (Enterprise > Free).
* Backpressure kind lets the coordinator surface "replay rate limited
  for tenant X" distinctly from "tenant X hit op_inflight_full".

**Consequences.**
* Sprint 9 modifies `AdmissionGate::builder()` to take an optional
  `with_replay_drr(drr)`.  Defaults to a fresh DRR with capacity =
  `op_drr.capacity() / 10`.
* New SDK variant `BackpressureKind::ReplayRateLimited` — preserves
  the closed-enum invariant ; downstream metric labels updated.

**Rejected alternatives.**  Sharing the production `op_drr` and just
adjusting weight — doesn't bound replay aggregate impact ; one busy
operator could still saturate.

---

### ADR-6. Mode-classified comparison tolerance configurable per-agent.

**Context.**  Mode B local-model agents have intrinsic ~10⁻⁶ numerical
noise (tract-onnx) ; Mode C LLM agents have semantic-but-not-syntactic
match.

**Decision.**  `NanoAgent` gains an optional `replay_tolerance:
ReplayTolerance` field :

```rust
pub struct ReplayTolerance {
    pub mode_a: MatchPolicy,   // default: Exact
    pub mode_b: MatchPolicy,   // default: Structural { float_epsilon: 1e-5 }
    pub mode_c: MatchPolicy,   // default: Semantic { min_score: 0.95 }
}

pub enum MatchPolicy {
    Exact,
    Structural { float_epsilon: f64 },
    Semantic { min_score: f64 },
    Custom(String),  // name of a registered comparator
}
```

**Rationale.**  Agent authors know their determinism profile best ;
defaults work for 90 % of agents ; ops override for outliers.

**Consequences.**
* SDK extension : `ReplayTolerance` + `MatchPolicy` types.
* Catalog migration : `replay_tolerance` column on `m50.nano_agents`.
* N13 `ComparisonStrategy` (Exact / Structural / Semantic / None) is
  the engine — N16 wires per-agent policy into the call.

---

### ADR-7. Replay cost attribution : `CostAttribution::Operator(operator_id)`.

**Context.**  Mode C replays issue real LLM calls = real cost.  Who pays ?

**Decision.**  `CostAttribution::Operator(operator_id)` ; M31 billing
charges the EFE-internal operator account, NOT the customer.

**Rationale.**
* Replays are operator-initiated (by definition) ; customer billing
  would surprise customers reading their invoices.
* Separates operational forensic cost from customer revenue accounting.

**Consequences.**
* SDK extension : `CostAttribution::Operator(operator_id)` variant.
* M31 billing : aggregates per-operator monthly into a "replay cost
  budget" line item.
* Operator dashboard surfaces replay cost YTD for budget management.

---

### ADR-8. Multi-tenant replay schema isolation : RLS + trait-level checks.

**Context.**  How to prevent cross-tenant reads in `replay.*` tables.

**Decision.**  Defense in depth :
1. Postgres RLS policy : `tenant_id = current_setting('app.current_tenant
   _id')::uuid` on every `replay.*` table.
2. Trait-level assertion : every public method on `MultiRegionReplay
   Engine` validates `tenant_id` against the `OperatorClaims.tenant_id`.
3. Audit trace on every access ; cross-tenant attempt → security alert.

**Rationale.**  Single layer (RLS only) is fragile (misconfigured
`SET app.current_tenant_id` bypasses).  Single layer (trait only) is
bypassable via direct SQL.  Two layers fail-closed.

**Consequences.**
* Schema migration ships RLS policies on day 1.
* Engine layer enforces app-level invariant ; mismatch is a panic in
  debug, security alert in release.

---

### ADR-9. Region taxonomy : fixed enum, not a String.

**Context.**  `RegionCode` typing.

**Decision.**  Closed enum :

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegionCode {
    UsEast1,
    UsWest2,
    EuWest1,
    ApSoutheast1,
    /// Reserved for future regions ; carries a String discriminant.
    /// Forbidden in stable / production contexts ; allowed in dev.
    Custom(String),
}
```

**Rationale.**
* Closed enum = catalog of supported regions visible to type checker.
* `Custom(String)` escape hatch for dev clusters, not production.
* Serde `kebab-case` keeps wire format `"us-east-1"`.

**Consequences.**
* New region requires SDK release + workspace upgrade ; deliberate
  friction matches the operational cost of adding a region.
* `Custom` variant gated behind a `cfg(feature = "non-prod-regions")`
  on the SDK ; production builds compile WITHOUT it.

---

### ADR-10. Operator authentication : JWT with `replay:execute` scope.

**Context.**  Who can trigger replays ?

**Decision.**  Bearer JWT with claims :
```jsonc
{
  "sub": "operator-username",
  "operator_id": "uuid",
  "scope": ["replay:execute", "replay:read"],
  "tenant_id": "uuid",          // tenant the operator is acting on behalf of
  "ticket_id": "JIRA-1234",
  "iat": 1234567890,
  "exp": 1234571490             // 1h TTL
}
```

**Rationale.**
* Short TTL (1h) bounds the blast radius of leaked tokens.
* `ticket_id` mandatory in the token = forensic linkage to a real
  human-tracked decision.
* `scope` allows fine-grained delegation (e.g. read-only operators).

**Consequences.**
* New crate `efe-replay-auth` (Sprint 9 day 1 deliverable) implements
  `JwtValidator`.
* JWT signing key managed by Vault Transit ; rotated weekly.
* Audit chain records JWT fingerprint (NOT the token) on every replay
  submit.

---

## 7. Component architecture

### 7.1 The 8 components — at a glance

```text
                            ┌──────────────────┐
                            │  Operator HTTP   │
                            │   (Sprint 9 -    │
                            │  /replay/submit) │
                            └────────┬─────────┘
                                     │ ReplayRequest + JWT
                                     ▼
   ┌──────────────────────────────────────────────────────────────┐
   │                  ReplayCoordinator (target region)            │
   │  ┌──────────┐  ┌──────────────┐  ┌────────────────────────┐  │
   │  │ JWT auth │→ │ Residency    │→ │ Concurrent lease       │  │
   │  └──────────┘  │ policy check │  └────────────────────────┘  │
   │                └──────────────┘                                │
   │                       │                                        │
   │                       ▼                                        │
   │         ┌──────────────────────────────────┐                   │
   │         │   ChainShipmentClient (gRPC mTLS) │                   │
   │         └──────────────┬───────────────────┘                   │
   │                        │ fetch chain from source                │
   │                        ▼                                        │
   │         ┌──────────────────────────────────┐                   │
   │         │  N15 AdmissionGate (replay lane)  │                   │
   │         └──────────────┬───────────────────┘                   │
   │                        │ AdmissionTicket                        │
   │                        ▼                                        │
   │         ┌──────────────────────────────────┐                   │
   │         │   MultiRegionReplayEngine         │                   │
   │         │   - wraps Sprint 6 ReplayEngine   │                   │
   │         │   - injects ReplayCallContext     │                   │
   │         │   - per-mode tolerance            │                   │
   │         └──────────────┬───────────────────┘                   │
   │                        │ ReplayResult                           │
   │                        ▼                                        │
   │         ┌──────────────────────────────────┐                   │
   │         │ replay.*  Postgres schema         │                   │
   │         │ (jobs / results / diffs / chain)  │                   │
   │         └──────────────────────────────────┘                   │
   └──────────────────────────────────────────────────────────────┘
                                     ▲
                                     │ gRPC mTLS
                                     │ chain bytes
   ┌──────────────────────────────────────────────────────────────┐
   │            ChainShipmentServer (source region)                │
   │   - reads m50.audit_chain + m50.body_store                    │
   │   - verifies + signs                                          │
   │   - streams to target                                         │
   └──────────────────────────────────────────────────────────────┘
```

### 7.2 ReplayCoordinator

**Purpose** : Front door of N16.  Owns the workflow from
`ReplayRequest` to `ReplayResult`.

**Lives** : `crates/efe-replay-coordinator/` (new crate).

**Public surface** :

```rust
pub struct ReplayCoordinator {
    jwt_validator: Arc<JwtValidator>,
    residency_policy: Arc<dyn ResidencyPolicyLookup>,
    catalog: Arc<dyn Catalog>,
    chain_shipment_client: Arc<ChainShipmentClient>,
    admission_gate: Arc<AdmissionGate>,
    engine: Arc<MultiRegionReplayEngine>,
    storage: Arc<dyn ReplayStorage>,
    kms_resolver: Arc<dyn KmsKeyResolver>,
    metrics: Option<Arc<N16Metrics>>,
    region: RegionCode,
}

impl ReplayCoordinator {
    pub async fn submit(
        &self,
        request: ReplayRequest,
        jwt: &str,
    ) -> Result<ReplayJobId, ReplayError>;

    pub async fn status(
        &self,
        job_id: ReplayJobId,
        jwt: &str,
    ) -> Result<ReplayJobStatus, ReplayError>;

    pub async fn result(
        &self,
        job_id: ReplayJobId,
        jwt: &str,
    ) -> Result<ReplayResult, ReplayError>;

    pub async fn cancel(
        &self,
        job_id: ReplayJobId,
        jwt: &str,
    ) -> Result<(), ReplayError>;
}
```

**Submit lifecycle** (matches I15) :

1. Validate JWT (`jwt_validator.validate`) — returns `OperatorClaims`.
2. Check claims.scope contains `replay:execute`.
3. Validate attribution (`request.actor`, `request.reason`, claims.
   `ticket_id` all set — G14).
4. Lookup residency policy ; check `target_region` permitted (G1 + F5).
5. Check for nested replay (read envelope, check audit_tags — G7 + F12).
6. Lookup envelope to extract `expected_agent_versions` ; validate
   against target catalog (G3 + F4).
7. Enumerate KMS keys referenced ; check access (G12 + F6).
8. Acquire concurrent-replay lease (G15 + F11).
9. Insert `ReplayJob { status: Pending, ... }` into `replay.replay_jobs`.
10. Return `ReplayJobId` ; background worker picks up.

The async background worker (a `tokio::task` spawned by the
coordinator) :
* Polls `WHERE status = 'pending' LIMIT N FOR UPDATE SKIP LOCKED`.
* Per job : acquires N15 admission ticket ; fetches chain via
  `ChainShipmentClient` ; invokes `engine.replay(...)` ; persists
  result.

### 7.3 ChainShipmentProtocol

**Purpose** : Cross-region transfer of an HMAC chain + body BLOBs +
per-tenant signing key.

**Lives** : `crates/efe-chain-shipment/` (new crate).

**Wire protocol** (gRPC, file `chain_shipment.proto`) :

```proto
syntax = "proto3";
package efe.chain_shipment.v1;

service ChainShipment {
    rpc StageChain(StageChainRequest) returns (stream StageChainResponse);
    rpc VerifyStaging(VerifyRequest) returns (VerifyResponse);
    rpc CommitStaging(CommitRequest) returns (CommitResponse);
    rpc AbortStaging(AbortRequest) returns (AbortResponse);
}

message StageChainRequest {
    string source_region = 1;
    string target_region = 2;
    bytes  tenant_id = 3;            // uuid
    bytes  swarm_request_id = 4;     // uuid
    bytes  envelope_trace_id = 5;    // uuid
    uint32 sdk_schema_version = 6;
    bytes  signing_root_cert = 7;    // X.509 of source region
}

message StageChainResponse {
    oneof payload {
        ChainHeader header = 1;
        TraceRow row = 2;
        BodyBlob body = 3;
        ShipmentFooter footer = 4;
    }
}

message ChainHeader {
    bytes head_hash = 1;             // Blake3 of concatenated row hashes
    uint64 row_count = 2;
    bytes derived_hmac_key = 3;      // per-tenant key, signed by signing root
    bytes derived_hmac_key_sig = 4;  // signature
}

message TraceRow {
    bytes trace_id = 1;
    bytes canonical_row_body = 2;    // sprint 6 canonical form
    bytes hmac = 3;
    repeated string kms_key_arns = 4;
}

message BodyBlob {
    bytes trace_id = 1;
    string field = 2;                 // "full_input" | "full_output"
    bytes ciphertext = 3;
    string kms_key_arn = 4;
}

message ShipmentFooter {
    bytes total_signature = 1;       // signature over (header + all rows + bodies)
}
```

**Atomic-shipment guarantee** (G4 + I17) :

1. `StageChain` (streaming) : source writes everything to a transient
   staging area in target (`replay.staging`).
2. `VerifyStaging` : target re-computes head hash + verifies HMAC chain
   + verifies `total_signature`.  Returns `{ ok, errors }`.
3. `CommitStaging` : target moves staging → live (atomic table swap
   with row-level lock).
4. `AbortStaging` : staging GC'd ; idempotent.

Staging area auto-GC'd after 5 minutes if no commit.

### 7.4 MultiRegionReplayEngine

**Purpose** : Wraps Sprint 6 `ReplayEngine` with region context + agent
version pinning + result diff storage.

**Lives** : `crates/efe-nano/src/replay/multi_region.rs` (extension of
existing `replay/` module).

```rust
pub struct MultiRegionReplayEngine {
    inner: Arc<crate::replay::ReplayEngine>,
    catalog: Arc<dyn Catalog>,
    dispatcher: Arc<Dispatcher>,
    storage: Arc<dyn ReplayStorage>,
    n16_metrics: Option<Arc<N16Metrics>>,
    region: RegionCode,
}

impl MultiRegionReplayEngine {
    pub async fn replay(
        &self,
        ctx: ReplayCallContext,
    ) -> Result<ReplayResult, ReplayError>;
}

pub struct ReplayCallContext {
    pub job_id: ReplayJobId,
    pub tenant_id: TenantId,
    pub origin_region: RegionCode,
    pub target_region: RegionCode,
    pub swarm_request_id: Uuid,
    pub envelope_trace_id: TraceId,
    pub expected_agent_versions: HashMap<NanoAgentId, NanoAgentVersion>,
    pub virtual_clock: SystemTime,           // ADR-4
    pub mode_tolerance: ReplayToleranceMap,
    pub cost_attribution: CostAttribution,    // Operator(_)
    pub admission_ticket: AdmissionTicket,
    pub deadline: Instant,
}
```

`replay(ctx)` :
1. Inserts `ReplayDispatched` audit trace into `replay.replay_audit_chain`.
2. Calls `inner.replay(...)` with the version-pinned catalog (catalog
   wrapper that always returns the recorded version).
3. Reads original ops from `m50.audit_chain` (already in target via
   shipment).
4. Per op : compute `MatchOutcome` via N13 `ComparisonStrategy` selected
   by mode + tolerance config.
5. Insert per-op `ReplayCompared` trace + structured diff in
   `replay.replay_diffs`.
6. Aggregate `ReplayResult` ; insert `ReplayCompleted` envelope trace.
7. Release admission ticket on drop.

### 7.5 Per-mode comparison strategy

The comparison decision tree :

```text
┌─────────────────────────┐
│ original.status         │
├─────────────────────────┤
│ if != replay.status     │ → MatchOutcome::Mismatch (regardless of mode)
│                         │
│ if   == replay.status   │
│      and == Success     │ → mode-specific output comparison
│                         │
│ if   == Failed or       │ → ErrorKind comparison only
│      == Skipped         │
└─────────────────────────┘

Per-mode (when both Success) :
  Mode A : Exact byte comparison.    threshold = 100%
  Mode B : Structural with float ε.  threshold ≥ 99.9%
  Mode C : N13 Semantic comparator.  threshold ≥ 95% (configurable)
```

### 7.6 ResidencyPolicyLookup

```rust
#[async_trait]
pub trait ResidencyPolicyLookup: Send + Sync + 'static {
    async fn policy(&self, tenant: TenantId) -> Result<ResidencyPolicy, ResidencyError>;
    fn validate(&self, policy: &ResidencyPolicy, region: RegionCode) -> Result<(), ResidencyViolation>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResidencyPolicy {
    /// Data may reside in any region (default for non-EU tenants).
    Global,
    /// US-only : data must stay in {us-east-1, us-west-2}.
    UsOnly,
    /// EU-only (GDPR Article 44 compliant).
    EuOnly,
    /// APAC-only.
    ApacOnly,
    /// Custom allow-list.
    Strict { allowed: Vec<RegionCode> },
    /// Like Strict but redacts PII before any cross-residency transfer.
    RedactOnTransfer { allowed: Vec<RegionCode> },
}
```

`InMemoryResidencyPolicyLookup` ships in Sprint 9 ; Postgres-backed
version follows in Sprint 10.

### 7.7 ReplayStorage trait

```rust
#[async_trait]
pub trait ReplayStorage: Send + Sync + 'static {
    async fn insert_job(&self, job: ReplayJob) -> Result<(), StorageError>;
    async fn update_status(&self, id: ReplayJobId, status: ReplayJobStatus) -> Result<(), StorageError>;
    async fn fetch_pending(&self, limit: usize) -> Result<Vec<ReplayJob>, StorageError>;
    async fn insert_result(&self, result: ReplayResult) -> Result<(), StorageError>;
    async fn fetch_result(&self, id: ReplayJobId, tenant: TenantId) -> Result<Option<ReplayResult>, StorageError>;
    async fn insert_diff(&self, diff: ReplayOpDiff) -> Result<(), StorageError>;
    async fn lease(&self, tenant: TenantId, swarm_id: Uuid) -> Result<ReplayLease, StorageError>;
    async fn release_lease(&self, lease: ReplayLease) -> Result<(), StorageError>;
}
```

Two impls : `PostgresReplayStorage` (production) + `InMemoryReplayStorage`
(tests).

### 7.8 N16 metrics surface

Ten new Prometheus metrics in `n16_*` namespace :

| # | Name | Type | Labels |
|---|---|---|---|
| 1 | `n16_replay_requests_total` | counter | `target_region`, `outcome` |
| 2 | `n16_replay_jobs_inflight` | gauge | `target_region` |
| 3 | `n16_replay_duration_seconds` | histogram | `mode` (A/B/C/mixed) |
| 4 | `n16_chain_shipment_bytes_total` | counter | `source_region`, `target_region` |
| 5 | `n16_chain_shipment_duration_seconds` | histogram | `source_region`, `target_region` |
| 6 | `n16_residency_violations_total` | counter | `target_region`, `policy` |
| 7 | `n16_match_outcome_total` | counter | `mode`, `outcome` (exact/tolerated/mismatch) |
| 8 | `n16_cost_cents_total` | counter | `operator_id` |
| 9 | `n16_concurrent_replay_collisions_total` | counter | (none) |
|10 | `n16_kms_access_denied_total` | counter | `target_region` |

---

## 8. Module hierarchy & data model

### 8.1 Crate layout

```text
crates/
├── efe-nano-sdk/                  (existing) — SDK
│   └── src/types/
│       ├── replay.rs              (NEW) — ReplayRequest, ReplayResult, etc.
│       ├── region.rs              (NEW) — RegionCode, ResidencyPolicy
│       └── ...
├── efe-nano/                      (existing) — runtime
│   └── src/replay/
│       ├── multi_region.rs        (NEW) — MultiRegionReplayEngine
│       └── ...
├── efe-replay-coordinator/        (NEW crate) — operator front door
│   ├── src/
│   │   ├── lib.rs
│   │   ├── coordinator.rs
│   │   ├── jwt.rs
│   │   ├── residency.rs
│   │   ├── storage/
│   │   │   ├── trait.rs
│   │   │   ├── postgres.rs
│   │   │   └── memory.rs
│   │   ├── metrics.rs
│   │   └── http.rs                (Axum endpoint handlers)
│   └── Cargo.toml
├── efe-chain-shipment/            (NEW crate) — gRPC transfer
│   ├── src/
│   │   ├── lib.rs
│   │   ├── client.rs
│   │   ├── server.rs
│   │   ├── proto.rs               (generated from .proto)
│   │   └── verify.rs
│   ├── proto/
│   │   └── chain_shipment.proto
│   └── Cargo.toml
└── efe-replay-auth/               (NEW crate) — JWT validation
    ├── src/
    │   ├── lib.rs
    │   ├── validator.rs
    │   └── claims.rs
    └── Cargo.toml
```

### 8.2 SDK types — `efe_nano_sdk::types::replay`

```rust
/// **Sprint 9 — N16** : a replay request submitted to the
/// ReplayCoordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRequest {
    pub tenant_id: TenantId,
    pub swarm_request_id: Uuid,
    pub origin_region: RegionCode,
    pub target_region: RegionCode,
    /// Operator-side justification ; required (G14).
    pub actor: String,
    pub ticket_id: String,
    pub reason: String,
    /// Per-swarm timeout ; hard cap 60 minutes (I16).
    pub timeout_ms: u32,
    /// Optional override of per-mode tolerance.  None → use catalog default.
    pub tolerance_override: Option<ReplayToleranceMap>,
    /// Replay mode (DispatchOnly skips real LLM calls).
    pub mode: ReplayMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayMode {
    /// Real re-dispatch ; Mode C makes real LLM calls (billed to operator).
    Full,
    /// Mode C uses a stub LLM returning recorded outputs ; cheaper.
    DispatchOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayJobId(pub Uuid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplayJobStatus {
    Pending,
    InProgress { progress_pct: u8 },
    Completed { result_id: Uuid },
    CompletedWithRegression { result_id: Uuid, mismatch_count: u32 },
    Failed { error: ReplayError },
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResult {
    pub job_id: ReplayJobId,
    pub tenant_id: TenantId,
    pub origin_region: RegionCode,
    pub target_region: RegionCode,
    pub swarm_request_id: Uuid,
    pub envelope_trace_id: TraceId,
    pub per_op: HashMap<String, ReplayOpDiff>,
    pub aggregate: ReplayAggregate,
    pub cost_attribution: CostAttribution,
    pub replay_started_at: SystemTime,
    pub replay_completed_at: SystemTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayOpDiff {
    pub op_id: String,
    pub mode: ExecutionMode,
    pub match_outcome: MatchOutcome,
    pub original_status: OpStatus,
    pub replay_status: OpStatus,
    pub structural_diff: Option<serde_json::Value>,
    pub semantic_score: Option<f64>,
    pub latency_ms_original: u32,
    pub latency_ms_replay: u32,
    pub cost_cents_original: f64,
    pub cost_cents_replay: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchOutcome {
    /// Output bit-exact or semantic match.
    Exact,
    /// Within configured tolerance.
    Tolerated { score: f64 },
    /// Beyond tolerance — possible regression.
    Mismatch,
    /// Op skipped in either original or replay.
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayAggregate {
    pub total_ops: u32,
    pub exact: u32,
    pub tolerated: u32,
    pub mismatch: u32,
    pub skipped: u32,
    pub overall_outcome: ReplayOverallOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplayOverallOutcome {
    /// Every op matched within tolerance.
    Clean,
    /// One or more ops mismatched.
    Regression,
    /// Engine errored before completion.
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum ReplayError {
    #[error("residency violation : policy {policy:?} forbids replay in {target_region:?}")]
    ResidencyViolation { policy: ResidencyPolicy, target_region: RegionCode },

    #[error("agent {agent_id} v{version} not in target region {region:?}")]
    AgentVersionMissing { agent_id: NanoAgentId, version: NanoAgentVersion, region: RegionCode },

    #[error("KMS keys not accessible in target region : {arns:?}")]
    KmsKeyNotAvailable { arns: Vec<String>, target_region: RegionCode },

    #[error("nested replays are forbidden")]
    NestedReplayForbidden,

    #[error("concurrent replay already in flight : job {existing_job_id}")]
    ConcurrentReplayInFlight { existing_job_id: ReplayJobId },

    #[error("source region {region:?} unavailable")]
    SourceRegionUnavailable { region: RegionCode },

    #[error("chain integrity broken at trace {trace_id}")]
    ChainBroken { trace_id: Uuid },

    #[error("schema too new : source {source}, target max {target_max}")]
    SchemaTooNew { source: u32, target_max: u32 },

    #[error("operator attribution missing or invalid")]
    AttributionInvalid,

    #[error("replay timeout after {0} ms")]
    Timeout(u32),

    #[error("internal error : {0}")]
    Internal(String),
}
```

### 8.3 SDK types — `efe_nano_sdk::types::region`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegionCode {
    UsEast1,
    UsWest2,
    EuWest1,
    ApSoutheast1,
    #[cfg(feature = "non-prod-regions")]
    Custom(String),
}

impl RegionCode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::UsEast1 => "us-east-1",
            Self::UsWest2 => "us-west-2",
            Self::EuWest1 => "eu-west-1",
            Self::ApSoutheast1 => "ap-southeast-1",
            #[cfg(feature = "non-prod-regions")]
            Self::Custom(s) => s,
        }
    }
    pub fn is_us(self) -> bool { matches!(self, Self::UsEast1 | Self::UsWest2) }
    pub fn is_eu(self) -> bool { matches!(self, Self::EuWest1) }
    pub fn is_apac(self) -> bool { matches!(self, Self::ApSoutheast1) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResidencyPolicy {
    Global,
    UsOnly,
    EuOnly,
    ApacOnly,
    Strict { allowed: Vec<RegionCode> },
    RedactOnTransfer { allowed: Vec<RegionCode> },
}
```

### 8.4 Postgres schemas

```sql
-- Schema for replay subsystem
CREATE SCHEMA replay;

-- Row-Level Security : every replay table partitioned by tenant.
CREATE TABLE replay.replay_jobs (
    job_id              UUID PRIMARY KEY,
    tenant_id           UUID NOT NULL,
    swarm_request_id    UUID NOT NULL,
    origin_region       TEXT NOT NULL,
    target_region       TEXT NOT NULL,
    status              TEXT NOT NULL,          -- json-encoded ReplayJobStatus
    actor               TEXT NOT NULL,
    ticket_id           TEXT NOT NULL,
    reason              TEXT NOT NULL,
    mode                TEXT NOT NULL,
    timeout_ms          INTEGER NOT NULL,
    submitted_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    operator_jwt_fp     TEXT NOT NULL           -- SHA-256 fingerprint of JWT
);
CREATE INDEX ON replay.replay_jobs (tenant_id, status, submitted_at);
ALTER TABLE replay.replay_jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON replay.replay_jobs
    USING (tenant_id = current_setting('app.current_tenant_id')::uuid);

CREATE TABLE replay.replay_results (
    job_id              UUID PRIMARY KEY REFERENCES replay.replay_jobs(job_id),
    tenant_id           UUID NOT NULL,
    aggregate_json      JSONB NOT NULL,
    cost_cents          NUMERIC(18, 4) NOT NULL,
    operator_id         UUID NOT NULL,
    replay_started_at   TIMESTAMPTZ NOT NULL,
    replay_completed_at TIMESTAMPTZ NOT NULL,
    region              TEXT NOT NULL
);
ALTER TABLE replay.replay_results ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON replay.replay_results
    USING (tenant_id = current_setting('app.current_tenant_id')::uuid);

CREATE TABLE replay.replay_diffs (
    job_id              UUID NOT NULL REFERENCES replay.replay_jobs(job_id),
    tenant_id           UUID NOT NULL,
    op_id               TEXT NOT NULL,
    mode                TEXT NOT NULL,
    match_outcome       TEXT NOT NULL,
    structural_diff     JSONB,
    semantic_score      DOUBLE PRECISION,
    PRIMARY KEY (job_id, op_id)
);
ALTER TABLE replay.replay_diffs ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON replay.replay_diffs
    USING (tenant_id = current_setting('app.current_tenant_id')::uuid);

CREATE TABLE replay.replay_lease (
    tenant_id           UUID NOT NULL,
    swarm_request_id    UUID NOT NULL,
    job_id              UUID NOT NULL,
    leased_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, swarm_request_id)
);
-- Lease only valid for 30 minutes ; expired leases must be released.

CREATE TABLE replay.replay_audit_chain (
    trace_id            UUID PRIMARY KEY,
    tenant_id           UUID NOT NULL,
    job_id              UUID NOT NULL,
    sequence_number     BIGINT NOT NULL,
    trace_type          TEXT NOT NULL,
    canonical_row_body  BYTEA NOT NULL,
    hmac                BYTEA NOT NULL,
    prev_trace_id       UUID,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
ALTER TABLE replay.replay_audit_chain ENABLE ROW LEVEL SECURITY;
CREATE POLICY tenant_isolation ON replay.replay_audit_chain
    USING (tenant_id = current_setting('app.current_tenant_id')::uuid);

CREATE TABLE replay.staging (
    staging_id          UUID PRIMARY KEY,
    tenant_id           UUID NOT NULL,
    swarm_request_id    UUID NOT NULL,
    source_region       TEXT NOT NULL,
    head_hash           BYTEA NOT NULL,
    sdk_schema_version  INTEGER NOT NULL,
    chain_bytes         BYTEA NOT NULL,
    body_blobs          BYTEA NOT NULL,
    signing_root_cert   BYTEA NOT NULL,
    staged_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
-- staging GC : DELETE FROM replay.staging WHERE staged_at < NOW() - INTERVAL '5 minutes';
```

---

## 9. API surface (HTTP + internal Rust traits)

### 9.1 HTTP endpoints

```text
POST /v1/replay
    Auth: Bearer <JWT>
    Body: ReplayRequest (JSON)
    → 202 Accepted { job_id }
    → 400 Bad Request { error: "AttributionInvalid" }
    → 401 Unauthorized
    → 403 Forbidden { error: "ResidencyViolation" }
    → 404 Not Found { error: "AgentVersionMissing" }
    → 409 Conflict { existing_job_id, error: "ConcurrentReplayInFlight" }
    → 426 Upgrade Required { error: "SchemaTooNew" }
    → 503 Service Unavailable { error: "SourceRegionUnavailable", retry_after_seconds }

GET /v1/replay/{job_id}/status
    Auth: Bearer <JWT>
    → 200 OK ReplayJobStatus (JSON)
    → 401 / 403 / 404

GET /v1/replay/{job_id}
    Auth: Bearer <JWT>
    → 200 OK ReplayResult (JSON)
    → 401 / 403 / 404 / 425 Too Early (if InProgress)

DELETE /v1/replay/{job_id}
    Auth: Bearer <JWT>
    → 204 No Content
    → 401 / 403 / 404 / 409 (if already Completed)

GET /v1/replay
    Auth: Bearer <JWT>
    Query: ?tenant_id=...&status=pending&limit=50&offset=0
    → 200 OK [ReplayJob]

GET /v1/replay/regions
    → 200 OK { regions: ["us-east-1", "eu-west-1", ...] }

GET /healthz
    → 200 OK { region, version, db: "ok", chain_shipment: "ok" }
```

### 9.2 Internal Rust traits

```rust
#[async_trait]
pub trait JwtValidator: Send + Sync + 'static {
    async fn validate(&self, token: &str) -> Result<OperatorClaims, JwtError>;
}

#[async_trait]
pub trait KmsKeyResolver: Send + Sync + 'static {
    async fn check_access(
        &self,
        target_region: RegionCode,
        kms_arn: &str,
    ) -> Result<(), KmsError>;
    async fn check_access_bulk(
        &self,
        target_region: RegionCode,
        arns: &[String],
    ) -> Result<Vec<String>, KmsError>;  // returns inaccessible ARNs
}

#[async_trait]
pub trait ChainShipmentClient: Send + Sync + 'static {
    async fn fetch_chain(
        &self,
        source_region: RegionCode,
        target_region: RegionCode,
        tenant_id: TenantId,
        swarm_request_id: Uuid,
    ) -> Result<StagedChainRef, ChainShipmentError>;

    async fn commit_staging(&self, staged: StagedChainRef) -> Result<(), ChainShipmentError>;
    async fn abort_staging(&self, staged: StagedChainRef) -> Result<(), ChainShipmentError>;
}
```

---

## 10. End-to-end workflows

### 10.1 Workflow A — Single-swarm replay (happy path)

```
1. Operator: POST /v1/replay  (JWT, body = ReplayRequest)
2. Coordinator: JWT validation       (G10, I10)
3. Coordinator: attribution check    (G14, I15.2)
4. Coordinator: residency lookup     (G1, F5, I15.3)
5. Coordinator: nested-replay check  (G7, I15.4)
6. Coordinator: catalog version check (G3, F4, I15.5)
7. Coordinator: KMS access check     (G12, F6, I15.6)
8. Coordinator: acquire lease        (G15, F11)
9. Coordinator: insert job (status=pending)
10. → return 202 Accepted { job_id }

[ async worker picks up ]
11. Worker: AdmissionGate.admit_swarm(tenant, op_count) → AdmissionTicket
12. Worker: ChainShipmentClient.fetch_chain(source, target, ...)
    → 12a. StageChain (streamed)
    → 12b. VerifyStaging  (head hash + HMAC verification)
    → 12c. CommitStaging
13. Worker: MultiRegionReplayEngine.replay(ctx)
    → 13a. Insert ReplayDispatched trace
    → 13b. For each op : re-dispatch with pinned version + virtual clock
    → 13c. Compare per N13 ComparisonStrategy + per-mode tolerance
    → 13d. Insert per-op ReplayCompared trace + structured diff
14. Worker: Insert ReplayResult (status=Completed) ; release lease
15. Worker: Drop AdmissionTicket → DRR slot freed

[ operator polls ]
16. GET /v1/replay/{job_id}/status → Completed
17. GET /v1/replay/{job_id}        → ReplayResult JSON
```

### 10.2 Workflow B — Bulk replay (DR drill)

Operator wants to replay 200 swarms in us-west-2 to validate the standby
region during a planned us-east-1 quiet period.

```
1. Operator: POST /v1/replay/bulk (NEW endpoint)
    Body: { swarm_request_ids: [s_1, s_2, ..., s_200], origin: UsEast1, target: UsWest2,
            actor, ticket_id, reason }
2. Coordinator: per-swarm validation (steps 2-7 from Workflow A)
3. Coordinator: per-swarm lease acquisition ; collisions reported in batch
   response.
4. Coordinator: bulk insert ; return job IDs.
5. Worker: drains jobs paced by N15 admission gate (effectively at
   `replay_op_drr` capacity = 10 % of op_drr).
```

### 10.3 Workflow C — Compliance audit replay

EU regulator requests forensic replay of tenant T's swarms from
2026-Q1.

```
1. Operator (compliance team): JWT with replay:execute, ticket_id = audit-2026-Q1
2. POST /v1/replay  with target_region=EuWest1 (matches T's ResidencyPolicy)
3. Coordinator validates residency → ResidencyPolicy::EuOnly, target=EuWest1 → OK
4. Worker fetches chain from source = EuWest1 (already in EU) → no
   cross-border transfer
5. Replay executes ; ReplayResult signed via N16 attestation root
6. Compliance team downloads result + signature ; presents to regulator
```

### 10.4 Workflow D — Catalog drift detection

Platform team deployed v2 to eu-west-1 ; want to replay 50 v1 swarms
in EU before global rollout.

```
1. Platform team: POST /v1/replay with mode=DispatchOnly (no real LLM cost)
2. Coordinator: catalog version check : v1 must be in EU catalog
   (it still is, deployment was additive) → OK
3. Replay runs ; outputs compared against original v1 outputs
4. Result : 8 mismatches reported → block v2 rollout, investigate
```

### 10.5 Workflow E — Network partition recovery

Chain shipment from us-east-1 to eu-west-1 fails mid-stream at row 50/100.

```
1. ChainShipmentClient detects gRPC stream error.
2. Calls AbortStaging(staging_id) — idempotent ; staging area GC'd.
3. Retries entire StageChain with exponential backoff (3 attempts).
4. If all retries fail : Coordinator marks job status=Failed with
   SourceRegionUnavailable.
5. Operator notified ; manual re-submit when source recovers.
```

---

## 11. Five-day implementation plan

### Day 1 — SDK types + region enum

* `efe-nano-sdk/src/types/region.rs` — `RegionCode`, `ResidencyPolicy`.
* `efe-nano-sdk/src/types/replay.rs` — `ReplayRequest`, `ReplayJob`,
  `ReplayResult`, `MatchOutcome`, `ReplayMode`, `ReplayError`.
* `efe-nano-sdk/src/lib.rs` — re-exports.
* SDK schema version bump : `SCHEMA_VERSION = 5`.
* 40+ unit tests : serde round-trip, region helpers, default policies.

**Compile gate** : `cargo check -p efe-nano-sdk`.
**Test gate** : `cargo test -p efe-nano-sdk --lib region replay`.

### Day 2 — Replay coordinator skeleton + storage

* `efe-replay-coordinator/` new crate.
* `JwtValidator` trait + `Hs256JwtValidator` impl.
* `ResidencyPolicyLookup` trait + in-memory impl.
* `KmsKeyResolver` trait + stub impl (returns Ok in tests).
* `ReplayStorage` trait + in-memory impl.
* `ReplayCoordinator::submit` happy path + 8 rejection paths (F4, F5,
  F6, F10, F12, F14, F16, plus auth-fail).
* HTTP layer with Axum : POST /v1/replay + status + result.
* 30+ unit tests.

**Compile gate** : `cargo check -p efe-replay-coordinator`.

### Day 3 — Chain shipment gRPC

* `efe-chain-shipment/` new crate.
* `chain_shipment.proto` + tonic-generated Rust stubs.
* `ChainShipmentClient` : `StageChain` streaming, `VerifyStaging`,
  `CommitStaging`, `AbortStaging`.
* `ChainShipmentServer` : same RPCs on the source side.
* Atomic-shipment unit tests (mock gRPC).
* Cross-region integration test using two in-process servers.

**Compile gate** : `cargo check -p efe-chain-shipment`.

### Day 4 — Multi-region replay engine

* `efe-nano/src/replay/multi_region.rs` — `MultiRegionReplayEngine`.
* `ReplayCallContext` — virtual clock, version map, admission ticket.
* Catalog version pinning via `Catalog::get_pinned` (new trait method).
* Per-mode comparison via `MatchOutcome` calculation.
* Result diff storage via `ReplayStorage::insert_diff`.
* N15 admission gate wiring : new `BackpressureKind::ReplayRateLimited`,
  new `replay_op_drr` field on `AdmissionGate`.
* SDK update : `BackpressureKind` adds `ReplayRateLimited` variant.
* 50+ unit tests.

### Day 5 — Integration tests + N16 metrics + final verification

* End-to-end integration tests (`tests/phase_3_n16_*.rs`) :
  * `phase_3_n16_residency_blocks_eu_to_us`
  * `phase_3_n16_redaction_pre_shipment`
  * `phase_3_n16_cross_tenant_read_blocked`
  * `phase_3_n16_kms_unavailable_short_circuits`
  * `phase_3_n16_cross_region_chain_verifies`
  * `phase_3_n16_version_drift_rejected`
  * `phase_3_n16_shipment_atomic_on_partition`
  * `phase_3_n16_schema_too_new_rejected`
  * `phase_3_n16_replay_does_not_starve_real_traffic`
  * `phase_3_n16_no_nested_replays`
  * `phase_3_n16_replay_respects_timeout`
  * `phase_3_n16_missing_attribution_rejected`
  * `phase_3_n16_concurrent_replay_rejected`
  * `phase_3_n16_mode_c_tolerance_applied`
  * `phase_3_n16_virtual_clock_deterministic_replay`
* `N16Metrics` surface + Prometheus rendering.
* Final verification : `cargo test --workspace`, `cargo clippy --workspace
  --all-targets -- -D warnings`, full audit doc cross-reference check.

---

## 12. Test strategy

### 12.1 Test pyramid

```
                   Integration tests (Day 5)            ~ 20 tests
              ┌────────────────────────────────┐
              │ Full workflow: HTTP → result    │
              │ End-to-end: source + target     │
              │   in-process via two engines    │
              └────────────────────────────────┘

           Cross-crate tests                            ~ 50 tests
        ┌────────────────────────────────────┐
        │ coordinator + storage + auth        │
        │ shipment + replay engine            │
        └────────────────────────────────────┘

      Unit tests per crate                              ~ 200 tests
   ┌────────────────────────────────────────┐
   │ SDK serde round-trip                    │
   │ Coordinator submit() paths              │
   │ ChainShipmentClient retry semantics     │
   │ MultiRegionReplayEngine comparison      │
   │ JwtValidator edge cases                 │
   │ ResidencyPolicy.validate                │
   └────────────────────────────────────────┘
```

### 12.2 Property-based tests (proptest)

* `proptest_chain_shipment_serialisation_roundtrip` — every chain shape
  serializable + deserializable.
* `proptest_residency_policy_decisions_consistent` — same (policy,
  region) always returns same outcome.
* `proptest_match_outcome_monotonic` — Exact ⊃ Tolerated ⊃ Mismatch
  classification stable.

### 12.3 Chaos / fault-injection tests

* `chaos_partial_chain_shipment_no_commit` — server kills mid-stream ;
  staging GC'd ; no replay reads partial.
* `chaos_target_disk_full_during_replay` — partial result, no
  corruption.
* `chaos_concurrent_submit_race` — 100 concurrent same-swarm submits ;
  exactly 1 succeeds, 99 get 409.

### 12.4 Compliance tests

* `compliance_eu_only_tenant_us_replay_rejected` — F5.
* `compliance_redaction_strips_pii_before_transfer` — G5.
* `compliance_audit_chain_records_jwt_fingerprint` — G14 + ADR-10.

### 12.5 Performance benchmarks (criterion)

* `bench_chain_shipment_1mb_payload` — ≤ 500 ms p99 EU → US.
* `bench_replay_100_op_swarm` — ≤ 30 s p99.
* `bench_admission_overhead` — adds ≤ 1 ms per submit.

---

## 13. N11 / N12 / N13 / N14 / N15 alignment

### Backward compatibility

| Subsystem | Concern | Resolution |
|---|---|---|
| **N11 Firecracker pool** | Replays in Mode B/C still need microVM acquisition | `MultiRegionReplayEngine` shares the existing `FirecrackerPool` ; no separate pool ; N15 `firecracker_drr` already capped. |
| **N12 cache key hardening** | Replay must not poison the idempotency cache | `IdempotencyCache::new_read_only_view()` (NEW) for replay context ; reads OK, writes NO-OP. |
| **N13 shadow/canary catalog** | Replay must not interact with rollout policy | `Dispatcher::shadow_scheduler = None` during replay (I8). |
| **N14 swarm envelope trace** | Replay reads the envelope ; replay also CREATES new envelope rows in `replay.replay_audit_chain` | Envelope schema embedded in replay chain ; reads via target region's local envelope table. |
| **N15 fairness scheduling** | Replay traffic gated by dedicated lane | `AdmissionGate` extended with `replay_op_drr` ; new `BackpressureKind::ReplayRateLimited` ; cap = 10 % of op_drr capacity. |

### Audit chain integration

The Sprint 6 HMAC chain is unchanged for production traces.  N16 adds a
SECOND chain (`replay.replay_audit_chain`) with three new trace types :

```rust
pub enum ReplayTraceKind {
    /// Emitted when a replay job is dispatched (start of work).
    ReplayDispatched {
        job_id: ReplayJobId,
        origin_swarm_request_id: Uuid,
        origin_envelope_trace_id: TraceId,
        actor: String,
        ticket_id: String,
    },
    /// Emitted per-op comparison result.
    ReplayCompared {
        job_id: ReplayJobId,
        op_id: String,
        mode: ExecutionMode,
        match_outcome: MatchOutcome,
        score: Option<f64>,
    },
    /// Emitted on job completion.
    ReplayCompleted {
        job_id: ReplayJobId,
        aggregate: ReplayAggregate,
    },
}
```

Both chains are independent.  GDPR tombstones cascade : tombstoning a
production envelope ALSO tombstones the corresponding `replay.replay_*`
rows (M24 saga extension).

---

## 14. Sprint 9 ship vs Sprint 10/11 split

### What ships in Sprint 9 (this design)

* All 6 capabilities from § 0 TL;DR.
* 15 acceptance tests from § 12.5.
* Production wiring in 4 regions (us-east-1, us-west-2, eu-west-1, ap-southeast-1).
* CLI tool : `efe-replay submit/status/result` (operator UX).

### Deferred to Sprint 10 N17 — active-active multi-region writes

* Cross-region quorum consensus for catalog writes.
* Per-region active-active dispatcher (today : active-passive).
* Conflict-free replicated audit chain (CRDT-style).

### Deferred to Sprint 11 N18 — DR automation

* Auto-failover when source region health drops below threshold.
* Operator-less DR drills (scheduled monthly).
* Multi-region replay-based health probes.

### Deferred to Sprint 12+ — replay-driven canary

* Use N16 replay infrastructure to A/B test canary changes against
  production traffic.
* Statistical drift detection across versions.

---

## 15. Annexes

### Annex A1 — Region taxonomy + supported region pairs

| Source region | Target region | Latency p99 | Cross-border? |
|---|---|---|---|
| us-east-1 | us-east-1 | 5 ms | No |
| us-east-1 | us-west-2 | 70 ms | No |
| us-east-1 | eu-west-1 | 90 ms | YES (US → EU forbidden unless RedactOnTransfer) |
| us-east-1 | ap-southeast-1 | 220 ms | YES |
| eu-west-1 | eu-west-1 | 5 ms | No |
| eu-west-1 | us-east-1 | 90 ms | YES (EU → US forbidden by default) |
| eu-west-1 | ap-southeast-1 | 180 ms | YES |
| ap-southeast-1 | ap-southeast-1 | 5 ms | No |
| ap-southeast-1 | us-east-1 | 200 ms | YES |
| ap-southeast-1 | eu-west-1 | 180 ms | YES |

### Annex A2 — Replay JWT spec

```jsonc
{
  "iss": "efe-operator-idp",
  "sub": "alice@efe.com",
  "operator_id": "550e8400-e29b-41d4-a716-446655440000",
  "tenant_id": "tenant-uuid-here",
  "scope": ["replay:execute", "replay:read"],
  "ticket_id": "JIRA-12345",
  "iat": 1716200000,
  "exp": 1716203600,
  "jti": "unique-jwt-id"
}
```

* Signed with HS256 (Sprint 9) ; RS256 with Vault Transit (Sprint 10).
* `tenant_id` must match `ReplayRequest.tenant_id`.
* `scope` must contain `replay:execute` for submit ; `replay:read` for
  status/result/list.

### Annex A3 — Chain shipment payload schema (full)

(See § 7.3 `chain_shipment.proto` — copied here for completeness.)

```proto
// chain_shipment.proto — Sprint 9 N16
// All bytes are big-endian where applicable.
// All UUIDs use the standard 16-byte big-endian encoding.

syntax = "proto3";
package efe.chain_shipment.v1;

service ChainShipment {
    rpc StageChain(StageChainRequest) returns (stream StageChainResponse);
    rpc VerifyStaging(VerifyRequest) returns (VerifyResponse);
    rpc CommitStaging(CommitRequest) returns (CommitResponse);
    rpc AbortStaging(AbortRequest) returns (AbortResponse);
}

message StageChainRequest {
    string source_region = 1;
    string target_region = 2;
    bytes  tenant_id = 3;
    bytes  swarm_request_id = 4;
    bytes  envelope_trace_id = 5;
    uint32 sdk_schema_version = 6;
    bytes  signing_root_cert = 7;
}

message StageChainResponse {
    oneof payload {
        ChainHeader header = 1;
        TraceRow row = 2;
        BodyBlob body = 3;
        ShipmentFooter footer = 4;
    }
}

message ChainHeader {
    bytes head_hash = 1;
    uint64 row_count = 2;
    bytes derived_hmac_key = 3;
    bytes derived_hmac_key_sig = 4;
}

message TraceRow {
    bytes trace_id = 1;
    bytes canonical_row_body = 2;
    bytes hmac = 3;
    repeated string kms_key_arns = 4;
}

message BodyBlob {
    bytes trace_id = 1;
    string field = 2;
    bytes ciphertext = 3;
    string kms_key_arn = 4;
}

message ShipmentFooter {
    bytes total_signature = 1;
}

message VerifyRequest { bytes staging_id = 1; }
message VerifyResponse { bool verified = 1; repeated string errors = 2; }

message CommitRequest { bytes staging_id = 1; }
message CommitResponse { bool committed = 1; }

message AbortRequest { bytes staging_id = 1; }
message AbortResponse { bool aborted = 1; }
```

### Annex A4 — Multi-region deployment topology

```text
                        ┌──────────────────────┐
                        │  AWS Route 53 latency │
                        │  routing for HTTP API │
                        └──────┬───────────────┘
                               │
        ┌──────────┬───────────┴─────────┬─────────────┐
        ▼          ▼                     ▼             ▼
    ┌────────┐ ┌────────┐           ┌────────┐    ┌────────┐
    │us-east1│ │us-west2│           │eu-west1│    │ap-se-1 │
    └───┬────┘ └───┬────┘           └───┬────┘    └───┬────┘
        │          │                    │             │
   ┌────┴─────┐    │                    │             │
   │ Coord +  │    │ ─── gRPC mTLS ────►│             │
   │ Dispatch │    │ chain shipment     │             │
   │ + Catalog│    │                    │             │
   │ + Postgres   │                    │             │
   │ + Vault  │    │                    │             │
   └──────────┘    │                    │             │
                   │                    │             │
        ┌──────────┴─────────┐   ┌──────┴───────┐ ┌──┴────────┐
        │ Coord + Dispatch   │   │ Coord + ...  │ │ Coord+... │
        └────────────────────┘   └──────────────┘ └───────────┘

AWS KMS multi-region keys :
- Primary in us-east-1
- Replicas in us-west-2, eu-west-1, ap-southeast-1
- Per-tenant keys ; cross-region access granted via tenant onboarding flow

HashiCorp Vault :
- Replicated cluster across regions (Vault Enterprise Performance Replication)
- transit/sign/tenant-<id>-signing-root per tenant
```

### Annex A5 — Operator runbook (replay execution)

#### A5.1 How to trigger a single-swarm replay

```bash
# Set the operator JWT
export EFE_JWT="$(efe-auth issue --scope replay:execute --ticket JIRA-12345)"

# Submit
efe-replay submit \
    --tenant <tenant-uuid> \
    --swarm <swarm-request-id> \
    --origin eu-west-1 \
    --target eu-west-1 \
    --actor alice@efe.com \
    --ticket JIRA-12345 \
    --reason "GDPR audit Q1 2026" \
    --mode dispatch-only

# Returns:
# {"job_id": "uuid", "status": "pending"}

# Poll
efe-replay status uuid

# Fetch result
efe-replay result uuid > result.json
```

#### A5.2 Interpreting a `ReplayResult`

| `overall_outcome` | Meaning | Operator action |
|---|---|---|
| `Clean` | All ops matched within tolerance | None ; archive result |
| `Regression` | One+ op mismatched | Investigate via `replay_diffs` ; consider blocking rollout |
| `Failed` | Engine errored | Read `error` field ; F1-F16 mapping |

#### A5.3 When residency blocks a replay

If you receive `ResidencyViolation`, options :
1. Pick a target region in the policy's allowed list.
2. Request a `RedactOnTransfer` policy update (requires Compliance approval).
3. Use `mode=DispatchOnly` if you only need to validate dispatch logic
   without re-running LLM (may still violate policy if bodies are
   shipped).

#### A5.4 Recovering a stuck replay

```bash
# List in-progress replays older than 30 minutes
efe-replay list --status in_progress --older-than 30m

# Cancel a stuck replay
efe-replay cancel <job_id>

# The N15 admission ticket is released on cancel ; another replay can proceed.
```

#### A5.5 Concurrency conflict

```bash
$ efe-replay submit ...
Error: 409 Conflict
existing_job_id: 550e8400-e29b-41d4-a716-446655440000

$ efe-replay status 550e8400-e29b-41d4-a716-446655440000
{"status": "in_progress", "progress_pct": 42}

# Wait for the other replay or cancel it if mistaken.
```

#### A5.6 Cross-region KMS access denied

```
Error: KmsKeyNotAvailable
arns: ["arn:aws:kms:eu-west-1:.../tenant-X-prod"]
target_region: us-east-1
```

Action : open ticket with EFE Platform Engineering to grant
cross-region KMS access for tenant X.  Lead time : 1 business day.

---

**End of N16 design.**

**Author** : EFE M50 architecture team.
**Date** : 2026-05-20.
**Sprint** : 9 (Phase 3, day 26 – 30).
**Design lineage** : N11 + N12 + N13 + N14 + N15 → N16 (this doc) → N17 active-active → N18 DR automation.

**Implementation entry point** : `cargo new --lib efe-replay-coordinator efe-chain-shipment efe-replay-auth` ; day-1 starts in `crates/efe-nano-sdk/src/types/region.rs`.

**Last-mile checklist** (Sprint 9 exit) :
- [ ] All 15 acceptance tests GREEN.
- [ ] Clippy strict (`-D warnings`) on three new crates.
- [ ] Cross-region E2E test : EU swarm replayed in EU (forbidden in US enforced).
- [ ] Operator runbook A5 reviewed by SRE on-call lead.
- [ ] Compliance team sign-off on residency policy enforcement.
- [ ] M31 billing integration : operator-attributed replay costs visible in monthly bill.
- [ ] Annex A4 topology deployed via Terraform in all 4 regions.
- [ ] PagerDuty integration : F1 / F3 / F9 paging routes verified.
