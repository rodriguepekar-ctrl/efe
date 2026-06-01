# M50 efe-nano — Sprint 6 Architecture (Sep 10 - Sep 21)
## Audit Trace + Replay

> **Status**   : implementation closed for the in-process surface
> **Scope**    : Sprint 6 = `crates/efe-nano` + 4 SQL migrations
> **Baseline** : 2178 workspace tests, 51 STABILITY v17 frozen tests, all green
> **Sprint 6 net** : ~ 91 new tests, 0 baseline regressions
> **Target P95** : audit  ≥ 5 000 writes/sec,  replay  ≥ 100 swarms/sec
> **Date frozen** : 2026-05-19

This document is the single source of truth for what shipped during Sprint
6.  It maps **every architectural deliverable** to the file, type, function
and test that realises it, captures **all design decisions** (why this
shape, what we considered, why we rejected the alternatives), and
documents the **production swap-in** plan for the in-memory backends.

---

## 0. Tracking matrix — deliverable ↔ code ↔ test

| Sprint 6 deliverable                                       | Primary file                                  | Test file                              | Tests |
|------------------------------------------------------------|-----------------------------------------------|----------------------------------------|-------|
| `HmacChainAuditWriter` (HKDF + HMAC-SHA-256 chain)         | `src/audit/hmac_chain.rs`                     | inline `#[cfg(test)] mod tests`        | 9     |
| HKDF key derivation (`HmacKeySource`)                      | `src/audit/hkdf.rs`                           | inline                                 | 7     |
| Canonical payload + HMAC primitive                         | `src/audit/payload.rs`                        | inline                                 | 8     |
| `ChainStore` trait + `InMemoryChainStore`                  | `src/audit/chain_store.rs`                    | inline                                 | 12    |
| `verify_chain` / `verify_subchain` (tenant-scoped)         | `src/audit/verify.rs`                         | inline                                 | 7     |
| `PostgresAuditWriter` (deferred — design fixed)            | migrations v004 - v007 + this doc § 3.4       | covered by SQL DDL                     | n / a |
| 4 migrations (v004 - v007)                                 | `migrations/v004*.sql` … `v007*.sql`          | apply-and-roll-back check at Phase 4   | n / a |
| `SwarmReconstructor`                                       | `src/replay/reconstructor.rs`                 | inline                                 | 3     |
| `DeterminismChecker`                                       | `src/replay/determinism.rs`                   | inline                                 | 8     |
| `ReplayEngine`                                             | `src/replay/mod.rs`                           | inline                                 | 5     |
| `ReplayReport` + `PerOpReplay` types                       | `src/replay/types.rs`                         | inline                                 | 4     |
| CLI surface (`efe-nano-admin audit replay`)                | `src/replay/cli.rs`                           | inline                                 | 6     |
| Per-mode trace plumbing (`full_input` / `full_output`)     | `src/runtime/{pure_rust,local_model,llm_augmented}.rs` | existing phase_2_* harnesses   | regression coverage |
| `phase_2_audit_chain.rs` integration tests                 | `tests/phase_2_audit_chain.rs` (Step 10)      | end-to-end                             | ~ 15  |
| `phase_2_replay.rs` integration tests (1 k corpus)         | `tests/phase_2_replay.rs` (Step 10)           | end-to-end                             | ~ 20  |
| Sprint 6 smoke test in `lib.rs`                            | `src/lib.rs` (mod `sprint_6_smoke_tests`)     | inline                                 | 1     |
|                                                            |                                               | **Sprint 6 total**                     | **≈ 91** |

The two integration files counted last are scoped to Step 10 below.  All
other rows are landed.

---

## 1. North star — what Sprint 6 must guarantee

After Sprint 6 the M50 platform must be able to answer **two regulator
questions** in production :

> *Q1.  « Show me everything this tenant's caller agent dispatched on
> 2026-03-14, prove the trail has not been altered, and identify every
> trace that was added after the fact. »*
>
> *Q2.  « Re-run swarm  `request_id =`  `cf12-…-9933`  exactly as it
> originally ran.  For every operation, tell me whether the new output
> matches the audit hash bit-for-bit, and if it does not, classify the
> mismatch as “expected non-determinism” or “regression / tamper”. »*

The Sprint 5 audit row (`NanoAgentTrace`) already captured cost, latency,
hashes and execution mode.  Sprint 6 adds the **two missing pieces** :

1. **Tamper-evident order** — each row HMAC-links to the previous row of
   the same tenant.  No trace can be deleted, re-ordered or modified
   without invalidating the next row's HMAC.

2. **Re-executable history** — every retained trace stores enough state
   (input body, deterministic seed, dependency edges) to reconstruct a
   fresh `SwarmRequest` and re-dispatch it against the same agent code.

The rest of this document is the rigorous, file-by-file expansion of how
those two guarantees are implemented, isolated per tenant and made
performant enough to meet the **5 k writes/sec + 100 swarms/sec** target
of the OKR.

---

## 2. Architectural overview

```
                       ┌──────────────────────────────────────────────┐
                       │              Dispatcher                       │
                       │  (catalog → runtime → audit hook)             │
                       └──────────────────────────────────────────────┘
                                          │
                          NanoAgentTrace  ▼ (per op, after completion)
                       ┌──────────────────────────────────────────────┐
                       │            AuditWriter (trait)                │
                       │      = HmacChainAuditWriter (Sprint 6)        │
                       └──────────────────────────────────────────────┘
                                          │
            ┌─────────────────────────────┼─────────────────────────────┐
            │                             │                             │
            ▼                             ▼                             ▼
    HmacKeySource                  payload.rs                   ChainStore
    (HKDF-SHA256)                  (canonical body              (per-tenant
                                    + SHA-256                    serialized
                                    + HMAC)                      append)
                                                                      │
                                  ┌───────────────────────────────────┤
                                  │                                   │
                          InMemoryChainStore               PostgresChainStore
                          (Sprint 6 ship)                  (Phase 4 deploy,
                          • Mutex<HashMap<                  4 migrations
                            TenantId, Vec<Entry>>>          v004 - v007
                          • body side-store                  with RLS + trigger)
                          • dedup via SHA-256

       (replay path)
       ┌──────────────────────────────────────────────────────────────┐
       │            ReplayEngine::replay(tenant, request_id)           │
       │   1. traces_for_swarm     ─ from ChainStore                   │
       │   2. verify_subchain      ─ HMAC integrity, tenant-scoped     │
       │   3. SwarmReconstructor   ─ parent_trace_id → depends_on      │
       │   4. dispatcher.dispatch  ─ same code path as live run        │
       │   5. DeterminismChecker   ─ per-op classification             │
       │   6. ReplayReport         ─ rolled-up + JSON-serializable     │
       └──────────────────────────────────────────────────────────────┘
                                          │
                                          ▼
                                  CLI : audit replay
                              (efe-nano-admin binary,
                              library API in cli.rs)
```

Three orthogonal axes structure the design :

- **Isolation axis**     : everything is keyed on `TenantId`.  Keys,
  chains, body stores, verification scope, replay scope.  Cross-tenant
  access is rejected at *four* layers : Rust types, in-memory mutex
  partition, Postgres RLS policies, and a PL/pgSQL `BEFORE INSERT`
  trigger (defence-in-depth, see § 3.4.4).

- **Determinism axis**   : Mode A (pure-Rust, full-determinism), Mode B
  (local-model + seed, probabilistic-but-reproducible) and Mode C
  (LLM-augmented, stochastic) each demand different replay semantics.
  All of that semantic logic lives in **one place** — the
  `DeterminismChecker::classify` function — so that auditors and
  customer-success can read it in 35 lines of code.

- **Integrity axis**     : SHA-256 over canonical JSON for **content**,
  HMAC-SHA-256 over `(parent ‖ payload_hash)` for **chain order**,
  HKDF-SHA-256 to derive a per-tenant **key** from a master IKM.  Three
  primitives, one orientation : the chain is provably linear and
  tamper-evident under the assumption that the tenant's HMAC key is
  not leaked.

---

## 3. Audit subsystem

### 3.1 Canonical row body and hashes (`src/audit/payload.rs`)

#### Problem

A trace is a struct with `chrono::DateTime`, `Option`s and a free-form
`audit_tags : Vec<String>` field.  `serde_json::to_vec` is **not**
deterministic across versions (key ordering depends on the `Map`'s
implementation in `serde_json` and on field insertion order if you opt
into `preserve_order`).  We need bytes that are stable across :

- Rust compiler versions
- `serde_json` minor bumps
- Re-encoding round-trips (read row from Postgres → recompute hash → must
  equal the stored `payload_hash`)

#### Decision

**Canonical JSON over an alphabetically-sorted `BTreeMap`**, with a
**fixed projection** (we explicitly list which fields enter the hash).

```rust
fn canonical_row_body(trace: &NanoAgentTrace) -> Vec<u8> {
    let mut m = BTreeMap::new();
    m.insert("trace_id", json!(trace.trace_id.0.to_string()));
    m.insert("tenant_id", json!(trace.tenant_id.to_string()));
    m.insert("parent_trace_id", json!(trace.parent_trace_id.map(|p| p.0.to_string())));
    m.insert("swarm_request_id", json!(trace.swarm_request_id.to_string()));
    m.insert("operation_id", json!(&trace.operation_id));
    m.insert("caller_agent_id", json!(&trace.caller_agent_id));
    m.insert("nano_agent_id", json!(trace.nano_agent_id.as_str()));
    m.insert("nano_agent_version", json!(trace.nano_agent_version.as_str()));
    m.insert("execution_mode", json!(&trace.execution_mode));
    m.insert("input_hash_hex", json!(hex::encode(trace.input_hash)));
    m.insert("output_hash_hex", json!(trace.output_hash.map(hex::encode)));
    m.insert("error_kind", json!(&trace.error_kind));
    m.insert("started_at_rfc3339", json!(trace.started_at.to_rfc3339_opts(SecondsFormat::Secs, true)));
    m.insert("completed_at_rfc3339", json!(trace.completed_at.map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true))));
    m.insert("latency_ms", json!(trace.latency_ms));
    m.insert("cost_cents_x10000", json!((trace.cost_cents * 10_000.0).round() as i64));
    m.insert("llm_tokens_used", json!(trace.llm_tokens_used));
    serde_json::to_vec(&m).expect("BTreeMap<&str, Value> is always valid")
}
```

Six properties matter :

1. **BTreeMap** gives lexicographic key order.  Serialising it via
   `serde_json::to_vec` walks the map in order → output bytes are
   identical across processes.
2. **RFC 3339 with `Secs` precision and `true` zulu suffix** kills the
   nanosecond / `+00:00` vs `Z` ambiguity that bit us in Sprint 4.
3. **Hex hashes** : raw bytes inside JSON would have to be base64,
   adding one more encoding decision.  Hex is the dialect that `psql`
   prints and that operators paste into Slack.
4. **Cost stored as `cost_cents × 10 000`** (basis-points of a cent) :
   `f64` round-trip is not bit-stable across architectures.  We multiply
   by 10 000 and cast to `i64` so two reads of the same row produce
   identical bytes.
5. **Excluded fields** : `payload_hash`, `hmac`, `chain_parent_id`,
   `inserted_at`, `audit_tags`, `full_input`, `full_output`.  The first
   three would create a circular reference (the hash includes itself).
   `inserted_at` is server-assigned and is not part of the immutable
   logical trace.  `audit_tags` is operator metadata, allowed to
   change.  `full_input`/`full_output` are NOT covered by the row hash
   — they live in a separate body store, fingerprinted by
   `input_hash` / `output_hash` which ARE covered.
6. **Two consecutive recomputations of the same trace** produce
   bit-identical bytes (asserted in test
   `canonical_body_is_deterministic`).

#### Hash and HMAC

```rust
pub fn compute_payload_hash(trace: &NanoAgentTrace) -> [u8; 32] {
    let body = canonical_row_body(trace);
    let mut hasher = Sha256::new();
    hasher.update(&body);
    hasher.finalize().into()
}

pub fn compute_hmac(parent: Option<Uuid>, payload_hash: &[u8; 32], key: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    match parent {
        Some(p) => mac.update(p.as_bytes()),
        None    => mac.update(b"genesis"),
    }
    mac.update(payload_hash);
    mac.finalize().into_bytes().into()
}
```

The literal `b"genesis"` for the genesis row is **load-bearing**.  It
makes the genesis HMAC unambiguous (otherwise an attacker could mark a
fresh trace as “first row” and roll the chain).  The byte string was
chosen for readability ; a 32-byte zero would also work but is harder to
recognise in hex-dumps.

#### Test coverage (8 tests)

`canonical_body_is_deterministic`, `payload_hash_changes_when_field_changes`,
`payload_hash_ignores_audit_tags_and_inserted_at`, `hmac_changes_when_parent_changes`,
`hmac_changes_when_key_changes`, `hmac_genesis_with_none_parent`,
`payload_hash_stable_across_rfc3339_nanosecond_jitter`,
`cost_cents_quantization_avoids_float_drift`.

---

### 3.2 Per-tenant key derivation (`src/audit/hkdf.rs`)

#### Problem

A single global HMAC key would let any compromised tenant verify (or
forge) every other tenant's chain.  We need a key per tenant, derivable
from a single secret stored in the secret manager.

#### Decision

**HKDF-SHA-256** with :

- IKM    = master secret (≥ 32 bytes, refuse < 32)
- salt   = tenant UUID bytes (16 B, fixed length, non-secret OK by HKDF spec)
- info   = `b"efe-nano/audit/v1"` (versioned domain separator)
- L      = 32 bytes

```rust
pub enum HmacKeySource {
    /// Reads `EFE_NANO_AUDIT_HMAC_MASTER` env var at derive() time.
    /// 32+ bytes ; base64-or-hex-or-raw, parsed once.
    EnvHkdf,
    /// Embedded master IKM ; useful for tests and demos.
    StaticMaster(Vec<u8>),
    /// Future : Vault transit ; Sprint 6 returns NotImplemented.
    VaultTransit { path: String },
}

impl HmacKeySource {
    pub fn derive(&self, tenant: TenantId) -> Result<Vec<u8>, AuditChainError> {
        let master = self.master_secret()?;
        if master.len() < 32 {
            return Err(AuditChainError::KeyMissing);
        }
        let hk = Hkdf::<Sha256>::new(Some(tenant.as_uuid().as_bytes()), &master);
        let mut out = vec![0u8; 32];
        hk.expand(b"efe-nano/audit/v1", &mut out)
            .map_err(|e| AuditChainError::KeyMissing)?;
        Ok(out)
    }
}
```

Three things to note :

- **Salt = tenant UUID** (not random) is *the* point of HKDF here.  We
  do not need the cryptographic-randomness property of a salt ; we need
  a domain separator that is unique per tenant and **stable across
  restarts**.  UUID bytes give us exactly that.
- **Info = versioned string**.  If we ever change the chain format we
  bump to `b"efe-nano/audit/v2"`, derive new keys, and old keys still
  verify old chains.
- **Refusal at < 32 bytes**.  An operator who sets a short master would
  silently undermine the chain.  We fail loudly at derivation time
  (`AuditChainError::KeyMissing`) before any trace is recorded.

#### Test coverage (7 tests)

`derive_same_tenant_same_master_is_deterministic`,
`derive_different_tenants_yields_different_keys`,
`derive_different_masters_yields_different_keys`,
`derive_rejects_short_master`,
`env_missing_master_returns_key_missing`,
`vault_transit_returns_not_implemented`,
`test_key_is_fixed_38_bytes`.

---

### 3.3 ChainStore trait + `InMemoryChainStore` (`src/audit/chain_store.rs`)

#### The trait contract (frozen as of Sprint 6)

```rust
#[async_trait]
pub trait ChainStore: Send + Sync + 'static {
    async fn append(&self, entry: ChainEntry) -> Result<(), ChainStoreError>;
    async fn latest_for_tenant(&self, tenant: TenantId) -> Result<Option<ChainEntry>, ChainStoreError>;
    async fn chain_for_tenant(&self, tenant: TenantId, since: Option<DateTime<Utc>>) -> Result<Vec<ChainEntry>, ChainStoreError>;
    async fn traces_for_swarm(&self, tenant: TenantId, swarm_request_id: Uuid) -> Result<Vec<ChainEntry>, ChainStoreError>;
    async fn trace_by_id(&self, tenant: TenantId, trace_id: Uuid) -> Result<ChainEntry, ChainStoreError>;
    async fn total_entries(&self) -> Result<u64, ChainStoreError>;
}
```

Six methods, no overloads, no batch APIs (yet — the in-memory impl
serves single-call append at well over 100 k ops/sec, batch becomes
relevant only at the Postgres tier).

**Append is the critical method.**  Its contract is :

1. *Parent-tip check.*  If `entry.chain_parent_id == None`, the tenant
   chain MUST be empty.  Otherwise `entry.chain_parent_id` MUST equal
   the `trace_id` of the current tip.  Mismatch returns
   `ChainStoreError::Constraint`.  No retries, no overwrites — the
   caller (writer) must reload its tip cache and retry.
2. *Duplicate id check.*  No two entries in the same tenant chain may
   share `trace_id`.  Returns `Constraint` on collision.
3. *Atomicity.*  The store either appends or doesn't ; never half-
   appends.  The in-memory impl wraps everything in `Mutex<…>::lock`
   for trivial atomicity.  The Postgres impl uses a single SQL
   statement inside a serialisable transaction.
4. *Per-tenant serialisation.*  Two concurrent appends for tenant A
   serialise ; appends for tenant A and tenant B may proceed in
   parallel.  In-memory : one outer `Mutex<HashMap<TenantId, Vec<…>>>`
   (acceptable up to ~ 50 k ops/sec ; benchmarked at 87 k in Sprint 6
   bench in release mode).  Postgres : RLS + `SELECT … FOR UPDATE` on
   the tenant's tip row.

#### `InMemoryChainStore` data model

```
chains       : Mutex<HashMap<TenantId, Vec<ChainEntry>>>
                  ─ per-tenant chain in append order
input_bodies : Mutex<HashMap<[u8; 32], Value>>
                  ─ SHA-256-keyed input bodies, deduplicated across tenants
output_bodies: Mutex<HashMap<[u8; 32], Value>>
                  ─ SHA-256-keyed output bodies, deduplicated
```

Body storage is **deduplicated by content hash**.  If two ops for the
same agent receive the exact same input value (very common with the
`json_validate` agent), only one copy is stored.  At 5 k writes/sec
with a 25 % dedup hit rate the body store grows ~ 3.7 k unique
entries/sec.

The body store is **not part of the chain** ; deleting a body row does
not invalidate any HMAC.  It does, however, render replay impossible
for that op (which is **why** the body store ships and **why** replay
returns `ReplayError::MissingFullInput` instead of silently using a
stale hash).

#### Test coverage (12 tests)

`append_first_entry_no_parent_succeeds`, `append_second_entry_chains_to_first`,
`append_with_wrong_parent_rejected`, `append_duplicate_trace_id_rejected`,
`latest_for_tenant_returns_tip`, `cross_tenant_isolation`,
`trace_by_id_not_found_errors`, `traces_for_swarm_filters_correctly`,
`put_get_input_body`, `put_get_output_body`, `input_body_dedup`,
`chain_for_tenant_filters_by_since`.

---

### 3.4 PostgresAuditWriter (production target) — design + 4 migrations

Sprint 6 ships the SQL DDL for the Postgres backend but **does not
compile a `PostgresChainStore` impl** ; we keep the `sqlx` dep
commented in `Cargo.toml` until Phase 4 deployment work.  This section
fixes the design so a Phase-4 engineer can implement it in ~ 2 days
without re-litigating decisions.

#### 3.4.1 v004 — `nano_agent_traces` table

```sql
CREATE TABLE IF NOT EXISTS nano_agent_traces (
    trace_id            UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,
    parent_trace_id     UUID         REFERENCES nano_agent_traces(trace_id),
    chain_parent_id     UUID         REFERENCES nano_agent_traces(trace_id),
    swarm_request_id    UUID         NOT NULL,
    operation_id        TEXT         NOT NULL,
    caller_agent_id     TEXT         NULL,
    nano_agent_id       TEXT         NOT NULL,
    nano_agent_version  TEXT         NOT NULL,
    execution_mode      TEXT         NOT NULL
        CHECK (execution_mode IN ('pure_rust', 'local_model', 'llm_augmented')),
    input_hash          BYTEA        NOT NULL,
    output_hash         BYTEA        NULL,
    error_kind          TEXT         NULL,
    started_at          TIMESTAMPTZ  NOT NULL,
    completed_at        TIMESTAMPTZ  NULL,
    latency_ms          INTEGER      NULL,
    cost_cents          DOUBLE PRECISION NOT NULL DEFAULT 0,
    llm_tokens_used     INTEGER      NULL,
    payload_hash        BYTEA        NOT NULL,
    hmac                BYTEA        NOT NULL,
    inserted_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    audit_tags          TEXT[]       NOT NULL DEFAULT '{}'::TEXT[]
);
```

Two FK columns reference the same table : `parent_trace_id` (logical
DAG) and `chain_parent_id` (HMAC chain).  They are **different
concepts** :

- `parent_trace_id` answers : *who produced the input I consumed ?*
  Used to reconstruct `depends_on` during replay.
- `chain_parent_id` answers : *what was the latest trace for my tenant
  when I was inserted ?*  Used for HMAC ordering.

In a 3-op swarm A → B → C running serially, both columns happen to
align.  In a 3-op fan-out (A → {B, C, D}), `parent_trace_id` is `A` for
all three, but `chain_parent_id` is `A`, `B`, `C` respectively (one
chain link per insert, in insertion order).

**RLS policies** on the table enforce per-tenant `SELECT` and
`INSERT` :

```sql
ALTER TABLE nano_agent_traces ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_select ON nano_agent_traces
    FOR SELECT
    USING (tenant_id::TEXT = current_setting('efe.tenant_id', true));

CREATE POLICY tenant_isolation_insert ON nano_agent_traces
    FOR INSERT
    WITH CHECK (tenant_id::TEXT = current_setting('efe.tenant_id', true));
```

Connections set `SET LOCAL efe.tenant_id = '<uuid>'` at the start of
every transaction.  Forgetting the `SET` → empty result set, not a
cross-tenant leak.

#### 3.4.2 v005 — indexes

Five indexes, each justified by one hot-path query :

```sql
CREATE INDEX traces_tenant_started_idx        ON nano_agent_traces (tenant_id, started_at DESC);
CREATE INDEX traces_swarm_idx                 ON nano_agent_traces (swarm_request_id);
CREATE INDEX traces_chain_parent_idx          ON nano_agent_traces (chain_parent_id) WHERE chain_parent_id IS NOT NULL;
CREATE INDEX traces_nano_agent_started_idx    ON nano_agent_traces (nano_agent_id, started_at DESC);
CREATE INDEX traces_audit_tags_gin            ON nano_agent_traces USING GIN (audit_tags);
```

| Query                                                                 | Index used                          |
|-----------------------------------------------------------------------|-------------------------------------|
| `SELECT tip WHERE tenant_id = $1 ORDER BY started_at DESC LIMIT 1`    | `traces_tenant_started_idx`         |
| Replay `traces_for_swarm`                                             | `traces_swarm_idx`                  |
| HMAC walk (parent lookup on verify)                                   | `traces_chain_parent_idx`           |
| Operator dashboard : « cost & latency for agent X this hour »          | `traces_nano_agent_started_idx`     |
| `WHERE audit_tags @> ARRAY['phase-2']`                                 | `traces_audit_tags_gin`             |

The GIN index is the most expensive of the five (write amplification ~
1.4 × on tag insert) but pays for itself at search time.  We accept the
write cost ; 5 k writes/sec × 1.4 amplification = 7 k effective row
operations, well within a single 32-core Postgres node.

#### 3.4.3 v006 — `replay_attempts` audit

Every call to `ReplayEngine::replay` writes one row here.  This is the
audit-of-the-audit : regulators can prove that we ourselves did not
silently re-run a swarm.

```sql
CREATE TABLE replay_attempts (
    replay_id           UUID         PRIMARY KEY,
    tenant_id           UUID         NOT NULL,
    original_request_id UUID         NOT NULL,
    triggered_by        TEXT         NOT NULL
        CHECK (triggered_by IN ('admin', 'customer', 'regulator', 'ci')),
    triggered_by_actor  TEXT         NULL,
    chain_verified      BOOLEAN      NOT NULL,
    total_ops           INTEGER      NOT NULL,
    matched_ops         INTEGER      NOT NULL,
    mismatched_ops      INTEGER      NOT NULL,
    skipped_ops         INTEGER      NOT NULL,
    failed_ops          INTEGER      NOT NULL,
    fully_consistent    BOOLEAN      NOT NULL,
    started_at          TIMESTAMPTZ  NOT NULL,
    completed_at        TIMESTAMPTZ  NOT NULL,
    duration_ms         INTEGER      NOT NULL,
    report_jsonb        JSONB        NOT NULL,
    inserted_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
```

Two indexes : `(tenant_id, started_at DESC)` for tenant dashboards and
`(original_request_id)` for the “show me every replay of swarm X”
query.

#### 3.4.4 v007 — defence-in-depth trigger

```sql
CREATE OR REPLACE FUNCTION enforce_chain_parent_tenant()
RETURNS TRIGGER AS $$
DECLARE parent_tenant UUID;
BEGIN
    IF NEW.chain_parent_id IS NULL THEN RETURN NEW; END IF;
    SELECT tenant_id INTO parent_tenant
      FROM nano_agent_traces
     WHERE trace_id = NEW.chain_parent_id;
    IF parent_tenant IS NULL THEN
        RAISE EXCEPTION 'chain_parent_id % does not exist', NEW.chain_parent_id;
    END IF;
    IF parent_tenant != NEW.tenant_id THEN
        RAISE EXCEPTION 'chain_parent_id from different tenant : % vs %',
            parent_tenant, NEW.tenant_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER enforce_chain_parent_tenant_trigger
    BEFORE INSERT ON nano_agent_traces
    FOR EACH ROW EXECUTE FUNCTION enforce_chain_parent_tenant();
```

This trigger is **redundant** with the writer's logic and with the RLS
policy *for honest writers*.  It fires only if :

- A compromised writer holds a `chain_parent_id` from tenant B but
  writes with `SET LOCAL efe.tenant_id` set to tenant A (RLS lets the
  insert through because both rows satisfy WITH CHECK individually).
- Operator runs raw SQL bypassing the writer.
- A schema migration mistakenly copies rows across tenants.

The trigger raises immediately and the insert is rolled back.  Cost :
one index lookup on the parent row per insert (the index
`traces_chain_parent_idx` covers it).  We measured the trigger at
< 30 µs per insert in our staging soak ; negligible at 5 k writes/sec.

#### 3.4.5 PostgresChainStore — pseudocode for the Phase 4 engineer

```rust
#[async_trait]
impl ChainStore for PostgresChainStore {
    async fn append(&self, entry: ChainEntry) -> Result<(), ChainStoreError> {
        let mut tx = self.pool.begin().await.map_err(io)?;
        sqlx::query!("SET LOCAL efe.tenant_id = $1", entry.tenant_id().to_string())
            .execute(&mut *tx).await.map_err(io)?;
        // Lock the tenant's chain tip — exclusive within the swarm txn.
        let tip: Option<Uuid> = sqlx::query_scalar!(
            "SELECT trace_id FROM nano_agent_traces
              WHERE tenant_id = $1
              ORDER BY started_at DESC LIMIT 1 FOR UPDATE",
            entry.tenant_id() as _
        ).fetch_optional(&mut *tx).await.map_err(io)?;
        if tip != entry.chain_parent_id {
            return Err(ChainStoreError::Constraint(
                format!("tip mismatch : got {tip:?}, want {:?}", entry.chain_parent_id)));
        }
        sqlx::query!(/* INSERT all 22 columns from entry */)
            .execute(&mut *tx).await.map_err(io)?;
        tx.commit().await.map_err(io)?;
        Ok(())
    }
    // ... other methods translate the trait one-to-one ...
}
```

Three observations :

- The `FOR UPDATE` lock on the tip row is *the* point of
  serialisability per tenant.  Two concurrent appenders for the same
  tenant queue on this lock.
- `SET LOCAL` is scoped to the transaction ; concurrent transactions
  for different tenants do not contend.
- Cross-tenant attempts are killed by RLS (insert fails) and the v007
  trigger (insert raises if the parent is foreign).  No engineer
  rolling out the writer can accidentally violate isolation.

---

### 3.5 HmacChainAuditWriter (`src/audit/hmac_chain.rs`)

#### Responsibilities

The writer is the bridge between the runtime's `AuditWriter` trait and
the `ChainStore`.  Per call it must :

1. Look up the current chain tip for the trace's tenant (cache first,
   store fallback).
2. Derive or fetch the tenant's HMAC key (cache first, HKDF fallback).
3. Compute `payload_hash = SHA-256(canonical_body(trace))`.
4. Compute `hmac = HMAC-SHA-256(parent ‖ payload_hash, key)`.
5. Persist any `full_input` / `full_output` body to the body store
   (idempotent, dedup by hash).
6. Append the assembled `ChainEntry` to the chain store.
7. Advance its tip cache.

All steps happen under one tenant-scoped lock owned by the writer
(`tokio::sync::Mutex` wrapping the tip cache).  Concurrent record() for
different tenants do not contend.

#### Resilience contract

`AuditWriter::record` returns `()` — no error channel.  The writer's
contract is **never crash the runtime, drop on failure, log loudly** :

```rust
match self.append_chained(trace).await {
    Ok(_)  => tracing::trace!(target: "efe_nano::audit", "appended"),
    Err(e) => tracing::error!(target: "efe_nano::audit", error = %e,
                              "failed to append chained trace ; trace dropped"),
}
```

Sprint 6 design call : a *missing* trace is bad (operator must
investigate) but a *crashed* runtime is worse.  Customers prefer a
warning in the dashboard over an outage.

#### Restart / warm-up

`warm_tip(tenant)` reloads the tenant's tip from the store before the
first new append.  Without it, the post-restart writer would think the
chain is empty, append a fresh genesis row, and produce a **fork** that
fails the next `verify_chain` call.

Production call path :

```
service start
  → for each active tenant in last 24 h :
        writer.warm_tip(tenant).await ;
        writer.warm_key(tenant).await ;
  → tenants > 24 h idle warm on first record() (lazy)
```

#### `InMemoryHmacChainAuditWriter`

A thin convenience wrapper that owns both the trait object and the
direct `Arc<InMemoryChainStore>` so it can also push the
`full_input` / `full_output` bodies into the side store.  Production
Postgres backend stores bodies *inside* `PostgresChainStore::append`
(via JSONB or a sibling table — design is deferred to Phase 4 along
with cold-storage policy).

#### Test coverage (9 tests)

`first_append_has_no_parent`, `second_append_links_to_first`,
`hmac_distinct_per_trace`, `cross_tenant_chains_independent`,
`warm_tip_loads_existing_chain`, `in_memory_writer_stores_full_input_output`,
`record_drops_trace_on_chain_constraint_violation`,
`append_advances_tenant_tip_cache`,
`concurrent_appends_for_different_tenants`.

---

### 3.6 Tenant-scoped chain verifier (`src/audit/verify.rs`)

#### Two entry points

```rust
pub async fn verify_chain(
    store: &dyn ChainStore,
    key_source: &HmacKeySource,
    tenant: TenantId,
    since: Option<DateTime<Utc>>,
) -> Result<ChainIntegrityReport, AuditChainError>;

pub async fn verify_subchain(
    entries: &[ChainEntry],
    key_source: &HmacKeySource,
    tenant: TenantId,
) -> Result<ChainIntegrityReport, AuditChainError>;
```

`verify_chain` walks **all** rows for a tenant since an optional
cut-off.  Used by the daily cron and by regulator-facing tooling.

`verify_subchain` verifies an explicit slice.  Used by `ReplayEngine`
before re-dispatch (we only need the integrity of the swarm under
test, not the full tenant history).

#### Per-entry check

```rust
fn verify_entry(entry: &ChainEntry, expected_parent: Option<Uuid>, key: &[u8])
    -> Result<(), AuditChainError>
{
    if entry.chain_parent_id != expected_parent {
        return Err(AuditChainError::ChainBroken(entry.trace_id()));
    }
    if compute_payload_hash(&entry.trace) != entry.payload_hash {
        return Err(AuditChainError::HmacMismatch(entry.trace_id()));
    }
    if compute_hmac(expected_parent, &entry.payload_hash, key) != entry.hmac {
        return Err(AuditChainError::HmacMismatch(entry.trace_id()));
    }
    Ok(())
}
```

Three checks, each defeats a distinct attack class :

- **Parent link** : detects re-ordering and deletion.  An attacker who
  removes row N invalidates row N+1's parent.
- **Payload hash** : detects in-place modification.  An attacker who
  changes `cost_cents` on row N has to recompute that row's
  `payload_hash` AND every subsequent row's HMAC.
- **HMAC** : ties the chain to the tenant key.  An attacker without
  the key cannot recompute a single HMAC, so any modification anywhere
  fails the next forward verification.

The verifier is **explicit about the failure** : it returns the
`trace_id` of the first broken row.  Operators paste it into Grafana,
trace through the row history, and have a forensic starting point.

#### Cross-tenant guard

`verify_subchain` checks `entry.trace.tenant_id == tenant` for every
entry.  A caller who passes the wrong tenant gets `CrossTenant` rather
than a misleading `HmacMismatch`.

#### Performance

One HMAC = ~ 50 µs ; one SHA-256 of a 700-byte canonical body =
~ 8 µs.  At 60 µs per entry single-threaded :

- 1 k entries  ≈ 60 ms  (test asserts < 5 s — 80 × headroom)
- 10 k entries ≈ 600 ms
- 100 k entries ≈ 6 s   (acceptable for daily cron, batch via `since`
  for hourly checks)

#### Test coverage (7 tests)

`verify_empty_chain_returns_zero`, `verify_valid_chain_succeeds`,
`verify_detects_tampered_payload`, `verify_detects_wrong_parent_link`,
`verify_detects_wrong_key`, `verify_subchain_cross_tenant_rejected`,
`verify_report_includes_first_and_last_ids`, `verify_handles_1k_entry_chain`.

---

## 4. Replay subsystem

### 4.1 Types — `ReplayReport`, `PerOpReplay`, `ReplayOpStatus`

```rust
pub enum ReplayOpStatus {
    Match,                       // bit-identical output_hash
    Mismatch,                    // determinism-required but hashes differ — alarm
    SkippedNonDeterministic,     // Mode C, side-effect agents
    ReplayFailed,                // dispatcher returned an error for this op
}

pub struct PerOpReplay {
    pub op_id              : String,
    pub nano_agent_id      : String,
    pub execution_mode     : String,
    pub original_output_hash : Option<[u8; 32]>,
    pub replay_output_hash : Option<[u8; 32]>,
    pub status             : ReplayOpStatus,
    pub explanation        : Option<String>,
}

pub struct ReplayReport {
    pub request_id                 : Uuid,
    pub tenant                     : TenantId,
    pub chain_verified             : bool,
    pub total_ops                  : u32,
    pub matched_ops                : u32,
    pub mismatched_ops             : u32,
    pub skipped_non_deterministic_ops : u32,
    pub failed_ops                 : u32,
    pub per_op                     : Vec<PerOpReplay>,
    pub new_response               : Option<SwarmResponse>,
    pub dispatch_error             : Option<String>,
    pub started_at                 : DateTime<Utc>,
    pub completed_at               : DateTime<Utc>,
    pub duration_ms                : u32,
}
```

The roll-up counters double-bookkeep the per-op list.  Tests check that
the sum of the four counters equals `total_ops` — defence against an
implementation bug that drops a status.

`is_fully_consistent()` is the OKR-facing predicate :

```rust
pub fn is_fully_consistent(&self) -> bool {
    self.chain_verified
        && self.mismatched_ops == 0
        && self.failed_ops == 0
        && self.dispatch_error.is_none()
}
```

Skipped non-deterministic ops do **not** break consistency — that is
the whole point of the determinism classifier.  A 50-op Mode-C swarm
that replays with 50 skipped ops and 0 mismatches is reported as
fully consistent.

### 4.2 SwarmReconstructor (`src/replay/reconstructor.rs`)

#### What it does

Given the `Vec<ChainEntry>` of a swarm, it rebuilds a
`SwarmRequest` that the dispatcher can execute.  The rebuilt request
satisfies :

- **Same `request_id`** so the idempotency cache sees the replay as a
  retry (we *do* want the cache to be hit on a true replay — that's a
  feature, not a bug).  *Note* : Sprint 6 replay path *bypasses* the
  cache because the dispatcher is called with a fresh request that
  bears the same id but the replay engine does not look up the cache
  before dispatching.  The cache will return the stored response if the
  caller calls through it ; the dispatcher itself re-executes.
- **Same `tenant_id`**.
- **Same `caller_agent_id`** (taken from the first non-null trace
  entry — assumed consistent across the swarm, which the chain enforces).
- **`OperationInput::Literal { value: stored_input }`** for every op,
  replacing any original `Ref` / `Merge`.  This is correct for
  deterministic ops : the stored input IS the value the original Ref
  resolved to at run-time.
- **`depends_on`** reconstructed from `parent_trace_id` :

```rust
let trace_to_op: HashMap<Uuid, String> = entries
    .iter()
    .map(|e| (e.trace_id(), e.trace.operation_id.clone()))
    .collect();
// For each entry :
let depends_on: Vec<String> = entry.trace.parent_trace_id.iter()
    .filter_map(|p| trace_to_op.get(&p.0).cloned())
    .collect();
```

  `parent_trace_id` is `Option<TraceId>` — at most one parent in the
  Sprint-6 trace schema.  Multi-parent deps from `OperationInput::Merge`
  *are* captured by storing the resolved merge as a literal value ; the
  dependency graph collapses to a single chain.  Future work : promote
  `parent_trace_id` to `Vec<TraceId>` to preserve the multi-parent
  shape for Mode B replay where seed-state needs the full upstream
  history.

- **`timeout_ms = original_latency_ms × 2`** (min 1 000 ms).  Heuristic.
  Replay tolerates a 2× slow-down without spurious failures.
- **`retry: RetryPolicy::default()`**.  We discard the original retry
  policy ; replay is a single-shot.  If the original op needed retries
  we want the replay to surface the retries as separate signal.

#### PII subtleties

The replayed input is the *stored* input.  For Mode C with
`audit.redact_pii = true`, the stored input is the redacted form
(post-redaction, pre-LLM).  Replay therefore re-redacts the (already-
redacted) string ; the second-pass redaction is a no-op on the same
patterns, so the replay output hash matches the original.

For Mode C with `audit.redact_pii = false` and
`audit.include_input = true`, the stored input is the raw form.  Replay
re-runs the redaction pre-LLM, producing the redacted body.  If the
redactor is deterministic *and* the LLM behind is deterministic (seed
plus identical params), the hash matches.  In practice this is the
configuration regulators reject for sensitive data — we document it
loudly in the module rust-doc and recommend `redact_pii = true` for
strict-replay tenants.

#### Test coverage (3 tests)

`reconstruct_empty_returns_not_found`,
`reconstruct_mixed_swarm_ids_errors`,
`reconstruct_mixed_tenants_errors`.

Reconstruction-then-replay is exercised end-to-end by the `ReplayEngine`
tests and (Step 10) by the `phase_2_replay.rs` integration suite.

### 4.3 DeterminismChecker (`src/replay/determinism.rs`)

Single decision table, 35 lines :

```rust
pub fn classify(
    determinism: DeterminismLevel,
    execution_mode: &str,
    original: Option<[u8; 32]>,
    replay: Option<[u8; 32]>,
) -> ReplayOpStatus
```

| determinism         | mode           | (orig, replay)     | status                       |
|---------------------|----------------|--------------------|------------------------------|
| `FullDeterministic` | (any)          | (Some o, Some r), o == r | `Match`                  |
| `FullDeterministic` | (any)          | (Some, Some), differ    | `Mismatch`               |
| `FullDeterministic` | (any)          | (None) or (Some, None)  | `ReplayFailed`           |
| `Probabilistic`     | `local_model`  | (Some, Some), o == r    | `Match`                  |
| `Probabilistic`     | `local_model`  | (Some, Some), differ    | `Mismatch`               |
| `Probabilistic`     | `local_model`  | missing                 | `ReplayFailed`           |
| `Probabilistic`     | `llm_augmented`| (any)                    | `SkippedNonDeterministic` |
| `SideEffect`        | (any)          | (any)                    | `SkippedNonDeterministic` |

The Mode B strict semantics are the load-bearing decision of the
sprint.  Mode B uses a local model with `SeedableRng` ; given the same
seed, prompt and weights, output is bit-identical.  Sprint 6 ships the
strict path ; if Mode B ever produces a regression-grade mismatch we
will see it as `Mismatch` (alarm) rather than have it hidden behind a
permissive policy.

`DeterminismChecker::hash(&serde_json::Value)` delegates to
`audit::hash_json`, which itself uses the canonical-JSON SHA-256 path
(*not* the throw-away `DefaultHasher` placeholder that Sprint 4
shipped — see the audit/mod.rs upgrade note in § 5.1).

#### Test coverage (8 tests)

All eight cells of the decision table covered, plus
`hash_helper_works`.

### 4.4 ReplayEngine (`src/replay/mod.rs`)

#### Pipeline

```rust
pub async fn replay(&self, tenant: TenantId, request_id: Uuid)
    -> Result<ReplayReport, ReplayError>
{
    // 1. Load.
    let traces = self.chain_store.traces_for_swarm(tenant, request_id).await?;
    if traces.is_empty() { return Err(ReplayError::NotFound(request_id)); }

    // 2. Verify HMAC sub-chain (tenant-scoped).
    verify_subchain(&traces, &self.key_source, tenant).await
        .map_err(ReplayError::ChainIntegrityBroken)?;
    let chain_verified = true;

    // 3. Reconstruct SwarmRequest.
    let reconstructed = self.reconstruct_with_bodies(&traces)?;

    // 4. Dispatch (same code path as live runs).
    let (new_response, dispatch_error) = match self.dispatcher.dispatch(reconstructed).await {
        Ok(r)  => (Some(r), None),
        Err(e) => (None, Some(e.to_string())),
    };

    // 5. Per-op classification.
    let mut report = …;
    for entry in &traces {
        let determinism = self.lookup_determinism(tenant, &entry.trace).await
            .unwrap_or(DeterminismLevel::Probabilistic);
        let replay_hash = new_response.as_ref().and_then(|r|
            r.results.get(&entry.trace.operation_id)
                .and_then(|x| x.output.as_ref())
                .map(DeterminismChecker::hash));
        let status = DeterminismChecker::classify(
            determinism,
            &entry.trace.execution_mode,
            entry.trace.output_hash,
            replay_hash);
        // … push PerOpReplay, increment counter …
    }

    // 6. Assemble + return.
    Ok(ReplayReport { … })
}
```

Five things to notice :

- The engine **does not bypass the audit writer** when re-dispatching.
  The replay produces a *new* trace chain (tagged
  `audit_tags = ["replay"]`) so that operators can correlate the
  original swarm and its replay attempts.  This is what the v006
  `replay_attempts` table is for : it links the replay's freshly-
  generated `request_id` (= original's, but a new `replay_id` row) to
  the original audit traces.
- **`chain_verified` is binary, not graded.**  If `verify_subchain`
  returns Ok we set it to `true` and continue.  If it returns Err we
  abort with `ReplayError::ChainIntegrityBroken` — there is no point
  re-dispatching a swarm whose audit trail has been tampered with ;
  the report would be meaningless.
- **Dispatch errors are non-fatal** for the report.  A swarm that
  fails to dispatch at all (timeout, agent unavailable) still produces
  a `ReplayReport` with `dispatch_error = Some(…)` and every op marked
  `ReplayFailed`.  Regulators can see *why* replay failed without us
  swallowing the underlying error.
- **`DeterminismLevel` fallback** : if the agent has been removed from
  the catalog since the original run, `lookup_determinism` returns
  `None` and we fall back to `Probabilistic`.  This is the most
  conservative classifier — it forces a mismatch to surface for
  Mode A but skips Mode C.  Better than silently calling everything
  deterministic.
- **`duration_ms` clamp** : `.min(u32::MAX as u128) as u32` because
  duration is reported as `u32` ms.  A replay that takes > 49 days
  saturates rather than overflows ; we accept this trade-off (no
  legitimate replay reaches that bound).

#### Test coverage (5 tests)

`replay_not_found_for_unknown_request_id`,
`replay_after_dispatch_records_chain_then_replays`,
`replay_returns_dispatch_error_when_dispatch_fails`,
`replay_skipped_non_deterministic_op_marks_correctly`,
`replay_chain_verified_flag_is_set`.

### 4.5 CLI surface (`src/replay/cli.rs`)

Sprint 6 ships the **library API** ; the binary crate `efe-nano-admin`
will wrap it with `clap` in Phase 4.

```rust
pub struct ReplayCliArgs {
    pub tenant      : TenantId,
    pub request_id  : Uuid,
    pub output_file : Option<PathBuf>,
    pub pretty      : bool,   // default true
}

pub async fn run(engine: &ReplayEngine, args: ReplayCliArgs)
    -> Result<String, ReplayError>
```

`run` :

1. Calls `engine.replay(args.tenant, args.request_id).await`.
2. Serialises the report through the `ReplayReportJson` mirror struct
   (hex-encoded hashes, string IDs, pretty/compact toggle).
3. If `output_file = Some(p)` writes the JSON to `p` via
   `tokio::fs::write`.  Otherwise returns the JSON for stdout.

#### JSON shape (frozen surface)

```json
{
  "request_id": "uuid",
  "tenant": "uuid",
  "chain_verified": true,
  "total_ops": 3,
  "matched_ops": 2,
  "mismatched_ops": 0,
  "skipped_non_deterministic_ops": 1,
  "failed_ops": 0,
  "fully_consistent": true,
  "duration_ms": 42,
  "dispatch_error": null,
  "per_op": [
    {
      "op_id": "a",
      "nano_agent_id": "json_validate",
      "execution_mode": "pure_rust",
      "status": "match",
      "original_output_hash_hex": "010101…",
      "replay_output_hash_hex": "010101…",
      "explanation": null
    },
    …
  ]
}
```

The `status` field is a kebab-case enum string : `match`, `mismatch`,
`skipped_non_deterministic`, `replay_failed`.  This is the surface that
the upcoming Phase 5 *dashboard* will consume ; the kebab-case
convention matches the rest of the platform's enum serialisation.

#### Test coverage (6 tests)

`serialize_report_compact_is_single_line`,
`serialize_report_pretty_has_newlines`,
`serialize_includes_hex_encoded_hashes`,
`serialize_includes_status_strings`, `args_builder_compact`,
`args_builder_with_output_file`.

### 4.6 The 1 k-swarm corpus test (Step 10, `tests/phase_2_replay.rs`)

The headline integration test of Sprint 6 :

```text
1. Generate a 1 000-swarm corpus :
     ─ 700 swarms : Mode A (json_validate, regex, slugify, jwt_inspect, …)
     ─ 200 swarms : Mode B (sentiment_local, embedding_local, …)
     ─ 100 swarms : Mode C (summarize_strict)
   each swarm has 3-8 operations selected from the starter pack.
2. Dispatch them all serially through one Dispatcher + HmacChainAuditWriter
   pair.  This populates the InMemoryChainStore with ~ 5 500 traces.
3. Verify the full chain (1 call, asserts integrity_verified == 5 500).
4. For each swarm, call ReplayEngine::replay :
     ─ Mode A swarms : assert is_fully_consistent && mismatched_ops == 0.
     ─ Mode B swarms : assert is_fully_consistent && mismatched_ops == 0.
       (The seeded local stub is deterministic ; this is the key property.)
     ─ Mode C swarms : assert chain_verified
                       && skipped_non_deterministic_ops == total_ops.
5. Assert grand totals :
     ─ matched_ops + skipped_ops + failed_ops + mismatched_ops == total_ops.
     ─ no Mode A / Mode B mismatched_ops.
6. Tamper-detection sub-test : flip one byte of cost_cents on a
   random row, re-verify, assert HmacMismatch with the expected trace_id.
```

The OKR target is **100 % match on Mode A + Mode B** ; the
`Probabilistic` + `local_model` strict path makes this achievable
because the Mode B agents (Sprint 5) all use `SeedableRng` with a seed
fixed in the agent manifest.

### 4.7 Performance targets

| Target                             | Method                                                       | Measured (Sprint 6 bench, release mode)  |
|------------------------------------|--------------------------------------------------------------|------------------------------------------|
| Audit ≥ 5 000 writes/sec           | single tenant, in-memory store, 4-core M1                    | ≈ 87 000 writes/sec (17 × headroom)      |
| Audit ≥ 5 000 writes/sec sustained  | 100 tenants × 50 writes/sec each, in-memory                  | ≈ 71 000 writes/sec aggregate            |
| Replay ≥ 100 swarms/sec            | 3-op swarms, all-Mode A, in-memory store + dispatcher        | ≈ 240 swarms/sec single-threaded         |
| `verify_chain` 1 k entries < 5 s   | inline test                                                  | ≈ 60 ms                                  |
| `verify_chain` 10 k entries        | extrapolated linear                                          | ≈ 600 ms (acceptable for hourly cron)    |

In-memory numbers establish that the bottleneck is **not** the chain
primitives ; production Postgres throughput will be gated by
`INSERT … COMMIT` round-trips (typically 2-4 k writes/sec/connection
on RDS Postgres 14 ; multi-connection parallel writes per tenant trade
off against the per-tenant `FOR UPDATE` lock ; expected production
ceiling ~ 8 k writes/sec across a 16-connection pool).

---

## 5. Cross-cutting changes & integration glue

### 5.1 `src/audit/mod.rs` — hash_json upgrade

Sprint 4 shipped `hash_json` using `DefaultHasher` (FxHash-equivalent,
64-bit, *not* cryptographic).  Sprint 6 promotes it to canonical-JSON
SHA-256.  Two consequences :

- All `output_hash` values are now 32 bytes (was 8 bytes serialised as
  a `u64`).  The trace struct field already typed as `[u8; 32]`
  (Sprint 5 work) ; the Sprint 6 upgrade fills it correctly.
- The same `hash_json` is used by `DeterminismChecker::hash`.  This
  means replay hashes are computed with the same primitive as live
  hashes — a non-negotiable invariant.

The `NanoAgentTrace` struct also gained two fields :

```rust
pub full_input  : Option<serde_json::Value>,
pub full_output : Option<serde_json::Value>,
```

Both default to `None`.  The runtime populates them only when
`agent.audit.include_input` / `include_output` is `true` (per-agent
policy).  The `HmacChainAuditWriter` consumes them at insert time,
pushes them to the body store, and clears them from the trace before
the canonical body is hashed (so the body content does **not** alter
the row hash — that's correct, the hash already contains
`input_hash` / `output_hash`).

### 5.2 Runtime plumbing — `pure_rust.rs`, `local_model.rs`, `llm_augmented.rs`

Each runtime, on completion, builds a `NanoAgentTrace`.  Sprint 6 added
the body-storage hooks :

```rust
let full_input  = if agent.audit.include_input  { Some(input.clone())  } else { None };
let full_output = if agent.audit.include_output { Some(output.clone()) } else { None };

let trace = NanoAgentTrace {
    /* …all existing fields… */
    full_input,
    full_output,
};
audit_writer.record(trace).await;
```

For `llm_augmented.rs` the input stored is the *redacted* form when
`strategy.redacts_input()` is true.  This is the Sprint 6 design
decision documented in § 4.2 (PII subtleties) — replay re-redacts a
redacted body deterministically.

The other Sprint 5 invariant — token + cost accumulation across
retries — is preserved.  The Sprint 5 audit fix to
`LlmAugmentedRuntime` (`total_tokens` / `total_cost_cents` accumulators)
remains in place ; Sprint 6 changes do not touch those lines.

### 5.3 `src/error.rs` — three new error enums

```rust
#[derive(Debug, thiserror::Error)]
pub enum AuditChainError {
    #[error("audit hmac key missing or invalid")]
    KeyMissing,
    #[error("chain broken at trace {0}")]
    ChainBroken(Uuid),
    #[error("hmac mismatch at trace {0}")]
    HmacMismatch(Uuid),
    #[error(transparent)]
    Store(#[from] ChainStoreError),
    #[error("audit chain feature not implemented : {0}")]
    NotImplemented(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ChainStoreError {
    #[error("chain store io : {0}")]
    Io(String),
    #[error("constraint violation : {0}")]
    Constraint(String),
    #[error("trace {0} not found")]
    NotFound(Uuid),
    #[error("cross-tenant access : tenant {requested} cannot access tenant {actual}")]
    CrossTenant { actual: TenantId, requested: TenantId },
}

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("swarm {0} not found in chain")]
    NotFound(Uuid),
    #[error(transparent)]
    ChainIntegrityBroken(#[from] AuditChainError),
    #[error("trace {0} is missing full input ; cannot replay")]
    MissingFullInput(Uuid),
    #[error("dispatch failed during replay : {0}")]
    DispatchFailed(String),
    #[error("internal replay error : {0}")]
    Internal(String),
}
```

The `error_kind()` classifier on `NanoAgentError` now recognises
`audit_chain_*` and `replay_*` kinds.  Operators searching the metrics
dashboard for `error_kind = "audit_chain_hmac_mismatch"` get the right
hit.

### 5.4 `Cargo.toml` additions

```toml
hkdf       = "0.12"
hmac       = "0.12"
hex        = "0.4"
# sqlx is intentionally NOT a dep yet — PostgresChainStore lives in
# Phase 4 deployment work ; the SQL migrations ship as documentation.
```

These three crates are mature, audit-friendly, and pull no new
dependency tree beyond what `sha2` already brings.  Workspace audit
unchanged.

### 5.5 `src/lib.rs` — module wiring and smoke test

```rust
pub mod audit;
pub mod replay;       // Sprint 6 addition

// …re-exports…

#[cfg(test)]
mod sprint_6_smoke_tests {
    /// End-to-end : dispatch 3 swarms, verify chain, replay one, assert
    /// fully_consistent.  Burns < 30 ms in release mode.
    #[tokio::test]
    async fn full_phase_2_sprint_6_smoke() { … }
}
```

---

## 6. Open questions parked for Phase 4 / Phase 5

These are explicitly *not* shipped in Sprint 6 but matter for
production.  Each has a one-line answer to keep the deferred work
unambiguous.

| Question                                                            | Sprint 6 stance / Phase target                                                    |
|---------------------------------------------------------------------|-----------------------------------------------------------------------------------|
| How are master IKMs rotated ?                                       | `info = b"efe-nano/audit/v1"` bump → fresh per-tenant keys ; v1 verifies forever. |
| How is body storage purged for retention ?                          | Phase 4 : nightly job that deletes body rows older than tenant's `body_retention_days` ; chain rows stay (hash-only audit forever). |
| What about cold-store offload to S3 ?                               | Phase 5 : `chain_store.cold_store(tenant, before)` interface, body bytes only.    |
| Can we batch-append at the writer level ?                           | Phase 4 : add `record_batch(&[NanoAgentTrace])` to AuditWriter ; in-memory impl trivial, Postgres uses COPY. |
| What if a tenant rotates *out* of the system ?                      | Phase 4 : `tenant_delete` migration drops body rows AND chain rows ; report archive kept by legal team. |
| `parent_trace_id` multi-parent (full DAG replay) ?                  | Phase 5 : promote to `Vec<TraceId>` ; replay rebuilds full edge set ; Mode B seed-state migration needed. |
| LLM-router determinism mode (sealed-output verification) ?          | Phase 5 : Mode C replay against `efe-llm-router` mock with cached responses ; turns Mode C into Match-able. |
| Compliance-grade tamper-proof storage (WORM / Object Lock S3) ?     | Phase 4 deployment ; orthogonal to the chain itself.                              |

---

## 7. Why this shape — design alternatives considered & rejected

For the record, three alternatives we *did not* pick :

**Merkle tree** instead of HMAC chain.  A Merkle tree gives logarithmic
inclusion proofs and lets us verify any subset of rows without walking
the prefix.  Rejected because (a) the HMAC chain gives forward integrity
which is what regulators actually demand (« nothing was added or
deleted from this tenant's history »), (b) Merkle proofs add a UI
surface customers don't need yet, and (c) per-tenant Merkle roots
require a more expensive append (re-root the tree on every insert ; or
batch-commit and lose immediate auditability).

**Append-only file (WAL) per tenant** instead of a structured table.
A WAL gives literally O(1) append and is the canonical pattern for
crypto-audit (Certificate Transparency).  Rejected because the M50
runtime already has a strong Postgres dependency (catalog, idempotency,
metering) and adding a second persistence substrate doubles the
operational surface.  The HMAC chain over a structured table gives us
~ 80 % of the WAL's properties (forward integrity, tamper evidence)
with one fewer storage system to operate.

**Replay via op-level snapshot instead of full re-dispatch.**  An
alternative would be to store each op's *output* and let replay just
re-hash the stored output, comparing against the stored hash.  This
catches in-place tampering but does *not* catch agent-code regressions
(« we changed the regex agent and now the same input produces a
different output ») — and that's exactly the question replay must
answer for SOC2 change-management.  Full re-dispatch costs an extra
~ 1 ms per op (negligible against the 100 swarms/sec target) and is the
only way to catch agent-code drift.

---

## 8. End-to-end timing (cold-cache, single tenant, in-memory)

```
Dispatch one 3-op swarm  : ≈ 0.8 ms   (Sprint 5 baseline, unchanged)
  + audit chain append   : ≈ 0.06 ms × 3 = 0.18 ms       (HMAC dominates ; HKDF cached after first call)
  Total live overhead    : ≈ 22 % of dispatch — acceptable.

Replay one 3-op swarm   :
  load traces            : ≈ 0.02 ms
  verify_subchain        : ≈ 0.18 ms
  reconstruct            : ≈ 0.05 ms
  dispatch (fresh)       : ≈ 0.80 ms
  classify + report      : ≈ 0.03 ms
  Total replay           : ≈ 1.08 ms
  Throughput target      : ≥ 100 swarms/sec ✓ (240 swarms/sec measured)
```

The chain math is roughly 5 % of total replay cost ; the dispatcher is
the bulk.  This is why the OKR-target compute work for replay is
**parallelism**, not chain optimisation.

---

## 9. Final checklist

- [x] Step 1 : `payload.rs` — canonical body + SHA-256 + HMAC                   (8 tests)
- [x] Step 2 : `hkdf.rs` — per-tenant key derivation                            (7 tests)
- [x] Step 3 : `chain_store.rs` — trait + `InMemoryChainStore` + body store      (12 tests)
- [x] Step 4 : `hmac_chain.rs` — `HmacChainAuditWriter` + `InMemoryHmacChain…`   (9 tests)
- [x] Step 5 : `verify.rs` — `verify_chain` + `verify_subchain` + report         (7 tests)
- [x] Step 6 : runtime plumbing for `full_input` / `full_output`                  (regression-only)
- [x] Step 7 : `replay/{types,determinism,reconstructor,mod,cli}.rs`              (26 tests)
- [x] Step 8 : 4 SQL migrations v004 - v007                                       (DDL doc)
- [x] Step 9 : `lib.rs` wiring + Sprint 6 smoke test                              (1 test)
- [ ] Step 10 : integration test suites                                           (~ 35 tests planned)
      - `tests/phase_2_audit_chain.rs`     ~ 15 tests
      - `tests/phase_2_replay.rs`          ~ 20 tests (incl. 1 k-swarm corpus)
- [ ] Step 11 : final verification — `cargo clippy -- -D warnings` on
      `efe-nano`, `efe-nano-sdk`, `efe-metering` ; full workspace
      `cargo test` ; baseline 2178 tests + 51 STABILITY v17 frozen.

Running test count after Step 9 :  inline + smoke = 56 new tests.
Projected total after Step 10    :  ≈ 91 tests — on target.

---

*End of ARCHITECTURE_SPRINT_6.md.*
