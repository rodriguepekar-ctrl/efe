# EFE M50 — N20 Postgres + Real Embeddings Architecture

> **Sprint** : 12 (Phase 5, day 46 – 50 of M50).
> **Status** : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md` (Sprint 9, shipped)
> * `EFE_M50_N17_LLM_PROVENANCE_ARCHITECTURE.md` (Sprint 9, shipped)
> * `EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md` (Sprint 10, shipped)
> * `EFE_M50_N19_GOLD_TEST_GATE_TIER_1_ARCHITECTURE.md` (Sprint 11, shipped)
>
> **Anchors** : N19 (Sprint 11) shipped the gold-test gate with
> *in-memory backings*, *stub embedding provider*, *manual drift
> sweep* and *shape-only bypass certificate validation*.  **N20
> closes the production gap** : durable Postgres persistence with
> RLS, three real embedding providers with circuit breaker, cron-
> driven 30-day drift sweep, full Ed25519 cryptographic verification
> of bypass signatures, real `FirecrackerGoldRunner` with eBPF
> hermeticity probe, and the N18 `AgentPromotedToTier1` ledger
> variant that closes the SQL-backdoor loophole.

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2. [Fifteen incident patterns N20 closes (P1–P15)](#2-fifteen-incident-patterns-n20-closes-p1p15)
3. [Commitments — what ships in Sprint 12 (G1–G15)](#3-commitments--what-ships-in-sprint-12-g1g15)
4. [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5. [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6. [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7. [Component architecture](#7-component-architecture)
8. [Module hierarchy & data model](#8-module-hierarchy--data-model)
9. [API surface (HTTP + internal Rust traits)](#9-api-surface-http--internal-rust-traits)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [Cross-cutting alignment (N11 → N19)](#13-cross-cutting-alignment-n11--n19)
14. [Sprint 12 ship vs Sprint 13/14 split](#14-sprint-12-ship-vs-sprint-1314-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The problem.**  N19 shipped a working gold-test gate but every
component that touches durability, cryptography, or scheduled
execution is a stub :

- **Persistence** : `InMemoryGoldSetRegistry` + `InMemoryGoldRunLedger`
  lose all state on process restart.  Operator-approved gold sets
  *vanish* on every redeploy ; the entire HMAC ledger chain
  evaporates.  Real production ran for 6 days post-N19 and lost 18
  approved gold sets across 4 redeploys (Incident I-2026-07-10).
- **Embedding** : `StubEmbeddingProvider` returns hashed vectors
  that *look* like embeddings but encode no semantic content.
  `cosine(actual, expected)` returns the hash collision rate, not
  the semantic similarity.  A 0.92 cosine threshold passes 100 % of
  byte-identical outputs and 0 % of paraphrases — useless for the
  Mode C drift-detection job N19 was designed to do.
- **Cron drift sweep** : N19 wired the `GoldApi::drift_sweep`
  endpoint but the actual scheduled job is absent.  Three Tier 1
  agents drifted for 47 days without anyone noticing (Incident
  I-2026-07-18).
- **Bypass signature** : `BypassCertificate::validate_shape` only
  checks the number of approvers + ticket prefix + rationale length.
  The `OperatorSignature.signature` field (64 bytes Ed25519) is
  **never cryptographically verified** ; an operator with the
  approval keys can issue bypasses with empty signatures and N19
  accepts them.
- **Hermeticity** : the only `GoldRunner` impl is
  `InMemoryGoldRunner` which trusts the simulator closure.  Real
  Firecracker microVM runner + eBPF tap-interface watcher are
  declared in the ADR-7 of N19 but not implemented.
- **N18 ledger** : N19's `GoldRunCompleted` row lives in the N19
  ledger, but the N18 publish ledger has no
  `AgentPromotedToTier1 { gold_run_id, bypass }` entry — promoting
  to Tier 1 still requires a SQL UPDATE on `catalog.entries.stability_
  tier`, which the operator can do without involving N19 at all.

**The fix.**  N20 ships seven production capabilities :

1. **`PostgresGoldSetRegistry`** + **`PostgresGoldRunLedger`** with
   row-level security (RLS) per tenant_id, `UNIQUE(gold_run_id)`
   index, and `sqlx-cli` migrations.  100 % wire-compatible with
   the N19 traits.
2. **Three real `EmbeddingProvider` implementations** :
   - `OpenAiEmbeddingProvider` (text-embedding-3-small, 1536-dim)
   - `CohereEmbeddingProvider` (embed-multilingual-v3.0, 1024-dim)
   - `LocalSentenceTransformersProvider` (all-MiniLM-L6-v2 via
     `candle`, 384-dim, GPU-aware)
3. **`MultiProviderEmbeddingClient`** with circuit-breaker +
   exponential backoff + multi-provider failover (priority list).
4. **`DriftSweepCron`** — `tokio-cron-scheduler`-driven 24h job
   that re-runs every Tier 1 agent's latest approved gold set ;
   on fail emits `GoldDriftDetected` ledger row + demotes via
   N18 ledger.
5. **`BypassSignatureVerifier`** — reuses N18 `SignatureVerifier`
   to Ed25519-verify each `OperatorSignature.signature` against a
   canonical message `(gold_run_id || bypass_id || ticket_id ||
   rationale || valid_until)`.
6. **`FirecrackerGoldRunner`** — real `GoldRunner` impl backed by
   the N11 microVM pool, with eBPF tap-interface watcher that fires
   `HermeticityBroken` on any egress packet during a case
   execution.
7. **`PublishLedgerEntry::AgentPromotedToTier1`** variant on the
   N18 ledger.  `promote_to(Tier1)` in the catalog runtime now
   demands a `gold_run_id` (or a bypass certificate) and appends
   this row before flipping the tier.  Closes the SQL-backdoor :
   compliance can now run `SELECT * FROM publish_ledger WHERE
   entry_kind = 'agent_promoted_to_tier_1'` and reconstruct the
   complete audit trail.

**Backward compatibility** :
- All N19 wire types are preserved byte-for-byte.  N20 is a *binary
  drop-in upgrade* for any N19 consumer that pins the
  `efe-gold-test-gate` crate.
- `InMemoryGoldSetRegistry` and `InMemoryGoldRunLedger` continue to
  ship for tests / dev / CI — they're never removed.
- `StubEmbeddingProvider` ships as the default behind a
  `default-stub-embeddings` Cargo feature ; production deployments
  set `features = ["postgres", "real-embeddings"]`.
- The new `BypassCertificate.bypass_id: Uuid` field carries
  `#[serde(default)]` so pre-N20 producers can omit it.

**Out of scope (deferred to Sprint 13 N21 / Sprint 14 N22)** :
- Shadow-capture-driven gold case discovery (Sprint 13).
- LLM-as-judge experimental strategy (Sprint 14).
- Postgres-backed `efe-publisher-identity` registry (Sprint 13 — it
  shares the same migration pipeline).
- Cross-region gold-set replication via N16 — Sprint 12 ships
  per-region Postgres ; cross-region sync via N16 envelope shipment
  is N21 scope.

---

## 1. Context, motivation, sprint placement

### 1.1 Five real incidents driving N20

**Incident I-2026-07-10 — In-memory registry vaporised 18 gold sets.**
The Sprint 11 production deployment of N19 ran for 6 days.  Over
that window, 4 routine redeploys happened (config changes, dep
bumps, ops-driven restarts).  Each restart wiped the in-memory
`GoldSetRegistry`.  By the time the operator team noticed, 18
operator-approved gold sets across 12 agents were *gone* — including
the freshly-approved-and-celebrated `summarize_strict v3.0.0` flag-
ship set.  The 36 operator signatures that backed those approvals
became unreachable.  Compliance flagged it as a P1 audit gap.
Detection : 5 days.  Recovery : 14 hours of operator re-approval
ceremonies.

**Incident I-2026-07-18 — 47-day silent drift on 3 Tier 1 agents.**
N19's drift sweep is an `axum`-callable endpoint, never wired to a
cron.  Three Tier 1 LLM-backed agents (Mode C) ran without a re-
verified gold run for 47 days, during which their LLM provider
silently bumped the model snapshot (Claude haiku → haiku-20240307).
Two of the agents had cosine drift of ≥ 0.18 against their original
gold runs.  Customers noticed first.  Detection : reactive (customer
complaint).  Recovery : 6 hours rollback + re-approve.

**Incident I-2026-07-22 — Stub embeddings let a regression through.**
A Mode C agent was promoted with `SemanticSimilarity { cosine_thres
hold: 0.85 }`.  The stub provider returned cosine = 1.0 because the
agent's output hashed identically to the expected JSON.  Real
embedding would have shown 0.71 (paraphrased summary).  The bug
surfaced only when N20-stub-disabled fixture caught it pre-merge.

**Incident I-2026-07-25 — Empty-signature bypass accepted.**
A junior operator filed a Sev-1 bypass with `OperatorSignature.
signature = vec![]`.  N19's `validate_shape` accepted it (it only
counts approvers + checks ticket prefix + rationale length).  The
audit log shows the bypass was used to promote a buggy hot-fix that
broke 4 % of customer requests for 90 min.  Post-mortem identified
the *missing crypto verification* as the root cause.

**Incident I-2026-07-28 — SQL-backdoor Tier 1 promote.**
A new ops engineer promoted a community-published agent to Tier 1
by `UPDATE catalog.entries SET stability_tier = 'tier1' WHERE id =
'unsafe_redact_v2'`, bypassing the entire N19 gate.  N19's ledger
shows zero rows for this agent ; the N18 publish ledger has no
variant that would have captured the promote event with the missing
`gold_run_id`.  Compliance N+1 audit caught it on day 12.

### 1.2 Why Sprint 12, immediately after N19

N19 (Sprint 11) is the *first sprint to introduce mandatory
cryptographically-verified gates* on the Tier 1 promote.  Without
N20, the gate is a paper tiger : easy to bypass via restart, easy to
ignore via cron-not-wired, easy to forge via empty-sig bypass, easy
to side-step via SQL UPDATE.  Sprint 12 must close all four
backdoors **before** Sprint 13 starts adding shadow-capture features
that assume the gate works.

Sprint 12 ordering :

| Day | Deliverable | Depends on |
|---|---|---|
| 46 | Postgres schema + migrations + `PostgresGoldSetRegistry` | N18 schema patterns |
| 47 | `PostgresGoldRunLedger` + `UNIQUE(gold_run_id)` + chain verifier | N19 ledger trait |
| 48 | 3 real `EmbeddingProvider` impls + `MultiProviderEmbeddingClient` | N12 cache, N17 ModelIdentity |
| 49 | `DriftSweepCron` + `BypassSignatureVerifier` + N18 ledger variant | N18 SignatureVerifier, N19 GoldGate |
| 50 | `FirecrackerGoldRunner` + eBPF probe + integration tests + acceptance suite | N11 pool |

### 1.3 Who consumes N20 once it ships

| Consumer | Use case |
|---|---|
| **Ops / SRE** | Postgres backups + PITR replace operator re-approval ceremonies post-restart |
| **Compliance** | `SELECT * FROM publish_ledger WHERE entry_kind = 'agent_promoted_to_tier_1'` reconstructs full audit |
| **Security team** | Bypass signatures now cryptographically verified ; junior operators can't issue empty-sig bypasses |
| **Publishers** | Multi-provider embedding failover means their gold cases work even when OpenAI is rate-limited |
| **Data residency** | Postgres RLS enforces per-tenant isolation that in-memory could not (no inter-tenant DashMap leaks) |
| **N16 multi-region** | Per-region Postgres makes the gold-set shipment story trivial (Sprint 13 wires the actual cross-region replication) |
| **N17 LLM provenance** | `LlmProvenanceRecord.gold_case_id` auto-populated by the runner for every Mode C gold case call |
| **N18 publisher** | `AgentPromotedToTier1` variant + the deprecation of the SQL-direct path |

### 1.4 The 15 incident-class patterns (preview)

Section 2 enumerates 15 patterns clustered into 5 families :

* **Persistence & durability** (P1, P2, P3, P4) — 4 patterns
* **Embedding realism** (P5, P6, P7, P8) — 4 patterns
* **Scheduling & cron** (P9, P10, P11) — 3 patterns
* **Cryptographic verification** (P12, P13) — 2 patterns
* **Hermeticity probe** (P14, P15) — 2 patterns

---

## 2. Fifteen incident patterns N20 closes (P1–P15)

> **Methodology** : every P-row sources from (a) a real incident with
> ticket reference, (b) a Sprint 11 retro near-miss, or (c) a
> compliance / security review finding.  No speculative rows.

### Family A — Persistence & durability

**P1. In-memory registry vaporises operator approvals on restart.**
6-day production run lost 18 approved gold sets across 4 redeploys
(Incident I-2026-07-10).  N20's `PostgresGoldSetRegistry` puts
every approval into `gold.gold_set_approver` rows protected by
RLS — restart-safe, backup-safe, PITR-safe.

**P2. In-memory ledger HMAC chain doesn't survive restart.**
Same incident family : `GoldRunCompleted` rows existed for 4 hours
then evaporated.  N20's `PostgresGoldRunLedger` persists the
canonical body + HMAC per row with an `INSERT … RETURNING` flow ;
the chain verifier walks Postgres rows by `sequence_number`.

**P3. Cross-region replication impossible without persistence.**
N16 ships chains via `InMemoryChainShipmentClient` ; without
persistent gold-set storage, there's nothing to ship.  N20's
Postgres-backed gold tables become the source of truth for any
future cross-region N16 sync (Sprint 13 scope).

**P4. Backup / PITR unavailable for in-memory state.**
Compliance demands 7-year retention on operator approval signatures.
In-memory dies with the process.  Postgres + WAL archiving + PITR
gives the standard 7-year story for free.

### Family B — Embedding realism

**P5. Stub embeddings encode no semantic content.**
`cosine(hash(text_a), hash(text_b)) ≈ 0` for any distinct texts,
regardless of meaning.  A paraphrase fails ; a typo fails.  N20's
real providers actually measure semantic similarity (Incident
I-2026-07-22).

**P6. Provider drift detection requires version pinning + audit.**
`ModelIdentity.served_version != requested_version` must be
detected on every call.  The stub doesn't differentiate.  N20 wires
the real providers' response headers (OpenAI `x-served-model`,
Cohere `x-cohere-model-version`) into the verdict.

**P7. Cost overrun on long Mode C cases.**
A 50-case Mode C gold run with 4096-token outputs × $0.13 per 1M
tokens = $0.027 per call × 50 = $1.35 per run.  Multiplied by
200 agents × 30-day sweep = $8 100/month.  N20's N12 cache hit-rate
on real embeddings is ~85 % after the first sweep, dropping
marginal monthly cost to $1 200.

**P8. Single-provider outage stalls the entire drift sweep.**
OpenAI outage Sprint 11 Sept blocked all gold runs for 4 h.  N20's
`MultiProviderEmbeddingClient` priority list `[OpenAI, Cohere,
local]` fails over within 3 retries (1 s, 5 s, 15 s) ; the local
fallback never goes down.

### Family C — Scheduling & cron

**P9. 30-day drift sweep never fires without a cron driver.**
Incident I-2026-07-18 : 47-day silent drift.  N20's
`DriftSweepCron` runs every 24h via `tokio-cron-scheduler` ; the
runtime tracks last-run-at per agent in `gold.gold_set_drift_state`
so reschedules survive restarts.

**P10. Concurrent drift sweeps burst the embedding API quota.**
Naive cron : 200 agents × 50 cases / sweep concurrent = 10 000
embedding calls / minute → instant rate limit.  N20's sweep uses
N15's `OperationClass::GoldRun` quota (≤ 10 cases parallel
per region) and a moving-average rate limiter ≤ 60 cases/min.

**P11. Stale gold sets > 180 d undetected.**
Tier 1 agent unchanged for 6 months ; the gold set is still v1.
N20 emits `n20_stale_gold_set_total{agent_id}` daily ; > 180 d
triggers a warning email to the publisher.

### Family D — Cryptographic verification

**P12. Bypass signatures structurally accepted with empty bytes.**
Incident I-2026-07-25.  N20's `BypassSignatureVerifier` reuses N18
`Ed25519Verifier` to cryptographically verify each
`OperatorSignature.signature` against a canonical message ; empty
or invalid sigs fail-closed with `BypassSignatureInvalid`.

**P13. SQL-backdoor Tier 1 promote leaves no audit row.**
Incident I-2026-07-28.  N20's `PublishLedgerEntry::AgentPromotedTo
Tier1 { gold_run_id, bypass }` variant on N18 makes the promote
event a first-class ledger row.  The `catalog.entries.stability_
tier` flip is gated on this row's presence ; direct SQL UPDATEs
trigger an audit alarm (Postgres TRIGGER that compares to the
ledger and fails on mismatch).

### Family E — Hermeticity probe

**P14. `InMemoryGoldRunner` trusts the simulator — no isolation.**
The closure that simulates an agent can do anything.  Production
needs a real microVM.  N20's `FirecrackerGoldRunner` reuses the
N11 pool with network namespace disabled, frozen clock, and
restricted FS.

**P15. eBPF tap-interface watcher missing — network leaks invisible.**
N19 has a `HermeticityViolation` enum variant but nothing fires it.
N20's `EbpfHermeticityProbe` attaches a `tc` filter to the
microVM's tap interface ; any egress packet during a case execution
fires `HermeticityBroken` and aborts the run.

---

## 3. Commitments — what ships in Sprint 12 (G1–G15)

> **Read convention** : `G-N : commitment / impl surface / acceptance test`.

### Persistence

**G1. PostgresGoldSetRegistry with RLS.**
`gold.gold_set`, `gold.gold_case`, `gold.gold_set_approver` tables.
RLS policy `gold_set_per_tenant` keyed off `current_setting('
efe.tenant_id')`.  Acceptance :
`phase_5_n20_postgres_registry_round_trip`.

**G2. PostgresGoldRunLedger with `UNIQUE(gold_run_id)` index.**
`gold.gold_run_ledger` + `gold.bypass_certificate` tables ;
sequence-number + HMAC preserved from N19 in-memory pattern.
Acceptance : `phase_5_n20_postgres_ledger_chain_verifies`.

**G3. Backup-and-PITR-friendly schema.**
All `*_id` columns are UUIDs ; all timestamps `TIMESTAMPTZ` ; all
JSON in `JSONB` for queryability ; HMAC keys derived per-row from
`pg_hkdf` (Sprint 13 wires KMS root key).  Acceptance :
`phase_5_n20_pg_dump_round_trip`.

### Embedding realism

**G4. OpenAiEmbeddingProvider.**
Backed by `reqwest` ; uses `text-embedding-3-small` by default ;
returns the served model version from response headers.
Acceptance : `phase_5_n20_openai_provider_smoke` (gated behind
`OPENAI_API_KEY` env ; uses `wiremock` in CI).

**G5. CohereEmbeddingProvider.**
Backed by `reqwest` ; uses `embed-multilingual-v3.0` ; returns
served version.  Acceptance :
`phase_5_n20_cohere_provider_smoke`.

**G6. LocalSentenceTransformersProvider.**
Backed by `candle-core` + `candle-transformers` ; loads the
`all-MiniLM-L6-v2` ONNX model ; GPU-aware (CUDA if available, else
CPU).  Acceptance :
`phase_5_n20_local_provider_returns_consistent_vectors`.

**G7. MultiProviderEmbeddingClient with circuit breaker.**
Priority list `[primary, secondary, fallback]` ; opens the breaker
on 3 consecutive `RateLimited` or `Backend` errors ; half-open
after 30 s ; closes after 1 success.  Acceptance :
`phase_5_n20_multi_provider_failover_under_rate_limit`.

### Cron drift sweep

**G8. DriftSweepCron — 24h scheduled job.**
`tokio-cron-scheduler` ; tracks `last_run_at` per
`(agent_id, version)` in `gold.gold_set_drift_state` ; respects
N15 `OperationClass::GoldRun` quota.  Acceptance :
`phase_5_n20_drift_sweep_cron_fires_and_demotes`.

**G9. Drift sweep idempotent across restart.**
If the process restarts mid-sweep, the next cron tick re-reads
`gold.gold_set_drift_state` and skips agents whose `last_run_at` is
< 23h old.  Acceptance :
`phase_5_n20_drift_sweep_idempotent_after_restart`.

### Cryptographic verification

**G10. BypassSignatureVerifier via Ed25519.**
Each `OperatorSignature.signature` verified against canonical
message `(gold_run_id || bypass_id || ticket_id || rationale ||
valid_until_unix)`.  Acceptance :
`phase_5_n20_bypass_empty_signature_rejected`.

**G11. N18 `AgentPromotedToTier1` variant added.**
New ledger entry kind ; carries `(agent_id, version, gold_run_id,
bypass: Option<BypassCertificate>)`.  Catalog runtime's `promote_
to(Tier1)` appends this row before flipping `stability_tier`.
Acceptance :
`phase_5_n20_promote_emits_n18_ledger_entry`.

**G12. Postgres TRIGGER detects SQL-backdoor promotes.**
`BEFORE UPDATE OF stability_tier ON catalog.entries` checks that
the new value's `gold_run_id` exists in `publish_ledger` ;
otherwise raises `EXCEPTION 'unaudited tier 1 promote'`.
Acceptance :
`phase_5_n20_sql_backdoor_promote_blocked`.

### Hermeticity

**G13. FirecrackerGoldRunner uses N11 pool.**
`new(pool: Arc<dyn VmPool>, probe: Arc<EbpfHermeticityProbe>)`.
Acceptance :
`phase_5_n20_firecracker_runner_executes_case`.

**G14. EbpfHermeticityProbe fires on egress.**
Attaches `tc` filter on tap interface ; any packet → signals
`HermeticityBroken` to the gate.  Acceptance :
`phase_5_n20_ebpf_probe_fires_on_curl_outbound`.

### Operability

**G15. Prometheus surfaces extended.**
N19's 12 surfaces + N20's 8 new : `n20_postgres_pool_inflight`,
`n20_embedding_provider_calls_total`, `n20_embedding_provider_
errors_total`, `n20_circuit_breaker_state`, `n20_drift_sweep_runs_
total`, `n20_bypass_signature_verifications_total`, `n20_ebpf_
hermeticity_violations_total`, `n20_n18_promotions_total`.
Acceptance :
`phase_5_n20_metrics_emit_all_20_surfaces`.

---

## 4. Invariants — non-negotiables (I1–I20)

### Persistence

**I1. All gold writes are ACID-committed.**  `BEGIN ; INSERT ;
COMMIT` is mandatory ; no dirty reads.

**I2. RLS per tenant_id enforced at the Postgres role level.**  No
in-process row filtering — the database refuses to return
cross-tenant rows even on bug.

**I3. The HMAC chain link survives restart.**  The next row's
`prev_hmac` is `(SELECT hmac FROM gold_run_ledger WHERE agent_id =
$1 ORDER BY sequence_number DESC LIMIT 1)`.

**I4. `gold_run_id` is globally UNIQUE.**  Postgres `UNIQUE`
constraint, not application-level dedup.

### Embedding

**I5. Embedding model version pinned per gold case.**  The
`SemanticSimilarity.embedding_model.requested_version` is hardcoded
at gold-set authoring time ; the provider's served version is
recorded but **not used for comparison** — pinning is the floor.

**I6. Provider migration detected via `served_version` mismatch.**
If `OpenAI` returns `served_version != requested_version`, the
case errors with `EmbeddingProviderError::ModelDrift`.

**I7. Multi-provider failover keeps the ordered priority list.**
The fallback's verdict is *advisory* — gold set authors who pin
`OpenAI` get an `Errored` case if all providers fail, not a "lucky
fallback".

**I8. Embedding cache is per-(model, text-blake3).**  No cross-
model collisions ; N12 cache key includes the model's lineage_key.

### Cron sweep

**I9. Sweep does not block on per-agent failures.**  One agent's
failed gold run never stalls the remaining N-1.

**I10. Sweep emits exactly one ledger row per agent.**  Either
`GoldRunCompleted` (sweep passed) or `GoldDriftDetected` (sweep
failed).

**I11. Sweep respects N15 quota.**  Concurrent gold runs across all
agents ≤ N15's `OperationClass::GoldRun` slot count.

**I12. Sweep skips agents promoted in the last 24h.**  Avoids
double-work : a freshly-promoted agent already ran its gate.

### Crypto

**I13. Bypass signature canonical message is immutable.**  Any
mutation invalidates the signature ; no field-reorder safety net.

**I14. Bypass signature verified before the gate skips cases.**
The verify-fail path returns the error BEFORE writing anything.

**I15. N18 `AgentPromotedToTier1` row appended before the
`stability_tier` UPDATE.**  Two-phase write : ledger first, tier
second, with the trigger checking ledger presence.

### Hermeticity

**I16. eBPF probe attaches before the microVM boots.**  Race-free :
no window where the VM can leak before the probe is live.

**I17. eBPF probe egress count is exposed as a counter.**  Even
"clean" runs show 0 packets ; auditors verify the probe is
actually running.

**I18. Firecracker case timeout is enforced by a VM watchdog.**
Not the host — host watchdog races with case work.

### Operability

**I19. Postgres connection pool size ≥ 8.**  Tuned per region ;
deadlock-free as long as no single transaction holds > 2
connections.

**I20. All Postgres queries are parameterized (`$1`, `$2`…).**
Defense in depth against SQL injection ; sqlx compile-time-checks
queries against the schema.

---

## 5. Failure modes — what we PROTECT against (F1–F15)

> **Read convention** : `F-N : cause / detection / response / recovery`.

### Postgres / persistence

**F1. Postgres primary unavailable.**
* *Cause* : primary instance crashed, network partition, AZ failure.
* *Detection* : `sqlx` connection error or timeout > 5 s.
* *Response* : gate fails-closed with `GoldGateError::Backend` ;
  drift sweep tick skipped.
* *Recovery* : failover to replica (automatic via Postgres) ; sweep
  resumes on next tick.

**F2. Postgres deadlock on concurrent gold-set approval.**
* *Cause* : two operators approve the same `gold_set_id` at the
  same instant.
* *Detection* : Postgres returns `40P01` deadlock detected.
* *Response* : sqlx retries with exponential backoff up to 3 times.
* *Recovery* : automatic.

**F3. Postgres connection pool exhausted.**
* *Cause* : load spike, slow query, deadlock cluster.
* *Detection* : `sqlx::Error::PoolTimedOut` after 30 s.
* *Response* : metric `n20_postgres_pool_inflight` spikes ; gate
  fails-closed for new requests ; existing transactions complete.
* *Recovery* : ops increases pool size ; queries optimized.

### Embedding

**F4. OpenAI rate-limited (429).**
* *Detection* : provider returns 429 with `x-ratelimit-remaining: 0`.
* *Response* : multi-provider client opens the breaker after 3
  consecutive 429s ; failover to Cohere ; if Cohere also down,
  fallback to local.
* *Recovery* : breaker half-opens after 30 s ; closes after 1
  success.

**F5. Cohere quota exceeded.**
* Same handling as F4 but for Cohere.

**F6. Local sentence-transformers GPU OOM.**
* *Cause* : large batch input, GPU memory pressure.
* *Detection* : `candle::Error::Cuda(...)`.
* *Response* : retry on CPU (slower but reliable).
* *Recovery* : ops shrinks batch size in config.

**F7. Embedding provider mid-call model migration.**
* *Cause* : OpenAI promotes a new snapshot mid-sweep.
* *Detection* : `served_version != requested_version`.
* *Response* : case errors `EmbeddingProviderError::ModelDrift` ;
  drift sweep emits `GoldDriftDetected` ; agent demoted.
* *Recovery* : publisher re-approves with the new version pinned.

### Cron

**F8. Cron job stuck (no tick for > 2 × interval).**
* *Detection* : `n20_drift_sweep_last_tick_at` gauge stale.
* *Response* : ops alerted via PagerDuty.
* *Recovery* : manual restart.

**F9. Cron clock skew.**
* *Detection* : `tokio-cron-scheduler` uses monotonic time ; system
  clock jumps don't affect it.
* *Response* : N/A (designed-around).
* *Recovery* : N/A.

### Crypto

**F10. Bypass Ed25519 verification fails.**
* *Cause* : forged signature, revoked key, wrong canonical message.
* *Detection* : `Ed25519Verifier::verify` returns `Err`.
* *Response* : `GoldGateError::BypassSignatureInvalid { approver_
  index }`.
* *Recovery* : operator re-issues correct signature.

**F11. Operator key revoked between bypass issuance and use.**
* *Detection* : N18 `PublisherRegistry::is_key_active(key_id)`
  returns false.
* *Response* : reject the bypass.
* *Recovery* : ops re-issues with active key.

### Hermeticity

**F12. eBPF probe attachment fails.**
* *Cause* : missing CAP_BPF, host kernel too old, libbpf-rs error.
* *Detection* : `EbpfHermeticityProbe::attach` returns `Err`.
* *Response* : `GoldRunnerError::Backend` — the runner refuses to
  execute the case rather than run unprotected.
* *Recovery* : ops fixes host setup.

**F13. eBPF probe false positive (legitimate kernel-internal
syscall).**
* *Cause* : a benign DNS resolver, NTP query.
* *Detection* : `tc` filter sees an unexpected egress packet.
* *Response* : whitelist via destination IP/port range (config) ;
  re-run.
* *Recovery* : ops adjusts whitelist.

**F14. Firecracker microVM startup timeout.**
* *Cause* : N11 pool exhausted, kernel module unloaded.
* *Detection* : `VmPool::acquire` times out (> 30 s).
* *Response* : `GoldRunnerError::Backend` ; case marked `Errored`.
* *Recovery* : ops restarts the N11 pool.

### Concurrency

**F15. Concurrent drift sweep + manual run on same agent.**
* *Cause* : ops triggers a manual gate while cron is mid-sweep.
* *Detection* : the existing `GoldGate::run` per-agent in-progress
  flag (N19 + F15).
* *Response* : second request gets `RunAlreadyInProgress`.
* *Recovery* : automatic — second caller polls or accepts the
  in-flight result.

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1. PostgreSQL 16 + `sqlx` as the persistence layer.

**Context.**  Where N20 stores gold sets + ledger.

**Decision.**  PostgreSQL 16 (latest LTS) + `sqlx` 0.8 with
compile-time query checking against a Postgres dev DB.

**Rationale.**
* **ACID + RLS** — Postgres is the only OSS choice with first-class
  RLS that EFE security team accepts.
* **JSONB** — flexible for `cases JSONB` without schema churn.
* **`sqlx` compile-time checks** — `query!()` validates SQL at
  compile time against the schema ; runtime parse errors are
  impossible.

**Consequences.**
* Sprint 12 ships migrations via `sqlx-cli`.
* Dev fixture : `efectl pg up` spawns a docker-compose Postgres on
  port 5432.

**Rejected alternatives.**
* **CockroachDB** — overkill at Sprint 12 ; revisit at multi-region scope.
* **SQLite** — single-writer, no RLS ; testing only.

---

### ADR-2. RLS per tenant via session-variable role.

**Context.**  How to keep tenant A from reading tenant B's gold
sets.

**Decision.**  On every transaction, set `SET LOCAL efe.tenant_id =
'<uuid>'` from the operator's JWT.  Every gold-table RLS policy is
`USING (tenant_id::text = current_setting('efe.tenant_id'))`.

**Rationale.**
* **Defense in depth** — Postgres refuses to read cross-tenant
  rows even when the application has a bug.
* **No `WHERE tenant_id = $1` repetition** — the policy enforces it
  uniformly.

**Consequences.**
* Every Postgres pool checkout sets the session var first.
* Tests use a fixed `00000000-0000-0000-0000-000000000001` test
  tenant.

**Rejected alternatives.**
* **Separate schemas per tenant** — operationally expensive at
  10 000+ tenants.
* **Application-only filtering** — fails defense-in-depth.

---

### ADR-3. Multi-provider embedding with priority list + circuit
breaker.

**Context.**  Single-provider outages (P8).

**Decision.**  `MultiProviderEmbeddingClient` holds an ordered
`Vec<(name, Arc<dyn EmbeddingProvider>)>`.  Calls always try
provider[0] first ; on `RateLimited` or `Backend`, fall through.
Per-provider circuit breaker (3 fails → open → 30 s half-open → 1
success → close).

**Rationale.**
* **Predictable** — pinned ordering means audit logs are deterministic.
* **Failover** — local provider as last resort never goes down.

**Consequences.**
* `MultiProviderEmbeddingClient` exposes `breaker_state(provider:
  &str) → CircuitBreakerState` for ops dashboards.

**Rejected alternatives.**
* **Round-robin load balancing** — non-deterministic, hard to
  audit, embedding-version drift compounds.

---

### ADR-4. Cron via `tokio-cron-scheduler` (process-internal).

**Context.**  Where the 24h drift sweep lives.

**Decision.**  `tokio-cron-scheduler` runs inside the same process
as the gold-test-gate runtime.  Cron expression `0 0 3 * * *`
(daily at 03:00 UTC).

**Rationale.**
* **No external dep** — no need for K8s CronJob, AWS EventBridge.
* **Shares the process pool** — no extra Postgres connections.

**Consequences.**
* `DriftSweepCron::start(handle: tokio::task::JoinHandle)` returned
  to caller for graceful shutdown.

**Rejected alternatives.**
* **K8s CronJob** — opaque to N19 metrics ; harder to coordinate
  with N15 quota.

---

### ADR-5. Bypass Ed25519 verification via N18 `SignatureVerifier`.

**Context.**  No re-implementation of crypto.

**Decision.**  `BypassSignatureVerifier::new(inner: Arc<dyn
SignatureVerifier>, registry: Arc<dyn PublisherRegistry>)`.  For
each `OperatorSignature`, look up the operator's active key in the
publisher registry, build the canonical message, call
`inner.verify(...)`.

**Rationale.**
* **One crypto code path** — bugs caught by N18 audit pass.
* **Operator key rotation** — automatically picked up via
  `registry.lookup(operator_id)`.

**Consequences.**
* N20 depends on `efe-publisher-identity` directly (already a
  workspace member).

**Rejected alternatives.**
* **Inline Ed25519** — duplicate code, doubled audit surface.

---

### ADR-6. FirecrackerGoldRunner reuses N11 pool.

**Context.**  Where N20 runs gold cases hermetically.

**Decision.**  `FirecrackerGoldRunner::new(pool: Arc<dyn VmPool>,
probe: Arc<EbpfHermeticityProbe>)`.  On `run_case` :
1. `vm = pool.acquire()`
2. `probe.attach(&vm.tap_interface)`
3. `vm.exec(case.input)` with frozen clock
4. `probe.detach()` (records egress count)
5. `pool.release(vm)`

**Rationale.**
* **Reuse** — N11 already gives us network namespace, frozen
  clock, restricted FS.
* **eBPF probe is the missing piece** — N11 didn't have it (N20
  adds it as a separate component for clean separation).

**Consequences.**
* eBPF probe lives in `efe-gold-test-gate/src/runner/ebpf.rs`.

---

### ADR-7. eBPF probe via `libbpf-rs`.

**Context.**  How to detect network egress.

**Decision.**  `libbpf-rs` 0.24 ; attach a `tc` ingress/egress
program to the tap interface ; ring buffer to userland on every
packet ; userland aggregates count per case.

**Rationale.**
* **Mature** — `libbpf-rs` is BPF CO-RE-compatible (no kernel
  recompile).
* **Low overhead** — ~50 ns per packet.

**Consequences.**
* CAP_BPF + CAP_NET_ADMIN required ; the gold-test-gate runtime
  ships with these capabilities.

**Rejected alternatives.**
* **iptables LOG + parsing** — high overhead, ambiguous parsing.
* **Wireshark library** — too heavyweight.

---

### ADR-8. Embedding cache keyed by `(model_identity, text_blake3)`
via N12.

**Context.**  Drive marginal cost to near-zero on re-runs (P7).

**Decision.**  Cache key formula :

```
key = blake3(model.provider || "|" || model.model_id || "|" ||
             model.requested_version || "|" || blake3(text))
```

Stored in N12's existing cache infra ; TTL = 24h.

**Rationale.**
* **Stable across re-runs** — same text + same model = cache hit.
* **N12 already exists** — no new cache infra.
* **Model-pinned** — version bump invalidates automatically.

**Consequences.**
* Drift sweep re-runs have ~85 % cache hit rate.
* Cost metric `n20_embedding_cost_cents_total` drops 85 % after
  warm-up.

---

### ADR-9. Cost stored as integer microcents (no float).

**Context.**  Floating-point drift in cost aggregations leads to
"$0.000000001 short of budget" bugs.

**Decision.**  Cost columns are `BIGINT microcents` (1 USD =
1 000 000 microcents).  N20 converts to f64 only at API surface.

**Rationale.**
* **Exact** — no float reordering noise.
* **Integer arithmetic** — sum of 200 agents × 30 days never
  overflows i64.

**Consequences.**
* Public API still surfaces `cost_usd: f64` for convenience.
* Internal cost computations use `i64`.

---

### ADR-10. Schema migrations via `sqlx-cli` + checked-in `.sql`
files.

**Context.**  How schema evolves Sprint 12 → 13 → … .

**Decision.**  `migrations/` directory under `efe-gold-test-gate/` ;
each migration named `YYYYMMDDhhmmss_description.sql` ; checked-in
to git ; applied by `sqlx migrate run` at boot.

**Rationale.**
* **Versioned** — git history is the schema history.
* **Reversible** — each migration has an `up` and `down`.
* **Audit-friendly** — compliance can review every schema change.

**Consequences.**
* Sprint 12 ships migrations `001_initial_gold_schema.sql`,
  `002_drift_state.sql`, `003_bypass_certificate.sql`.

---

## 7. Component architecture

### 7.1 The 9 components at a glance

```text
                       Operator portal
                            │
                            ▼
        ┌──────────────────────────────────┐
        │ PostgresGoldSetRegistry          │
        │   RLS per tenant_id              │
        │   3 tables : set, case, approver │
        └────────────┬─────────────────────┘
                     │
                     ▼
        ┌──────────────────────────────────┐
        │ GoldGate (N19)                   │
        │  ┌────────────┐ ┌──────────────┐ │
        │  │Firecracker │→│ Assertion    │ │
        │  │GoldRunner  │ │ Engine       │ │
        │  │ (N11 + eBPF) │  + multi-prov │ │
        │  └────────────┘ └──────────────┘ │
        └────────────┬─────────────────────┘
                     │
                     ▼
        ┌──────────────────────────────────┐
        │ PostgresGoldRunLedger            │
        │   HMAC chain ; UNIQUE(gold_run_id)│
        │   bypass_certificate table       │
        └────────────┬─────────────────────┘
                     │
                     ▼
   ┌──────────────────────────────────────────┐
   │ N18 publish ledger ← AgentPromotedToTier1│
   │ N17 provenance     ← gold_case_id        │
   │ N6 audit chain     ← GoldGateRun         │
   └──────────────────────────────────────────┘

   Side-band components :
   - DriftSweepCron (24h)
   - BypassSignatureVerifier
   - MultiProviderEmbeddingClient
   - EbpfHermeticityProbe
```

### 7.2 `PostgresGoldSetRegistry`

```rust
pub struct PostgresGoldSetRegistry {
    pool: PgPool,
    metrics: Arc<N19Metrics>,
}

impl PostgresGoldSetRegistry {
    pub async fn connect(url: &str, max_conns: u32) -> Result<Self, GoldSetRegistryError>;
    fn set_tenant_session(tx: &mut PgConnection, tenant: TenantId) -> Result<(), sqlx::Error>;
}

#[async_trait]
impl GoldSetRegistry for PostgresGoldSetRegistry {
    async fn register(&self, draft: GoldSet, pii_required: bool) -> Result<GoldSetId, _>;
    async fn approve(&self, id: GoldSetId, approvers: Vec<OperatorSignature>) -> Result<(), _>;
    async fn archive(&self, id: GoldSetId, rationale: String) -> Result<(), _>;
    async fn lookup(&self, id: GoldSetId) -> Result<GoldSet, _>;
    async fn latest_approved_for(...) -> Result<Option<GoldSet>, _>;
    async fn list_for_agent(...) -> Result<Vec<GoldSet>, _>;
}
```

### 7.3 `PostgresGoldRunLedger`

```rust
pub struct PostgresGoldRunLedger {
    pool: PgPool,
    hmac_key: SecretKey,  // pulled at boot from Vault Transit
    metrics: Arc<N19Metrics>,
}

#[async_trait]
impl GoldRunLedger for PostgresGoldRunLedger {
    async fn append(&self, entry: GoldRunLedgerEntry) -> Result<GoldRunLedgerRow, _>;
    async fn history(&self, agent_id: &NanoAgentId, limit: usize) -> Result<Vec<_>, _>;
    async fn verify_chain(&self, agent_id: &NanoAgentId) -> Result<ChainVerifyReport, _>;
    async fn find_by_gold_run_id(&self, id: GoldRunId) -> Result<Option<_>, _>;
}
```

### 7.4 Three real `EmbeddingProvider` impls

```rust
pub struct OpenAiEmbeddingProvider {
    client: reqwest::Client,
    api_key: SecretString,
    default_model: String,
}

pub struct CohereEmbeddingProvider {
    client: reqwest::Client,
    api_key: SecretString,
}

pub struct LocalSentenceTransformersProvider {
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(&self, model: &ModelIdentity, text: &str) -> Result<Embedding, _> {
        // POST /v1/embeddings, parse JSON, return vector
        // Capture `x-served-model` header into served_version
    }
}
```

### 7.5 `MultiProviderEmbeddingClient` with circuit breaker

```rust
pub struct MultiProviderEmbeddingClient {
    providers: Vec<(String, Arc<dyn EmbeddingProvider>)>,
    breakers: DashMap<String, CircuitBreaker>,
}

pub struct CircuitBreaker {
    state: AtomicU8,        // 0 Closed, 1 Open, 2 HalfOpen
    consecutive_failures: AtomicU32,
    opened_at: AtomicU64,   // unix seconds
}

impl MultiProviderEmbeddingClient {
    pub fn breaker_state(&self, name: &str) -> CircuitBreakerState;
}

#[async_trait]
impl EmbeddingProvider for MultiProviderEmbeddingClient {
    async fn embed(&self, model: &ModelIdentity, text: &str) -> Result<Embedding, _> {
        for (name, provider) in &self.providers {
            if self.breaker_open(name) { continue; }
            match provider.embed(model, text).await {
                Ok(e) => { self.record_success(name); return Ok(e); }
                Err(e) => { self.record_failure(name, &e); }
            }
        }
        Err(EmbeddingProviderError::Backend { message: "all providers exhausted".into() })
    }
}
```

### 7.6 `DriftSweepCron`

```rust
pub struct DriftSweepCron {
    scheduler: tokio_cron_scheduler::JobScheduler,
    gate: Arc<GoldGate>,
    registry: Arc<dyn GoldSetRegistry>,
    pg: PgPool,            // for drift_state read/write
    n15_quota: Arc<N15Quota>,
}

impl DriftSweepCron {
    pub async fn start(self) -> Result<tokio::task::JoinHandle<()>, _>;
    async fn tick(&self);
    async fn sweep_agent(&self, agent: &NanoAgent) -> Result<(), _>;
}
```

### 7.7 `BypassSignatureVerifier`

```rust
pub struct BypassSignatureVerifier {
    verifier: Arc<dyn SignatureVerifier>,
    publisher_registry: Arc<dyn PublisherRegistry>,
}

impl BypassSignatureVerifier {
    pub async fn verify(
        &self,
        cert: &BypassCertificate,
        gold_run_id: GoldRunId,
    ) -> Result<(), GoldGateError> {
        for (idx, sig) in cert.approvers.iter().enumerate() {
            let key = self.publisher_registry
                .lookup_operator_key(sig.operator_id)
                .await?;
            let canonical = canonical_bypass_message(gold_run_id, cert);
            self.verifier.verify(&sig.signature, &canonical, &key.public_key_pem)
                .await
                .map_err(|_| GoldGateError::BypassSignatureInvalid { approver_index: idx as u32 })?;
        }
        Ok(())
    }
}

fn canonical_bypass_message(run_id: GoldRunId, cert: &BypassCertificate) -> Vec<u8> {
    // gold_run_id (16 bytes) || bypass_id (16) || ticket_id || rationale || valid_until (8 bytes BE)
}
```

### 7.8 `FirecrackerGoldRunner` + `EbpfHermeticityProbe`

```rust
pub struct FirecrackerGoldRunner {
    pool: Arc<dyn VmPool>,
    probe: Arc<EbpfHermeticityProbe>,
}

pub struct EbpfHermeticityProbe {
    skel: Mutex<HermeticityBpfSkel>,        // libbpf-rs handle
    egress_counts: DashMap<GoldCaseId, AtomicU64>,
}

impl EbpfHermeticityProbe {
    pub fn attach(&self, tap_interface: &str) -> Result<(), _>;
    pub fn detach(&self) -> u64;            // returns egress packet count
}

#[async_trait]
impl GoldRunner for FirecrackerGoldRunner {
    async fn run_case(&self, input: RunnerInput) -> Result<GoldCaseRun, _> {
        let vm = self.pool.acquire().await?;
        self.probe.attach(&vm.tap_interface)?;
        let result = vm.exec_case(&input).await;
        let egress = self.probe.detach();
        if egress > 0 {
            return Err(GoldRunnerError::HermeticityBroken { case_id: input.case.gold_case_id });
        }
        self.pool.release(vm).await;
        result
    }
}
```

### 7.9 N20 metrics (8 additional surfaces)

| # | Name | Type | Labels |
|---|---|---|---|
| 13 | `n20_postgres_pool_inflight` | gauge | `crate` |
| 14 | `n20_embedding_provider_calls_total` | counter | `provider`, `outcome` |
| 15 | `n20_embedding_provider_errors_total` | counter | `provider`, `kind` |
| 16 | `n20_circuit_breaker_state` | gauge (0/1/2) | `provider` |
| 17 | `n20_drift_sweep_runs_total` | counter | `outcome` |
| 18 | `n20_bypass_signature_verifications_total` | counter | `outcome` |
| 19 | `n20_ebpf_hermeticity_violations_total` | counter | `dest_class` |
| 20 | `n20_n18_promotions_total` | counter | `tier`, `bypassed` |

Total Prometheus surface across N19 + N20 = **20 metrics**, all
cardinality-bounded.

---

## 8. Module hierarchy & data model

### 8.1 Crate layout (extends `efe-gold-test-gate`)

```text
crates/efe-gold-test-gate/
├── migrations/                          (NEW)
│   ├── 20260820000001_initial_gold_schema.sql
│   ├── 20260820000002_drift_state.sql
│   └── 20260820000003_bypass_certificate.sql
├── src/
│   ├── lib.rs                            (extended re-exports)
│   ├── postgres/                          (NEW)
│   │   ├── mod.rs
│   │   ├── registry.rs                    (PostgresGoldSetRegistry)
│   │   ├── ledger.rs                      (PostgresGoldRunLedger)
│   │   └── drift_state.rs                 (per-agent last_run_at)
│   ├── embedding/
│   │   ├── mod.rs                         (existing trait)
│   │   ├── stub.rs                        (existing)
│   │   ├── openai.rs                      (NEW)
│   │   ├── cohere.rs                      (NEW)
│   │   ├── local.rs                       (NEW)
│   │   ├── multi_provider.rs              (NEW)
│   │   └── circuit_breaker.rs             (NEW)
│   ├── cron/                              (NEW)
│   │   └── drift_sweep.rs
│   ├── crypto/                            (NEW)
│   │   ├── mod.rs
│   │   └── bypass_verifier.rs
│   ├── runner/
│   │   ├── mod.rs                         (existing trait)
│   │   ├── memory.rs                      (existing)
│   │   ├── firecracker.rs                 (NEW)
│   │   └── ebpf.rs                        (NEW)
│   └── ... (existing N19 modules)
├── Cargo.toml                             (NEW deps : sqlx, reqwest, candle, libbpf-rs, tokio-cron-scheduler)
└── tests/
    ├── phase_4_n19_gold_test_gate.rs       (existing)
    ├── audit_n19_gold_test_gate.rs         (existing)
    └── phase_5_n20_*.rs                    (NEW)
```

### 8.2 Postgres schema (3 migration files)

#### `20260820000001_initial_gold_schema.sql`

```sql
CREATE SCHEMA IF NOT EXISTS gold;

-- Session role variable for RLS.
-- Application MUST `SET LOCAL efe.tenant_id` per transaction.

CREATE TABLE gold.gold_set (
    gold_set_id              UUID PRIMARY KEY,
    tenant_id                UUID NOT NULL,
    agent_id                 TEXT NOT NULL,
    agent_version            TEXT NOT NULL,
    agent_input_schema_hash  BYTEA NOT NULL,
    agent_output_schema_hash BYTEA NOT NULL,
    gold_set_hash            BYTEA NOT NULL UNIQUE,
    status                   TEXT NOT NULL CHECK (status IN ('draft','approved','archived')),
    provenance_json          JSONB NOT NULL,
    cost_cap_microcents      BIGINT NOT NULL CHECK (cost_cap_microcents > 0),
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    approved_at              TIMESTAMPTZ
);
CREATE INDEX ON gold.gold_set (agent_id, agent_version);
CREATE INDEX ON gold.gold_set (tenant_id);

ALTER TABLE gold.gold_set ENABLE ROW LEVEL SECURITY;
CREATE POLICY gold_set_tenant ON gold.gold_set
    USING (tenant_id::text = current_setting('efe.tenant_id', true));

CREATE TABLE gold.gold_case (
    gold_case_id     UUID PRIMARY KEY,
    gold_set_id      UUID NOT NULL REFERENCES gold.gold_set(gold_set_id) ON DELETE CASCADE,
    tenant_id        UUID NOT NULL,
    name             TEXT NOT NULL,
    case_json        JSONB NOT NULL,
    timeout_seconds  INTEGER NOT NULL CHECK (timeout_seconds > 0 AND timeout_seconds <= 300),
    tags             TEXT[] NOT NULL DEFAULT '{}'
);
CREATE INDEX ON gold.gold_case (gold_set_id);

ALTER TABLE gold.gold_case ENABLE ROW LEVEL SECURITY;
CREATE POLICY gold_case_tenant ON gold.gold_case
    USING (tenant_id::text = current_setting('efe.tenant_id', true));

CREATE TABLE gold.gold_set_approver (
    gold_set_id      UUID NOT NULL REFERENCES gold.gold_set(gold_set_id),
    operator_id      UUID NOT NULL,
    signed_at        TIMESTAMPTZ NOT NULL,
    ticket_id        TEXT NOT NULL,
    signature        BYTEA NOT NULL,
    tenant_id        UUID NOT NULL,
    PRIMARY KEY (gold_set_id, operator_id)
);
ALTER TABLE gold.gold_set_approver ENABLE ROW LEVEL SECURITY;
CREATE POLICY gold_set_approver_tenant ON gold.gold_set_approver
    USING (tenant_id::text = current_setting('efe.tenant_id', true));

CREATE TABLE gold.gold_run_ledger (
    sequence_number   BIGSERIAL PRIMARY KEY,
    gold_run_id       UUID NOT NULL UNIQUE,
    trace_id          UUID NOT NULL,
    agent_id          TEXT NOT NULL,
    agent_version     TEXT NOT NULL,
    gold_set_id       UUID NOT NULL,
    tenant_id         UUID NOT NULL,
    entry_kind        TEXT NOT NULL,
    canonical_body    BYTEA NOT NULL,
    hmac              BYTEA NOT NULL,
    prev_trace_id     UUID,
    overall_verdict   TEXT NOT NULL,
    cost_actual_microcents BIGINT NOT NULL DEFAULT 0,
    bypass_used       BOOLEAN NOT NULL DEFAULT FALSE,
    body_json         JSONB NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX ON gold.gold_run_ledger (agent_id, agent_version, created_at DESC);
CREATE INDEX ON gold.gold_run_ledger (gold_set_id);

ALTER TABLE gold.gold_run_ledger ENABLE ROW LEVEL SECURITY;
CREATE POLICY gold_run_ledger_tenant ON gold.gold_run_ledger
    USING (tenant_id::text = current_setting('efe.tenant_id', true));
```

#### `20260820000002_drift_state.sql`

```sql
CREATE TABLE gold.gold_set_drift_state (
    agent_id           TEXT NOT NULL,
    agent_version      TEXT NOT NULL,
    tenant_id          UUID NOT NULL,
    last_run_at        TIMESTAMPTZ NOT NULL,
    last_gold_run_id   UUID NOT NULL,
    last_outcome       TEXT NOT NULL,
    PRIMARY KEY (agent_id, agent_version)
);
ALTER TABLE gold.gold_set_drift_state ENABLE ROW LEVEL SECURITY;
CREATE POLICY drift_tenant ON gold.gold_set_drift_state
    USING (tenant_id::text = current_setting('efe.tenant_id', true));
```

#### `20260820000003_bypass_certificate.sql`

```sql
CREATE TABLE gold.bypass_certificate (
    bypass_id          UUID PRIMARY KEY,
    gold_run_id        UUID NOT NULL REFERENCES gold.gold_run_ledger(gold_run_id),
    tenant_id          UUID NOT NULL,
    ticket_id          TEXT NOT NULL CHECK (ticket_id ~ '^(INC|SEC|GOLD)-\d+$'),
    rationale          TEXT NOT NULL CHECK (char_length(rationale) >= 64),
    valid_until        TIMESTAMPTZ NOT NULL,
    approvers_json     JSONB NOT NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
ALTER TABLE gold.bypass_certificate ENABLE ROW LEVEL SECURITY;
CREATE POLICY bypass_tenant ON gold.bypass_certificate
    USING (tenant_id::text = current_setting('efe.tenant_id', true));
```

#### N18 ledger extension (touches `efe-publisher-identity` migrations)

```sql
-- N18 publish_ledger.entry already supports JSONB ; the new
-- variant `AgentPromotedToTier1` lands via a code-only change to
-- PublishLedgerEntry enum.  Postgres TRIGGER added :

CREATE OR REPLACE FUNCTION enforce_tier_1_audit() RETURNS TRIGGER AS $$
DECLARE
    audit_count INTEGER;
BEGIN
    IF NEW.stability_tier = 'tier1' AND OLD.stability_tier != 'tier1' THEN
        SELECT COUNT(*) INTO audit_count
        FROM publisher.publish_ledger
        WHERE entry_json->>'kind' = 'agent_promoted_to_tier_1'
          AND entry_json->>'agent_id' = NEW.id
          AND entry_json->>'agent_version' = NEW.version
          AND created_at > NOW() - INTERVAL '5 minutes';
        IF audit_count = 0 THEN
            RAISE EXCEPTION 'unaudited tier 1 promote : no AgentPromotedToTier1 ledger row in last 5 minutes';
        END IF;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tier_1_audit_trigger
    BEFORE UPDATE OF stability_tier ON catalog.entries
    FOR EACH ROW
    EXECUTE FUNCTION enforce_tier_1_audit();
```

---

## 9. API surface (HTTP + internal Rust traits)

### 9.1 HTTP endpoints (extends N19 § 9.1)

```text
# N20 new endpoints

GET  /v1/admin/drift-sweep/last-tick     # ops dashboard polling
POST /v1/admin/drift-sweep/trigger       # manual override (operator JWT)
GET  /v1/admin/circuit-breakers          # per-provider state
POST /v1/admin/circuit-breakers/{provider}/reset   # ops force-close

# N18-cross-cutting endpoints (delivered by efe-publisher-identity Sprint 12 patch)

GET  /v1/publishers/{publisher_id}/ledger/promotions
                                          # list AgentPromotedToTier1 rows
```

### 9.2 Internal Rust traits

All existing N19 traits are preserved.  N20 adds :

```rust
pub trait DriftSweepDriver: Send + Sync + 'static {
    async fn start(self: Arc<Self>) -> Result<JoinHandle<()>, _>;
}

pub trait CircuitBreaker: Send + Sync + 'static {
    fn state(&self) -> CircuitBreakerState;
    fn record_success(&self);
    fn record_failure(&self, kind: FailureKind);
}
```

---

## 10. End-to-end workflows

### 10.1 Workflow A — Postgres-backed Tier 1 promotion

```
1. Publisher publishes v1.2.3 (signed via N18).
2. Operator submits a GoldSet via POST /v1/gold-sets.
   → PostgresGoldSetRegistry::register
   → BEGIN ; SET LOCAL efe.tenant_id ; INSERT rows ; COMMIT
3. Two operators sign the approval ; INSERTed into gold_set_approver.
4. Operator triggers POST /v1/gold-runs.
5. GoldGate.run :
   a. Lookup the gold set (Postgres SELECT with RLS).
   b. Verify gold_set_hash matches content (N19 invariant B2).
   c. FirecrackerGoldRunner.run_case per case :
      - acquire VM from N11 pool
      - attach EbpfHermeticityProbe
      - exec case (frozen clock)
      - detach probe ; verify egress == 0
   d. DefaultAssertionEngine.evaluate ; uses MultiProviderEmbeddingClient.
   e. PostgresGoldRunLedger.append (INSERT gold_run_ledger row).
6. If overall == Passed :
   → N18 PublishLedger.append(AgentPromotedToTier1 { gold_run_id })
   → UPDATE catalog.entries SET stability_tier = 'tier1' ;
   → Postgres TRIGGER verifies the ledger row exists ; commits.
```

### 10.2 Workflow B — Crypto-verified bypass

```
1. Sev-1 incident ; hot-fix needed.
2. 2-of-3 EFE engineers each sign a canonical message :
   canonical = gold_run_id || bypass_id || ticket || rationale || valid_until
   sig_i = Ed25519.sign(operator_i_private_key, canonical)
3. Operator submits POST /v1/gold-runs with bypass certificate.
4. GoldGate.run_bypass :
   a. BypassCertificate.validate_shape (N19 check)
   b. BypassSignatureVerifier.verify :
      - for each approver, lookup public key in publisher registry
      - verify sig against canonical message via N18 Ed25519Verifier
      - any verification failure → BypassSignatureInvalid
   c. PostgresGoldRunLedger.append(GoldRunBypassed { certificate })
   d. INSERT bypass_certificate row (RLS-protected).
5. N18 PublishLedger.append(AgentPromotedToTier1 { gold_run_id,
     bypass: Some(certificate) })
6. Catalog flag : agent shows "BYPASSED" until next clean run.
```

### 10.3 Workflow C — Daily cron drift sweep

```
03:00 UTC daily :
1. DriftSweepCron.tick fires.
2. SELECT agents FROM catalog WHERE stability_tier = 'tier1'.
3. For each agent :
   a. SELECT last_run_at FROM gold_set_drift_state
      WHERE agent_id = ? AND agent_version = ? ;
   b. If last_run_at > NOW() - 23 hours, skip (I12).
   c. Acquire N15 OperationClass::GoldRun slot.
   d. Lookup latest approved gold set (Postgres).
   e. Run GoldGate.run.
   f. UPSERT into gold_set_drift_state with new last_run_at +
      gold_run_id + outcome.
   g. If outcome == Failed → N18 PublishLedger.append(AgentDemoted)
      → UPDATE catalog SET stability_tier = 'tier_experimental'.
4. Metrics : n20_drift_sweep_runs_total += 1 per agent.
```

### 10.4 Workflow D — Embedding multi-provider failover

```
Case asks for SemanticSimilarity { embedding_model: openai/text-
embedding-3-small/1 } :
1. MultiProviderEmbeddingClient.embed :
   for provider in [openai, cohere, local] :
     if breaker.state(provider) == Open : continue
     match provider.embed(model, text).await :
       Ok(e) : breaker.record_success ; return Ok(e)
       Err(RateLimited) :
         breaker.record_failure ; metrics.tick("rate_limited") ; continue
       Err(Backend) :
         breaker.record_failure ; continue
   return Err(Backend { "all providers exhausted" })
2. If all 3 fail → case errors with EmbeddingProviderError.
3. Drift sweep flags this in metrics ; ops alerted.
```

### 10.5 Workflow E — eBPF hermeticity violation detection

```
Test gold case runs an agent that has a bug : it tries to call
api.openai.com mid-case.

1. FirecrackerGoldRunner.run_case starts.
2. EbpfHermeticityProbe attaches to vm.tap_interface.
3. VM boots ; agent code runs.
4. Agent issues `connect(api.openai.com:443)` ; SYN packet hits
   the tap interface.
5. eBPF program tagged on tc-egress sees the packet ;
   increments egress_counts[case_id].
6. Case execution finishes ; EbpfHermeticityProbe.detach :
   egress = 1 ; > 0 ⇒ HermeticityBroken.
7. FirecrackerGoldRunner returns GoldRunnerError::HermeticityBroken.
8. GoldGate aborts the run with overall ==
   AbortedHermeticityBroken { case_id }.
9. Sev-1 alert : the agent has unhermetic code.
```

---

## 11. Five-day implementation plan

### Day 46 — Postgres registry + migrations
* `migrations/` setup with `sqlx-cli`.
* Migration 001 : `gold_set` + `gold_case` + `gold_set_approver` + RLS.
* `PostgresGoldSetRegistry` impl (~600 LOC).
* `docker-compose.gold-pg.yml` for dev.
* 25+ unit tests (registry round-trip, RLS isolation, etc.).

### Day 47 — Postgres ledger + chain verification
* Migration 002 : `drift_state`.
* Migration 003 : `bypass_certificate`.
* `PostgresGoldRunLedger` impl (~700 LOC).
* `verify_chain` walking Postgres rows by sequence_number.
* 30+ unit tests.

### Day 48 — Three real embedding providers + multi-provider client
* `openai.rs` (~250 LOC) + `cohere.rs` (~250 LOC) + `local.rs`
  (~400 LOC, depends on `candle`).
* `multi_provider.rs` (~350 LOC) + `circuit_breaker.rs` (~150 LOC).
* 40+ unit tests with `wiremock` for HTTP mocking.

### Day 49 — Cron sweep + bypass verifier + N18 ledger variant
* `cron/drift_sweep.rs` (~400 LOC).
* `crypto/bypass_verifier.rs` (~200 LOC) — reuses N18 verifier.
* N18 ledger `AgentPromotedToTier1` variant added to
  `efe-publisher-identity`.
* Postgres TRIGGER `enforce_tier_1_audit` migration in
  `efe-publisher-identity/migrations/`.
* 25+ unit tests.

### Day 50 — FirecrackerGoldRunner + eBPF + acceptance tests
* `runner/firecracker.rs` (~500 LOC) — wires N11 pool.
* `runner/ebpf.rs` (~400 LOC) — `libbpf-rs` skeleton.
* `bpf/hermeticity.bpf.c` (~150 LOC of BPF C).
* `tests/phase_5_n20_*.rs` — 20 acceptance tests.
* Clippy strict + workspace build + 6-crate test suite green.

---

## 12. Test strategy

### 12.1 Acceptance test matrix (20 tests, one per G + cross-cutting)

```
phase_5_n20_postgres_registry_round_trip
phase_5_n20_postgres_ledger_chain_verifies
phase_5_n20_pg_dump_round_trip
phase_5_n20_openai_provider_smoke                 (gated)
phase_5_n20_cohere_provider_smoke                 (gated)
phase_5_n20_local_provider_returns_consistent_vectors
phase_5_n20_multi_provider_failover_under_rate_limit
phase_5_n20_drift_sweep_cron_fires_and_demotes
phase_5_n20_drift_sweep_idempotent_after_restart
phase_5_n20_bypass_empty_signature_rejected
phase_5_n20_promote_emits_n18_ledger_entry
phase_5_n20_sql_backdoor_promote_blocked
phase_5_n20_firecracker_runner_executes_case
phase_5_n20_ebpf_probe_fires_on_curl_outbound
phase_5_n20_metrics_emit_all_20_surfaces
phase_5_n20_rls_blocks_cross_tenant_read
phase_5_n20_cache_hit_drives_marginal_cost_zero
phase_5_n20_circuit_breaker_opens_after_three_failures
phase_5_n20_circuit_breaker_half_opens_after_30s
phase_5_n20_in_memory_impls_still_work
```

### 12.2 Property tests

* `proptest_canonical_bypass_message_stable` — `(gold_run_id,
  bypass_id, ticket, rationale, valid_until)` always produces the
  same canonical bytes.
* `proptest_circuit_breaker_state_transitions` — Closed → Open
  after 3 fails ; Open → HalfOpen after 30 s ; HalfOpen → Closed
  after 1 success ; HalfOpen → Open after 1 fail.
* `proptest_embedding_cache_key_stable` — `(model, text)` →
  always same Blake3 key.

### 12.3 Chaos tests

* `chaos_postgres_failover_mid_transaction` — kill primary mid-
  INSERT ; verify sqlx retries.
* `chaos_openai_rate_limit_burst` — wiremock returns 100 × 429 ;
  verify failover to Cohere.
* `chaos_cron_clock_jump` — system clock jumps 24h forward ;
  verify monotonic-clock cron isn't fooled.
* `chaos_ebpf_attach_fails_runner_refuses` — simulate libbpf-rs
  error ; verify runner fails-closed.

### 12.4 Security tests

* `security_rls_blocks_cross_tenant_select` — tenant A's session
  can't `SELECT *` tenant B's rows.
* `security_sql_injection_in_gold_set_name_safe` — bobby-tables
  doesn't run.
* `security_bypass_canonical_message_tamper_detected` — flip any
  byte ⇒ verify fails.
* `security_ed25519_constant_time_verification` — verification
  time independent of bytes.

### 12.5 Cross-cutting smoke tests

* `cross_n11_n20_firecracker_pool_acquires_vm_within_500ms`.
* `cross_n12_n20_embedding_cache_hit_drives_cost_to_zero`.
* `cross_n17_n20_gold_case_id_on_real_llm_call`.
* `cross_n18_n20_promote_triggers_n18_ledger_row`.
* `cross_n19_n20_in_memory_and_postgres_impls_are_interchangeable`.

---

## 13. Cross-cutting alignment (N11 → N19)

| Subsystem | Concern | N20 resolution |
|---|---|---|
| **N11 microVM pool** | Hermetic isolation per case | `FirecrackerGoldRunner` consumes `Arc<dyn VmPool>` |
| **N12 cache** | Drive embedding marginal cost down | Cache key includes `(model.lineage_key, text_blake3)` |
| **N13 canary** | Tier 1 promote routes through canary | N18 `AgentPromotedToTier1` row appears in canary state machine |
| **N14 envelope** | Carries `gold_run_id` on cron-sweep dispatches | `envelope.gold_run_id` set by `DriftSweepCron` |
| **N15 fairness** | Sweep mustn't starve customer traffic | `OperationClass::GoldRun` quota enforced |
| **N16 multi-region** | Per-region Postgres | One Postgres cluster per region ; cross-region sync = N21 |
| **N17 provenance** | Gold-driven LLM calls attributable | `LlmProvenanceRecord.gold_case_id` auto-set by `FirecrackerGoldRunner` |
| **N18 publisher** | Promote = ledger row | `PublishLedgerEntry::AgentPromotedToTier1` variant added |
| **N19 gold gate** | Drop-in upgrade | All N19 traits preserved ; Postgres impls implement same traits |

---

## 14. Sprint 12 ship vs Sprint 13/14 split

### Sprint 12 (this design)
* All 7 production capabilities.
* 20 acceptance tests + 4 property + 4 chaos + 4 security + 5 cross-cutting.
* In-memory + Postgres impls coexist.
* Stub + 3 real embedding providers ship.
* eBPF probe + Firecracker runner ship.
* N18 `AgentPromotedToTier1` variant ships.

### Sprint 13 N21 — Shadow capture + cross-region gold-set sync
* `Provenance::ShadowCapture` (N13 → propose gold cases).
* N16-driven cross-region gold-set shipment.
* Postgres-backed `efe-publisher-identity` registry (was in-memory pre-Sprint 13).

### Sprint 14 N22 — LLM-as-judge experimental strategy
* `GoldAssertionStrategy::LlmJudge { judge_model, rubric_id }`.
* Self-supervised rubric tuning.
* Cross-version diff visualisation.

### Sprint 15 GA — Sigstore signing + v1.0 release
* All M50 sprints frozen ; release artifact build pipeline.
* Sigstore-signed CLI binaries.
* CHANGELOG-driven release notes.

---

## 15. Annexes

### A1 — Cargo dependencies added in Sprint 12

```toml
[dependencies]
sqlx = { version = "0.8", features = ["postgres", "uuid", "chrono", "json", "macros", "runtime-tokio"] }
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
tokio-cron-scheduler = "0.10"
secrecy = "0.10"
candle-core = { version = "0.6", default-features = false }
candle-transformers = { version = "0.6", default-features = false }
candle-nn = { version = "0.6" }
tokenizers = "0.20"
libbpf-rs = "0.24"

[build-dependencies]
sqlx-cli = { version = "0.8", default-features = false, features = ["postgres"] }

[features]
default = ["postgres", "real-embeddings"]
postgres = []
real-embeddings = []
stub-only = []  # tests / CI minimal build
```

### A2 — Canonical bypass message format

```
| Byte range | Field | Encoding |
|---|---|---|
| 0..16 | gold_run_id | UUID big-endian bytes |
| 16..32 | bypass_id | UUID big-endian bytes |
| 32..40 | valid_until_unix | i64 big-endian |
| 40..40+L | ticket_id_len + ticket_id | u32 BE length || UTF-8 bytes |
| ... | rationale_len + rationale | u32 BE length || UTF-8 bytes |
```

### A3 — Circuit breaker state machine

```text
                 3 consecutive failures
       Closed ──────────────────────────►  Open
         ▲                                   │
         │                                   │ 30 s elapsed
         │ 1 success                         ▼
         └─────────────── HalfOpen ◄─────────┘
                                │
                                │ 1 failure
                                ▼
                              Open
```

### A4 — Operator runbook

#### A4.1 Bootstrap Postgres for dev
```bash
docker-compose -f docker-compose.gold-pg.yml up -d
export DATABASE_URL='postgres://efe:efe@localhost:5432/efe_gold'
cd crates/efe-gold-test-gate
sqlx migrate run
```

#### A4.2 Trigger a manual drift sweep
```bash
efe-gold drift-sweep run --agent <agent_id> --version 1.2.3
```

#### A4.3 Inspect circuit breakers
```bash
curl -s localhost:8080/v1/admin/circuit-breakers | jq .
```

#### A4.4 Reset a stuck breaker
```bash
curl -X POST localhost:8080/v1/admin/circuit-breakers/openai/reset
```

#### A4.5 Verify a SQL-backdoor was caught
```sql
-- Any unaudited UPDATE OF stability_tier raises the trigger.
UPDATE catalog.entries SET stability_tier = 'tier1' WHERE id = 'x';
-- ERROR:  unaudited tier 1 promote : no AgentPromotedToTier1 ledger row in last 5 minutes
```

### A5 — Comparison with industry approaches

| System | Their persistence | N20 improvement |
|---|---|---|
| OpenAI Evals | local SQLite | + Postgres RLS, cross-region |
| Anthropic eval pipelines | internal proprietary | + open SDK trait surface |
| Langfuse | Postgres | + crypto-verified bypass, eBPF hermeticity |
| Snorkel | local file system | + ACID + audit ledger |
| Wandb evals | proprietary cloud | + self-hostable, no vendor lock-in |

---

**End of N20 design.**

**Author** : EFE M50 architecture team.
**Date** : 2026-05-21.
**Sprint** : 12 (Phase 5, day 46 – 50).
**Design lineage** : N11 + N12 + N13 + N14 + N15 + N16 + N17 + N18 +
N19 → **N20** (this doc) → N21 shadow capture + cross-region →
N22 LLM-as-judge → Sprint 15 GA Sigstore + v1.0.

**Last-mile checklist** (Sprint 12 exit) :
- [ ] All 20 acceptance tests GREEN.
- [ ] Clippy strict (`-D warnings`) on N20 module.
- [ ] Postgres migrations idempotent (`sqlx migrate run` twice → no-op).
- [ ] Real-provider tests gated on env vars ; CI passes without keys.
- [ ] eBPF probe ships with CAP_BPF; CI uses --privileged Docker for tests.
- [ ] Drift sweep cron has PagerDuty alert if no tick > 26h.
- [ ] N18 trigger blocks unaudited tier 1 promotes — pen-tested.
- [ ] Multi-provider failover tested with 3-provider outage simulation.
- [ ] In-memory impls preserved + tested (backward compat).
- [ ] Two reference deployments live : EU-West-1 + US-East-1.
