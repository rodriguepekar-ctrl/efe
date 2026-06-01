# efe-nano

**Module** : M50 — Nano Agent Swarm Runtime (private, commercial)
**Status** : Scaffold (Sprint 0)
**License** : UNLICENSED (commercial)
**Companion** : `efe-nano-sdk` (public SDK, MIT)

## What this crate is

Private runtime for the EFE M50 Nano Agent Swarm platform.  Implements :

- **Catalog** (Sprint 1-2) : Postgres-backed registry of nano agents
- **Runtime** (Sprints 1, 4, 5) : 3 execution modes (pure Rust, local model, LLM-augmented)
- **Dispatcher** (Sprint 3) : DAG planning + parallel layer execution
- **Audit** (Sprint 6) : per-call HMAC trace extending M23 audit chain
- **Billing** (Sprints 7, 11) : Stripe integration + per-call metering
- **Starter pack** (Sprints 2-5) : 20 EFE-shipped nano agents

## What this crate is NOT

This is **not** customer-facing.  Customers consume `efe-nano-sdk`
(public, MIT) which has minimal deps.  This crate has heavy backend
deps (Postgres, Firecracker, Stripe, etc.).

## Status by sprint

See `docs/roadmap/EFE_M50_ROADMAP.md` § 4-7 for the full sprint plan.

| Sprint | Deliverable in this crate |
|--------|---------------------------|
| 0 | Scaffold + module stubs (this state) |
| 1 | `catalog::*` + `runtime::pure_rust` + 5 Mode A starter agents |
| 2 | Postgres-backed catalog + 5 more Mode A agents |
| 3 | `dispatcher::*` (DAG runner) |
| 4 | `runtime::local_model` + 3 Mode B starter agents |
| 5 | `runtime::llm_augmented` + 2 Mode C + 5 more starter agents |
| 6 | `audit::*` + replay capability |
| 7 | `billing::*` (Stripe + meter) |
| 11-12 | Audit-grade tier hardening |
| 13 | GA release |

## Internal dependencies (planned)

When implementation lands :
- `efe-curation` (M23) — HMAC chain reuse (Sprint 6)
- `efe-agent-runner` (M25) — Firecracker driver reuse (Sprint 4)
- `efe-llm-router` (M21) — LLM call routing (Sprint 5)
- `efe-cache` (M27) — idempotent call caching (Sprint 1+)
- `efe-events` (M28) — event emission (Sprint 6+)
- `efe-pii` (M29) — PII redaction in Mode C (Sprint 5)

All consumed via public APIs — no modification of existing crates.

## License

UNLICENSED (commercial — see workspace `LICENSE`).

The public-facing SDK `efe-nano-sdk` is MIT-licensed.

## See also

- [M50 Architecture](../../docs/architecture/EFE_M50_AGENTIC_ARCHITECTURE.md)
- [M50 Roadmap](../../docs/roadmap/EFE_M50_ROADMAP.md)
- [Decision : 2-crate split](../../docs/decisions/D-M50-CRATES-001.md)
- [Sprint 0 validation report](../../docs/architecture/EFE_M50_SPRINT_0_VALIDATION_REPORT.md)
