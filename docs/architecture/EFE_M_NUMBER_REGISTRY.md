# EFE M-Number Registry — Canonical Mapping

**Status**         : LIVE — authoritative source
**Date drafted**   : 2026-05-18
**Owner**          : @bench-infra-lead (Sprint 0 Action 0.1)
**Methodology**    : grep over lib.rs headers + docs/EFE_M*_ARCHITECTURE.md filenames
**Purpose**        : Resolve verification report D-M50-AUDIT-6 — confirm M-number mappings
                     before M50 architecture doc references them.

> Source of truth : each crate's `src/lib.rs` doc-comment + matching
> `docs/EFE_M*_ARCHITECTURE.md` file.  All entries below have been
> verified live on 2026-05-18.

---

## Authoritative M-number table

| M-# | Title / Function | Crate(s) | Architecture doc |
|-----|------------------|----------|-------------------|
| **M10** | gRPC surface of EFE (server core) | `efe-server` | `M10_ARCHITECTURE.md` (workspace root) |
| M14 | Compliance + docs / freeze 1.0 GA | (docs only, multi-crate) | `EFE_COMPLIANCE_*.md` |
| M15 | (intermediate) | — | — |
| M16 | MCP Server (host integration layer) | `mcp-server/` (Python) | `EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md` |
| M17 | Advanced agent-native tools | (extends M16) | `EFE_M17_ADVANCED_AGENT_TOOLS_ARCHITECTURE.md` |
| M18 | Bundle & deploy tools | (extends M14 build chain) | `EFE_M18_BUNDLE_DEPLOY_TOOLS_ARCHITECTURE.md` |
| M19 | Production hardening | (multi-crate) | `EFE_M19_PRODUCTION_HARDENING_ARCHITECTURE.md` |
| **M20** | **BYO-LLM Agent SDK + LLM-helper MCP tools** | (multi-crate + MCP) | `EFE_M20_BYO_LLM_AGENT_SDK_ARCHITECTURE.md` |
| **M21** | Multi-provider LLM router + prompts (versioned + A/B) | `efe-llm-router`, `efe-prompts` | (in-crate docs) |
| **M22** | **Mode-3 multi-turn iterative reasoning** | (extends `efe-prompts` + `efe-server`) | `EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md` |
| **M23** | LLM-assisted taxonomy curation (+ HMAC audit chain) | `efe-curation` | (in-crate docs) |
| **M24** | Closed-loop active learning | `efe-active-learning` | (in-crate docs) |
| **M25** | Agent runtime — EXPERIMENTAL Firecracker microVM sandbox | `efe-agent-runner` | (in-crate docs) |
| **M26** | Agent Runtime Production Graduation | (extends `efe-agent-runner` + `efe-server`) | `EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` |
| **M27** | Redis-backed cache plane (embedding + LLM + tenant quotas) | `efe-cache` | (in-crate docs) |
| **M28** | Typed event envelope + NATS JetStream pub/sub + webhooks | `efe-events` | (in-crate docs) |
| **M29** | **PII detection + auto-redaction** | `efe-pii` | (in-crate docs) |
| **M30** | Multimodal inference plane + Region awareness + DR | `efe-multimodal`, `efe-region` | (in-crate docs) |
| **M31** | Per-tenant real-time usage metering + Stripe billing | `efe-metering`, `efe-billing` | (in-crate docs) |
| **M32** | Hybrid retrieval (dense vector + sparse BM25) | `efe-retrieval`, `efe-vector` | (in-crate docs) |
| **M33** | Knowledge graph + GraphRAG | `efe-kg`, `efe-graphrag` | `EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md` |
| M34 | (referenced in M33 doc, unimplemented) | — | (forward reference) |
| M35 | (referenced in M33 doc, unimplemented) | — | (forward reference) |
| **M50** | **Nano Agent Swarm (THIS WORKSTREAM)** | `efe-nano`, `efe-nano-sdk` (TBD Sprint 1) | `EFE_M50_AGENTIC_ARCHITECTURE.md` |

---

## Corrections to M50 architecture doc (D-M50-AUDIT-6 resolution)

The M50 architecture doc had **3 M-number mis-references** that this
registry now corrects :

| Mis-reference in M50 doc | Correct M-number | Functional name |
|---------------------------|------------------|-----------------|
| "M14 = Classification API" | M14 = **Compliance + docs** (Classification API is older, pre-M14) | Compliance freeze 1.0 GA |
| "M20 = PII redaction" | **M29** = PII detection + auto-redaction (M20 = BYO-LLM Agent SDK) | PII / M29 |
| "M22 = SLOs" | M22 = **Mode-3 multi-turn iterative reasoning** | (SLOs are not yet a numbered module) |

**Impact** : the M50 doc references should be updated post-Sprint 0
(non-breaking documentation patch) :
- L3 layer description: drop "M22 SLOs" claim
- Mode C uses M21 LLM router (correct) and M22 Mode-3 templates if applicable
- PII redaction in Mode C correctly cited as M29, not M20

These corrections do **not** require any code change.  They are
documentation-only fixes in `docs/architecture/EFE_M50_AGENTIC_ARCHITECTURE.md`.

---

## M-number naming convention

- **M10-M19** : Phase 1 foundational modules (server + API + integrations + tools)
- **M20-M29** : Phase 2 advanced features (LLM ops + audit + active learning + agents)
- **M30-M39** : Phase 3 horizontal scale (multimodal + retrieval + KG + metering)
- **M50-M59** : Phase 4 EFE Agentic (nano agent swarm + B2A platform)
- **M60+** : Future (multi-region scale, federation, etc.)

The gap M34-M49 is intentional : reserved for incremental Phase 3
extensions that may appear between current M33 and M50.

---

## Verification methodology

This registry was built by :

```bash
# Pass 1 : grep all M-numbers from codebase
grep -rhEo "M[0-9]{2}\b" docs/ Cargo.toml crates/*/Cargo.toml crates/*/src/lib.rs | sort -u

# Pass 2 : for each M-#, read the lib.rs header (most authoritative)
head -15 crates/<crate>/src/lib.rs | grep -E "^//!"

# Pass 3 : for each M-# with no crate, find the docs/EFE_M*_ARCHITECTURE.md file
ls docs/EFE_M${m}*.md

# Pass 4 : cross-check with mcp-server/README.md for M16
```

All entries above are traceable to one of these 4 sources.

---

## Reserved M-numbers (going forward)

| M-# | Reserved for | Status |
|-----|--------------|--------|
| M34-M49 | Incremental Phase 3 extensions | Available |
| **M50** | **Nano Agent Swarm** | This M50 workstream |
| M51 | Multi-region nano agent federation | Reserved (post-GA) |
| M52 | Sovereign / air-gapped deployment | Reserved |
| M53 | Nano agent marketplace governance | Reserved |
| M54-M59 | EFE Agentic v2 features | Available |
| M60+ | Future Phase 5 | Available |

---

## Sign-off

- [x] All currently-implemented M-numbers (M10-M33) traced to lib.rs or docs/
- [x] M50 mis-references identified and listed for correction
- [x] Reserved M-numbers documented
- [x] Registry usable as canonical source by Sprint 1 work

**This document supersedes any prior M-number reference in EFE docs.**
**Future docs should cite this registry as canonical source.**

---

_Action 0.1 complete — 2026-05-18._
