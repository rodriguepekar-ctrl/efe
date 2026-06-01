# EFE-A1 Phase 2 — PR #22 : `efe-slo-rollup` — 4 SLOs + multi-tier rollup compute (consumes PR #20 loadgen)

| Field | Value |
|---|---|
| **PR** | #22 (Sprint A1 Phase 2 / P2.7 W13 → P2.8 W15) |
| **Branch** | `efe-a1/p2-pr22-slo-rollup` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team — SRE area (lead) + compliance area (co-lead) |
| **Reviewers** | 3× SRE + 2× compliance + 1× security + 1× perf-eng + 1× release-eng + 1× legal (quarterly evidence chain) |
| **Depends on** | PR #14 (Helm production-grade), PR #16 (mTLS), PR #17 (Prometheus metrics — `efe_l2_request_total` + `efe_l2_request_duration_seconds_bucket` + `efe_swarm_recovery_seconds` + `efe_replication_lag`), PR #18 (OTel propagation — span attributes used as SLI labels), PR #19 (Grafana dashboards + recording rules + 4 SLO **definitions** in STABILITY_CONTRACT v1.1 ; this PR **freezes** their thresholds + windows in v1.4), PR #20 (`efe-meta-swarm-loadgen` — synthetic-traffic source for non-prod SLI cells), PR #21 (`efe-bench-harness` — scenario YAMLs whose `tolerance:` blocks now derive from the v1.4 SLO thresholds) |
| **Blocks** | PR #23 (Phase-3 SLO-driven autoscaler — needs the rollup pipeline as the only legitimate signal source), GA v1.0 (no public release without 90 days of green quarterly evidence), customer SLA contracts (legal cannot sell uptime without a published rollup + audit chain) |
| **L2 gate** | G19 (production-grade SLO surface : 4 frozen SLO IDs + 8-tier window taxonomy + 4-stage rollup pipeline + signed quarterly evidence + GitHub-Pages-published `https://efe.github.io/slos/` + STABILITY_CONTRACT v1.4 commitment + 2-window Google-SRE burn-rate alerts) |
| **Threat IDs in scope** | T96 – T105 (10 new : (T96) SLO evidence tampering, (T97) rollup PII leak via raw-metric label, (T98) Parquet result DoS via window-explosion, (T99) burn-rate alert oscillation (flap-storm), (T100) public-page defacement via SLO badge, (T101) compliance signature replay, (T102) Prom recording-rule poisoning, (T103) rollup CronJob priority inversion, (T104) cross-tenant SLO bleed via label confusion, (T105) quarterly PDF embedded-JS execution) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS3 + § 15 (Public SLOs) + § 21 (Compliance evidence chain) |
| **CVSS bandwidth** | All ≤ SEV-2 (no PII in SLO measurements — see AD18 + T97 mitigation ; evidence signing key rotation is a SEV-3 concern handled separately) |
| **Crate version** | New crate `efe-slo-rollup` v0.1.0 at `slos/rollup/`, binary `efe-slo-rollup` |
| **Existing infrastructure consumed** | `deploy/grafana/recording-rules/` (PR #19 — extended with `slo:*` namespace) ; `deploy/alertmanager/alerts/slo-burn-rate.yaml` (PR #19 — replaced with v1.4-locked variant) ; `tools/loadgen` (PR #20 — consumed as a feature-gated library `--features=slo-source`) ; `bench/harness` (PR #21 — its baseline JSONs feed the non-prod SLI cells) ; `crates/efe-meta-swarm-server` (PR #17 — its metric registry is the SLI numerator/denominator source of truth) |
| **STABILITY_CONTRACT v** | 1.3 → **1.4** (new tier : *SLO IDs* + *SLO thresholds* + *window taxonomy* + *rollup file schemas* + *evidence chain format*) |
| **Total LOC budget** | Rust rollup crate ~5 200 ; SLI compute kernels (4× SLOs) ~1 100 ; window aggregation kernel ~750 ; Parquet/JSON storage layer ~880 ; signed-evidence chain ~620 ; mdbook public site generator ~520 ; 4 SVG badge generators ~280 ; Helm chart (3 CronJobs) ~480 ; recording-rule YAML diff (additive over PR #19) ~360 ; alertmanager YAML (2-window burn) ~220 ; CI workflows (4 new) ~640 ; tests ~2 100 ; runbook + public methodology page ~1 280 ; STABILITY_CONTRACT v1.4 addendum ~480 → **~14 910 LOC** |
| **CI cost delta** | + 31 s for `cargo test -p efe-slo-rollup` (in-memory SLI computation — no I/O) ; + 9 s for `cargo test --features golden-evidence` (signed-evidence golden) ; hourly `slo-compute` cron job (~ 18 s wall-clock on shared runner — separate workflow, not blocking PRs) ; monthly `slo-monthly` cron (~ 4 min) ; quarterly `slo-quarterly` cron (~ 11 min, generates signed PDF) ; total committed PR-blocking CI delta **+ 40 s** |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals + scope
   2. Pre-flight inventory — what PR #19 / PR #20 / PR #21 already deliver vs. what's missing
   3. Branch + commit plan (14 commits across 3 sprints)
   4. Architectural decisions (AD1 – AD52)
   5. Operator personas + 9 use cases
   6. The 4 frozen SLOs (taxonomy + threshold + window + rationale)
   7. SLI math — per-SLO numerator / denominator / boundary conditions
   8. Window taxonomy (8 tiers : 5m → 90d) + aggregation rules
   9. Rollup compute pipeline (T0 → T3, 4 stages)
  10. Storage layer (Prom + Parquet daily + JSON monthly + signed PDF quarterly)
  11. Burn-rate alert design (Google SRE 2-window, 14.4× / 6×)
  12. Public SLO page (`https://efe.github.io/slos/`) — site map + JSON API
  13. Quarterly compliance evidence (chain of custody + signing)
  14. Rust crate `efe-slo-rollup` — module map + public API
  15. CLI design — 7 subcommands (compute, rollup, publish, evidence, list, validate, version)
  16. Helm chart for in-cluster rollup compute (3 CronJobs)
  17. CI workflows (4 new : slo-compute / slo-monthly / slo-quarterly / slo-publish)
  18. PR #20 loadgen consumer contract (`--features=slo-source`)
  19. PR #21 bench harness coupling (baselines as SLI cells)
  20. Multi-tenancy + multi-chart-version (label discipline)
  21. STABILITY_CONTRACT v1.4 additions
  22. Security model + Kubernetes RBAC + evidence-chain signing
  23. Threat-model coverage (T96 – T105)
  24. Acceptance criteria (AC1 – AC38)
  25. Tests strategy (unit kernel + integration chain + golden evidence + property-based windows)
  26. Risks + mitigations
  27. Rollback strategy
  28. Sequence diagrams (hourly rollup, monthly rollup, quarterly evidence)
  29. Reviewer checklist
  30. Appendix A — CLI flag reference
  31. Appendix B — SLI math derivation (per-SLO)
  32. Appendix C — recording-rule YAML diff vs PR #19
  33. Appendix D — burn-rate alert YAML (PrometheusRule)
  34. Appendix E — daily Parquet schema v1
  35. Appendix F — monthly JSON schema v1
  36. Appendix G — quarterly PDF + JSON evidence template
  37. Appendix H — error-budget formula + worked example
  38. Appendix I — cross-version SLO compatibility matrix
  39. Appendix J — Phase-3 follow-ups (G.1 – G.12)

---

## 0. TL;DR

PR #17 wired Prometheus metric exposition.  PR #19 stood up Grafana
dashboards + alertmanager + **defined** 4 SLOs in STABILITY_CONTRACT
v1.1 — but only at the *threshold-string* level (e.g. *"availability
≥ 99.5% over 30d"*).  PR #20 produced the synthetic-traffic source.
PR #21 produced reproducible bench scenarios.  What's missing is the
**numerical engine** that :

1. **Computes** each SLO's SLI numerator / denominator with frozen,
   auditable, mathematically-justified formulas — **NOT a thresholds
   string in a Markdown file**.
2. **Rolls up** raw observations through an 8-tier window taxonomy
   (5m → 30m → 1h → 6h → 24h → 7d → 30d → 90d) with explicit
   boundary semantics (clock-aligned, no double-counting, no gap
   inflation).
3. **Publishes** continuously to `https://efe.github.io/slos/` with
   4 SVG badges (one per SLO), trend graphs, and a JSON API.
4. **Generates** signed quarterly compliance evidence (PDF + JSON)
   that legal can attach to enterprise contracts.
5. **Alerts** on burn-rate using the Google-SRE 2-window method
   (14.4× fast burn ⇒ exhausts monthly budget in 2 days ; 6× slow
   burn ⇒ exhausts in 5 days).
6. **Freezes** the SLO IDs, thresholds, window taxonomy, storage
   schemas, and evidence-chain format in STABILITY_CONTRACT v1.4 for
   ≥ 6 chart-minor versions.

PR #22 ships :

- **1 Rust crate** `efe-slo-rollup` (binary + library) with 4 SLI
  compute kernels, 1 window aggregation kernel, Parquet + JSON
  storage, 4 SVG badge generators, and a signed-evidence builder.
- **3 CronJobs** (hourly rollup, monthly rollup, quarterly evidence)
  packaged as a Helm chart.
- **4 CI workflows** (`slo-compute.yml`, `slo-monthly.yml`,
  `slo-quarterly.yml`, `slo-publish.yml`).
- **2-window burn-rate alerts** (PrometheusRule) replacing the
  threshold-string alerts shipped by PR #19.
- **GitHub-Pages site** generated from `docs/slos/site/`.
- **STABILITY_CONTRACT v1.4 § 22** (this PR) committing the SLO IDs,
  threshold values, windows, and file schemas as frozen.

Everything is mathematically defined, deterministic given the same
metric scrape, and auditable.

---

## 1. Goal + non-goals + scope

### 1.1 Goal

Deliver the **complete, frozen, public, auditable SLO surface** for
the EFE Phase-2 meta-swarm gRPC L2 plane — covering :

  - 4 SLOs with closed-form SLI definitions (`§ 6`, `§ 7`),
  - 8-window rollup pipeline (`§ 8`, `§ 9`),
  - Tiered storage (Prom + Parquet + JSON + signed PDF — `§ 10`),
  - Burn-rate alerts (`§ 11`),
  - Public dashboard at `https://efe.github.io/slos/` (`§ 12`),
  - Quarterly compliance evidence chain (`§ 13`),
  - STABILITY_CONTRACT v1.4 commitments (`§ 21`).

### 1.2 Non-goals (defer to Phase-3 — `§ 39 / Appendix J`)

  - **Adaptive thresholds** (ML-driven) — G.1.
  - **Cross-region SLO aggregation** with weighted blends — G.2.
  - **Per-customer SLOs** (multi-tenant billing) — G.3.
  - **Multi-cluster federation** of recording rules — G.4.
  - **Continuous deployment gating** on SLO trend — G.5.
  - **SLO-driven autoscaler** (consumes this rollup but ships in PR
    #23) — G.6.
  - **Legal contract auto-binding** (insurance, refunds) — G.7.
  - **PDF embedded-signature with HSM** (Phase-3 hardens beyond
    cosign) — G.8.
  - **Cross-chart-minor SLO history compaction** (we keep all
    daily files for Phase-2 ; Phase-3 introduces compaction policy) — G.9.
  - **Real-time WebSocket SLO stream** (current is HTTP-pull on JSON
    API) — G.10.
  - **SLO regression detection** (this PR detects the verdict ; trend
    regression is G.11).
  - **Customer-facing landing page** with branded design — G.12.

### 1.3 Explicit scope boundary vs PR #19

| Concern | PR #19 | PR #22 |
|---|---|---|
| **SLO definition (threshold-string)** | ✅ Defines 4 SLOs in STABILITY_CONTRACT v1.1 § 7 (markdown sentences) | ✅ Re-states them as frozen numerical constants (`SLO_THRESHOLDS` table — see § 6) |
| **Grafana dashboards** | ✅ Ships the visual layer | ⚠ Adds 1 new dashboard `slo-overview` consuming the recording rules ; PR #19's panels remain authoritative |
| **Prometheus recording rules** | ✅ Ships rules for *every* metric (broad) | ✅ Adds `slo:*` namespace rules (narrow — only the 8-tier windows × 4 SLOs = 32 series) |
| **Alertmanager routing** | ✅ Threshold-string alerts (e.g. "p95 > 100ms for 10m") | 🟢 **Replaces** with 2-window burn-rate alerts (sloth-style) |
| **Compliance evidence** | ❌ Absent | ✅ Quarterly signed PDF + JSON |
| **Public SLO page** | ❌ Absent (Grafana is internal) | ✅ `https://efe.github.io/slos/` (mdbook) |
| **STABILITY_CONTRACT commitment** | v1.1 (SLO defined) | v1.4 (SLO frozen — IDs + thresholds + windows + schemas) |

### 1.4 Explicit scope boundary vs PR #21

PR #21 ships `tolerance:` blocks in scenario YAMLs ; those tolerances
ARE NOT SLOs.  They are per-scenario regression thresholds for the
bench harness.  PR #22 re-uses the bench harness's `summary.json`
output as the **non-production SLI cell source** (synthetic-traffic
SLI), but the SLO contract itself is independent : a passing bench
run is necessary, not sufficient, for the SLO to be met.

| Concern | PR #21 tolerance | PR #22 SLO |
|---|---|---|
| Scope | One scenario, one run | Rolling 30 days, all prod traffic + synthetic |
| Granularity | Per-metric, per-scenario | Per-SLO, all-prod-aggregated |
| Failure mode | CI blocking | Burn-rate alert + compliance evidence |
| Stability | STABILITY_CONTRACT v1.3 § 21 | STABILITY_CONTRACT v1.4 § 22 |
| Window | Single run wall-clock | 8 tiered windows |

---

## 2. Pre-flight inventory

### 2.1 What PR #19 already ships (CONSUMED, NOT REPLACED)

```
deploy/grafana/dashboards/
   ├── l2-overview.json           # consumed for SLO dashboard inheritance
   ├── meta-swarm-overview.json   # idem
   └── slo-overview.json          # NEW — added by PR #22 (consumes recording rules)
deploy/grafana/recording-rules/
   ├── l2-rates.yaml              # consumed — produces efe_l2_request_total:rate5m
   ├── l2-quantiles.yaml          # consumed — produces efe_l2_request_duration_seconds:p95
   ├── swarm-recovery.yaml        # consumed — efe_swarm_recovery_seconds:p99
   ├── replication-lag.yaml       # consumed — efe_replication_lag:p99
   └── slo-windows.yaml           # NEW — PR #22 adds the 32 series (8 windows × 4 SLOs)
deploy/alertmanager/alerts/
   ├── infra.yaml                 # untouched
   ├── meta-swarm.yaml             # untouched
   └── slo-burn-rate.yaml         # REPLACED — PR #22 swaps threshold-string alerts for 2-window burn-rate
docs/STABILITY_CONTRACT.md
   § 7 (v1.1 — SLOs threshold-string)   # consumed
   § 22 (v1.4 — SLO IDs frozen)         # NEW — PR #22 adds
```

### 2.2 What PR #20 already ships (CONSUMED via feature gate)

- `tools/loadgen/src/lib.rs` — `pub fn run(config) -> RunSummary`
- `tools/loadgen/src/profile/v1.rs` — profile schema
- 6 profile YAMLs (`smoke`, `canary`, `chaos-injection`, `nominal`,
  `breakdown`, `closed-loop`)

PR #22 adds an **optional Cargo feature** `slo-source` to
`tools/loadgen` that emits an `SloSourceEvent` enum to a callback,
letting the rollup binary consume synthetic-traffic SLI cells in-process.

```toml
# tools/loadgen/Cargo.toml (DIFF added by PR #22)
[features]
default      = []
slo-source   = []  # AD2 — opt-in SLI emission for `efe-slo-rollup`
```

### 2.3 What PR #21 already ships (CONSUMED)

- `bench/results/{run_id}/{scenario_id}/summary.json` — canonical
  per-run metrics (schema v1.0).
- `bench_baselines/v{chart_version}/L2/{scenario_id}.json` — baseline
  store (used as synthetic-traffic SLI snapshot in non-production
  cells).

PR #22 reads these as **read-only inputs**, never writes.

### 2.4 What this PR delivers (NEW)

```
slos/
├── rollup/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs                  # CLI entry (compute, rollup, publish, evidence, list, validate, version)
│   │   ├── lib.rs                   # public re-exports + module map
│   │   ├── error.rs                 # BenchError + 7 ExitCodes
│   │   ├── cli.rs                   # clap structs + arg validation
│   │   ├── sli/
│   │   │   ├── mod.rs               # SliKernel trait + 4-SLO dispatch
│   │   │   ├── availability.rs      # SLO-1 numerator/denominator
│   │   │   ├── latency_p95.rs       # SLO-2 (windowed compliance)
│   │   │   ├── recovery_time.rs     # SLO-3 (chaos cell)
│   │   │   └── replication_lag.rs   # SLO-4
│   │   ├── window/
│   │   │   ├── mod.rs               # WindowSpec enum + alignment
│   │   │   ├── align.rs             # clock-aligned bucket math
│   │   │   ├── aggregate.rs         # 8-tier roll-up
│   │   │   └── boundary.rs          # gap / restart / re-clock handling
│   │   ├── rollup/
│   │   │   ├── mod.rs               # 4-stage orchestrator
│   │   │   ├── stage_t0.rs          # T0 — Prom recording rules
│   │   │   ├── stage_t1.rs          # T1 — hourly → daily Parquet
│   │   │   ├── stage_t2.rs          # T2 — daily → monthly JSON
│   │   │   └── stage_t3.rs          # T3 — monthly → quarterly evidence
│   │   ├── storage/
│   │   │   ├── mod.rs
│   │   │   ├── parquet.rs           # write_daily / read_daily
│   │   │   ├── json.rs              # write_monthly / read_monthly
│   │   │   ├── pdf.rs               # write_quarterly_pdf (printpdf)
│   │   │   ├── object_store.rs      # S3 / GCS / file:// abstraction
│   │   │   └── retention.rs         # 90d daily / 7y monthly / forever quarterly
│   │   ├── source/
│   │   │   ├── mod.rs               # SourceProvider trait
│   │   │   ├── prom_query.rs        # PromQL fetcher (HTTP)
│   │   │   ├── loadgen_lib.rs       # in-process via PR #20 feature `slo-source`
│   │   │   ├── bench_baseline.rs    # PR #21 summary.json reader
│   │   │   └── replay_parquet.rs    # rewind from a stored daily file (debugging)
│   │   ├── burn_rate/
│   │   │   ├── mod.rs               # Google-SRE 2-window math
│   │   │   ├── prom_rule.rs         # PrometheusRule emitter
│   │   │   └── verdict.rs           # FastBurn / SlowBurn / Healthy
│   │   ├── badge/
│   │   │   ├── mod.rs               # 4 SVG generators
│   │   │   ├── availability.rs      # SLO-1 dial
│   │   │   ├── latency_p95.rs       # SLO-2 histogram strip
│   │   │   ├── recovery.rs          # SLO-3 stopwatch
│   │   │   └── replication.rs       # SLO-4 bar
│   │   ├── evidence/
│   │   │   ├── mod.rs               # ChainBuilder
│   │   │   ├── pdf_template.rs      # printpdf layout
│   │   │   ├── json_summary.rs
│   │   │   ├── sign.rs              # cosign / minisign wrapper
│   │   │   └── verify.rs            # offline verifier (also bundled in CLI)
│   │   ├── publish/
│   │   │   ├── mod.rs               # mdbook generator
│   │   │   ├── site_map.rs          # 4 SLO sub-pages + trend + archive
│   │   │   ├── trend.rs             # sparkline SVG
│   │   │   └── json_api.rs          # /current.json + /history/{yyyy-mm}.json
│   │   └── build.rs                 # git_sha + version metadata
│   ├── tests/
│   │   ├── compute_golden.rs        # 28 golden SLI inputs → outputs
│   │   ├── rollup_chain.rs          # T0→T1→T2→T3 end-to-end
│   │   ├── window_property.rs       # proptest — bucket alignment
│   │   ├── burn_rate_invariant.rs   # 2-window math
│   │   ├── badge_svg_golden.rs      # 4 SVGs byte-snapshot
│   │   ├── evidence_signature.rs    # PDF cosign + verify roundtrip
│   │   └── data/                    # 12 fixture files (Prom JSON, parquet)
│   └── helm/
│       ├── Chart.yaml
│       ├── values.yaml
│       └── templates/
│           ├── cronjob-hourly.yaml      # T1 every 1 h
│           ├── cronjob-monthly.yaml     # T2 1st of month @ 03:00 UTC
│           ├── cronjob-quarterly.yaml   # T3 1st of Apr/Jul/Oct/Jan @ 04:00 UTC
│           ├── pvc.yaml                 # for offline replay
│           ├── secret-signing-key.yaml  # SealedSecret-ready
│           ├── rbac.yaml                # namespace-scoped
│           ├── serviceaccount.yaml
│           ├── _helpers.tpl
│           └── NOTES.txt
├── recording-rules/                 # NEW Prom rules (additive to PR #19)
│   └── slo-windows.yaml
├── alerts/                          # REPLACES PR #19's slo-burn-rate.yaml
│   └── slo-burn-rate-2window.yaml
├── schemas/
│   ├── daily-parquet-v1.json        # JSON-Schema for the Parquet column set
│   ├── monthly-json-v1.json
│   └── quarterly-evidence-v1.json
├── site/                            # mdbook source
│   ├── book.toml
│   ├── theme/
│   └── src/
│       ├── SUMMARY.md
│       ├── overview.md
│       ├── slo-1-availability.md
│       ├── slo-2-latency-p95.md
│       ├── slo-3-recovery-time.md
│       ├── slo-4-replication-lag.md
│       ├── methodology.md
│       ├── error-budget.md
│       ├── quarterly-archive.md
│       └── stability-contract-v1.4.md
└── README.md                        # public-facing methodology landing
docs/
├── EFE_A1_PHASE2_PR22_ARCHITECTURE.md   # THIS DOC
└── STABILITY_CONTRACT.md
    § 22 (v1.4 — SLO IDs frozen)          # NEW
.github/workflows/
├── slo-compute.yml      # cron 0 * * * * (every hour, T1 rollup)
├── slo-monthly.yml      # cron 0 3 1 * * (1st of month, T2 rollup)
├── slo-quarterly.yml    # cron 0 4 1 1,4,7,10 * (T3 evidence + PDF)
└── slo-publish.yml      # on push:main + after the 3 above complete
deploy/cosign/
└── slo-evidence.pub     # public key — committed alongside the artefact (NEVER the private key)
```

---

## 3. Branch + commit plan

| # | Commit | Scope | Approx LOC | Notes |
|---|---|---|---:|---|
| 1 | `chore(workspace): register slos/rollup crate` | Cargo.toml + workspace deps | 60 | mechanical |
| 2 | `feat(slo): SliKernel trait + 4 SLO dispatch shells` | `src/sli/{mod,availability,latency_p95,recovery_time,replication_lag}.rs` | 720 | each module compiles + has 3 unit tests with golden inputs |
| 3 | `feat(slo): window taxonomy + alignment kernel + boundary semantics` | `src/window/*.rs` | 690 | proptest passes for window-edge cases |
| 4 | `feat(slo): source providers (prom / loadgen / bench / parquet)` | `src/source/*.rs` | 880 | 4 traits + 4 impls + mock |
| 5 | `feat(slo): rollup stages T0–T3 orchestrator` | `src/rollup/*.rs` | 940 | T1→T2→T3 chain with idempotency check |
| 6 | `feat(slo): Parquet daily + JSON monthly storage` | `src/storage/{parquet,json,object_store,retention}.rs` | 860 | apache-arrow + serde_json |
| 7 | `feat(slo): burn-rate alert (Google SRE 2-window)` | `src/burn_rate/*.rs` + `slos/alerts/slo-burn-rate-2window.yaml` | 480 | replaces PR #19's slo-burn-rate.yaml |
| 8 | `feat(slo): 4 SVG badge generators` | `src/badge/*.rs` | 380 | byte-stable SVG |
| 9 | `feat(slo): signed PDF quarterly evidence` | `src/evidence/*.rs` + `src/storage/pdf.rs` + cosign wrapper | 940 | requires deploy/cosign/slo-evidence.pub |
| 10 | `feat(slo): mdbook public site + JSON API` | `src/publish/*.rs` + `slos/site/**` | 1 020 | static HTML + /current.json + /history/{yyyy-mm}.json |
| 11 | `feat(slo): CLI (7 subcommands)` | `src/{main,cli}.rs` | 480 | clap derive |
| 12 | `feat(slo): Helm chart (3 CronJobs)` | `slos/rollup/helm/**` | 520 | hourly + monthly + quarterly |
| 13 | `ci(slo): 4 workflows (compute / monthly / quarterly / publish)` | `.github/workflows/slo-*.yml` | 640 | uses cosign keyless OIDC |
| 14 | `docs(slo): STABILITY_CONTRACT v1.4 + public methodology + this arch doc + README` | `docs/STABILITY_CONTRACT.md` + `slos/README.md` + `slos/site/src/*` | 4 250 | locks SLO IDs + thresholds + windows + file schemas |

**Total LOC** ≈ 14 880 (within budget of 14 910 from PR header).

---

## 4. Architectural decisions (AD1 – AD52)

### 4.1 Data sources + scope

- **AD1 — Closed 4-SLO taxonomy.** Exactly 4 SLOs, IDs frozen.
  Adding a 5th requires a major version of STABILITY_CONTRACT.
- **AD2 — In-process PR #20 consumption.** Loadgen as a library
  behind a Cargo feature, never as a subprocess.  Rationale : 30×
  cheaper than spawning ; deterministic seed propagation.
- **AD3 — Read-only consumption of PR #21 baselines.** Rollup never
  mutates `bench_baselines/` ; only reads `summary.json`.
- **AD4 — Prometheus is the single source of truth for prod cells.**
  Synthetic cells (loadgen + bench) feed the SLI sidecar series, NOT
  the prod cells.  Mixing is forbidden (AD18).
- **AD5 — No raw-metric labels propagated into SLO storage.** Only
  the aggregated SLI value goes to Parquet/JSON (T97 mitigation).
- **AD6 — Determinism guarantee.** Given identical input scrape +
  identical wall-clock window boundaries, two rollup runs MUST emit
  byte-identical Parquet files (after the deterministic-write flag
  is set).  Tested via a roundtrip property test.
- **AD7 — Idempotent rollup.** T1 must be safe to re-run for any
  historical hour without corrupting daily aggregates ; a sentinel
  `__rollup_run_id` column tracks the writer.
- **AD8 — Wall-clock UTC.** All windows are aligned to UTC midnight ;
  no local-time fallback.  Saves us from DST corner cases.
- **AD9 — No floating-point comparisons in burn-rate logic.** All
  thresholds expressed as ratios of integer counts where possible ;
  comparison uses an epsilon = 1e-12 (documented).
- **AD10 — Schema-versioned Parquet + JSON + PDF.** Each artefact
  carries a `schema_version` field at column / key / metadata level ;
  PR #22 fixes v1 and Phase-3 introduces a v2 migrator.

### 4.2 SLO taxonomy + SLI math

- **AD11 — SLO IDs use SNAKE_CASE_UPPER + `slo:` namespace** for the
  Prom series, e.g. `slo:L2_AVAILABILITY_5m`.  Frozen.
- **AD12 — Numerator/denominator are integers in the source layer.**
  Float division happens only at the SLI compute step ; we never
  store a pre-divided ratio in T0/T1.
- **AD13 — Latency SLO is window-compliance, not raw percentile.**
  SLO-2 measures *the fraction of 5-min windows where p95 ≤ 50ms*,
  not *the p95 over 30d*.  This is the Google-SRE canonical pattern
  (Google SRE Workbook, ch.4).
- **AD14 — Boundary semantics for partial windows.** A 5-min window
  is counted only if scrape coverage ≥ 80% of the bucket ; else
  marked `coverage_low` and excluded from the denominator (AC8).
- **AD15 — Recovery time SLO is event-driven, not time-windowed.**
  SLO-3 measures *fraction of chaos-induced recoveries within
  60 s* ; a recovery is a discrete event triggered by a chaos
  fault apply → first all-participants-ready transition.
- **AD16 — Replication-lag SLI is per-participant max-of-min.**
  For every 1-min window, take the *minimum* across participants
  (least-laggy) — this is the LOG-CATCHUP-METRIC defined by Raft.
  Document the choice (the alternative — max-across-participants —
  would be alarmist).
- **AD17 — No PII derivation from SLI labels.** SLI series carry
  only `slo_id`, `window`, `chart_version`, `region` (cardinality
  ≤ 10).  No request_id, no user_id, no IP.
- **AD18 — Synthetic + prod cells are separated by `source=` label.**
  `source=prod`, `source=synthetic_loadgen`, `source=bench`.  Burn-
  rate alerts fire only on `source=prod`.
- **AD19 — Closed-form numerator formula in code, not config.** Each
  SLI's numerator/denominator math lives in Rust (`src/sli/{slo}.rs`),
  not in YAML — this prevents drift.  STABILITY_CONTRACT v1.4 locks
  the function names + signatures.
- **AD20 — Daylight-saving-time-immune.** All time arithmetic via
  `chrono::Utc::now()` ; clippy lints against any `chrono::Local`
  usage in this crate.

### 4.3 Rollup pipeline

- **AD21 — 4-stage rollup, fixed.** T0 (Prom recording rules) →
  T1 (hourly daily Parquet) → T2 (monthly JSON) → T3 (quarterly
  signed PDF + JSON).  Adding stages requires a major STABILITY
  bump.
- **AD22 — T0 is computed in Prom, NOT in the rollup binary.**
  Recording rules are cheaper + already battle-tested ; the binary
  only ingests their output.
- **AD23 — T1 → T2 → T3 are batch jobs (CronJob).**  Streaming is
  Phase-3 (G.10).  Rationale : evidence-chain integrity benefits
  from atomic, signed batch outputs.
- **AD24 — Each tier reads exactly N+1 of the prior tier.**  T1
  reads 24 hourly Prom queries → 1 daily Parquet.  T2 reads ~30
  daily Parquet → 1 monthly JSON.  T3 reads 3 monthly JSON → 1
  quarterly PDF.  Predictable I/O cost.
- **AD25 — Tier outputs are addressable by `(chart_version, period)`.**
  Daily : `slos/v0.6.0/2026-05/daily/2026-05-15.parquet`.  Monthly :
  `slos/v0.6.0/monthly/2026-05.json`.  Quarterly : `slos/v0.6.0/
  quarterly/Q2-2026.{json,pdf}`.  Frozen.
- **AD26 — Object-store-agnostic.** File schemes : `file://`,
  `s3://`, `gs://`.  Default writes to a PVC ; CI uploads to S3 with
  AWS-OIDC (no static creds).
- **AD27 — Retention policy.** 90 days for daily Parquet, 7 years
  for monthly JSON, **forever** for quarterly evidence (regulatory).
- **AD28 — Backfill is explicit, not implicit.** A T1 run for a
  past hour is allowed only via `efe-slo-rollup compute --backfill
  --hour 2026-05-15T03:00:00Z` with the `--release-backfill-confirmed`
  guard (analog of PR #21 baseline regen).
- **AD29 — Concurrency safety.** Each rollup job writes to a
  temp file then renames atomically ; the rename is the commit point.
  Concurrent jobs for the same period race-and-last-wins (acceptable
  : both produce identical outputs by AD6).
- **AD30 — No mutation of historical files.** Daily/monthly/quarterly
  files are write-once.  Re-running with new code requires bumping
  the schema version (Phase-3 G.9).

### 4.4 Storage + retention

- **AD31 — Parquet for daily, JSON for monthly, PDF + JSON for
  quarterly.**  Daily : compact analytic columnar (we run ad-hoc
  DuckDB queries).  Monthly : human-readable + machine-readable.
  Quarterly : PDF for legal + JSON for machines.
- **AD32 — All filenames + URLs are lowercase + kebab-case.**
  `2026-05-15.parquet`, `Q2-2026.pdf`.
- **AD33 — Daily Parquet is row-group-aligned.** One row group per
  hour ; 24 row groups per file.  Enables predicate pushdown.
- **AD34 — Monthly JSON is canonical-form.**  Keys sorted ;
  pretty-printed with 2-space indent.  Tested via golden.
- **AD35 — Quarterly PDF carries an embedded JSON sidecar +
  cosign signature.**  PDF metadata contains
  `/EFE_EVIDENCE_JSON_SHA256` + `/EFE_EVIDENCE_SIG_PATH`.  Verifier
  re-computes the SHA and verifies the signature.
- **AD36 — PDF is NOT a render of the JSON ; both are independently
  authoritative.**  JSON ≡ rendered form is checked via the verifier.
- **AD37 — Signing key is per-environment + rotated annually.**
  Public key committed at `deploy/cosign/slo-evidence.pub`.  Private
  key lives in the org's KMS, never on disk in CI.  CI uses
  cosign keyless OIDC against GitHub.

### 4.5 Burn-rate alerts

- **AD38 — Google-SRE 2-window method.** Fast burn = 14.4× error
  rate over 1h ⇒ exhausts 2% of monthly budget ⇒ critical.  Slow
  burn = 6× error rate over 6h ⇒ exhausts 10% ⇒ warning.
- **AD39 — One alert pair per SLO.** 4 SLOs × 2 windows = 8 alerts.
  Both must fire to alert ⇒ no flap on isolated spike (T99 mitigation).
- **AD40 — Burn-rate alerts replace PR #19's threshold alerts.**
  PR #19's `slo-burn-rate.yaml` is removed and replaced with the
  PR #22 variant.  PR #19's `infra.yaml` + `meta-swarm.yaml` are
  untouched.
- **AD41 — Alert routing inherits PR #19's alertmanager config.**
  We emit only the `PrometheusRule` ; route to existing receivers.

### 4.6 Public surface + compliance

- **AD42 — Public page is mdbook-generated static HTML.**  No
  client-side JS for SLO data fetch ; the page is self-contained.
  JSON API is a sibling `/current.json` ; embedded sparklines are
  pre-rendered SVG.
- **AD43 — 4 SVG badges, one per SLO.**  Badge filenames frozen :
  `slo-1-availability.svg`, `slo-2-latency-p95.svg`,
  `slo-3-recovery-time.svg`, `slo-4-replication-lag.svg`.
- **AD44 — JSON API endpoints frozen.**  `/current.json`,
  `/history/{yyyy-mm}.json`, `/quarterly/{Q1..4}-{yyyy}.json`,
  `/methodology.json`, `/version.json`.
- **AD45 — Quarterly PDF has a frozen 7-section layout.**
  Cover / methodology / SLO-1 / SLO-2 / SLO-3 / SLO-4 / appendix
  (raw daily breakdown + signature page).
- **AD46 — Compliance chain is verifiable offline.**  `efe-slo-rollup
  evidence verify slos/v0.6.0/quarterly/Q2-2026.pdf` runs without
  network — needs only the public key.

### 4.7 Crate + CLI

- **AD47 — Library + binary.** Same pattern as PR #20 / PR #21.
- **AD48 — CLI subcommands : 7 (compute / rollup / publish / evidence
  / list / validate / version).** Frozen.
- **AD49 — Exit codes : 7 (Success=0, BurnRateExceeded=1,
  MathError=2, Config=3, Interrupted=4, StorageIO=5, EvidenceSign=6,
  Internal=7).**  Frozen.
- **AD50 — `--log-format json` is default in CronJob context** ;
  text in TTY.

### 4.8 Stability + ops

- **AD51 — STABILITY_CONTRACT v1.4 freezes 4 SLO IDs + 4 thresholds
  + 8-tier window taxonomy + 3 file schemas + signed-evidence chain
  format**, all for ≥ 6 chart-minor versions.
- **AD52 — `efe-slo-rollup` binary is the ONLY producer of files
  under `slos/v*`.**  Manual edits are forbidden (CI lint rejects
  any non-binary commit to that path).

---

## 5. Operator personas + 9 use cases

### 5.1 Personas

| Persona | Goal | Tool used | Required output |
|---|---|---|---|
| **SRE on-call** | Diagnose burn-rate alert | `efe-slo-rollup compute --slo L2_AVAILABILITY --window 1h` | which 5-min cell drove the burn |
| **Platform owner** | Monthly compliance report | nightly cron → `slo-monthly` workflow | `slos/v0.6.0/monthly/2026-05.json` |
| **Compliance officer** | Quarterly evidence | quarterly cron → signed PDF | `Q2-2026.pdf` signed by org KMS |
| **Auditor (external)** | Verify a signed PDF | `efe-slo-rollup evidence verify` | offline, exit 0 if valid |
| **Customer / sales** | Public SLO check | `https://efe.github.io/slos/` | 4 badges + trends |
| **Release engineer** | Pre-release SLO check | `efe-slo-rollup compute --since v0.6.0-rc1` | error budget remaining |
| **Perf engineer** | Trend regression | `efe-slo-rollup list --slo L2_LATENCY_P95 --range 90d` | 90-day series |
| **Security reviewer** | Verify no PII | `cargo check --features deny-pii` (lint pass) | T97 mitigation |
| **Legal counsel** | Attach to contract | quarterly PDF | signed, dated, hash-witnessed |

### 5.2 Use cases (9)

1. **UC-1 — Fast burn alert at 02:14 UTC** : SRE pages, runs
   `efe-slo-rollup compute --slo L2_LATENCY_P95 --window 1h
   --offset -1h`, sees that 5-min bucket [01:55, 02:00) had p95 =
   180 ms (vs 50ms threshold).  Correlates with deploy of
   v0.6.1-rc2.  Rolls back.
2. **UC-2 — Monthly compliance** : 1st of June at 03:00 UTC, the
   `slo-monthly` workflow generates `slos/v0.6.0/monthly/2026-05.json`,
   sees `availability=99.62%` (above 99.5% target), opens a PR
   adding the file (peter-evans).
3. **UC-3 — Quarterly evidence** : 1st of July at 04:00 UTC,
   `slo-quarterly` generates `Q2-2026.{json,pdf}`, signs the PDF
   with cosign keyless OIDC, uploads to S3, opens a PR.
4. **UC-4 — Auditor verifies a PDF** : reads
   `efe-slo-rollup evidence verify Q2-2026.pdf` → "VALID :
   issuer=github.com/efe-ai/efe, identity=slo-quarterly-workflow,
   subject_hash=...".
5. **UC-5 — Customer check at sales-call** : visits
   `https://efe.github.io/slos/` → sees 4 green badges + 30d trend.
6. **UC-6 — Release-eng pre-cuts a release** : runs
   `efe-slo-rollup compute --since 7d`, sees `budget_remaining_pct=82%`
   → green-lights the release.
7. **UC-7 — Perf-eng diagnoses a 90d regression** : runs
   `efe-slo-rollup list --slo L2_LATENCY_P95 --range 90d --format csv |
    duckdb …` → sees p95 trending +5% per week since v0.5.4.
8. **UC-8 — Security review** : confirms `cargo check --features
   deny-pii` rejects any code that names a forbidden label.
9. **UC-9 — Legal counsel attaches** Q2-2026.pdf to an enterprise
   MSA ; auditor at customer side re-verifies via the public key.

---

## 6. The 4 frozen SLOs

| ID | Name | SLI | Target | Window | Error budget / 30d |
|---|---|---|---|---|---|
| `L2_AVAILABILITY` | L2 request success rate | `sum(rate(efe_l2_request_total{status="success"}[5m])) / sum(rate(efe_l2_request_total{status!="client_error"}[5m]))` | **≥ 99.5%** | rolling 30d | 0.5% × 30d ≈ 3.6 h non-success time |
| `L2_LATENCY_P95` | L2 request latency p95 compliance | fraction of 5-min windows where `histogram_quantile(0.95, sum(rate(efe_l2_request_duration_seconds_bucket[5m])) by (le)) ≤ 0.050` | **≥ 99% compliant windows** | rolling 30d | 1% × 30d ≈ 86 non-compliant 5-min windows |
| `SWARM_RECOVERY_TIME` | Chaos-induced recovery time compliance | fraction of recovery events where `efe_swarm_recovery_seconds ≤ 60` | **≥ 99%** | rolling 30d | 1% of recovery events |
| `REPLICATION_LAG_P99` | Replication lag compliance | fraction of 1-min windows where `histogram_quantile(0.99, sum(rate(efe_replication_lag_bucket[1m])) by (le)) ≤ 100` | **≥ 99% compliant windows** | rolling 30d | 1% × 30d / 1min ≈ 432 non-compliant windows |

These values are frozen in STABILITY_CONTRACT v1.4 § 22 for ≥ 6
chart-minor versions.  Changing any value requires a major
version bump.

### 6.1 Why these 4 specifically (and not more)

Choosing 4 SLOs (not 6, not 12) is itself a design decision :

- Each SLO covers one **dimension** : availability, latency,
  resilience, correctness.
- 4 SLOs × 8 windows × 32 series ≈ 1 024 Prom series — well within
  the per-namespace budget (PR #19 cardinality cap = 50 k).
- Operator cognitive load : SRE on-call must hold the 4 thresholds
  in mind ; > 5 SLOs degrades response.  (Source : Google SRE
  Workbook ch.4 sec.4.4.)

### 6.2 Why these specific thresholds

| SLO | Threshold | Justification |
|---|---|---|
| `L2_AVAILABILITY` ≥ 99.5% | "3 nines and a half" | Industry standard for an internal control plane.  GA pricing requires this in contracts (legal §). |
| `L2_LATENCY_P95` ≤ 50 ms | < 1× human-perception threshold (100 ms) and matches PR #20 loadgen S02 canary target | Conservative — leaves headroom for chaos scenarios. |
| `SWARM_RECOVERY_TIME` ≤ 60 s | 4× cluster reformation theoretical lower bound (15 s under chaos-engineering benchmarks) | Operationally meaningful (an outage of 1 min ≈ "did anyone notice"). |
| `REPLICATION_LAG_P99` ≤ 100 entries | < 1× consensus snapshot interval (configured at 200 entries) | Avoids snapshot rebuild on lagging participants. |

---

## 7. SLI math — per-SLO numerator / denominator

This section defines each SLI in **closed mathematical form**.  The
Rust code in `src/sli/{slo}.rs` must implement these exactly.  Any
discrepancy between code and this section is a contract violation.

### 7.1 SLO-1 — `L2_AVAILABILITY`

**Numerator** (good events in window W) :

$$
N_1(W) = \sum_{s \in \text{scrapes}(W)} \text{efe\_l2\_request\_total}\{ \text{status} = \text{success} \}(s)
$$

**Denominator** (valid events in window W — **excludes client errors** ;
T96 mitigation : an attacker spamming 4xx must not inflate the
numerator side) :

$$
D_1(W) = \sum_{s \in \text{scrapes}(W)} \text{efe\_l2\_request\_total}\{ \text{status} \in \{\text{success}, \text{server\_error}, \text{transport\_error}\} \}(s)
$$

**SLI** :

$$
\text{SLI}_1(W) = \frac{N_1(W)}{D_1(W)}, \quad \text{undefined if } D_1(W) < 100 \text{ (low-traffic guard, AC8)}
$$

**Compliance** : window W is compliant if $\text{SLI}_1(W) \geq 0.995$.

**Rolling 30d SLI** :

$$
\text{SLI}_1^{30d} = \frac{\sum_{W \in \mathcal{W}_{30d}} N_1(W)}{\sum_{W \in \mathcal{W}_{30d}} D_1(W)}
$$

(NB : this is **not** the mean of per-window SLIs ; we sum the integer
counts and divide once at the end — this is the canonical Google-SRE
ratio-of-sums.)

### 7.2 SLO-2 — `L2_LATENCY_P95`

**Per-window p95** (using Prom histogram quantile interpolation) :

$$
P_{95}(W) = \text{histogram\_quantile}\left(0.95, \sum_{le} \text{rate}\left(\text{efe\_l2\_request\_duration\_seconds\_bucket}[5\text{m}]\right)\right)
$$

**Per-window compliance indicator** :

$$
C_2(W) = \begin{cases}
1 & \text{if } P_{95}(W) \leq 0.050 \text{ AND coverage}(W) \geq 0.80 \\
0 & \text{if } P_{95}(W) > 0.050 \text{ AND coverage}(W) \geq 0.80 \\
\text{NaN} & \text{if coverage}(W) < 0.80 \text{ (excluded from denominator)}
\end{cases}
$$

**SLI** :

$$
\text{SLI}_2(\mathcal{W}_{30d}) = \frac{\sum_{W : C_2(W) \neq \text{NaN}} C_2(W)}{|\{W : C_2(W) \neq \text{NaN}\}|}
$$

### 7.3 SLO-3 — `SWARM_RECOVERY_TIME`

**Event-driven** — uses the discrete event series
`efe_swarm_recovery_event_total{result="recovered", duration_seconds}`
emitted by the controller upon a participant-set transition.

**Per-event compliance** :

$$
C_3(e) = \begin{cases} 1 & \text{if duration}(e) \leq 60 \\ 0 & \text{otherwise} \end{cases}
$$

**SLI** :

$$
\text{SLI}_3(\mathcal{E}_{30d}) = \frac{\sum_{e \in \mathcal{E}_{30d}} C_3(e)}{|\mathcal{E}_{30d}|}
$$

**Low-volume guard** : if $|\mathcal{E}_{30d}| < 10$, SLI is reported
as "INSUFFICIENT_EVENTS" (window compliance neither passes nor
fails — see AD15 + AC11).

### 7.4 SLO-4 — `REPLICATION_LAG_P99`

**Per-window p99 (min-across-participants)** — see AD16 :

$$
L_{99}(W) = \min_{p \in \text{participants}} \text{histogram\_quantile}\left(0.99, \text{rate}(\text{efe\_replication\_lag\_bucket}\{participant=p\}[1\text{m}])\right)
$$

**Per-window compliance** :

$$
C_4(W) = \begin{cases} 1 & \text{if } L_{99}(W) \leq 100 \\ 0 & \text{otherwise} \end{cases}
$$

**SLI** :

$$
\text{SLI}_4(\mathcal{W}_{30d}) = \frac{\sum_W C_4(W)}{|\mathcal{W}_{30d}|}
$$

### 7.5 Edge cases (binding, tested via property tests)

| Case | Behaviour | Test ID |
|---|---|---|
| Scrape gap (Prom restart) > 5 min | Affected window marked `coverage_low`, excluded | AC8 |
| Negative rate (Prom counter reset) | Affected window marked `counter_reset`, excluded | AC9 |
| Single participant down (denominator=0 on that label) | Compute min across the OTHER participants | AC12 |
| < 10 recovery events in 30d | SLI-3 = INSUFFICIENT_EVENTS, no compliance verdict | AC11 |
| Daily file missing for an in-range day | T2 reports `missing_days=[…]` in monthly JSON ; verdict ⇒ INDETERMINATE | AC15 |
| Quarterly file references a missing monthly | T3 fails with ExitCode::StorageIO | AC16 |

---

## 8. Window taxonomy

| Tier | Window | Purpose | Stored where | Retention |
|---|---|---|---|---|
| T0a | 5 min | Burn-rate fast window | Prom (recording rule) | 15d (prom) |
| T0b | 30 min | Burn-rate intermediate | Prom (recording rule) | 15d (prom) |
| T0c | 1 h | Hourly point | Prom (recording rule) | 15d (prom) |
| T1 | 6 h | Operator dashboard | Prom + daily Parquet | 90d (parquet) |
| T1d | 24 h | Daily compliance | Prom + daily Parquet | 90d (parquet) |
| T2a | 7 d | Weekly trend | Computed from daily, monthly JSON | 7y (json) |
| T2b | 30 d | Monthly compliance | Computed from daily, monthly JSON | 7y (json) |
| T3 | 90 d | Quarterly compliance | Computed from monthly, quarterly PDF + JSON | forever |

### 8.1 Alignment rules

- All windows are aligned to UTC.
- 5 min windows : `[HH:MM:00, HH:MM+5:00)` where `MM % 5 == 0`.
- 30 min windows : `[HH:00:00, HH:30:00) or [HH:30:00, HH+1:00:00)`.
- 1 h : `[HH:00:00, HH+1:00:00)`.
- 6 h : aligned to `[00:00, 06:00), [06:00, 12:00), [12:00, 18:00),
  [18:00, 24:00)` UTC.
- 24 h : `[00:00, 24:00)` UTC.
- 7 d : ISO 8601 week (Mon 00:00 UTC → next Mon 00:00 UTC).
- 30 d : rolling — see AC14.
- 90 d : quarter (Q1 = Jan–Mar, Q2 = Apr–Jun, Q3 = Jul–Sep, Q4 = Oct–Dec).

### 8.2 Aggregation rules (T1 → T2)

For ratio-style SLIs (SLO-1, SLO-3) :

$$
\text{SLI}^{30d} = \frac{\sum N_i}{\sum D_i} \quad \text{(sum the raw counters)}
$$

For window-compliance SLIs (SLO-2, SLO-4) :

$$
\text{SLI}^{30d} = \frac{\sum C_i}{\sum_{C_i \neq \text{NaN}} 1} \quad \text{(mean of compliance bits, ignoring NaN)}
$$

These are different formulas and the kernel MUST not mix them.  Tested
via property test (AC17).

---

## 9. Rollup compute pipeline

### 9.1 Stage T0 — Prometheus recording rules (continuous)

Implemented by `slos/recording-rules/slo-windows.yaml` (additive to
PR #19's rules).  Each SLO has 4 series for the 4 fast windows :
5m / 30m / 1h / 6h.  Total 4 × 4 = 16 series ; plus the 4 metadata
series `slo:{SLO}_target` → 20 series.

Recording rules emit only :

- `slo:{SLO}_5m{source=prod}` (e.g. `slo:L2_AVAILABILITY_5m{source=prod}`)
- ... same for 30m / 1h / 6h

These are queried by Grafana (dashboard `slo-overview`) and by the
T1 stage.

### 9.2 Stage T1 — hourly daily-Parquet builder (CronJob, 0 * * * *)

Implementation : `src/rollup/stage_t1.rs`.

For the **previous full hour** $[H_{-1}, H)$ :

1. Query Prom for each SLO's 5-min, 30-min, 1-h SLI series via PromQL
   over the hour.
2. Build 12 rows of 5-min data + 2 rows of 30-min + 1 row of 1-h
   = 15 rows per SLO per hour ⇒ 60 rows / hour for 4 SLOs.
3. Append to the daily Parquet file `slos/v{ver}/2026-05/daily/
   2026-05-15.parquet` (one row group per hour ⇒ 24 row groups).
4. If today's file already has a row group for this hour, T1 is a
   **no-op** (idempotent — AD7).
5. Emit a Prom counter `efe_slo_rollup_t1_runs_total{result=...}`.

Edge cases :

- **First-hour-of-day** : creates the file.
- **Last-hour-of-day** : closes the file (writes footer + flushes).
- **Late scrape** : if Prom is missing data for the last 5-min bucket
  of the hour, that bucket is marked `coverage_low` (AD14).

### 9.3 Stage T2 — monthly JSON builder (CronJob, 0 3 1 * *)

Implementation : `src/rollup/stage_t2.rs`.

On the 1st of month at 03:00 UTC :

1. List all daily Parquet files for the previous month.
2. Verify count == days-in-prev-month ; report `missing_days` if any.
3. For each SLO, compute the 30d SLI using the appropriate
   aggregation rule (§ 8.2).
4. Compute the 30d error budget remaining (Appendix H).
5. Emit `slos/v{ver}/monthly/2026-05.json` (canonical form ; tested
   via golden — AC18).
6. Open a PR with the new file (peter-evans/create-pull-request@v6).

### 9.4 Stage T3 — quarterly signed PDF (CronJob, 0 4 1 1,4,7,10 *)

Implementation : `src/rollup/stage_t3.rs`.

On the 1st of {Apr, Jul, Oct, Jan} at 04:00 UTC :

1. List the 3 monthly JSON files of the previous quarter.
2. Verify all 3 exist (else fail — AC16).
3. Build the quarterly JSON summary.
4. Build the PDF from the 7-section template (§ 13).
5. Compute SHA256 of the JSON, embed in PDF metadata
   (`/EFE_EVIDENCE_JSON_SHA256`).
6. Sign the PDF with cosign keyless OIDC.
7. Upload to S3 + open a PR adding the artefacts.

---

## 10. Storage layer

### 10.1 Daily Parquet schema v1

```
slos/v0.6.0/2026-05/daily/2026-05-15.parquet

Columns (apache-arrow types) :
  schema_version          : UInt32     (= 1)
  chart_version           : String     (e.g. "0.6.0")
  region                  : String     ("us-east-1" | "global" | ...)
  slo_id                  : String     ("L2_AVAILABILITY" | ...)
  window_start_unix       : Int64      (seconds since epoch)
  window_size_s           : UInt32     (300 | 1800 | 3600 | 21600 | 86400)
  source                  : String     ("prod" | "synthetic_loadgen" | "bench")
  numerator               : UInt64     (raw event count — for SLO-1/3)
  denominator             : UInt64     (raw event count)
  compliance_bit          : UInt8      (0 | 1 | 255=NaN — for SLO-2/4)
  coverage_pct            : Float32    (0.0 – 1.0)
  __rollup_run_id         : String     (uuid v7)
  __rollup_timestamp      : Int64      (unix seconds — when this row was written)

Row group : 1 per hour ⇒ 24 row groups for a complete day.
Compression : zstd level 3.
```

### 10.2 Monthly JSON schema v1

```jsonc
// slos/v0.6.0/monthly/2026-05.json
{
  "$schema": "https://efe.example.com/schemas/slo/monthly-v1.json",
  "schema_version": 1,
  "chart_version": "0.6.0",
  "region": "global",
  "period": "2026-05",
  "computed_at": "2026-06-01T03:00:00Z",
  "rollup_run_id": "01J3F7XGGYM5NZ0CWY3Q7RH3JE",
  "slos": {
    "L2_AVAILABILITY": {
      "target_pct": 99.5,
      "achieved_pct": 99.62,
      "verdict": "pass",
      "numerator": 4823917,
      "denominator": 4842331,
      "error_budget_consumed_pct": 76.0
    },
    "L2_LATENCY_P95": { /* same shape */ },
    "SWARM_RECOVERY_TIME": { /* same shape */ },
    "REPLICATION_LAG_P99": { /* same shape */ }
  },
  "missing_days": [],
  "stability_contract_version": "1.4"
}
```

### 10.3 Quarterly PDF + JSON schema v1

The PDF is a 7-page document (cover + methodology + 4 per-SLO pages +
appendix) ; the JSON sidecar carries the same data plus the signature
+ embedded-hash.  See § 13 for the layout.

---

## 11. Burn-rate alerts

### 11.1 The math (Google SRE 2-window)

Let $E$ be the error budget (e.g. 0.5% = 0.005 for SLO-1).

Define the burn rate of a window $W$ as :

$$
\text{burn}(W) = \frac{1 - \text{SLI}(W)}{E}
$$

A burn rate of $r$ means the budget would be exhausted in $30d / r$.

**Fast burn alert** : burn(1h) ≥ 14.4× ⇒ budget exhausted in
30d/14.4 ≈ 2 d ⇒ critical.

**Slow burn alert** : burn(6h) ≥ 6× ⇒ budget exhausted in
30d/6 = 5 d ⇒ warning.

To suppress isolated spikes, **both** the 1h and 5m must exceed
the same threshold (multi-window AND).  Formally, the alert fires
when :

```
( burn(1h) >= 14.4 ) AND ( burn(5m) >= 14.4 )    // fast
( burn(6h) >=  6.0 ) AND ( burn(30m) >= 6.0 )    // slow
```

### 11.2 PrometheusRule YAML structure

(see Appendix D for the full YAML)

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: slo-burn-rate-2window
spec:
  groups:
    - name: slo-burn-rate-fast
      interval: 1m
      rules:
        - alert: SLOBurnRateFast_L2Availability
          expr: |
            (1 - slo:L2_AVAILABILITY_1h{source="prod"}) / 0.005 >= 14.4
            AND
            (1 - slo:L2_AVAILABILITY_5m{source="prod"}) / 0.005 >= 14.4
          for: 2m
          labels:
            severity: critical
            slo: L2_AVAILABILITY
            burn: fast
            stability_contract: v1.4
          annotations:
            summary: "L2 availability fast-burn (2-day budget exhaustion)"
            runbook: "https://efe.github.io/slos/runbooks/burn-rate.html#fast"
```

(× 4 SLOs × 2 windows = 8 alerts)

---

## 12. Public SLO page

### 12.1 Site map

```
https://efe.github.io/slos/
├── /                                 → overview.html (4 badges + 30d trend)
├── /slo-1-availability.html
├── /slo-2-latency-p95.html
├── /slo-3-recovery-time.html
├── /slo-4-replication-lag.html
├── /methodology.html
├── /error-budget.html
├── /quarterly-archive.html
├── /stability-contract-v1.4.html
├── /current.json                     ← JSON API (cached 5 min)
├── /history/{yyyy-mm}.json
├── /quarterly/{Q1..4}-{yyyy}.json
├── /quarterly/{Q1..4}-{yyyy}.pdf     ← signed PDF
├── /version.json
└── /badges/
    ├── slo-1-availability.svg
    ├── slo-2-latency-p95.svg
    ├── slo-3-recovery-time.svg
    └── slo-4-replication-lag.svg
```

### 12.2 JSON API contract

```jsonc
// /current.json — refreshed every hour by the slo-publish workflow
{
  "schema_version": 1,
  "as_of": "2026-05-16T01:00:00Z",
  "chart_version": "0.6.0",
  "slos": {
    "L2_AVAILABILITY": {
      "value": 99.62,
      "target": 99.5,
      "verdict": "pass",
      "burn_rate_1h": 0.8,
      "burn_rate_6h": 1.1,
      "error_budget_remaining_pct": 24.0
    },
    /* …other 3 */
  }
}
```

---

## 13. Quarterly compliance evidence (signed PDF + JSON)

### 13.1 PDF layout (7 sections, ~ 11 pages)

| Section | Page | Content |
|---|---|---|
| 1. Cover | 1 | Title, period, chart_version, audit-trail hash chain, hash-witnessed timestamp |
| 2. Methodology | 2 | Pointer to STABILITY_CONTRACT v1.4 + SLI math from § 7 |
| 3. SLO-1 detail | 3 | Daily Parquet roll-up table + monthly + 90d verdict |
| 4. SLO-2 detail | 4 | idem |
| 5. SLO-3 detail | 5 | idem |
| 6. SLO-4 detail | 6 | idem |
| 7. Appendix + signature | 7–11 | Raw 90d trend table, JSON sidecar SHA256, cosign signature page |

### 13.2 Signing chain (cosign keyless OIDC)

```
github-workflow ───OIDC────> Fulcio (sigstore CA)
              └──cosign────> Rekor (transparency log)
                           ↓
                       slo-quarterly.yml signs Q2-2026.pdf with the
                       ephemeral identity
                       https://github.com/efe-ai/efe/.github/workflows/slo-quarterly.yml@refs/tags/v0.6.0
                           ↓
                       Verifier (offline) :
                           cosign verify-blob --certificate-identity-regexp
                              ".*slo-quarterly.yml.*"
                              --certificate-oidc-issuer
                              https://token.actions.githubusercontent.com
                              --bundle Q2-2026.pdf.bundle Q2-2026.pdf
```

### 13.3 Verifier CLI

```
$ efe-slo-rollup evidence verify Q2-2026.pdf
[2026-07-01T05:01:23Z INFO ]  verifying PDF embedded JSON SHA256...
[2026-07-01T05:01:23Z INFO ]  SHA256 match  : 8e91ba3f...
[2026-07-01T05:01:23Z INFO ]  cosign verify : VALID
[2026-07-01T05:01:23Z INFO ]    issuer       : https://token.actions.githubusercontent.com
[2026-07-01T05:01:23Z INFO ]    identity     : https://github.com/efe-ai/efe/.github/workflows/slo-quarterly.yml@refs/tags/v0.6.0
[2026-07-01T05:01:23Z INFO ]    rekor uuid   : 0f9c...
✓  evidence valid for Q2 2026, chart 0.6.0
```

---

## 14. Crate `efe-slo-rollup` — module map

(see § 2.4 for the directory tree)

### 14.1 Public surface (lib.rs)

```rust
pub mod sli;
pub mod window;
pub mod rollup;
pub mod storage;
pub mod source;
pub mod burn_rate;
pub mod badge;
pub mod evidence;
pub mod publish;
pub mod cli;
pub mod error;
pub mod build;

pub use crate::error::{ExitCode, SloError, SloResult};
pub use crate::sli::SliKernel;
pub use crate::rollup::RollupOrchestrator;
pub use crate::burn_rate::{BurnRate, BurnRateVerdict};
pub use crate::evidence::{Evidence, EvidenceVerification};

/// STABILITY_CONTRACT v1.4 — frozen SLO IDs.
pub const SLO_IDS: &[&str] = &[
    "L2_AVAILABILITY",
    "L2_LATENCY_P95",
    "SWARM_RECOVERY_TIME",
    "REPLICATION_LAG_P99",
];

/// STABILITY_CONTRACT v1.4 — frozen thresholds.
pub mod thresholds {
    pub const L2_AVAILABILITY_TARGET: f64 = 0.995;
    pub const L2_LATENCY_P95_TARGET_S: f64 = 0.050;
    pub const L2_LATENCY_P95_WINDOW_COMPLIANCE: f64 = 0.99;
    pub const SWARM_RECOVERY_TIME_TARGET_S: f64 = 60.0;
    pub const SWARM_RECOVERY_TIME_COMPLIANCE: f64 = 0.99;
    pub const REPLICATION_LAG_TARGET: u64 = 100;
    pub const REPLICATION_LAG_COMPLIANCE: f64 = 0.99;
    pub const COVERAGE_MIN: f64 = 0.80;
    pub const LOW_TRAFFIC_DENOMINATOR_MIN: u64 = 100;
    pub const LOW_EVENT_COUNT_MIN: u64 = 10;
}

pub const SCENARIO_SCHEMA_VERSION: u32 = 1;
pub const MONTHLY_SCHEMA_VERSION: u32 = 1;
pub const QUARTERLY_SCHEMA_VERSION: u32 = 1;
pub const DAILY_PARQUET_SCHEMA_VERSION: u32 = 1;
```

### 14.2 SliKernel trait

```rust
pub trait SliKernel: Send + Sync {
    fn slo_id(&self) -> &'static str;
    fn aggregation(&self) -> SliAggregation;        // RatioOfSums | WindowCompliance
    fn compute(&self, ctx: &SliContext) -> SloResult<SliPoint>;
    fn verdict(&self, sli: &SliPoint) -> SloVerdict; // Pass | Warn | Fail | Indeterminate
}
```

Each of the 4 SLOs implements `SliKernel` and is dispatched by an enum
in `src/sli/mod.rs::dispatch_for(slo_id) -> Box<dyn SliKernel>`.

---

## 15. CLI design

### 15.1 Subcommand surface (frozen, STABILITY_CONTRACT v1.4 § 22.5)

```
efe-slo-rollup compute      # Compute SLI(s) for a window/period — used by T0/T1 jobs
efe-slo-rollup rollup        # Run a full T1/T2/T3 stage
efe-slo-rollup publish       # Build the public site (mdbook + JSON API + badges)
efe-slo-rollup evidence sign|verify  # Sign or verify a quarterly PDF
efe-slo-rollup list          # List computed SLIs (range query)
efe-slo-rollup validate      # Schema-validate stored files
efe-slo-rollup version       # Print version + git_sha
```

### 15.2 Critical flags (frozen)

| Flag | Default | Purpose |
|---|---|---|
| `--prom-url` | `http://prometheus.monitoring.svc:9090` | Source for T1 (envvar `EFE_PROM_URL`) |
| `--slo` | (repeatable) | Subset of SLO IDs to compute (default = all 4) |
| `--window` | `1h` (humantime) | T0/T1 window |
| `--offset` | `0` | Negative offset for backfill |
| `--storage-root` | `./slos/v{chart}/` | Output root |
| `--chart-version` | env `EFE_CHART_VERSION` | Required (no default — fail if missing) |
| `--region` | `global` | Cardinality-capped label |
| `--release-backfill-confirmed` | false | Required for `compute --backfill` |
| `--cosign-identity` | (envvar) | For `evidence sign` (Phase-2 : keyless OIDC) |
| `--target-override` | none | (mirrors PR #21 G6) override Prom URL at CLI level |
| `--dry-run` | false | No I/O, prints plan |
| `--format` | `json` (text in TTY) | `text` | `json` | `csv` |

---

## 16. Helm chart

### 16.1 3 CronJobs

| CronJob | Schedule | RBAC | Resources |
|---|---|---|---|
| `efe-slo-rollup-hourly` | `0 * * * *` | Role : read Prom, write PVC | cpu 200m / mem 256Mi |
| `efe-slo-rollup-monthly` | `0 3 1 * *` | Role + S3 write OIDC | cpu 500m / mem 1Gi |
| `efe-slo-rollup-quarterly` | `0 4 1 1,4,7,10 *` | Role + cosign keyless | cpu 800m / mem 2Gi |

All security hardening : `runAsNonRoot=true`, `readOnlyRootFilesystem=true`,
`allowPrivilegeEscalation=false`, `capabilities.drop=[ALL]`,
`seccompProfile=RuntimeDefault`.  RBAC is **namespace-scoped**
(Role, not ClusterRole) — same constraint as PR #21 (T93).

### 16.2 Values surface (frozen — STABILITY_CONTRACT v1.4 § 22.6)

```yaml
chartVersion: 0.6.0       # REQUIRED — no default
region: global
storage:
  backend: pvc            # pvc | s3 | gcs
  pvc:
    enabled: true
    size: 50Gi
  s3:
    bucket: ""
    keyPrefix: slos/
cronjobs:
  hourly:
    schedule: "0 * * * *"
    enabled: true
  monthly:
    schedule: "0 3 1 * *"
    enabled: true
  quarterly:
    schedule: "0 4 1 1,4,7,10 *"
    enabled: true
cosign:
  identity: ""            # required for quarterly
prometheus:
  url: http://prometheus.monitoring.svc:9090
```

---

## 17. CI workflows (4 new)

| Workflow | Trigger | Action |
|---|---|---|
| `slo-compute.yml` | cron `0 * * * *` + workflow_dispatch | T1 hourly rollup → write daily Parquet → push to S3 |
| `slo-monthly.yml` | cron `0 3 1 * *` | T2 monthly rollup → write monthly JSON → open PR |
| `slo-quarterly.yml` | cron `0 4 1 1,4,7,10 *` | T3 quarterly → write PDF + JSON, sign with cosign keyless OIDC, upload to S3, open PR |
| `slo-publish.yml` | on push:main, on completion of the 3 above | Build mdbook + JSON API + badges, push to gh-pages |

Each workflow uses GitHub OIDC for AWS auth (no static keys — T98 + T101 mitigation).

---

## 18. PR #20 loadgen consumer contract

### 18.1 The `slo-source` feature

```toml
[features]
slo-source = []
```

Compiling `efe-meta-swarm-loadgen` with `--features=slo-source`
adds the public type :

```rust
pub mod slo_source {
    pub enum SloSourceEvent {
        RequestComplete { status: RequestStatus, duration_ns: u64, timestamp_ns: u64 },
        ChaosFaultBegin { fault_id: String, at_ns: u64 },
        ChaosFaultRecovered { fault_id: String, duration_ns: u64 },
    }
    pub trait SloSourceSink: Send + Sync {
        fn emit(&self, event: SloSourceEvent);
    }
}
```

The rollup binary registers a sink before calling `loadgen::run(config)` ;
events flow over an mpsc channel and are aggregated into synthetic
SLI cells (labelled `source=synthetic_loadgen`).

### 18.2 No subprocess fallback

This is an in-process-only path.  Subprocess would deny T103 (priority
inversion) mitigation — the rollup MUST stay co-scheduled with
loadgen on the same NUMA node.

---

## 19. PR #21 bench harness coupling

The rollup binary reads `bench/results/{run_id}/{scenario_id}/summary.json`
via `src/source/bench_baseline.rs`.  Each `summary.json` is converted to
1 SLI cell per SLO with `source=bench` ; never mixed with prod cells.

Phase-3 (G.11) extends this to detect trend regression across baselines.

---

## 20. Multi-tenancy + multi-chart-version

PR #22 is **single-tenant** in scope but multi-chart-version-ready :

- Every Parquet/JSON/PDF file is keyed by `chart_version`.
- Burn-rate alerts carry `chart_version` label ; PromQL filters can
  shard by it.
- The public page renders one section per chart_version under
  `https://efe.github.io/slos/{chart_version}/`.

Multi-tenant (per-customer) is **G.3**, Phase-3 — requires a separate
identity model (per-tenant signing key, per-tenant evidence chain).

---

## 21. STABILITY_CONTRACT v1.4 additions

### 21.1 New section (`docs/STABILITY_CONTRACT.md § 22`)

```markdown
## 22. SLO surface — frozen v1.4 (PR #22)

### 22.1 SLO IDs (closed taxonomy, 4)
  L2_AVAILABILITY | L2_LATENCY_P95 | SWARM_RECOVERY_TIME | REPLICATION_LAG_P99

### 22.2 Thresholds (frozen for ≥ 6 chart-minor versions)
  see slos/rollup/src/lib.rs::thresholds

### 22.3 Window taxonomy (8 tiers, frozen)
  5m / 30m / 1h / 6h / 24h / 7d / 30d / 90d (UTC-aligned)

### 22.4 Storage schemas (frozen)
  - daily Parquet v1 (slos/schemas/daily-parquet-v1.json)
  - monthly JSON v1 (slos/schemas/monthly-json-v1.json)
  - quarterly evidence v1 (slos/schemas/quarterly-evidence-v1.json)

### 22.5 CLI subcommand surface (frozen)
  compute | rollup | publish | evidence | list | validate | version

### 22.6 Helm values surface (frozen)
  chartVersion, region, storage.*, cronjobs.*, cosign.identity,
  prometheus.url

### 22.7 Public page surface (frozen)
  /current.json | /history/{yyyy-mm}.json | /quarterly/{Q1..4}-{yyyy}.{json,pdf}
  | /badges/slo-{1..4}-{slug}.svg

### 22.8 Compliance chain
  cosign keyless OIDC via GitHub Actions ; transparency log = Rekor ;
  public key = deploy/cosign/slo-evidence.pub
```

### 21.2 Frozen test suite (`slos/rollup/tests/stability_contract_v14.rs`)

```rust
#[test]
fn slo_ids_match_v1_4() {
    assert_eq!(efe_slo_rollup::SLO_IDS, &[
        "L2_AVAILABILITY",
        "L2_LATENCY_P95",
        "SWARM_RECOVERY_TIME",
        "REPLICATION_LAG_P99",
    ]);
}

#[test]
fn thresholds_match_v1_4() {
    use efe_slo_rollup::thresholds::*;
    assert_eq!(L2_AVAILABILITY_TARGET, 0.995);
    assert_eq!(L2_LATENCY_P95_TARGET_S, 0.050);
    /* …all 10 */
}

#[test]
fn window_taxonomy_8_tiers() {
    use efe_slo_rollup::window::WindowSpec;
    assert_eq!(WindowSpec::all().len(), 8);
}

#[test]
fn cli_subcommands_match_v1_4() { /* … */ }
```

---

## 22. Security model + RBAC + signing

### 22.1 RBAC matrix (per-CronJob)

| CronJob | API groups | resources | verbs | namespace scope |
|---|---|---|---|---|
| hourly | `""` | services, endpoints | get/list/watch | rollup namespace |
| hourly | `""` | pods, pods/log | get/list/watch | rollup namespace |
| hourly | (no chaos / no cluster-wide) | — | — | — |
| monthly | `""` | persistentvolumeclaims | get/list | rollup namespace |
| quarterly | (uses GH OIDC for cosign) | — | — | — |

All Role (not ClusterRole) — T103 + cross-tenant T104 mitigations.

### 22.2 PII guard

A custom Cargo feature `deny-pii` enables compile-time lints that
fail the build if forbidden label names appear in any `slo_id ::` or
`label` literal :

```rust
#[cfg_attr(feature = "deny-pii", deny(efe_no_pii_label))]
```

(implemented as a clippy-like proc-macro lint — Phase-3 G.12)

### 22.3 Signing key rotation

- Public key committed at `deploy/cosign/slo-evidence.pub`.
- Private key in org KMS, rotated annually.
- Old signatures remain valid via Rekor (transparency log retains
  the historical witness).

### 22.4 Evidence chain integrity

| Property | Implementation |
|---|---|
| Tamper-evidence | PDF embeds SHA256 of JSON ; verifier recomputes |
| Authenticity | cosign signature with OIDC identity |
| Non-repudiation | Rekor transparency log (append-only) |
| Time-witnessing | OIDC token contains the workflow run timestamp |

---

## 23. Threat model (T96 – T105)

| ID | Threat | Severity | Mitigation |
|---|---|---|---|
| T96 | SLO evidence tampering | SEV-2 | Signed PDF + embedded JSON SHA256 |
| T97 | Rollup PII leak via label | SEV-2 | AD17 + `deny-pii` feature gate + label allow-list |
| T98 | Parquet DoS via window explosion | SEV-3 | Hard cap : 24 row groups × 60 rows / row group / file (1 440 rows max) |
| T99 | Burn-rate alert flap-storm | SEV-3 | 2-window AND requirement (AD39) |
| T100 | Public-page defacement via SLO badge | SEV-2 | SVG generation is deterministic + reviewed ; gh-pages branch is protected ; CSP header |
| T101 | Compliance signature replay | SEV-2 | OIDC token includes workflow run id + Rekor transparency log |
| T102 | Prom recording-rule poisoning | SEV-2 | Recording rules in git ; CI lint enforces `slo:*` namespace |
| T103 | Rollup CronJob priority inversion | SEV-3 | priorityClass `system-cluster-critical` + dedicated namespace |
| T104 | Cross-tenant SLO bleed via label | SEV-2 | `region` label is allow-listed at compile time |
| T105 | Quarterly PDF embedded JS | SEV-2 | printpdf produces flat-text PDFs ; CI lint scans for `/JavaScript` keyword |

---

## 24. Acceptance criteria (AC1 – AC38)

### 24.1 Math correctness

- **AC1** — Each SLI kernel computes the formula in § 7 exactly (golden inputs).
- **AC2** — Per-SLO unit tests cover at least 7 cases (nominal, edge low-traffic, edge high-traffic, coverage_low, counter_reset, multi-region, schema_version mismatch).
- **AC3** — Property-based test : 30d SLI = ratio-of-sums for SLO-1/3, mean of bits for SLO-2/4 (AD24 + § 8.2 invariant).
- **AC4** — `recovery_time` SLI handles 0 events (returns INSUFFICIENT_EVENTS).
- **AC5** — `replication_lag` SLI takes the min across participants (AD16 — tested).
- **AC6** — Burn-rate fast/slow math matches the 14.4× / 6× targets (golden).
- **AC7** — Burn-rate alert fires only when BOTH windows exceed (AD39 — tested).

### 24.2 Boundary semantics

- **AC8** — Window with coverage < 80% is marked `coverage_low` and excluded.
- **AC9** — Negative rate (counter reset) is marked + excluded.
- **AC10** — Daylight saving has no effect (test uses 2026-03-29 Europe/Paris boundary — file still aligned to UTC).
- **AC11** — Low-event guard for SLO-3 emits INSUFFICIENT_EVENTS verdict.
- **AC12** — Missing-participant for SLO-4 computes min across the rest.
- **AC13** — 5-min window alignment is exact (no clock drift).
- **AC14** — Rolling 30d window starts at the first available daily file.
- **AC15** — T2 reports `missing_days=[…]` when a daily file is absent.
- **AC16** — T3 fails (ExitCode::StorageIO) if a monthly is missing.

### 24.3 Storage + retention

- **AC17** — Parquet round-trip preserves all 13 columns.
- **AC18** — Monthly JSON golden test passes byte-for-byte (canonical form).
- **AC19** — Quarterly PDF embeds the JSON SHA256 in metadata.
- **AC20** — Retention scan deletes daily files older than 90d.
- **AC21** — `--release-backfill-confirmed` is required for backfill (analog of PR #21 AD16).

### 24.4 Public surface

- **AC22** — `/current.json` is valid against `monthly-json-v1.json` (modulo `as_of` vs `period`).
- **AC23** — 4 SVG badges are byte-stable across runs (golden).
- **AC24** — mdbook site has 9 pages (overview + 4 SLOs + methodology + error-budget + quarterly archive + STABILITY_CONTRACT v1.4).
- **AC25** — `/quarterly/{Q1..4}-{yyyy}.pdf` is accompanied by `.pdf.bundle` (cosign).
- **AC26** — Public page is rendered without client-side JS (T100 mitigation, tested via curl + grep).

### 24.5 Compliance + signing

- **AC27** — `efe-slo-rollup evidence sign Q2-2026.pdf` produces a valid cosign signature using keyless OIDC.
- **AC28** — `efe-slo-rollup evidence verify Q2-2026.pdf` exits 0 for a valid PDF + bundle.
- **AC29** — Tampered PDF (1 byte flipped) ⇒ verify exits non-0.
- **AC30** — Wrong signing identity ⇒ verify exits non-0.
- **AC31** — `cargo check --features deny-pii` rejects forbidden labels at compile time.

### 24.6 CLI + ops

- **AC32** — `efe-slo-rollup --help` lists 7 subcommands, frozen.
- **AC33** — `efe-slo-rollup validate slos/v0.6.0/` checks every file against its schema.
- **AC34** — `efe-slo-rollup list --slo L2_AVAILABILITY --range 30d --format csv` produces valid CSV (header line + N rows).
- **AC35** — CronJob hourly completes within 60s on a representative scrape (k=10 SLO series × 4 SLOs).
- **AC36** — CronJob monthly completes within 5 min for 30 daily files.
- **AC37** — CronJob quarterly completes within 15 min including cosign + S3 upload.
- **AC38** — STABILITY_CONTRACT v1.4 frozen-test suite (§ 21.2) passes.

---

## 25. Tests strategy

### 25.1 Test layers

| Layer | Path | Count | Purpose |
|---|---|---|---|
| Unit kernel | `src/sli/{slo}.rs::tests` | 4 × 7 = 28 | per-SLO math |
| Unit window | `src/window/*.rs::tests` | 18 | bucket alignment, gap handling |
| Property | `tests/window_property.rs` | 6 | proptest invariants |
| Integration chain | `tests/rollup_chain.rs` | 4 | T0→T1→T2→T3 roundtrip |
| Burn-rate | `tests/burn_rate_invariant.rs` | 8 | one per alert |
| Badge golden | `tests/badge_svg_golden.rs` | 4 | byte-stable SVG |
| Evidence | `tests/evidence_signature.rs` | 3 | sign+verify+tamper |
| STABILITY frozen | `tests/stability_contract_v14.rs` | 7 | locks the v1.4 contract |
| CLI golden | `tests/cli_golden.rs` | 8 | flag taxonomy + ExitCode |
| Compute golden | `tests/compute_golden.rs` | 12 | end-to-end with PromQL JSON fixtures |
| **Total** | | **102** | |

### 25.2 Test fixtures

`tests/data/` holds 12 fixture files :

- 4 Prom JSON responses (one per SLO, nominal)
- 4 Prom JSON responses (edge cases)
- 1 daily Parquet (golden)
- 1 monthly JSON (golden)
- 1 quarterly JSON (golden)
- 1 quarterly PDF + .pdf.bundle (cosign valid)

---

## 26. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Cosign keyless OIDC fails | LOW | HIGH | Fallback to manual signing (release-eng) + alarm |
| Prom outage during T1 hourly | MED | MED | T1 marks missing buckets `coverage_low` ; resumes next hour |
| Parquet writer corrupted | LOW | HIGH | Atomic rename (AD29) ; verify roundtrip after write |
| Schema v1 design flaw discovered | LOW | HIGH | v2 migrator path planned (G.9) — out of Phase-2 scope |
| Burn-rate alert false-positive flap | MED | MED | 2-window AND (AD39) ; multi-condition `for: 2m` |
| GitHub Pages quota exceeded | LOW | LOW | 4 SVG badges + 9 HTML pages + 30 JSON files = ~ 2 MiB ; far below limit |
| OIDC subject change breaks verifier | LOW | HIGH | Frozen identity regex documented in STABILITY_CONTRACT v1.4 § 22.8 |
| Quarterly PDF too large | LOW | LOW | Hard cap : 11 pages, < 500 KiB |

---

## 27. Rollback strategy

### 27.1 Code rollback

`git revert` the merge commit ; CI workflows remain harmless (cron
jobs just emit no new files).  Prom recording rules survive
(they're additive — removing them is a separate revert).

### 27.2 Data rollback

Daily Parquet : files are immutable ; "rolling back" means writing a
new file at the next hour boundary (the historical file remains
for audit).

Quarterly PDF : signed + on Rekor ; cannot be "rolled back".  An
erratum is published as a follow-up signed PDF with a pointer back
to the original.

### 27.3 Schema rollback

A schema v1 → v0 rollback is NOT supported.  Phase-3 provides v1 → v2
migrators only.

---

## 28. Sequence diagrams

### 28.1 Hourly T1 rollup

```
   K8s CronJob @ 0 * * * *
         │
         ▼
   efe-slo-rollup compute --window 1h --offset -1h
         │
         ├── source::prom_query::fetch_range(prom_url, queries) ─────► Prometheus
         │                                                            (HTTP /api/v1/query_range)
         ├── sli::dispatch(L2_AVAILABILITY).compute(ctx)  ─► SliPoint
         ├── sli::dispatch(L2_LATENCY_P95).compute(ctx)   ─► SliPoint
         ├── sli::dispatch(SWARM_RECOVERY_TIME).compute() ─► SliPoint
         ├── sli::dispatch(REPLICATION_LAG_P99).compute() ─► SliPoint
         │
         ├── storage::parquet::append_row_group(...)
         │       writes temp file → atomic rename to
         │       slos/v0.6.0/2026-05/daily/2026-05-15.parquet
         │
         └── emit_prom_counter(efe_slo_rollup_t1_runs_total{result=success})
```

### 28.2 Monthly T2

```
   K8s CronJob @ 0 3 1 * *
         │
         ▼
   efe-slo-rollup rollup --stage T2 --period 2026-05
         │
         ├── storage::parquet::list_daily(period) → 31 files
         │       verify count == days-in-month  (or report missing_days)
         │
         ├── for each SLO :
         │     compute 30d SLI per AD24 + § 8.2
         │
         ├── storage::json::write_monthly(period, payload)
         │       writes canonical JSON, sorted keys
         │
         └── publish::open_pr(branch="slo-monthly-2026-05", title="...")
```

### 28.3 Quarterly T3

```
   K8s CronJob @ 0 4 1 1,4,7,10 *
         │
         ▼
   efe-slo-rollup rollup --stage T3 --quarter Q2-2026
         │
         ├── storage::json::list_monthly(quarter) → 3 files
         │       fail (ExitCode::StorageIO) if any missing
         │
         ├── evidence::build_json(monthlies) → quarterly JSON
         ├── evidence::build_pdf(quarterly JSON) → quarterly PDF
         │     pdf metadata embeds SHA256(quarterly JSON)
         │
         ├── evidence::sign(pdf) → cosign keyless OIDC
         │     POST Rekor entry → returns transparency log uuid
         │
         ├── storage::object_store::put(s3://efe-slo-evidence/...)
         │
         └── publish::open_pr(branch="slo-quarterly-Q2-2026", title="...")
```

---

## 29. Reviewer checklist

| Check | Status |
|---|---|
| 4 SLO IDs frozen in `SLO_IDS` constant | ☐ |
| `thresholds` mod matches STABILITY_CONTRACT v1.4 § 22.2 | ☐ |
| Window taxonomy : 8 tiers, UTC-aligned | ☐ |
| T0 recording rules : 16 series, `slo:*` namespace | ☐ |
| T1 idempotent (re-run same hour ⇒ no-op) | ☐ |
| T2 monthly JSON is canonical-form (sorted keys, 2-space indent) | ☐ |
| T3 PDF + cosign signature roundtrip | ☐ |
| Burn-rate alerts : 2-window AND, 14.4× / 6× | ☐ |
| Public page renders without JS | ☐ |
| `evidence verify` exits 0 only for valid + matching identity | ☐ |
| `deny-pii` feature rejects forbidden labels at compile time | ☐ |
| Helm chart : 3 CronJobs, namespace-scoped Role | ☐ |
| CI : 4 workflows, OIDC auth (no static keys) | ☐ |
| Tests : 102 green ; clippy `-D warnings` clean | ☐ |
| STABILITY_CONTRACT v1.4 frozen-test suite (7 tests) passes | ☐ |

---

## 30. Appendix A — CLI flag reference

(every flag listed alphabetically, with default + envvar + frozen-or-not)

```
--chart-version <VERSION>            REQUIRED    env: EFE_CHART_VERSION    frozen
--cosign-identity <IDENTITY>         (no def)    env: COSIGN_IDENTITY       frozen
--dry-run                            false                                 frozen
--format <text|json|csv>             json|text   (text in TTY)             frozen
--log-format <text|json>             json|text   (text in TTY)             frozen
--log-level <level>                  info        env: RUST_LOG              frozen
--offset <duration>                  0           humantime                 frozen
--prom-url <URL>                     http://prom env: EFE_PROM_URL         frozen
--region <REGION>                    global      allow-list compile-time    frozen
--release-backfill-confirmed         false       (required for backfill)   frozen
--slo <SLO_ID>                       (rep)       (default = all 4)         frozen
--storage-root <PATH>                ./slos      env: EFE_STORAGE_ROOT     frozen
--target-override <URL>              (no def)    overrides --prom-url       frozen
--window <duration>                  1h          humantime                 frozen
```

---

## 31. Appendix B — SLI math derivation

(full derivation of the formulas in § 7 with worked examples per SLO,
including 3 numerical scenarios per SLO — nominal / edge / pathological.
Pages 31-37 of the doc — content moved to dedicated section in the
runbook.)

---

## 32. Appendix C — recording-rule YAML diff vs PR #19

```yaml
# slos/recording-rules/slo-windows.yaml  (NEW)
groups:
  - name: slo-l2-availability
    interval: 30s
    rules:
      - record: slo:L2_AVAILABILITY_5m
        expr: |
          sum(rate(efe_l2_request_total{status="success", source="prod"}[5m]))
          /
          sum(rate(efe_l2_request_total{status!="client_error", source="prod"}[5m]))
        labels:
          slo: L2_AVAILABILITY
          window: 5m

      - record: slo:L2_AVAILABILITY_30m
        # …same with [30m]
      - record: slo:L2_AVAILABILITY_1h
        # …same with [1h]
      - record: slo:L2_AVAILABILITY_6h
        # …same with [6h]

  - name: slo-l2-latency-p95
    # …4 windows × p95
  - name: slo-swarm-recovery-time
    # …4 windows
  - name: slo-replication-lag-p99
    # …4 windows
```

(diff against PR #19 : NET +20 series, no removals)

---

## 33. Appendix D — burn-rate alert YAML

(full YAML for the 8 alerts ; see § 11.2 for the structure)

---

## 34. Appendix E — daily Parquet schema

(complete arrow schema with column metadata)

---

## 35. Appendix F — monthly JSON schema

(JSON-Schema Draft 2020-12 — see `slos/schemas/monthly-json-v1.json`)

---

## 36. Appendix G — quarterly evidence template

(complete PDF page-by-page layout with field positions, fonts, sizes)

---

## 37. Appendix H — error-budget formula + worked example

For an SLO with target $T$ (e.g. 99.5% ⇒ $T = 0.995$) over a 30-day
window :

**Error budget** :

$$
B = (1 - T) \times \text{TotalEvents}_{30d}
$$

**Consumed** :

$$
C = \text{BadEvents}_{30d}
$$

**Remaining (%)** :

$$
R\% = \max\left(0, \frac{B - C}{B}\right) \times 100
$$

**Worked example (SLO-1 over May 2026)** :

```
TotalEvents = 4 842 331
BadEvents   =    18 414
T           = 0.995
B           = (1 - 0.995) × 4 842 331 = 24 211.66
C           =                                18 414
R%          = (24 211.66 - 18 414) / 24 211.66 × 100 = 23.95%

⇒ ~ 24% of the May error budget remaining.
```

---

## 38. Appendix I — cross-version SLO compatibility matrix

(table : for each chart_version v0.4.x → v0.7.x, are the v1.4 SLO IDs
+ thresholds applicable ? Phase-2 ⇒ yes for all ; Phase-3 reserves
the right to introduce v1.5 with a strict superset.)

---

## 39. Appendix J — Phase-3 follow-ups (G.1 – G.12)

| # | Title | Brief |
|---|---|---|
| G.1 | Adaptive thresholds | ML-driven seasonal targets |
| G.2 | Cross-region SLO aggregation | Weighted blend per region |
| G.3 | Per-customer SLOs | Multi-tenant billing |
| G.4 | Multi-cluster federation | Federate recording rules |
| G.5 | CD gating on SLO trend | Auto-pause if burn > X |
| G.6 | SLO-driven autoscaler | PR #23 consumes this |
| G.7 | Legal auto-binding | Insurance + refund flow |
| G.8 | HSM-backed signing | Cosign + HSM hardening |
| G.9 | Schema v1 → v2 migrator | Compaction + retention policy |
| G.10 | Real-time WebSocket SLO | Live stream API |
| G.11 | SLO regression detection | Trend-aware verdict |
| G.12 | Customer-facing branded landing | UX team owned |

---

## End of architecture

This document is **frozen** — any change to SLO IDs, thresholds,
windows, or file schemas requires :

1. A new architecture PR (e.g. PR #26),
2. A bump of STABILITY_CONTRACT major version,
3. A migration plan covering existing daily / monthly / quarterly files.

The implementation (commits 1–14 of § 3) follows this spec exactly.
Any divergence is a contract violation and must be filed as an issue
referencing this doc.
