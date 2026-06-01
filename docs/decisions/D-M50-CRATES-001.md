# D-M50-CRATES-001 — 1 vs 2 crates for M50

**Status**       : ACCEPTED — **2 crates**
**Date drafted** : 2026-05-18
**Decision**     : Split M50 into **2 crates** : `efe-nano` (runtime + dispatcher + catalog) and `efe-nano-sdk` (public SDK + macros)
**Owner**        : @bench-infra-lead (Sprint 0 Action 0.5)
**Resolves**     : Verification report D-M50-AUDIT-1 (1 vs 2 crates contradiction)

---

## Decision in one sentence

> **M50 will be split into 2 crates :  `efe-nano` (private runtime) and
> `efe-nano-sdk` (public SDK).  The split is justified by the divergent
> dependency footprints and security boundaries between server runtime
> and customer-facing SDK.**

---

## Why this question matters

The M50 architecture doc had a contradiction (D-M50-AUDIT-1) :
- L19 said "1 nouvelle crate `efe-nano`"
- L2174 said "1 new crate, 1 SDK crate"
- Annex B file layout showed both

Sprint 0 must resolve this **before any Sprint 1 code is written**.

---

## Option analysis

### Option A — Single crate (`efe-nano`)

```
crates/efe-nano/
  ├── src/
  │   ├── lib.rs              public + private
  │   ├── runtime/             internal
  │   ├── dispatcher/          internal
  │   ├── catalog/             internal
  │   ├── audit/                internal
  │   ├── sdk/                  public re-export
  │   └── macros/               public proc-macro
```

**Pros** :
- Simpler workspace structure
- Single version to manage
- Easier internal refactoring (everything in one place)

**Cons** :
- Customer SDK pulls all transitive deps (sqlx, tokio-postgres, redis, etc.)
- Compile time on customer machines : 5-10 minutes (full EFE backend deps)
- Security : a CVE in the runtime forces SDK customers to upgrade
- Versioning : runtime patches force SDK version bumps (semantic conflict)

### Option B — Two crates (`efe-nano` + `efe-nano-sdk`) ★ CHOSEN

```
crates/efe-nano/
  ├── src/
  │   ├── lib.rs               (private — server-side)
  │   ├── runtime/             (Mode A/B/C executors)
  │   ├── dispatcher/          (DAG + parallel)
  │   ├── catalog/             (Postgres-backed)
  │   ├── audit/                (HMAC chain)
  │   └── billing/              (Stripe + Postgres + Redis)
  └── deps : sqlx, postgres, redis, stripe, cosign, ...

crates/efe-nano-sdk/
  ├── src/
  │   ├── lib.rs                (public — customer-facing)
  │   ├── client.rs             (HTTP client to efe-nano backend)
  │   ├── types.rs              (shared types : NanoAgent, SwarmRequest)
  │   └── macros.rs             (proc-macro #[nano_agent])
  └── deps : reqwest, serde, tracing — only HTTP client deps
```

**Pros** :
- Customer SDK compile time : < 30 sec (clean checkout)
- Clean security boundary : SDK can't reach into runtime internals
- Independent versioning :
  - `efe-nano` (runtime) : breaks frequently in development
  - `efe-nano-sdk` (SDK) : stable, semver-strict
- Open source strategy aligns : SDK MIT-licensed, runtime commercial
- Customers can author nano agents without ever touching Postgres / sqlx

**Cons** :
- 2 crates to manage (Cargo.toml, CI)
- Shared types live in `efe-nano-sdk` (re-exported by `efe-nano`)
- One additional cross-crate dependency edge

---

## Decision matrix

| Criterion | Option A (1 crate) | Option B (2 crates) ★ |
|-----------|--------------------|------------------------|
| Customer SDK compile time | 5-10 min | < 30 sec |
| Customer dep footprint | huge (full backend) | minimal (reqwest + serde) |
| Security boundary | weak (shared crate) | strong (process + crate split) |
| Versioning hygiene | conflict-prone | independent semver |
| Open source license consistency | hard (one license for all) | clean (sdk MIT, runtime commercial) |
| Code complexity | simpler workspace | 1 more crate |
| Refactoring friction | low (everything inside) | low (shared types in SDK) |
| Time to scaffold (Sprint 0 D7) | ~ 2 hours | ~ 4 hours |

**Verdict : Option B wins on the criteria that matter (compile time, security, OSS strategy).  Slight complexity overhead is justified.**

---

## Architectural pattern : where do shared types live ?

Shared types between server and SDK :
- `NanoAgent` (catalog entry)
- `NanoAgentRef` (catalog reference)
- `SwarmRequest` / `SwarmResponse`
- `Operation` / `OperationResult`
- `NanoAgentResponse`
- `TraceId`, `CallContext` (subset visible to SDK)
- Error enums (`NanoAgentError`, `SwarmError`)

**Decision** : shared types live in `efe-nano-sdk`.

Rationale :
- SDK is the **public contract**
- Runtime imports SDK : `efe-nano` depends on `efe-nano-sdk`
- Customers writing nano agents only depend on `efe-nano-sdk`
- Backend developers working on runtime can edit shared types via SDK PR

```
Dependency graph :

  customer code         efe-nano-sdk
       │                       ▲
       └─────────────────────┘  (use efe_nano_sdk::nano_agent)
                                
  efe-nano (runtime)            efe-nano-sdk
       │                            ▲
       └────────────────────────────┘  (uses shared types)
```

---

## Cargo workspace integration

Updated `Cargo.toml` workspace section :

```toml
[workspace]
members = [
    # ... existing 29 crates ...
    "crates/efe-nano-sdk",        # NEW (M50 — public SDK)
    "crates/efe-nano",              # NEW (M50 — runtime, depends on -sdk)
]
```

Note : `efe-nano-sdk` listed first because Cargo respects ordering for
display but topologically it's a leaf (no internal EFE deps).

---

## Alternative considered : 3-crate split

What about `efe-nano-types` + `efe-nano-sdk` + `efe-nano-runtime` ?

**Rejected** — 3 crates is over-engineered for M50 scope.  Types living
in SDK is fine because :
- Customers writing nano agents WILL use SDK
- Runtime importing SDK is acceptable (cyclical = no, since types are
  leaf-level)
- 3 crates means 3 Cargo.toml's to maintain

Re-evaluate at M+12 if SDK grows past 5k lines.

---

## Alternative considered : separate proc-macro crate

`efe-nano-macros` could be its own crate (proc-macros traditionally live
in a separate crate from the consuming code).

**Decision** : include macros INSIDE `efe-nano-sdk` for simplicity.
Rust's proc-macro split rule is convention, not requirement.  If
compile-time issues arise later, split is non-breaking.

---

## Impact on roadmap

- Sprint 0 Day 7 : scaffold both crates (~ 4 hours work, OK in 1 day budget)
- Sprint 1+ : both crates evolve in parallel
- Sprint 8 : SDK becomes public (efe-nano-sdk@0.1.0 on crates.io)
- Sprint 13 : SDK @1.0 with stability tier guarantees

---

## Sign-off

- [x] 2-crate split confirmed
- [x] `efe-nano-sdk` as public API + shared types
- [x] `efe-nano` as private runtime
- [x] Cargo workspace ordering decided
- [x] Decision documented for future contributors

**Decision accepted.**

---

_Action 0.5 complete — 2026-05-18._
