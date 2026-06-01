# EFE M50 — Nano Agent Swarm — Roadmap

**Status**           : DRAFT v1
**Date drafted**     : 2026-05-18
**Owner**            : Rodrigue Pekar (Founding Engineer)
**Companion docs**   :
- [`docs/architecture/EFE_M50_AGENTIC_ARCHITECTURE.md`](../architecture/EFE_M50_AGENTIC_ARCHITECTURE.md) (2,219 lignes)
- [`docs/architecture/EFE_M50_VERIFICATION_REPORT_2026-05-18.md`](../architecture/EFE_M50_VERIFICATION_REPORT_2026-05-18.md) (383 lignes, 15 défauts → 9 fixés)
- [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md) (cible : § 30 ajouté en Sprint 13)
- [`docs/marketing/EFE-demo-EXP6-agent-2026-05-18.md`](../marketing/EFE-demo-EXP6-agent-2026-05-18.md) (PoC wrapper)

**Pre-condition**    : pre-seed round closed (Kovant lead OU alternative), capital débloqué
**Target close**     : December 2026 — GA launch
**Total duration**   : ~ 6.5 mois, 14 sprints × 2 weeks
**Team scaling**     : 1 → 4 personnes (founder + 3 hires)
**Budget envelope**  : CHF 500 k - 1 M (from pre-seed round)

> **Contrat additif STRICT** : aucun sprint ne modifie un fichier source
> existant des 33 crates EFE actuels.  Tous les sprints sont **purement
> additifs** (nouvelles crates, nouveaux tests, nouvelles sections doc).
> Les 174 tests PR #27 scope + 51 STABILITY freeze tests restent verts
> en continu, vérifiés par CI à chaque PR.

---

## Table des matières

0. Executive summary (la roadmap en 1 page)
1. Pre-conditions (qu'est-ce qui doit être vrai avant Sprint 0)
2. Phase decomposition (5 phases sur 14 sprints)
3. Sprint 0 — Validation & Bootstrap (Phase 0)
4. Sprints 1-3 — Foundation (Phase 1)
5. Sprints 4-6 — Audit + Modes B/C (Phase 2)
6. Sprints 7-10 — SDK + Distribution (Phase 3)
7. Sprints 11-13 — Hardening + GA (Phase 4)
8. Resource plan & hiring sequence
9. Budget allocation
10. Validation gates entre phases
11. Risk register (per-sprint + global)
12. Dependencies sur EFE existant (L1/L2/L3)
13. Parallel tracks (sales / customer dev / marketing)
14. Failure modes & contingency plans
15. Success metrics
16. Decision points (go/no-go gates)
17. Integration avec broader EFE roadmap
18. Post-M+6 vision
19. Communication plan (LP updates, customer comms)
20. Annexe A — Gantt visualization
21. Annexe B — Sprint 0 deep dive (le sprint critique)
22. Annexe C — Capability matrix per sprint
23. Annexe D — Decisions log

---

## 0. Executive summary

La roadmap EFE M50 en 1 page :

```
═════════════════════════════════════════════════════════════════════
  EFE M50 — Nano Agent Swarm — Roadmap
  Pre-condition  : pre-seed round closed (T = 0)
  Final state    : GA launch + first paying customers + Series A path
═════════════════════════════════════════════════════════════════════

  PHASE 0  : VALIDATION & BOOTSTRAP                      Sprint 0
             (2 weeks)
             Solo founder
             Output : Sprint 0 validation report + scaffold crates

  PHASE 1  : FOUNDATION                                  Sprints 1-3
             (6 weeks)
             Founder + 1 senior platform engineer
             Output : NAR Mode A + dispatcher v0
                      + 10 starter nano agents (Mode A)

  PHASE 2  : AUDIT + MODES B/C                           Sprints 4-6
             (6 weeks)
             Founder + 2 senior platform engineers
             Output : NAR Mode B/C + Audit Trace + Replay capability

  PHASE 3  : SDK + DISTRIBUTION                          Sprints 7-10
             (8 weeks)
             Founder + 2 engineers + 1 dev advocate
             Output : MCP bridge + Rust/Py/TS SDK + Public Beta launch

  PHASE 4  : HARDENING + GA                              Sprints 11-13
             (6 weeks)
             Same team
             Output : Audit-grade tier + GA launch + Series A pitch

═════════════════════════════════════════════════════════════════════

  Validation gates entre phases :
    Phase 0 → 1  : Sprint 0 report PASS + first hire signed
    Phase 1 → 2  : NAR Mode A throughput 1k calls/sec/node demonstrated
    Phase 2 → 3  : Replay test pass on 1k synthetic swarms
    Phase 3 → 4  : Public Beta has 10+ active users
    Phase 4 → GA : 1 paying customer + SOC 2 Type I scope kickoff

═════════════════════════════════════════════════════════════════════

  18-month outcomes after Sprint 0 start :
    M+6  GA launch  +  free tier open  +  1 design partner signed
    M+12 GA scaling +  3 design partners + 1-2 enterprise + EUR 400k ARR
    M+18 Series A   +  $1.5-3M ARR run-rate + 3 enterprise contracts
═════════════════════════════════════════════════════════════════════
```

---

## 1. Pre-conditions (avant Sprint 0)

```
[ ] Pre-seed round closed
    - Kovant lead investment OR alternative funding secured
    - CHF 500k-1M wired to operating account
    - Term sheet executed, board structured

[ ] Legal entity ready
    - Swiss SA / SARL incorporated in Vaud (or wherever decided)
    - Bank account opened, sourced
    - Standard contracts ready (employment, NDA, MSA template)

[ ] Compliance scaffold
    - DPO designated (or counsel engaged)
    - SOC 2 readiness assessment kickoff
    - Privacy notice + Terms of Service drafted

[ ] Founder readiness
    - Rodrigue 100 % focused on EFE (no other commitments)
    - 6.5 months runway commitment confirmed
    - Personal life setup (no major personal projects competing)

[ ] Hire pipeline
    - At least 2 candidate senior platform engineers in pipeline
    - Compensation strategy decided (CHF + equity split)
    - Offer letter template ready

[ ] Existing EFE infra accessible
    - Production EFE deployment running (Postgres, Cilium, etc.)
    - Access to L1/L2/L3 APIs documented
    - CI/CD pipeline ready for M50 new crate
```

**Gate avant Sprint 0** : tous les items ci-dessus cochés.  Sinon
Sprint 0 délayé jusqu'à ce que tous les pre-conditions soient OK.

---

## 2. Phase decomposition (5 phases sur 14 sprints)

| Phase | Sprints | Weeks | Team | Codename | Output |
|-------|---------|-------|------|----------|--------|
| **Phase 0** | 0 | 1-2 | 1 | "Foundations" | Validation report, scaffold |
| **Phase 1** | 1-3 | 3-8 | 2 | "Mode A" | Pure Rust runtime + dispatcher v0 + 10 agents |
| **Phase 2** | 4-6 | 9-14 | 3 | "Audit" | Mode B/C + audit trace + replay |
| **Phase 3** | 7-10 | 15-22 | 4 | "Distribution" | MCP bridge + SDK + Public Beta |
| **Phase 4** | 11-13 | 23-28 | 4 | "Audit-grade" | Hardening + GA + Series A pitch |

**Total** : 14 sprints × 2 semaines = 28 semaines = ~ 6.5 mois.

**Pourquoi 5 phases et pas 14 sprints à plat** :
- Chaque phase est un **incrément observable** par investors / customers
- Chaque transition de phase est un **decision point** (go / no-go / pivot)
- Permet de **faire un pause** entre phases si nécessaire (recruiting, feedback customer, etc.)

---

## 3. Sprint 0 — Validation & Bootstrap (Phase 0)

**Codename** : Foundations
**Dates** : Jun 1-15, 2026
**Team** : Rodrigue (solo)
**Goal** : Valider les hypothèses architecturales **avant d'écrire du code**

### 3.1 Les 7 actions Sprint 0 (issues de la verification report)

#### Action 0.1 — Valider mapping M-numbers existants

**Pourquoi** : la verification report (D-M50-AUDIT-6) flagger M14, M16, M20, M22 référencés sans grep match dans Cargo.toml.

**Tâche** :
- Auditer `docs/` pour confirmer que M14 = classification API, M16 = MCP server, M20 = PII, M22 = SLOs
- Écrire `docs/architecture/EFE_M_NUMBER_REGISTRY.md` (canonical registry)
- Confirmer les références M50 vers ces modules sont valides

**Deliverable** : `EFE_M_NUMBER_REGISTRY.md` — table M-number → crate(s) → docs

**Durée estimée** : 1 jour

#### Action 0.2 — Inventaire exhaustif MCP tools existants

**Pourquoi** : verification report (D-M50-AUDIT-8) flagger "M16 expose 6 tools" mais réalité ≥ 10.

**Tâche** :
- `grep -r "@register_tool\|@tool" mcp-server/mcp_server/` → liste exhaustive
- Documenter chaque tool : nom, input schema, output schema, version
- Identifier les conflits potentiels avec les nouveaux tools nano agents

**Deliverable** : `mcp-server/MCP_TOOLS_INVENTORY_2026-06.md`

**Durée estimée** : 1 jour

#### Action 0.3 — Décision infra ClickHouse vs Postgres-only

**Pourquoi** : verification report (D-M50-AUDIT-14) flagger ClickHouse comme assumption.

**Tâche** :
- Benchmark : si volume metering < 100M events/month, Postgres OLAP suffit ?
- Estimation coût ClickHouse Cloud vs self-hosted vs Postgres-only
- Décision write-up dans `docs/decisions/D-M50-INFRA-001.md`

**Deliverable** : décision technique signée + cost projection

**Durée estimée** : 2 jours

#### Action 0.4 — Valider pipeline Firecracker

**Pourquoi** : verification report (D-M50-AUDIT-7) flagger Firecracker dep absente.

**Tâche** :
- Confirmer mécanisme actuel dans `efe-agent-runner` (binary spawn vs Rust crate)
- Setup local Firecracker test environment
- POC : lancer 1 microVM avec un nano agent Mode B simple

**Deliverable** : POC Firecracker fonctionnel + runbook

**Durée estimée** : 3 jours

#### Action 0.5 — Décider 1 crate vs 2 crates

**Pourquoi** : verification report (D-M50-AUDIT-1) — actuellement 2 crates planifiées (efe-nano + efe-nano-sdk).

**Tâche** :
- Analyser dependency tree : un SDK doit-il être séparé du runtime ?
- Customer SDK doit pouvoir être consommé sans pulling toute la runtime crate
- Décision : 2 crates confirmées (déclaration finale dans architecture doc)

**Deliverable** : décision write-up + Cargo.toml workspace updated

**Durée estimée** : 0.5 jour

#### Action 0.6 — Scaffold les 2 nouvelles crates

**Tâche** :
```
crates/efe-nano/
  ├── Cargo.toml (workspace member)
  ├── src/lib.rs (empty stubs)
  └── README.md

crates/efe-nano-sdk/
  ├── Cargo.toml
  ├── src/lib.rs
  └── README.md
```
- Empty stubs avec doc-comments
- CI integration : `cargo check` et `cargo test` ajoutés au workflow
- Vérifier que les 174 PR #27 tests + 51 STABILITY freeze restent verts

**Deliverable** : 2 crates scaffold + CI green

**Durée estimée** : 1 jour

#### Action 0.7 — Sprint 0 validation report

**Tâche** :
- Document final synthétisant les 6 actions précédentes
- Identifier les blockers découverts
- Update architecture doc M50 si nécessaire (annotations Sprint 0 results)

**Deliverable** : `docs/architecture/EFE_M50_SPRINT_0_VALIDATION_REPORT.md`

**Durée estimée** : 1 jour

### 3.2 Sprint 0 exit gate

```
[ ] Tous les 7 actions complétées
[ ] Validation report signed off (Rodrigue)
[ ] 174 PR #27 tests + 51 STABILITY freeze tests toujours verts
[ ] 2 nouvelles crates scaffold compilées et intégrées CI
[ ] First senior platform engineer hire signed
[ ] Aucun blocker bloquant Phase 1
```

Si tous cochés → Phase 1 démarre Sprint 1 le Jun 16.
Si pas tous cochés → Sprint 0 prolongé d'une semaine, re-évaluation.

### 3.3 Sprint 0 risk register

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Hiring delay (no senior engineer signs) | Medium | High | Activate backup candidates, consider contract hire |
| ClickHouse decision unclear | Medium | Medium | Default to Postgres-only, defer ClickHouse to Phase 3 |
| Firecracker POC blocking | Low | High | Fall back to lighter sandbox (Wasmtime, V8 isolates) |
| 174 tests break during scaffolding | Low | Critical | Strict additivity check on every commit |

---

## 4. Sprints 1-3 — Foundation (Phase 1)

**Codename** : Mode A
**Dates** : Jun 16 - Aug 10, 2026
**Team** : Rodrigue + Senior Platform Engineer #1
**Goal** : NAR Mode A runtime + dispatcher v0 + 10 starter nano agents

### 4.1 Sprint 1 — Catalog + Mode A runtime

**Dates** : Jun 16-30
**Team** : Rodrigue + Senior #1

**Deliverables** :
```
crates/efe-nano/src/
├── catalog/
│   ├── mod.rs           NanoAgent struct + traits
│   ├── types.rs          ExecutionMode, DeterminismLevel, etc.
│   └── store.rs          In-memory store (Postgres backend in Sprint 2)
├── runtime/
│   ├── mod.rs            Runtime trait
│   └── pure_rust.rs      Mode A executor
```

**Tests added** :
- `crates/efe-nano/tests/catalog_test.rs` : ~ 15 tests
- `crates/efe-nano/tests/runtime_mode_a_test.rs` : ~ 10 tests

**Acceptance criteria** :
- [ ] Catalog peut register/get un nano agent
- [ ] Mode A executor peut exécuter un nano agent simple (JSON validate)
- [ ] Schema validation in/out fonctionne
- [ ] Tests : 25+ new tests, all PASS
- [ ] 174 PR #27 tests still PASS
- [ ] 51 STABILITY freeze tests still PASS

### 4.2 Sprint 2 — 5 first starter nano agents (Mode A)

**Dates** : Jul 1-15
**Team** : Rodrigue + Senior #1

**Deliverables — 5 nano agents implémentés** :
```
crates/efe-nano/src/starter_pack/
├── json_validate.rs        Mode A — validate JSON against schema
├── xml_validate.rs         Mode A — validate XML against XSD
├── regex_extract.rs         Mode A — run regex, return matches
├── date_normalize.rs        Mode A — parse date, output ISO 8601
└── currency_normalize.rs   Mode A — parse currency string
```

**Plus** :
- Catalog backed by Postgres (sqlx integration)
- Versioning system (semver)
- Per-tenant private catalog

**Tests** : ~ 25 nouveaux tests (5 par nano agent)

**Acceptance criteria** :
- [ ] 5 nano agents Mode A passent leurs contract tests
- [ ] Postgres catalog stores + retrieves nano agents
- [ ] Versioning : un nano agent peut avoir 2 versions, latest is preferred
- [ ] Throughput Mode A : ≥ 500 calls/sec/node (single thread baseline)

### 4.3 Sprint 3 — Dispatcher v0 + 5 more nano agents

**Dates** : Jul 16-30
**Team** : Rodrigue + Senior #1

**Deliverables** :
```
crates/efe-nano/src/dispatcher/
├── mod.rs               Dispatcher trait + entry point
├── dag.rs                DAG planner (topological sort + cycle detect)
├── executor.rs           Parallel layer execution
└── types.rs              SwarmRequest, SwarmResponse, Operation

crates/efe-nano/src/starter_pack/
├── pdf_extract_text.rs     Mode A
├── html_extract_main.rs    Mode A
├── markdown_extract.rs     Mode A
├── csv_normalize.rs         Mode A
└── sql_parse_safe.rs       Mode A
```

**Tests** : ~ 30 nouveaux tests (dispatcher DAG + 5 nano agents)

**Acceptance criteria** :
- [ ] Dispatcher accepte SwarmRequest avec 10+ operations + DAG
- [ ] DAG cycle detection works
- [ ] Layer-by-layer parallel execution
- [ ] All 10 Mode A nano agents pass contract tests
- [ ] Throughput target Mode A : ≥ 1000 calls/sec/node (gate Phase 1 → 2)

### 4.4 Phase 1 exit gate

```
[ ] 10 nano agents Mode A implementés et testés
[ ] Catalog Postgres-backed et versionné
[ ] Dispatcher v0 fonctionnel (DAG + parallel execution)
[ ] Throughput Mode A : 1k calls/sec/node demonstrated (live bench)
[ ] ~ 80 new tests added (catalog + runtime + dispatcher + 10 agents)
[ ] 174 PR #27 + 51 STABILITY toujours verts
[ ] Senior Platform Engineer #2 hire signed (pour Phase 2 démarrage)
```

---

## 5. Sprints 4-6 — Audit + Modes B/C (Phase 2)

**Codename** : Audit
**Dates** : Aug 11 - Sep 28, 2026
**Team** : Rodrigue + Senior #1 + Senior #2
**Goal** : Mode B/C runtimes + audit trace + replay capability

### 5.1 Sprint 4 — Mode B runtime (local model)

**Dates** : Aug 11-25
**Team** : Rodrigue + Senior #1 + Senior #2

**Deliverables** :
```
crates/efe-nano/src/runtime/
├── local_model.rs           Mode B executor
├── pool.rs                  Firecracker microVM pool
└── model_loader.rs           ML model loading + caching

crates/efe-nano/src/starter_pack/
├── language_detect.rs        Mode B — language detection (99 langs)
├── sentiment_classify.rs     Mode B — pos/neg/neutral
└── ner_generic.rs            Mode B — PERSON/ORG/LOC/DATE/MONEY
```

**Plus** : intégration avec `efe-agent-runner` (M25) pour Firecracker

**Tests** : ~ 25 tests (runtime Mode B + 3 nano agents)

**Acceptance criteria** :
- [ ] Mode B executor lance microVM avec modèle pré-loaded
- [ ] Pool : 20 microVMs warm at startup, dynamic scaling
- [ ] Pool hit rate ≥ 95 % at moderate load
- [ ] 3 starter nano agents Mode B fonctionnels
- [ ] Throughput Mode B : ≥ 50 calls/sec/node (single model)

### 5.2 Sprint 5 — Mode C runtime + 2 LLM-augmented agents

**Dates** : Aug 26 - Sep 9
**Team** : full team

**Deliverables** :
```
crates/efe-nano/src/runtime/
├── llm_augmented.rs           Mode C executor
└── output_validator.rs       Strict JSON schema enforcement on LLM output

crates/efe-nano/src/starter_pack/
├── summarize_strict.rs        Mode C — LLM summary with length cap
└── extract_structured.rs       Mode C — LLM extraction with JSON schema

crates/efe-nano/src/starter_pack/
├── email_extract_body.rs      Mode A — parse email RFC 5322
├── docx_extract_text.rs        Mode A — extract from .docx
├── intent_classify_generic.rs Mode B — generic intent classification
├── ner_clinical.rs              Mode B — DRUG/DISEASE/GENE/PHASE
└── ner_legal.rs                Mode B — CASE/CITATION/PARTY/CLAUSE
```

**Plus** : intégration avec `efe-llm-router` (M21) pour LLM calls

**Tests** : ~ 35 tests (runtime Mode C + 7 nano agents)

**Acceptance criteria** :
- [ ] Mode C uses M21 LLM router (pas de duplication)
- [ ] Cost cap pre-check + post-charge fonctionnel
- [ ] 2 LLM-augmented nano agents passent leurs contracts
- [ ] 5 more Mode A/B starter agents (15 total au sprint 5)
- [ ] Throughput Mode C : ≥ 5 calls/sec/node

### 5.3 Sprint 6 — Audit Trace + Replay

**Dates** : Sep 10-24
**Team** : full team

**Deliverables** :
```
crates/efe-nano/src/audit.rs
  - Extension du HMAC chain M23
  - Postgres schema : nano_agent_traces table
  - Per-call trace recording

crates/efe-nano/src/replay.rs
  - Historical swarm replay capability
  - Determinism check per nano agent
  - HMAC chain verification

Postgres migration :
  - nano_agent_traces table
  - Indexes : (tenant_id, started_at), (swarm_request_id), (nano_agent_id)
```

**Tests** : ~ 30 tests (audit trace + replay + chain integrity)

**Acceptance criteria** :
- [ ] Every nano agent call generates a trace entry
- [ ] HMAC chain linking trace → audit_chain (M23)
- [ ] Replay of 1k synthetic swarms : 100 % determinism match
- [ ] Trace retention policy enforced per tier
- [ ] Storage : < 1 KB per trace average

### 5.4 Phase 2 exit gate

```
[ ] Mode A + Mode B + Mode C runtimes operational
[ ] 15 starter nano agents (5 A + 5 B + 5 C-or-A) implémentés
[ ] Audit trace : per-call trace recorded with HMAC chain
[ ] Replay capability : 1k synthetic swarms replay 100 % match
[ ] Throughput per mode : 1k Mode A, 50 Mode B, 5 Mode C
[ ] Pool hit rate ≥ 95 % at moderate load
[ ] 174 PR #27 + 51 STABILITY toujours verts
[ ] ~ 180 new tests added cumulative (gate to Phase 3)
```

---

## 6. Sprints 7-10 — SDK + Distribution (Phase 3)

**Codename** : Distribution
**Dates** : Sep 29 - Nov 23, 2026
**Team** : Rodrigue + Senior #1 + Senior #2 + Dev Advocate
**Goal** : MCP bridge + Rust/Python/TypeScript SDK + Public Beta

### 6.1 Sprint 7 — MCP Bridge Extension

**Dates** : Sep 29 - Oct 13
**Team** : full team + Senior #2 leading

**Deliverables** :
```
mcp-server/mcp_server/tools/nano_agents.py
  - Generic dispatcher tool : efe_nano_swarm
  - Auto-generated per-nano-agent tools (from catalog)
  - Per-tenant tool visibility

mcp-server/tests/
  - test_nano_swarm_tool.py
  - test_per_agent_tools.py
  - test_tenant_isolation.py
```

**Plus** : 5 more starter nano agents (20 total)

**Tests** : ~ 25 MCP integration tests + 25 last starter nano agent tests

**Acceptance criteria** :
- [ ] MCP server expose 20+ nano agent tools auto-discovered
- [ ] efe_nano_swarm tool works end-to-end via Claude Desktop
- [ ] Per-tenant tool visibility enforced
- [ ] Existing M16 MCP tools (≥ 10 existing) **inchangés**
- [ ] First 20 starter pack complete

### 6.2 Sprint 8 — Rust SDK + CLI

**Dates** : Oct 14-28
**Team** : full team

**Deliverables** :
```
crates/efe-nano-sdk/
├── src/lib.rs
├── src/macros.rs              proc-macro #[nano_agent]
└── src/runtime.rs              client-side runtime

cli/efe-nano/
├── src/main.rs
└── src/commands/
    ├── init.rs                 efe-nano init
    ├── test.rs                  efe-nano test
    └── publish.rs               efe-nano publish
```

**Tests** : ~ 30 tests (SDK + CLI integration)

**Acceptance criteria** :
- [ ] proc-macro #[nano_agent] generates valid catalog manifest
- [ ] CLI : `efe-nano init my-agent` creates a working scaffold
- [ ] CLI : `efe-nano test` runs local nano agent against schema
- [ ] CLI : `efe-nano publish` uploads to catalog (per-tenant)

### 6.3 Sprint 9 — Documentation + MCP Marketplace

**Dates** : Oct 29 - Nov 11
**Team** : Dev Advocate leading + Rodrigue review

**Deliverables** :
- Public documentation site : `efe-nano.com`
  - Getting started guide (5 min)
  - 20 starter nano agents reference docs
  - SDK reference (Rust, complete)
  - Architecture overview
  - Compliance / audit guide
- MCP Marketplace submission package
- 3-5 blog posts (technical deep dives)

**Acceptance criteria** :
- [ ] Documentation site live with all 20 nano agents documented
- [ ] MCP Marketplace submission accepted
- [ ] At least 5 external developers signed up (free tier waitlist)

### 6.4 Sprint 10 — Python SDK + Performance hardening

**Dates** : Nov 12-25
**Team** : full team

**Deliverables** :
```
sdk-python/
├── pyproject.toml
├── efe_nano/
│   ├── __init__.py
│   ├── decorator.py            @nano_agent decorator (Python)
│   ├── client.py                Python client (calls EFE backend)
│   └── runtime.py               pyo3 bindings to Rust core

Performance hardening :
- Pool warm-up optimization (target 99.5 % hit rate)
- Cost cap pre-check optimization (sub-1 ms)
- Audit chain write batching (target 5k writes/sec)
```

**Tests** : ~ 35 tests (Python SDK + perf benchmarks)

**Acceptance criteria** :
- [ ] Python SDK installable via `pip install efe-nano`
- [ ] Python @nano_agent decorator works for Mode A agents
- [ ] Performance targets met for Phase 4 gate

### 6.5 Phase 3 exit gate

```
[ ] MCP bridge live, ≥ 20 tools exposed
[ ] Rust SDK + Python SDK shipped
[ ] CLI : init/test/publish commands work
[ ] Documentation site live (efe-nano.com)
[ ] Public Beta opens : free tier signup live
[ ] ≥ 10 external developers signed up (active users)
[ ] Throughput per mode targets met (Phase 2 +20 %)
[ ] ~ 280 new tests added cumulative
[ ] 174 PR #27 + 51 STABILITY toujours verts
```

---

## 7. Sprints 11-13 — Hardening + GA (Phase 4)

**Codename** : Audit-grade
**Dates** : Nov 24 - Dec 31, 2026
**Team** : full team (4 personnes)
**Goal** : Audit-grade tier + GA launch + Series A pitch

### 7.1 Sprint 11 — TypeScript SDK + Stripe billing

**Dates** : Nov 24 - Dec 7
**Team** : full team

**Deliverables** :
```
sdk-ts/
├── package.json
├── src/
│   ├── index.ts                @nano_agent decorator (TypeScript)
│   ├── client.ts               TS client
│   └── runtime.ts              napi bindings to Rust core

crates/efe-nano/src/billing/
├── mod.rs
├── meter.rs                    Per-call counter
├── budget.rs                   Real-time cost cap
└── stripe.rs                   Stripe integration

Free tier rate limiter activated.
```

**Tests** : ~ 30 tests (TS SDK + billing flow)

**Acceptance criteria** :
- [ ] TypeScript SDK installable via `npm install @efe/nano`
- [ ] Stripe customer creation + invoice generation works
- [ ] Free tier : rate limit 1k calls/month enforced
- [ ] Developer tier : 1M calls/month + $0.001 overage billed correctly

### 7.2 Sprint 12 — Audit-grade tier hardening

**Dates** : Dec 8-21
**Team** : full team

**Deliverables** :
```
crates/efe-nano/src/audit.rs (extended)
├── cosign_signer.rs              Cosign signing of trace bundles
└── seven_year_retention.rs       Long-term audit retention

Compliance docs :
- DPIA template for nano agents
- AI Act Article 9 attestation template
- FINMA / FDA mapping document
```

**Tests** : ~ 25 tests (audit-grade tier features)

**Acceptance criteria** :
- [ ] Cosign signing applied to audit trace bundles daily
- [ ] Rekor inclusion proof recorded
- [ ] 7-year retention infrastructure provisioned
- [ ] DPIA + AI Act attestation flow operational
- [ ] First Audit-grade tier customer onboarding documented

### 7.3 Sprint 13 — GA launch + STABILITY § 30 + Series A pitch

**Dates** : Dec 22-31
**Team** : full team

**Deliverables** :
- GA launch sequence :
  - Public announcement (blog post + press release)
  - Conversion funnel from Free → Developer → Pro activated
  - First 3 design partners onboarding kickoff
- STABILITY contract § 30 added (bump v1.7 → v1.8)
  - 51 v17 freeze tests + 30 new v18 freeze tests = 81 total
  - Schema gel for catalog + swarm request + audit trace
- Kovant case study (with permission)
- Series A pitch deck (separate from pre-seed deck)
  - $10-15 M raise at $30-50 M post-money valuation
  - 12-18 month go-to-Series-A plan

**Acceptance criteria** :
- [ ] GA launched : signup form live, billing operational
- [ ] STABILITY v1.8 published, 81 freeze tests pass
- [ ] First paying customer (Developer or Pro tier) onboarded
- [ ] Series A deck reviewed by 3 advisors
- [ ] Kovant case study draft submitted for permission

### 7.4 Phase 4 exit gate (= GA gate)

```
[ ] GA launch executed (signup live, billing live)
[ ] First paying customer onboarded
[ ] Audit-grade tier infrastructure operational
[ ] STABILITY contract bumped v1.7 → v1.8 with § 30
[ ] 81 freeze tests pass (51 v17 + 30 v18)
[ ] Series A deck reviewed by 3 advisors
[ ] SOC 2 Type I prep kicked off
[ ] ~ 385 total tests pass (174 PR #27 + 51 v17 + 30 v18 + 130 M50)
```

---

## 8. Resource plan & hiring sequence

### 8.1 Team scaling timeline

```
Sprint 0   (Jun 1-15)    Solo : Rodrigue
Sprint 1   (Jun 16-30)   +1 : Senior Platform Engineer #1
                                    Hire signed in Sprint 0, starts Sprint 1
Sprint 4   (Aug 11-25)   +1 : Senior Platform Engineer #2
                                    Hire process Sprints 2-3, starts Sprint 4
Sprint 9   (Oct 29-Nov 11)  +1 : Dev Advocate / Tech Writer
                                    Hire process Sprints 7-8, starts Sprint 9

Final team M+6 : 4 people
```

### 8.2 Hiring criteria

**Senior Platform Engineer #1 (joins Sprint 1)** :
- 5+ years Rust experience
- Distributed systems background
- Comfortable with async tokio
- Production Postgres / sqlx experience
- Bonus : Firecracker / microVM experience

**Senior Platform Engineer #2 (joins Sprint 4)** :
- Either ML systems engineer (for Mode B model serving)
- OR security/compliance specialist (for audit chain + Firecracker)
- Cosign / Sigstore experience valued

**Dev Advocate (joins Sprint 9)** :
- Technical writing strong
- Open source community experience
- Polyglot : Rust + Python + TS
- Speaker / blogger background

### 8.3 Compensation strategy (Swiss SARL)

```
Founder (Rodrigue) :
  Cash    : CHF 80-100 k/y (minimum viable)
  Equity  : ~ 75 % post-pre-seed
  
Senior Engineer #1 :
  Cash    : CHF 130-150 k/y
  Equity  : 1.5-2.5 % stock options (4-year vest, 1-year cliff)
  
Senior Engineer #2 :
  Cash    : CHF 130-150 k/y
  Equity  : 1.0-1.5 %
  
Dev Advocate :
  Cash    : CHF 100-130 k/y
  Equity  : 0.5-1.0 %
```

Note : equity numbers indicatifs, à confirmer avec advisor + tax.

### 8.4 Hire pipeline plan

| Month | Activity |
|-------|----------|
| Sprint 0 (Jun) | Finalize Senior #1 offer, sign before Sprint 1 |
| Sprints 1-2 | Activate Senior #2 search (LinkedIn, EPFL alumni, etc.) |
| Sprint 3 | Final interviews Senior #2, offer letter |
| Sprint 4 | Senior #2 onboarding |
| Sprints 7-8 | Dev Advocate search + interviews |
| Sprint 9 | Dev Advocate onboarding |

---

## 9. Budget allocation

### 9.1 18-month budget envelope (from pre-seed CHF 500k-1M)

Assuming **CHF 750k middle of range** raised :

```
Engineering (50 %) = CHF 375 k
  Senior #1   : CHF 140 k × 18 mois / 12 = CHF 210 k
  Senior #2   : CHF 140 k × 15 mois / 12 = CHF 175 k
  → over-allocated, reduce duration or salaries

Compute & infra (15 %) = CHF 112 k
  - Postgres managed : ~ 200 USD/mois × 18 = CHF 3.6 k
  - ClickHouse cloud : ~ 500 USD/mois × 18 = CHF 9 k (if needed)
  - Compute (Firecracker hosts) : ~ 1500 USD/mois × 18 = CHF 27 k
  - LLM provider costs (POC + dev) : ~ 1000 USD/mois × 18 = CHF 18 k
  - Misc cloud (S3, networking) : ~ 500 USD/mois × 18 = CHF 9 k
  - Domain, SaaS tools, etc. : ~ 3 k

Sales & marketing (15 %) = CHF 112 k
  - Conferences / events : CHF 30 k
  - Marketing site + design : CHF 20 k
  - Travel (Kovant + design partners) : CHF 20 k
  - SEO + content : CHF 15 k
  - Misc (LinkedIn ads, etc.) : CHF 15 k
  - PR / launch : CHF 12 k

Compliance & legal (10 %) = CHF 75 k
  - SOC 2 Type I prep : CHF 30 k
  - Legal counsel (contracts, IP) : CHF 25 k
  - DPO outsourced : CHF 10 k
  - Misc compliance tooling : CHF 10 k

Founder compensation (8 %) = CHF 60 k
  CHF 5 k / mois × 12 mois (year 2 reviewed)

Buffer / contingency (2 %) = CHF 15 k
```

**Total** : CHF 750 k allocated.  Tight but workable.

### 9.2 Sensitivity analysis

**If raise is CHF 500 k** (low end) :
- Cut Senior #2 duration (12 mois instead of 15)
- Defer Dev Advocate to month 12 instead of month 9
- Compute budget down by 50 % (more aggressive cost management)
- Founder salary CHF 60 k instead of 100 k

**If raise is CHF 1 M** (high end) :
- Senior #2 starts 1 sprint earlier
- Dev Advocate full-time from Sprint 7
- Compute budget +30 % (faster scaling)
- Bridge to Series A with more runway

---

## 10. Validation gates entre phases

```
                       Pre-condition           Output / Exit criteria
                       ─────────────            ────────────────────────

Phase 0 → 1 gate       Pre-seed closed         Sprint 0 report PASS
(end Jun)              Solo founder ready      Senior #1 hire signed
                                                174 + 51 tests green

Phase 1 → 2 gate       Phase 1 complete         NAR Mode A throughput
(early Aug)            Senior #2 hire signed   1k calls/sec/node
                                                10 nano agents live
                                                ~ 80 new tests added

Phase 2 → 3 gate       Phase 2 complete         Mode B + C operational
(end Sep)              Audit replay validated  Replay 1k swarms = 100% match
                                                15 nano agents live
                                                ~ 180 new tests added

Phase 3 → 4 gate       Public Beta live         MCP marketplace live
(end Nov)              Dev Advocate hired      10+ active developers
                                                Documentation site live
                                                Stripe billing operational

Phase 4 → GA gate      Audit-grade ready        STABILITY v1.8 published
(end Dec)              First customer onboard  GA launched
                                                Series A deck ready
                                                ~ 385 total tests
```

**Tous les gates sont "go/no-go" decisions** :
- Si gate failed → 1-2 weeks de catch-up, re-evaluate
- Si catch-up fails → strategic pivot discussion with board

---

## 11. Risk register

### 11.1 Critical risks (impact = severe)

| Risk | Probability | Mitigation |
|------|-------------|------------|
| **Senior hire delay** (Sprint 0-3) | Medium | Pipeline of 5+ candidates, contract option, reduce burn |
| **Tech stack pivot needed** (e.g. Firecracker → Wasmtime) | Low | POC in Sprint 0 minimizes this |
| **Kovant invest doesn't close** | Medium | Bridge with Innosuisse + Venture Kick + family funds |
| **Compliance gate failure** (DPIA, AI Act) | Low | Compliance counsel engaged Sprint 0 |
| **174 tests broken by M50 work** | Low | Strict additivity, CI green check on every PR |

### 11.2 Moderate risks

| Risk | Probability | Mitigation |
|------|-------------|------------|
| Throughput targets missed Phase 1/2 | Medium | Buffer Sprint 10 for perf hardening |
| Customer adoption slower than expected Phase 3 | Medium | Free tier loss-leader strategy, MCP marketplace push |
| Senior engineer attrition | Low | Competitive comp + equity, transparent comms |
| LLM provider cost overrun | Medium | Cost cap enforced from Sprint 4 (Mode C launch) |
| Postgres scaling pain | Medium | Plan migration to ClickHouse in Phase 3 if needed |

### 11.3 Low risks

| Risk | Probability | Mitigation |
|------|-------------|------------|
| Cosign / Rekor outage during Audit tier launch | Very low | Local signing fallback |
| STABILITY v1.8 schema breaking change leak | Very low | 30 new freeze tests catch this |
| MCP Marketplace rejection | Low | Engage Anthropic team early |

---

## 12. Dependencies sur EFE existant

```
M50 dépend de (mais ne modifie pas) :

  L1 :
    M14 — Classification API           (no direct use, indirect via lib)
    M16 — MCP Server                   (extended additively in Sprint 7)
    M20 — PII Redaction                (used by Mode C nano agents)
    M21 — LLM Router                   (used by Mode C runtime in Sprint 5)
    M21 — Prompts                      (used by Mode C templates)
    
  L2 :
    M14 — Meta-swarm controller        (used by dispatcher for Mode B/C)
    M14 — Meta-swarm participant       (used as nano agent host in Mode B/C)
    M25 — Agent Runner (Firecracker)   (used by Mode B/C runtime)
    
  L3 :
    M22 — SLOs                         (used to emit SLI metrics)
    M23 — HMAC audit chain             (extended via FK in Sprint 6)
    M23 — Curation                     (used for LLM-augmented agents)
    M24 — Active learning              (consumes M50 events, no dep on M50)
    M27 — Cache                        (used by idempotent nano agents)
    M28 — Events                       (M50 emits events here)
    
  Infrastructure :
    Postgres                            (catalog + traces + billing)
    sqlx                                (Rust DB client)
    Stripe                              (billing, feature flag already exists)
    cosign / Rekor                      (audit-grade signing in Sprint 12)
    ClickHouse                          (Sprint 0 decision)

Vérification Sprint 0 :
  Pour chaque dependency, confirmer :
    - Production deployment running
    - API contract documented
    - Access credentials for M50 team
    - Capacity / cost projections OK
```

---

## 13. Parallel tracks (sales / customer dev / marketing)

L'engineering n'est pas tout.  En parallèle :

```
Phase 0 (Jun)
  Sales        : Contacter 5-10 prospects design partner (Roche, Pictet, Kudelski, etc.)
  Marketing    : Refresh marketing site, prepare launch sequence

Phase 1 (Jul-Aug)
  Sales        : 2-3 design partner pitch meetings
  Marketing    : Blog post "M50 architecture" published
  Customer dev : First validation interviews (do customers actually want this ?)

Phase 2 (Sep)
  Sales        : 1 design partner LOI signed (target)
  Marketing    : MCP Marketplace pre-announcement
  Customer dev : 5+ validation interviews complete

Phase 3 (Oct-Nov)
  Sales        : 1 design partner contract signed
                  Pipeline 3-5 more design partners
  Marketing    : Documentation site launch
                  Blog series (5 posts)
                  Conference talk submission
  Customer dev : Free tier waitlist (target 100+ signups)

Phase 4 (Dec)
  Sales        : 1-2 enterprise pipeline opened
                  First paying customer signed
  Marketing    : GA launch press
                  Kovant case study
  Customer dev : Series A preview meetings (5 funds)
```

---

## 14. Failure modes & contingency plans

### 14.1 Failure : Throughput Mode A < 1k calls/sec/node at Phase 1 gate

**Probability** : 25 %
**Diagnosis** : Rust async overhead, allocator pressure, or tokio scheduling
**Mitigation** :
- Profile with `perf` / `flamegraph`
- Consider crossbeam-channel for hot paths
- If still under, extend Phase 1 by 1 sprint (Sprint 4 becomes "Phase 1 hardening")

### 14.2 Failure : Customer adoption < 10 active users at Phase 3 gate

**Probability** : 35 %
**Diagnosis** : Marketing reach, MCP Marketplace timing, product-market fit
**Mitigation** :
- Direct outreach to 50 Anthropic / OpenAI users on Twitter
- Conference talk at Anthropic Build Day (Oct 2026)
- Free tier expansion : 10k calls/month instead of 1k
- Reduce Developer tier price to $49 instead of $99
- If still under, extend Phase 3 by 1 sprint, defer GA to mid-January

### 14.3 Failure : First paying customer not onboarded by GA gate

**Probability** : 40 %
**Diagnosis** : Sales cycle too long, customer not converted, technical blockers
**Mitigation** :
- GA can launch without first paying customer (acceptable)
- Adjust narrative : "GA launched, first customer in pipeline"
- Push aggressive Q1 2027 sales calendar

### 14.4 Failure : Tests broken during a sprint

**Probability** : 10 %
**Diagnosis** : Strict additivity violated (most likely D-S3-PN-style audit defect)
**Mitigation** :
- CI gates every PR : 174 PR #27 + 51 STABILITY must be green
- Revert the offending PR
- Audit-driven fix (cf S3 audit pattern PR #27)
- Add to risk register for next sprint

### 14.5 Failure : Founder burnout

**Probability** : 25 % (real risk for solo founder)
**Diagnosis** : Sustained 60+ hour weeks, sole decision-maker stress
**Mitigation** :
- Mandatory 2-week vacation after Phase 2 gate (October)
- Therapist / coach engagement throughout
- Senior #2 hire is a force multiplier for decision sharing
- 1-on-1 with board member every 2 weeks

---

## 15. Success metrics

### 15.1 Engineering metrics (per sprint)

```
- Tests added per sprint                    : target ≥ 15
- Code review turnaround                    : target < 24h
- CI green rate                              : target 100 %
- Sprint completion rate (commitments met) : target ≥ 80 %
- Defects found in audit per sprint         : target 5-15 (audit-driven culture)
```

### 15.2 Product metrics (per phase)

```
Phase 1 end :
  - 10 starter nano agents shipped
  - Throughput Mode A : 1k calls/sec/node
  
Phase 2 end :
  - 15 starter nano agents shipped
  - 3 execution modes operational
  - 1k swarms replay : 100 % determinism match
  
Phase 3 end :
  - 20 starter nano agents shipped
  - 3 SDK languages
  - 10+ active developers (free tier)
  
Phase 4 end (GA) :
  - 1 paying customer
  - SOC 2 prep kicked off
  - STABILITY v1.8 published
```

### 15.3 Business metrics (cumulative)

```
M+6  (GA)     : 1 paying customer, 50 signups
M+9           : 3 design partners signed
M+12          : 1 enterprise contract, CHF 400 k ARR run-rate
M+18          : 2-3 enterprise contracts, CHF 1.5-3 M ARR run-rate
                 Series A conversation opened
```

### 15.4 Compliance metrics

```
Phase 2 end   : DPIA template ready
Phase 3 end   : AI Act Article 9 attestation framework
Phase 4 end   : SOC 2 Type I scope kickoff
Post-GA       : SOC 2 Type I report (M+10 target)
                 ISO 27001 readiness assessment (M+12)
                 FINMA-aligned audit-grade tier launched (M+13)
```

---

## 16. Decision points (go/no-go gates)

Beyond inter-phase gates, 3 strategic decision points :

### 16.1 Decision Point A — End Phase 1 (~ Aug 10)

**Question** : Is M50 throughput / latency on track ?

```
GO    : Mode A throughput 1k calls/sec/node demonstrated
NO-GO : < 700 calls/sec/node → 1 sprint of hardening, then re-eval
PIVOT : If fundamental issue, consider Wasmtime instead of Firecracker
```

### 16.2 Decision Point B — Mid Phase 3 (~ Nov 1)

**Question** : Is customer adoption viable ?

```
GO    : 5+ design partners pipeline + Public Beta signups > 50
NO-GO : Less than 2 design partners + Beta < 20 → reduce Marketing budget,
        consider $0 free tier (vs $99 developer), extend Phase 3 by 2 sprints
PIVOT : If product-market fit signal weak, consider B2B-only positioning
        (no developer tier, enterprise only)
```

### 16.3 Decision Point C — Pre-GA (~ Dec 15)

**Question** : Are we ready to launch GA ?

```
GO    : All Phase 4 gates met + 1 paying customer signed (or LOI signed)
NO-GO : Delay GA by 2-4 weeks, focus on signing first customer
PIVOT : If no customer signing, "soft GA" : free tier remains, no paid tier yet
```

---

## 17. Integration avec broader EFE roadmap

EFE has other ongoing initiatives.  M50 must coordinate :

```
M50 timeline    :  Jun 2026 → Dec 2026
                   (this roadmap)

Coordinated parallel work :

PR #28 (Phase 3 next gap)   :  Sep-Oct 2026
  Owner : Senior #1 50 % time
  Scope : G-CB-IMPL or G-ENGINE-CB resolution
  Coord : M50 doesn't block PR #28, but Senior #1 split focus

PR #29 (Phase 3 third gap)  :  Nov-Dec 2026
  Owner : TBD
  Scope : To be defined post-PR #27 closure
  Coord : Sequenced after M50 Phase 3

SOC 2 Type I journey         :  Started Phase 4 (Sprint 12)
                                Audit window M+8 to M+12

Series A fundraise           :  M+12 to M+18 conversation
                                Closure target M+18

Future M51-M55 (post-M50)    :  Yet to be defined
                                Likely : compute backends, multi-region, ML platform
```

---

## 18. Post-M+6 vision

M50 GA is the **floor**, not the ceiling.  After Dec 2026 :

### 18.1 M+6 to M+12 (Q1-Q2 2027)

```
Engineering :
- Performance scaling : 5k Mode A throughput, 100k concurrent swarms
- Multi-region deployment (Zurich + Frankfurt + AWS US East)
- Additional 50 community nano agents
- Specialized verticals : pharma starter pack, banking starter pack

Sales :
- 3-5 design partners signed
- First 1-2 enterprise contracts (CHF 500k-1.5M)

Compliance :
- SOC 2 Type I report
- ISO 27001 readiness
- AI Act Article 9 certification for high-risk customers

Team :
- +2 engineers (platform + ML)
- +1 customer success / TAM
```

### 18.2 M+12 to M+18 (Q3-Q4 2027) — Series A

```
Targeted Series A : USD 10-15M @ USD 30-50M post-money
Use of funds :
- 60 % engineering scaling
- 25 % sales (US team kickoff)
- 15 % compliance + audit-grade tier expansion

Strategic moves :
- US market entry (San Francisco / NYC presence)
- Audit-grade tier flagship customers (Roche / UBS)
- Open source 5-10 more community nano agents
- Partnership announcements (Anthropic, OpenAI MCP)
```

### 18.3 M+18+ (2028+) — Series B horizon

```
Targets at Series B (~ M+24-30) :
- CHF 10-20 M ARR
- 10-20 enterprise customers
- 50-200 design partners + developer accounts
- Multi-vertical : pharma + banking + healthcare + cybersec
- Audit-grade tier defendable as EU regulatory mandatory
```

---

## 19. Communication plan

### 19.1 Investor communications

```
Monthly investor update (start of month) :
  - Metrics snapshot
  - Sprint completion status
  - Risks and asks
  - Customer pipeline state

Quarterly board meeting (3 per year during M50 phase) :
  - Phase gate review
  - Financial state vs budget
  - Strategic decisions

Ad-hoc updates :
  - Major customer signing
  - Hire signing
  - Significant compliance event
```

### 19.2 Customer communications

```
Sprint 0 — Pre-pitch        : Outreach to 5-10 design partner prospects
Sprint 3 — Phase 1 done     : Update interested prospects on progress
Sprint 6 — Phase 2 done     : Demo invite to qualified design partners
Sprint 10 — Public Beta     : Mass announcement (newsletter, social, MCP marketplace)
Sprint 13 — GA              : Major launch announcement
```

### 19.3 Engineering team communications

```
Daily standup (15 min, async-friendly)
Bi-weekly sprint planning (1.5h)
Bi-weekly retrospective (1h)
Monthly architecture review (2h)
Quarterly all-hands (half-day)
```

### 19.4 Open source community

```
Phase 3 (Sprint 9) onwards :
  - GitHub repo public
  - Discord community
  - 3-5 blog posts during Phase 3-4
  - Conference talk submissions
```

---

## 20. Annexe A — Gantt visualization

```
                  Jun    Jul    Aug    Sep    Oct    Nov    Dec
─────────────────┬──────┬──────┬──────┬──────┬──────┬──────┬─────
PHASE 0  Bootstrap█▌
PHASE 1  Foundation  █████████
PHASE 2  Audit                    ████████
PHASE 3  Distribution                       ████████████
PHASE 4  Hardening + GA                                 ███████

Sprints :    0  1 2 3   4 5 6   7 8 9 10   11 12 13

Hires :
  Senior #1 ▲ Sprint 1 start
  Senior #2                   ▲ Sprint 4 start
  Dev Adv                              ▲ Sprint 9 start

Major events :
  Public Beta launch                              ▲ end Sprint 11
  GA launch                                              ▲ end Sprint 13
  STABILITY v1.8                                          ▲ Sprint 13
  Series A pitch begin                                     ▲ Sprint 13
─────────────────┴──────┴──────┴──────┴──────┴──────┴──────┴─────
```

---

## 21. Annexe B — Sprint 0 deep dive (le sprint critique)

Sprint 0 est le sprint **avant le code**.  C'est ce qui empêche
toutes les autres sprints de partir dans le mur.

### 21.1 Sprint 0 — Day-by-day plan (2 weeks, 10 working days)

```
Day 1 (Jun 1, lundi)
  Morning   : Pre-condition checklist (cf § 1)
                 → All items confirmed
  Afternoon : Action 0.5 — Décider 1 vs 2 crates → 2 crates decided
                 + write decision doc

Day 2 (Jun 2, mardi)
  Morning   : Action 0.1 — Audit M-numbers in docs/
                Run grep, verify mappings
  Afternoon : Action 0.1 (suite) — Write EFE_M_NUMBER_REGISTRY.md

Day 3 (Jun 3, mercredi)
  All day    : Action 0.2 — Inventory MCP tools existing
                grep, document, identify conflicts
                Write MCP_TOOLS_INVENTORY_2026-06.md

Day 4 (Jun 4, jeudi)
  Morning   : Action 0.3 — ClickHouse vs Postgres decision
                Benchmark Postgres analytical query speeds on synthetic data
  Afternoon : Action 0.3 (suite) — Cost estimates + decision doc

Day 5 (Jun 5, vendredi)
  All day    : Action 0.4 — Firecracker POC
                Setup local Firecracker env
                Launch 1 microVM, run nano-agent-like workload

Day 6 (Jun 8, lundi — second week)
  All day    : Action 0.4 (suite) — Document POC results
                Write Firecracker runbook for future engineers

Day 7 (Jun 9, mardi)
  Morning   : Action 0.6 — Scaffold efe-nano crate
                Cargo.toml, src/lib.rs, README
  Afternoon : Action 0.6 (suite) — Scaffold efe-nano-sdk crate
                Verify cargo check workspace clean

Day 8 (Jun 10, mercredi)
  All day    : CI integration : add efe-nano + efe-nano-sdk to CI workflow
                Verify 174 PR #27 + 51 STABILITY tests still green

Day 9 (Jun 11, jeudi)
  All day    : Senior #1 onboarding (if hire signed)
                + draft Sprint 0 validation report

Day 10 (Jun 12, vendredi)
  Morning   : Finalize EFE_M50_SPRINT_0_VALIDATION_REPORT.md
  Afternoon : Sprint 0 retro
                Phase 1 kickoff prep
                Sign off Phase 0 → Phase 1 gate
```

### 21.2 Sprint 0 contingency

If Day 5 (Firecracker POC) blocks :
- Day 6 used for diagnosis
- If still blocked, fallback to Wasmtime evaluation
- Adjust Phase 2 plan if pivot needed

If Senior #1 hire doesn't sign before Sprint 1 :
- Sprint 0 extended by 1 week
- Phase 1 starts Sprint 2 instead of Sprint 1
- All subsequent sprints shifted by 2 weeks
- Final GA shifted to mid-January 2027

---

## 22. Annexe C — Capability matrix per sprint

```
                                   Spr0 Spr1 Spr2 Spr3 Spr4 Spr5 Spr6 Spr7 Spr8 Spr9 Sp10 Sp11 Sp12 Sp13

Catalog (NAC)                       ─    ▓    ▓    ▓    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─
Runtime Mode A (NAR-A)              ─    ─    ▓    ▓    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─
Runtime Mode B (NAR-B)              ─    ─    ─    ─    ▓    ▓    ─    ─    ─    ─    ─    ─    ─    ─
Runtime Mode C (NAR-C)              ─    ─    ─    ─    ─    ▓    ─    ─    ─    ─    ─    ─    ─    ─
Dispatcher (SD)                     ─    ─    ─    ▓    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─
Audit Trace (ATC)                   ─    ─    ─    ─    ─    ─    ▓    ─    ─    ─    ─    ─    ─    ─
Replay capability                   ─    ─    ─    ─    ─    ─    ▓    ─    ─    ─    ─    ─    ─    ─
MCP Bridge (MCB)                    ─    ─    ─    ─    ─    ─    ─    ▓    ─    ─    ─    ─    ─    ─
Rust SDK                             ─    ─    ─    ─    ─    ─    ─    ─    ▓    ─    ─    ─    ─    ─
CLI                                  ─    ─    ─    ─    ─    ─    ─    ─    ▓    ─    ─    ─    ─    ─
Documentation site                  ─    ─    ─    ─    ─    ─    ─    ─    ─    ▓    ─    ─    ─    ─
Python SDK                           ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ▓    ─    ─    ─
TypeScript SDK                      ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ▓    ─    ─
Billing & Stripe                     ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ▓    ─    ─
Audit-grade tier                     ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ▓    ─
STABILITY v1.8                       ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ▓
GA launch                            ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ─    ▓

Starter nano agents shipped         0    1    5    10   13   15   15   20   20   20   20   20   20   20

Cumulative new tests                 0    25   50   80   105  140  170  195  225  235  270  300  325  340
```

---

## 23. Annexe D — Decisions log

| ID | Decision | Date | Rationale |
|----|----------|------|-----------|
| D-M50-ROADMAP-001 | 14 sprints (Phase 0-4) | 2026-05-18 | Aligned with architecture doc § 13 post-audit fix |
| D-M50-ROADMAP-002 | Sprint 0 = validation, not coding | 2026-05-18 | Verification report (D-M50-AUDIT-*) flagged 7 actions to validate before code |
| D-M50-ROADMAP-003 | 2 crates (efe-nano + efe-nano-sdk) | 2026-05-18 | Verification report D-M50-AUDIT-1 resolution |
| D-M50-ROADMAP-004 | Team scaling 1 → 4 over Phase 1-3 | 2026-05-18 | Realistic Swiss hiring market + budget envelope |
| D-M50-ROADMAP-005 | GA target end-December 2026 | 2026-05-18 | 6.5 months from pre-seed close = aggressive but feasible |
| D-M50-ROADMAP-006 | STABILITY v1.7 → v1.8 bump at Sprint 13 | 2026-05-18 | Audit-grade tier requires schema freeze post-launch |
| D-M50-ROADMAP-007 | 3 decision points (post-Phase 1, mid-Phase 3, pre-GA) | 2026-05-18 | Standard go/no-go gates for strategic adjustment |
| D-M50-ROADMAP-008 | Free tier loss-leader strategy | 2026-05-18 | PLG funnel required for B2A bottom-up adoption |

---

## 24. References & sign-off

**Companion docs** :
- `docs/architecture/EFE_M50_AGENTIC_ARCHITECTURE.md`
- `docs/architecture/EFE_M50_VERIFICATION_REPORT_2026-05-18.md`
- `docs/STABILITY_CONTRACT.md` (current v1.7)
- `docs/gaps/master_tracker.md`

**Approval gates** :
- [ ] Founder sign-off (Rodrigue) — required before Sprint 0
- [ ] Lead investor sign-off (Kovant or alternative) — required before pre-condition gate
- [ ] First senior hire sign-off — required before Sprint 1 start

---

_End of EFE M50 roadmap._

_Cette roadmap est versionnée v1 ; sera updated quarterly aligned
avec les exit gates de chaque phase. Toute déviation > 1 sprint
trigger une décision board.  Les 14 sprints décrits ici sont
indicatifs : la réalité ajustera selon les findings de Sprint 0
et les feedbacks customers de Phase 3._
