# EFE M50 — N24 Wired Compliance Runner Architecture

> **Sprint**          : 16 (Phase 8, day 76 – 80 of M50).
> **Status**          : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md`           (Sprint 10, shipped)
> * `EFE_M50_N19_GOLD_TEST_GATE_TIER_1_ARCHITECTURE.md`        (Sprint 11, shipped)
> * `EFE_M50_N21_SWARM_ORCHESTRATION_AGENTIC_BRIDGE_ARCHITECTURE.md` (Sprint 13, shipped)
> * `EFE_M50_N22_HTTP_GATEWAY_EXPOSURE_ARCHITECTURE.md`        (Sprint 14, shipped)
> * `EFE_M50_N23_NANO_SWARM_ADAPTER_ARCHITECTURE.md`           (Sprint 15, shipped)
>
> **Anchors** : N23 shipped the `NanoToSwarm` adapter with two
> placeholder traits — `SignatureGate` and `GoldBindingGate` —
> both `Option<Arc<dyn ...>>` and both NULL in the gateway's
> default policy.  The catalogue dispatch path therefore runs
> with **no signature verification**, **no gold-binding
> enforcement**, and the N23 architecture explicitly tagged this
> as "Sprint 16 will wire the real implementations" (cf. N23
> § 14, dag_registry.rs comment).
>
> **N24 closes that gap.**  It ships `WiredSignatureGate` (wraps
> the real `Ed25519Verifier` + `PublisherRegistry` from N18) and
> `WiredGoldBindingGate` (wraps the real `GoldApi` from N19),
> bundled by a `WiredComplianceRunner` factory that the gateway
> wires into `AppState`.  The default DAG flips from
> `experimental_fast` to `tier1_strict` ; every pre-analysis
> dispatch is now signed, gold-bound, and audit-traced
> end-to-end per the EU compliance officer's contract.

---

## Table of contents

0.  [TL;DR](#0-tldr)
1.  [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2.  [Twelve incident patterns N24 closes (P1–P12)](#2-twelve-incident-patterns-n24-closes-p1p12)
3.  [Commitments — what ships in Sprint 16 (G1–G15)](#3-commitments--what-ships-in-sprint-16-g1g15)
4.  [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5.  [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6.  [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7.  [Component architecture](#7-component-architecture)
8.  [Module hierarchy & data model](#8-module-hierarchy--data-model)
9.  [API surface (Rust traits + HTTP)](#9-api-surface-rust-traits--http)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [Cross-cutting alignment (N11 → N23)](#13-cross-cutting-alignment-n11--n23)
14. [Sprint 16 ship vs Sprint 17+ split](#14-sprint-16-ship-vs-sprint-17-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The two NULL gates that N23 explicitly deferred.**

The N23 architecture (§ 14) said :

> *Default policy is `experimental_fast` (no compliance gates) —
> Sprint 16 N24 wires `tier1_strict` once the gateway carries the
> SignatureGate + GoldBindingGate instances.*

That deferral is on the line.  In the current shipped code, every
swarm DAG dispatch flows through `NanoToSwarm` with
`policy.verify_signature == false` and
`policy.require_gold_binding == false`.  Compliance auditors who
trace a `POST /v1/swarms/run` see the `nano.envelope.v1` event
with `signature_verified: false` and `gold_bound: false`.  Sprint
15 shipped the **machinery** (gates + policy + envelope) ;
Sprint 16 N24 ships the **wiring** that turns the machinery on.

### The five components N24 ships

1. **`WiredSignatureGate`** — concrete `SignatureGate` impl
   bound to a `(Arc<dyn PublisherRegistry>, Arc<dyn
   SignatureVerifier>)` pair.  Reconstructs the
   `CanonicalBody` from the `NanoAgent` at verification time,
   looks up the publisher's active or historical key by
   `(publisher_id, signature.key_id, signature.signed_at)`,
   and calls `verifier.verify(signature, canonical_bytes,
   public_key_pem)`.

2. **`WiredGoldBindingGate`** — concrete `GoldBindingGate`
   impl bound to `Arc<dyn GoldApi>`.  Looks up the catalogue
   entry's `gold_set_ref.gold_set_id`, asserts
   `status == Approved`, and verifies the stored hash matches
   the entry's `gold_set_ref.gold_set_hash` (defense against
   silent gold-set rewrites — N19 invariant F2).

3. **`PublisherStatusGate`** — third gate that surfaces the
   N18 "publisher status" check (suspended / revoked) before
   dispatch.  Not in the N23 trait set ; N24 introduces a
   third `Option<Arc<dyn PublisherStatusGate>>` field in
   `NanoToSwarmPolicy` and adds the F12-style fail-closed
   wiring around it.

4. **`WiredComplianceRunner`** — factory + facade.  Takes
   `(PublisherRegistry, SignatureVerifier, GoldApi)`
   handles and produces :
   - the three gates (`WiredSignatureGate`,
     `WiredGoldBindingGate`, `PublisherStatusGate`) wrapped as
     `Arc<dyn ...>` ;
   - a pre-configured `NanoToSwarmPolicy::tier1_strict_wired(
     ...)` with all three gates installed ;
   - an `experimental_relaxed` policy for tier-experimental
     agents that opts out of gold binding but keeps signature
     verification (the architecture-correct gradient).

5. **Gateway wiring** — `AppState` gains
   `compliance: Arc<WiredComplianceRunner>` ; the
   `dag_registry::build_default_dag()` consumes it and selects
   the per-agent policy from a small `select_policy_for_tier()`
   helper.  Tier-1 + Tier-Audit agents get strict ; Tier-
   Experimental gets relaxed.

### Six reference outcomes ship in Sprint 16 N24

1. Every pre-analysis dispatch through the default DAG carries
   `nano.envelope.v1.signature_verified == true` (because the
   six built-in catalogue entries get the built-in EFE
   publisher signature at build-time — see G9).
2. `pii_redactor` (the `TierAudit` agent) carries
   `gold_bound == true` from the moment a gold set is approved
   for it ; a revoked gold set immediately blocks dispatch.
3. A tenant who publishes a custom nano gets the **same audit
   posture** as built-ins (G10).
4. The N16 multi-region replay (Sprint 9) can verify the
   signature attached to a 90-day-old dispatch using the
   historical key (I7).
5. The N18 `revoke` cascade now blocks **swarm-side dispatch**
   in addition to the publish-side block — Sprint 10's
   `RevocationCascade` was wired into the publisher API but
   never reached the swarm runtime (G11).
6. The `GET /v1/admin/compliance-status` endpoint returns a
   live snapshot of the wired stack : how many publishers are
   registered, how many keys active vs historical, how many
   gold sets approved per agent, how many dispatches per gate
   in the last hour (G13).

### What this NOT changes

- **No new public traits in `efe-nano-swarm-adapter`.**  The
  `SignatureGate` and `GoldBindingGate` traits shipped in N23
  are stable ; N24 only adds **impls** in a new crate
  `efe-compliance-wiring`.  Sprint 15 callers do not need to
  rebuild.
- **No new HTTP surface beyond the admin endpoint.**  The
  six existing endpoints (`/v1/nano-catalogue`,
  `/v1/swarms/run`, …) are untouched.
- **No catalogue format change.**  `NanoAgent.signature`
  remains optional ; existing unsigned tenant catalogues
  continue to dispatch under `experimental_relaxed` policy
  (a 90-day grandfather window per ADR-4).

---

## 1. Context, motivation, sprint placement

### 1.1 Where N24 lands in the M50 timeline

```
Sprint 8  N11..N15  Firecracker pool / cache hardening / shadow-canary
Sprint 9  N16,N17   Multi-region replay / LLM provenance
Sprint 10 N18       Publisher identity (Ed25519 + KYC + sanctions)
Sprint 11 N19       Gold-test gate Tier-1 promotion
Sprint 12 N20       Postgres real embeddings (M33 pivot)
Sprint 13 N21       Swarm orchestration + Agentic Bridge (Claude)
Sprint 14 N22       HTTP gateway exposure
Sprint 15 N23       NanoAgent ↔ SwarmAgent adapter + reuse
Sprint 16 N24  <<<  THIS — Wired compliance runner
Sprint 17 N25       Multi-region nano federation
```

N24 is the **trust-on-the-wire** sprint.  N18-N20 shipped the
**control plane** for compliance (publisher identity, gold
sets, sanctions screening).  N21-N23 shipped the **runtime
plane** (swarm orchestrator, gateway, adapter).  Until now the
two planes have been bridged only by a *placeholder* — N23's
`SignatureGate` and `GoldBindingGate` traits with no concrete
impls in the dispatch path.  N24 ships those impls.

### 1.2 The N23 architecture pinned the deferral explicitly

Three artifacts already name N24 :

1. **`crates/efe-gateway/src/dag_registry.rs`** :
   > *N23 + G10 + Workflow D : pull every node from the
   > catalogue and wrap with `NanoToSwarm`.  Default policy is
   > `experimental_fast` (no compliance gates) — Sprint 16 N24
   > wires `tier1_strict` once the gateway carries the
   > SignatureGate + GoldBindingGate instances.*

2. **`EFE_M50_N23_*.md` § 3 G2 + G4** :
   > *G2 — Signature verification on dispatch.  If
   > `policy.verify_signature == true` …*  (the trait surface
   > is shipped ; the wired impl is N24.)

3. **`EFE_M50_N23_*.md` § 14 Sprint 15 ship vs Sprint 16+
   split** lists "wire real signature verifier" as the first
   item in the Sprint 16 column.

### 1.3 Stakeholders

| Role                       | Concern                                  | Mitigation                              |
|----------------------------|------------------------------------------|-----------------------------------------|
| EU compliance officer      | "Is every dispatch signed by a known publisher ?" | G1 + G2 ; envelope shows `signature_verified=true` |
| Customer (finance)         | "Will a forged catalogue accidentally bill us ?" | G2 + F1 ; verify_strict + fail-closed |
| Customer (DPO)             | "If we revoke a publisher, do their nanos stop running ?" | G11 + I12 ; revocation cascades to swarm |
| Platform SRE               | "If the publisher DB is slow, does the swarm hang ?" | G7 + I9 ; bounded lookup with circuit-breaker fallback |
| Auditor                    | "Can I reconstruct a 90-day-old dispatch's signature ?" | G6 + I7 ; historical key chain in verification |
| Developer (onboarding)     | "How do I sign a new built-in catalogue entry ?" | G9 ; build.rs signs with the baked EFE publisher key |
| Tenant (publishes a nano)  | "If my key rotates, do my old dispatches keep working ?" | I7 ; historical keys preserved |
| Sales engineer             | "Can I show a customer the live compliance posture ?" | G13 ; admin endpoint |

### 1.4 Why a separate `efe-compliance-wiring` crate

Three rejected alternatives during the design freeze :

- **A1 — Put the wired impls in `efe-nano-swarm-adapter`.**
  Rejected.  That crate is dependency-light by design (no
  `efe-publisher-identity`, no `efe-gold-test-gate`).  Pulling
  them in cancels the ADR-1 / I17 dependency hygiene.

- **A2 — Put the wired impls in `efe-gateway`.**  Rejected.
  Couples the gateway HTTP surface to the verification logic.
  Sprint 17+ wants to ship a CLI / FFI / MCP wrapper that
  reuses the same wired stack — the wiring should live in a
  shared crate.

- **A3 — Inline the impls into `efe-publisher-identity` +
  `efe-gold-test-gate`.**  Rejected.  Those crates ship the
  *primitives* (registry, verifier, gold gate).  The wired
  compliance runner is an **integration** of the primitives ;
  the integration should not pollute the upstream crates.

The chosen shape : a new tiny crate
`efe-compliance-wiring` with `[dependencies]` :

```
efe-nano-sdk
efe-nano-swarm-adapter
efe-publisher-identity
efe-gold-test-gate
```

The gateway picks it up via one dependency line.  Sprint 17+
CLI / FFI / MCP layers do the same.

---

## 2. Twelve incident patterns N24 closes (P1–P12)

### P1 — Signed catalogue entry but no verification path

The EFE staging environment crashed an internal audit when a
catalogue entry shipped with a signature but the dispatch path
ignored it.  Auditor reported "the platform stores trust but
does not check it."  **Fix** : G2 + I4 ; every dispatch under
the tier1_strict policy verifies.

### P2 — Revoked publisher's nanos kept dispatching

Customer revoked a publisher via
`POST /v1/publishers/<id>/revoke` (N18 G15 RevocationCascade).
The publish-side block fired ; the swarm-side dispatch did
not.  An orchestration kept invoking the revoked publisher's
agents for ~6 hours.  **Fix** : G11 + I12 ; PublisherStatusGate
consults the registry on every dispatch.

### P3 — Forged signature with a stolen key

An attacker who stole a leaked private key signed a malicious
catalogue entry.  Without verify_strict, the signature passed
because no canonical-hash sanity check fired.  **Fix** : I3 +
F4 ; verify_strict + canonical-hash sanity (the N18 contract
already enforces this — N24 just routes through it).

### P4 — Gold-set silently rewritten by an operator

A junior ops engineer ran a registry-side patch that updated a
gold set's content without re-approving it.  The catalogue's
`gold_set_ref.gold_set_hash` still referenced the old content
hash, but the gate didn't detect the drift.  **Fix** : G3 + I5
; WiredGoldBindingGate verifies the stored hash matches
`nano.gold_set_ref.gold_set_hash` AND the status is `Approved`.

### P5 — Verifier latency turned a fast swarm into a slow one

The publisher registry lookup ran against a cold cache ; each
of the 6 default-DAG agents added ~150 ms.  Total swarm latency
jumped from 280 ms (Sprint 15) to ~1.3 s.  **Fix** : G7 + I9 +
ADR-7 ; in-memory LRU cache for `(publisher_id) →
PublisherIdentity` with a 60 s TTL.

### P6 — Catalogue hash forgery via timestamp grinding

The canonical body includes `timestamp_epoch_seconds`.  An
attacker who controlled timestamps could grind a near-collision
on the canonical hash (cosmetic — Ed25519 stops short of
grinding-resistance).  **Fix** : F6 ; the signature carries
its `signed_at` ; the verifier recomputes the canonical hash
from that exact timestamp (no caller-supplied bypass).

### P7 — Historical-key lookup edge case in replay

Multi-region replay (N16) reconstructed a 45-day-old dispatch
where the publisher had rotated keys 30 days ago.  The verifier
used the *current* active key instead of the historical one ;
verification failed.  **Fix** : I7 + G6 ; verifier uses
`PublisherIdentity::key_active_at(signature.signed_at)`.

### P8 — Unsigned tenant catalogue caused a hard failure

A small tenant migrated from a pre-N18 deployment and their
existing nanos had no signatures.  Strict policy rejected them
all.  **Fix** : ADR-4 + G14 ; a 90-day grandfather window
where unsigned catalogue entries dispatch under
`experimental_relaxed` (no signature, no gold, full audit
trail) with a per-tenant deadline tracked in the registry.

### P9 — Gold-binding lookup blocked the bus

A misconfigured `GoldApi` impl held a tokio Mutex across an
await.  Three concurrent dispatches deadlocked.  **Fix** : I10
; `WiredGoldBindingGate` does NOT hold any lock across await,
and the integration test enforces that lemma via the
`tokio-deadlock-detector` (already wired in N20 audit).

### P10 — Admin had no visibility into the compliance posture

The customer asked "how many of my dispatches were signed in
the last 24h ?" and the answer required a DB query.  **Fix** :
G13 ; `GET /v1/admin/compliance-status` returns a live
snapshot (publishers, keys, gold sets, last-hour gate
breakdown).

### P11 — Signature gate failed but envelope said success

A bug in the N23 adapter's error-path envelope flushing
labelled an `AgentOutcome::Errored` outcome with
`signature_verified: false` but `outcome: completed` (a stale
local variable).  Caught during the N23 audit ; N24
re-confirms with a regression test.  **Fix** : G15 + I8 ;
audit_n24_envelope_outcome_label_matches_agent_outcome.

### P12 — Cache poisoning via registry handle reuse

A test shared the same `Arc<dyn PublisherRegistry>` between
two `WiredComplianceRunner` instances ; the cache leaked across
tenants.  **Fix** : ADR-7 ; the cache lives **inside** the
runner, not the registry trait object ; each runner instance
owns its cache.

---

## 3. Commitments — what ships in Sprint 16 (G1–G15)

### G1 — `efe-compliance-wiring` crate

New workspace crate :
- `src/lib.rs` : module declarations + re-exports.
- `src/signature_gate.rs` : `WiredSignatureGate` impl.
- `src/gold_gate.rs` : `WiredGoldBindingGate` impl.
- `src/status_gate.rs` : `PublisherStatusGate` trait + wired impl.
- `src/canonical.rs` : `nano_to_canonical_body(&NanoAgent,
  &PublisherSignature) -> CanonicalBody` reconstruction.
- `src/runner.rs` : `WiredComplianceRunner` factory.
- `src/cache.rs` : `PublisherIdentityCache` (LRU + TTL).
- `src/metrics.rs` : `N24ComplianceMetrics` (5 Prometheus
  surfaces : signature_verified_total{outcome},
  gold_bound_total{outcome}, status_blocked_total{reason},
  cache_hit_ratio, lookup_latency_ms).

### G2 — Signature verification on every Tier-1+ dispatch

`WiredSignatureGate::verify(nano, signature)` MUST :

1. Resolve the publisher : `publisher_id` from
   `signature.publisher_id` ; look up `PublisherIdentity`.
2. Resolve the key : prefer `signature.key_id` exact match in
   `identity.active_key` or `identity.historical_keys` ; fall
   back to `key_active_at(signature.signed_at)` only if the
   exact match fails.
3. Reconstruct the `CanonicalBody` from the `NanoAgent` +
   signature (cf. annex A1).
4. Call `verifier.verify(signature, &canonical_bytes,
   &public_key.public_key_pem)`.

Failure modes :
- Publisher not found → `Err("publisher <id> not registered")`.
- Key not found for `key_id` AND not active at signed_at →
  `Err("no key matches signature.key_id <id> at <ts>")`.
- Verifier returns `SignatureError::*` → `Err(error.to_string())`.

### G3 — Gold binding verification with hash sanity

`WiredGoldBindingGate::assert_approved(nano)` MUST :

1. Read `nano.gold_set_ref` ; if `None` → `Err("no gold_set_ref")`.
2. `gold_api.lookup_gold_set(gold_set_ref.gold_set_id)`.
3. Assert `status == Approved` ; else `Err("status is <status>")`.
4. Assert `lookup_result.gold_set_hash == nano.gold_set_ref.gold_set_hash` ;
   else `Err("gold set hash drift")`.

### G4 — `PublisherStatusGate` trait + new policy field

New trait in `efe-nano-swarm-adapter::policy` (additive — does
NOT break N23 callers) :

```rust
#[async_trait]
pub trait PublisherStatusGate: Send + Sync + 'static {
    async fn assert_active(&self, nano: &NanoAgent) -> Result<(), String>;
}
```

`NanoToSwarmPolicy` gains :

```rust
pub publisher_status_gate: Option<Arc<dyn PublisherStatusGate>>,
pub require_publisher_status: bool,
```

Both default to `None` / `false` to preserve N23 behaviour.
The `tier1_strict_wired` preset turns them ON.

### G5 — `tier1_strict_wired` policy preset

```rust
pub fn tier1_strict_wired(
    signature_gate: Arc<dyn SignatureGate>,
    gold_gate: Arc<dyn GoldBindingGate>,
    status_gate: Arc<dyn PublisherStatusGate>,
    cache: Arc<dyn IdempotencyCache>,
) -> Self {
    Self {
        verify_signature: true,
        require_gold_binding: true,
        require_publisher_status: true,
        enforce_limits: true,
        idempotency_cache: Some(cache),
        signature_gate: Some(signature_gate),
        gold_gate: Some(gold_gate),
        publisher_status_gate: Some(status_gate),
        idempotency_ttl_override: None,
    }
}
```

### G6 — Historical-key chain support

The verifier walks `identity.active_key`, then
`identity.historical_keys`, matching on `key_id` AND
`was_active_at(signed_at)`.  An exact `key_id` match wins ; if
none, fall through to `key_active_at(signed_at)`.

### G7 — Bounded-latency lookup with LRU cache

`PublisherIdentityCache` :

```rust
pub struct PublisherIdentityCache {
    map: DashMap<PublisherId, (PublisherIdentity, Instant)>,
    capacity: usize,
    ttl: Duration,
}
```

Defaults : 1024 entries, 60 s TTL.  Cache MISS → underlying
`registry.lookup(...).await` with a `tokio::time::timeout` of
500 ms.  Timeout → soft fail (`Err("registry lookup timed out")`).
The N22 CatchPanicLayer ensures panics never propagate.

### G8 — `Author::Publisher` enforcement

If `nano.author == Author::Publisher { publisher_id, kyc_tier }`,
the gate asserts that the `signature.publisher_id` matches
`author.publisher_id` (defense against the F8 author-vs-signer
mismatch attack).  This is checked **before** the registry
lookup so a malicious entry with a forged author label fails
fast.

### G9 — Built-in EFE publisher signing at build time

A new `build.rs` in `efe-nano-preanalysis` :

1. Reads a baked-in EFE publisher key (PEM + private — committed
   in `crates/efe-nano-preanalysis/ed25519/efe_publisher_v1.{
   pem, sk}` under a dedicated SPDX `efe-builtin-signer-v1`
   licence).
2. For each of the 6 catalogue descriptors, computes the
   `CanonicalBody`, signs it, and emits a
   `descriptor.signature_b64.rs` static blob.
3. `default_catalogue()` reads the blobs at runtime and
   attaches the signature to each `NanoAgent`.

The built-in signer's public key is **also** baked into a
static `EFE_BUILTIN_PUBLISHER_IDENTITY` constant in
`efe-compliance-wiring::builtin` so a fresh deployment can
verify the catalogue *without* needing the registry to
contain the EFE publisher (chicken-and-egg : the gateway boots
before any external state is loaded).

### G10 — Tenant nano publishing flow

A tenant publishes via the existing
`POST /v1/publishers/<id>/publish-nano` (Sprint 11+) which
already does signature creation.  The wired stack picks up the
new catalogue entry on the next gateway restart (Sprint 16
ships restart-only refresh ; Sprint 17 wires hot reload).

### G11 — Revocation cascades to swarm-side dispatch

When a publisher is revoked via N18's `POST /v1/publishers/<id>/revoke`,
the registry status flips to `Revoked`.  The next dispatch
that touches that publisher's nano hits `PublisherStatusGate`
which checks `identity.dispatch_blocked()` (from N18) and
fails closed.

### G12 — Cache invalidation on registry mutation

When N18 emits a registry mutation event (publish, rotate,
suspend, unsuspend, revoke), the cache evicts the affected
`publisher_id` entry.  Sprint 16 ships **manual invalidation**
exposed via `WiredComplianceRunner::evict_publisher(id)` ;
Sprint 17 wires the N28 event-bus subscription.

### G13 — `GET /v1/admin/compliance-status` endpoint

Operator-scoped read-only endpoint :

```json
{
  "publishers": {
    "total_registered": 142,
    "active": 138, "suspended": 3, "revoked": 1
  },
  "keys": {
    "active": 138,
    "historical": 27
  },
  "gold_sets": {
    "draft": 5, "approved": 89, "rejected": 2
  },
  "dispatches_last_hour": {
    "total": 1832,
    "by_gate_outcome": {
      "signature_verified_true": 1830,
      "signature_verified_false": 2,
      "gold_bound_true": 412,
      "gold_bound_false": 0,
      "status_active": 1832,
      "status_blocked": 0
    }
  },
  "cache_hit_ratio_1h": 0.967,
  "lookup_p99_ms": 12
}
```

### G14 — 90-day grandfather window for unsigned tenant catalogues

Each tenant has a `compliance_deadline: Option<DateTime<Utc>>`
attached to their registry entry.  Until the deadline,
unsigned catalogue entries dispatch under
`experimental_relaxed` (status checked, signature + gold
skipped).  After the deadline they hard-fail.  Sprint 16 ships
the gate ; Sprint 17 wires the operator endpoint to extend /
revoke the grace.

### G15 — Adapter test crate

`efe-compliance-wiring/tests/audit_n24_*.rs`,
`phase_8_n24_*.rs`, `chaos_n24_*.rs`.  Target : 25 acceptance
+ 40 unit + 10 chaos + 8 audit regression.

---

## 4. Invariants — non-negotiables (I1–I20)

| #   | Invariant                                                                                          |
|-----|----------------------------------------------------------------------------------------------------|
| I1  | A tier1_strict dispatch ALWAYS verifies the signature ; failure is fail-closed.                    |
| I2  | The canonical body is reconstructed at verification time ; the catalogue MUST NOT store it.        |
| I3  | The `verifier.verify` call is the SOLE crypto primitive ; the gate does NOT reimplement Ed25519.   |
| I4  | The `key_id` exact match wins over `was_active_at(signed_at)` fallback.                            |
| I5  | Gold-set hash sanity is checked AFTER the status check (cheap-first).                              |
| I6  | The publisher registry lookup is wrapped in a 500 ms timeout ; never blocks the swarm beyond that. |
| I7  | Historical keys preserve verification of N16 multi-region replay (≥ 90 days).                      |
| I8  | The envelope `signature_verified` boolean reflects the REAL gate outcome, never a defaulted true.  |
| I9  | The cache returns a CLONE of `PublisherIdentity` ; no Arc-sharing across requests (avoid surprise mutation). |
| I10 | The wiring crate is `#![deny(unsafe_code)]` ; entire crate compiles under `cargo clippy -- -D warnings`. |
| I11 | Verification failure produces `AdapterError::SignatureInvalid` ; the envelope outcome is `Errored`. |
| I12 | A revoked publisher's dispatch fails BEFORE the signature gate (cheap-first ; status is the first check). |
| I13 | The wiring crate does NOT depend on `efe-gateway` ; arrow is `gateway → wiring`.                   |
| I14 | The cache MUST evict on revoke / suspend ; explicit invalidation API exists for ops.                |
| I15 | The built-in EFE publisher key is baked at build-time ; no runtime fetch.                          |
| I16 | The `experimental_relaxed` policy still emits the envelope ; only the gate booleans change.        |
| I17 | A nano with `Author::Publisher { publisher_id }` MUST match `signature.publisher_id` (G8).         |
| I18 | The admin endpoint requires the `operator` scope (per N22 G6) ; tenants cannot read other tenants. |
| I19 | The grace window is per-tenant ; once it expires, unsigned entries hard-fail.                      |
| I20 | The wiring crate exposes ONE public factory (`WiredComplianceRunner::new`) — no scattered builders.|

---

## 5. Failure modes — what we PROTECT against (F1–F15)

| #   | Failure                                                                  | Mitigation                                |
|-----|--------------------------------------------------------------------------|-------------------------------------------|
| F1  | Forged signature with a stolen key, on a valid catalogue body            | I3 + verify_strict + Ed25519 hardness     |
| F2  | Tampered catalogue body, signature unchanged                             | I2 + canonical_hash sanity inside `Ed25519Verifier::verify` (CanonicalHashMismatch) |
| F3  | Forged author label : `Author::Publisher { other_id }` w/ valid signature | G8 + I17 ; author.publisher_id must match signature.publisher_id |
| F4  | Replay attack : an old (valid) signature applied to a new catalogue body | F2 catches this via canonical hash mismatch ; F4 explicitly tested |
| F5  | Publisher key rotated mid-flight ; in-flight dispatch verifies against new key | I7 + I4 ; key_id match preferred ; was_active_at fallback by timestamp |
| F6  | Timestamp grinding to fake a near-collision                              | Ed25519 + SHA-256 collision-resistance ; not feasible at this layer |
| F7  | Gold set hash silently rewritten by ops                                  | G3 ; verify nano.gold_set_ref.gold_set_hash matches lookup result |
| F8  | Revoked publisher continues to dispatch                                  | G11 + I12 ; status gate first             |
| F9  | Suspended publisher dispatches via stale cache                           | G12 + I14 ; cache invalidation on suspend |
| F10 | Publisher DB outage hangs the swarm                                      | G7 + I6 ; 500 ms timeout + soft-fail      |
| F11 | Cache TTL races between concurrent lookups                               | DashMap shard-locks ; insert is idempotent  |
| F12 | Built-in EFE publisher key compromised post-deployment                   | Rotate via `efe-publisher-identity` admin endpoint ; bumped key version baked next release |
| F13 | An unsigned legacy tenant catalogue hard-fails on day 1                  | G14 + I19 ; 90-day grace                  |
| F14 | The grace window is silently extended past 90 days                       | I19 ; per-tenant deadline + automatic enforcement |
| F15 | A test fixture accidentally hardcodes a real publisher key               | Test crate uses `Ed25519Signer::from_seed([fixed seed])` ; CI scan rejects committed PEMs outside the static dir |

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1 — Separate `efe-compliance-wiring` crate

**Decision** : new workspace crate ; do not pollute existing
ones.  Already covered in § 1.4.

### ADR-2 — Reconstruct the CanonicalBody at verification time

**Decision** : the catalogue does NOT store the canonical
body bytes ; only the signature.  The verifier rebuilds the
canonical body from `NanoAgent` fields + `signature.signed_at`.

**Rejected alternatives** :
- Store `canonical_body_bytes: Vec<u8>` alongside `signature` —
  doubles the catalogue size ; risk of drift between body
  bytes and `NanoAgent` fields.
- Lazy-compute the canonical body and cache it — added
  complexity for negligible win (Mode A pre-analysis dispatch
  is ~5 ms ; canonical body recompute is < 100 µs).

### ADR-3 — Three-gate model (sig + gold + status)

**Decision** : ship three independent gates instead of one
combined "compliance gate".

**Rationale** : compliance is **policy-tiered**.
TierExperimental wants the signature check (cheap proof of
authenticity) without the gold gate (expensive infra).  A
single combined gate would force all-or-nothing.

### ADR-4 — 90-day grandfather window for unsigned tenants

**Decision** : tenants get 90 days from the N24 release date
to sign their catalogues.  Until then, unsigned entries
dispatch under `experimental_relaxed`.  After day 90, unsigned
entries hard-fail.

**Rationale** : balances compliance posture (we want every
dispatch signed by EOY 2026) with customer adoption smoothness.

### ADR-5 — Cache TTL of 60 s

**Decision** : `PublisherIdentityCache` defaults to a 60 s
TTL.

**Rationale** : a 60 s window is shorter than the typical
publisher state transition (suspend → revoke takes minutes by
operator workflow).  Sprint 17 wires the N28 event bus for
real-time invalidation ; for Sprint 16 60 s is the bridge.

### ADR-6 — Cache cap of 1024 entries

**Decision** : LRU cap at 1024 publishers.  Eviction on cap
breach is LRU (oldest-first).

**Rationale** : 1024 publishers × ~2 KB per identity = 2 MB ;
well under the gateway's heap footprint.  EU enterprise launch
target is ~200 publishers ; 5× headroom.

### ADR-7 — Cache lives inside `WiredComplianceRunner`, not on the registry

**Decision** : the cache is OWNED by the runner ; the registry
remains stateless (per N18's design).

**Rationale** : avoids the F11 cross-tenant cache poisoning
risk and the F12 cache-poisoning-via-handle-reuse.  Two
runners with different policies (e.g. EU vs US) can run
side-by-side with independent caches.

### ADR-8 — Verifier injection via trait object

**Decision** : `WiredSignatureGate` holds
`Arc<dyn SignatureVerifier>`, not a concrete
`Ed25519Verifier`.

**Rationale** : Sprint 17+ ships `MultiAlgVerifier`
(Ed25519 + Ed448 + post-quantum stub) ; the gate plumbing
should not need to know.

### ADR-9 — Built-in EFE publisher identity baked into the binary

**Decision** : `EFE_BUILTIN_PUBLISHER_IDENTITY` is a `pub
const` in `efe-compliance-wiring::builtin`.  Verifies the
catalogue without any external state.

**Rationale** : chicken-and-egg : the gateway boots before the
publisher registry is loaded.  Without a baked identity, the
first dispatch would always fail.

### ADR-10 — Grace window enforcement in the gate, not the policy

**Decision** : `WiredSignatureGate` carries a reference to a
`GraceWindowResolver` ; the gate checks the grace window per
tenant before returning a verification verdict.

**Rationale** : the policy is intentionally tenant-agnostic.
Tenant-specific logic stays in the gate, where the runtime
context (the calling JWT's tenant_id) is available.

---

## 7. Component architecture

```
                                ┌──────────────────────────────────────┐
                                │  efe-gateway (HTTP surface, N22)     │
                                │   AppState                            │
                                │   ├── jwt_validator  (N22)            │
                                │   ├── publisher_api  (N18)            │
                                │   ├── gold_api       (N19)            │
                                │   ├── swarm          (N21)            │
                                │   ├── agentic        (N21)            │
                                │   └── compliance     (N24, NEW)       │
                                └─────────────┬────────────────────────┘
                                              │ Arc<WiredComplianceRunner>
                                              ▼
                ┌─────────────────────────────────────────────────────┐
                │  efe-compliance-wiring (NEW crate, N24)             │
                │                                                      │
                │   ┌──────────────────────────────────────────┐      │
                │   │  WiredComplianceRunner (factory + facade) │      │
                │   │   - signature_gate   : WiredSignatureGate │      │
                │   │   - gold_gate        : WiredGoldBindingGate│     │
                │   │   - status_gate      : WiredPublisherStatusGate │
                │   │   - cache            : PublisherIdentityCache  │
                │   │   - metrics          : N24ComplianceMetrics    │
                │   │   - tier1_strict_policy() / experimental_relaxed_policy() │
                │   │   - evict_publisher() / refresh_cache()         │
                │   └──────────────────────────────────────────┘      │
                └───────────────┬───────────────────────────────┬─────┘
                                │ Arc<dyn SignatureGate>          │ Arc<dyn GoldBindingGate>
                                │ Arc<dyn PublisherStatusGate>    │
                                ▼                                  ▼
            ┌──────────────────────────────┐    ┌──────────────────────────────┐
            │  efe-nano-swarm-adapter (N23) │   │  efe-gold-test-gate (N19)    │
            │   NanoToSwarm + Policy        │   │   GoldApi + GoldGate         │
            │   (gates are dyn-trait OPTS)  │   │                              │
            └──────────────────────────────┘    └──────────────────────────────┘
                          │                                  ▲
                          ▼                                  │
            ┌──────────────────────────────┐    ┌──────────────────────────────┐
            │  efe-publisher-identity (N18) │   │  efe-nano-sdk (M50)          │
            │   PublisherRegistry            │   │   NanoAgent, PublisherSignature, │
            │   Ed25519Verifier              │   │   CanonicalBody              │
            └──────────────────────────────┘    └──────────────────────────────┘
```

### 7.1 Module boundaries

- **`WiredSignatureGate`** : the only module that calls
  `Ed25519Verifier::verify` from inside the swarm dispatch
  path.
- **`WiredGoldBindingGate`** : the only module that calls
  `GoldApi::lookup_gold_set` from inside the swarm dispatch.
- **`WiredPublisherStatusGate`** : the only module that calls
  `identity.dispatch_blocked()` from inside the swarm
  dispatch.
- **`PublisherIdentityCache`** : ONLY used by the three gates ;
  never exposed to the policy or the adapter.
- **`WiredComplianceRunner`** : the only public factory ; all
  three gates + cache are private fields.

### 7.2 Dependency arrows

```
efe-gateway  →  efe-compliance-wiring  →  efe-nano-swarm-adapter
                                        →  efe-publisher-identity
                                        →  efe-gold-test-gate
                                        →  efe-nano-sdk
```

No back-arrows.  `efe-compliance-wiring` never depends on
`efe-gateway`.

---

## 8. Module hierarchy & data model

```
crates/efe-compliance-wiring/
├── Cargo.toml
├── src/
│   ├── lib.rs                # module declarations + public re-exports
│   ├── builtin.rs            # baked-in EFE publisher identity + signing-key constants
│   ├── canonical.rs          # nano_to_canonical_body() reconstruction helper
│   ├── cache.rs              # PublisherIdentityCache (LRU + TTL)
│   ├── error.rs              # WiringError taxonomy (translates to AdapterError)
│   ├── gates/
│   │   ├── mod.rs
│   │   ├── signature.rs      # WiredSignatureGate
│   │   ├── gold.rs           # WiredGoldBindingGate
│   │   └── status.rs         # WiredPublisherStatusGate + PublisherStatusGate trait
│   ├── grace.rs              # GraceWindowResolver trait + InMemoryGraceWindow impl
│   ├── metrics.rs            # N24ComplianceMetrics (5 Prometheus surfaces)
│   ├── runner.rs             # WiredComplianceRunner factory + facade
│   └── builtin/
│       ├── efe_publisher_v1.pem  # baked EFE publisher public key (PEM)
│       └── efe_publisher_v1.sk   # baked EFE publisher private key (build-time only)
└── tests/
    ├── phase_8_n24_wired_compliance.rs
    ├── audit_n24_wired_compliance.rs
    └── chaos_n24_wired_compliance.rs
```

### 8.1 Public surface (re-exported from `lib.rs`)

```rust
pub use builtin::{
    EFE_BUILTIN_PUBLISHER_ID, EFE_BUILTIN_PUBLISHER_IDENTITY,
    EFE_BUILTIN_PUBLISHER_KEY_ID, EFE_BUILTIN_PUBLISHER_PEM,
};
pub use cache::PublisherIdentityCache;
pub use canonical::nano_to_canonical_body;
pub use error::WiringError;
pub use gates::{
    WiredGoldBindingGate, WiredPublisherStatusGate, WiredSignatureGate,
};
pub use grace::{GraceWindowResolver, InMemoryGraceWindow};
pub use metrics::N24ComplianceMetrics;
pub use runner::{
    WiredComplianceConfig, WiredComplianceRunner, DEFAULT_CACHE_CAPACITY,
    DEFAULT_CACHE_TTL, DEFAULT_LOOKUP_TIMEOUT_MS,
};
```

### 8.2 Data model

```rust
// runner.rs
pub struct WiredComplianceRunner {
    publisher_registry: Arc<dyn PublisherRegistry>,
    verifier: Arc<dyn SignatureVerifier>,
    gold_api: Arc<dyn GoldApi>,
    cache: Arc<PublisherIdentityCache>,
    grace: Arc<dyn GraceWindowResolver>,
    metrics: Arc<N24ComplianceMetrics>,
    config: WiredComplianceConfig,
}

pub struct WiredComplianceConfig {
    pub cache_capacity: usize,    // default 1024
    pub cache_ttl: Duration,      // default 60 s
    pub lookup_timeout_ms: u32,   // default 500 ms
    pub builtin_publisher_identity: PublisherIdentity, // ADR-9
}

// cache.rs
pub struct PublisherIdentityCache {
    map: DashMap<PublisherId, (PublisherIdentity, Instant)>,
    capacity: usize,
    ttl: Duration,
    hit_count: AtomicU64,
    miss_count: AtomicU64,
}

// error.rs
#[derive(Debug, Clone, PartialEq, Error)]
pub enum WiringError {
    #[error("publisher {0} not found")]
    PublisherNotFound(PublisherId),
    #[error("publisher {0} is blocked from dispatch ({reason})")]
    PublisherBlocked { publisher_id: PublisherId, reason: String },
    #[error("publisher registry lookup timed out after {ms} ms")]
    RegistryTimeout { ms: u32 },
    #[error("no key matches signature.key_id {key_id} at {signed_at:?}")]
    KeyNotFound { key_id: KeyId, signed_at: SystemTime },
    #[error("author.publisher_id mismatch ; nano says {nano_pub_id}, signature says {sig_pub_id}")]
    AuthorMismatch { nano_pub_id: PublisherId, sig_pub_id: PublisherId },
    #[error("signature verification failed : {0}")]
    SignatureFailed(String),
    #[error("gold set lookup failed : {0}")]
    GoldLookupFailed(String),
    #[error("gold set hash drift ; nano expects {expected}, registry has {actual}")]
    GoldHashDrift { expected: String, actual: String },
    #[error("gold set status is {0}, expected Approved")]
    GoldNotApproved(String),
    #[error("grace window expired for tenant {0}")]
    GraceExpired(uuid::Uuid),
}
```

### 8.3 Gate impls (sketches)

```rust
// gates/signature.rs
pub struct WiredSignatureGate {
    registry_cache: Arc<PublisherIdentityCache>,
    registry: Arc<dyn PublisherRegistry>,
    verifier: Arc<dyn SignatureVerifier>,
    lookup_timeout: Duration,
    metrics: Arc<N24ComplianceMetrics>,
    builtin_identity: PublisherIdentity,
}

#[async_trait]
impl SignatureGate for WiredSignatureGate {
    async fn verify(
        &self,
        nano: &NanoAgent,
        signature: &PublisherSignature,
    ) -> Result<(), String> {
        // 1. Author-vs-signer match (G8 / I17).
        if let Author::Publisher { publisher_id: author_id, .. } = &nano.author {
            if *author_id != signature.publisher_id {
                self.metrics.signature_verified_total("author_mismatch");
                return Err(WiringError::AuthorMismatch {
                    nano_pub_id: *author_id,
                    sig_pub_id: signature.publisher_id,
                }.to_string());
            }
        }

        // 2. Lookup publisher (cache → registry with timeout).
        let identity = self.resolve_identity(signature.publisher_id).await
            .map_err(|e| e.to_string())?;

        // 3. Find the key matching `signature.key_id` (I4).
        let key = self.find_key(&identity, signature)
            .ok_or_else(|| WiringError::KeyNotFound {
                key_id: signature.key_id,
                signed_at: signature.signed_at,
            }.to_string())?;

        // 4. Reconstruct canonical body (I2).
        let canonical = nano_to_canonical_body(nano, signature);
        let bytes = canonical.canonical_bytes();

        // 5. Cryptographic verify (I3).
        self.verifier.verify(signature, &bytes, &key.public_key_pem).await
            .map_err(|e| {
                self.metrics.signature_verified_total("verify_failed");
                WiringError::SignatureFailed(e.to_string()).to_string()
            })?;

        self.metrics.signature_verified_total("ok");
        Ok(())
    }
}

// gates/gold.rs
pub struct WiredGoldBindingGate {
    gold_api: Arc<dyn GoldApi>,
    metrics: Arc<N24ComplianceMetrics>,
}

#[async_trait]
impl GoldBindingGate for WiredGoldBindingGate {
    async fn assert_approved(&self, nano: &NanoAgent) -> Result<(), String> {
        let gold_ref = nano.gold_set_ref.as_ref().ok_or_else(|| {
            self.metrics.gold_bound_total("no_ref");
            "nano has no gold_set_ref".to_string()
        })?;
        let set = self.gold_api.lookup_gold_set(gold_ref.gold_set_id).await
            .map_err(|e| {
                self.metrics.gold_bound_total("lookup_failed");
                WiringError::GoldLookupFailed(e.to_string()).to_string()
            })?;
        if !matches!(set.status, GoldSetStatus::Approved) {
            self.metrics.gold_bound_total("not_approved");
            return Err(WiringError::GoldNotApproved(set.status.label().into()).to_string());
        }
        if set.gold_set_hash != gold_ref.gold_set_hash {
            self.metrics.gold_bound_total("hash_drift");
            return Err(WiringError::GoldHashDrift {
                expected: gold_ref.gold_set_hash.to_hex(),
                actual: set.gold_set_hash.to_hex(),
            }.to_string());
        }
        self.metrics.gold_bound_total("ok");
        Ok(())
    }
}

// gates/status.rs
#[async_trait]
pub trait PublisherStatusGate: Send + Sync + 'static {
    async fn assert_active(&self, nano: &NanoAgent) -> Result<(), String>;
}

pub struct WiredPublisherStatusGate {
    registry_cache: Arc<PublisherIdentityCache>,
    registry: Arc<dyn PublisherRegistry>,
    lookup_timeout: Duration,
    metrics: Arc<N24ComplianceMetrics>,
}

#[async_trait]
impl PublisherStatusGate for WiredPublisherStatusGate {
    async fn assert_active(&self, nano: &NanoAgent) -> Result<(), String> {
        let Some(publisher_id) = nano.author.publisher_id() else {
            // Built-in / community / tenant — no publisher to check.
            self.metrics.status_blocked_total("not_publisher");
            return Ok(());
        };
        let identity = self.resolve(publisher_id).await
            .map_err(|e| e.to_string())?;
        if identity.dispatch_blocked() {
            self.metrics.status_blocked_total(identity.status.label());
            return Err(WiringError::PublisherBlocked {
                publisher_id,
                reason: identity.status.label().into(),
            }.to_string());
        }
        self.metrics.status_blocked_total("ok");
        Ok(())
    }
}
```

---

## 9. API surface (Rust traits + HTTP)

### 9.1 Public Rust API

| Symbol                                      | Kind   | Purpose                                                       |
|---------------------------------------------|--------|---------------------------------------------------------------|
| `WiredComplianceRunner`                     | struct | Factory + facade ; owns the cache + the three gates           |
| `WiredComplianceRunner::new(cfg, reg, ver, gold, grace, metrics)` | fn | Construct                                            |
| `WiredComplianceRunner::tier1_strict_policy(&self) -> NanoToSwarmPolicy` | fn | Returns a policy with all three gates ON              |
| `WiredComplianceRunner::experimental_relaxed_policy(&self) -> NanoToSwarmPolicy` | fn | Status check only ; signature + gold OFF                |
| `WiredComplianceRunner::evict_publisher(id)` | fn    | Manual cache invalidation                                     |
| `WiredComplianceRunner::compliance_status() -> ComplianceStatus` | fn | Live snapshot for the admin endpoint                          |
| `WiredSignatureGate`                        | struct | Concrete `SignatureGate` impl                                 |
| `WiredGoldBindingGate`                      | struct | Concrete `GoldBindingGate` impl                               |
| `WiredPublisherStatusGate`                  | struct | Concrete `PublisherStatusGate` impl                           |
| `PublisherStatusGate` trait                 | trait  | New trait (additive in `efe-nano-swarm-adapter::policy`)      |
| `PublisherIdentityCache`                    | struct | LRU + TTL ; exposed for tests / metrics                       |
| `nano_to_canonical_body(nano, sig) -> CanonicalBody` | fn | Reconstruction helper                                  |
| `EFE_BUILTIN_PUBLISHER_IDENTITY`            | const  | Baked-in EFE publisher identity                               |
| `N24ComplianceMetrics`                      | struct | 5 Prometheus surfaces                                         |
| `WiringError`                               | enum   | Closed error taxonomy                                         |

### 9.2 Public HTTP API (new in `efe-gateway`)

`GET /v1/admin/compliance-status` — operator scope.

Response shape per § 0 G13.  Implementation reads
`AppState::compliance.compliance_status()`.

### 9.3 N23 adapter contract — additive only

`efe-nano-swarm-adapter::policy::NanoToSwarmPolicy` gets two
new fields :

```rust
pub publisher_status_gate: Option<Arc<dyn PublisherStatusGate>>,
pub require_publisher_status: bool,
```

Both default to `None` / `false` ; existing N23 callers
continue to compile + behave identically.  The N23 audit test
`audit_n23_experimental_fast_has_all_gates_off` is extended to
assert these new defaults.

The forward adapter (`forward.rs`) gains a new gate step in
the dispatch sequence between signature and gold (cheapest
first) :

```rust
// 1. (NEW) Publisher status gate (G11 / F8 / F9).
// 2. Signature gate (G2 / I4).        ── existing N23
// 3. Gold binding gate (G4 / I5).     ── existing N23
// 4. Idempotency cache (G5).          ── existing N23
// 5. Dispatch.                        ── existing N23
```

### 9.4 OpenAPI 3.1 spec update

`handlers/openapi.rs::openapi_spec()` gains the
`/v1/admin/compliance-status` path + a `ComplianceStatus`
schema component.

---

## 10. End-to-end workflows

### Workflow A — Built-in DAG dispatch with tier1_strict

```
Client                Gateway                Adapter                Wiring                  N18/N19
  │                      │                      │                      │                      │
  │ POST /v1/swarms/run  │                      │                      │                      │
  │ Bearer <jwt>         │                      │                      │                      │
  │─────────────────────►│                      │                      │                      │
  │                      │ check_tenant         │                      │                      │
  │                      │ build_default_dag    │                      │                      │
  │                      │ (uses N24 policy)    │                      │                      │
  │                      │                      │ NanoToSwarm.run(ctx) │                      │
  │                      │─────────────────────►│                      │                      │
  │                      │                      │ status_gate.assert_active(nano)             │
  │                      │                      │────────────────────►│                      │
  │                      │                      │                      │ cache.get(publisher_id)?
  │                      │                      │                      │   MISS               │
  │                      │                      │                      │────────────────────►│
  │                      │                      │                      │ ←─ PublisherIdentity │
  │                      │                      │                      │ cache.put(...)       │
  │                      │                      │ ←─ Ok(())            │                      │
  │                      │                      │ signature_gate.verify(nano, sig)            │
  │                      │                      │────────────────────►│                      │
  │                      │                      │                      │ resolve_identity (cache HIT)
  │                      │                      │                      │ find_key by key_id   │
  │                      │                      │                      │ canonical = nano_to_canonical_body
  │                      │                      │                      │ verifier.verify(...) ►│
  │                      │                      │                      │ ←─ Ok                │
  │                      │                      │ ←─ Ok(())            │                      │
  │                      │                      │ gold_gate.assert_approved(nano)             │
  │                      │                      │────────────────────►│                      │
  │                      │                      │                      │ gold_api.lookup_gold_set
  │                      │                      │                      │────────────────────►│
  │                      │                      │                      │ ←─ GoldSet { Approved, hash }
  │                      │                      │                      │ verify hash match    │
  │                      │                      │ ←─ Ok(())            │                      │
  │                      │                      │ dispatch runner      │                      │
  │                      │                      │ write blackboard     │                      │
  │                      │                      │ emit envelope V1     │                      │
  │                      │                      │   signature_verified=true                   │
  │                      │                      │   gold_bound=true                            │
  │                      │ ←─ SwarmReport       │                      │                      │
  │ ←─ 200               │                      │                      │                      │
```

### Workflow B — Revoked publisher

A POST /v1/publishers/<id>/revoke fires.  Within 60 s
(cache TTL ; Sprint 17 cuts this to ~1 s via N28 events) the
next dispatch sees `status_gate` return
`PublisherBlocked { reason: "Revoked" }`.  The envelope shows
`outcome: errored, message: "publisher <uuid> is blocked
(Revoked)"`.

### Workflow C — Key rotation mid-dispatch

A POST /v1/publishers/<id>/rotate-key fires.  The signature
gate is called with the OLD signature (older `key_id` +
`signed_at`).  `WiredSignatureGate.find_key` matches the
historical key by `key_id` (I4) ; verification succeeds.  No
client-visible change ; the envelope's `signature_verified` is
`true`.

### Workflow D — Tenant grace window

A tenant with `compliance_deadline = 2027-01-01` publishes an
**unsigned** nano on 2026-09-01.  The gateway picks it up.
Dispatch :

- `status_gate.assert_active` : OK (tenant not blocked).
- `signature_gate.verify` : `nano.signature.is_none()` → the
  gate consults `GraceWindowResolver.in_grace_window(tenant_id)`
  → `true` → returns `Ok(())` with a metric tick
  `signature_verified_total{outcome="grace_skip"}`.
- `gold_gate.assert_approved` : `nano.gold_set_ref.is_none()`
  → returns `Ok(())` with a metric tick
  `gold_bound_total{outcome="grace_skip"}`.

The envelope shows `signature_verified=false` and
`gold_bound=false` but the dispatch succeeds.  After day 90
the resolver returns `false` ; the same dispatch hard-fails
with `WiringError::GraceExpired(tenant_id)`.

### Workflow E — Cache eviction on revoke

Operator calls `POST /v1/publishers/<id>/revoke`.  The gateway
handler — Sprint 16 ships the wiring change in
`handlers/publishers.rs::revoke_publisher_handler` :

```rust
let cascade = state.publisher_api.revoke(...).await?;
// NEW : evict the cache so the next dispatch sees the change.
state.compliance.evict_publisher(publisher_id);
Ok(Json(cascade))
```

---

## 11. Five-day implementation plan

### Day 1 — Crate skeleton + canonical reconstruction

- Create `crates/efe-compliance-wiring/`.
- Workspace registration.
- `lib.rs` + `error.rs` + `canonical.rs`.
- `nano_to_canonical_body(nano, sig) -> CanonicalBody` + unit
  tests (10+ : agent_id round-trip, version round-trip,
  publisher_id round-trip, hash stability across runs, etc.).
- `cargo check -p efe-compliance-wiring` green.

### Day 2 — Cache + status gate + signature gate

- `cache.rs` : `PublisherIdentityCache` with LRU + TTL +
  metrics counters.
- `gates/status.rs` : `PublisherStatusGate` trait + impl.
- `gates/signature.rs` : `WiredSignatureGate` impl.
- Unit tests : 25+.
- Cargo build + clippy strict green.

### Day 3 — Gold gate + grace window + builtin

- `gates/gold.rs` : `WiredGoldBindingGate` impl.
- `grace.rs` : `GraceWindowResolver` trait + in-memory impl.
- `builtin.rs` : bake the EFE publisher identity ; ship a
  `build.rs` that signs every preanalysis catalogue entry.
- N23 adapter additive change : add `publisher_status_gate`
  field to `NanoToSwarmPolicy` + adjust dispatch order in
  `forward.rs`.
- Cargo build + clippy strict green.

### Day 4 — `WiredComplianceRunner` + gateway integration

- `runner.rs` : `WiredComplianceRunner` factory ; expose
  `tier1_strict_policy()` + `experimental_relaxed_policy()` +
  `compliance_status()`.
- `efe-gateway` :
  - Add `compliance: Arc<WiredComplianceRunner>` to AppState.
  - `dag_registry::build_default_dag()` uses
    `select_policy_for_tier()` helper to pick strict vs relaxed
    per catalogue entry.
  - `handlers/admin.rs::compliance_status_handler` :
    operator-scope GET.
  - `router.rs` : new route.
  - `handlers/openapi.rs` : new path + schema.
  - `main.rs::build_app_state` : wire the runner.
- Acceptance tests `phase_8_n24_*.rs` start landing.

### Day 5 — Tests + smoke + docs

- `audit_n24_wired_compliance.rs` (8 invariants : I1, I2, I3,
  I4, I7, I12, I17, I20).
- `chaos_n24_wired_compliance.rs` (10 chaos : F1, F2, F5, F8,
  F10, F11, F13, plus 3 concurrent-cache tests).
- `phase_8_n24_wired_compliance.rs` (25 acceptance : every
  workflow A-E).
- Workspace tests + clippy strict + live smoke.
- `docs/migration/N24_COMPLIANCE_WIRING.md`.

---

## 12. Test strategy

### 12.1 Unit tests (target : 100+)

Per-module unit tests for :
- `canonical.rs` : 12+ (one per CanonicalBody field).
- `cache.rs` : 15+ (TTL eviction, LRU eviction, concurrent
  insert, idempotent insert, metrics counters).
- `gates/signature.rs` : 18+ (author match, key_id match,
  historical key fallback, verify_strict success, all
  WiringError branches).
- `gates/gold.rs` : 12+ (Approved happy path, status not
  Approved, hash drift, lookup error, no gold_set_ref).
- `gates/status.rs` : 10+ (active, suspended, revoked, not a
  publisher author).
- `grace.rs` : 8+ (in window, past window, never set,
  deadline boundary).
- `runner.rs` : 12+ (factory builds, tier1_strict policy,
  experimental_relaxed policy, compliance_status snapshot).
- `metrics.rs` : 10+ (each counter increments).

### 12.2 Phase-8 acceptance (`phase_8_n24_*.rs`, 25 tests)

- Workflow A : built-in DAG with strict policy → signature
  verified + gold bound + status active.
- Workflow B : revoked publisher → dispatch fails closed.
- Workflow C : key rotation → historical key used.
- Workflow D : tenant in grace window → dispatch succeeds with
  envelope flags false.
- Workflow E : cache eviction on revoke.
- Plus 20 cross-cutting tests : envelope correctness, metric
  emission, admin endpoint, OpenAPI spec correctness.

### 12.3 Audit regression (`audit_n24_*.rs`, 8+ tests)

Each invariant from § 4 that has observable behaviour gets a
test :
- I1 : tier1_strict dispatch sees `signature_verified=true`.
- I2 : tampering nano fields between sign and verify is
  detected.
- I3 : the gate does NOT reimplement Ed25519 (test that it
  calls `SignatureVerifier::verify` via a mock).
- I4 : exact key_id match wins over was_active_at fallback.
- I7 : historical keys verify 90-day-old signatures.
- I12 : status check fires BEFORE signature check (cheap-first).
- I17 : author_id mismatch is rejected (G8).
- I20 : single public factory (`WiredComplianceRunner::new`).

### 12.4 Chaos (`chaos_n24_*.rs`, 10+ tests)

- F1 : forged signature → fail closed.
- F2 : tampered body → fail closed (CanonicalHashMismatch).
- F5 : key rotation mid-flight → use the right key.
- F8 : revoked publisher in flight → fail closed within 60 s.
- F10 : 1 s registry latency → 500 ms timeout fires → soft
  fail.
- F11 : 50 concurrent cache lookups → no torn reads.
- F13 : unsigned tenant within grace → pass.
- F14 : grace deadline boundary → flips at the second.
- Concurrent revoke + dispatch.
- Concurrent cache eviction + lookup.

### 12.5 Smoke

After Sprint 16 ship :
1. Start the gateway with a fresh registry seeded with the
   EFE built-in publisher.
2. `GET /v1/nano-catalogue` returns the 6 entries, each with
   `author.type == "publisher"` and a signature.
3. `POST /v1/swarms/run` returns 200 ; the envelope on the
   blackboard shows `signature_verified: true`.
4. `GET /v1/admin/compliance-status` (operator JWT) shows
   `signature_verified_true: N` where N = 6 × number_of_runs.
5. Revoke the EFE built-in publisher via N18 admin endpoint
   (test scenario).  Within 60 s the next swarm run fails
   with `publisher blocked (Revoked)` for all 6 agents.
6. Restore the built-in publisher ; verify dispatch succeeds
   again.

---

## 13. Cross-cutting alignment (N11 → N23)

| Module | Sprint | What N24 changes / aligns                                            |
|--------|--------|----------------------------------------------------------------------|
| N11 — Firecracker pool | 8 | No change.                                                  |
| N12 — Cache key hardening | 8 | The cache key extends with `signature_verified` boolean so cached responses don't cross compliance tiers. |
| N13 — Shadow / canary catalog | 8 | No change.                                              |
| N14 — Swarm envelope trace | 8 | N24 envelope (`signature_verified`, `gold_bound`) is consumed by the existing N14 swarm envelope ; no schema change. |
| N15 — Fairness scheduling | 8 | No change.                                                |
| N16 — Multi-region replay | 9 | N24 historical key chain (I7) is the replay anchor ; the replay subsystem already calls `key_active_at(signed_at)`. |
| N17 — LLM provenance | 9 | No direct change ; the LLM provenance envelope can now reference the same signature trail. |
| N18 — Publisher identity | 10 | N24 is its first runtime consumer outside the publish endpoint.  No change to N18 contracts. |
| N19 — Gold test gate | 11 | N24 is its first dispatch-time consumer.  No change to N19 contracts. |
| N20 — Postgres real embeddings | 12 | No change.                                          |
| N21 — Swarm orchestration | 13 | The orchestrator continues to dispatch via the adapter ; the adapter now has gates filled in. |
| N22 — HTTP gateway | 14 | New endpoint + new `AppState` field.  Operator-scope check uses existing N22 plumbing. |
| N23 — NanoAgent ↔ SwarmAgent adapter | 15 | Additive policy field ; existing callers compile unchanged.  `forward.rs` dispatch order extends to insert the new status gate first. |

---

## 14. Sprint 16 ship vs Sprint 17+ split

### Sprint 16 ships (N24)

- `efe-compliance-wiring` crate, full public surface.
- Three gates (signature, gold, status).
- LRU + TTL cache.
- 90-day grace window (read-only ; manual deadlines).
- Gateway wiring : AppState, dag_registry, admin endpoint.
- Built-in EFE publisher signing via build.rs.
- 100+ unit / 25 acceptance / 8 audit / 10 chaos tests.

### Sprint 17+ defers

- **N25 — Multi-region nano federation** : cache invalidation
  via N28 event-bus subscription (sub-second cross-region).
- **N26 — Hot reload** : gateway picks up new tenant
  catalogues without restart.
- **N27 — Grace-window admin endpoint** : tenants extend /
  revoke their compliance deadline.
- **N28 — Post-quantum signatures** : `MultiAlgVerifier`
  (Ed25519 + Ed448 + Dilithium stub).
- **N29 — Sanctions freshness gate** : fourth gate that
  enforces the N18 sanctions-check-fresh invariant per dispatch
  (Sprint 16 ships the *check* in the registry, not the *gate*).
- **N30 — Per-tenant compliance dashboards** : drill-down into
  the admin endpoint with tenant filtering.

---

## 15. Annexes

### A1 — `nano_to_canonical_body` reconstruction

```rust
pub fn nano_to_canonical_body(
    nano: &NanoAgent,
    signature: &PublisherSignature,
) -> CanonicalBody {
    use efe_publisher_identity::canonical::CANONICAL_SCHEMA_VERSION;

    // The KYC tier is captured at sign time in `nano.author`
    // (Author::Publisher { kyc_tier }).  For non-publisher authors
    // (Efe / Tenant / Community) we use Tier0 — these paths are
    // only reached during the grace window per ADR-4.
    let kyc_tier = match &nano.author {
        Author::Publisher { kyc_tier, .. } => *kyc_tier,
        _ => KycTier::Tier0,
    };

    // Recompute the agent body SHA-256 from a canonical
    // serialisation of the nano *minus the signature*.  Sprint 16
    // uses serde_json with sorted keys ; Sprint 18 may switch to
    // CBOR if performance demands.
    let agent_body_sha256 = compute_agent_body_sha256(nano);

    let timestamp_epoch_seconds = signature
        .signed_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    CanonicalBody {
        schema_version: CANONICAL_SCHEMA_VERSION,
        agent_id: nano.id.as_str().to_string(),
        version: nano.version.as_str().to_string(),
        publisher_id: signature.publisher_id,
        kyc_tier,
        agent_body_sha256,
        timestamp_epoch_seconds,
    }
}

fn compute_agent_body_sha256(nano: &NanoAgent) -> Sha256Hash {
    // Strip signature + co_publishing (the two fields that depend
    // on the signature itself, per chicken-and-egg).
    let mut clone = nano.clone();
    clone.signature = None;
    clone.co_publishing = None;
    let bytes = serde_json::to_vec(&clone).expect("nano serialises");
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    Sha256Hash::from_bytes(out)
}
```

### A2 — `WiredComplianceRunner::tier1_strict_policy` shape

```rust
impl WiredComplianceRunner {
    pub fn tier1_strict_policy(&self) -> NanoToSwarmPolicy {
        NanoToSwarmPolicy {
            verify_signature: true,
            require_gold_binding: true,
            require_publisher_status: true,
            enforce_limits: true,
            idempotency_cache: Some(self.idempotency_cache.clone()),
            signature_gate: Some(self.signature_gate.clone()),
            gold_gate: Some(self.gold_gate.clone()),
            publisher_status_gate: Some(self.status_gate.clone()),
            idempotency_ttl_override: None,
        }
    }

    pub fn experimental_relaxed_policy(&self) -> NanoToSwarmPolicy {
        NanoToSwarmPolicy {
            verify_signature: false,
            require_gold_binding: false,
            require_publisher_status: true,   // status check stays on
            enforce_limits: true,
            idempotency_cache: Some(self.idempotency_cache.clone()),
            signature_gate: None,
            gold_gate: None,
            publisher_status_gate: Some(self.status_gate.clone()),
            idempotency_ttl_override: None,
        }
    }
}
```

### A3 — `select_policy_for_tier` helper

```rust
// crates/efe-gateway/src/dag_registry.rs
fn select_policy_for_tier(
    runner: &WiredComplianceRunner,
    nano: &NanoAgent,
) -> NanoToSwarmPolicy {
    match nano.stability_tier {
        StabilityTier::Tier1 | StabilityTier::Tier2 | StabilityTier::TierAudit => {
            runner.tier1_strict_policy()
        }
        StabilityTier::TierExperimental => {
            runner.experimental_relaxed_policy()
        }
    }
}

fn build_default_dag(
    runner: &WiredComplianceRunner,
) -> Result<SwarmDag, SwarmDagError> {
    let mut builder = SwarmDagBuilder::new();
    for (nano, runner_) in default_catalogue() {
        let policy = select_policy_for_tier(runner, &nano);
        let adapter = NanoToSwarm::new(nano, runner_).with_policy(policy);
        builder = builder.add_agent(AgentNode::new(Arc::new(adapter)));
    }
    builder.add_agent(AgentNode::new(Arc::new(NoOpAggregator)))
        // ... dependencies as before
        .with_aggregator(AgentId::new("context-aggregator").unwrap())
        .build()
}
```

### A4 — Cache implementation outline

```rust
// cache.rs
impl PublisherIdentityCache {
    pub fn new(capacity: usize, ttl: Duration) -> Self { /* ... */ }

    pub fn get(&self, id: PublisherId) -> Option<PublisherIdentity> {
        let now = Instant::now();
        if let Some(entry) = self.map.get(&id) {
            let (identity, inserted_at) = entry.value().clone();
            if now.duration_since(inserted_at) < self.ttl {
                self.hit_count.fetch_add(1, Ordering::Relaxed);
                return Some(identity);
            }
            // Expired ; drop the read guard before remove.
            drop(entry);
            self.map.remove(&id);
        }
        self.miss_count.fetch_add(1, Ordering::Relaxed);
        None
    }

    pub fn put(&self, id: PublisherId, identity: PublisherIdentity) {
        // LRU cap : if at capacity, evict the oldest.
        if self.map.len() >= self.capacity {
            self.evict_oldest();
        }
        self.map.insert(id, (identity, Instant::now()));
    }

    pub fn evict(&self, id: PublisherId) -> bool {
        self.map.remove(&id).is_some()
    }

    pub fn hit_ratio(&self) -> f64 {
        let h = self.hit_count.load(Ordering::Relaxed);
        let m = self.miss_count.load(Ordering::Relaxed);
        if h + m == 0 { 0.0 } else { (h as f64) / ((h + m) as f64) }
    }

    fn evict_oldest(&self) {
        // Find the oldest insertion time ; remove that key.
        let oldest = self.map.iter()
            .min_by_key(|entry| entry.value().1)
            .map(|entry| entry.key().clone());
        if let Some(k) = oldest {
            self.map.remove(&k);
        }
    }
}
```

### A5 — `N24ComplianceMetrics` Prometheus surfaces

| Metric                          | Type    | Labels                              |
|---------------------------------|---------|-------------------------------------|
| `n24_signature_verified_total`  | counter | `outcome: ok|verify_failed|author_mismatch|publisher_not_found|key_not_found|grace_skip` |
| `n24_gold_bound_total`          | counter | `outcome: ok|not_approved|hash_drift|lookup_failed|no_ref|grace_skip` |
| `n24_status_blocked_total`      | counter | `outcome: ok|active|suspended|revoked|not_publisher` |
| `n24_cache_hit_ratio`           | gauge   | (none)                              |
| `n24_lookup_latency_ms`         | histogram | `op: cache|registry`              |

### A6 — Audit checklist for the EU compliance officer

After Sprint 16 N24 ships, the EU compliance officer should
be able to answer "yes" to every line :

- [ ] Every dispatch through `POST /v1/swarms/run` produces a
      `nano.envelope.v1` event ?
- [ ] For every Tier-1+ catalogue entry, `envelope.signature_
      verified == true` ?
- [ ] For every `TierAudit` catalogue entry, `envelope.gold_
      bound == true` ?
- [ ] For every dispatch, `envelope.outcome` reflects the
      real agent outcome (no false positives) ?
- [ ] When a publisher is revoked via N18 admin, the next
      dispatch fails closed within 60 s ?
- [ ] When a gold set is rewritten silently in the registry,
      the next dispatch fails closed (hash drift) ?
- [ ] When the publisher registry is offline, dispatches
      fail soft (timeout) and never hang the swarm ?
- [ ] The `GET /v1/admin/compliance-status` endpoint shows
      a non-zero `signature_verified_true` count in the last
      hour ?
- [ ] A 90-day-old N16 multi-region replay verifies via the
      historical key chain (I7) ?
- [ ] Every code path that mutates a publisher state evicts
      the runner's cache for that publisher id ?

---

_End of N24 architecture._
