# EFE M50 — N18 Publisher Identity Architecture

> **Sprint** : 10 (Phase 3, day 36 – 40 of M50).
> **Status** : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N15_FAIRNESS_SCHEDULING_ARCHITECTURE.md` (Sprint 8, shipped)
> * `EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md` (Sprint 9, shipped)
> * `EFE_M50_N17_LLM_PROVENANCE_ARCHITECTURE.md` (Sprint 9, shipped)
>
> **Anchors** : Sprint 1 catalog (`Author` enum, `NanoAgent` type) — N18
> formalises the publisher identity behind the `Author::Tenant`,
> `Author::Community`, and (NEW) `Author::Publisher` variants.  Sprint
> 6 HMAC audit chain provides the integrity substrate for the publish
> ledger.

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2. [Fifteen incident patterns N18 closes (P1–P15)](#2-fifteen-incident-patterns-n18-closes-p1p15)
3. [Commitments — what ships in Sprint 10 (G1–G15)](#3-commitments--what-ships-in-sprint-10-g1g15)
4. [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5. [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6. [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7. [Component architecture](#7-component-architecture)
8. [Module hierarchy & data model](#8-module-hierarchy--data-model)
9. [API surface (HTTP + internal Rust traits)](#9-api-surface-http--internal-rust-traits)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [N12 / N13 / N14 / N15 / N16 / N17 alignment](#13-n12--n13--n14--n15--n16--n17-alignment)
14. [Sprint 10 ship vs Sprint 11/12 split](#14-sprint-10-ship-vs-sprint-1112-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The problem.**  The Sprint 1 catalog has an `Author` enum with three
opaque variants (`Efe`, `Tenant { tenant_id }`, `Community { name }`)
that name **WHO** published an agent — but provide ZERO of the
operational primitives that production multi-tenant publishing
demands :

- **No cryptographic signature** on published agent versions.  Anyone
  with database write access can publish under any name.
- **No KYC tier** distinguishing a verified enterprise from an
  anonymous community contributor.
- **No revocation mechanism** : if a publisher account is compromised
  and pushes a backdoored update, we have no way to revoke that
  version + the agent stays in dispatch rotation.
- **No publish-action audit chain** : forensic queries like "show me
  every version Publisher X published in Q1" require scanning the
  full catalog table.
- **No cooldown** between publish and rollout : a malicious update
  can land in customer traffic within seconds.
- **No sanctions screening** : a publisher on an OFAC list can sign
  up + publish, exposing EFE to regulatory liability.
- **No publisher revocation cascade** : when revoking a publisher,
  there is no atomic way to mark all their agents as untrusted.

Five concrete incidents documented in the Sprint 9 retro (§ 1.1)
motivate making publisher identity a first-class citizen.

**The fix.**  N18 introduces **`PublisherIdentity`** as a first-class
SDK type backed by a dedicated `PublisherRegistry` + a
`PublishLedger` (HMAC-chained, append-only).  Eight strict
guarantees ship :

1. **Ed25519 signature** on every catalog publish.  The signature
   binds `(agent_id, version, content_hash, publisher_id, KycTier,
   timestamp)` so any tampering is detectable post-fact.
2. **KYC tier ladder** (`Tier0..Tier3`) gates the catalog : Tier 2+
   required for production publishing ; Tier 0 (anonymous) restricted
   to a sandbox catalog.
3. **Publish ledger** : every publish / revoke / rotate / promote
   action is an HMAC-chained trace ; the ledger is itself an audit
   chain (reuses Sprint 6 primitives).
4. **Mandatory cooldown** : 24h delay between publish and rollout
   for Tier 2+ publishers (architecture G5 + I16) — defense against
   key compromise rapid-exploit.
5. **Revocation cascade** : revoking a publisher atomically tombstones
   their published versions ; in-flight dispatch returns
   `OperationErrorKind::PublisherRevoked`.
6. **Sanctions screening hook** : `SanctionsScreener` trait checked
   at registration + every 24h ; failures move publisher to
   `Suspended` status (publish + dispatch blocked).
7. **Backdated revocation** : when key compromise is discovered after
   the fact, the revocation lists the discovery time AND the
   suspected-compromise time ; downstream tools replay traffic in
   that window for anomaly detection.
8. **Cross-cutting with N17** : every Mode C LlmProvenance record
   carries `publisher_id` + `kyc_tier` ; the EU AI Act audit query
   (P1 of N17) becomes a JOIN on publisher identity.

**Six capabilities ship in Sprint 10 N18** :

1. **`PublisherIdentity`** + **`PublisherId`** + **`KycTier`** +
   **`PublisherStatus`** + **`PublisherKey`** SDK types.
2. **`PublisherSignature`** + **`PublishLedgerEntry`** + new audit
   chain trace kinds (`PublisherRegistered`, `AgentPublished`,
   `PublisherKeyRotated`, `PublisherRevoked`, `VersionRevoked`).
3. **`PublisherRegistry`** trait + `InMemoryPublisherRegistry` impl —
   register, lookup, rotate, suspend, revoke.
4. **`PublishLedger`** trait + `InMemoryPublishLedger` impl — HMAC-
   chained ledger of publish actions, with verifier.
5. **`KycProvider`** trait + **`SanctionsScreener`** trait + stub
   implementations (sprint 11 wires real providers).
6. **`SignatureVerifier`** trait + Ed25519 implementation via the
   `ed25519-dalek` crate ; constant-time verification.

**Out of scope (deferred to Sprint 11 N19 / Sprint 12 N20)** :

- Real KYC integration (Persona, Onfido, Sumsub).
- Real sanctions list provider (OFAC + UN + EU Lists API).
- Co-publishing N-of-M signature threshold — format frozen now,
  runtime wiring deferred to Sprint 11.
- Publisher monetization / royalty tracking — Sprint 12 (M31
  billing integration).
- Pseudonymous publisher unmask-on-court-order — Sprint 12 legal
  workflow.

**Backward compatibility.**  N18 changes are strictly additive :

- The existing `Author` enum gains a new variant
  `Author::Publisher { publisher_id, kyc_tier }` (the existing
  `Tenant` / `Community` / `Efe` variants stay valid).
- `NanoAgent` gains an optional `signature: Option<PublisherSignature>`
  field — `None` for pre-N18 agents ; new publishes set `Some`.
- Sprint 6 HMAC chain canonical_row_body is **unchanged**.  The new
  trace kinds attach `publisher_id` via an excluded-from-canonical
  field (same pattern as N14 `swarm_envelope_trace_id` and N17
  `provenance_payload`).

---

## 1. Context, motivation, sprint placement

### 1.1 Five real incidents driving N18

**Incident I-2026-05-25 — Mock typo-squat attempt.**
A red-team exercise registered `summarize_strikt` (misspelled `e→i`)
as an anonymous `Community { name }` agent.  Several Free-tier
tenants ran it before the platform team noticed.  No backdoor was
present (it was a drill) but the post-mortem showed : (a) the
catalog has no namespace separation between trusted and untrusted
publishers, (b) no signature meant we couldn't prove who actually
published, (c) takedown took 3 hours through manual SQL.

**Incident I-2026-05-29 — Backdated key compromise.**
A community publisher's GitHub credentials leaked.  An attacker
pushed three agent updates over 4 days before the publisher
noticed.  Customers running those agents had inputs silently
exfiltrated to a third-party endpoint via a Mode A nano-agent that
called outbound.  We had no signature → couldn't determine the
attack window precisely.  No publisher revocation primitive →
we manually delisted by SQL.

**Incident I-2026-06-01 — Sanctions exposure.**
A new tenant onboarded with display name "ACME Corp" via the
self-service portal.  6 weeks later, OFAC added "ACME Corp Holdings"
to the SDN list.  Compliance team had no automated screening.  EFE
operations team manually scanned the publisher list against the
OFAC CSV — process took 9 person-days.

**Incident I-2026-06-04 — EU regulator AI provenance audit.**
The Irish DPC asked "for the 47 Mode C agents that produced
automated decisions affecting EU citizens in Q1 2026, list the
publisher's legal identity + KYC tier + most-recent sanctions check
timestamp".  We could answer for EFE-published agents (Tier 3 by
default) but not for the 12 customer-published ones.  Manual
investigation : 4 days.

**Incident I-2026-06-08 — Co-publishing intent without infrastructure.**
A Phase 4 partner asked to co-publish an agent under their +
EFE's joint identity ("co-branded enterprise SLA").  Today's
catalog lets a single `Tenant { tenant_id }` claim authorship ;
multi-party signing requires an architectural primitive we don't
have.

### 1.2 Why Sprint 10, immediately after N17

N17 (LLM Provenance) records the **model + prompt + caller context**
of every Mode C call.  But for accountability, we ALSO need to know
**who published the agent** that orchestrated those LLM calls.

Without publisher identity, the chain "EU regulator → trace_id →
agent_id → ???" terminates at a string with no legal weight.

N18 closes that loop : every audit chain row (N6), every envelope
(N14), every provenance record (N17) gains the `publisher_id`
back-reference, and the publisher registry answers the
"who is this entity?" question with cryptographic proof + KYC tier.

### 1.3 Who consumes N18 once it ships

| Consumer | Use case |
|---|---|
| **Compliance team** | OFAC/UN/EU sanctions screening + EU AI Act publisher attribution |
| **Legal team** | Subpoena response — publisher KYC dossier |
| **Security team** | Key compromise incident response (replay window, revocation) |
| **Customer success** | "Who published this agent?" support queries |
| **Marketplace team** | Publisher reputation + tier badges |
| **M31 billing** | Royalty / revenue split between publishers (Sprint 12) |
| **N16 replay** | Replay verifies publisher signature on original version |
| **N17 provenance** | LlmProvenanceRecord carries publisher_id + kyc_tier |
| **M24 GDPR saga** | Publisher tombstone cascade to their published agents |
| **N13 canary** | Tier gate : Tier 2+ required for non-shadow rollout |

### 1.4 The 15 incident-class patterns (preview)

Section 2 enumerates 15 patterns clustered into 5 families :

* **Cryptographic integrity** (P1, P2, P7, P11) — 4 patterns
* **Regulatory & sanctions** (P3, P4, P5, P12) — 4 patterns
* **Marketplace safety** (P6, P9, P13) — 3 patterns
* **Lifecycle management** (P8, P10, P14, P15) — 4 patterns

---

## 2. Fifteen incident patterns N18 closes (P1–P15)

> **Methodology** : every P-row sources from (a) a real incident with
> ticket reference, (b) a regulator-level requirement, or (c) a
> Sprint 9 retro-flagged near-miss.  No speculative rows.

### Family A — Cryptographic integrity

**P1. Catalog backdoor discovered post-deploy.**
A published agent version contains malicious code that exfiltrates
inputs.  Today, we can only confirm "agent X v1.2.3 has the bad
code" — we cannot cryptographically prove WHO pushed it (was it the
legitimate publisher, or a compromised account?).  N18's Ed25519
signature on every publish + an HMAC-chained `PublishLedger`
provides non-repudiable proof.

**P2. Publisher account compromised, attacker pushes update.**
A community publisher's GitHub creds leak ; attacker pushes 3
versions over 4 days (Incident I-2026-05-29).  Without signing :
no audit trail.  With N18 : publisher's hardware key is required
to sign ; an attacker with only password / API token cannot sign.
Forensic timeline : the publish-ledger HMAC chain proves the exact
sequence.

**P7. Publisher repudiates a published version.**
Publisher claims "I never published v1.2.3" — perhaps for legal
reasons (the version contained IP they wish to disown).  Today,
catalog audit logs are mutable.  N18's HMAC-chained ledger makes
the publish action **non-repudiable** : Ed25519 signature + chain
HMAC together prove the publisher's private key was used to sign,
on the recorded timestamp.

**P11. Backdated key compromise — attack window discovery.**
The publisher discovers their key was compromised 2 weeks ago.
We need to (a) revoke all versions signed in that window, (b)
identify customers who ran those versions, (c) replay-validate to
detect anomalies.  N18 provides backdated revocation : the
`RevocationCertificate` carries `suspected_compromise_at` (claimed
historic time) alongside `discovered_at` (real time of revocation
filing) ; the revocation cascade marks ALL versions signed in that
window as untrusted ; N16 replay can re-dispatch over the window
for anomaly detection.

### Family B — Regulatory & sanctions

**P3. Anonymous publisher dispute → court order.**
Community contributor "anon_alice" published an agent that
defamed a public figure.  Court order demands the real identity.
Today, `Community { name: "anon_alice" }` carries zero identity
information.  N18's pseudonymous Tier 0 publishers register with
real KYC info hidden behind a one-way pseudonym ; court-order
unmask procedure exists (Sprint 12 workflow).

**P4. EU regulator audit query.**
The Irish DPC asks (Incident I-2026-06-04) "for these 47 Mode C
agents, list legal publisher identity + KYC tier + last sanctions
check".  N18 answers via the publisher registry's `lookup` +
`status` API ; the kyc_tier field on every trace makes the
JOIN trivial.

**P5. Sanctioned entity discovered post-onboarding.**
"ACME Corp Holdings" gets added to OFAC SDN (Incident I-2026-06-01).
N18 runs `SanctionsScreener::check` on every publisher at
registration AND every 24h ; flagged publishers move to
`Suspended` status — dispatch + publish blocked.  Catalog shows
"Provider unavailable" badge.

**P12. Multi-jurisdictional publisher.**
A publisher operates from a country whose AI regulations require
specific disclaimers.  N18's `jurisdiction: Vec<Iso3166>` field
on the registry record lets the platform inject jurisdiction-
appropriate compliance metadata on every dispatch.

### Family C — Marketplace safety

**P6. Typo-squat agent ID.**
Attacker registers `summarize_strikt` (Incident I-2026-05-25).
N18 introduces a `confusable_alert` lint that rejects new
publishes whose `agent_id` is within edit-distance 1 of any
existing well-established (≥ 1k dispatches/day) agent without
operator override.

**P9. Reseller derivative work.**
Tenant T1 forks an EFE-shipped agent + customises the prompt
template.  N18's `Author::Publisher` variant carries `forked_from:
Option<NanoAgentRef>` so derivative works are tracked.  Original
publisher retains attribution.

**P13. Pseudonymous community contributor.**
A safety researcher wants to publish a "jailbreak detector" agent
under a pseudonym.  N18 supports Tier 0 pseudonymous publishers
with full KYC backing the pseudonym ; their agents are confined to
the `sandbox` catalog scope and require explicit Tier-2+ co-sign
to promote to production.

### Family D — Lifecycle management

**P8. Customer attribution support query.**
A customer asks "which company published the medication-lookup
agent I'm using?".  N18's `display_name + kyc_tier` answer this
in O(1) via the registry.  Today : SQL spelunking.

**P10. Co-publishing dispute.**
Phase 4 partner + EFE want joint authorship (Incident I-2026-06-08).
N18 ships the SDK format frozen NOW : a `CoPublishingPolicy` field
on `NanoAgent` carries `signers: Vec<PublisherId>` +
`threshold: u8`.  Runtime enforcement of the threshold ships
Sprint 11 ; the wire format is frozen now so chain integrity
holds.

**P14. Publisher status change mid-flight.**
A swarm is mid-dispatch (3 of 7 ops complete) when the publisher
gets suspended.  N18 : already-started ops finish ; new ops on
that agent in the same swarm fail with
`OperationErrorKind::PublisherSuspended`.  Trace records the
status snapshot at dispatch time vs at completion.

**P15. Cross-tenant publisher impersonation attempt.**
A Free-tier tenant tries to claim it's the publisher of EFE's
own `summarize_strict` (which is `Author::Efe` → publisher ID
`00000000-0000-0000-0000-000000000000`).  N18 : signature
verification on the publish action requires the actual
`Author::Efe` private key, which only EFE's HSM holds.
Impersonation impossible without the key.

---

## 3. Commitments — what ships in Sprint 10 (G1–G15)

> **Read convention** : `G-N : commitment / impl surface / acceptance test`.

### Identity & integrity

**G1. Ed25519 signature on every catalog publish.**  The signature
covers a canonical SHA-256 hash of `(agent_id, version, agent_body
_canonical_json, publisher_id, kyc_tier_at_publish, timestamp)`.
Sprint 10 ships via `ed25519-dalek` crate.  Acceptance :
`phase_3_n18_publish_signature_verifies`.

**G2. KYC tier ladder gates the catalog.**  Tier 0 → sandbox only ;
Tier 1 (verified email) → community catalog ; Tier 2 (KYC org) →
production catalog ; Tier 3 (insured enterprise) → enterprise
catalog with promotion fast-path.  Acceptance :
`phase_3_n18_kyc_tier_gates_catalog`.

**G3. Publish action recorded in HMAC-chained ledger.**  Every
register / publish / rotate / suspend / revoke is an immutable
chain entry.  Reuses Sprint 6 chain primitives.  Acceptance :
`phase_3_n18_publish_ledger_chain_verifies`.

**G6. Cryptographic non-repudiation.**  The combination of (a)
Ed25519 signature + (b) HMAC chain integrity + (c) registry-
verified public key at publish time is mathematically non-
repudiable.  Acceptance :
`phase_3_n18_signature_proves_publisher_action`.

### Operational safety

**G4. Revocation cascade.**  `PublisherRegistry::revoke(publisher,
reason, suspected_compromise_at)` atomically tombstones every
version published by that publisher in the suspected window.
Future dispatches on those versions return
`OperationErrorKind::PublisherRevoked`.  Acceptance :
`phase_3_n18_revocation_cascades_to_versions`.

**G5. Mandatory 24h cooldown for Tier 2+ rollout.**  A published
version cannot be promoted from `Shadow` / `Canary` to `Stable`
within 24h of publish.  Defense against rapid-exploit on key
compromise.  Acceptance : `phase_3_n18_cooldown_blocks_promotion`.

**G12. Key rotation lifecycle.**  Publishers can rotate keys ; old
key remains valid for **verifying** old signatures (invariant I7).
New publishes after rotation use the new key.  Acceptance :
`phase_3_n18_key_rotation_preserves_old_signatures`.

**G15. Backdated revocation with replay assist.**  Revocation
certificate carries `suspected_compromise_at` ; revocation cascade
walks the publish ledger for the window ; N16 replay can be
triggered against the window to validate.  Acceptance :
`phase_3_n18_backdated_revocation_lists_window`.

### Regulatory & compliance

**G11. Sanctions screening hook.**  `SanctionsScreener::check(
identity) -> SanctionsVerdict` runs at registration + every 24h
via a scheduled job.  `Verdict::Flagged` moves publisher to
`Suspended`.  Acceptance :
`phase_3_n18_sanctions_flag_suspends_publisher`.

**G13. Pseudonymous Tier 0 + reveal-on-court-order.**  Tier 0
publishers register with full KYC backing a pseudonymous public
identity ; the reveal procedure exists as a documented operational
runbook (Sprint 12 codified).  Acceptance :
`phase_3_n18_tier_0_pseudonym_kyc_separation`.

### Marketplace surface

**G7. Publisher metadata on every dispatch trace.**  N6 audit
chain row gains an excluded-from-canonical `publisher_id` field ;
Sprint 6 verify_chain continues to pass.  Acceptance :
`phase_3_n18_dispatch_trace_carries_publisher_id`.

**G14. Publisher tier upgrade ceremony.**  Promotion from Tier N
to Tier N+1 emits an `PublisherTierPromoted` ledger entry with the
KYC dossier reference + operator approvers.  Acceptance :
`phase_3_n18_tier_promotion_recorded`.

### Multi-tenancy & isolation

**G9. Publisher registry is global ; dispatch traces are tenant-scoped.**
A publisher record is visible to all tenants (publishers are a
shared resource) ; per-tenant dispatch traces remain RLS-isolated.
Acceptance : `phase_3_n18_registry_shared_dispatch_tenant_scoped`.

**G10. Query API for publisher reputation + history.**
`GET /v1/publishers/{publisher_id}` returns the public record ;
`GET /v1/publishers/{publisher_id}/history` returns the (paginated)
publish ledger entries for forensic / audit use.  Acceptance :
`phase_3_n18_publisher_history_returns_ledger_entries`.

### Cross-cutting

**G8. Co-publishing N-of-M signature threshold (format frozen).**
`CoPublishingPolicy { signers: Vec<PublisherId>, threshold: u8 }`.
Runtime enforcement deferred to Sprint 11.  Acceptance :
`phase_3_n18_co_publishing_format_round_trip`.

---

## 4. Invariants — non-negotiables (I1–I20)

### Identity & integrity

**I1. Every NanoAgent in catalog (post-N18) has exactly one
PublisherSignature per version.**  Pre-N18 agents have
`signature: None` and are treated as Tier 0 anonymous (architecture
ADR-9).

**I2. Signature verifies against publisher's active public key at
publish time.**  Subsequent key rotations DON'T invalidate old
signatures (architecture I7 below) ; verification uses the key
recorded in the publish-ledger entry.

**I3. Revoked versions remain queryable (forensic) but cannot be
dispatched.**  The catalog `get` returns them with a
`RevocationCertificate` attached ; dispatch fails with
`OperationErrorKind::PublisherRevoked`.

**I4. Publisher registry + ledger are HMAC-chained.**  Sprint 6
chain primitives applied — every registry mutation is a chain row.

**I14. PublisherId is opaque UUID, never reused.**  Even after
revocation, the UUID is retained in the registry forever (forensic
archaeology).  A new publisher (perhaps under similar display name)
gets a different UUID.

**I15. Publish ledger HMAC verifies independently.**  Tools like
`efe-publisher-verify` validate the ledger chain without needing
catalog access.

**I19. Signature verification is constant-time.**  Defense against
timing attacks.  Verified via `ed25519-dalek`'s `verify_strict`
function.

### Key + KYC lifecycle

**I5. Tier upgrade is one-way (no demotion except via revocation).**
Tier 2 publishers never silently regress to Tier 1.  The only
"down" path is revocation → permanent.

**I6. Sanctions check is mandatory at registration + every 24h.**
Skipping is forbidden ; the registry rejects a publish action if
the publisher's last sanctions check is > 26h ago (24h + 2h
grace).

**I7. Key rotation : old key remains valid for verifying old
signatures.**  This is mathematically necessary — if rotation
invalidated old signatures, every rotation would break the chain
of trust on existing agents.

**I8. Co-publish threshold is invariant at agent creation.**
Cannot change mid-life.  A new agent version can introduce a new
threshold but a NEW agent is registered (with a new agent_id).

**I9. Pseudonymous publishers cannot publish Tier 2+ agents.**
Pseudonymous = Tier 0 ; production catalog requires Tier 2+.

**I13. Signature scheme upgrade : new versions stay verifiable.**
When/if Ed25519 is deprecated, the new scheme is added side-by-side
+ old signatures continue to verify forever.

### Lifecycle & cascade

**I10. Publisher display name uniqueness within tier.**  Two Tier
2 publishers cannot share display name "ACME Corp" (avoids
confusion).  Two Tier 0 + Tier 2 publishers CAN share (the tier
badge disambiguates).

**I11. Revocation is append-only, never undone (only superseded).**
A revoked publisher can register a NEW publisher_id post-incident
but the old PublisherId stays in `Revoked` status forever.

**I12. Cross-tenant publisher records ARE shared.**  Publishers are
a global resource ; the registry is NOT per-tenant.  Per-tenant
dispatch traces are still RLS-scoped.

**I17. Revocation cascade is atomic per publisher.**  A
`PublisherRevoked` ledger row + every dependent `VersionRevoked`
row land in the same transaction.

**I18. KYC tier visible on every audit chain trace mentioning the
publisher.**  The N6 trace's `publisher_id` lookup resolves to
the registry record at trace time ; the chain row stores the tier
snapshot (avoids race with later tier changes).

### Cross-cutting

**I16. Mandatory cooldown enforced at rollout layer (not publish).**
The publish action records the version ; the rollout layer (N13)
gates promotion from Shadow → Canary → Stable.  Cooldown is the
time between publish + Stable rollout, not the publish action
itself.

**I20. Publisher metadata accessible via N17 LlmProvenanceRecord.**
Every Mode C provenance row carries `publisher_id`.  The N17 query
API can JOIN against the publisher registry.

---

## 5. Failure modes — what we PROTECT against (F1–F15)

> **Read convention** : `F-N : cause / detection / response / recovery`.

### Cryptographic faults

**F1. Publisher key not found at verify time.**
* *Cause* : registry lookup fails (rare — should always be present).
* *Detection* : `PublisherRegistry::lookup` returns `NotFound`.
* *Response* : reject the dispatch with
  `OperationErrorKind::PublisherKeyMissing` ; emit a
  `PublisherKeyMissing` audit trace ; alert ops.
* *Recovery* : ops investigates ; likely a catalog/registry sync
  bug.

**F2. Signature corrupted.**
* *Cause* : storage corruption, MITM attack, or actual key
  compromise.
* *Detection* : `verify_strict` returns `Err`.
* *Response* : reject ; emit security audit trace ; page on-call.
* *Recovery* : ops investigates immediately (potential active
  attack).

**F13. Old signature scheme verification on new chain.**
* *Cause* : pre-N18 agent dispatched on post-N18 build.
* *Detection* : `agent.signature.is_none()`.
* *Response* : treat as Tier 0 anonymous (architecture ADR-9) ;
  dispatch only if tenant policy allows Tier 0.
* *Recovery* : ops re-publishes the agent under a known publisher.

### KYC + sanctions

**F3. KYC service down at publish time.**
* *Cause* : Persona / Onfide / Sumsub temporarily unavailable.
* *Detection* : `KycProvider::lookup` returns `Backend(...)`.
* *Response* : publish is **rejected** (fail-closed) ; operator
  must retry once KYC service recovers.  Tier 0 sandbox publishes
  bypass this (no KYC required).
* *Recovery* : operator retries.

**F4. Sanctions list update failure.**
* *Cause* : OFAC API down ; UN list parse error.
* *Detection* : 24h scheduled job returns `Err`.
* *Response* : ALL publishers move to `SanctionsCheckStale` flag ;
  publish is blocked for Tier 2+ (defense in depth — better to
  block legitimate publishes for a few hours than to admit a
  sanctioned entity).
* *Recovery* : ops fixes the integration ; flag clears
  automatically.

**F5. Co-publish threshold not met.**
* *Cause* : Tier-2 publisher tries to publish without all required
  signers' signatures.
* *Detection* : signature count < threshold at publish time.
* *Response* : reject publish with `CoPublishThresholdNotMet`.
* *Recovery* : publisher collects missing signatures ; re-publishes.

### Lifecycle

**F6. Publisher revoked mid-publish.**
* *Cause* : security team revokes a publisher while their publish
  is in flight.
* *Detection* : revocation status check at signature verification
  time.
* *Response* : the publish fails ; the publisher's pending publishes
  are listed in the revocation cascade.
* *Recovery* : publisher (post-incident) registers as a new
  publisher_id.

**F14. Cooldown bypass attempt.**
* *Cause* : Publisher (or compromised account) tries to promote a
  version to Stable within 24h of publish.
* *Detection* : `rollout::promote_canary` checks
  `now - published_at < 24h` for Tier 2+ publishers.
* *Response* : reject with `CooldownNotElapsed`.
* *Recovery* : publisher waits the cooldown.  Operator override
  exists for hot fixes via emergency procedure.

**F8. Key rotation race with publish.**
* *Cause* : publisher rotates key concurrently with another publish.
* *Detection* : two valid keys exist briefly ; the publish records
  which key signed.
* *Response* : both keys validate ; the publish-ledger records
  the exact key_id used.
* *Recovery* : none needed (works correctly by design).

**F9. Backdated revocation while dispatching.**
* *Cause* : Revocation marks suspected_compromise_at = T-7d ; an
  in-flight dispatch is using a version signed at T-5d (inside
  the window).
* *Detection* : dispatch's pre-flight checks the version against
  the revocation cascade.
* *Response* : in-flight ops complete (per architecture P14) ; new
  ops on the revoked version fail.
* *Recovery* : N16 replay over the window can be triggered.

### Marketplace safety

**F7. Hash collision in agent ID.**
* *Cause* : 64-character agent_id collision (cosmically rare with
  proper UUID).
* *Detection* : registry's UNIQUE constraint at insertion.
* *Response* : insertion fails ; publisher retries with a different
  ID.
* *Recovery* : automatic (UUID collisions ~ 10⁻¹⁸ chance).

**F10. Pseudonymous publisher attempts Tier 2.**
* *Cause* : Tier 0 publisher tries to publish to production
  catalog.
* *Detection* : tier check at publish.
* *Response* : `TierInsufficient` error.
* *Recovery* : publisher completes KYC to Tier 2.

**F11. Display name collision.**
* *Cause* : Two Tier-2 publishers want display name "Acme Corp".
* *Detection* : registry's tier-scoped uniqueness constraint.
* *Response* : second registration fails with `DisplayNameCollision`.
* *Recovery* : publisher chooses different name ; admin can mediate.

**F12. Cross-tenant publisher impersonation attempt.**
* *Cause* : Tenant B tries to publish under Tenant A's
  publisher_id.
* *Detection* : the publish request must be signed with A's
  private key ; B doesn't have it.
* *Response* : signature verification fails ; rejected.
* *Recovery* : standard signature-mismatch handling.

**F15. Multi-publisher with conflicting signatures.**
* *Cause* : Co-publishing N-of-M ; one signer signs a malicious
  version body, others sign the legitimate one.
* *Detection* : the hash binding for the signatures doesn't match
  between signers.
* *Response* : reject with `CoSignerHashMismatch`.
* *Recovery* : co-publishers re-coordinate on a single canonical
  agent body.

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1. Signature scheme : Ed25519 via `ed25519-dalek`.

**Context.**  Choice of digital signature algorithm for publishes.

**Decision.**  Ed25519 (RFC 8032) via the `ed25519-dalek` Rust
crate (version 2.x).

**Rationale.**
* **Speed** : ~ 50 µs sign + 100 µs verify on x86-64.  Negligible
  on publish path.
* **Compact** : 32-byte public key + 64-byte signature.  Storage-
  friendly for the catalog.
* **Security** : 128-bit security level ; well-studied since 2011 ;
  used by SSH, NaCl, libsodium, Signal Protocol.
* **No collision-flakiness** : `verify_strict` enforces canonical
  encoding (rejects malleable signatures that some Ed25519
  implementations accept).
* **Crate maturity** : `ed25519-dalek` is RustCrypto-managed,
  audited, no `unsafe`.

**Consequences.**
* SDK gains a dep on `ed25519-dalek = "2"` and `rand_core = "0.6"`.
* Per-publisher key is 32 + 32 = 64 bytes (public + private if
  storage allowed).  In production, private keys live in Vault
  Transit, not the SDK.

**Rejected alternatives.**
* RSA-2048 — large signatures (256 bytes), slow.
* ECDSA-P256 — historically had encoding pitfalls.
* BLS12-381 — aggregation features unused for Sprint 10.

---

### ADR-2. Publisher key storage : Vault Transit (prod), in-memory (test).

**Context.**  Where private keys live.

**Decision.**  Production : HashiCorp Vault Transit with per-
publisher signing keys.  Tests : `InMemoryKeyStore` carrying raw
private keys.

**Rationale.**
* **Custody** : Vault Transit keeps the private key in HSM ; clients
  sign via API (private key never leaves Vault).
* **Audit** : every Vault Transit signature is logged in Vault's
  audit log — independent forensic trail.
* **Rotation** : Vault supports versioned keys natively (matches
  I7 — old versions remain verifiable).

**Consequences.**
* Production deployments need Vault.  Dev / staging can use
  in-memory.
* SDK has a `KeyStore` trait abstracting the two.

**Rejected alternative.**  AWS KMS — supports signing but no native
versioned-key chain ; requires per-version aliasing.

---

### ADR-3. Publisher identity format : opaque UUID + immutable display name.

**Context.**  How to represent publisher identity.

**Decision.**  `PublisherId(Uuid)` is opaque ; `display_name` is
mutable (display) but **uniqueness is enforced per tier (I10)**.

**Rationale.**
* **Stability** : UUID never changes even on rebrand.  Forensic
  queries always resolve.
* **Display flexibility** : publishers may rebrand without
  invalidating their cryptographic identity.
* **Tier-scoped uniqueness** : avoids name collisions within the
  same trust tier ; allows community/enterprise tiers to share
  names (the tier badge disambiguates).

**Consequences.**
* Every audit row carries `publisher_id` (UUID) — opaque to
  customers ; resolved to display via the registry query API.
* The publisher portal displays "ACME Corp ⊕ Tier 2" rather than
  the UUID.

---

### ADR-4. Mandatory 24h cooldown before Tier 2+ Stable rollout.

**Context.**  Defense against rapid key-compromise exploit.

**Decision.**  A version published by a Tier 2+ publisher cannot
be promoted from Canary to Stable within 24h of publication.
Operator override exists via emergency procedure.

**Rationale.**
* **Attack window** : if an attacker compromises a publisher key
  and pushes a backdoored version, they typically want it to hit
  customer traffic ASAP.  24h delay gives detection systems time
  to fire.
* **Industry alignment** : Apple Developer Program has a 24h
  review window ; Chrome Web Store has a 24-48h review.

**Consequences.**
* `rollout::promote_canary` checks `now - published_at < 24h` for
  Tier 2+ publishers.
* Tier 1 / Tier 0 publishers bypass cooldown (they're sandbox
  anyway).
* Operator-override path : signed by 2-of-3 EFE platform
  engineers + ticket ID.

---

### ADR-5. Catalog namespace : agent_id is global ; publisher_id is metadata.

**Context.**  How to handle agent ID conflicts.

**Decision.**  `agent_id` is a globally unique identifier (URI-
shaped, e.g. `summarize_strict`).  Two publishers cannot register
the same agent_id ; first-come-first-served + namespacing
suggestions in the publisher portal.

**Rationale.**
* **No forced prefixing** : agent_id remains readable
  (`summarize_strict` vs `pub_acme_corp.summarize_strict`).
* **Conflict resolution** : the publisher portal proactively
  warns about confusables (P6) but doesn't force prefixes.

**Consequences.**
* The catalog's UNIQUE constraint on `agent_id` is global.
* Forking : `summarize_strict` (EFE) → `summarize_strict_acme`
  (community) — publisher chooses a new ID.

**Rejected alternative.**  Mandatory `pub-prefix/agent_id` —
breaks SDK URLs + reduces typo-squat risk only marginally.

---

### ADR-6. Publish ledger : append-only HMAC chain (mirrors Sprint 6).

**Context.**  How to ensure publish action history is non-
repudiable.

**Decision.**  The publish ledger is itself an HMAC-chained audit
log, mirroring Sprint 6's `m50.audit_chain`.  Separate Postgres
table (`publisher.publish_ledger`) ; separate HMAC chain context
keyed by `publisher_id`.

**Rationale.**
* **Non-repudiation** : combined with Ed25519 signatures, the
  chain HMAC + the signature give cryptographic non-repudiation.
* **Independent verification** : `efe-publisher-verify` CLI walks
  the ledger without needing other backends.

**Consequences.**
* `efe-publisher-identity` crate reuses `ChainStore` trait from
  Sprint 6.
* The ledger has its own HMAC key source ; rotation independent
  from the production audit chain.

---

### ADR-7. Co-publishing : N-of-M signature threshold per agent.

**Context.**  Multi-party publisher collaborations.

**Decision.**  `CoPublishingPolicy { signers: Vec<PublisherId>,
threshold: u8 }` — published version must have `threshold`
signatures by signers in the list.  Sprint 10 ships the SDK
format ; Sprint 11 wires runtime enforcement.

**Rationale.**
* **Forward compat** : freezing the wire format now means agents
  published in Sprint 10 with `threshold: 1` will validate against
  Sprint 11's stricter runtime.
* **Trust escalation** : Tier 3 enterprise publishers can require
  multiple signers for high-stakes agents (e.g. medical-decision).

**Consequences.**
* New SDK type `CoPublishingPolicy`.
* `NanoAgent.co_publishing: Option<CoPublishingPolicy>` field
  (None = single-signer).
* Sprint 11 runtime : threshold check at publish + at dispatch.

---

### ADR-8. Pseudonymous community publishers : KYC backs pseudonym.

**Context.**  Community contributors want to publish without
revealing identity.

**Decision.**  Tier 0 publishers register with FULL KYC backing
the pseudonym (legal name + jurisdiction known to EFE ;
pseudonym shown to customers).  Court-order unmask procedure
exists (Sprint 12 codified workflow).

**Rationale.**
* **Liability** : pseudonymous publishers can still be held
  accountable for IP claims, defamation, etc. via court order.
* **Community vibrancy** : safety researchers, students, ML
  enthusiasts can publish without job-search blowback.

**Consequences.**
* Pseudonymous Tier 0 sandbox-only by I9.
* Promotion to Tier 1 requires identity reveal.
* Sprint 12 ships the unmask workflow + legal team integration.

---

### ADR-9. Publisher tier ladder : 4 tiers with strict gates.

**Context.**  How many tiers and what gates them.

**Decision.**
* **Tier 0** : community/anonymous — verified email only ;
  sandbox catalog ; no production rollout.
* **Tier 1** : verified email + phone + 30-day cooling period ;
  community catalog ; canary up to 5 % traffic.
* **Tier 2** : KYC organisation (legal entity verification +
  director identification) ; production catalog ; up to 100 %
  Stable rollout (after cooldown).
* **Tier 3** : Tier 2 + insurance bond (EFE-specified amount) +
  enterprise SLA contract ; promotion fast-path (cooldown
  reducible to 1h with co-sign by 2 EFE engineers).

**Rationale.**
* **Friction matches stakes** : higher trust requires higher
  effort.
* **Industry alignment** : matches app-store + financial KYC
  norms.

**Consequences.**
* 4-variant `KycTier` enum.
* Tier-gate checks on every publish + promote action.

---

### ADR-10. Publisher revocation cascade : atomic per publisher.

**Context.**  When revoking a publisher, what happens to their
versions ?

**Decision.**  Revocation is atomic per publisher : a single
`PublisherRevoked` ledger entry + N `VersionRevoked` entries
(one per affected version) land in the same transaction.

**Rationale.**
* **Consistency** : intermediate state (some versions revoked,
  some not) confuses ops + opens attack windows.
* **Auditability** : every revoked version is enumerable from a
  single ledger query.

**Consequences.**
* `PublisherRegistry::revoke` is atomic.
* Postgres TX wraps the cascade.
* In-flight dispatches honor I3 (in-flight finishes, new fails).

---

## 7. Component architecture

### 7.1 The 8 components — at a glance

```text
            Publisher self-service portal
                       │
                       ▼
    ┌─────────────────────────────────────────────────┐
    │ PublisherRegistry  (global, HMAC-chained)        │
    │  ┌─────────┐  ┌────────────┐  ┌──────────────┐  │
    │  │ KYC     │→ │ Sanctions  │→ │ Tier upgrade │  │
    │  │ provider│  │ screener   │  │ ceremony     │  │
    │  └─────────┘  └────────────┘  └──────────────┘  │
    └─────────────────────────┬───────────────────────┘
                              ▼
    ┌─────────────────────────────────────────────────┐
    │ PublishLedger  (HMAC-chained audit log)         │
    │ - PublisherRegistered                           │
    │ - AgentPublished + signature                    │
    │ - PublisherKeyRotated                           │
    │ - PublisherSuspended                            │
    │ - PublisherRevoked                              │
    │ - VersionRevoked                                │
    │ - PublisherTierPromoted                         │
    └─────────────────────────┬───────────────────────┘
                              ▼
              Catalog (NanoAgent + signature)
                              │
                              ▼ dispatch
              N6 audit trace ← publisher_id metadata
              N17 provenance ← publisher_id + kyc_tier
```

### 7.2 `PublisherIdentity` SDK type

**Lives** : `efe-nano-sdk/src/types/publisher.rs`.

```rust
pub struct PublisherIdentity {
    pub publisher_id: PublisherId,
    pub display_name: String,
    pub kyc_tier: KycTier,
    pub status: PublisherStatus,
    pub jurisdiction: Vec<Iso3166>,
    pub registered_at: SystemTime,
    pub last_sanctions_check_at: Option<SystemTime>,
    pub active_key_id: KeyId,
    pub public_key_pem: String,        // PEM-encoded Ed25519 public key
    pub previous_keys: Vec<HistoricalKey>,
    pub kyc_dossier_reference: Option<String>,  // opaque to SDK ; opaque to customers
    pub revocation: Option<RevocationCertificate>,
    pub forks_of: Option<NanoAgentRef>,
    pub co_publishing_threshold: u8,    // 1 = single-signer
}

pub struct PublisherId(pub Uuid);

pub enum KycTier {
    Tier0,    // verified email only
    Tier1,    // email + phone + 30d cooling
    Tier2,    // KYC organisation
    Tier3,    // KYC + insurance + enterprise SLA
}

pub enum PublisherStatus {
    Active,
    SanctionsCheckStale,
    Suspended { reason: String, suspended_at: SystemTime },
    Revoked,
}

pub struct PublisherSignature {
    pub publisher_id: PublisherId,
    pub key_id: KeyId,
    pub algorithm: SignatureAlgorithm,  // Ed25519 for Sprint 10
    pub signature: Vec<u8>,             // 64 bytes for Ed25519
    pub signed_at: SystemTime,
    pub canonical_hash: Sha256Hash,     // SHA-256 of the canonical body
}

pub struct KeyId(pub Uuid);

pub struct HistoricalKey {
    pub key_id: KeyId,
    pub public_key_pem: String,
    pub valid_from: SystemTime,
    pub valid_until: Option<SystemTime>,  // None for current key
}

pub struct RevocationCertificate {
    pub revoked_at: SystemTime,
    pub suspected_compromise_at: Option<SystemTime>,
    pub reason: RevocationReason,
    pub revoking_operator_id: Uuid,
    pub signature: Vec<u8>,  // signed by EFE platform key
}

pub enum RevocationReason {
    KeyCompromise,
    SanctionsViolation,
    PolicyViolation,
    PublisherRequest,
    Operator { rationale: String },
}

pub enum SignatureAlgorithm {
    Ed25519,
    // future variants here ; old verifications still work via I13
}

pub struct CoPublishingPolicy {
    pub signers: Vec<PublisherId>,
    pub threshold: u8,
}
```

### 7.3 `PublisherRegistry` trait

```rust
#[async_trait]
pub trait PublisherRegistry: Send + Sync + 'static {
    async fn register(&self, candidate: NewPublisherCandidate) -> Result<PublisherIdentity, PublisherError>;
    async fn lookup(&self, publisher_id: PublisherId) -> Result<PublisherIdentity, PublisherError>;
    async fn rotate_key(&self, publisher_id: PublisherId, new_key_pem: String, signature_proof: Vec<u8>) -> Result<KeyId, PublisherError>;
    async fn suspend(&self, publisher_id: PublisherId, reason: String) -> Result<(), PublisherError>;
    async fn revoke(&self, publisher_id: PublisherId, reason: RevocationReason, suspected_compromise_at: Option<SystemTime>) -> Result<RevocationCascade, PublisherError>;
    async fn promote_tier(&self, publisher_id: PublisherId, new_tier: KycTier, operator_approval: Vec<OperatorSignature>) -> Result<(), PublisherError>;
    async fn refresh_sanctions(&self, publisher_id: PublisherId) -> Result<SanctionsVerdict, PublisherError>;
    async fn list_active(&self, tier_floor: KycTier) -> Result<Vec<PublisherIdentity>, PublisherError>;
}

pub struct RevocationCascade {
    pub publisher_id: PublisherId,
    pub revoked_versions: Vec<(NanoAgentId, NanoAgentVersion)>,
    pub revocation_certificate: RevocationCertificate,
    pub replay_window: Option<(SystemTime, SystemTime)>,
}
```

### 7.4 `PublishLedger` trait

```rust
#[async_trait]
pub trait PublishLedger: Send + Sync + 'static {
    async fn append(&self, entry: PublishLedgerEntry) -> Result<TraceId, LedgerError>;
    async fn verify_chain(&self, publisher_id: PublisherId) -> Result<ChainVerifyReport, LedgerError>;
    async fn history(&self, publisher_id: PublisherId, limit: usize) -> Result<Vec<PublishLedgerEntry>, LedgerError>;
}

pub enum PublishLedgerEntry {
    PublisherRegistered { identity: PublisherIdentity },
    AgentPublished { publisher_id: PublisherId, agent_id: NanoAgentId, version: NanoAgentVersion, signature: PublisherSignature, kyc_tier_snapshot: KycTier },
    PublisherKeyRotated { publisher_id: PublisherId, previous_key_id: KeyId, new_key_id: KeyId },
    PublisherSuspended { publisher_id: PublisherId, reason: String },
    PublisherRevoked { publisher_id: PublisherId, certificate: RevocationCertificate },
    VersionRevoked { publisher_id: PublisherId, agent_id: NanoAgentId, version: NanoAgentVersion },
    PublisherTierPromoted { publisher_id: PublisherId, from: KycTier, to: KycTier, approvers: Vec<OperatorSignature> },
}
```

### 7.5 `SignatureVerifier` trait

```rust
#[async_trait]
pub trait SignatureVerifier: Send + Sync + 'static {
    async fn verify(&self, signature: &PublisherSignature, canonical_body: &[u8], publisher_public_key_pem: &str) -> Result<(), SignatureError>;
}

pub struct Ed25519Verifier;
```

### 7.6 `KycProvider` + `SanctionsScreener` traits

```rust
#[async_trait]
pub trait KycProvider: Send + Sync + 'static {
    async fn verify_identity(&self, candidate: &NewPublisherCandidate, target_tier: KycTier) -> Result<KycVerdict, KycError>;
    async fn refresh_status(&self, dossier_reference: &str) -> Result<KycVerdict, KycError>;
}

#[async_trait]
pub trait SanctionsScreener: Send + Sync + 'static {
    async fn check(&self, identity: &PublisherIdentity) -> Result<SanctionsVerdict, SanctionsError>;
}

pub enum KycVerdict {
    Passed { tier_granted: KycTier, dossier_reference: String },
    Pending,
    Failed { reason: String },
}

pub enum SanctionsVerdict {
    Clear,
    Flagged { source: SanctionsList, matched_at: SystemTime },
    Inconclusive,
}

pub enum SanctionsList {
    OfacSdn,
    UnSecurityCouncil,
    EuConsolidated,
    UkOfsi,
    Other(String),
}
```

### 7.7 N18 metrics

Twelve Prometheus surfaces in `n18_*` namespace :

| # | Name | Type | Labels |
|---|---|---|---|
| 1 | `n18_publishers_total` | gauge | `tier`, `status` |
| 2 | `n18_publish_actions_total` | counter | `tier`, `outcome` |
| 3 | `n18_signature_verify_total` | counter | `algorithm`, `outcome` |
| 4 | `n18_revocations_total` | counter | `reason` |
| 5 | `n18_sanctions_checks_total` | counter | `verdict` |
| 6 | `n18_kyc_verifications_total` | counter | `target_tier`, `outcome` |
| 7 | `n18_cooldown_blocks_total` | counter | `tier` |
| 8 | `n18_co_publishing_threshold_misses_total` | counter | (none) |
| 9 | `n18_ledger_chain_verifies_total` | counter | `outcome` |
| 10 | `n18_key_rotations_total` | counter | `algorithm` |
| 11 | `n18_publisher_lookup_latency_seconds` | histogram | (none) |
| 12 | `n18_revocation_cascade_size` | histogram | `reason` |

---

## 8. Module hierarchy & data model

### 8.1 Crate layout

```text
crates/
├── efe-nano-sdk/                  (existing) — SDK
│   └── src/types/
│       ├── publisher.rs            (NEW)
│       └── ...
├── efe-publisher-identity/         (NEW crate)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── registry/
│   │   │   ├── mod.rs              (trait)
│   │   │   ├── memory.rs           (InMemoryPublisherRegistry)
│   │   │   └── postgres.rs         (Sprint 11)
│   │   ├── ledger/
│   │   │   ├── mod.rs              (trait + verifier)
│   │   │   └── memory.rs
│   │   ├── verifier.rs             (Ed25519Verifier)
│   │   ├── kyc/
│   │   │   ├── mod.rs              (trait)
│   │   │   └── stub.rs             (StubKycProvider)
│   │   ├── sanctions/
│   │   │   ├── mod.rs              (trait)
│   │   │   └── stub.rs             (StubSanctionsScreener)
│   │   ├── metrics.rs
│   │   ├── api.rs                  (HTTP query handlers)
│   │   └── canonical.rs            (canonical body hashing)
│   └── Cargo.toml
```

### 8.2 SDK types layout

```rust
pub mod publisher {
    // All types from § 7.2
}

// Re-exports at types::*
pub use publisher::{
    CoPublishingPolicy, Iso3166, KeyId, KycTier, NewPublisherCandidate,
    PublisherId, PublisherIdentity, PublisherSignature, PublisherStatus,
    RevocationCertificate, RevocationReason, SignatureAlgorithm,
};
```

### 8.3 Postgres schema (Sprint 11)

```sql
CREATE SCHEMA publisher;

CREATE TABLE publisher.publisher_registry (
    publisher_id            UUID PRIMARY KEY,
    display_name            TEXT NOT NULL,
    kyc_tier                TEXT NOT NULL,
    status                  TEXT NOT NULL,
    jurisdiction            TEXT[] NOT NULL,
    registered_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_sanctions_check_at TIMESTAMPTZ,
    active_key_id           UUID NOT NULL,
    public_key_pem          TEXT NOT NULL,
    kyc_dossier_reference   TEXT,
    revocation_json         JSONB,
    forks_of_json           JSONB,
    co_publishing_threshold INTEGER NOT NULL DEFAULT 1,
    UNIQUE (display_name, kyc_tier)  -- I10 tier-scoped uniqueness
);

CREATE TABLE publisher.historical_keys (
    publisher_id   UUID NOT NULL REFERENCES publisher.publisher_registry(publisher_id),
    key_id         UUID NOT NULL,
    public_key_pem TEXT NOT NULL,
    valid_from     TIMESTAMPTZ NOT NULL,
    valid_until    TIMESTAMPTZ,
    PRIMARY KEY (publisher_id, key_id)
);

CREATE TABLE publisher.publish_ledger (
    sequence_number   BIGSERIAL PRIMARY KEY,
    trace_id          UUID NOT NULL UNIQUE,
    publisher_id      UUID NOT NULL,
    entry_kind        TEXT NOT NULL,
    canonical_body    BYTEA NOT NULL,
    hmac              BYTEA NOT NULL,
    prev_trace_id     UUID,
    body_json         JSONB NOT NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX ON publisher.publish_ledger (publisher_id, sequence_number);

CREATE TABLE publisher.revocation_cascade (
    publisher_id    UUID NOT NULL,
    agent_id        TEXT NOT NULL,
    version         TEXT NOT NULL,
    revoked_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (publisher_id, agent_id, version)
);
```

---

## 9. API surface (HTTP + internal Rust traits)

### 9.1 HTTP endpoints

```text
POST /v1/publishers
    Body: NewPublisherCandidate (JSON)
    → 201 Created PublisherIdentity
    → 400 (KYC failed)
    → 409 (display name collision)

GET /v1/publishers/{publisher_id}
    → 200 OK PublisherIdentity (public fields only)
    → 404

GET /v1/publishers/{publisher_id}/history
    Query: ?limit=50&offset=0
    → 200 OK [PublishLedgerEntry]
    Auth: optional bearer for tenant scoping of view

POST /v1/publishers/{publisher_id}/rotate-key
    Body: { new_key_pem, signature_proof }
    Auth: bearer with publisher's current private key
    → 200 OK { new_key_id }

POST /v1/publishers/{publisher_id}/revoke
    Body: { reason, suspected_compromise_at }
    Auth: bearer with operator JWT (security:revoke scope)
    → 200 OK RevocationCascade

POST /v1/publishers/{publisher_id}/promote
    Body: { target_tier, approvers: [OperatorSignature, ...] }
    Auth: bearer
    → 200 OK PublisherIdentity (updated)

GET /v1/publishers
    Query: ?tier_floor=tier_2&status=active
    → 200 OK [PublisherIdentity]

POST /v1/publishers/{publisher_id}/refresh-sanctions
    → 200 OK SanctionsVerdict
```

### 9.2 Internal Rust traits

Already enumerated in § 7.3–7.6.

---

## 10. End-to-end workflows

### 10.1 Workflow A — Normal publish

```
1. Publisher portal: operator drafts a new agent.
2. POST /v1/agents (with publisher_id + version) ; the catalog
   service computes canonical body hash + sends to publisher portal.
3. Publisher portal: caller signs the hash with their hardware key
   (Vault Transit Sign).
4. POST /v1/agents/{id}/{version}/sign — signature attached.
5. Publish service:
   a. SignatureVerifier.verify(signature, body, public_key) ;
   b. PublisherRegistry.lookup ; check status = Active ;
   c. Check sanctions check ≤ 26h ;
   d. KYC tier OK for target catalog ;
   e. PublishLedger.append(AgentPublished entry) ;
   f. Catalog INSERT.
6. Tier 2+ : 24h cooldown applies before promote.
```

### 10.2 Workflow B — Backdated revocation

```
1. Publisher: "My key was compromised 7 days ago".
2. POST /v1/publishers/{id}/revoke with suspected_compromise_at = T-7d.
3. PublisherRegistry.revoke:
   a. Walk publish_ledger for entries in window ;
   b. Build VersionRevoked entries ;
   c. Append PublisherRevoked + N×VersionRevoked in single TX ;
   d. Append revocation cascade record ;
   e. Catalog dispatch checks updated.
4. Return RevocationCascade with replay_window suggestion.
5. Operator triggers N16 replay over window to detect anomalies.
```

### 10.3 Workflow C — Key rotation

```
1. Publisher rotates Vault Transit key.
2. POST /v1/publishers/{id}/rotate-key with new_key_pem + signature
   of new_key_pem using old key.
3. PublisherRegistry:
   a. Verify signature_proof with old key.
   b. Append PublisherKeyRotated ledger entry.
   c. Move old key to historical_keys (valid_until = now).
   d. Set active_key_id = new key.
4. Old signatures continue verifying via historical_keys lookup.
```

### 10.4 Workflow D — Sanctions screening 24h cycle

```
1. Scheduled job (every hour, sweeps publishers whose
   last_sanctions_check_at > 23h ago).
2. For each: SanctionsScreener.check(identity).
3. Flagged → PublisherRegistry.suspend(reason="Sanctions match").
4. Cleared → update last_sanctions_check_at = now.
5. Sanctions service down → all publishers marked
   SanctionsCheckStale ; alert ops.
```

### 10.5 Workflow E — EU AI Act audit query

```
1. Compliance operator queries N17 LlmProvenanceQueryApi with
   model + window.
2. Result includes publisher_id per record.
3. Compliance operator JOINs against PublisherRegistry.lookup
   for each unique publisher_id.
4. Output table: model + publisher_legal_name + kyc_tier +
   last_sanctions_check.
```

---

## 11. Five-day implementation plan

### Day 1 — SDK types
* `efe-nano-sdk/src/types/publisher.rs` (~700 LOC).
* `Author::Publisher { publisher_id, kyc_tier }` variant added.
* `NanoAgent.signature: Option<PublisherSignature>` field added.
* `OperationErrorKind::PublisherRevoked` + `PublisherSuspended` +
  `PublisherKeyMissing` + `CooldownNotElapsed` + `TierInsufficient`
  + `SignatureMismatch` variants.
* 60+ unit tests.

### Day 2 — Crate skeleton + Registry + Ed25519 verifier
* `efe-publisher-identity` new crate ; workspace member.
* `PublisherRegistry` trait + `InMemoryPublisherRegistry` impl.
* `SignatureVerifier` trait + `Ed25519Verifier` impl using
  `ed25519-dalek`.
* `Iso3166` SDK helper.
* 50+ unit tests.

### Day 3 — Publish ledger + revocation cascade
* `PublishLedger` trait + `InMemoryPublishLedger` impl with HMAC
  chain (reuses Sprint 6 primitives).
* `RevocationCascade` flow : registry + ledger atomic transaction
  (Sprint 11 Postgres ; Sprint 10 in-memory).
* `efe-publisher-verify` CLI stub.

### Day 4 — KYC + sanctions hooks + cooldown enforcement
* `KycProvider` trait + `StubKycProvider`.
* `SanctionsScreener` trait + `StubSanctionsScreener`.
* `CooldownEnforcer` wrapper integrating with N13 rollout.
* Operator approval signature scheme.

### Day 5 — Integration tests + metrics + final verification
* 15+ acceptance tests (`phase_3_n18_*`).
* `N18Metrics` Prometheus surface (12 surfaces).
* Cross-cutting tests with N16 + N17.
* Clippy strict + workspace build.

---

## 12. Test strategy

### 12.1 Acceptance test matrix (15 tests, one per G)

```
phase_3_n18_publish_signature_verifies
phase_3_n18_kyc_tier_gates_catalog
phase_3_n18_publish_ledger_chain_verifies
phase_3_n18_revocation_cascades_to_versions
phase_3_n18_cooldown_blocks_promotion
phase_3_n18_signature_proves_publisher_action
phase_3_n18_dispatch_trace_carries_publisher_id
phase_3_n18_co_publishing_format_round_trip
phase_3_n18_registry_shared_dispatch_tenant_scoped
phase_3_n18_publisher_history_returns_ledger_entries
phase_3_n18_sanctions_flag_suspends_publisher
phase_3_n18_key_rotation_preserves_old_signatures
phase_3_n18_tier_0_pseudonym_kyc_separation
phase_3_n18_tier_promotion_recorded
phase_3_n18_backdated_revocation_lists_window
```

### 12.2 Property tests

* `proptest_signature_round_trip` — sign → verify always succeeds.
* `proptest_canonical_body_stable` — same struct fields → same
  canonical hash.
* `proptest_ledger_chain_extends` — every append produces a valid
  chain.

### 12.3 Chaos tests

* `chaos_sanctions_service_down` — F4 enforcement.
* `chaos_revocation_mid_dispatch` — F6 + P14 behaviour.
* `chaos_concurrent_key_rotations` — F8 race resolution.

### 12.4 Security tests

* `security_signature_malleability_rejected` — Ed25519
  verify_strict rejects malleable sigs.
* `security_constant_time_verification` — no timing leak
  (verified by `ed25519-dalek` design + test that runtimes are
  within 10% across malformed inputs).

---

## 13. N12 / N13 / N14 / N15 / N16 / N17 alignment

| Subsystem | Concern | Resolution |
|---|---|---|
| **N12 cache** | Cache key must scope by publisher | Cache key includes publisher_id + version |
| **N13 canary** | Rollout cooldown | N13 promote_canary checks N18 cooldown |
| **N14 envelope** | Envelope needs publisher reference | Envelope `publisher_id` field (excluded from canonical) |
| **N15 fairness** | Tier-based scheduling | publisher tier → quota tier mapping |
| **N16 replay** | Replay verifies signature on original | Replay engine queries publisher registry |
| **N17 provenance** | Mode C record carries publisher_id | LlmProvenanceRecord.publisher_id field |

---

## 14. Sprint 10 ship vs Sprint 11/12 split

### Sprint 10 (this design)
* All 6 capabilities from § 0.
* 15 acceptance tests.
* In-memory registry + ledger.
* Stub KYC + sanctions.

### Sprint 11 N19 — Postgres backing + co-publishing runtime
* PostgresPublisherRegistry + PostgresPublishLedger with RLS.
* Co-publishing N-of-M signature verification at publish.

### Sprint 12 N20 — Real KYC + sanctions + reveal workflow
* Persona / Onfido integration.
* OFAC + UN + EU sanctions API.
* Pseudonymous reveal-on-court-order legal workflow.

---

## 15. Annexes

### A1 — KYC tier requirements

| Tier | Requirements | Catalog access | Cooldown |
|---|---|---|---|
| Tier 0 | Verified email | Sandbox | N/A (no Stable promotion) |
| Tier 1 | + Phone + 30d cooling | Community | 24h |
| Tier 2 | + Legal entity KYC | Production | 24h |
| Tier 3 | + Insurance bond | Enterprise + fast-path | 1h (with 2-of-3 EFE sig) |

### A2 — Canonical body hash spec

```
SHA-256 over CBOR-encoded :
{
  "agent_id": <string>,
  "version": <string>,
  "publisher_id": <bytes-16>,
  "kyc_tier": <string>,
  "agent_body": <bytes>,    // CBOR-encoded NanoAgent (minus signature field)
  "timestamp": <int64>
}
```

### A3 — Revocation reason taxonomy

| Reason | Sanctions match | KYC fail | Operator action |
|---|---|---|---|
| KeyCompromise | No | No | Yes |
| SanctionsViolation | Yes | No | Auto |
| PolicyViolation | No | No | Yes |
| PublisherRequest | No | No | Self-service |
| Operator | No | No | Yes |

### A4 — Operator runbook

#### A4.1 Trigger a revocation

```bash
efe-publishers revoke <publisher_id> \
  --reason KeyCompromise \
  --suspected-compromise-at 2026-05-20T00:00:00Z \
  --rationale "JIRA-SEC-1234"
```

#### A4.2 Verify the publish ledger

```bash
efe-publisher-verify ledger <publisher_id>
```

#### A4.3 Run sanctions check manually

```bash
efe-publishers sanctions-check <publisher_id>
```

### A5 — Comparison with related systems

| System | Their approach | N18's improvement |
|---|---|---|
| npm | Email-verified publisher + 2FA | + Ed25519 signature on every publish, mandatory cooldown |
| Apple App Store | Manual review, code-signing | + Open audit trail, KYC tier ladder |
| crates.io | Owner-based authorization | + Tier-based ladder, sanctions screening |
| Hugging Face | Org / user namespacing | + Cryptographic non-repudiation, revocation cascade |

---

**End of N18 design.**

**Author** : EFE M50 architecture team.
**Date** : 2026-05-21.
**Sprint** : 10 (Phase 3, day 36 – 40).
**Design lineage** : N11 + N12 + N13 + N14 + N15 + N16 + N17 → N18
(this doc) → N19 Postgres + co-publishing → N20 KYC integration.

**Last-mile checklist** (Sprint 10 exit) :
- [ ] All 15 acceptance tests GREEN.
- [ ] Clippy strict (`-D warnings`) on new crate.
- [ ] Pre-N18 catalog entries still load (signature: None backward compat).
- [ ] Ed25519 sign + verify benchmarked at ≤ 200 µs combined.
- [ ] Operator runbook A4 reviewed by security team.
- [ ] Compliance team sign-off on tier ladder + sanctions hook.
- [ ] PagerDuty integration for F4 (sanctions down).
- [ ] CLI tools `efe-publishers` + `efe-publisher-verify` shipped.
