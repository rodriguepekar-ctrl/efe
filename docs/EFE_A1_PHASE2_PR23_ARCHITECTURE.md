# EFE-A1 Phase 2 — PR #23 : `efe-smoke` — 9 smoke checks (every-PR merge gate)

| Field | Value |
|---|---|
| **PR** | #23 (Sprint A1 Phase 2 / P2.8 W15 → P2.8 W16) |
| **Branch** | `efe-a1/p2-pr23-smoke` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team — release engineering area (lead) + SRE (co-lead) + perf-eng |
| **Reviewers** | 2× release-eng + 2× SRE + 1× security + 1× perf-eng + 1× compliance |
| **Depends on** | PR #5 (proto), PR #6 (gRPC scaffolds), PR #9 (controller/participant business logic — provides the L2 surface the smoke suite gates on), PR #14 (Helm production-grade), PR #16 (mTLS), PR #17 (Prometheus metrics — 14 STABLE families enumerated by S6), PR #18 (OTel propagation — span chain validated by S7), PR #19 (recording rules + alerts), PR #20 (`efe-meta-swarm-loadgen` — consumed as a library for the request-issuing side of S2 / S3 / S4 / S5), PR #21 (`efe-bench-harness` — its `cluster::ClusterProvisioner` abstraction is **promoted to a shared crate** in this PR — see § 17), PR #22 (`efe-slo-rollup` — smoke pass/fail is consumed by SLO-1 numerator as a fast-fail gate) |
| **Blocks** | GA v1.0 (no public release without a hard merge-gate that proves every component still works on every commit), PR #24 (Phase-3 deploy automation — needs a reliable green-bit signal from smoke), customer trust (no shipping a chart with a red smoke status badge) |
| **L2 gate** | G20 (production-grade smoke surface : 9 closed-taxonomy checks + frozen JSON result schema + Rust `efe-smoke` binary + 2 CI workflows (PR + nightly) + GitHub-Pages badge `https://efe.github.io/smoke/badge.svg` + STABILITY_CONTRACT v1.5 § 23 commitment + < 3 min total wall-clock budget on a standard CI runner) |
| **Threat IDs in scope** | T106 – T115 (10 new : (T106) smoke flake-storm gate-bypass, (T107) credential leak in smoke logs, (T108) smoke mis-targeted at production cluster, (T109) baseline drift undetected, (T110) gate-bypass via `bench-skip` label, (T111) unbounded cluster DoS on CI, (T112) test secrets committed in git, (T113) smoke uses prod network egress, (T114) smoke clobbers shared PVC, (T115) smoke as timing oracle) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS3 + § 16 (CI quality gates) + § 24 (Release readiness) |
| **CVSS bandwidth** | All ≤ SEV-2 (smoke runs in ephemeral CI sandboxes ; no PII surface ; failed smoke ≠ user-visible incident) |
| **Crate version** | New crate `efe-smoke` v0.1.0 at `smoke/runner/`, binary `efe-smoke`.  **Promoted** : `efe-cluster-provisioner` v0.1.0 at `crates/efe-cluster-provisioner/` (extracted from PR #21 `bench/harness/src/cluster/`). |
| **Existing infrastructure consumed** | `tools/loadgen` (PR #20 — gRPC request engine for S2-S5) ; `bench/harness/src/cluster/` (PR #21 — extracted into shared crate) ; `deploy/charts/efe-meta-swarm/` (PR #14 — Helm chart under test) ; `crates/efe-meta-swarm-server` (PR #6+#9 — the binary under smoke) ; `deploy/mtls/` (PR #16 — cert library) |
| **STABILITY_CONTRACT v** | 1.4 → **1.5** (new tier : *Smoke check IDs* + *Smoke result JSON schema* + *Smoke thresholds* + *Cluster-provisioner crate API*) |
| **Total LOC budget** | Rust `efe-smoke` crate ~4 800 ; 9 check kernels ~1 800 ; cluster orchestrator ~720 ; result aggregator ~520 ; CLI ~480 ; provisioner extraction + new shared crate ~880 ; Helm chart (in-cluster runner) ~420 ; manifest library (9 K8s manifests under `smoke/manifests/`) ~1 040 ; CI workflows ~480 ; STABILITY_CONTRACT v1.5 addendum ~360 ; tests ~2 100 ; runbook + public README + smoke badge generator ~880 ; docs ~3 240 → **~17 720 LOC** |
| **CI cost delta** | + **24 s** for `cargo test -p efe-smoke --lib` (unit kernels — no I/O) ; + **2 m 48 s** for `smoke-pr` workflow on every PR (8 m hard cap) ; + **6 m 12 s** for `smoke-nightly` (extended scenarios — separate runner, not blocking PRs) ; total **PR-blocking CI delta + 3 m 12 s** (committed against the 5 m budget reserved by Phase-2 roadmap § WS3.6) |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals + scope
   2. Pre-flight inventory — what already exists vs. what's new
   3. Branch + commit plan (12 commits across 2 sprints)
   4. Architectural decisions (AD1 – AD56)
   5. Operator personas + 11 use cases
   6. The 9 frozen smoke checks (taxonomy + threshold + wall-clock)
   7. Per-check design (S1 – S9, deep dive)
   8. Smoke check orchestrator (`efe-smoke` Rust binary)
   9. Smoke result JSON schema v1
  10. Cluster-provisioner extraction (PR #21 → shared crate `efe-cluster-provisioner`)
  11. PR #20 loadgen consumer contract (request-side of S2-S5)
  12. PR #22 SLO-rollup coupling (smoke as fast-fail SLI cell)
  13. CLI design — 5 subcommands (run, list, validate, gate, version)
  14. Manifest library (`smoke/manifests/`)
  15. Helm chart for in-cluster smoke (Phase-2 optional)
  16. CI workflows (`smoke-pr.yml`, `smoke-nightly.yml`)
  17. STABILITY_CONTRACT v1.5 additions
  18. Security model + Kubernetes RBAC + cluster isolation
  19. Threat-model coverage (T106 – T115)
  20. Acceptance criteria (AC1 – AC34)
  21. Tests strategy (unit kernel + integration kind-cluster + golden + smoke-of-smoke)
  22. Risks + mitigations
  23. Rollback strategy
  24. Sequence diagrams (PR-trigger flow, nightly flow, failure investigation flow)
  25. Reviewer checklist
  26. Appendix A — CLI flag reference
  27. Appendix B — per-check pass criteria (formal)
  28. Appendix C — smoke result JSON schema (annotated)
  29. Appendix D — Manifest library inventory
  30. Appendix E — Smoke badge SVG template
  31. Appendix F — Flake-storm mitigation playbook (T106)
  32. Appendix G — Phase-3 follow-ups (G.1 – G.10)

---

## 0. TL;DR

PR #21 ships the **heavy** benchmark suite (10 scenarios, 5min–2h each).
PR #22 ships the **continuous SLO** surface.  What's missing is the
**fast merge-gate** : a sub-3-minute suite of pass/fail checks that
runs on **every PR** and **every push to main**, hard-blocks merge on
failure, and is the only thing standing between a broken build and
the customer's running cluster.

PR #23 ships :

- **1 Rust crate** `efe-smoke` (binary + library) with **9 closed-
  taxonomy checks** (S1–S9), each ≤ 20 s wall-clock, sum ≤ 3 min.
- **1 shared crate** `efe-cluster-provisioner` (extracted from PR
  #21's `bench/harness/src/cluster/` — DRY for kind / k3s / external
  kubeconfig provisioning), consumed by PR #21 + PR #23.
- **9 K8s manifest sets** under `smoke/manifests/{s1..s9}/` (one
  directory per check, kustomize-friendly).
- **2 CI workflows** : `smoke-pr.yml` (every PR, hard gate) +
  `smoke-nightly.yml` (extended scope, advisory).
- **Smoke badge** at `https://efe.github.io/smoke/badge.svg`
  refreshed nightly + on every push to main.
- **STABILITY_CONTRACT v1.5 § 23** (this PR) committing the 9 check
  IDs, thresholds, result JSON schema, and CLI surface as frozen.

The 9 checks (each frozen + thresholded — see § 6) :

| ID | Name | Validates | Budget |
|---|---|---|---|
| **S1** | boot | controller + 3 participants ready | ≤ 30 s |
| **S2** | L1 SwarmService | 5 canonical L1 RPCs respond < 500 ms p99 | ≤ 15 s |
| **S3** | L2 meta-swarm | controller↔participant RPCs respond < 1 s p99 | ≤ 20 s |
| **S4** | mTLS rotation | cert rotation mid-traffic : 0 dropped requests | ≤ 25 s |
| **S5** | SIGHUP reload | manifest reload : 0 dropped in-flight requests | ≤ 20 s |
| **S6** | Prometheus metrics | 14 STABLE families exposed (PR #17 contract) | ≤ 10 s |
| **S7** | OTel propagation | trace propagates controller → participant → backend < 5 s | ≤ 15 s |
| **S8** | restart resilience | pod restart : recovers within drain deadline | ≤ 30 s |
| **S9** | graceful shutdown | SIGTERM drains within `shutdown_deadline` | ≤ 15 s |
| **Σ** | total | including cluster boot (~ 60 s) | **≤ 3 min** |

Smoke is **NOT** :

- a performance bench (PR #21 covers that),
- a long-running soak (PR #21 S09 covers that),
- a security pen-test (PR #16 covers mTLS, PR #25 will cover audit),
- an SLO compliance check (PR #22 covers that).

Smoke IS **the green-bit gate** : if smoke is green, the PR is
**safe to merge** w.r.t. obvious regressions ; bench / SLO are still
needed for the full release-readiness signal.

---

## 1. Goal + non-goals + scope

### 1.1 Goal

Deliver the **complete, frozen, fast, hard-blocking merge gate** for
the EFE Phase-2 meta-swarm gRPC L1+L2 plane :

  - 9 closed-taxonomy smoke checks (§ 6, § 7),
  - Rust orchestrator + binary (§ 8),
  - Result JSON schema v1 (§ 9),
  - Shared cluster-provisioner crate (§ 10),
  - 5-subcommand CLI (§ 13),
  - 2 CI workflows + GitHub-Pages badge (§ 16),
  - STABILITY_CONTRACT v1.5 § 23 commitment (§ 17).

### 1.2 Non-goals (Phase-3 — Appendix G)

  - **Cross-region smoke** (G.1).
  - **Smoke vs Phase-1 binary** (compatibility regression — G.2).
  - **Smoke-driven canary rollout** (G.3).
  - **Smoke-as-a-service** (multi-tenant hosted — G.4).
  - **GPU smoke** (CUDA dependencies — G.5).
  - **Smoke with chaos faults** (PR #21 chaos scenarios already cover
    this in the heavyweight tier — G.6).
  - **Multi-arch smoke** (arm64 vs amd64 — G.7).
  - **Smoke with full L1 swarm — > 3 participants** (G.8).
  - **Smoke as IDE pre-commit hook** (G.9).
  - **Smoke result-as-Helm-test** (`helm test` integration — G.10).

### 1.3 Scope boundary vs PR #21 (bench)

| Concern | PR #21 bench | PR #23 smoke |
|---|---|---|
| **Trigger** | PR label `bench` + nightly | Every PR + every push |
| **Wall-clock** | 5 min – 2 h | < 3 min total |
| **Cluster** | Multi-node kind, 5 controllers + 15 participants | Single-node kind, 1 controller + 3 participants |
| **Output** | Per-metric tolerance + verdict + PR comment | Pass/fail per check + summary badge |
| **Failure mode** | Verdict=fail blocks merge (advisory in fast tier) | Any S1-S9 fail blocks merge **immediately** |
| **Stability** | v1.3 § 21 | v1.5 § 23 (this PR) |
| **Shared infra** | cluster::ClusterProvisioner | **promoted** to `efe-cluster-provisioner` crate |

### 1.4 Scope boundary vs PR #22 (SLO rollup)

PR #22 computes SLO compliance over rolling 30-day windows.  PR #23
smoke contributes to PR #22's `source=synthetic_smoke` SLI cell
(distinct from `source=prod`, `source=synthetic_loadgen`,
`source=bench` — AD2).  A failing smoke increments PR #22's burn-rate
counters but does **not** itself page operators ; that's the bench/SLO
job.  Smoke just **stops the bad code from landing in the first place**.

---

## 2. Pre-flight inventory

### 2.1 What exists today (CONSUMED, NOT REPLACED)

```
crates/
  efe-meta-swarm-proto/             (PR #5 — Tonic stubs)
  efe-meta-swarm-server/             (PR #6+#9 — controller + participant)
deploy/charts/efe-meta-swarm/        (PR #14 — Helm chart)
deploy/mtls/                         (PR #16 — cert generation)
tools/loadgen/                       (PR #20 — gRPC client lib)
bench/harness/                       (PR #21 — heavyweight bench)
  src/cluster/                       ← **EXTRACTED** to shared crate
slos/rollup/                         (PR #22 — SLO rollup)
.github/workflows/                    (existing CI : ci.yml, bench-*.yml, slo-*.yml)
```

### 2.2 What this PR delivers (NEW)

```
smoke/                                 # top-level, parallels bench/
├── README.md                          # public-facing
├── runner/                            # efe-smoke Rust crate
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs                    # CLI entry
│   │   ├── lib.rs                     # public re-exports + module map
│   │   ├── error.rs                   # SmokeError + 6 ExitCodes
│   │   ├── cli.rs                     # 5 subcommands (run/list/validate/gate/version)
│   │   ├── build.rs                   # version + git_sha
│   │   ├── orchestrator.rs            # boots cluster, sequences checks
│   │   ├── checks/
│   │   │   ├── mod.rs                 # Check trait + dispatch
│   │   │   ├── s1_boot.rs             # ≤ 30 s — controller + 3 participants ready
│   │   │   ├── s2_l1_swarm.rs         # ≤ 15 s — 5 canonical L1 RPCs
│   │   │   ├── s3_l2_meta_swarm.rs    # ≤ 20 s — L2 RPCs
│   │   │   ├── s4_mtls_rotation.rs    # ≤ 25 s — cert rotation mid-traffic
│   │   │   ├── s5_reload_sighup.rs    # ≤ 20 s — SIGHUP reload
│   │   │   ├── s6_metrics.rs          # ≤ 10 s — Prom scrape
│   │   │   ├── s7_otel.rs             # ≤ 15 s — OTel span chain
│   │   │   ├── s8_restart.rs          # ≤ 30 s — pod restart
│   │   │   └── s9_shutdown.rs         # ≤ 15 s — SIGTERM drain
│   │   ├── result/
│   │   │   ├── mod.rs                 # CheckResult + SmokeResult aggregate
│   │   │   ├── json.rs                # canonical JSON output
│   │   │   ├── markdown.rs            # PR-comment formatter
│   │   │   └── badge.rs               # gh-pages SVG badge
│   │   ├── gate.rs                    # CI gate decision logic (AD5)
│   │   ├── loadgen_client.rs          # PR #20 loadgen as a library
│   │   ├── metrics_scrape.rs          # Prom HTTP client for S6
│   │   └── otel_collector.rs          # OTLP collector probe for S7
│   ├── tests/
│   │   ├── golden_checks.rs           # 9 in-memory check kernels
│   │   ├── orchestrator_chain.rs      # check ordering + budget
│   │   ├── cli_golden.rs              # 5 subcommand surface
│   │   ├── badge_golden.rs            # SVG byte-stability
│   │   ├── stability_contract_v15.rs  # locks v1.5 § 23
│   │   └── data/                      # fixture YAMLs + golden JSONs
│   └── helm/                          # optional in-cluster runner (Phase-2 stub)
│       ├── Chart.yaml
│       ├── values.yaml
│       └── templates/
├── manifests/                         # K8s manifest library
│   ├── s1-boot/
│   │   ├── kustomization.yaml
│   │   ├── controller-deployment.yaml
│   │   ├── participant-statefulset.yaml
│   │   └── readiness-probe-overlay.yaml
│   ├── s2-l1-swarm/
│   ├── s3-l2-meta-swarm/
│   ├── s4-mtls-rotation/
│   ├── s5-reload-sighup/
│   ├── s6-metrics/
│   ├── s7-otel/
│   ├── s8-restart/
│   └── s9-shutdown/
└── schemas/
    └── smoke-result-v1.json

crates/
└── efe-cluster-provisioner/           # extracted from bench/harness/src/cluster/
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs
    │   ├── error.rs
    │   ├── kind.rs                    # kind-action emulator (Phase-2 invokes kind binary)
    │   ├── k3s.rs                     # k3sup invocation
    │   ├── external.rs                # kubeconfig passthrough
    │   ├── traits.rs                  # ClusterProvisioner trait
    │   └── compat.rs                  # bench/harness compatibility shim
    └── tests/
        └── provisioner_golden.rs

.github/workflows/
├── smoke-pr.yml                       # on PR + push:main — hard gate
└── smoke-nightly.yml                   # extended scope — advisory

docs/
├── EFE_A1_PHASE2_PR23_ARCHITECTURE.md  # THIS DOC
├── STABILITY_CONTRACT.md
│   └── § 23 (v1.5 — Smoke surface)    # NEW
└── runbooks/
    └── smoke-flake-storm.md            # T106 mitigation playbook

bench/harness/Cargo.toml                # diff : depends on efe-cluster-provisioner
bench/harness/src/cluster/mod.rs        # diff : re-exports from new crate (compat shim)
```

### 2.3 The cluster-provisioner extraction

PR #21 implemented `bench/harness/src/cluster/{kind,k3s,external}.rs`
as private modules.  PR #23 needs the **same** abstraction for smoke ;
duplicating is unacceptable (DRY).  Therefore :

  1. **Extract** the `ClusterProvisioner` trait + 3 impls into a new
     workspace crate `crates/efe-cluster-provisioner` v0.1.0.
  2. **Add a compat shim** in `bench/harness/src/cluster/mod.rs` that
     re-exports the symbols (so PR #21 callers keep working).
  3. **Tests** : the existing PR #21 tests continue to pass byte-
     identically (verified via `cargo test -p efe-bench-harness`
     before/after the extraction).
  4. **STABILITY_CONTRACT v1.5 § 23.6** freezes the trait's public
     surface (method names + signatures) for ≥ 6 chart-minor versions.

---

## 3. Branch + commit plan

| # | Commit | Scope | Approx LOC | Risk |
|---|---|---|---:|---|
| 1 | `chore: register smoke/runner + efe-cluster-provisioner crates in workspace` | `Cargo.toml` | 90 | none |
| 2 | `refactor(bench): extract cluster::ClusterProvisioner to efe-cluster-provisioner crate (PR #21 compat shim preserved)` | `bench/harness/src/cluster/**` + new crate | 880 | medium — PR #21 tests must remain green |
| 3 | `feat(smoke): Check trait + 9 check kernel shells (compile-only)` | `smoke/runner/src/checks/*.rs` | 540 | low |
| 4 | `feat(smoke): orchestrator + cluster wiring + per-check budget enforcement` | `smoke/runner/src/orchestrator.rs` + `lib.rs` | 720 | medium |
| 5 | `feat(smoke): S1 boot + S2 L1 + S3 L2 (the 3 most critical checks)` | `smoke/runner/src/checks/{s1,s2,s3}.rs` | 720 | medium |
| 6 | `feat(smoke): S4 mTLS rotation + S5 SIGHUP reload (in-flight preservation)` | `smoke/runner/src/checks/{s4,s5}.rs` | 540 | high — concurrent + signal handling |
| 7 | `feat(smoke): S6 Prom metrics scrape + S7 OTel span chain` | `smoke/runner/src/checks/{s6,s7}.rs` + `metrics_scrape.rs` + `otel_collector.rs` | 480 | medium |
| 8 | `feat(smoke): S8 restart resilience + S9 graceful shutdown` | `smoke/runner/src/checks/{s8,s9}.rs` | 420 | medium |
| 9 | `feat(smoke): result aggregator + JSON output + PR-comment markdown + gh-pages badge` | `smoke/runner/src/result/*.rs` | 620 | low |
| 10 | `feat(smoke): CLI (5 subcommands) + gate decision logic` | `smoke/runner/src/cli.rs` + `gate.rs` + `main.rs` | 580 | low |
| 11 | `feat(smoke): 9 K8s manifest sets (kustomize-friendly)` | `smoke/manifests/**` | 1 040 | medium |
| 12 | `ci(smoke): smoke-pr.yml + smoke-nightly.yml + gh-pages badge publish` | `.github/workflows/smoke-*.yml` | 480 | medium |
| 13 | `docs(smoke): STABILITY_CONTRACT v1.5 § 23 + runbook + public README + this arch doc` | `docs/**` + `smoke/README.md` | 3 600 | low |
| 14 | `test(smoke): unit kernels + golden + frozen v1.5 + smoke-of-smoke meta-test` | `smoke/runner/tests/**` | 2 100 | low |

**Total LOC** ≈ 12 810 (within budget of 17 720 — buffer covers
Helm chart + extra manifests + runbook iterations).

**Sprint allocation** :
- Sprint W15 (5 days) : commits 1 – 7 (foundation + 6 checks)
- Sprint W16 (5 days) : commits 8 – 14 (remaining checks + CI + docs)

---

## 4. Architectural decisions (AD1 – AD56)

### 4.1 Scope + taxonomy

- **AD1 — Closed 9-check taxonomy.** Exactly 9 checks, IDs frozen :
  S1 – S9.  Adding a 10th requires a major-version bump of
  STABILITY_CONTRACT.
- **AD2 — Smoke is `source=synthetic_smoke` for PR #22.**  Smoke
  results feed the SLO rollup as a distinct source label ; smoke
  never contributes to `source=prod`.
- **AD3 — 3-minute total wall-clock cap.**  Per-check budgets sum to
  ≤ 165 s ; the cluster boot adds ~ 60 s → 225 s ≤ 240 s
  (4-minute hard CI cap).  Operators may pass `--check-only S1,S2`
  to skip checks for debugging.
- **AD4 — No flaky checks tolerated.**  A check that fails > 1 % over
  100 PRs is flagged for **immediate removal** by SRE rotation
  (runbook § F).  Re-introduction requires a stability proof
  (proptest seed bank — see § 21).
- **AD5 — Hard gate, not advisory.**  `smoke-pr.yml` runs with
  `required: true` ; merge is blocked on any S1–S9 fail.  No `bench-
  skip` analog (T110 mitigation).
- **AD6 — Single-node cluster (kind).**  Smoke uses a single-node
  kind cluster ; multi-node coverage is bench's job (PR #21).
- **AD7 — Manifest library, not Helm.**  9 K8s manifest sets in
  `smoke/manifests/` ; each check applies its own.  Reasons :
  - Helm has too much state to be repeatable in < 20 s,
  - Kustomize overlays give fine-grained control,
  - PR #14's Helm chart **is** validated by S1 (we install it once,
    then layer overlays).

### 4.2 Cluster + isolation

- **AD8 — Each check runs in its own k8s namespace.**  Namespace is
  `efe-smoke-S{n}-{uuid}` ; deleted on completion + after timeout.
- **AD9 — Smoke rejects `mode=production` clusters.**  If the
  kubeconfig points to a cluster with a `efe.io/environment=production`
  label, smoke aborts with `ExitCode::Config` (T108 mitigation).
- **AD10 — Smoke writes nothing outside its own namespace.**  RBAC
  is strictly namespace-scoped (Role, not ClusterRole — T103/T108).
- **AD11 — Smoke never persists state.**  All PVCs created by smoke
  are deleted on cleanup ; smoke namespaces have a `efe.io/smoke=true`
  label so an orphan-sweep CronJob can reap leftovers (T114).
- **AD12 — Network egress is explicitly whitelisted.**  Only the
  cluster's in-namespace services + the kind node-local Prom can be
  reached.  No external HTTP (T113).

### 4.3 Check execution

- **AD13 — Each check has a max wall-clock budget.**  If exceeded,
  the check is marked `timeout` and the overall verdict is `fail`.
- **AD14 — Checks run sequentially, not in parallel.**  Single-
  node kind has limited CPU ; parallelism would inflate variance
  and increase flake rate.  Total budget compatible with sequential
  execution (Σ ≈ 180 s).
- **AD15 — Each check is idempotent.**  Re-running with the same
  inputs produces the same verdict (no random seeds in checks).
- **AD16 — Each check emits a structured `CheckResult` (§ 9.2).**
  Operators can `jq` the result without parsing logs.
- **AD17 — Each check has 1 retry on a transient signal.**  `connection
  refused` during the first 5 s of S2 / S3 is retried once ; thereafter
  considered a hard fail.  T106 — flake mitigation.
- **AD18 — A check failure logs the full pod logs of the failing
  service.**  Operators need the full diagnostic on first attempt.
- **AD19 — Checks are deterministic on the wall-clock.**  No
  `Instant::now()` -dependent thresholds ; only `Duration` comparisons.

### 4.4 Loadgen integration

- **AD20 — Loadgen as a library (not subprocess).**  Same pattern
  as PR #21 ; saves ~ 500 ms per check invocation.
- **AD21 — Loadgen profile per check is in-Rust, not YAML.**  Smoke
  checks are deterministic ; profile YAML loading adds I/O latency
  and a parse error surface that smoke can ill afford.
- **AD22 — Loadgen runs with `mode=dev`.**  `production` is
  hard-rejected (AD11 from PR #20).
- **AD23 — Loadgen seed is fixed per check.**  `seed=42` for S2,
  `seed=43` for S3, etc.  Stable across smoke runs ⇒ identical
  traffic patterns ⇒ identical pass/fail outcome modulo cluster
  variance.

### 4.5 Result format + reporting

- **AD24 — Smoke result JSON schema is frozen v1.5.**  Adding a
  field is non-breaking ; removing is a major bump.
- **AD25 — Pass/fail is the only verdict.**  No warn/indeterminate —
  smoke is binary by design.  Indeterminate cases (e.g. timeout)
  are explicitly `fail`.
- **AD26 — Each result carries a `git_sha` + `chart_version`.**
  Operators can correlate a failed smoke with the exact PR head SHA.
- **AD27 — PR-comment markdown is HTML-escaped (T89 from PR #21).**
  Smoke output goes into a GitHub PR comment ; same XSS guard.
- **AD28 — GH Pages badge is byte-stable.**  Like PR #22, the SVG
  generator is deterministic.
- **AD29 — JSON output goes to a path, not stdout, by default.**
  CI workflows read the path ; stdout is for humans only.

### 4.6 Gate decision logic

- **AD30 — `gate` subcommand is the **only** consumer of the result.**
  It reads the JSON, applies the threshold-only fail criterion (any
  S1-S9 fail = exit 1), and emits the merge-blocking exit code.
- **AD31 — Gate failure → ExitCode::SmokeFail = 1.**  Mirrors PR #21
  `ExitCode::BenchFail` for muscle-memory.
- **AD32 — Gate accepts `--allow-skip` for explicit operator override.**
  Used only by the release-eng rotation for emergency unblocks ;
  logged with full reason in the GitHub audit log (T110 mitigation).
- **AD33 — `--allow-skip` requires `--reason <text>`.**  CI workflow
  rejects empty reasons.

### 4.7 Crate extraction (`efe-cluster-provisioner`)

- **AD34 — Trait surface is frozen v1.5 § 23.6.**  Adding a method
  is breaking ; only adding default-impl methods is non-breaking.
- **AD35 — 3 impls : kind / k3s / external.**  Same set as PR #21.
- **AD36 — Async trait.**  All methods return `Pin<Box<Future<...>>>`
  via `#[async_trait]`.  No sync escape hatch.
- **AD37 — Error type is `ProvisionError` (re-exported).**  PR #21
  `BenchError::Cluster` and PR #23 `SmokeError::Cluster` both wrap it.
- **AD38 — Compat shim in `bench/harness`.**  `bench/harness/src/
  cluster/mod.rs` becomes a 3-line `pub use efe_cluster_provisioner::*;`
  so existing PR #21 callers continue to work without changes.

### 4.8 Stability + ops

- **AD39 — STABILITY_CONTRACT v1.5 § 23 freezes 9 check IDs + 9
  thresholds + result schema + CLI + cluster-provisioner trait** for
  ≥ 6 chart-minor versions.
- **AD40 — Smoke result schema v1 is forward-compat.**  Unknown
  fields are ignored on read ; future versions add fields without
  bumping major.
- **AD41 — Smoke binary `--version` reports stability tier.**
  `efe-smoke 0.1.0\n  stability : v1.5 (PR #23 § 23)`.

### 4.9 CI integration

- **AD42 — `smoke-pr.yml` runs on PR + push:main.**  Both flows use
  the same workflow file (parameterized by `event_name`).
- **AD43 — Smoke artefacts uploaded for 14 days.**  Includes pod
  logs from all failed checks (T106 — fast diagnosis).
- **AD44 — Smoke badge is published only on push:main.**  PR
  workflows do not update the badge (avoid race conditions).
- **AD45 — Smoke status is propagated to the PR check API.**  GitHub
  shows "smoke / S1-boot ✅" etc. as 9 separate status checks.

### 4.10 Loadgen + metrics + OTel

- **AD46 — Smoke metrics use `source=synthetic_smoke` label.**  Same
  AD18 from PR #22 ; smoke output is distinguishable from
  loadgen-bench output.
- **AD47 — S6 validates 14 specific metric families.**  Closed
  taxonomy frozen in PR #17 STABILITY_CONTRACT v1.0 § 4 ; any
  missing family is a fail.
- **AD48 — S7 verifies span chain length.**  A trace must contain
  at least 1 controller span → 1 participant span (cardinality
  proves propagation).

### 4.11 Manifest library

- **AD49 — Each manifest set is kustomize-buildable.**  Operators
  can `kustomize build smoke/manifests/s1-boot/` to inspect.
- **AD50 — Manifest sets reuse PR #14's Helm chart as base.**  S1
  installs the chart ; S2-S9 apply overlays over the same release.
- **AD51 — Manifests have `efe.io/smoke=true` label.**  Orphan-sweep
  CronJob filters on this label.

### 4.12 Security + RBAC

- **AD52 — Smoke pods run as non-root + read-only rootfs.**  Mirror
  PR #21 AD24 ; `seccompProfile=RuntimeDefault`.
- **AD53 — Smoke ServiceAccount has Role (NOT ClusterRole).**  T108
  mitigation.
- **AD54 — kubeconfig is mounted read-only.**  Smoke can read but
  not modify cluster-wide RBAC.
- **AD55 — Test secrets are generated in-cluster, not committed.**
  T112 mitigation.  Smoke generates a fresh mTLS CA per run, scopes
  it to its namespace, and destroys it on cleanup.
- **AD56 — No timing oracle.**  S4 (cert rotation) uses jittered
  request times to avoid leaking the rotation timing through
  response-latency variance (T115).

---

## 5. Operator personas + 11 use cases

### 5.1 Personas

| Persona | Goal | Tool used | Output |
|---|---|---|---|
| **Contributor (PR author)** | Confirm smoke passes before requesting review | local `cargo run -p efe-smoke -- run` | green ✅ or actionable error |
| **Reviewer** | See smoke status in the PR | GH Pages badge in PR comment | green ✅ + per-check breakdown |
| **Release engineer** | Pre-release smoke check across all 9 checks | `efe-smoke run --chart-version v0.7.0-rc1` | summary + JSON for ratcheting |
| **SRE on-call (smoke red)** | Diagnose a flaky smoke | `efe-smoke run --check S4 --keep-cluster` | pod logs + raw events |
| **CI engineer** | Tune smoke duration | `efe-smoke run --time-budget 90s` | per-check timing histogram |
| **Customer / sales** | Public proof of merge-gate quality | `https://efe.github.io/smoke/badge.svg` | green ✅ badge |
| **Security reviewer** | Verify no secret leak | `efe-smoke run --redact-logs` | logs with `[REDACTED]` substitution |
| **Compliance auditor** | Evidence smoke ran for v0.6.0 release | `smoke-result.json` in the release tarball | signed JSON |
| **Phase-3 ML engineer** | Train flake-classifier | `efe-smoke list --format jsonl | …` | dataset of 1000s of smoke results |

### 5.2 Use cases

1. **UC-1 — PR author runs smoke locally** : `cargo run -p efe-smoke -- run --provisioner kind` (≤ 4 min on laptop) → green ✅.
2. **UC-2 — PR triggers `smoke-pr.yml`** : workflow runs on `pull_request`, posts a comment with the 9-check matrix, status check `smoke / overall` is required.
3. **UC-3 — Push to main triggers `smoke-pr.yml`** : same as UC-2 ; if green, the gh-pages badge is refreshed.
4. **UC-4 — Smoke fails on S4 mTLS rotation** : workflow fails ; PR comment shows `❌ S4 mTLS rotation : 2 requests dropped during cert switchover` ; pod logs uploaded as artefact ; reviewer asks PR author to fix.
5. **UC-5 — Release-eng pre-cuts v0.7.0** : runs `efe-smoke run --chart-version 0.7.0-rc1 --provisioner external --kubeconfig ~/.kube/staging --output /tmp/smoke.json` ; reviews ; signs off.
6. **UC-6 — SRE on-call diagnoses a flake** : ad-hoc workflow rerun with `--keep-cluster --log-format json` ; ssh into kind node to inspect.
7. **UC-7 — Smoke detects regression in S6 metrics** : a recent PR removed a STABLE metric family ; S6 fails with `missing metric family: efe_l2_request_duration_seconds_bucket` ; reviewer rejects the PR.
8. **UC-8 — Smoke contributes to SLO** : PR #22's nightly cron picks up smoke results from `https://efe.github.io/smoke/history/` and includes them as `source=synthetic_smoke` SLI cells.
9. **UC-9 — Customer sales asks "do you have green CI?"** : URL to badge in proposal.
10. **UC-10 — Security review** : `--redact-logs` confirms no secrets emitted in stderr.
11. **UC-11 — Compliance audit** : release tarball contains `smoke-result-v0.7.0.json` signed with cosign keyless OIDC (Phase-3 hardening — Appendix G.4).

---

## 6. The 9 frozen smoke checks

| ID | Name | Wall-clock budget | Primary signal | Frozen target |
|---|---|---|---|---|
| **S1** | `boot` | ≤ 30 s | controller `/healthz` returns 200 + 3 participants in `Ready` state | controller-replicas ≥ 1 AND participant-replicas == 3 |
| **S2** | `l1_swarm` | ≤ 15 s | 5 canonical L1 SwarmService RPCs respond | p99 < 500 ms ; error_rate == 0 ; all 5 RPCs receive 200 OK |
| **S3** | `l2_meta_swarm` | ≤ 20 s | meta-swarm gRPC L2 RPCs respond | p99 < 1 s ; error_rate == 0 ; controller-to-participant chain works |
| **S4** | `mtls_rotation` | ≤ 25 s | TLS cert rotation mid-traffic | 0 dropped requests ; latency p99 increase ≤ 50 ms during rotation |
| **S5** | `reload_sighup` | ≤ 20 s | SIGHUP reload preserves in-flight | 0 dropped requests ; manifest version observed before+after |
| **S6** | `metrics` | ≤ 10 s | Prometheus scrape exposes 14 STABLE families | every family from PR #17 STABILITY v1.0 § 4 present |
| **S7** | `otel` | ≤ 15 s | OTel trace propagates end-to-end | trace contains ≥ 1 controller span AND ≥ 1 participant span |
| **S8** | `restart` | ≤ 30 s | pod restart resumes within drain deadline | participant pod restart : new pod `Ready` within 30 s ; no quorum loss |
| **S9** | `shutdown` | ≤ 15 s | SIGTERM completes drain within `shutdown_deadline` | controller exits with code 0 within 10 s ; in-flight requests complete |
| **Σ** | | **≤ 180 s + ~60 s cluster boot = ≤ 240 s** | | total ≤ 4 min hard cap |

### 6.1 Why these 9 specifically

- Each check covers one **failure class** that has historically caused a Phase-1 production incident (see internal post-mortem catalog) :
  - S1 — boot loop (incident I-2025-04-12)
  - S2 — L1 contract violation (I-2025-06-03)
  - S3 — L2 chain break (I-2025-09-21)
  - S4 — cert rotation drop (I-2025-11-08, the famous one)
  - S5 — manifest reload race (I-2025-12-15)
  - S6 — Prom metric rename regression (I-2026-01-29)
  - S7 — OTel propagation loss (I-2026-02-14)
  - S8 — restart cascade (I-2026-03-07)
  - S9 — shutdown leak (I-2026-04-11)
- Cognitive load : 9 ≈ √(100 incidents) — operators can keep them in head.
- Each check ≤ 30 s ⇒ deterministic per-PR cost.

### 6.2 Why these specific thresholds

| Check | Threshold | Rationale |
|---|---|---|
| S1 | boot < 30 s | 1× container-runtime cold start ; cluster < this would mean CPU starvation |
| S2 | p99 < 500 ms | 10× the SLO target (50 ms) — smoke checks "does it work", not "is it fast" |
| S3 | p99 < 1 s | Same reasoning, looser because L2 has 1 hop extra |
| S4 | 0 dropped requests | Hard constraint : cert rotation must be glass-smooth |
| S5 | 0 dropped requests | Same — reload must be non-disruptive |
| S6 | 14 families | Frozen by PR #17 STABILITY_CONTRACT v1.0 § 4 |
| S7 | ≥ 2 spans in chain | Minimum for propagation proof ; full multi-hop is bench's job |
| S8 | restart < 30 s | Drain deadline + image pull |
| S9 | exit code 0 within 10 s | Bash-friendly assertion |

---

## 7. Per-check design

### 7.1 S1 — `boot`

**Pre-conditions** :
  - Kind cluster created (~ 45 s before this check starts).
  - PR #14 Helm chart applied (`helm install efe-meta-swarm`).
  - Smoke namespace created : `efe-smoke-s1-{uuid}`.

**Steps** :
  1. Wait for controller pod to reach `Running` (timeout 20 s).
  2. Curl `controller:8080/healthz` and assert HTTP 200.
  3. Wait for 3 participant pods (`Ready` condition true).
  4. Each participant exposes `:8081/healthz` ; assert 200.
  5. Verify controller log line `participants_ready=3` appears.

**Pass criteria (formal)** :
  - `t_total ≤ 30 s`
  - `controller.healthz == 200`
  - `count(participants.ready) == 3`
  - `participant.healthz[*] == 200`
  - Log line regex `participants_ready=3` matches in controller stdout.

**Failure modes** :
  - Image pull timeout → diagnose via `kubectl events`.
  - Init-container failure → upload init logs.
  - Crash loop → upload all container logs + restart count.

### 7.2 S2 — `l1_swarm`

**Pre-conditions** : S1 passed.

**Steps** : Invoke the 5 canonical L1 SwarmService RPCs (in order) :
  1. `CreateSwarm` (idempotent — name = `smoke-s2-{uuid}`)
  2. `JoinSwarm`
  3. `ListSwarms`
  4. `LeaveSwarm`
  5. `DeleteSwarm`

Each via PR #20 loadgen as a library.  Fixed seed = 42.

**Pass criteria** :
  - All 5 RPCs return `OK`.
  - `error_rate == 0.0`.
  - `p99(rpc_latency) < 500 ms`.

**Failure modes** :
  - Status `UNAVAILABLE` → controller crashed mid-test.
  - Status `DEADLINE_EXCEEDED` → CPU starvation ; flag for re-run.
  - Status `PERMISSION_DENIED` → mTLS cert misconfigured.

### 7.3 S3 — `l2_meta_swarm`

**Pre-conditions** : S1 passed.

**Steps** : Invoke L2 meta-swarm RPCs :
  1. `RegisterParticipant` (3× from each participant)
  2. `Heartbeat` (3× from each participant)
  3. `ParticipantUpdate` (1× from controller)
  4. `ListParticipants` (1× from controller)

Seed = 43.

**Pass criteria** :
  - All RPCs return `OK`.
  - `error_rate == 0.0`.
  - `p99 < 1 s`.
  - Controller log confirms `participant_chain_complete=true`.

### 7.4 S4 — `mtls_rotation`

**Pre-conditions** : S1 passed.  Initial cert (CA-A, cert-A) issued
to controller + participants.

**Steps** :
  1. Start a sustained 50 QPS request stream (loadgen library).
  2. After 5 s of steady traffic, rotate the cert (CA-A → CA-B via
     `kubectl rollout restart` with new Secret).
  3. Continue stream for 10 s.
  4. Stop stream.
  5. Verify 0 dropped requests during rotation (count via loadgen
     `RunSummary.total_failed`).

**Pass criteria** :
  - `total_failed == 0`.
  - `p99 during rotation ≤ p99 before + 50 ms` (anti-T115 timing
     oracle : jittered request times prevent leak).
  - Cert CN observed by client BEFORE rotation matches CA-A.
  - Cert CN observed AFTER matches CA-B.

### 7.5 S5 — `reload_sighup`

**Pre-conditions** : S1 passed.

**Steps** :
  1. Read controller's current manifest version
     (`/manifest-version` endpoint).
  2. Send `kubectl exec controller -- kill -HUP 1` (which the
     server interprets as "reload config").
  3. During the next 5 s, run 100 RPCs.
  4. Read controller's manifest version again.
  5. Assert version incremented + 0 dropped requests.

**Pass criteria** :
  - `manifest_version_post > manifest_version_pre`.
  - `total_failed == 0`.

### 7.6 S6 — `metrics`

**Pre-conditions** : S1 passed.

**Steps** :
  1. Curl `controller:8080/metrics` (Prom scrape).
  2. Parse the output (text exposition format).
  3. Assert every STABLE family from PR #17 is present.
  4. Curl `participant:8081/metrics` and repeat.

**Frozen STABLE metric families** (from PR #17 STABILITY_CONTRACT v1.0 § 4) :
```
efe_l2_request_total
efe_l2_request_duration_seconds_bucket
efe_swarm_recovery_event_total
efe_swarm_recovery_seconds
efe_replication_lag_bucket
efe_replication_lag
efe_controller_participants_total
efe_controller_uptime_seconds
efe_participant_join_total
efe_participant_uptime_seconds
efe_l2_active_connections
efe_l2_tls_handshakes_total
efe_l2_tls_handshakes_failed_total
efe_swarm_size_current
```

(14 families exact — count locked by `tests/stability_contract_v15.rs`)

**Pass criteria** :
  - 14 families present on the controller scrape.
  - 14 families present on at least 1 participant scrape (some
     families only on participants).

### 7.7 S7 — `otel`

**Pre-conditions** : S1 passed.  In-cluster OTel collector running
(deployed by S1's overlay).

**Steps** :
  1. Send a single L2 RPC with `traceparent` header set.
  2. Wait 5 s for the collector to receive the span.
  3. Query the collector's HTTP API : `/v1/traces?trace_id=<hex>`.
  4. Parse the response ; count spans.

**Pass criteria** :
  - Trace returned by collector contains ≥ 2 spans.
  - At least one span has `service.name=efe-meta-swarm-controller`.
  - At least one span has `service.name=efe-meta-swarm-participant`.
  - Span chain length ≤ 4 (sanity — avoid infinite recursion).

### 7.8 S8 — `restart`

**Pre-conditions** : S1 passed.  Steady-state traffic at 10 QPS via
loadgen (background).

**Steps** :
  1. `kubectl delete pod participant-0 --grace-period=10`.
  2. Wait for new pod to reach `Ready` (timeout 30 s).
  3. Verify controller's `efe_controller_participants_total` returns
     to 3 within 30 s.
  4. Verify the background loadgen stream had `error_rate < 5 %` during
     the restart window (a brief uptick is acceptable ; sustained loss is not).

**Pass criteria** :
  - New pod `Ready` within 30 s.
  - `participants_total == 3` within 30 s after restart.
  - `error_rate < 5%` over the 30-s restart window.

### 7.9 S9 — `shutdown`

**Pre-conditions** : S1 passed.

**Steps** :
  1. Start a stream of 20 long-running RPCs (200 ms each).
  2. After 2 s, send `kubectl rollout restart deployment efe-meta-
     swarm-controller`.
  3. Wait up to 15 s for the old controller pod to terminate.
  4. Verify exit code 0 in the terminated pod's status.
  5. Verify all 20 RPCs completed (or returned `UNAVAILABLE` with
     graceful retry, NOT abruptly cut).

**Pass criteria** :
  - Old pod exits with code 0 within 10 s.
  - `loadgen.RunSummary.gracefully_completed == 20`.
  - No `connection reset` errors in stream.

---

## 8. Smoke check orchestrator

```rust
// smoke/runner/src/orchestrator.rs

pub struct SmokeOrchestrator {
    provisioner: Box<dyn ClusterProvisioner>,
    checks: Vec<Box<dyn Check>>,
    config: SmokeConfig,
}

pub struct SmokeConfig {
    pub chart_version: String,
    pub keep_cluster_on_failure: bool,
    pub redact_logs: bool,
    pub per_check_timeout: Duration,
    pub global_timeout: Duration,    // 240 s hard cap (AD3)
}

#[async_trait]
pub trait Check: Send + Sync {
    fn id(&self) -> SmokeCheckId;
    fn budget(&self) -> Duration;
    async fn run(&self, ctx: &CheckContext) -> CheckResult;
}

pub enum SmokeCheckId {
    S1Boot,
    S2L1Swarm,
    S3L2MetaSwarm,
    S4MtlsRotation,
    S5ReloadSighup,
    S6Metrics,
    S7Otel,
    S8Restart,
    S9Shutdown,
}
```

The orchestrator:
1. Provisions the cluster (~60 s).
2. Installs PR #14 Helm chart with smoke overrides.
3. Iterates checks in order, enforcing per-check budget.
4. Aggregates results.
5. Emits JSON + markdown + (on push:main) updates the gh-pages badge.
6. Tears down cluster on completion (unless `--keep-cluster`).

---

## 9. Smoke result JSON schema v1

```jsonc
// smoke/schemas/smoke-result-v1.json
{
  "schema_version": 1,
  "chart_version": "0.6.0",
  "git_sha": "abc1234",
  "harness_version": "0.1.0",
  "run_id": "01J3F7XGGYM5NZ0CWY3Q7RH3JE",
  "started_at": "2026-05-16T03:00:00Z",
  "completed_at": "2026-05-16T03:03:12Z",
  "duration_s": 192.4,
  "overall_verdict": "pass",   // pass | fail
  "checks": {
    "S1": {
      "verdict": "pass",
      "duration_s": 28.7,
      "budget_s": 30.0,
      "details": { "controller_healthz": 200, "participants_ready": 3 }
    },
    "S2": {
      "verdict": "pass",
      "duration_s": 12.1,
      "budget_s": 15.0,
      "details": { "rpcs_succeeded": 5, "p99_ms": 87.2 }
    },
    // … S3-S9
  },
  "stability_contract_version": "1.5"
}
```

Schema frozen v1.5 § 23.5.

---

## 10. Cluster-provisioner extraction

The new crate `efe-cluster-provisioner` :

```rust
// crates/efe-cluster-provisioner/src/traits.rs

#[async_trait]
pub trait ClusterProvisioner: Send + Sync {
    async fn provision(&self) -> Result<PathBuf, ProvisionError>;
    async fn teardown(&self) -> Result<(), ProvisionError>;
    fn kind(&self) -> ProvisionerKind;
    fn name(&self) -> &str;
}

pub enum ProvisionerKind { Kind, K3s, External }

// Compat shim in bench/harness/src/cluster/mod.rs
pub use efe_cluster_provisioner::{ClusterProvisioner, ProvisionerKind};
pub use efe_cluster_provisioner::kind::KindProvisioner;
// ...
```

Migration is **byte-identical** for PR #21 callers — the public symbols match exactly.  This is enforced by a CI compatibility test (`tests/compat.rs` runs the PR #21 test suite against the new crate).

---

## 11. PR #20 loadgen consumer contract

`smoke/runner/src/loadgen_client.rs` :

```rust
use efe_meta_swarm_loadgen::{run as loadgen_run, LoadgenConfig};

pub async fn s2_l1_traffic(target_url: &str) -> Result<RunSummary, ...> {
    let cfg = LoadgenConfig::build_minimal(target_url, 50, "constant");
    cfg.seed = 42;  // AD23 — frozen per check
    cfg.duration = Duration::from_secs(10);
    cfg.mode = RunMode::Dev;  // AD22
    loadgen_run(cfg).await
}
```

---

## 12. PR #22 SLO-rollup coupling

The `smoke-pr.yml` workflow on push:main also POSTs the JSON to a
SLO collector endpoint (if `SLO_COLLECTOR_URL` env is set).  PR #22's
rollup binary then ingests smoke results via the `source=synthetic_smoke`
label.  This is an **additive** integration — smoke does not depend
on PR #22 being deployed.

---

## 13. CLI design

5 subcommands (STABILITY_CONTRACT v1.5 § 23.4 — frozen) :

```
efe-smoke run        # one-shot, runs all 9 checks
efe-smoke list       # list 9 checks + their budgets
efe-smoke validate   # schema-validate a smoke-result.json
efe-smoke gate       # read a smoke-result.json, exit 0/1 per AD30
efe-smoke version    # print version + git_sha + stability tier
```

Critical flags (frozen) :

| Flag | Purpose |
|---|---|
| `--chart-version <VERSION>` | Required.  The chart under test. |
| `--provisioner <KIND>` | `kind` (default) \| `k3s` \| `external` |
| `--kubeconfig <PATH>` | Required for `external`. |
| `--check <ID>` (repeatable) | Subset of checks (default = all 9). |
| `--keep-cluster` | Don't tear down on completion (debug). |
| `--keep-cluster-on-failure` | Tear down only if green. |
| `--per-check-timeout <DURATION>` | Override AD3 budget per-check. |
| `--global-timeout <DURATION>` | Override 240 s hard cap. |
| `--output <PATH>` | smoke-result.json path. |
| `--pr-comment-file <PATH>` | Markdown PR comment. |
| `--badge-file <PATH>` | gh-pages SVG. |
| `--redact-logs` | T112 — substitute secret-like strings with `[REDACTED]`. |
| `--allow-skip` + `--reason <TEXT>` | Gate override (AD32-33). |

---

## 14. Manifest library

```
smoke/manifests/s1-boot/
  kustomization.yaml
  controller-deployment.yaml     # overlay onto PR #14 base chart
  participant-statefulset.yaml
  readiness-probe-overlay.yaml   # short probes (boot fast)
smoke/manifests/s4-mtls-rotation/
  kustomization.yaml
  ca-a-secret.yaml
  ca-b-secret.yaml
  rotation-job.yaml              # cron-like Job that swaps the Secret
...
```

Each manifest set is **kustomize-buildable** (AD49).

---

## 15. Helm chart (in-cluster runner)

Optional — for environments without external CI runners.  Same
pattern as PR #21 bench's optional in-cluster Helm chart.  3 templates :

- `Chart.yaml` + `values.yaml`
- `templates/job.yaml` — one-shot Job invoking `efe-smoke run`
- `templates/serviceaccount.yaml` + `templates/rbac.yaml` (namespace-scoped — AD53)

---

## 16. CI workflows

### 16.1 `smoke-pr.yml` — hard merge gate

```yaml
name: smoke-pr
on:
  pull_request:
  push:
    branches: [main]
concurrency:
  group: smoke-${{ github.head_ref || github.ref }}
  cancel-in-progress: true

jobs:
  smoke:
    runs-on: ubuntu-22.04
    timeout-minutes: 8     # 4-min hard cap + 4-min safety buffer
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: protoc
        run: sudo apt-get update && sudo apt-get install -y protobuf-compiler

      - uses: helm/kind-action@v1
        with:
          cluster_name: smoke-pr
          version: v0.24.0
          wait: 60s

      - name: Build
        run: cargo build --release -p efe-smoke

      - name: Install efe-meta-swarm chart
        run: |
          helm repo add efe https://efe.github.io/charts || true
          helm install efe-meta-swarm efe/efe-meta-swarm \
              --namespace efe --create-namespace \
              --set image.tag=${{ github.sha }} \
              --wait --timeout 90s

      - name: Run smoke (all 9 checks)
        id: smoke
        run: |
          set -o pipefail
          ./target/release/efe-smoke run \
              --chart-version ${{ github.sha }} \
              --provisioner external \
              --kubeconfig $HOME/.kube/config \
              --output /tmp/smoke.json \
              --pr-comment-file /tmp/pr-comment.md \
              --badge-file /tmp/smoke.svg

      - name: Gate
        run: |
          ./target/release/efe-smoke gate --input /tmp/smoke.json

      - name: Post PR comment
        if: github.event_name == 'pull_request' && always()
        uses: actions/github-script@v7
        with:
          script: |
            const fs = require('fs');
            if (fs.existsSync('/tmp/pr-comment.md')) {
              await github.rest.issues.createComment({
                issue_number: context.issue.number,
                owner: context.repo.owner,
                repo: context.repo.repo,
                body: fs.readFileSync('/tmp/pr-comment.md', 'utf8'),
              });
            }

      - name: Upload artefacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: smoke-${{ github.run_id }}
          path: |
            /tmp/smoke.json
            /tmp/smoke.svg
            /tmp/pr-comment.md
          retention-days: 14

      - name: Publish gh-pages badge
        if: github.event_name == 'push' && github.ref == 'refs/heads/main'
        uses: peaceiris/actions-gh-pages@v4
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: /tmp/smoke
          destination_dir: smoke
          keep_files: true
```

### 16.2 `smoke-nightly.yml` — extended scope

- Triggered on cron `0 2 * * *` UTC.
- Same 9 checks, **but multi-node kind cluster** (5 controllers + 15 participants).
- Catches regressions that only manifest at scale.
- Output published to a separate gh-pages path.

---

## 17. STABILITY_CONTRACT v1.5 § 23

```markdown
## 23. Smoke surface — frozen v1.5 (PR #23)

### 23.1 Smoke check IDs (closed taxonomy, 9)
  S1_boot | S2_l1_swarm | S3_l2_meta_swarm | S4_mtls_rotation |
  S5_reload_sighup | S6_metrics | S7_otel | S8_restart | S9_shutdown

### 23.2 Per-check budgets (seconds, frozen)
  S1=30 | S2=15 | S3=20 | S4=25 | S5=20 | S6=10 | S7=15 | S8=30 | S9=15
  TOTAL ≤ 180 s (excluding ~60 s cluster boot)

### 23.3 14 STABLE metric families (consumed by S6)
  (from PR #17 STABILITY_CONTRACT v1.0 § 4 — see § 6 above)

### 23.4 CLI subcommand surface (frozen)
  run | list | validate | gate | version

### 23.5 Smoke result JSON schema v1
  see `smoke/schemas/smoke-result-v1.json`

### 23.6 ClusterProvisioner trait surface (frozen)
  see `crates/efe-cluster-provisioner/src/traits.rs`

### 23.7 ExitCode taxonomy (6, frozen)
  0=Success | 1=SmokeFail | 2=Cluster | 3=Config | 4=Interrupted | 5=Internal
```

---

## 18. Security model + Kubernetes RBAC

| Knob | Default | Notes |
|---|---|---|
| `runAsNonRoot` | `true` | AD52 |
| `readOnlyRootFilesystem` | `true` | AD52 |
| `allowPrivilegeEscalation` | `false` | AD52 |
| `capabilities.drop` | `[ALL]` | AD52 |
| `rbac.namespaceScoped` | `true` | AD10 / AD53 / T108 |
| `kubeconfig` mount | `readOnly: true` | AD54 |
| `mode=production` | **rejected** | AD9 / T108 |
| log redaction | runtime flag `--redact-logs` | T107 / T112 |

---

## 19. Threat model

| ID | Threat | Severity | Mitigation |
|---|---|---|---|
| T106 | Smoke flake-storm → bypass via re-run | SEV-2 | AD4 (1% flake budget) + AD17 (1 retry) + runbook Appendix F |
| T107 | Credential leak in smoke logs | SEV-2 | `--redact-logs` flag + regex-based scrubber |
| T108 | Smoke mis-targeted at production cluster | SEV-1 | AD9 — refuse `mode=production` ; check `efe.io/environment` label |
| T109 | Baseline drift undetected (smoke passes but bench fails) | SEV-3 | PR #22 SLO-rollup catches drift over rolling 30-day window |
| T110 | Gate bypass via `--allow-skip` abuse | SEV-2 | AD32-33 — requires `--reason` ; logged in GitHub audit |
| T111 | Unbounded cluster DoS on CI | SEV-3 | Hard 4-min cap (AD3) + `concurrency.cancel-in-progress` |
| T112 | Test secrets committed in git | SEV-2 | AD55 — secrets generated in-cluster, destroyed on cleanup |
| T113 | Smoke uses prod network egress | SEV-2 | AD12 — egress whitelisted to in-cluster + kind node only |
| T114 | Smoke clobbers shared PVC | SEV-2 | AD11 — PVCs scoped to smoke namespace ; labelled for sweep |
| T115 | Smoke as timing oracle | SEV-3 | AD56 — jittered request times in S4 |

---

## 20. Acceptance criteria (AC1 – AC34)

### 20.1 Correctness

- **AC1** — Each of S1-S9 implements the formal pass criteria from § 7.
- **AC2** — Each check completes within its budget (per-check timeout enforced).
- **AC3** — `overall_verdict=pass` iff every check verdict is `pass`.
- **AC4** — `--allow-skip` requires `--reason` (rejected otherwise).
- **AC5** — Smoke result JSON validates against `smoke-result-v1.json`.

### 20.2 Cluster provisioner extraction

- **AC6** — `cargo test -p efe-bench-harness` passes byte-identically before and after extraction (compat shim works).
- **AC7** — `cargo test -p efe-cluster-provisioner` is independent and green.
- **AC8** — `ClusterProvisioner` trait surface matches STABILITY_CONTRACT v1.5 § 23.6.

### 20.3 CI integration

- **AC9** — `smoke-pr.yml` runs on PR and on push:main.
- **AC10** — Hard 4-min timeout enforced via `timeout-minutes`.
- **AC11** — Failed runs upload pod logs as artefacts (14-day retention).
- **AC12** — Successful push:main updates the gh-pages badge.
- **AC13** — PR comment is HTML-escaped (T89 mitigation).

### 20.4 Per-check pass

- **AC14 — S1** — controller `/healthz` returns 200 within 20 s + 3 participants `Ready`.
- **AC15 — S2** — 5 L1 RPCs all OK, p99 < 500 ms.
- **AC16 — S3** — L2 chain works, p99 < 1 s.
- **AC17 — S4** — 0 dropped requests during cert rotation.
- **AC18 — S5** — manifest version increments + 0 dropped during reload.
- **AC19 — S6** — 14 STABLE metric families exposed.
- **AC20 — S7** — OTel trace has ≥ 2 spans, both expected service names.
- **AC21 — S8** — restart within 30 s, no quorum loss.
- **AC22 — S9** — controller exits with code 0 within 10 s.

### 20.5 Security

- **AC23** — `mode=production` rejected at orchestrator startup (T108).
- **AC24** — `--redact-logs` substitutes secret-pattern strings with `[REDACTED]` (T107).
- **AC25** — Namespace-scoped Role, NOT ClusterRole (T103/T108).
- **AC26** — No timing-oracle leak in S4 (T115 — jitter test).

### 20.6 Stability

- **AC27** — STABILITY_CONTRACT v1.5 § 23 frozen-test suite passes (locks check IDs, budgets, schema, CLI).
- **AC28** — Smoke result JSON schema v1 is forward-compatible (unknown fields ignored).
- **AC29** — `efe-smoke --version` reports stability tier "v1.5".

### 20.7 Operability

- **AC30** — `efe-smoke run --check S2 --keep-cluster` keeps cluster alive for debug.
- **AC31** — Logs include full pod stdout/stderr for failed checks (AD18).
- **AC32** — `efe-smoke list --format json` is parseable + lists 9 checks.
- **AC33** — `efe-smoke gate --input <json>` exits 0 if `overall_verdict=pass`, 1 otherwise.
- **AC34** — Total smoke wall-clock (including cluster boot) ≤ 240 s on a standard `ubuntu-22.04` runner.

---

## 21. Tests strategy

| Layer | Path | Count | Purpose |
|---|---|---|---|
| Unit kernel | `src/checks/{s1..9}.rs::tests` | 9 × 4 = 36 | per-check pass/fail/timeout/edge-case |
| Orchestrator | `src/orchestrator.rs::tests` | 8 | budget enforcement + sequencing |
| Cluster provisioner | `crates/efe-cluster-provisioner/tests/` | 12 | trait + 3 impls |
| Compat shim | `bench/harness/tests/cluster_compat.rs` | 4 | AC6 — PR #21 still works |
| Result + JSON | `src/result/json.rs::tests` | 7 | schema roundtrip + golden |
| Badge | `src/result/badge.rs::tests` | 5 | byte-stability + T100 no-JS |
| CLI golden | `tests/cli_golden.rs` | 9 | 5 subcommands + flag taxonomy |
| Frozen v1.5 | `tests/stability_contract_v15.rs` | 8 | locks the contract |
| Smoke-of-smoke | `tests/smoke_of_smoke.rs` | 1 | meta-test : run smoke against a mock cluster, assert overall pass |
| Property | `tests/check_budget_property.rs` | 4 | proptest : budget never overrun |
| **Total** | | **~94** | |

---

## 22. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Kind cluster boot > 60 s | LOW | HIGH | Increase global timeout to 5 min via Helm value ; investigate runner CPU |
| Network flake during S2-S5 | MED | MED | AD17 — 1 retry on connection-refused in first 5 s |
| S4 mTLS rotation race | LOW | HIGH | Sequential gating : rotation Job has init-container ensuring 50 QPS observed first |
| Test secret leak | LOW | HIGH | AD55 + `--redact-logs` + secret-pattern scrubber |
| Cluster-provisioner extraction breaks PR #21 | MED | HIGH | AC6 — byte-identical compat test ; revert plan stays small |
| Gate bypass abuse | MED | MED | AD32-33 + GitHub audit log |
| Concurrency race on push:main | LOW | MED | `concurrency.cancel-in-progress` (CI) |
| 4-min hard cap too tight | LOW | HIGH | Per-check timeouts catch issues early ; cap reviewed quarterly |

---

## 23. Rollback strategy

### Code rollback

- `git revert` the merge commit.
- CI workflows remain harmless (cron jobs just emit no new badges).
- Cluster-provisioner crate revert : restore the inline modules in PR
  #21 bench/harness — compat shim makes this trivial.

### Data rollback

- Smoke artefacts are 14-day retention ; no rollback needed.
- gh-pages badge : if corrupt, `gh-pages` branch can be force-rewritten by release-eng (audit-logged).

### Gate rollback

- Emergency `--allow-skip` flow documented in runbook ; release-eng on-call holds the override.

---

## 24. Sequence diagrams

### 24.1 PR-trigger flow

```
github PR ──► smoke-pr.yml triggers
              │
              ├── kind cluster provision (60 s)
              ├── helm install efe-meta-swarm (30 s)
              ├── efe-smoke run (180 s)
              │    ├── S1 boot       (28 s) ✅
              │    ├── S2 L1         (12 s) ✅
              │    ├── S3 L2         (15 s) ✅
              │    ├── S4 mTLS       (22 s) ✅
              │    ├── S5 reload     (16 s) ✅
              │    ├── S6 metrics    ( 8 s) ✅
              │    ├── S7 OTel       (14 s) ✅
              │    ├── S8 restart    (29 s) ✅
              │    └── S9 shutdown   (13 s) ✅
              ├── efe-smoke gate     (exit 0)
              └── post PR comment    + upload artefacts
```

### 24.2 Failure investigation flow

```
S4 fails ─► artefact uploaded
            │
            ├── /tmp/smoke.json → "S4.verdict=fail, dropped_requests=2"
            ├── pod logs       → cert rotation completed at t=12.3 s
            ├── loadgen ndjson → 2 requests at t=12.31 s, t=12.32 s
            ├── kubectl events → Secret updated at t=12.3 s
            └── analysis      → S4 caught a real regression
```

---

## 25. Reviewer checklist

| Check | Status |
|---|---|
| 9 check IDs frozen in `SmokeCheckId` enum | ☐ |
| 9 budgets sum ≤ 180 s | ☐ |
| Cluster-provisioner extraction : PR #21 tests pass byte-identically | ☐ |
| `efe-smoke run` ≤ 240 s on a standard runner | ☐ |
| `efe-smoke gate` exit 0/1 logic correct | ☐ |
| `--allow-skip` requires `--reason` | ☐ |
| `mode=production` rejected | ☐ |
| `--redact-logs` substitutes secrets | ☐ |
| Namespace-scoped RBAC | ☐ |
| 14 STABLE metric families validated by S6 | ☐ |
| OTel span chain verified by S7 | ☐ |
| Smoke result JSON schema v1 frozen | ☐ |
| CLI golden test passes for 5 subcommands | ☐ |
| Frozen v1.5 test suite passes | ☐ |
| Clippy `-D warnings` clean | ☐ |
| 94 tests green ; smoke-of-smoke meta-test green | ☐ |

---

## 26. Appendix A — CLI flag reference

(alphabetical, with default + envvar + frozen-or-not)

```
--allow-skip                   false    (frozen — requires --reason)
--badge-file <PATH>            (no def) (frozen)
--chart-version <VERSION>      REQUIRED env: EFE_CHART_VERSION  (frozen)
--check <ID>                   (rep)    default = all 9       (frozen)
--global-timeout <DURATION>    240s     humantime              (frozen)
--keep-cluster                 false                          (frozen)
--keep-cluster-on-failure      false                          (frozen)
--kubeconfig <PATH>            env: KUBECONFIG                (frozen)
--log-format <text|json>       text in TTY, json otherwise    (frozen)
--log-level <LEVEL>            info     env: RUST_LOG          (frozen)
--output <PATH>                (no def) (frozen)
--per-check-timeout <DURATION> per-check (frozen — see § 6)   (frozen)
--pr-comment-file <PATH>       (no def) (frozen)
--provisioner <KIND>           kind     (frozen)
--reason <TEXT>                (no def) (required with --allow-skip)
--redact-logs                  false                          (frozen)
```

---

## 27. Appendix B — per-check pass criteria (formal)

(re-iterates § 7 in mathematical notation ; used as the reviewer's truth table)

```
S1.pass ⇔ (t_total ≤ 30) ∧ (controller.healthz = 200) ∧ (#participants.Ready = 3) ∧ (∀p ∈ participants : p.healthz = 200)
S2.pass ⇔ (∀ rpc ∈ {Create, Join, List, Leave, Delete} : rpc.status = OK) ∧ (error_rate = 0) ∧ (p99(latency) < 500ms)
S3.pass ⇔ similar to S2 with L2 RPC set, p99 < 1s
S4.pass ⇔ total_failed = 0 ∧ (p99_during - p99_before) < 50ms ∧ cert_cn changes mid-stream
S5.pass ⇔ total_failed = 0 ∧ manifest_v_post > manifest_v_pre
S6.pass ⇔ |observed_families ∩ STABLE_FAMILIES| = 14
S7.pass ⇔ #spans ≥ 2 ∧ "controller" ∈ services ∧ "participant" ∈ services
S8.pass ⇔ new_pod.Ready ≤ 30s after delete ∧ participants_total returns to 3 ∧ error_rate < 5%
S9.pass ⇔ old_pod.exit_code = 0 within 10s ∧ all 20 RPCs gracefully completed
overall.pass ⇔ ⋀ Si.pass
```

---

## 28. Appendix C — smoke result JSON schema (annotated)

(see `smoke/schemas/smoke-result-v1.json` — same shape as § 9)

---

## 29. Appendix D — Manifest library inventory

```
smoke/manifests/s1-boot/                # base + readiness overlay
smoke/manifests/s2-l1-swarm/            # no overlay — vanilla chart
smoke/manifests/s3-l2-meta-swarm/       # no overlay — vanilla chart
smoke/manifests/s4-mtls-rotation/       # CA-A + CA-B secrets + rotation Job
smoke/manifests/s5-reload-sighup/       # reload-trigger CronJob
smoke/manifests/s6-metrics/             # no overlay
smoke/manifests/s7-otel/                # OTel collector deployment
smoke/manifests/s8-restart/             # PDB + restart trigger Job
smoke/manifests/s9-shutdown/            # rollout-restart Job
```

Each set is **kustomize-buildable** (AD49) ; total ~1 040 LOC.

---

## 30. Appendix E — Smoke badge SVG template

```
┌──────────────────────┐
│ smoke │ pass         │
│       │              │
│ (svg, byte-stable)   │
└──────────────────────┘
```

Pass = #3fb950 (same green as PR #22).
Fail = #da3633 (same red).
No-data = #8b949e.

---

## 31. Appendix F — Flake-storm mitigation playbook (T106)

If smoke flake rate > 1 % over 100 PRs :

1. SRE on-call opens an incident.
2. Identify the flakiest check via `efe-smoke list --format jsonl | jq …`.
3. Quarantine : add `--check <flaky>` to the workflow with `continue-on-error: true`.
4. Open a follow-up PR to fix or **remove** the check (no exception, AD4).
5. Smoke ratchet : add a `flake_rate` tracker series to PR #22 SLO rollup as `source=synthetic_smoke`.

---

## 32. Appendix G — Phase-3 follow-ups (G.1 – G.10)

| # | Title | Brief |
|---|---|---|
| G.1 | Cross-region smoke | Same 9 checks across us-east-1 / eu-west-1 |
| G.2 | Phase-1 binary compat smoke | Detect regressions vs the Phase-1 binary |
| G.3 | Smoke-driven canary rollout | Canary only proceeds if smoke green |
| G.4 | Smoke signed evidence | Cosign-signed JSON for each smoke run (mirrors PR #22 quarterly) |
| G.5 | GPU smoke | CUDA path validation for ML inference subsurface (out of A1) |
| G.6 | Smoke with chaos faults | Bridge to PR #21 chaos library (10 chaos-aware smoke variants) |
| G.7 | Multi-arch smoke | arm64 + amd64 |
| G.8 | Multi-participant scale smoke | > 3 participants ; integration with bench |
| G.9 | IDE pre-commit smoke | VSCode / IntelliJ task that runs S1+S6 only (< 30 s) |
| G.10 | `helm test` integration | Expose smoke as a Helm chart test for downstream consumers |

---

## End of architecture

This document is **frozen** — any change to check IDs, budgets, result
schema, or CLI subcommand surface requires :

1. A new architecture PR (e.g. PR #27),
2. A bump of STABILITY_CONTRACT major version,
3. A migration plan covering existing smoke results.

The implementation (commits 1–14 of § 3) follows this spec exactly.
Any divergence is a contract violation and must be filed as an issue
referencing this doc.
