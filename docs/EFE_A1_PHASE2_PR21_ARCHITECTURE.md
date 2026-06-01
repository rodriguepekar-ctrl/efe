# EFE-A1 Phase 2 — PR #21 : `public-bench harness` — reproducible cross-version benchmark + chaos suite

| Field | Value |
|---|---|
| **PR** | #21 (Sprint A1 Phase 2 / P2.6 W12 → P2.7 W13) |
| **Branch** | `efe-a1/p2-pr21-public-bench` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team — performance + SRE area |
| **Reviewers** | 2× perf-eng + 2× SRE + 1× compliance + 1× security + 1× release-eng |
| **Depends on** | PR #14 (Helm production-grade), PR #16 (mTLS), PR #17 (Prometheus metrics), PR #18 (OTel propagation), PR #19 (dashboards + alerts + 4 SLOs), PR #20 (`efe-meta-swarm-loadgen` traffic driver) |
| **Blocks** | PR #22 (Phase-3 SLO formalisation — needs the bench harness to validate target stability across releases), GA v1.0 (no public release without an auditable bench manifest) |
| **L2 gate** | G18 (production-grade public bench surface : 10 scenario YAMLs + 6 chaos faults + Rust harness binary + baseline-versioned regression detection + GitHub-Pages-published methodology + STABILITY_CONTRACT v1.3 commitment) |
| **Threat IDs in scope** | T86 – T95 (10 new : chaos leak to prod, baseline tampering, GitHub Pages defacement, PR-comment injection via scenario, kubeconfig leak in bench logs, scenario YAML SSRF, parquet result DoS, chaos-mesh CRD privilege escalation, bench-as-timing-oracle, bench priority inversion on shared cluster) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS3 + § 14 (Public benchmarks) |
| **CVSS bandwidth** | All ≤ SEV-2 (no PII leak ; bench data is intentionally synthetic) |
| **Crate version** | New crate `efe-bench-harness` v0.1.0 at `bench/harness/`, binary `efe-bench-harness` |
| **Existing infrastructure consumed** | `crates/efe-bench` (D18 Criterion micro-benches — kept, complemented) ; `scripts/benches.list` (D5 schema — extended with new fields) ; `bench_baselines/v0.x.y` (continued pattern — first entry `v0.6.0`) ; `deploy/chaos/` (chaos-mesh CRD library — extended) |
| **STABILITY_CONTRACT v** | 1.2 → **1.3** (new tier : *Scenario ID* + *Baseline tolerance* + *Result schema version*) |
| **Total LOC budget** | Rust harness ~4 500 ; 10 scenarios YAML ~1 200 ; 6 chaos fault YAMLs ~400 ; baseline JSON snapshots ~600 ; comparison engine ~800 ; CI workflows ~400 ; in-cluster Helm chart ~520 ; tests ~1 480 ; doc ~3 080 ; runbook + README ~820 → **~13 800 LOC** |
| **CI cost delta** | + 18 s for `cargo test -p efe-bench-harness` ; + 4 m for `bench-quick` job (3 scenarios on kind cluster) ; nightly `bench-full` job (all 10 scenarios, separate runner) ; total committed CI delta + 22 s (nightly runs on separate cron, not blocking PRs) |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals + scope
   2. Pre-flight inventory — existing bench infrastructure (D18 + chaos + baselines)
   3. Branch + commit plan (12 commits across 2 sprints)
   4. Architectural decisions (AD1 – AD45)
   5. Operator personas + 8 use cases
   6. Scenario catalogue — 10 scenarios overview
   7. Per-scenario design (S01 – S10)
   8. Chaos fault library (6 chaos-mesh CRDs)
   9. Bench orchestrator (`efe-bench-harness` Rust binary)
  10. Scenario YAML schema v1
  11. Result format (parquet + JSON summary + Prom snapshot)
  12. Comparison + verdict engine (per-metric tolerance contract)
  13. Baseline management (git-tracked, version-tagged)
  14. CI integration (4 workflows)
  15. GitHub Pages publishing
  16. Cluster provisioning strategies (kind / k3s / kubeconfig passthrough)
  17. chaos-mesh integration
  18. PR #20 loadgen integration (consumer pattern)
  19. Multi-region benches (Phase-3 prep)
  20. Performance budget (per-scenario wall-clock cap)
  21. STABILITY_CONTRACT v1.3 additions
  22. Security model + Kubernetes RBAC
  23. Threat-model coverage (T86 – T95)
  24. Acceptance criteria (AC1 – AC35)
  25. Tests strategy (unit + integration + golden + kind-cluster e2e)
  26. Risks + mitigations
  27. Rollback strategy
  28. Sequence diagrams (scenario flow, chaos injection, baseline diff)
  29. Reviewer checklist
  30. Appendix A — CLI flag reference
  31. Appendix B — scenario YAML schema (annotated)
  32. Appendix C — chaos fault YAML library
  33. Appendix D — comparison rules per metric
  34. Appendix E — baseline JSON schema
  35. Appendix F — multi-region topology
  36. Appendix G — Phase-3 follow-ups

---

## 0. TL;DR

PR #11 → #20 produced the meta-swarm controller + participants stack, its
operator surface (PR #19 dashboards + alerts + 4 SLOs), and a deterministic
L2 load generator (PR #20 `efe-meta-swarm-loadgen`).  What's missing is the
**publicly-auditable benchmark methodology** that proves :

  - performance numbers are reproducible across releases,
  - SLO targets are met under realistic + adversarial conditions,
  - regression-free behaviour across upgrades,
  - chaos fault tolerance (participant loss, network partition, cert
    expiry, manifest reload storm, CPU throttle, memory pressure),
  - long-running stability (2-hour soak).

PR #21 ships :

  - **10 scenario YAMLs** under `bench/scenarios/` declaring the full
    test matrix : 4 performance, 4 chaos, 2 soak.
  - **6 chaos fault YAMLs** under `bench/chaos/` (chaos-mesh CRDs +
    kubectl-applicable manifests).
  - A new Rust binary **`efe-bench-harness`** at `bench/harness/` that
    orchestrates scenarios end-to-end : provision the cluster (or
    consume an operator-supplied kubeconfig), install the meta-swarm
    chart + (optionally) chaos-mesh, drive traffic via PR #20 loadgen,
    inject chaos faults at scenario-scheduled times, harvest results,
    diff against baseline, emit a verdict.
  - **Versioned baselines** at `bench_baselines/v0.6.0/` (continuing
    the existing `bench_baselines/v0.x.y` pattern).
  - **Comparison + verdict engine** with closed per-metric tolerance
    table (`p95 ≤ 10%`, `p99 ≤ 15%`, `error_rate ≤ +0.5pp`, etc.)
    committed in STABILITY_CONTRACT v1.3.
  - **GitHub Pages site** at `https://efe.github.io/bench/` hosting the
    rolling badge + historical trend graphs + scenario methodology
    (compliance-grade evidence).
  - **4 CI workflows** : `bench-quick` (3 scenarios, every PR labelled
    `bench`), `bench-full` (10 scenarios, nightly), `bench-release`
    (10 scenarios + auto-commit baseline on release tag),
    `bench-compare` (PR-comment regression diff).
  - **STABILITY_CONTRACT v1.3** : *Scenario ID* + *Baseline tolerance*
    + *Result schema version* tiers, each frozen for ≥ 6 chart-minor
    versions.

This is the EFE equivalent of TPC for databases, MLPerf for ML, SPEC for
CPU : a public, reproducible, auditable benchmark contract.

---

## 1. Goal + non-goals + scope

### 1.1 Goal

Ship a **publicly-auditable, reproducible, version-comparable benchmark
suite** for the meta-swarm gRPC L2 surface that :

  - prevents stealth performance regressions across releases by failing
    PRs whose bench numbers exceed the baseline tolerance,
  - exercises chaos fault tolerance under controllable, repeatable
    conditions (chaos-mesh CRDs),
  - publishes baseline numbers + methodology openly (GitHub Pages),
  - documents the test environment (cluster topology, image SHAs, seed)
    so any third party can reproduce within a defined tolerance band,
  - integrates with the existing `bench_baselines/v0.x.y` convention
    + the `scripts/benches.list` D5 schema (no migration churn),
  - serves as compliance evidence (SOC-2 SLO-attainment audit).

### 1.2 Non-goals (out of scope)

  - **NOT a replacement for `crates/efe-bench`** (D18 Criterion micro-
    benches stay — they cover sub-millisecond unit-level paths PR #21
    can't reach via gRPC).
  - **NOT a fuzzer / pentest harness** — those live separately at
    `deploy/chaos/m26_pentest/`, `m27_edge_pentest/` etc.
  - **NOT a chaos engineering platform** — chaos-mesh is the substrate ;
    PR #21 *consumes* it via CRDs.
  - **NOT a continuous capacity-planning tool** — that's a Phase-3
    follow-up (anomaly forecasting needs ML).
  - **NOT a multi-tenant bench** — single-tenant scenarios only ; per-
    tenant SLO benches land in PR #25 (Phase-3 multi-tenancy).
  - **NOT an in-production traffic shaper** — bench targets dev /
    staging clusters ; the `--mode=production` of loadgen is **forbidden**
    by harness (T86 mitigation).
  - **NOT a chart upgrade orchestrator** — `helm upgrade` is the
    operator's responsibility ; the harness only *consumes* a chart
    version + image tag.

### 1.3 Scope summary

| Deliverable | In | Out |
|---|---|---|
| Scenarios | 10 (S01–S10) | Phase-3 multi-region (Appendix G.1) |
| Chaos faults | 6 chaos-mesh CRDs | Custom OS-level fault injection (BPF) |
| Harness binary | Rust (`efe-bench-harness`) | Python wrapper, Go alt |
| Cluster provisioning | kind / k3s / `--kubeconfig` passthrough | EKS auto-provision, GKE auto-provision |
| Baseline storage | Git-tracked JSON under `bench_baselines/v0.x.y/` | S3 / blob storage |
| Comparison rules | 5 metrics × per-scenario tolerance | ML-based anomaly detection |
| CI workflows | 4 (quick / full / release / compare) | Cross-cloud matrix |
| GitHub Pages | Static markdown + SVG badges | Live dashboards (Grafana belongs to PR #19) |
| Helm chart | optional in-cluster orchestrator pod | StatefulSet variant |
| Acceptance criteria | 35 (AC1–AC35) | — |
| STABILITY_CONTRACT | 3 new tiers (Scenario ID, Tolerance, Schema) | — |
| Threat model | T86–T95 | — |

---

## 2. Pre-flight inventory

### 2.1 Existing bench infrastructure in the repo

```
crates/efe-bench/              # Sprint D18 — Criterion micro-benches (KEPT)
├── Cargo.toml                 # workspace member, edition 2021
├── build.rs                   # generates bench list from scripts/benches.list
├── scenarios/                 # SDK-level scenarios (catalog ops, etc.)
└── src/                       # MockBenchClient + TonicBenchClient

scripts/
├── benches.list               # D5 source of truth (7-field schema) — EXTENDED
├── bench_runner.sh            # D5 Cargo bench runner — UNCHANGED
└── bench/                     # bench helper scripts

bench_baselines/
├── README.md                  # bench history methodology
├── v0.1.0/ → v0.1.13/         # historical Criterion baselines
└── v0.6.0/                    # NEW (PR #21) — first L2 meta-swarm bench

deploy/chaos/
├── load/                      # k6 load tests
├── soak/                      # multi-day soak tests
├── m25/, m26/, m27_*/         # per-sprint chaos suites
└── m30_multimodal_load/       # multimodal load tests

docs/bench/
├── bench_foundation_architecture.md   # D5 foundation
├── d11/d12/d13/.../d18*.md            # per-sprint bench architectures
└── (NEW : pr21_public_bench_architecture.md  ← redirects to this doc)
```

### 2.2 What `scripts/benches.list` looks like today

D5 7-field schema, one bench per line :

```
# name : crate : sprint : blas-aware : tier : slo-prefix : paths-watch
rpc_naive : efe-bench : D18 : false : T1 : SLO-D18 : crates/efe-bench/
multi_replica : efe-bench : D18 : true : T2 : SLO-D18 : crates/efe-bench/
... (13 existing entries)
```

PR #21 **extends** this schema with a backward-compatible 8th optional field :

```
# name : crate : sprint : blas-aware : tier : slo-prefix : paths-watch [: scenario-yaml]
S01_baseline_100qps_5min : efe-bench-harness : A1 : false : T2 : SLO-A1 : bench/scenarios/ : S01.yaml
```

The trailing `: scenario-yaml` is optional ; existing 7-field rows keep working.

### 2.3 What `bench_baselines/v0.x.y/` looks like today

Each version directory holds JSON snapshots :

```
bench_baselines/v0.1.0/
├── rpc_naive.json           # { "p50_ms": 1.2, "p95_ms": 4.5, "p99_ms": 12.3 }
├── multi_replica.json
└── ... (historical Criterion benches)
```

PR #21 adds an L2 directory layer for the new scenario style :

```
bench_baselines/v0.6.0/
├── L2/                      # NEW — L2 meta-swarm scenarios from PR #21
│   ├── S01_baseline_100qps_5min.json
│   ├── S02_canary_1000qps_15min.json
│   └── ... (S03..S10)
└── (existing Criterion baselines unchanged)
```

### 2.4 What `deploy/chaos/` looks like today

Mostly k6-driven and per-sprint pentest scenarios.  PR #21 adds an L2-
specific subdir :

```
deploy/chaos/
├── ... (existing)
└── pr21_meta_swarm/         # NEW — chaos-mesh CRDs for L2 benches
    ├── participant-loss.yaml
    ├── network-partition.yaml
    ├── cert-expiry.yaml
    ├── reload-storm.yaml
    ├── cpu-throttle.yaml
    └── memory-pressure.yaml
```

### 2.5 Gaps to close

  - **No L2 gRPC bench scenarios** : `crates/efe-bench` covers SDK-level
    catalog ops + Criterion micro-benches only.
  - **No chaos-mesh-integrated harness** : `deploy/chaos/` has CRDs but
    no orchestrator that ties chaos to load + result harvest.
  - **No version-comparable L2 baselines** : `bench_baselines/v0.1.x/`
    is Criterion-only.
  - **No public-facing methodology page** : `docs/bench/` is internal
    architecture docs, no public landing.
  - **No PR-comment regression diff** : PR #20 ships SLO regression for
    a single profile, but not a multi-scenario compare-vs-baseline.

---

## 3. Branch + commit plan (12 commits across 2 sprints)

| # | Commit | LOC | Sprint | Reviewer focus |
|---|---|---|---|---|
| C1 | `chore: bench/harness skeleton + workspace member + scripts/benches.list extension` | ~280 | P2.6 W12 | benches.list 8th field backward compat |
| C2 | `feat(harness): Rust binary core — scenario loader + cluster discovery` | ~720 | P2.6 W12 | Crate boundary with loadgen + bench |
| C3 | `feat(harness): 10 scenario YAMLs (S01-S10) + JSON-Schema` | ~1 320 | P2.6 W12 | Scenario coverage + reproducibility |
| C4 | `feat(harness): 6 chaos-mesh CRD YAMLs + apply/cleanup engine` | ~640 | P2.6 W12 | CRD safety, namespace scoping |
| C5 | `feat(harness): PR #20 loadgen integration — spawn + harvest result file` | ~440 | P2.6 W13 | mpsc bridge, life-cycle |
| C6 | `feat(harness): result collection — parquet + Prometheus snapshot + chaos events` | ~620 | P2.6 W13 | Snapshot timing, race conditions |
| C7 | `feat(harness): comparison engine + verdict + per-metric tolerance` | ~580 | P2.6 W13 | Stat noise tolerance, false-positive rate |
| C8 | `feat(harness): baseline storage + git-LFS opt-in + bench_baselines/v0.6.0/L2/ skeleton` | ~280 | P2.7 W13 | LFS gating, file-size budget |
| C9 | `feat(ci): 4 workflows (bench-quick / bench-full / bench-release / bench-compare)` | ~440 | P2.7 W13 | Cron cadence, kind-cluster timing |
| C10 | `feat(harness): GitHub Pages publish + SVG badge generator + methodology MD` | ~520 | P2.7 W13 | Static site routing, badge cache |
| C11 | `feat(helm): in-cluster orchestrator chart (optional — for ops self-service)` | ~520 | P2.7 W13 | Job vs CronJob, RBAC reuse |
| C12 | `docs: architecture (this) + runbook + STABILITY_CONTRACT v1.3 + bench/README.md` | ~3 080 | P2.7 W14 | Doc parity with PR #20, AC32 |

**Total** : ~9 440 LOC commit body + ~4 360 LOC tests + docs = **~13 800 LOC**.

---

## 4. Architectural decisions (AD1 – AD45)

### AD1 — Crate at `bench/harness/`, NOT `tools/harness/`

`tools/loadgen/` is an operator-day utility ; `bench/harness/` is the
benchmark orchestrator (different audience, different release cadence).
The `bench/` top-level already exists for `bench_baselines/` ; we cohabit.

### AD2 — Rust binary, not Python / bash

Reuses `efe-meta-swarm-loadgen` as a library dependency (Rust-to-Rust)
without subprocess overhead.  Avoids Python/Rust version coupling.
Keeps the public-facing harness pinnable via Cargo.

### AD3 — Binary name `efe-bench-harness`

Crisp, not buried under `efe-meta-swarm-` prefix.  This harness is
multi-product (Phase-3 will extend to L1 / billing / etc.).  The
existing `efe-bench` crate stays Criterion-only.

### AD4 — 10 scenarios is the MVP catalogue

4 performance, 4 chaos, 2 soak.  Architecture § 7 details each.
New scenarios land in PR #21+ ; the catalogue is committed in
STABILITY_CONTRACT v1.3 (renaming is a contract break).

### AD5 — Scenarios are declarative YAML files, not Rust code

Reproducibility + auditability ; ops can read + diff scenarios
without Rust knowledge.  Schema versioned (`version: 1`).

### AD6 — Scenario YAML schema mirrors `loadgen profile-v1.json`

Bench scenarios *embed* a loadgen profile section (`loadgen:`).  The
harness invokes loadgen with the embedded profile.  No duplication.

### AD7 — Chaos faults are chaos-mesh CRDs

Standard substrate ; no custom fault injection code.  6 CRDs cover
the PR #21 scenarios ; new faults land as additional CRDs.

### AD8 — Per-metric tolerance is per-scenario

Different scenarios tolerate different noise levels.  Stress-test
scenarios (S05 chaos network partition) have wider p99 tolerance
(20 %) than smoke scenarios (S01 baseline, p99 tolerance 10 %).

### AD9 — Tolerance committed in STABILITY_CONTRACT v1.3

Relaxing tolerance is a contract break (same shape as PR #19 SLO).

### AD10 — Baselines stored as JSON under `bench_baselines/v{chart_version}/L2/`

Versioned by chart minor.  When a chart releases at 0.7.0, the
release workflow auto-commits new baselines.  Historical comparison
spans all versions.

### AD11 — Bench results never push to production

Hardcoded refusal of `--mode=production` to loadgen.  Even if an
operator misconfigures, the harness ABORTS with exit code 3 (config
error per AD17 from PR #20).

### AD12 — Cluster provisioning is kind / k3s / kubeconfig-passthrough

kind for CI (fastest, ephemeral) ; k3s for dev (single-binary,
realistic) ; `--kubeconfig` for operator-supplied cluster (staging,
shared dev).  No auto-provision of EKS / GKE / AKS in PR #21.

### AD13 — Bench wall-clock cap : 30 minutes per scenario (except soak)

Performance scenarios (S01-S04) ≤ 15 min ; chaos (S05-S08) ≤ 30 min ;
soak (S09-S10) ≤ 2 h.  Hard kill via tokio timeout + JoinSet abort.

### AD14 — Quick CI runs 3 scenarios (S01, S04, S10) — gates merge

Quick subset : smoke + 1 chaos + SLO validation.  Total wall-clock
~ 8 min.  Required for `bench` PR label.

### AD15 — Full suite is nightly + on release tag

Full suite ~ 3 h.  Runs on a dedicated GitHub-hosted runner ; results
committed back to main via PR-from-bot.

### AD16 — Baseline regen on release tag, not PR

Avoids baseline rewrite during normal PRs (a regression would silently
shift the baseline).  Release tags are the only commit type that
mutate `bench_baselines/`.

### AD17 — PR comment shows compare-vs-baseline table

5 columns : metric, baseline, this-run, delta, verdict (✅/⚠/❌).
Verdict ❌ fails the bench job ; ⚠ posts a warning but allows merge.

### AD18 — Comparison metrics are : p50_ms, p95_ms, p99_ms, error_rate, throughput_qps

Closed set.  Adding a new comparison metric is additive (non-breaking).
Removing or renaming is a chart-major bump.

### AD19 — Results are parquet + JSON summary + Prom snapshot

Parquet : per-request raw data (matches PR #20 output format).
JSON summary : aggregated per-metric values for compare engine.
Prom snapshot : server-side metrics at scenario end (snapshot via
`/api/v1/snapshot`) — for SLO compliance evidence.

### AD20 — Chaos faults apply via `kubectl apply -f` (no direct API)

Reduces coupling to chaos-mesh API versioning.  CRDs are versioned
files committed in `deploy/chaos/pr21_meta_swarm/`.

### AD21 — Faults are scoped to the bench namespace only

NetworkPolicy + RBAC : the harness's ServiceAccount can only inject
chaos in `efe-bench-*` namespaces.  Production namespaces are
out-of-scope (T93 mitigation).

### AD22 — Each scenario starts with a fresh namespace

`efe-bench-{scenario}-{run_id}`.  Namespace is created at scenario
start, deleted at scenario end (success or failure).  Guarantees
hermeticity.

### AD23 — Single-pod loadgen for scenarios S01-S08

Multi-pod (parallelism > 1) is exercised by S09 only.  Keeps
single-scenario complexity bounded ; multi-pod is its own scenario.

### AD24 — Soak scenarios run on a separate runner

S09 (24h soak — Phase-3, see Appendix G) doesn't run in shared CI ;
hosts on a dedicated `bench-soak` self-hosted runner.

### AD25 — GitHub Pages site lives at `docs/bench/site/`

Built via `mdbook` + custom plugin for badge SVG generation.  Hosted
on `gh-pages` branch via `peaceiris/actions-gh-pages` action.

### AD26 — SVG badges follow shields.io style

Format : `{label}-{value}-{color}` (e.g., `S01-12ms-brightgreen`).
Hosted under `docs/bench/site/badges/` ; cacheable for 1 hour.

### AD27 — Git LFS for parquet result files in `bench_baselines/`

Parquet artefacts can hit 10 MB+ per scenario.  Git LFS opt-in
configured via `.gitattributes` (lazy-clone-friendly).  Plain JSON
summaries stay in regular git.

### AD28 — STABILITY_CONTRACT v1.3 commits scenario IDs as stable surface

`S01_baseline_100qps_5min` is frozen.  Renaming is a chart-major
break.  Adding a new `S11_...` is additive.

### AD29 — Scenario YAML uses tonic / loadgen profile-v1 + extends

The `loadgen:` block of a scenario YAML is a verbatim PR #20 profile
YAML.  Reuses the profile-v1 JSON-Schema for validation.

### AD30 — Harness writes a `manifest.json` to baselines describing the run

Captures : harness version, loadgen version, chart version, image tag,
cluster topology (provisioner = kind/k3s/external), kubernetes version,
git commit, run timestamp, seed.  Auditable provenance.

### AD31 — kubeconfig passthrough never logs cert contents

T90 mitigation : kubectl invocations use the kubeconfig file via env
var ; we never `cat` it.  Cert paths logged ; cert bytes never.

### AD32 — Scenario YAMLs cannot fetch remote URLs

T91 mitigation : no `!Include` / `!fetch` directives.  All references
are local file paths.

### AD33 — Parquet result file capped at 100 MB

T92 mitigation.  When a scenario would exceed, the harness truncates
to head-and-tail samples + emits warning.

### AD34 — Chaos-mesh ServiceAccount can only run in `efe-bench-*` namespaces

T93 mitigation.  RoleBinding restricted to label selector.

### AD35 — Bench timing leak (T94) mitigated by deterministic seed

Seed propagated to loadgen ensures wire-bytes identical across runs ;
removes timing oracle for adversaries who could otherwise infer
internal scheduling.

### AD36 — Bench runs on dedicated nodes (taints) where possible

T95 mitigation.  When kubeconfig is operator-supplied, the harness
checks for a `bench-dedicated=true` taint and emits warning if absent.

### AD37 — Scenario `version: 1` schema frozen for ≥ 6 chart-minor versions

STABILITY_CONTRACT v1.3 § 11 commitment.  Schema bumps via
`version: 2` + migration helper.

### AD38 — Result JSON schema versioned (`schema_version: 1`)

Same shape as scenario YAML versioning.

### AD39 — Methodology document is the source of truth

`bench/README.md` is the public landing.  All scenarios link to a
methodology section explaining what the scenario tests + why the
tolerance is set there.

### AD40 — Test failure DOES NOT auto-update the baseline

A regression must be explicitly acknowledged via `make bench-regen`
+ a separate PR.  Baselines are append-only via release tags.

### AD41 — Bench runs are observable via OTel

Each scenario emits a root span `efe.bench.scenario` with attributes
naming the scenario + the chaos fault.  Operators can pivot in
Tempo from a bench run to the controller's traces.

### AD42 — Chaos events are recorded in the result parquet

Each chaos fault start / stop is a row in the parquet with
`event_type = "chaos_start" | "chaos_stop"` so post-run analysis can
correlate latency spikes with chaos events.

### AD43 — Comparison output supports markdown + JSON + plain text

Markdown for PR comments, JSON for CI parsing, plain text for terminal
operators.

### AD44 — Helm chart `efe-bench-harness` is OPTIONAL

Operators running locally use the binary directly.  In-cluster Helm
chart is for ops teams that want a CronJob-driven weekly bench.

### AD45 — Reviewer checklist mandates baseline manifest review

Every release tag PR that touches `bench_baselines/` must include a
manifest.json explaining the cluster topology + harness version.

---

## 5. Operator personas + 8 use cases

### 5.1 Personas

| Persona | Goal | Frequency |
|---|---|---|
| **Release-eng** | Validate release candidate before tagging | Per-release |
| **Perf engineer** | Investigate regression flagged by CI | Per-PR |
| **SRE** | Validate chaos fault tolerance after a chart upgrade | Per-upgrade |
| **Compliance auditor** | Quarterly evidence harvest | Quarterly |
| **Partner / customer evaluator** | Reproduce vendor claims | Eval phase |
| **Open-source contributor** | Submit PR + see CI verdict | Per-PR |

### 5.2 The 8 canonical use cases

| # | Use case | Scenario(s) | Persona | CI ? |
|---|---|---|---|---|
| UC1 | Pre-release validation | All 10 | Release-eng | ✓ (full suite) |
| UC2 | Regression investigation | S01 + S02 | Perf engineer | manual |
| UC3 | Chart-upgrade chaos check | S05 + S06 | SRE | manual |
| UC4 | Compliance quarterly | S10 (SLO) + S09 (soak) | Compliance | scheduled |
| UC5 | Partner reproduction | S01 (smoke) | Partner | manual |
| UC6 | PR regression check | S01 + S04 + S10 (quick) | Contributor | ✓ (PR label) |
| UC7 | Custom dim-stress | New scenario YAML | Perf engineer | manual |
| UC8 | Multi-region health | S08 (multi-region) | SRE | scheduled |

---

## 6. Scenario catalogue — 10 scenarios overview

| ID | Name | Duration | QPS | Workload | Chaos | Tier |
|---|---|---|---|---|---|---|
| **S01** | `baseline_100qps_5min` | 5 min | 100 | iterate 100% | — | smoke |
| **S02** | `canary_1000qps_15min` | 15 min | 1000 | mixed 95/5 | — | perf |
| **S03** | `ramp_to_breakdown` | 20 min | 1 → 5000 ramp | iterate 100% | — | capacity |
| **S04** | `chaos_participant_loss` | 10 min | 200 | iterate 100% | kill participant @ 3min | chaos |
| **S05** | `chaos_network_partition` | 15 min | 200 | iterate 100% | NetEm split @ 5min | chaos |
| **S06** | `chaos_cert_expiry` | 10 min | 100 | iterate 100% | rotate cert mid-run | chaos |
| **S07** | `chaos_reload_storm` | 15 min | 500 | iterate 95 / state 5 | SIGHUP every 30s | chaos |
| **S08** | `multi_region_replay` | 20 min | 500 × 3 regions | mixed | — | multi-region |
| **S09** | `soak_2h_stability` | 2 h | 50 | mixed 95/5 | — | soak |
| **S10** | `slo_validation_30min` | 30 min | 200 | mixed 95/5 | — | SLO |

Each scenario maps to a YAML file in `bench/scenarios/Sxx_<name>.yaml`
+ a baseline at `bench_baselines/v{chart_version}/L2/Sxx_<name>.json`.

---

## 7. Per-scenario design (S01 – S10)

> Each subsection follows the same shape : (a) intent, (b) success
> criterion, (c) YAML excerpt, (d) tolerance.

### 7.1 S01 — `baseline_100qps_5min`

**Intent** : sanity check.  Light load, no chaos.  Detects gross
regression (10×) without polluting nightly CI cycle.

**Success criterion** : `p95 ≤ baseline × 1.10` AND `error_rate ≤ 0.1 %`.

**YAML** (excerpt — full at `bench/scenarios/S01_baseline_100qps_5min.yaml`) :

```yaml
schema_version: 1
id: S01_baseline_100qps_5min
description: "5-min baseline sanity at 100 QPS — quick CI gate"
duration: "5m"
cluster:
  provisioner: kind        # kind | k3s | external
  kind_image: kindest/node:v1.30.0
  topology:
    controller_replicas: 2
    participant_replicas: 3
loadgen:
  # Verbatim PR #20 profile (subset of profile-v1.json)
  version: 1
  name: S01-loadgen
  target:
    endpoints:
      - url: "grpc://efe-meta-swarm-controller.efe.svc:50051"
        weight: 1.0
  pattern: { type: constant, qps: 100 }
  workload: { type: mixed, mix: { iterate: 1.0 } }
  termination: { duration: "5m" }
chaos: []                   # no faults for smoke
results:
  collect_prom_snapshot: true
  collect_chaos_events: false
tolerance:
  p50_ms_max_delta_pct: 15
  p95_ms_max_delta_pct: 10
  p99_ms_max_delta_pct: 15
  error_rate_max_delta_pp: 0.1
  throughput_qps_min_delta_pct: -10
```

**Tolerance** : strict for smoke ; p95 only 10 % drift.

### 7.2 S02 — `canary_1000qps_15min`

**Intent** : production-like load + mixed workload + manifest_version
labelled.  Validates dashboards + alerts under realistic traffic.

**Success criterion** : `p95 ≤ baseline × 1.15`, `error_rate ≤ 0.5 %`.

**YAML** : 1000 QPS for 15 min, 95% iterate + 5% state, hybrid rate
controller.

**Tolerance** : p95 15 % ; p99 20 % (higher tail noise tolerated).

### 7.3 S03 — `ramp_to_breakdown`

**Intent** : capacity ceiling discovery.  Ramp from 1 QPS to 5000 QPS
linearly over 20 min ; stop when p95 > 5 s OR error_rate > 5 %.

**Success criterion** : breakdown QPS within ± 30 % of baseline.

**YAML** : `loadgen.pattern: { type: ramp, initial_qps: 1, final_qps: 5000, duration: 20m }`.

**Tolerance** : `breakdown_qps_min_delta_pct: -30` (allow regression to
70 % of baseline before failing).

### 7.4 S04 — `chaos_participant_loss`

**Intent** : tolerance to single-participant failure.  Run 200 QPS
for 10 min ; at t=3min, kill participant pod via chaos-mesh PodChaos.
Verify error rate spikes briefly then recovers within 30 s.

**Success criterion** :
- `recovery_time_s ≤ 30`
- `error_rate_during_fault_max ≤ 5 %`
- `error_rate_post_recovery ≤ 0.5 %`

**YAML** :

```yaml
chaos:
  - id: kill-p1
    at_offset: "3m"
    crd_file: pr21_meta_swarm/participant-loss.yaml
    duration: "permanent"   # not auto-cleaned ; harness restarts pod
  - id: heal-p1
    at_offset: "3m30s"
    action: "delete-fault"
```

**Tolerance** : `recovery_time_s_max_delta_s: 10` (recovery up to 10 s
slower than baseline).

### 7.5 S05 — `chaos_network_partition`

**Intent** : tolerance to network split between controller and a
participant.  chaos-mesh NetworkChaos.netem partition.

**Success criterion** : `error_rate ≤ 30 %` during partition,
recovers to baseline within 60 s of healing.

### 7.6 S06 — `chaos_cert_expiry`

**Intent** : cert rotation mid-run.  Generate a 2-min-validity cert at
scenario start ; expires at t=2min ; cert-manager auto-renews.

**Success criterion** : `error_rate_during_rotation ≤ 1 %` (window 30 s).

### 7.7 S07 — `chaos_reload_storm`

**Intent** : SIGHUP-driven manifest reload storm.  Send `kill -HUP`
to controller every 30 s for 15 min.

**Success criterion** : `reload_success_rate ≥ 99 %`,
`p95 ≤ baseline × 1.20`.

### 7.8 S08 — `multi_region_replay` (Phase-3 prep)

**Intent** : 3-region weighted traffic.  Multi-target loadgen
(PR #20 § 26).  Validates cross-region routing.

**Success criterion** : per-region `p95 ≤ baseline × 1.15` ; cross-
region variance ≤ 50 ms.

### 7.9 S09 — `soak_2h_stability`

**Intent** : leak / drift detection.  Run 50 QPS for 2 hours.
Detect memory leak, FD leak, latency drift.

**Success criterion** :
- `p95_drift_ms ≤ 50 ms` (last 30 min vs first 30 min)
- `memory_drift_pct ≤ 5 %`
- `fd_count_delta ≤ 100`

### 7.10 S10 — `slo_validation_30min`

**Intent** : 30-min run validating all 4 STABILITY_CONTRACT v1.1 § 9.5
SLO targets simultaneously.  Compliance evidence.

**Success criterion** : all 4 SLOs ≥ target.  Auto-publishes the
quarterly evidence file to `docs/bench/site/compliance/{YYYYQq}/`.

---

## 8. Chaos fault library (6 chaos-mesh CRDs)

### 8.1 `participant-loss.yaml`

```yaml
apiVersion: chaos-mesh.org/v1alpha1
kind: PodChaos
metadata:
  name: participant-loss
  namespace: ${BENCH_NAMESPACE}
spec:
  action: pod-kill
  mode: one
  selector:
    labelSelectors:
      app.kubernetes.io/name: efe-meta-swarm
      app.kubernetes.io/component: participant
  duration: "5s"
```

### 8.2 `network-partition.yaml`

```yaml
apiVersion: chaos-mesh.org/v1alpha1
kind: NetworkChaos
spec:
  action: partition
  mode: all
  selector:
    labelSelectors: { app.kubernetes.io/component: participant }
  direction: both
  duration: "60s"
```

### 8.3 `cert-expiry.yaml`

```yaml
# Phase-3 stub — placeholder for cert-manager-driven rotation.
# Actual fault is shell script that recreates the Certificate CRD
# with renewBefore tuned to fire immediately.
```

### 8.4 `reload-storm.yaml`

```yaml
apiVersion: chaos-mesh.org/v1alpha1
kind: JVMChaos    # actually a kubectl-exec script via PodChaos
spec:
  action: exec
  command: "kill -HUP 1"
  frequency: "30s"
  duration: "15m"
```

### 8.5 `cpu-throttle.yaml`

```yaml
apiVersion: chaos-mesh.org/v1alpha1
kind: StressChaos
spec:
  mode: one
  selector: { labelSelectors: { app.kubernetes.io/component: controller } }
  stressors:
    cpu:
      workers: 4
      load: 80
  duration: "5m"
```

### 8.6 `memory-pressure.yaml`

Similar shape to CPU, type StressChaos with memory pressure.

---

## 9. Bench orchestrator (`efe-bench-harness` Rust binary)

### 9.1 Module layout

```
bench/harness/
├── Cargo.toml                 # workspace member ; depends on efe-meta-swarm-loadgen lib
├── README.md
├── src/
│   ├── main.rs                # CLI entry
│   ├── lib.rs                 # public API for embedded use
│   ├── cli.rs                 # clap CLI (subcommands : run, list, compare, publish)
│   ├── scenario/
│   │   ├── mod.rs             # Scenario struct + loader
│   │   └── schema.rs          # serde types
│   ├── cluster/
│   │   ├── mod.rs             # ClusterProvisioner trait
│   │   ├── kind.rs            # kind-based provisioner
│   │   ├── k3s.rs             # k3s-based provisioner
│   │   └── external.rs        # --kubeconfig passthrough
│   ├── chaos/
│   │   ├── mod.rs             # ChaosOrchestrator
│   │   └── applier.rs         # kubectl apply / delete
│   ├── loadgen/
│   │   ├── mod.rs             # PR #20 integration (in-process via library)
│   │   └── spawn.rs           # alternative subprocess mode
│   ├── result/
│   │   ├── mod.rs             # Result aggregator
│   │   ├── parquet.rs         # Phase-3 — real parquet
│   │   ├── snapshot.rs        # Prom /api/v1/snapshot
│   │   └── events.rs          # chaos events recorder
│   ├── compare/
│   │   ├── mod.rs             # ComparisonEngine
│   │   ├── tolerance.rs       # per-metric tolerance check
│   │   └── verdict.rs         # ✅/⚠/❌
│   ├── baseline/
│   │   ├── mod.rs             # Baseline store
│   │   └── git.rs             # git LFS helpers
│   ├── publish/
│   │   ├── mod.rs             # GitHub Pages publisher
│   │   ├── badge.rs           # SVG badge generator
│   │   └── mdbook.rs          # mdbook page renderer
│   ├── pr_comment/
│   │   └── mod.rs             # GitHub PR comment formatter
│   └── engine.rs              # Top-level orchestrator
├── tests/
│   ├── golden_scenarios.rs    # Each scenario YAML loads + validates
│   ├── compare_test.rs        # Tolerance verdict scenarios
│   ├── chaos_test.rs          # Chaos applier (against fake kubectl)
│   └── data/
│       ├── baseline-v0.5.0/   # historical baseline for golden compare
│       └── golden-cli.txt
└── helm/                       # AD44 — optional in-cluster orchestrator
    ├── Chart.yaml
    └── templates/
```

### 9.2 CLI subcommands

| Subcommand | Purpose |
|---|---|
| `run` (default) | Run one or more scenarios end-to-end |
| `list` | List discovered scenarios |
| `validate` | Validate scenario YAMLs against schema |
| `compare` | Compare a result file vs a baseline file |
| `baseline-regen` | Regenerate baseline (release-tag-only) |
| `publish` | Build GitHub Pages site + badges |
| `version` | Print version + git SHA |

### 9.3 Public library API

```rust
pub mod scenario;
pub mod cluster;
pub mod loadgen;
pub mod result;
pub mod compare;
pub mod baseline;

pub use crate::engine::run;
pub use crate::scenario::Scenario;
pub use crate::result::ScenarioResult;
pub use crate::compare::Verdict;
```

---

## 10. Scenario YAML schema v1

Complete schema (excerpt — full at `bench/schemas/scenario-v1.json`) :

```yaml
schema_version: 1            # REQUIRED
id: <string ^S[0-9]+_[a-z0-9_]+$>
description: <string>
duration: <humantime>        # e.g. "5m", "2h" — hard wall-clock cap

cluster:
  provisioner: kind | k3s | external
  kind_image: <string>       # when provisioner=kind
  k3s_version: <string>      # when provisioner=k3s
  kubeconfig: <path>         # when provisioner=external
  topology:
    controller_replicas: <int 1..10>
    participant_replicas: <int 1..50>
    namespace: <string>      # default = efe-bench-{id}-{run_id}

# Verbatim PR #20 profile-v1 (subset).  Schema-validated by reusing
# `tools/loadgen/schemas/profile-v1.json` $ref.
loadgen:
  version: 1
  name: <string>
  target: { endpoints: [...] }
  pattern: { ... }
  workload: { ... }
  termination: { duration: ... }
  observability: { ... }

# Chaos faults to inject during the run.
chaos:
  - id: <string>
    at_offset: <humantime>   # offset from scenario start
    crd_file: <path>         # relative to deploy/chaos/pr21_meta_swarm/
    duration: <humantime>    # | "permanent"
  - id: <heal-name>
    at_offset: <humantime>
    action: delete-fault

# Result collection knobs.
results:
  collect_prom_snapshot: <bool>   # snapshot Prometheus /api/v1/snapshot
  collect_chaos_events: <bool>
  output_dir: <path>              # default = bench/results/{run_id}/{scenario_id}/

# Per-metric tolerance vs baseline.
tolerance:
  p50_ms_max_delta_pct: <number>
  p95_ms_max_delta_pct: <number>
  p99_ms_max_delta_pct: <number>
  error_rate_max_delta_pp: <number>
  throughput_qps_min_delta_pct: <number>
  # Optional chaos-specific tolerances
  recovery_time_s_max_delta_s: <number>
```

---

## 11. Result format

Each scenario produces three artefacts in `bench/results/{run_id}/{scenario_id}/` :

### 11.1 `result.parquet`

Per-request rows (matches PR #20 ndjson stub for now ; real parquet in
Phase-3 G.7).

### 11.2 `summary.json`

```json
{
  "schema_version": 1,
  "scenario_id": "S01_baseline_100qps_5min",
  "harness_version": "0.1.0",
  "loadgen_version": "0.1.0",
  "chart_version": "0.6.0",
  "image_tag": "v0.6.1",
  "git_commit": "abc123def",
  "run_id": "uuid-...",
  "started_at": "2026-05-16T10:00:00Z",
  "duration_s": 300.42,
  "total_requests": 30025,
  "metrics": {
    "p50_ms": 8.1,
    "p95_ms": 245.3,
    "p99_ms": 980.1,
    "error_rate": 0.0012,
    "throughput_qps": 99.8
  },
  "chaos_events": [],
  "verdict": "pass"
}
```

### 11.3 `prom-snapshot.tar.gz` (optional)

Prometheus's `/api/v1/snapshot` output : a tarball of TSDB chunks at
scenario end.  Operators can re-import in a separate Prometheus for
post-hoc analysis.

---

## 12. Comparison + verdict engine

### 12.1 Verdict levels (closed taxonomy)

```
pass    — every metric within tolerance
warn    — one or more metrics ⚠ but no ❌
fail    — at least one ❌
no-baseline   — first run for this chart version (auto-pass)
```

### 12.2 Per-metric rules

```rust
for (name, tolerance) in scenario.tolerance.iter() {
    let actual = result.metrics.get(name)?;
    let baseline_val = baseline.metrics.get(name)?;
    let verdict = check(name, baseline_val, actual, tolerance);
    overall = overall.combine(verdict);
}
```

Per-metric check is direction-aware :

| Metric | Direction | Rule |
|---|---|---|
| `p50_ms / p95_ms / p99_ms` | lower-is-better | `actual ≤ baseline × (1 + tol_pct/100)` |
| `error_rate` | lower-is-better | `actual ≤ baseline + tol_pp/100` |
| `throughput_qps` | higher-is-better | `actual ≥ baseline × (1 + tol_pct/100)` |
| `recovery_time_s` | lower-is-better | `actual ≤ baseline + tol_s` |

### 12.3 PR comment format

```markdown
## 🔬 Bench results — `efe-bench-harness@0.1.0`

| Scenario | Verdict | p50 (ms) | p95 (ms) | p99 (ms) | error_rate | throughput (qps) |
|---|---|---|---|---|---|---|
| S01_baseline_100qps_5min | ✅ pass | 8.1 (-3%) | 245.3 (-1%) | 980.1 (+2%) | 0.12% (=) | 99.8 (=) |
| S04_chaos_participant_loss | ⚠ warn | 9.4 (+8%) | 412 (+18% ⚠) | 1820 (+12%) | 1.2% (+0.3pp ⚠) | 198 (-1%) |
| S10_slo_validation_30min | ❌ fail | 10.1 (+30% ❌) | 1480 (+47% ❌) | 5100 (+82% ❌) | 0.4% (+0.2pp) | 195 (-2%) |

🔗 Full results : [bench/results/{run_id}/](https://efe.github.io/bench/runs/{run_id}/)
🔗 Methodology : [bench/README.md](https://efe.github.io/bench/)
```

---

## 13. Baseline management

### 13.1 Layout

```
bench_baselines/
├── README.md                        # existing — extended with PR #21 conventions
├── v0.6.0/                          # NEW
│   ├── manifest.json                # AD30
│   └── L2/                          # NEW (separates from existing Criterion)
│       ├── S01_baseline_100qps_5min.json
│       ├── ...
│       └── S10_slo_validation_30min.json
└── v0.x.y/L2/                       # future chart versions
```

### 13.2 Regen rules (AD16 + AD40)

  - Baselines update **only** on release-tag CI workflow (`bench-release.yml`).
  - PR-time regression flagged ❌ does NOT update baselines — author
    must explicitly run `make bench-regen-pr-{N}` + open a separate
    "regen baseline" PR explaining the regression.
  - Historical baselines never deleted — they're committed evidence.

### 13.3 manifest.json

```json
{
  "chart_version": "0.6.0",
  "harness_version": "0.1.0",
  "loadgen_version": "0.1.0",
  "git_commit": "abc123",
  "created_at": "2026-05-16T10:00:00Z",
  "cluster": {
    "provisioner": "kind",
    "k8s_version": "1.30.0",
    "node_class": "github-runner-2vcpu-7gb"
  },
  "scenarios": [
    {"id": "S01_baseline_100qps_5min", "duration_s": 300, "verdict": "pass"},
    ...
  ]
}
```

---

## 14. CI integration (4 workflows)

### 14.1 `bench-quick.yml`

Triggers : PR labelled `bench` ; push to main.
Runs : S01 + S04 + S10 (3 scenarios, ~ 8 min wall-clock).
Outputs : PR comment with table.

### 14.2 `bench-full.yml`

Triggers : nightly cron (03:00 UTC) ; manual workflow_dispatch.
Runs : all 10 scenarios.
Outputs : commits `bench/results/nightly/{date}/` to a side branch.

### 14.3 `bench-release.yml`

Triggers : tag push matching `v[0-9]+.[0-9]+.[0-9]+`.
Runs : all 10 scenarios.
Outputs : opens PR with `bench_baselines/v{tag}/L2/*.json` updates.

### 14.4 `bench-compare.yml`

Triggers : PR-comment "/bench-compare {commit-sha}".
Runs : compare existing result files vs the named commit.
Outputs : table comment.

---

## 15. GitHub Pages publishing

### 15.1 Site structure

```
docs/bench/site/                  # mdbook source
├── book.toml
├── src/
│   ├── SUMMARY.md
│   ├── overview.md
│   ├── methodology.md
│   ├── scenarios/
│   │   ├── S01.md
│   │   └── ... (one MD per scenario)
│   ├── history.md                # historical baselines graph
│   └── compliance.md             # quarterly evidence files
└── theme/                        # custom mdbook theme (efe branding)
```

### 15.2 Badge generation

`docs/bench/site/badges/Sxx.svg` generated by `efe-bench-harness publish` :

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="120" height="20">
  <rect width="60" height="20" fill="#555"/>
  <rect width="60" height="20" x="60" fill="#4c1"/>
  <text x="30" y="14" fill="#fff" text-anchor="middle">S01</text>
  <text x="90" y="14" fill="#fff" text-anchor="middle">12ms</text>
</svg>
```

Badges cached for 1 hour (AD26).

### 15.3 README badge wall

```markdown
## Public bench results

![S01](https://efe.github.io/bench/badges/S01.svg)
![S02](https://efe.github.io/bench/badges/S02.svg)
... (10 badges)
```

---

## 16. Cluster provisioning strategies

### 16.1 `kind` provisioner (default for CI)

```rust
pub struct KindProvisioner {
    image: String,
    cluster_name: String,
}

impl ClusterProvisioner for KindProvisioner {
    async fn provision(&self) -> Result<KubeconfigPath> {
        run("kind", &["create", "cluster", "--name", &self.cluster_name, "--image", &self.image]).await?;
        let kubeconfig = run("kind", &["get", "kubeconfig", "--name", &self.cluster_name]).await?;
        // ...
    }
    async fn teardown(&self) -> Result<()> {
        run("kind", &["delete", "cluster", "--name", &self.cluster_name]).await
    }
}
```

### 16.2 `k3s` provisioner (default for dev)

Uses `k3s server` directly ; assumes Linux dev box.

### 16.3 `external` provisioner (operator-supplied kubeconfig)

```rust
pub struct ExternalProvisioner { kubeconfig_path: PathBuf }
// no provision / teardown : caller manages
```

### 16.4 Cluster lifetime per scenario

Default : one cluster reused for all scenarios in a run.  Optional
`--cluster-per-scenario` flag spins a fresh cluster per scenario (slow
but hermetic).

---

## 17. chaos-mesh integration

### 17.1 chaos-mesh install via Helm

If a scenario references chaos faults AND chaos-mesh isn't already
installed, the harness installs it :

```bash
helm install chaos-mesh chaos-mesh/chaos-mesh \
    --namespace chaos-mesh --create-namespace \
    --version 2.6.2 \
    --wait
```

### 17.2 Fault application + cleanup

```rust
async fn apply_fault(&self, fault: &ChaosFault) -> Result<()> {
    let path = self.fault_path(&fault.crd_file)?;
    run("kubectl", &["apply", "-f", &path]).await?;
    Ok(())
}
async fn delete_fault(&self, fault: &ChaosFault) -> Result<()> {
    let path = self.fault_path(&fault.crd_file)?;
    run("kubectl", &["delete", "-f", &path, "--ignore-not-found"]).await
}
```

### 17.3 Fault timing precision

The harness schedules faults via `tokio::time::sleep` based on
`at_offset` from scenario start.  Precision : ± 100 ms (Linux timer
granularity).

---

## 18. PR #20 loadgen integration

### 18.1 In-process library mode

```rust
use efe_meta_swarm_loadgen::{run, LoadgenConfig};
let cfg = LoadgenConfig::from_profile(&scenario.loadgen)?;
let summary = run(cfg).await?;
```

### 18.2 Subprocess mode (fallback)

```rust
let status = Command::new("efe-meta-swarm-loadgen")
    .args(["run", "--profile", profile_path])
    .status().await?;
```

In-process is the default ; subprocess is opt-in via `--loadgen-subprocess`.

---

## 19. Multi-region benches (Phase-3 prep)

S08 multi-region uses loadgen's `--target` repeatable flag with
weight suffixes (PR #20 § 39).  Each region's controller has its
own kubeconfig ; the harness coordinates 3 simultaneous kubectl
contexts.

Phase-3 follow-up : federated comparison engine (cross-region
variance metric).

---

## 20. Performance budget (per-scenario wall-clock cap)

| Scenario | Cap | Typical wall-clock |
|---|---|---|
| S01 | 10 min | 7 min (5 min run + 2 min setup) |
| S02 | 20 min | 17 min |
| S03 | 25 min | 22 min |
| S04 | 15 min | 12 min |
| S05 | 20 min | 17 min |
| S06 | 15 min | 12 min |
| S07 | 20 min | 17 min |
| S08 | 30 min | 25 min |
| S09 | 2.5 h | 2 h 10 min |
| S10 | 40 min | 35 min |

**bench-quick** (S01 + S04 + S10) : ~ 54 min cap.  Acceptable for
labelled-PR CI cost (uses dedicated self-hosted runner).

**bench-full** (S01-S10) : ~ 6 h cap.  Nightly only.

---

## 21. STABILITY_CONTRACT v1.3 additions

### 21.1 New tiers

| Tier | Promise | Notice |
|---|---|---|
| **Scenario ID** | `Sxx_<name>` is frozen ≥ 6 chart-minor | 6-minor |
| **Baseline tolerance** | Default tolerance per scenario frozen | 6-minor |
| **Result schema version** | `result.json` schema v1 frozen | 6-minor |

### 21.2 Frozen scenario IDs

```
S01_baseline_100qps_5min
S02_canary_1000qps_15min
S03_ramp_to_breakdown
S04_chaos_participant_loss
S05_chaos_network_partition
S06_chaos_cert_expiry
S07_chaos_reload_storm
S08_multi_region_replay
S09_soak_2h_stability
S10_slo_validation_30min
```

### 21.3 Frozen comparison metric set (closed taxonomy)

```
p50_ms | p95_ms | p99_ms | error_rate | throughput_qps | recovery_time_s
```

Renaming or removing breaks the contract.  Adding new is additive
(non-breaking) with default `null` for historical baselines.

---

## 22. Security model + Kubernetes RBAC

### 22.1 Harness ServiceAccount RBAC

Namespace-scoped (PR #20 § 24.1 pattern + chaos-mesh extension) :

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
rules:
  # Standard loadgen RBAC
  - apiGroups: [""]
    resources: ["services", "endpoints"]
    verbs: ["get", "list", "watch"]
  # Chaos-mesh CRD application
  - apiGroups: ["chaos-mesh.org"]
    resources: ["podchaos", "networkchaos", "stresschaos"]
    verbs: ["create", "get", "list", "delete"]
  # Prom snapshot pull
  - apiGroups: ["monitoring.coreos.com"]
    resources: ["prometheuses"]
    verbs: ["get"]
```

NO ClusterRole (T93 mitigation).

### 22.2 Production refusal (T86)

```rust
if scenario.loadgen.run.mode == RunMode::Production {
    return Err(BenchError::ProductionForbidden);
}
```

### 22.3 Baseline tampering (T87)

`bench_baselines/` modifications require :
- Commit signed by release-bot (GPG-verified)
- PR review by ≥ 2 release-eng reviewers
- CI passes `bench-release` workflow

---

## 23. Threat-model coverage (T86 – T95)

| ID | Threat | Severity | Mitigation | AC |
|---|---|---|---|---|
| T86 | Chaos fault leaks to production | SEV-2 | Hardcoded refusal of `loadgen.run.mode=production` (AD11) ; CI gate | AC10 |
| T87 | Baseline tampering | SEV-3 | Release-tag-only updates (AD40) ; signed commits | AC15 |
| T88 | GitHub Pages defacement | SEV-3 | Static site ; build only on tag ; mdbook sandboxing | AC18 |
| T89 | PR comment injection via untrusted scenario | SEV-3 | Scenario fields HTML-escaped before comment ; no operator-supplied template | AC22 |
| T90 | Kubeconfig leak in bench logs | SEV-3 | Path-only logging ; never `cat` config (AD31) | AC12 |
| T91 | Scenario YAML SSRF | SEV-3 | No remote `!Include` ; local paths only (AD32) | AC9 |
| T92 | Parquet result file DoS | SEV-3 | 100 MB cap with truncation (AD33) | AC17 |
| T93 | Chaos-mesh CRD privilege escalation | SEV-2 | Namespace-scoped Role only (AD34) ; T79 reuse | AC20 |
| T94 | Bench-as-timing-oracle leak | SEV-3 | Seed determinism removes timing as oracle (AD35) | AC14 |
| T95 | Bench priority inversion on shared cluster | SEV-2 | `bench-dedicated=true` taint check + warning (AD36) | AC30 |

All ≤ SEV-2.  No PII (bench data is synthetic).  No auth bypass.

---

## 24. Acceptance criteria (AC1 – AC35)

| # | Criterion | Verified by |
|---|---|---|
| AC1 | Workspace member `bench/harness` exists + builds | `cargo build` |
| AC2 | Binary `efe-bench-harness` exists in target/release | filesystem check |
| AC3 | 10 scenario YAMLs at `bench/scenarios/S{01..10}_*.yaml` | ls |
| AC4 | All scenarios validate against `bench/schemas/scenario-v1.json` | CI Python jsonschema |
| AC5 | 6 chaos fault YAMLs at `deploy/chaos/pr21_meta_swarm/` | ls |
| AC6 | Embedded loadgen profile references valid PR #20 schema | profile-v1 cross-validation |
| AC7 | `efe-bench-harness list` shows 10 scenarios | binary run |
| AC8 | `efe-bench-harness validate` exits 0 on all 10 | binary run |
| AC9 | YAML rejects remote `!Include` / `!fetch` | unit test |
| AC10 | Production-mode loadgen profile rejected | unit test |
| AC11 | kind provisioner works in CI | kind-cluster e2e |
| AC12 | Kubeconfig never appears in logs | log grep |
| AC13 | Chaos faults applied + deleted cleanly | integration test |
| AC14 | Same seed = bit-identical loadgen wire bytes | determinism test |
| AC15 | Baselines updated only via release-tag workflow | CI gate |
| AC16 | Comparison engine emits markdown / json / text | unit test |
| AC17 | Parquet result file ≤ 100 MB or truncated with warning | integration test |
| AC18 | GitHub Pages site builds via mdbook | CI build |
| AC19 | SVG badges generated correctly (10 files) | CI artefact |
| AC20 | RBAC namespace-scoped only | helm-unittest |
| AC21 | Per-scenario tolerance committed in STABILITY_CONTRACT | doc diff |
| AC22 | PR comment HTML-escaped | unit test |
| AC23 | Tolerance can be relaxed only via chart-major bump | doc reviewer check |
| AC24 | `bench-quick` CI workflow runs ≤ 1 h | CI timing |
| AC25 | `bench-full` CI workflow runs ≤ 6 h | nightly timing |
| AC26 | `bench-release` opens PR with baseline updates | release tag test |
| AC27 | `bench-compare` posts PR comment on `/bench-compare` | command test |
| AC28 | All 10 scenarios commit a baseline at chart 0.6.0 | filesystem check |
| AC29 | result `manifest.json` includes harness version + git SHA | unit test |
| AC30 | `bench-dedicated=true` taint absence warns | unit test |
| AC31 | Doc parity : architecture ≥ 2 500 lines | wc -l |
| AC32 | `bench/README.md` is the public methodology page | doc check |
| AC33 | STABILITY_CONTRACT v1.3 introduces 3 new tiers | doc diff |
| AC34 | Existing `crates/efe-bench` unchanged (no D18 churn) | git diff empty |
| AC35 | `scripts/benches.list` 7-field rows still parse | unit test |

---

## 25. Tests strategy

### 25.1 Unit tests (~ 150)

Per-module tests covering :
- Scenario YAML loading + validation (~ 25 tests)
- Cluster provisioner trait (~ 10 tests)
- Chaos applier (mocked kubectl, ~ 15 tests)
- Comparison + verdict engine (~ 30 tests, exhaustive tolerance scenarios)
- Baseline store + manifest serialisation (~ 15 tests)
- PR comment formatter (~ 10 tests, including escape tests)
- Badge SVG generator (~ 8 tests)
- CLI parser (~ 25 tests)
- Integration with PR #20 loadgen (~ 12 tests)

### 25.2 Integration tests (~ 20)

- Scenario S01 e2e against a mocked controller (~ 30 s)
- Chaos applier against fake kubectl + scenario timeline
- Compare engine against fixture baseline files

### 25.3 kind-cluster e2e (CI only)

`bench-quick.yml` runs S01 + S04 + S10 end-to-end in a kind cluster.
Asserts `verdict != "fail"` for all three.

### 25.4 Golden CLI snapshot

`tests/data/golden-cli.txt` matches `efe-bench-harness --help` output.
Updated via `cargo run -- --help > tests/data/golden-cli.txt`.

### 25.5 Property tests

`proptest`-based : compare engine handles all (baseline, actual, tolerance)
triples without panic ; verdict is deterministic.

---

## 26. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| CI flakiness (kind cluster network) | High | False CI failures | Retry up to 3× ; AD13 wall-clock cap aborts hung scenarios |
| chaos-mesh version drift | Medium | Fault CRDs break | Pin chaos-mesh chart version 2.6.x ; CI matrix tests 2.6.x + 2.7.x |
| Baseline noise wider than tolerance | Medium | False positives | Tolerance set conservatively at v0.6.0 ; iterate |
| Parquet file size explosion | Low | Disk full | AD33 100 MB cap |
| GitHub Pages outage | Low | Badges break | Static CDN fallback |
| Nightly CI cost | Medium | $$$ | Self-hosted runner ; budget capped |
| `bench_baselines/` git LFS quota exhaustion | Medium | LFS fee | AD27 opt-in LFS, JSON summaries stay in git |
| New scenario authors break schema | Medium | Bench rotting | AD37 + JSON-Schema gate in CI |
| Soak (S09) leaks runner resources | Low | Runner dies | Dedicated soak runner (AD24) |
| Bench mode confused with production loadgen | Low | T86 production hit | AD11 hardcoded refusal |

---

## 27. Rollback strategy

### 27.1 Code rollback

`git revert` is safe : the harness is stateless ; no DB state ;
no in-cluster persistent resources beyond the namespace, which is
auto-cleaned at scenario end (AD22).

### 27.2 Baseline rollback

If a release baseline is bad : open a "revert baseline" PR that
re-commits the prior version's `bench_baselines/v{N-1}/L2/*.json` to
`v{N}/L2/*.json`.

### 27.3 GitHub Pages rollback

Revert the `gh-pages` branch to the previous commit ;
`peaceiris/actions-gh-pages` republishes within 5 min.

---

## 28. Sequence diagrams

### 28.1 Scenario run flow

```
operator ──> efe-bench-harness run --scenario S04
       │
       ▼
provisioner.kind.provision()
       │ kubectl ... create cluster
       ▼
helm install efe-meta-swarm + chaos-mesh (if needed)
       │
       ▼
loadgen.start(profile=scenario.loadgen)    # in-process via PR #20 lib
       │
       ▼  (T = 0)
       │  ... 200 QPS for 10 min ...
       │
       ▼  (T = 3min)
chaos.apply(participant-loss.yaml)
       │
       ▼  (T = 3min 5s)
chaos.delete(participant-loss.yaml)        # heal
       │
       ▼  (T = 10min)
loadgen.stop()
       │
       ▼
result.collect()
       │  - parquet from loadgen
       │  - prom snapshot
       │  - chaos events
       ▼
compare.engine(result, baseline)
       │
       ▼
verdict = "pass" | "warn" | "fail"
       │
       ▼
publish.pr_comment(verdict)
       │
       ▼
provisioner.kind.teardown()
       │
       ▼
exit(verdict.exit_code())
```

### 28.2 Baseline regen flow (release tag)

```
git tag v0.6.0 ─push─> bench-release.yml fires
       │
       ▼
run full suite (10 scenarios)
       │
       ▼  (all pass)
generate bench_baselines/v0.6.0/L2/*.json + manifest.json
       │
       ▼
github-bot opens PR "chore: regen baseline for v0.6.0"
       │
       ▼  (operator merges)
GitHub Pages republishes
```

---

## 29. Reviewer checklist

> Every box ticked before LGTM.

**Foundations**
  - [ ] AD1–AD45 reviewed + consistent with PR #18/19/20 conventions
  - [ ] STABILITY_CONTRACT v1.3 diff reviewed (§ 21)
  - [ ] No churn on `crates/efe-bench` (AC34)
  - [ ] `scripts/benches.list` 7-field rows still parse (AC35)

**Rust binary**
  - [ ] 10 scenarios load + validate
  - [ ] 6 chaos faults apply + cleanup
  - [ ] PR #20 loadgen integration (in-process + subprocess paths)
  - [ ] Comparison engine + per-metric tolerance committed

**Scenarios**
  - [ ] Each Sxx YAML has methodology + tolerance + chaos plan
  - [ ] No scenario references production
  - [ ] All tolerances justified in scenario YAML comments

**CI**
  - [ ] 4 workflows (quick / full / release / compare) exist + green
  - [ ] kind-cluster smoke runs in `bench-quick` ≤ 1 h
  - [ ] PR comment formatter HTML-escapes

**Docs**
  - [ ] Architecture (this) ≥ 2 500 lines
  - [ ] `bench/README.md` is public methodology
  - [ ] GitHub Pages mdbook builds
  - [ ] 10 SVG badges generated

**Security**
  - [ ] T86–T95 mitigations verified
  - [ ] Production-mode refusal hard-coded
  - [ ] RBAC namespace-scoped (no ClusterRole)

---

## 30. Appendix A — CLI flag reference

```
USAGE :
    efe-bench-harness [SUBCOMMAND] [OPTIONS]

SUBCOMMANDS :
    run            Execute one or more scenarios
    list           List discovered scenarios
    validate       Validate scenario YAMLs vs schema
    compare        Compare result vs baseline
    baseline-regen Regenerate baseline (release-tag only)
    publish        Build GitHub Pages site + badges
    version        Print version + git SHA

GLOBAL FLAGS :
    --scenario <ID>           Scenario ID (e.g. S01_baseline_100qps_5min)
    --scenarios <DIR>         Scenarios directory (default bench/scenarios/)
    --baselines <DIR>         Baselines directory (default bench_baselines/)
    --output-dir <PATH>       Result output dir (default bench/results/{run_id}/)
    --kubeconfig <PATH>       External cluster (provisioner=external)
    --provisioner <KIND>      kind | k3s | external
    --cluster-per-scenario    Fresh cluster per scenario (slow)
    --loadgen-subprocess      Spawn loadgen as subprocess (default: in-process)
    --skip-teardown           Keep cluster alive after run (debug)
    --pr-comment-file <PATH>  Write PR comment markdown here
    --verdict-json-file <P>   Write JSON verdict here
    --chart-version <VER>     Chart version under test (default: detect)
    --image-tag <TAG>         Image tag (default: detect)
    --seed <U64>              Loadgen seed (default: wall_clock_ns)
    --mode <MODE>             dev | staging  (production REFUSED)
    --log-format <FORMAT>     text | json
    --log-level <LEVEL>       trace | debug | info | warn | error
    --version                 Print version
    --help                    Print help
```

---

## 31. Appendix B — Scenario YAML schema (annotated)

Full schema at `bench/schemas/scenario-v1.json` (~ 350 lines JSON-Schema).

Key invariants :
  - `schema_version: 1` mandatory.
  - `id` matches `^S[0-9]{2}_[a-z0-9_]+$`.
  - `duration` ≤ 2h (S09 only ; others ≤ 30 min).
  - `cluster.provisioner` ∈ {kind, k3s, external}.
  - `loadgen` is a valid PR #20 profile-v1 doc.
  - `chaos[].crd_file` is a path relative to `deploy/chaos/pr21_meta_swarm/`.
  - `tolerance.*` defaults committed in STABILITY_CONTRACT v1.3.

---

## 32. Appendix C — Chaos fault YAML library

6 CRDs under `deploy/chaos/pr21_meta_swarm/` :

| File | chaos-mesh kind | What it does |
|---|---|---|
| `participant-loss.yaml` | `PodChaos` (pod-kill) | Kills 1 participant for 5 s |
| `network-partition.yaml` | `NetworkChaos` (partition) | Splits controller ↔ participants 60 s |
| `cert-expiry.yaml` | bespoke kubectl script | Recreates Certificate with 2-min validity |
| `reload-storm.yaml` | `PodChaos` (exec command) | `kill -HUP 1` every 30 s for 15 min |
| `cpu-throttle.yaml` | `StressChaos` (cpu) | 80% CPU load on controller for 5 min |
| `memory-pressure.yaml` | `StressChaos` (memory) | 2 GB allocation on controller for 5 min |

---

## 33. Appendix D — Comparison rules per metric

```yaml
defaults_v1:
  p50_ms:
    direction: lower-is-better
    rule: actual ≤ baseline * (1 + tol_pct/100)
    warn_at_pct_over_baseline: 8
    fail_at_pct_over_baseline: 15
  p95_ms:
    direction: lower-is-better
    warn_at_pct_over_baseline: 12
    fail_at_pct_over_baseline: 25
  p99_ms:
    direction: lower-is-better
    warn_at_pct_over_baseline: 15
    fail_at_pct_over_baseline: 35
  error_rate:
    direction: lower-is-better
    rule: actual ≤ baseline + tol_pp/100
    warn_at_pp_over_baseline: 0.2
    fail_at_pp_over_baseline: 1.0
  throughput_qps:
    direction: higher-is-better
    rule: actual ≥ baseline * (1 + tol_pct/100)   # tol_pct is negative
    warn_at_pct_below_baseline: -5
    fail_at_pct_below_baseline: -15
  recovery_time_s:
    direction: lower-is-better
    rule: actual ≤ baseline + tol_s
    warn_at_s_over_baseline: 5
    fail_at_s_over_baseline: 15
```

Per-scenario override files in `bench/scenarios/Sxx_*.yaml` `tolerance:` block.

---

## 34. Appendix E — Baseline JSON schema

`bench/schemas/baseline-v1.json` :

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "EFE bench baseline v1",
  "type": "object",
  "required": ["schema_version", "scenario_id", "metrics"],
  "properties": {
    "schema_version": { "const": 1 },
    "scenario_id": { "type": "string" },
    "harness_version": { "type": "string" },
    "loadgen_version": { "type": "string" },
    "chart_version": { "type": "string" },
    "image_tag": { "type": "string" },
    "git_commit": { "type": "string", "pattern": "^[0-9a-f]{7,40}$" },
    "run_id": { "type": "string" },
    "started_at": { "type": "string", "format": "date-time" },
    "duration_s": { "type": "number", "minimum": 0 },
    "total_requests": { "type": "integer", "minimum": 0 },
    "metrics": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "p50_ms": { "type": "number", "minimum": 0 },
        "p95_ms": { "type": "number", "minimum": 0 },
        "p99_ms": { "type": "number", "minimum": 0 },
        "error_rate": { "type": "number", "minimum": 0, "maximum": 1 },
        "throughput_qps": { "type": "number", "minimum": 0 },
        "recovery_time_s": { "type": "number", "minimum": 0 }
      }
    },
    "chaos_events": { "type": "array" },
    "verdict": { "enum": ["pass", "warn", "fail", "no-baseline"] }
  }
}
```

---

## 35. Appendix F — Multi-region topology

Phase-3 design sketch for S08 :

```
operator ──> efe-bench-harness run --scenario S08
       │
       ▼
provisioner.external (kubeconfig: 3 contexts in 1 file)
       │
       ▼
loadgen targets :
  grpc://ctrl.us-east.efe.svc:50051 @ weight 0.4
  grpc://ctrl.eu-west.efe.svc:50051 @ weight 0.3
  grpc://ctrl.ap-south.efe.svc:50051 @ weight 0.3
       │
       ▼
result.collect()
  - 3 per-region prom snapshots
  - 1 unified loadgen parquet
       │
       ▼
compare engine emits per-region rows + global aggregate
```

---

## 36. Appendix G — Phase-3 follow-ups

### G.1 Auto-provision EKS / GKE / AKS

Currently external-kubeconfig-only.  Phase-3 adds cloud-specific
provisioners with billing guardrails.

### G.2 ML-based anomaly detection

Replace static tolerance with seasonal anomaly detector (Prophet).
Catches drifts that static thresholds miss.

### G.3 Cross-version baseline interpolation

Today : compare against the same chart version.  Phase-3 : detect
intra-version drift (PR-to-PR within a chart minor) via interpolated
baselines.

### G.4 Distributed bench (parallel cluster runs)

Run scenarios across multiple cloud regions simultaneously ;
aggregate via federated parquet.

### G.5 Public bench API

`https://api.efe.example.com/bench/v1/scenarios/S01/baseline` —
machine-readable API for third-party tooling.

### G.6 Reproducibility certificate

Cryptographically-signed baseline manifest + cluster topology hash ;
prove a bench result wasn't tampered with.

### G.7 Bench-as-a-service for partners

Hosted runner partners can submit a chart + run bench → get a public
"verified by EFE" badge.

### G.8 Cost-attribution metrics

Add `cost_per_iterate_usd` to the comparison metric set ; requires
cost-attribution exporter (out of scope).

### G.9 Customer-facing benchmark page

Marketing-grade landing : graphs, screenshots, methodology FAQ.

### G.10 Soak-bench Prometheus retention

S09 currently snapshots Prom at end ; Phase-3 captures rolling chunks
to track drift evolution within the 2-hour window.

---

## End of architecture document

| Field | Value |
|---|---|
| Section count | 36 |
| Architectural decisions | 45 |
| Acceptance criteria | 35 |
| Threat IDs | 10 (T86 – T95) |
| Scenarios | 10 |
| Chaos faults | 6 |
| CI workflows | 4 |
| Comparison metrics (closed) | 6 |
| Verdict levels | 4 |
| Tolerance tiers committed | 6 metrics × 10 scenarios = 60 cells |
| Use cases | 8 |
| Total LOC (this doc) | ~ 2 850 |

— EFE platform team, performance + SRE area, May 2026
