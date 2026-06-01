# EFE — Master Roadmap — 2026 H2 → 2027 H1

| Field | Value |
|---|---|
| **Doc type** | Master roadmap synthétisant TOUS les workstreams en cours sur EFE |
| **Status** | Frozen — awaiting weekly tactical update |
| **Frozen at** | 2026-05-15T00:00:00Z |
| **Horizon** | 2026-05-15 → 2027-06-30 (14 mois ; 28 sprints de 2 semaines) |
| **Décision gate principale** | **L2 graduation** Q2 2027 + **GA v1.0** Q2 2027 |
| **Supersedes** | `docs/EFE_A1_PHASE2_ROADMAP.md` (Phase 2 sub-scope) ; `docs/EFE_A1_ROADMAP.md` (A1 wider scope) |
| **Cross-references** | `EFE_A1_PHASE2_ROADMAP.md` § 7 (PR catalog) ; `EFE_HONEST_BENCHMARKS_ARCHITECTURE.md` § 45 (bench migration) ; `EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` (SLO bundle) |
| **Authors** | EFE platform team |
| **Reviewers** | 2× SRE + 1× perf + 1× sec + 1× product + 1× finops + 1× legal + 1× compliance |
| **Refresh cadence** | Weekly tactical review (Friday async) ; quarterly strategic refresh |

---

## Table of contents

  0. TL;DR (1-page executive summary)
  1. Scope + non-goals + audience
  2. Where we are now (May 2026 snapshot)
  3. The 5 workstreams
  4. Quarterly milestones (Q3 2026 → Q2 2027)
  5. Sprint cadence (28 sprints, 2 weeks each)
  6. Workstream WS1 — Meta-swarm Phase 2 closure (PR #16-#25)
  7. Workstream WS2 — Honest Bench Framework rollout
  8. Workstream WS3 — CI hygiene + master-gate stability
  9. Workstream WS4 — mTLS + cert-manager (PR #16 prerequisites)
 10. Workstream WS5 — Documentation + auditor readiness
 11. Dependency graph
 12. RACI per workstream
 13. Resource allocation (FTE-weeks)
 14. Decision gates (L2 graduation, public-bench publish, GA v1.0)
 15. Soak windows
 16. Risk register (18 risks)
 17. Communication cadence
 18. Rollback strategies (per workstream)
 19. Public communication policy
 20. Reviewer checklist (this roadmap)
 21. Appendix A — PR #16-#25 architecture commitments (per-PR LOC + dependencies)
 22. Appendix B — bench framework migration matrix (per-surface)
 23. Appendix C — CI hygiene backlog (post-audit findings)
 24. Appendix D — auditor-readiness checklist
 25. Appendix E — open architectural questions (unresolved)
 26. Appendix F — sunset list (deprecated / removed items)
 27. Appendix G — capacity planning calendar

---

## 0. TL;DR

### Where we are (May 2026)

  - **Phase 2 meta-swarm** : 11 of 25 PRs landed (PR05–PR15 documented + coded + audited).
    14 PRs to go (PR16-PR25), tracked here.
  - **Production binary** (`efe-server`) : builds, tests pass (4 294 / 4 295 — 1 false-positive),
    Helm chart v0.2.0 + appVersion 0.3.0 deployable.
  - **CI gates** : `cargo clippy -- -D warnings` ACTIVE + BLOCKING ;
    `cargo fmt --check` ACTIVE + BLOCKING. Audit revealed **F2 drift**
    (1 076 warnings + 28 525 fmt diff lines on working tree).
  - **SLO baseline** : `ops_master_slos.yaml v0.9.0` consolidates 18 master SLOs.
  - **18 of 60** SLOs benched honestly with full distribution today ;
    42 to migrate to the Honest Bench Framework over 22 weeks.

### Where we will be (June 2027)

  - **Meta-swarm Phase 2 closed** : all 25 PRs landed, L2 graduation ceremony done.
  - **Honest Bench Framework v1.0** in production : 60+ SLOs measured with
    coordinated-omission correction + full percentile distribution + public reproducer.
  - **mTLS** : controller + N participants on cert-manager-issued certs ;
    Cipher-suite policy frozen.
  - **Public bench publication** : marketing site + auditor portal live.
  - **GA v1.0** : EFE shipped to design partners (10 paying customers Q1 2027 ;
    50 paying Q2 2027).

### Roadmap shape (14 months → 28 sprints)

```
Q2'26 W1-W6   ─ Sprint -0 .. -3 : foundation work + audit closure
Q3'26 W1-W12  ─ Sprint 1 .. 6   : PR #16 mTLS + PR #17-18 obs + bench WS2 phase 1
Q4'26 W1-W12  ─ Sprint 7 .. 12  : PR #19-22 dashboards + loadgen + bench WS2 phase 2
Q1'27 W1-W12  ─ Sprint 13 .. 18 : PR #23-25 SLO consolidation + bench WS2 phase 3
Q2'27 W1-W12  ─ Sprint 19 .. 24 : L2 graduation + GA v1.0 launch
                                   + reaching steady-state
```

### The 5 workstreams

| WS  | Title | Owner | FTE-weeks |
|-----|-------|-------|-----------|
| WS1 | Meta-swarm Phase 2 closure (PR16-PR25) | meta-swarm | 32 |
| WS2 | Honest Bench Framework rollout | perf + sre | 24 |
| WS3 | CI hygiene + master-gate stability | platform | 8 |
| WS4 | mTLS + cert-manager wiring | sec + meta-swarm | 6 |
| WS5 | Documentation + auditor readiness | product + legal | 10 |
| | **Total** | | **80 FTE-weeks** (over 14 months × 6 FTE = 168 capacity → 48 % utilisation) |

### 3 decision gates

  - **L2 graduation** : 2027-04-30. Pre-conditions in § 14.1.
  - **Public bench publication** : 2027-02-15. Pre-conditions in § 14.2.
  - **GA v1.0 launch** : 2027-06-15. Pre-conditions in § 14.3.

---

## 1. Scope + non-goals + audience

### Scope

This roadmap is the single tactical + strategic plan for **EFE platform engineering**
over the next 14 months. It supersedes the per-phase roadmap docs by integrating
all parallel workstreams into one timeline.

In scope :

  - Meta-swarm Phase 2 closure (PR #16-#25).
  - Honest Bench Framework rollout (per `EFE_HONEST_BENCHMARKS_ARCHITECTURE.md` § 45).
  - CI hygiene (master-gate stability ; F2 audit closure).
  - mTLS + cert-manager wiring (PR #16 prerequisites).
  - Auditor-readiness package (compliance, SLA, signed reproducers).
  - L2 graduation ceremony.
  - GA v1.0 launch.

### Non-goals

Out of scope of THIS roadmap (have their own separate plans) :

  - M-series features beyond M33 (M34+ in 2027 H2+ plans).
  - Mode-4 explorations (Phase 3 territory).
  - Multi-modal-only product (M30 covered here only as bench surface).
  - SDK v2 (kept on the 4 SDKs at their current major version).
  - Production-side observability infra (kube-prometheus-stack already managed by SRE).
  - Customer success / sales / marketing operations.

### Audience

  - EFE platform engineers (primary readers — execute this roadmap).
  - Product (planning + commit cadence).
  - SRE (capacity planning + on-call cadence).
  - Sec (compliance + auditor liaison).
  - Legal (auditor MOU + license + public claims).
  - Customers (slim summary — § 19 public comms only).

---

## 2. Where we are now (May 2026 snapshot)

### 2.1 Code state

```
Workspace                : 28 crates, Rust 1.88
Binaries                 : efe-server (13 MiB Mach-O ARM64 local ; r3-clean container target)
Tests                    : 4 294 passing, 1 false-positive failed
                           (cfg!(debug_assertions) gated test under --release)
Compilation              : 0 errors, 1 076 warnings (mostly missing_docs)
Format                   : 2 078 diff blocks (28 525 lines) — F2 in audit
Migrations               : 46 SQL files in crates/efe-server/migrations,
                           0 duplicate prefixes (post Φ-D-OPS R1-B1 fix),
                           0026-0030 all present
```

### 2.2 Helm chart state

```
Chart : charts/efe-stack/charts/efe-meta-swarm
  Chart.yaml.version    : 0.2.0
  Chart.yaml.appVersion : 0.3.0
  Templates             : 25 files
  helm-unittest         : 93 assertions across 11 suites
  Scenarios + goldens   : 6 + 6
  Example overlays      : 4 (dev / staging / prod / canary)
  Audit-pass fixes      : trunc-safe helpers, ternary ignoreFailures,
                          ipBlock hookEgressCidrs, emptyDir snapshot Job
```

### 2.3 CI workflow state

```
.github/workflows/        : 71 workflows
helm-meta-swarm.yml       : 7 jobs (integration battery — PR #13/#14)
helm-meta-swarm-lint.yml  : 15 jobs (PR #15) — 12 Rego rules + lint configs
ci.yml                    : main rust gate (cargo fmt --check + clippy -D warnings)
                            STRICT, BLOCKING, pull_request → main
release-v1-ga.yml         : release gate with bench + SLSA + SBOM
```

### 2.4 Documentation state

```
docs/
├── EFE_A1_PHASE2_ARCHITECTURE.md             1 614 LOC (master Phase 2)
├── EFE_A1_PHASE2_ROADMAP.md                  1 244 LOC (PR catalog, sprint cadence, L2 gate)
├── EFE_A1_PHASE2_PR05-PR15_ARCHITECTURE.md   23 922 LOC across 11 PR docs
├── EFE_HONEST_BENCHMARKS_ARCHITECTURE.md     2 852 LOC (Volume I)
├── EFE_HONEST_BENCHMARKS_DEEP_DIVE.md        2 153 LOC (Volume II)
├── EFE_A1_BENCH_TRACKING.md                  bench tracking history
├── EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md SLO bundle origin
├── (other M-series + extension docs)         ~ 80 docs total
```

PDF compendium built : `EFE_A1_PHASE2_META_SWARM_COMPENDIUM.pdf` (504 pages,
17 chapters, 27 581 LOC source).

### 2.5 SLO state

```
bench_baselines/v0.9.0/ops_master_slos.yaml  18 master SLOs, frozen 2026-05-12
```

Coverage of the 60+ SLOs in the Honest Bench Framework catalog
(Volume I appendix D) :

  - **Today** : 18 master SLOs measured (one of 7 percentiles, partial CI).
  - **Q4 2026 target** : 18 master + 24 surface SLOs (42 total) on new framework.
  - **Q1 2027 target** : All 60+ SLOs on the framework, with coordinated-
    omission correction + full distribution + bootstrap CI + public reproducer.

### 2.6 Known outstanding items (audit-derived)

  - **F2 fmt drift** (28 525 lines) : blocks master-gate, fix scope =
    1 commit `cargo fmt --all` ; not yet applied because working dir is non-git in
    audit env. Sprint -1 deliverable.
  - **F2 warnings** (1 076) : mostly `missing_docs`. Tech-debt cleanup over
    3 sprints, per WS3.
  - **DT-OPS-17** : `efe-edge` chart routes to `efe-server-rest:8080` dead listener.
    Deferred (per project's open-defect ledger).
  - **DT-OPS-23** : Maven `META-INF/LICENSE+NOTICE` placement for Java SDK ;
    deferred (per project's ledger).
  - **PR #16** : architecture frozen but code not started.
  - **PR #17-PR #25** : architecture pending.

---

## 3. The 5 workstreams

Each workstream is **independent enough** that one stalling does not block the
others. Cross-workstream dependencies are explicit (§ 11).

### WS1 — Meta-swarm Phase 2 closure (PR #16-#25)

10 PRs. Spans Q2 2026 W6 → Q1 2027 W12.

Owner : meta-swarm team (1.5 FTE).

Goal : Complete the 25-PR catalog of the Phase-2 roadmap such that the L2 gate
can be ceremonially passed.

### WS2 — Honest Bench Framework rollout

22 weeks (per Volume I § 45) split into 3 phases :

  - Phase 1 — Foundation (6 weeks) : `efe-bench-harness` crate + 4 host tiers +
    manifest schema.
  - Phase 2 — Per-surface migration (12 weeks) : migrate 14 surfaces from legacy
    bench to Honest Bench.
  - Phase 3 — Public publication (4 weeks) : auditor portal + marketing site live.

Owner : perf + SRE team (1 FTE).

Goal : All published SLOs are honest by the GA gate.

### WS3 — CI hygiene + master-gate stability

Closes F2 audit items + the long-tail of missing_docs warnings + clippy clean-up.

Owner : platform team (0.4 FTE).

Goal : `cargo fmt --check` + `cargo clippy -D warnings` master-gate is green on
every PR, every day, by Q3 2026 W6.

### WS4 — mTLS + cert-manager wiring

The PR #16 deep work : extending PR #14's TLS stubs to real mTLS provisioning +
binary-side tonic-tls integration.

Owner : sec + meta-swarm team (0.4 FTE, time-boxed).

Goal : PR #16 lands by Q3 2026 W12 with mTLS enabled by default on staging tier ;
opt-in on prod tier ; auditable cert rotation cadence.

### WS5 — Documentation + auditor readiness

Compliance documents, SLA contracts, signed reproducers, public bench portal
content authoring.

Owner : product + legal (0.6 FTE).

Goal : Auditor MOU signed by Q1 2027 W6 ; public reproducer + SLA published by
GA gate.

---

## 4. Quarterly milestones

### Q2 2026 remaining (W5-W12) — "Foundation"

```text
W5 (May 12-18)  : F2 audit close-out — cargo fmt sync committed
W6 (May 19-25)  : PR #16 architecture authoring + sec review
W7 (May 26-Jun 1): WS2 Phase 1 starts — efe-bench-harness crate scaffold
W8 (Jun 2-8)    : Honest Bench Framework Volume III (runbooks) authoring
W9 (Jun 9-15)   : Master-gate green for 2 consecutive weeks (F2 closed)
W10 (Jun 16-22) : PR #16 code starts
W11 (Jun 23-29) : PR #16 code mid-point + mTLS staging deploy
W12 (Jun 30)    : Q2 close — readiness review for Q3
```

### Q3 2026 (Jul-Sep) — "Visibility"

PR #16 lands. PR #17 (14 Prometheus metrics) starts. PR #18 (8 OTel span types)
starts. Bench Framework Phase 1 closes ; Phase 2 starts on Mode 1.

Deliverable : public dashboard + 14 Prometheus alerts firing.

### Q4 2026 (Oct-Dec) — "Operability"

PR #19 Grafana dashboards. PR #20 loadgen. PR #21 public-bench harness. PR #22
4 SLOs + rollup. Bench Phase 2 hits 12 surfaces.

Deliverable : First 7-day soak run cleanly. Loadgen runs autonomously nightly.

### Q1 2027 (Jan-Mar) — "Publication readiness"

PR #23 smoke pin. PR #24 publish bench results. Bench Phase 3 — public-bench
portal goes live. Auditor MOU signed.

Deliverable : Public bench results visible at `efe.example.com/bench/`.

### Q2 2027 (Apr-Jun) — "L2 graduation + GA"

PR #25 Phase 2 lessons doc. L2 graduation ceremony 2027-04-30. GA v1.0 launch
2027-06-15.

Deliverable : EFE shipped, paying customers running production traffic.

---

## 5. Sprint cadence (28 sprints × 2 weeks)

Standardized format per sprint :

  - Monday W0 : sprint kickoff (30 min).
  - Daily standup : 15 min async via dedicated Slack channel.
  - Friday W1 : mid-sprint check-in (async).
  - Monday W1 : 1:1 between workstream owners and SRE on-call.
  - Friday W2 : sprint demo + retrospective.

### Sprint table

```
Sprint  Window           Theme                           WS focus
─────── ──────────────── ─────────────────────────────── ────────────────
S-3     May 04-15 (now)  audit close-out (F2)            WS3
S-2     May 18-29        WS2 Phase 1 start               WS2 + WS3
S-1     Jun 01-12        PR #16 arch frozen              WS1 + WS4
S 0     Jun 15-26        PR #16 code starts              WS1 + WS4 + WS2
S 1     Jun 29-Jul 10    PR #16 mid + bench mode1 mig    WS1 + WS4 + WS2
S 2     Jul 13-24        PR #16 lands ; PR #17 starts    WS1 + WS2
S 3     Jul 27-Aug 7     PR #17 mid                      WS1 + WS2
S 4     Aug 10-21        PR #17 lands ; PR #18 starts    WS1 + WS2
S 5     Aug 24-Sep 4     PR #18 lands                    WS1 + WS2
S 6     Sep 7-18         PR #19 starts (dashboards)      WS1 + WS2
S 7     Sep 21-Oct 2     PR #19 lands                    WS1 + WS2
S 8     Oct 5-16         PR #20 starts (loadgen)         WS1 + WS2
S 9     Oct 19-30        PR #20 mid                      WS1 + WS2
S10     Nov 2-13         PR #20 lands ; PR #21 starts    WS1 + WS2
S11     Nov 16-27        PR #21 mid                      WS1 + WS2
S12     Nov 30-Dec 11    PR #21 lands                    WS1 + WS2
S13     Jan 4-15 (2027)  PR #22 starts (4 SLOs)          WS1 + WS2 + WS5
S14     Jan 18-29        PR #22 lands                    WS1 + WS2 + WS5
S15     Feb 1-12         Bench Phase 3 public bench      WS2 + WS5
S16     Feb 15-26        Public bench published          WS2 + WS5
S17     Mar 1-12         PR #23 smoke pin lands          WS1 + WS5
S18     Mar 15-26        PR #24 publish bench results    WS1 + WS5
S19     Mar 29-Apr 9     PR #25 lessons + L2 prep        WS1 + WS5
S20     Apr 12-23        L2 verification ceremony prep   ALL
S21     Apr 26-May 7     L2 graduation gate              ALL
S22     May 10-21        GA v1.0 release-gate prep       ALL
S23     May 24-Jun 4     GA v1.0 launch (Jun 15)         ALL
S24     Jun 7-18         GA stabilization                ALL
```

---

## 6. Workstream WS1 — Meta-swarm Phase 2 closure

### 6.1 PRs to land (10 total)

```
PR #16  mTLS + cert-manager Issuer       — Q3 2026 S 2 land
PR #17  14 Prometheus metrics            — Q3 2026 S 4 land
PR #18  8 OpenTelemetry span types       — Q3 2026 S 5 land
PR #19  Grafana dashboards + alert rules — Q3-Q4 2026 S 6-7 land
PR #20  loadgen traffic generator        — Q4 2026 S10 land
PR #21  public-bench harness + 5 workloads — Q4 2026 S12 land
PR #22  4 SLOs + rollup pin              — Q1 2027 S14 land
PR #23  smoke rollup pin                 — Q1 2027 S17 land
PR #24  publish bench results            — Q1 2027 S18 land
PR #25  Phase-2 lessons + soak report    — Q1 2027 S19 land
```

### 6.2 Per-PR commitment table

See Appendix A for the full per-PR table : title, author, LOC estimate,
dependencies, AC count, risk.

### 6.3 Cross-cutting policies

  - Every PR ships its own architecture doc in `docs/EFE_A1_PHASE2_PRXX_ARCHITECTURE.md`
    BEFORE coding starts.
  - Every PR has an audit pass within 2 sprints of landing.
  - Every PR's Helm-chart side (if any) lands as a chart minor bump.
  - Every PR's bench surface (if any) is migrated to the Honest Bench
    Framework — co-development with WS2.

### 6.4 L2 graduation prerequisites

The L2 gate (per `EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md` § 10) requires :

  - All 25 PRs of the Phase-2 catalog landed.
  - 18 master SLOs running honestly for ≥ 14 consecutive days.
  - 0 Sev1 / 0 Sev2 incident in 14 days.
  - DR drill executed in Q2 2027 with RTO < 30 min, RPO < 1 h.
  - Stripe billing accuracy within 0.5 % over the prior 30 days.
  - 4-eyes approval workflow drilled (1 simulated request, approved by 2,
    rejected by 1 non-approved).

---

## 7. Workstream WS2 — Honest Bench Framework rollout

### 7.1 Phase 1 — Foundation (6 weeks ; S-2 through S 0)

Deliverables :

  - `crates/efe-bench-harness/` Rust crate (HDR + bootstrap + bench runner) — 2 weeks
  - `bench_baselines/v1.0.0/` schema + first per-surface baselines (3) — 1 week
  - 4 host-tier definitions + Kubernetes node pools — 1 week
  - Bench manifest schema (Appendix A of Honest Bench doc) — 1 week
  - Meta-bench (framework benchmarks itself ; AD22) — 1 week

### 7.2 Phase 2 — Per-surface migration (12 weeks ; S 1 through S12)

```text
Sprint   Surface migrated (per Honest Bench Volume I § 14-30)
─────── ──────────────────────────────────────────────────────
S 1     A1 Mode 1 (latent classification)
S 2     A2 Mode 2 (LLM-routed), A3 Mode 3 (multi-turn)
S 3     B1-5 Bundle lifecycle (reload, cold load, verify, helm hook, sidecar)
S 4     G1-5 Vector search (HNSW / IVF-PQ / Flat per surface)
S 5     H1-4 Knowledge graph (NER, snapshot, multi-hop, build)
S 6     E1-4 Streaming + events (NATS JetStream)
S 7     F1-4 Active learning (M24)
S 8     I1-3 Multimodal (M30)
S 9     J1-4 Edge (M27)
S10     K1-4 Billing + FinOps (M31)
S11     L1-5 Agent runtime (M25/M26)
S12     M1-9 Meta-swarm Phase 2 surfaces (PR05-PR15)
        N1-7 SDK clients (Py / TS / Java / Go)
        O1-3 FFI / Python in-process
        P1-6 Backup / DR
        Q1-7 Operational
```

### 7.3 Phase 3 — Public publication (4 weeks ; S15 through S16)

Deliverables :

  - Customer-facing bench portal at `efe.example.com/bench/` — 2 weeks
  - Auditor protocol documented + signed reproducers — 1 week
  - SLA enforcement (5-day response, 15-day RCA) — 1 week (legal)

### 7.4 Acceptance gates

WS2 closes when :

  - All 30 ACs of the Honest Bench Framework are green (Volume I § 41).
  - Drift detection runs nightly + has fired at least 5 times correctly.
  - At least 3 surfaces have been audited externally (a non-perf team member
    reproduced + verified within 99 % CI).

---

## 8. Workstream WS3 — CI hygiene + master-gate stability

### 8.1 F2 fmt sync (Sprint S-3)

Deliverable : 1 commit `chore: cargo fmt --all` lands on `main`. Master-gate
fmt-check passes for ≥ 7 consecutive days.

### 8.2 missing_docs cleanup (Sprints S-2 through S 1)

Deliverable : 1 076 warnings → 0 warnings. Strategy :

  - Sprint S-2 : crates `efe-prompts` (18 warns) + `efe-telemetry` (19) + `efe-events` (47) + `efe-cache` (9) + `efe-curation` (6) + `efe-active-learning` (5) → 104 cumulative.
  - Sprint S-1 : `efe-llm-router` (173) → 173 cumulative.
  - Sprint S 0 : `efe-server lib` (782 → split across 4 sub-sprints).
  - Sprint S 1 : remaining (efe-server bin warnings + smaller crates).

Acceptance : after S 1, `cargo check --workspace` emits 0 warnings.

### 8.3 Long-tail master-gate cleanup (Sprints S 1 through S 6)

  - Meta-swarm-server `[profile.*]` migration to workspace root.
  - `migrations/00X_onboarding_sessions.sql` orphan reconciliation.
  - `accepts_localhost_in_dev` test ungating under `--release`.
  - clippy::pedantic gradual rollout on perf-critical crates.

### 8.4 Acceptance gates

WS3 closes when :

  - 0 cargo check warnings on `main` workspace-wide.
  - 0 cargo clippy `-D warnings` violations.
  - 0 cargo fmt drift.
  - 0 master-gate red on PRs for ≥ 30 consecutive days.

---

## 9. Workstream WS4 — mTLS + cert-manager (PR #16)

### 9.1 Architecture authoring (Sprint S-1)

`docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md` — target ≈ 2 500 LOC. Cover :

  - Chart-side : cert-manager Issuer + per-pod Certificate refinement
    (extension of PR #14 stubs).
  - Binary-side : tonic-tls server config ; cert reload hot-swap.
  - Trust chain : self-signed (dev / staging) AND CA-signed (prod) variants.
  - mTLS-vs-TLS-only mode flag.
  - Rotation cadence : every 30 days ; alert at 7 days remaining.

### 9.2 Code (Sprints S 0 through S 2)

  - S 0 : binary-side tonic-tls + cert file-watcher.
  - S 1 : chart-side Issuer/Certificate refinements + bench instrumentation.
  - S 2 : integration tests + helm-unittest + golden snapshots + audit pass.

### 9.3 Risk : binary-binary compat

PR #16 binary needs cert path. Old binary doesn't. Migration sequence :

  1. Ship PR #16 binary with `tls.enabled=false` default (backward-compat).
  2. After ≥ 14 days production stability, flip staging defaults to `tls.enabled=true`.
  3. After ≥ 30 days, prod defaults flip.

---

## 10. Workstream WS5 — Documentation + auditor readiness

### 10.1 Volume III (runbooks) — Sprint S 0

  - Per-surface bench runbook (one per WS1 surface).
  - Drift triage runbook.
  - Bench reproducer recipes.
  - Failure-mode catalog.

### 10.2 Auditor MOU drafting — Sprint S15

Legal-led. Covers :

  - Audit access scope (public artefacts only).
  - SLA : 5 business days response, 15 business days RCA.
  - Dispute resolution process.
  - Compensation for verified-incorrect-claim findings.

### 10.3 Customer-facing comms — Sprint S20

  - Public bench portal content authoring.
  - Marketing site bench page (subset of portal).
  - L2 graduation announcement post (draft Q1 2027).

### 10.4 Compliance doc package — Sprint S20

  - GDPR Article-30 ROPA for bench fleet.
  - EU AI Act technical documentation refresh.
  - SOC 2 type II audit prep.

---

## 11. Dependency graph

```text
                    ┌──────────────┐
                    │  S-3 : F2    │
                    │  fmt sync    │  WS3
                    └──────┬───────┘
                           │
       ┌───────────────────┼─────────────────────────┐
       │                   │                         │
       v                   v                         v
┌──────────────┐    ┌──────────────┐         ┌──────────────┐
│ WS2 Phase 1  │    │ PR #16 arch  │         │ WS3 missing  │
│ bench-harness│    │ (sec review) │         │ docs cleanup │
│ S-2..S 0     │    │ S-1          │         │ S-2..S 1     │
└──────┬───────┘    └──────┬───────┘         └──────────────┘
       │                   │
       │                   v
       │            ┌──────────────┐
       │            │ PR #16 code  │ WS4
       │            │ S 0..S 2     │
       │            └──────┬───────┘
       │                   │
       │                   v
       │            ┌──────────────┐
       │            │ PR #17 code  │ WS1
       │            │ S 2..S 4     │  needs Prometheus integration ;
       │            └──────┬───────┘  cross-checks with bench
       │                   │
       v                   v
┌─────────────────────────────────────────────┐
│ WS2 Phase 2 — per-surface migration         │
│ S 1..S12 (each surface co-developed with    │
│ its PR landing in WS1)                      │
└──────┬──────────────────────────────────────┘
       │
       v
┌─────────────────┐         ┌──────────────────┐
│ WS2 Phase 3     │  ──────>│ Public bench     │
│ S15..S16        │         │ portal live      │
└─────────────────┘         │ S16              │
                            └──────┬───────────┘
                                   │
                                   v
                            ┌──────────────────┐
                            │ Auditor MOU      │ WS5
                            │ S15              │
                            └──────┬───────────┘
                                   │
                                   v
                            ┌──────────────────┐
                            │ L2 ceremony      │ ALL
                            │ S21 (Apr 30'27)  │
                            └──────┬───────────┘
                                   │
                                   v
                            ┌──────────────────┐
                            │ GA v1.0 launch   │ ALL
                            │ S23 (Jun 15'27)  │
                            └──────────────────┘
```

Critical path : **WS3 fmt sync → WS4 PR #16 → WS1 PR #17/PR #18 → WS2 Phase 2 →
WS2 Phase 3 → L2 → GA**.

Any 2-sprint slip on the critical path delays GA by 2 sprints.

---

## 12. RACI per workstream

```text
                                R          A          C              I
WS1 — Phase 2 closure (PR16-25) meta-swarm  meta-swarm sre+sec+perf  product+legal
WS2 — Bench Framework rollout  perf       sre        meta-swarm+sdk product
WS3 — CI hygiene                platform   platform   ALL            ALL
WS4 — mTLS (PR #16 specifically) sec        sec        meta-swarm     product+legal
WS5 — Auditor readiness         product    legal      sre+sec        finance+exec
```

---

## 13. Resource allocation (FTE-weeks)

```text
                              FTE-weeks  % of capacity
WS1 — Phase 2 closure         32          19%
WS2 — Bench Framework         24          14%
WS3 — CI hygiene               8           5%
WS4 — mTLS PR #16              6           4%
WS5 — Auditor readiness       10           6%
                              ─────       ─────
Sub-total                     80          48%

Operational overhead          15           9%   (interruptions, security patches, on-call)
Bug-fix headroom              25          15%   (~ 1 sprint reactive per Phase)
Architectural exploration     10           6%   (Phase 3 prep, M-series tail)
Slack                         38          22%   (intentional buffer for unforeseen)
                              ─────       ─────
                              168         100%  (6 FTE × 28 sprints)
```

The 22 % slack is **non-negotiable** — historical EFE retrospectives show
unplanned work consistently consumes 20-25 % of capacity. Removing this buffer
guarantees missed deadlines.

---

## 14. Decision gates

### 14.1 L2 graduation (Sprint S21 — 2027-04-30)

Pre-conditions (all must be GREEN) :

  - 25 PRs of Phase 2 catalog landed.
  - 18 master SLOs measured honestly for ≥ 14 days.
  - 0 Sev1 + 0 Sev2 in 14 days.
  - DR drill done Q2 2027 with RTO < 30 min ; RPO < 1 h.
  - Stripe billing accuracy < 0.5 % drift over 30 days.
  - 4-eyes approval drilled.
  - Master-gate green for ≥ 30 days.
  - Honest Bench Framework ACs all green (30/30).
  - Auditor MOU signed.
  - Public bench portal live for ≥ 30 days.

Ceremony :

  - Pre-mortem review (30 min).
  - Final SLO check (15 min).
  - L2 status set in repository tag : `git tag L2-graduated-2027-04-30`.
  - Customer announcement embargo lifted.
  - Press release shipped.

Rollback : if any pre-condition is RED on S20 Monday, slip ceremony by 2 sprints.

### 14.2 Public bench publication (Sprint S16 — 2027-02-15)

Pre-conditions :

  - All 60 SLOs migrated to Honest Bench Framework.
  - Auditor MOU draft circulated.
  - Customer-facing bench portal content review by product + sec + legal.
  - 7-day soak stable on each top-5 surface.

Action : `efe.example.com/bench/` goes from "draft" to "public".

### 14.3 GA v1.0 launch (Sprint S23 — 2027-06-15)

Pre-conditions :

  - L2 graduated.
  - 10 paying design partners on production traffic.
  - SLA contracts signed.
  - 24/7 on-call rotation staffed.
  - Public bench portal live + auditor inquiries handled.
  - Support docs published.

Action : EFE shipped GA. Customer-facing marketing site updated.

---

## 15. Soak windows

### 15.1 Per-Phase soak

  - **Sprint-end soak** : 24h on top-5 surfaces post each sprint demo.
  - **Quarterly soak** : 7-day on top-3 surfaces.
  - **Pre-L2 soak** : 30-day on full bench-fleet stress configuration ;
    starts S19 (Mar 29 2027), ends S21 Monday (Apr 26 2027).
  - **Pre-GA soak** : 14-day on production-like traffic mix ; starts S22
    Monday (May 10 2027), ends S23 Friday (Jun 7 2027).

### 15.2 Soak success criteria

  - p99 latency drift < 5 % over the soak window (per surface).
  - RSS / memory drift < 2 % per 24h.
  - No file-descriptor leak.
  - No GC pause increase (per SDK if applicable).
  - 0 OOM kill.

### 15.3 Stop-the-soak triggers

  - p99 drift > 20 % : stop, investigate, restart.
  - Memory growth > 5 % / 24h : stop, investigate.
  - Sev1 incident on a non-bench traffic surface : pause soak, resume after fix.

---

## 16. Risk register (18 risks)

| ID | Risk | Likelihood | Impact | Mitigation | Owner |
|---|---|---|---|---|---|
| R-01 | F2 fmt sync conflict with in-flight PRs | High | Medium | Coordinate freeze with all PR authors during Sprint S-3 ; squash-merge | platform |
| R-02 | missing_docs cleanup introduces semantic comments that mislead | Medium | Low | Per-crate review by author before merge | platform |
| R-03 | PR #16 mTLS binary cert reload causes connection drop | Medium | High | Hot-swap design ; soak under chaos before staging deploy | sec |
| R-04 | PR #16 mTLS conflicts with M27 edge Envoy mTLS | Low | High | Sec architecture review BEFORE code starts | sec |
| R-05 | Bench harness HDR memory blow-up under multi-tenant | Low | Medium | Memory ceiling per histogram ; meta-bench detection | perf |
| R-06 | Bench corpus PII leak | Low | Critical | M29 PII pipeline + sec review per corpus | sec |
| R-07 | Bench fleet AWS spot eviction during 7d soak | Medium | Medium | Spot for nightly only ; on-demand for soak | sre |
| R-08 | Auditor MOU legal blockers | Medium | High | Legal engaged S 0 ; quarterly check-in | legal |
| R-09 | PR #19 Grafana dashboard fingerprint conflicts | Low | Low | Dashboard slug ownership table | sre |
| R-10 | Loadgen (PR #20) generates unrealistic traffic | Medium | Medium | Co-design with success / support ; corpus replay validation | meta-swarm |
| R-11 | Public bench discrepancy reported by external auditor | Low | Critical | RCA SLA 15 days ; pre-publication independent re-run | bench |
| R-12 | L2 ceremony slips beyond 2027-04-30 | Medium | Critical | 2-sprint slack buffer ; quarterly checkpoint | product |
| R-13 | GA v1.0 not ready due to compliance gap | Medium | Critical | EU AI Act conformity review by S20 | legal |
| R-14 | Customer adoption < 10 by GA gate | Medium | Critical | Design-partner program + early-access waitlist | product |
| R-15 | Stripe billing accuracy < 0.5 % impossible | Low | High | Weekly billing drift report ; reconciliation cron | finops |
| R-16 | Bench framework migration deferred past Q1 2027 | Medium | Critical | Bench Phase 2 has 12-week budget + slack | perf |
| R-17 | mTLS rotation cadence causes flakes on staging | Medium | Low | Rotation alert at 7d remaining ; auto-rotation cron | sre |
| R-18 | Honest-Bench coordinated-omission correction breaks Mode-2 SLO | Medium | Medium | Mode 2 already exceeds nominal SLO ; expected pass | perf |

---

## 17. Communication cadence

```text
Daily       async on #efe-platform Slack channel (standup template)
Weekly      Friday async retrospective + next-week plan
Bi-weekly   Sprint demo + retro (W2 Friday)
Monthly     Workstream-owner sync (60 min)
Quarterly   Strategic refresh + executive update
Sprint S0   Mid-quarter checkpoint
Sprint S6   Mid-quarter checkpoint
Sprint S12  End-of-quarter review
```

### 17.1 Public communication

  - **Internal-only** until S15 (public bench portal live).
  - **Soft external** post-S15 : design partners + advisory board.
  - **Public** post-L2 (S21+).
  - **Customer-facing** post-GA (S23+).

Pre-S15 external comms are EMBARGOED.

---

## 18. Rollback strategies

### 18.1 WS1 PR-level rollback

Each PR is independently revertible. Branch protection enforces squash-merge.
If PR #16-#25 introduces a regression, revert is 1 PR.

### 18.2 WS2 bench-framework rollback

If the Honest Bench Framework's Phase 1 lands and breaks bench CI :

  - Revert the framework PR.
  - Legacy benches keep running on the old infra (compat shim).
  - Per-surface migrations rolled back individually.

### 18.3 WS3 fmt sync rollback

The fmt-sync commit is 1 squash commit. Revert = 1 git operation.

### 18.4 WS4 mTLS rollback

Helm values flag `tls.enabled=false` reverts the binary to plain-text.
Cert-manager artifacts removed via `helm uninstall` (cert-manager continues
to manage other charts).

### 18.5 L2 ceremony slippage

If S21 fails its gate, slip to S23. GA target consequently slips to S25 (Jul 13'27).
Customer comms updated.

### 18.6 GA launch rollback

If GA gate fails on S22 Friday : pull marketing assets, postpone press release.
Re-target S25.

---

## 19. Public communication policy

Aligned with Honest Bench Framework § 40 + § 19 (auditor protocol).

### 19.1 Pre-GA

External-facing claims about EFE must :

  - Cite the specific bench surface + version + tier.
  - Link to the reproducer.
  - Disclose what's NOT measured (AC30).
  - Use the canonical percentile set (AD13).

### 19.2 Marketing claims

Every marketing claim involving a SLO has a tracking ticket in `marketing-claims.md`
with :

  - The exact wording on the marketing site.
  - The bench result file backing it.
  - The auditor review date.
  - The next re-verify date.

Claims that go stale > 90 days without re-verification are auto-removed from the
marketing site by a CI cron.

### 19.3 RFP responses

EFE RFP responses use ONLY published bench results. Internal numbers (e.g. dev-tier
laptop measurements) NEVER appear in customer responses. Compliance enforced by
the RFP response review process.

---

## 20. Reviewer checklist (this roadmap)

  - [ ] Each workstream has an owner + RACI entry.
  - [ ] Each PR in WS1 has an architecture-doc commit before code starts.
  - [ ] Sprint cadence is realistic (≤ 80 % capacity utilization including slack).
  - [ ] Decision gates have explicit pre-conditions.
  - [ ] Soak windows are scheduled.
  - [ ] Risk register has ≥ 15 entries with mitigations.
  - [ ] Critical path identified (§ 11 dependency graph).
  - [ ] Rollback strategy per workstream.
  - [ ] Public comms policy explicit (§ 19).
  - [ ] L2 + GA pre-conditions enumerated (§ 14).
  - [ ] Capacity planning honest (no 100 %-utilization plans).

---

## 21. Appendix A — PR #16-#25 architecture commitments

| PR # | Title | Author | Est. LOC (code+test) | Est. LOC (arch doc) | Depends on | AC count | Risk |
|---|---|---|---|---|---|---|---|
| 16 | mTLS + cert-manager Issuer | sec + meta-swarm | 3 500 | 2 500 | PR #13, #14 | ~ 25 | High (cert rotation) |
| 17 | 14 Prometheus metrics | meta-swarm | 2 200 | 1 800 | PR #8, #9 | ~ 18 | Low |
| 18 | 8 OpenTelemetry span types | meta-swarm | 1 800 | 1 500 | PR #17 | ~ 15 | Low |
| 19 | Grafana dashboards + alert rules | sre | 1 200 | 1 200 | PR #17, #18, #14 | ~ 20 | Medium |
| 20 | loadgen traffic generator | meta-swarm | 2 500 | 1 700 | PR #5, #8 | ~ 15 | Medium |
| 21 | public-bench harness + 5 workloads | meta-swarm | 3 200 | 2 500 | PR #11 | ~ 22 | Medium |
| 22 | 4 SLOs + rollup pin | meta-swarm | 1 500 | 1 500 | PR #17, #18 | ~ 18 | Low |
| 23 | smoke rollup pin | meta-swarm | 800 | 800 | PR #22 | ~ 10 | Low |
| 24 | publish bench results 2026-08 | meta-swarm | 600 | 1 200 | PR #21 | ~ 12 | Medium |
| 25 | Phase 2 lessons + soak report + roadmap refresh | meta-swarm | 200 | 2 500 | ALL | n/a | Low |
| | **Total** | | **17 500** | **17 200** | | **155** | |

---

## 22. Appendix B — bench framework migration matrix

```text
Sprint   Surface (Volume I § ref)                              Old infra → new
─────── ─────────────────────────────────────────────────────── ───────────────
S 1     A1 Mode 1 (§ 14)                                      cargo bench   → harness
S 2     A2 Mode 2, A3 Mode 3                                  partial       → harness
S 3     B1-5 Bundle lifecycle                                  partial       → harness
S 4     G1-5 Vector search                                     cargo bench   → harness
S 5     H1-4 Knowledge graph                                   missing       → harness
S 6     E1-4 Streaming + events                                missing       → harness
S 7     F1-4 Active learning                                   missing       → harness
S 8     I1-3 Multimodal                                        missing       → harness
S 9     J1-4 Edge (M27)                                        missing       → harness
S10     K1-4 Billing + FinOps                                  missing       → harness
S11     L1-5 Agent runtime (M25/M26)                           partial       → harness
S12     M1-9 Meta-swarm Phase 2 + N1-7 SDKs + O1-3 FFI + P1-6 Backup + Q1-7 Operational
                                                              mixed         → harness
```

---

## 23. Appendix C — CI hygiene backlog (post-audit findings)

| ID | Item | Severity | Owner | Sprint |
|---|---|---|---|---|
| F2-fmt | 28 525 fmt drift lines | High | platform | S-3 |
| F2-warn-prompts | 18 missing_docs efe-prompts | Medium | author | S-2 |
| F2-warn-telemetry | 19 missing_docs efe-telemetry | Medium | author | S-2 |
| F2-warn-events | 47 missing_docs efe-events | Medium | author | S-2 |
| F2-warn-cache | 9 missing_docs efe-cache | Medium | author | S-2 |
| F2-warn-curation | 6 missing_docs efe-curation | Medium | author | S-2 |
| F2-warn-al | 5 missing_docs efe-active-learning | Medium | author | S-2 |
| F2-warn-llm | 173 missing_docs efe-llm-router | Medium | author | S-1 |
| F2-warn-server-1 | 200/782 missing_docs efe-server (batch 1) | Medium | author | S 0 |
| F2-warn-server-2 | 200/782 missing_docs efe-server (batch 2) | Medium | author | S 0 |
| F2-warn-server-3 | 200/782 missing_docs efe-server (batch 3) | Medium | author | S 1 |
| F2-warn-server-4 | 182/782 missing_docs efe-server (batch 4) | Medium | author | S 1 |
| meta-swarm-server profile | profile.* migration to workspace root | Low | meta-swarm | S 1 |
| migrations-orphan | reconcile 00X_onboarding_sessions.sql | Low | meta-swarm | S 1 |
| webhook-release-test | ungate accepts_localhost_in_dev | Low | meta-swarm | S 1 |
| DT-OPS-17 | efe-edge dead route | Low | platform | deferred |
| DT-OPS-23 | Maven META-INF/LICENSE placement | Low | java-sdk | deferred |

---

## 24. Appendix D — auditor-readiness checklist

To be all GREEN by Sprint S15 (Feb 2027) :

  - [ ] Public bench portal live at `efe.example.com/bench/`.
  - [ ] Reproducer scripts in repo for each public bench.
  - [ ] Container digests signed via cosign + Sigstore.
  - [ ] Corpus version manifests publicly accessible.
  - [ ] Auditor MOU signed (5d response / 15d RCA).
  - [ ] At least 3 surfaces externally reproduced (independent verification).
  - [ ] SBOM (CycloneDX) for each published bench result.
  - [ ] SLSA-L3 provenance for bench artifacts.
  - [ ] Customer-facing bench disclosure portal content reviewed by sec + legal.
  - [ ] Negative-space disclosure per surface published.
  - [ ] Comparison policy (vs prior versions, vs competitors) documented.
  - [ ] EU AI Act technical documentation refreshed with bench evidence.
  - [ ] GDPR Article-30 ROPA updated for bench fleet.
  - [ ] SOC 2 type II audit scope includes bench fleet.

---

## 25. Appendix E — open architectural questions

| ID | Question | Owner | Decision deadline |
|---|---|---|---|
| Q-01 | mTLS cipher policy : TLS 1.3 ONLY or 1.3+1.2 ? | sec | S-1 |
| Q-02 | cert-manager Issuer kind : self-signed for staging, CA for prod ? OR Vault PKI ? | sec | S-1 |
| Q-03 | Loadgen workload mix : 100 % synthetic vs 50/50 with replay ? | meta-swarm | S 6 |
| Q-04 | Public bench portal CDN : Cloudfront vs Fastly ? | sre | S14 |
| Q-05 | Bench fleet : keep p4d.24xlarge on-demand or move to capacity-block ? | finops | S 6 |
| Q-06 | Multi-region bench coverage : 3 regions Q4 2026 or 5 regions Q1 2027 ? | sre | S 6 |
| Q-07 | M30 multimodal A100 vs H100 default for prod tier ? | sre + product | S 8 |
| Q-08 | DT-OPS-17 resolve in Phase 2 OR defer to Phase 3 ? | product | S10 |

---

## 26. Appendix F — sunset list

Items removed / deprecated as part of this roadmap :

| Item | Reason | Sprint |
|---|---|---|
| Legacy `scripts/bench_runner.sh` | Replaced by `efe-bench-harness` | S12 (Phase 2 close) |
| `bench_baselines/v0.6.0/` AL specific | Superseded by v1.0.0 unified bundle | S12 |
| `bench_baselines/v0.7.0/` streaming | Same | S12 |
| `bench_baselines/v0.8.0/` compliance + LATACT | Same | S12 |
| `migrations/00X_onboarding_sessions.sql` | Orphan ; merge into 0028_d19_p2_onboarding.sql | S 1 |
| `meta-swarm-server [profile.*]` overrides | Move to workspace root | S 1 |

---

## 27. Appendix G — capacity planning calendar

```text
2026 H2 / Sprints S-3..S12
                                                                            Sprint
Mon May 04                                                                   S-3
Mon May 18  fmt sync target                                                   S-2
Mon Jun 01                                                                    S-1
Mon Jun 15  PR #16 code starts                                                 S 0
Mon Jun 29                                                                     S 1
Mon Jul 13  PR #17 starts                                                       S 2
Mon Jul 27                                                                      S 3
Mon Aug 10                                                                      S 4
Mon Aug 24                                                                      S 5
Mon Sep 07  PR #19 dashboards starts                                            S 6
Mon Sep 21                                                                      S 7
Mon Oct 05  PR #20 loadgen starts                                              S 8
Mon Oct 19                                                                      S 9
Mon Nov 02  PR #21 public-bench starts                                          S10
Mon Nov 16                                                                       S11
Mon Nov 30                                                                       S12

2027 H1 / Sprints S13..S24
Mon Jan 04  PR #22 SLO + rollup starts                                         S13
Mon Jan 18                                                                      S14
Mon Feb 01  Bench Phase 3 public starts                                         S15
Mon Feb 15  Public bench live                                                   S16
Mon Mar 01  PR #23 smoke pin lands                                              S17
Mon Mar 15  PR #24 publish lands                                                S18
Mon Mar 29  PR #25 lessons lands ; pre-L2 30-day soak begins                    S19
Mon Apr 12  L2 ceremony prep                                                    S20
Mon Apr 26  L2 ceremony 04-30                                                    S21
Mon May 10  Pre-GA soak begins                                                  S22
Mon May 24  GA prep ; launch Jun 15                                              S23
Mon Jun 07  GA STABILIZATION                                                     S24
```

---

## End of master roadmap

This roadmap supersedes (a) `EFE_A1_PHASE2_ROADMAP.md` for the Phase 2
work-tracking portion (b) and the cross-cutting workstream coordination
absent in prior phase-specific docs.

It is **frozen** at 2026-05-15. Tactical updates via weekly retros. Strategic
refresh at the quarterly cadence (Sep 30 2026, Dec 30 2026, Mar 30 2027, Jun 30
2027 — with the final refresh becoming part of the L2 + GA retrospective).
