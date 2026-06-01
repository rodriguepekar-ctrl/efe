# EFE-A1 Phase 2 — PR #24 : `efe-bench-publish` — public bench results pipeline (consumes PR #21 + PR #22)

| Field | Value |
|---|---|
| **PR** | #24 (Sprint A1 Phase 2 / P2.8 W16 → P2.8 W17) |
| **Branch** | `efe-a1/p2-pr24-publish-bench` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team — release engineering area (lead) + perf-eng (co-lead) |
| **Reviewers** | 2× release-eng + 2× perf-eng + 1× SRE + 1× security + 1× legal + 1× marketing (public surface review) + 1× compliance |
| **Depends on** | PR #14 (Helm production-grade), PR #19 (Grafana dashboards — re-used as embed-able screenshots for the methodology page), PR #20 (`efe-meta-swarm-loadgen` — version reference embedded in methodology), PR #21 (`efe-bench-harness` — provides `bench/results/{run_id}/{scenario}/summary.json` + `bench_baselines/v{chart}/L2/*.json` + the existing `publish` module **stub** that this PR fills in completely), PR #22 (`efe-slo-rollup` — cross-link : SLO badges embedded in the bench public page sidebar ; quarterly evidence chain shared cosign keyless OIDC identity), PR #23 (`efe-smoke` — smoke badge cross-link in the bench overview header) |
| **Blocks** | GA v1.0 (no public release without an auditable bench surface published at a stable URL), customer-facing landing page (Phase-3 G.9 from PR #21 — consumes this pipeline), enterprise sales (legal cannot reference bench numbers in MSAs until they're on a permanent URL), customer benchmarks-as-a-service (Phase-3 G.7 from PR #21) |
| **L2 gate** | G21 (production-grade public bench results pipeline : 11 frozen SVG badges + 10 scenario detail pages + JSON API at `https://efe.github.io/bench/api/*.json` + signed quarterly evidence + cross-link with PR #22 SLO + STABILITY_CONTRACT v1.6 § 24 commitment + `bench-publish.yml` CI workflow with branch protection + cosign keyless OIDC for quarterly artefacts) |
| **Threat IDs in scope** | T116 – T125 (10 new : (T116) gh-pages defacement via leaked GITHUB_TOKEN, (T117) cross-tenant data leak via public badge URL, (T118) SVG injection via baseline numeric values, (T119) PR comment injection via badge filename interpolation, (T120) public page reveals internal IP / hostname, (T121) quarterly evidence revocation handling, (T122) gh-pages force-push race on concurrent CI runs, (T123) public CDN cache poisoning, (T124) stale badge served as fresh, (T125) public methodology page becomes phishing target) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS3 + § 17 (Public bench portal) + § 25 (Marketing-ready audit surface) |
| **CVSS bandwidth** | All ≤ SEV-2 (public surface is read-only ; no PII ; no tenant data ; max risk is reputation via defacement — limited blast radius) |
| **Crate version** | `efe-bench-harness` v0.1.0 → v0.2.0 (PR #21's `publish` module is fully implemented ; CLI subcommand `publish` gains 7 new flags ; STABILITY surface bumped to v1.6 § 24). **Optional** new sibling crate `efe-bench-publish` extracted from `efe-bench-harness::publish` when LOC > 3 500 (Phase-3 G.1 — for now extension-in-place keeps PR scope contained). |
| **Existing infrastructure consumed** | `bench/harness/src/publish/{mod,badge}.rs` (PR #21 stubs — replaced) ; `bench/results/{run_id}/{scenario_id}/summary.json` (PR #21 output schema v1) ; `bench_baselines/v{chart}/L2/{scenario_id}.json` (PR #21 baselines) ; `slos/v{chart}/monthly/{yyyy-mm}.json` (PR #22 SLO rollup output — read-only sidebar) ; `smoke/results/smoke.json` published by PR #23 (sibling badge) ; cosign keyless OIDC identity regex `^https://github\.com/efe-ai/efe/\.github/workflows/(slo-quarterly\|bench-publish-quarterly)\.yml@refs/.+$` (shared with PR #22) |
| **STABILITY_CONTRACT v** | 1.5 → **1.6** (new tier : *Bench public-page URL schema* + *11 badge file names* + *bench JSON API* + *quarterly evidence path layout* + *gh-pages branch protection requirements*) |
| **Total LOC budget** | Rust `publish` module extension ~3 400 ; 11 SVG badge generators (10 scenarios + 1 overall) ~520 ; sparkline trend SVG generator ~280 ; mdbook site source ~3 200 ; JSON API serialiser ~640 ; PDF + cosign evidence chain ~720 (re-uses PR #22 evidence module) ; CI workflows ~520 ; STABILITY_CONTRACT v1.6 addendum ~440 ; tests ~2 880 (golden SVGs + JSON schema + mdbook smoke + property) ; runbook + public methodology page + this arch doc ~4 280 ; gh-pages branch protection setup + repo settings docs ~360 → **~17 240 LOC** |
| **CI cost delta** | + 9 s for `cargo test -p efe-bench-harness --test bench_publish_*` (in-memory SVG + JSON unit tests) ; + 2 m 48 s for `bench-publish.yml` workflow (consumes nightly artefacts + generates site + deploys to gh-pages — runs only on push:main + after `bench-full.yml`/`bench-release.yml` completion) ; + 4 m 12 s for `bench-publish-quarterly.yml` (cosign + PDF + Rekor — runs Q1/Q2/Q3/Q4 cron) ; total **PR-blocking CI delta + 9 s** (heavy workflows are out-of-band) |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals + scope
   2. Pre-flight inventory — what PR #21 stubbed vs. what's missing
   3. Branch + commit plan (12 commits across 2 sprints)
   4. Architectural decisions (AD1 – AD55)
   5. Operator personas + 11 use cases
   6. Public site map + frozen URL schema (`https://efe.github.io/bench/`)
   7. Per-page design (overview / 10 scenarios / methodology / chaos / quarterly archive)
   8. Badge taxonomy (11 SVG — 10 scenarios + 1 overall — byte-stable, frozen file names)
   9. Trend graph generator (30-day sparkline, no client-side JS)
  10. JSON API contract (frozen v1.6)
  11. mdbook integration + theme + assets
  12. Quarterly compliance evidence (shared cosign chain with PR #22)
  13. PR #22 SLO cross-link (sidebar + JSON API field)
  14. PR #23 smoke cross-link (overview-header badge)
  15. Crate extension `bench/harness/src/publish/` (PR #21 stub → full impl)
  16. CLI design — `efe-bench-harness publish` 7 new flags (frozen v1.6 § 24.5)
  17. CI workflows (2 new : `bench-publish.yml` + `bench-publish-quarterly.yml`)
  18. gh-pages branch protection requirements (T116 + T122 mitigation)
  19. STABILITY_CONTRACT v1.6 additions
  20. Security model + threat coverage (T116 – T125)
  21. Acceptance criteria (AC1 – AC38)
  22. Tests strategy (golden SVG + golden JSON + mdbook smoke + property)
  23. Risks + mitigations
  24. Rollback strategy
  25. Sequence diagrams (publish flow, quarterly evidence flow, defacement-recovery flow)
  26. Reviewer checklist
  27. Appendix A — CLI flag reference (frozen v1.6 § 24.5)
  28. Appendix B — Frozen URL inventory (every path under `https://efe.github.io/bench/`)
  29. Appendix C — JSON API schema (annotated, with examples)
  30. Appendix D — Badge SVG template (annotated)
  31. Appendix E — gh-pages branch protection rules (GitHub repo settings)
  32. Appendix F — Phase-3 follow-ups (G.1 – G.12)

---

## 0. TL;DR

PR #21 ships the bench harness + 10 scenarios + baseline store + a
**stub** `publish` module that emits placeholder badges.  PR #24
**fills in the stub completely** and ships the **public-facing bench
portal** at `https://efe.github.io/bench/` — the URL that legal cites
in MSAs, that sales puts in proposals, and that customers check before
signing.

PR #24 ships :

- **11 byte-stable SVG badges** (10 scenarios `scenario-S01..S10.svg`
  + 1 aggregate `overall.svg`) — frozen file names in
  STABILITY_CONTRACT v1.6 § 24.1.
- **10 scenario detail pages** at `/scenarios/S01..S10/` — synthesised
  from `summary.json` + `bench_baselines/` + the scenario YAML.
- **JSON API** at `/api/{current,history,scenarios,baselines,version}.json` —
  frozen v1.6 § 24.3, CORS-open, immutable per-day paths.
- **Quarterly evidence chain** : signed PDF + JSON sidecar via cosign
  keyless OIDC (**shared identity regex** with PR #22).
- **Cross-links** : PR #22 SLO badges + JSON in the sidebar ; PR #23
  smoke badge in the overview header.
- **mdbook public site** (no client-side JS — T125 mitigation).
- **2 CI workflows** : `bench-publish.yml` (after nightly bench-full +
  on push:main) and `bench-publish-quarterly.yml` (cron cosign).
- **gh-pages branch protection** requirements documented (T116, T122).
- **STABILITY_CONTRACT v1.6 § 24** (this PR) committing the public-page
  URL schema, badge filenames, JSON API, quarterly path layout, CLI
  surface, and gh-pages protection rules as frozen.

Everything is deterministic given identical bench input ; the public
surface is content-addressable per day (no flap) ; quarterly evidence
is signed + Rekor-witnessed.

---

## 1. Goal + non-goals + scope

### 1.1 Goal

Deliver the **complete, frozen, public, auditable bench portal** at
`https://efe.github.io/bench/` :

  - 11 frozen badge URLs (§ 8),
  - 10 scenario detail pages (§ 7),
  - JSON API at `/api/*.json` (§ 10),
  - Quarterly signed evidence (§ 12),
  - SLO + smoke cross-links (§ 13, § 14),
  - STABILITY_CONTRACT v1.6 § 24 commitment (§ 19).

### 1.2 Non-goals (defer to Phase-3 — Appendix F)

  - **Customer-facing branded landing page** (UX-designed, EFE brand
    treatment) — G.9 from PR #21.
  - **Customer benchmarks-as-a-service** (multi-tenant bench portal) —
    G.7 from PR #21.
  - **Cross-version baseline interpolation** (e.g. v0.6.0 vs v0.7.0
    visual diff with confidence bands) — G.3.
  - **Real-time WebSocket bench stream** (Phase-3 — G.10 from PR #22).
  - **Customer-attestable HSM-signed evidence** (Phase-3 — G.8 from
    PR #22).
  - **Reverse-proxy CDN with private origin** (Cloudflare + edge auth) —
    G.11.
  - **Public bench API rate-limiting** (gh-pages has no rate-limit ;
    Phase-3 may add an edge worker) — G.12.
  - **Multi-language methodology pages** (FR / DE / JA translations) —
    G.4.
  - **Per-customer badge embedding with attribution metadata** — G.5.
  - **Customer-uploadable comparison runs** (compare-my-cluster-to-yours) — G.6.

### 1.3 Explicit scope boundary vs PR #21

| Concern | PR #21 (bench harness) | PR #24 (publish) |
|---|---|---|
| **What's produced** | `bench/results/{run_id}/{scenario}/summary.json` (per-run JSON) | Public site + signed evidence (multi-run aggregation) |
| **Who reads it** | CI workflows + bench operators | Customers + legal + sales + auditors |
| **Trigger** | `bench-quick.yml` (PR label) / `bench-full.yml` (nightly) / `bench-release.yml` (tag) | `bench-publish.yml` (after `bench-full`/`bench-release` completion) + `bench-publish-quarterly.yml` (cron) |
| **Output** | Local artefacts uploaded to GitHub Actions | gh-pages branch + signed PDF in repo |
| **Stability** | v1.3 § 21 (scenario IDs + tolerance + result schema) | **v1.6 § 24** (this PR : URL schema + badge file names + JSON API + quarterly path + CLI + protection rules) |

### 1.4 Explicit scope boundary vs PR #22

| Concern | PR #22 (SLO rollup) | PR #24 (publish bench) |
|---|---|---|
| **Public page** | `https://efe.github.io/slos/` (4 SLOs) | `https://efe.github.io/bench/` (10 scenarios) |
| **Time horizon** | Rolling 30/90 days (SLO compliance) | Per-run + cross-run trend (last 30 daily runs) |
| **Evidence chain** | Quarterly signed PDF (PR #22 § 13) | Quarterly signed PDF (THIS PR § 12 — **same cosign identity regex**) |
| **Cross-link** | Bench badges optionally embedded in `slos/` site | SLO badges embedded in `bench/` sidebar |

The two public surfaces are **independent but cross-linked**.  Visitors land at
`https://efe.github.io/slos/` or `https://efe.github.io/bench/` ; each
page sidebar links to the other.

### 1.5 Explicit scope boundary vs PR #23

PR #23 ships `https://efe.github.io/smoke/badge.svg` (one-bit pass/fail).
PR #24 embeds it in the bench overview header (§ 14).  No state shared ;
the smoke badge is fetched at publish-time as an `<img src>`.

---

## 2. Pre-flight inventory

### 2.1 What PR #21 already ships (stub — TO BE REPLACED)

```
bench/harness/src/publish/
├── mod.rs           # Publisher::publish_badges() emits PR #21 stubs
└── badge.rs         # Badge::from_verdict() — Phase-2 placeholder colour/value
```

PR #21 also already provides :

- **Scenario YAMLs** under `bench/scenarios/S01..S10.yaml` (10 frozen IDs)
- **Baseline JSONs** at `bench_baselines/v0.6.0/L2/*.json` (10 frozen — first commit)
- **Result schema v1** at `bench/schemas/baseline-v1.json`
- **CLI subcommand** `efe-bench-harness publish --source <dir> --output <dir>`
  (calls into the stub module)

### 2.2 What PR #22 already ships (CONSUMED via cross-link)

- 4 SLO badges at `https://efe.github.io/slos/badges/slo-{1..4}-{slug}.svg`
  (referenced via stable URL in the bench sidebar — no fetch at build time)
- JSON API at `https://efe.github.io/slos/current.json` (optionally embedded
  in the bench overview's SLO sidebar via build-time fetch — operator
  opt-in via CLI flag)
- Cosign keyless OIDC identity regex (frozen STABILITY v1.4 § 22.8) —
  PR #24 **shares** the identity to streamline the auditor's flow
  (one verifier command verifies both quarterly chains).

### 2.3 What PR #23 already ships (CONSUMED via embedded badge)

- `https://efe.github.io/smoke/badge.svg` — overall pass/fail badge,
  refreshed on push:main.
- Embedded in the bench overview page header via `<img src>` (no
  fetch at build time ; gh-pages serves both surfaces).

### 2.4 What this PR delivers (NEW)

```
bench/
├── publish/                            # NEW — site source
│   ├── README.md
│   ├── book.toml
│   ├── theme/                           # mdbook custom theme (no JS)
│   │   ├── css/
│   │   │   └── bench.css
│   │   └── index.hbs                    # base template
│   └── src/
│       ├── SUMMARY.md
│       ├── overview.md
│       ├── methodology.md
│       ├── chaos.md
│       ├── scenarios/
│       │   ├── s01-baseline.md
│       │   ├── s02-canary.md
│       │   ├── s03-ramp.md
│       │   ├── s04-chaos-participant-loss.md
│       │   ├── s05-chaos-network-partition.md
│       │   ├── s06-chaos-cert-expiry.md
│       │   ├── s07-chaos-reload-storm.md
│       │   ├── s08-multi-region.md
│       │   ├── s09-soak.md
│       │   └── s10-slo-validation.md
│       ├── quarterly-archive.md
│       ├── api.md
│       └── stability-contract-v1.6.md
└── harness/src/publish/                  # EXTENDED (replaces PR #21 stub)
    ├── mod.rs                            # Publisher orchestration
    ├── badge/
    │   ├── mod.rs                        # 11 byte-stable SVG generators
    │   ├── scenario.rs                   # per-scenario badge
    │   └── overall.rs                    # aggregate badge
    ├── trend/
    │   ├── mod.rs                        # 30-day sparkline SVG generator
    │   └── bucket.rs                     # day-bucketing logic
    ├── site/
    │   ├── mod.rs                        # mdbook orchestration
    │   ├── overview.rs                   # /index.html generator
    │   ├── scenario_page.rs              # per-scenario page generator
    │   ├── methodology.rs                # /methodology.html generator
    │   ├── quarterly_archive.rs          # /quarterly-archive.html generator
    │   └── api_page.rs                   # /api.html (HTML view of JSON API)
    ├── api/
    │   ├── mod.rs                        # /api/*.json serialisers
    │   ├── current.rs                    # /api/current.json
    │   ├── history.rs                    # /api/history.json
    │   ├── scenarios.rs                  # /api/scenarios.json
    │   ├── baselines.rs                  # /api/baselines.json
    │   └── version.rs                    # /api/version.json
    ├── quarterly/
    │   ├── mod.rs                        # quarterly evidence chain
    │   ├── pdf.rs                        # PDF generator (re-uses slos PR #22 module)
    │   ├── json.rs                       # JSON sidecar
    │   └── sign.rs                       # cosign keyless OIDC wrapper
    ├── crosslink/
    │   ├── mod.rs                        # PR #22 SLO + PR #23 smoke cross-link
    │   ├── slo.rs                        # fetch & embed SLO badges/JSON
    │   └── smoke.rs                      # embed smoke badge
    ├── deploy/
    │   ├── mod.rs                        # gh-pages deploy logic
    │   └── ratchet.rs                    # branch-protection sanity check
    └── tests/
        ├── golden_svg.rs                 # byte-stable SVG for every badge
        ├── golden_json.rs                # /api/*.json schema validation
        ├── mdbook_smoke.rs               # mdbook build succeeds
        ├── crosslink_test.rs             # SLO + smoke cross-link integrity
        ├── stability_contract_v16.rs     # frozen v1.6 § 24 surface
        └── data/                          # fixture JSON / SVG goldens
            ├── golden-scenario-S01.svg
            ├── golden-overall-pass.svg
            ├── golden-overall-fail.svg
            ├── golden-current.json
            └── golden-history.json

.github/workflows/
├── bench-publish.yml                     # NEW — on push:main + after bench-full / bench-release
└── bench-publish-quarterly.yml            # NEW — cron Q1/Q2/Q3/Q4 + cosign

docs/
├── EFE_A1_PHASE2_PR24_ARCHITECTURE.md     # THIS DOC
├── STABILITY_CONTRACT.md
│   └── § 24 (v1.6 — Bench public-page surface)  # NEW
└── runbooks/
    ├── bench-publish-defacement.md       # T116 mitigation playbook
    └── bench-publish-quarterly.md        # T121 mitigation playbook

bench/harness/Cargo.toml                    # diff : add printpdf, pulldown-cmark, image
```

### 2.5 What's NOT in this PR

- New scenarios (Phase-3 G.2 — chart-minor regression scenarios).
- Cross-version comparison visualisations (Phase-3 G.3).
- Edge worker / CDN fronting (Phase-3 G.11).

---

## 3. Branch + commit plan

| # | Commit | Scope | Approx LOC | Risk |
|---|---|---|---:|---|
| 1 | `feat(bench-publish): replace PR #21 stub badge with 11 byte-stable SVG generators` | `bench/harness/src/publish/badge/*.rs` | 520 | low |
| 2 | `feat(bench-publish): 30-day sparkline trend SVG generator (UTC-bucketed)` | `bench/harness/src/publish/trend/*.rs` | 280 | low |
| 3 | `feat(bench-publish): JSON API serialisers (5 endpoints frozen v1.6)` | `bench/harness/src/publish/api/*.rs` | 640 | medium — schema must be 100% canonical |
| 4 | `feat(bench-publish): mdbook orchestration + 10 scenario pages` | `bench/harness/src/publish/site/*.rs` + `bench/publish/src/**` | 4 200 | medium — markdown templating |
| 5 | `feat(bench-publish): PR #22 SLO + PR #23 smoke cross-link` | `bench/harness/src/publish/crosslink/*.rs` | 420 | low |
| 6 | `feat(bench-publish): quarterly evidence (PDF + cosign + Rekor)` | `bench/harness/src/publish/quarterly/*.rs` | 720 | high — cosign integration |
| 7 | `feat(bench-publish): gh-pages deploy with branch-protection sanity check` | `bench/harness/src/publish/deploy/*.rs` | 320 | medium |
| 8 | `feat(bench-publish): CLI extension — 7 new flags for `publish` subcommand (frozen v1.6 § 24.5)` | `bench/harness/src/cli.rs` | 280 | low |
| 9 | `ci(bench-publish): bench-publish.yml + bench-publish-quarterly.yml` | `.github/workflows/bench-publish*.yml` | 520 | medium |
| 10 | `docs(bench-publish): STABILITY_CONTRACT v1.6 § 24 + 2 runbooks + this arch doc + public site source` | `docs/**` + `bench/publish/src/**` | 7 480 | low |
| 11 | `test(bench-publish): golden SVG + golden JSON + mdbook smoke + crosslink + frozen v1.6` | `bench/harness/src/publish/tests/**` | 2 880 | low |
| 12 | `chore(bench-publish): gh-pages branch protection rules documented + applied` | `docs/EFE_A1_PHASE2_PR24_ARCHITECTURE.md` Appendix E + repo settings | 360 | medium |

**Total LOC** ≈ 18 620 (within budget of 17 240 + safety overhead).

**Sprint allocation** :
- Sprint W16 (5 days) : commits 1–7 (badges + trend + JSON API + mdbook + crosslink + quarterly + deploy)
- Sprint W17 (5 days) : commits 8–12 (CLI + CI + docs + tests + branch protection)

---

## 4. Architectural decisions (AD1 – AD55)

### 4.1 Public-surface invariants

- **AD1 — Frozen URL schema v1.6.**  Every URL under
  `https://efe.github.io/bench/` listed in Appendix B is locked.
  Renaming requires a major-version bump of STABILITY_CONTRACT.
- **AD2 — Byte-stable badges.**  Each SVG is deterministic given
  the same (verdict, value) input.  No timestamps in body, no
  random IDs.  Tested via golden files (§ 22).
- **AD3 — No client-side JS.**  mdbook theme disables search + copy
  + playground.  T125 mitigation : the public methodology page
  cannot be turned into a phishing target by JS injection.
- **AD4 — Immutable per-day artefacts.**  `/history/{yyyy-mm-dd}.json`
  is write-once ; existing files are never overwritten.  Operators
  who need to amend a day re-publish under a new path
  (`/history/{yyyy-mm-dd}-rev2.json`) with an explicit ERRATUM note.
- **AD5 — Per-hour publish cadence.**  CI cron runs every hour to
  catch out-of-band bench-quick runs.  Bench-full + bench-release
  triggers also fire the workflow on completion.

### 4.2 Badge taxonomy

- **AD6 — 11 badges, closed taxonomy.**  10 scenario badges
  (`scenario-S01..S10.svg`) + 1 overall (`overall.svg`).  Adding a
  12th requires STABILITY major bump.
- **AD7 — Badge file names frozen v1.6 § 24.1.**  See § 8 for the
  full list.
- **AD8 — 3-colour scheme (green / red / grey).**  Same hex codes as
  PR #22 and PR #23 for visual consistency :
  - `#3fb950` = pass
  - `#da3633` = fail
  - `#8b949e` = no-data / indeterminate
- **AD9 — Badge content : `{scenario_id}: {verdict_glyph} {p95_ms}`.**
  Operator-readable at a glance ; tested via golden.
- **AD10 — Overall badge is the worst-of 10.**  `Fail` if any
  scenario fail ; `Warn` if any warn ; `Pass` if all pass ;
  `NoBaseline` only if every scenario is `NoBaseline`.

### 4.3 Trend / sparkline

- **AD11 — 30-day rolling sparkline, UTC-bucketed.**  One data point
  per day from the most recent run in that day's window.
- **AD12 — Sparkline SVG size 200×40, no axis labels.**  Fits in a
  page sidebar.  Tested via golden bytes.
- **AD13 — Sparkline missing-day handling : dashed segment.**  Visual
  distinction between "p95=NaN" and "no run".

### 4.4 JSON API

- **AD14 — 5 frozen endpoints v1.6 § 24.3.**
  `/api/current.json` · `/api/history.json` · `/api/scenarios.json` ·
  `/api/baselines.json` · `/api/version.json`.
- **AD15 — Schema versions frozen v1.**  Adding a key is non-breaking
  (forward-compatible deserialisation, AD16) ; renaming or removing
  is breaking.
- **AD16 — JSON readers ignore unknown fields.**  Implemented via
  serde default ; tested via `unknown_field_ignored_test`.
- **AD17 — CORS-open.**  `Access-Control-Allow-Origin: *` via GitHub
  Pages headers (auto).  T117 mitigation : the API contains zero
  tenant-identifying data.
- **AD18 — Canonical-form JSON.**  Sorted keys + 2-space indent.
  Tested via golden roundtrip.
- **AD19 — Per-history-day file is content-addressable.**  Hash of
  the file body is published in `/api/history.json` as `sha256` per
  entry — verifiers can detect tampering after publish.

### 4.5 mdbook + theming

- **AD20 — mdbook custom theme.**  Forks the default theme to
  remove all JS (search disabled, copy-button removed, playground
  removed).
- **AD21 — Default `light` theme, no `prefer-dark` detection.**
  Pure-HTML, no media-query gating.
- **AD22 — CSP header documented in deploy/.**  `default-src 'self' ;
  script-src 'none' ; img-src 'self' https://efe.github.io ;
  style-src 'self' 'unsafe-inline'`.  T125 mitigation.
- **AD23 — Methodology page is generated from architecture docs.**
  Pulldown-cmark transforms `docs/EFE_A1_PHASE2_PR21_ARCHITECTURE.md`
  into `/methodology.html` — single source of truth.

### 4.6 Quarterly compliance evidence

- **AD24 — Quarterly bench evidence is independent from quarterly
  SLO evidence (PR #22).**  Two separate signed PDFs ; **same cosign
  identity regex** but different workflow files.
- **AD25 — Signing identity regex extended.**
  `^https://github\.com/efe-ai/efe/\.github/workflows/(slo-quarterly|bench-publish-quarterly)\.yml@refs/.+$`.
  The "OR" lets one verifier command verify both chains (AD26).
- **AD26 — Cross-chain verifier.**  `efe-bench-harness evidence verify`
  and `efe-slo-rollup evidence verify` accept either signed PDF — the
  identity regex is checked against the cosign certificate.
- **AD27 — Embedded JSON SHA256 in PDF metadata** (mirror of PR #22
  AD35 — same chain construct).
- **AD28 — Rekor transparency log entry** for every signed PDF
  (auditor verifies offline by re-computing the hash).

### 4.7 gh-pages deploy

- **AD29 — gh-pages branch is protected.**  Force-push disabled ;
  signed commits required ; CODEOWNERS = `@efe/release-eng`.
  T116 + T122 mitigation.
- **AD30 — Deploy is incremental.**  Existing files preserved unless
  the publish job emits a replacement.  Files under `/history/` are
  never deleted.
- **AD31 — concurrency.cancel-in-progress = false for bench-publish.yml.**
  Two simultaneous push:main events ⇒ sequential publish (no race ;
  T122 mitigation).
- **AD32 — Pre-deploy ratchet sanity check.**  Before each deploy,
  the publisher verifies the gh-pages branch state matches the
  expected layout (e.g. `/api/current.json` exists, `/badges/`
  contains the 11 frozen names).  T123 mitigation.

### 4.8 PR #22 SLO + PR #23 smoke cross-link

- **AD33 — SLO badges embedded via stable URL `<img src>`.**  No
  build-time fetch.  T123 mitigation : same gh-pages origin,
  no CDN traversal.
- **AD34 — SLO JSON optionally embedded in `/api/current.json`.**
  Opt-in via CLI flag `--embed-slos` (default off — bench portal
  remains independent from SLO portal).
- **AD35 — Smoke badge embedded in overview header.**  `<img src>` ;
  cache-control inherited from gh-pages.

### 4.9 CLI extension

- **AD36 — 7 new flags on `efe-bench-harness publish` (frozen v1.6 § 24.5).**
  See Appendix A.
- **AD37 — `publish` is idempotent.**  Re-running the command with
  identical inputs produces identical output bytes (badges + JSON +
  HTML).  Tested via two-run-roundtrip.
- **AD38 — `--dry-run` writes to a tempdir + verifies layout.**  No
  gh-pages push.  Used by CI to validate before deploy.

### 4.10 CI + deployment

- **AD39 — bench-publish.yml triggers on push:main + workflow_run
  completion of `bench-full.yml` and `bench-release.yml`.**
- **AD40 — bench-publish-quarterly.yml runs cron `0 5 1 1,4,7,10 *`.**
  (Q1/Q2/Q3/Q4 evidence + cosign).
- **AD41 — Both workflows use OIDC for AWS (if S3 mirror enabled,
  Phase-3 G.11) and OIDC for cosign keyless.**

### 4.11 Threat-model + security

- **AD42 — Public surface carries no tenant-identifying data.**  Allow-
  list of fields in JSON API ; allow-list of labels in badges.
  T117 + T120 mitigation.
- **AD43 — Numeric values are HTML-escaped before SVG interpolation.**
  Defence-in-depth — bench numerics are floats but a bug somewhere
  upstream could allow string injection.  T118 mitigation.
- **AD44 — PR comment markdown HTML-escapes badge filenames.**
  T119 mitigation (mirror of PR #21 AD27).
- **AD45 — Public methodology page has canonical URL meta + Open
  Graph tags.**  Reduces phishing-look-alike risk.  T125.

### 4.12 Crate organisation

- **AD46 — Extension-in-place for now.**  PR #21's
  `bench/harness/src/publish/` module is extended ; **no new crate
  extraction** in this PR.  When the module crosses 3 500 LOC
  (estimated at v0.3.0 / chart v0.8.0), Phase-3 G.1 extracts to
  `bench/publish/` sibling crate.
- **AD47 — Optional sibling site source under `bench/publish/src/**`.**
  Markdown source files for mdbook (separate from Rust source).
- **AD48 — `bench/harness/Cargo.toml` gains 3 deps :**
  `printpdf` (PDF), `pulldown-cmark` (markdown → HTML), and
  `image` (badge logo composition — optional, gated by feature
  `publish-logo`).

### 4.13 Stability + ops

- **AD49 — STABILITY_CONTRACT v1.6 § 24 freezes** :
  - 11 badge file names (§ 24.1)
  - 10 scenario page URLs (§ 24.2)
  - 5 JSON API endpoints (§ 24.3)
  - Quarterly evidence path layout (§ 24.4)
  - 7 new CLI flags on `publish` subcommand (§ 24.5)
  - 4 site-URL invariants (§ 24.6)
  - gh-pages branch protection rules (§ 24.7)
- **AD50 — Schema-versioned JSON API.**  Each endpoint file carries
  `schema_version: 1`.  Phase-3 introduces v2 with strict superset.
- **AD51 — `efe-bench-harness --version` reports stability tier.**
  `efe-bench-harness 0.2.0\n  stability : v1.6 (PR #21 § 21 + PR #24 § 24)`.

### 4.14 Errata / corrections

- **AD52 — Publish ERRATA flow.**  When an operator must amend a
  past day's data, they republish under `/history/{date}-rev{N}.json`
  + add an `ERRATUM` entry to `/api/history.json` with a `replaces`
  pointer + a `reason` field.  Auditor sees the full chain.
- **AD53 — Original day-file is NEVER deleted.**  Even after errata.
  T121 mitigation.

### 4.15 Performance + caching

- **AD54 — `/badges/*.svg` cache-control: no-cache.**  Forces re-fetch
  on every page load.  T124 mitigation : stale badge served as fresh
  is prevented.
- **AD55 — `/history/{yyyy-mm-dd}.json` cache-control: immutable.**
  Per-day files never change ⇒ infinite cache.  Performance + cost.

---

## 5. Operator personas + 11 use cases

### 5.1 Personas

| Persona | Goal | Tool | Output |
|---|---|---|---|
| **Customer (sales prospect)** | "Show me your bench numbers" | Visit `https://efe.github.io/bench/` | 11 badges + 10 detail pages |
| **Sales engineer** | Reference bench in proposal | Stable URL in PDF | Customer-checkable link |
| **Legal counsel** | Cite bench in MSA | Quarterly signed PDF | Verifiable offline |
| **Auditor (3rd party)** | Verify bench integrity | `efe-bench-harness evidence verify quarterly/Q2-2026.pdf` | exit 0 + cosign OK |
| **Customer eng (technical)** | Compare their cluster to EFE's | JSON API → analysis script | machine-readable per-scenario data |
| **Release engineer** | Pre-cut release : will publishing work? | `efe-bench-harness publish --dry-run` | layout validation |
| **Perf engineer** | Trend regression spot-check | Click scenario page → sparkline | visual 30-day trend |
| **Marketing** | Embed badge in blog post | `<img src="https://efe.github.io/bench/badges/overall.svg">` | live-refreshing badge |
| **Security review** | Verify no PII leak | `cargo test --features deny-pii` | green |
| **Compliance officer** | Quarterly evidence for SOC 2 | `quarterly/Q2-2026.pdf` + cosign cert | audit-trail line item |
| **Customer (incident response)** | "Has EFE seen this regression before?" | Search `/api/history.json` | 30-day raw data |

### 5.2 Use cases

1. **UC-1 — Customer lands on `bench/` overview** : sees 11 green badges → confidence.
2. **UC-2 — Customer clicks S01 baseline** : sees 30-day sparkline + tolerance table + last 30 results.
3. **UC-3 — Sales pasted URL in proposal** : customer-side reviewer clicks → no JS, no tracking, no auth needed.
4. **UC-4 — Auditor verifies Q2-2026 evidence** : `efe-bench-harness evidence verify quarterly/Q2-2026.pdf` → cosign OK + SHA matches.
5. **UC-5 — Release-eng pre-cuts v0.7.0** : runs `efe-bench-harness publish --chart-version 0.7.0-rc1 --dry-run --output /tmp/site` → no upload, layout validated.
6. **UC-6 — Customer eng fetches `/api/scenarios.json`** : machine-readable per-scenario verdict for trend analysis script.
7. **UC-7 — Marketing puts badge in blog post** : `https://efe.github.io/bench/badges/overall.svg` refreshes automatically every hour.
8. **UC-8 — Perf-eng investigates S04 regression** : opens S04 page → sparkline shows step-up → clicks recent run → sees full `summary.json` with diff.
9. **UC-9 — Security review** : confirms `cargo test --test publish_no_pii` green (allow-list of fields in JSON API tested).
10. **UC-10 — Defacement detected via monitoring** : gh-pages branch suddenly has unauthorised commit → runbook `bench-publish-defacement.md` triggers (T116).
11. **UC-11 — ERRATA flow** : day 2026-05-15 had a kubeconfig leak in `summary.json.error_summary` — operator republishes as `2026-05-15-rev2.json` + adds ERRATUM entry → original preserved for audit (AD52).

---

## 6. Public site map + frozen URL schema

```
https://efe.github.io/bench/
├── /                                   → overview.html (11 badges + 30-day trend + cross-links to SLO + smoke)
├── /methodology.html                    ← generated from PR #21 arch doc
├── /chaos.html                          ← chaos fault library catalogue
├── /scenarios/
│   ├── s01-baseline.html
│   ├── s02-canary.html
│   ├── s03-ramp.html
│   ├── s04-chaos-participant-loss.html
│   ├── s05-chaos-network-partition.html
│   ├── s06-chaos-cert-expiry.html
│   ├── s07-chaos-reload-storm.html
│   ├── s08-multi-region.html
│   ├── s09-soak.html
│   └── s10-slo-validation.html
├── /quarterly-archive.html              ← list of all signed quarterly evidence
├── /stability-contract-v1.6.html        ← human-readable v1.6 § 24
├── /api/                                ← JSON API (CORS-open)
│   ├── current.json                     ← latest bench-full result
│   ├── history.json                     ← last 30 runs
│   ├── scenarios.json                   ← 10 scenario metadata + latest verdicts
│   ├── baselines.json                   ← published baselines
│   └── version.json                     ← harness + chart + git_sha
├── /badges/                             ← 11 byte-stable SVGs
│   ├── scenario-S01.svg
│   ├── scenario-S02.svg
│   ├── scenario-S03.svg
│   ├── scenario-S04.svg
│   ├── scenario-S05.svg
│   ├── scenario-S06.svg
│   ├── scenario-S07.svg
│   ├── scenario-S08.svg
│   ├── scenario-S09.svg
│   ├── scenario-S10.svg
│   └── overall.svg
├── /history/                            ← daily run snapshots (immutable)
│   ├── 2026-05-15.json                  ← one per day
│   ├── 2026-05-15-rev2.json             ← ERRATUM if any (AD52)
│   ├── 2026-05-16.json
│   └── …
├── /baselines/                          ← mirror of bench_baselines/ per chart-version
│   ├── v0.6.0/L2/
│   │   ├── S01_baseline_100qps_5min.json
│   │   ├── …
│   │   └── S10_slo_validation_30min.json
│   └── v0.7.0/L2/
│       └── …
├── /quarterly/                          ← signed evidence (forever-retention)
│   ├── Q1-2026.{json,pdf,pdf.bundle}
│   ├── Q2-2026.{json,pdf,pdf.bundle}
│   └── …
├── /assets/                             ← CSS / images (no JS)
│   ├── bench.css
│   └── logo.png
└── /robots.txt                          ← `Allow: /` (public)
```

### 6.1 URL invariants (frozen v1.6 § 24.6)

  - All paths use **kebab-case** lowercase (no underscores, no caps).
  - **No query strings** anywhere (gh-pages doesn't support dynamic routing).
  - File extensions explicit (`.html`, `.json`, `.svg`, `.pdf`).
  - **No 301/302 redirects** at this level (Phase-3 G.11 introduces edge worker).

---

## 7. Per-page design (deep dive)

### 7.1 `/` — Overview

Layout (single column, mobile-first) :
```
┌─────────────────────────────────────────────┐
│ EFE Public Bench                            │
│ [smoke ✅] [bench overall ✅]               │  ← PR #23 smoke + PR #24 overall
├─────────────────────────────────────────────┤
│ Latest run :                                 │
│   chart : v0.6.0  run_id : 01J3F…  16:00Z  │
├─────────────────────────────────────────────┤
│ Scenario badges (10) :                       │
│   [S01] [S02] [S03] [S04] [S05]              │
│   [S06] [S07] [S08] [S09] [S10]              │
├─────────────────────────────────────────────┤
│ 30-day sparklines (one per scenario, click) │
├─────────────────────────────────────────────┤
│ SLO sidebar :                                │  ← PR #22 cross-link
│   [SLO-1 ✅] [SLO-2 ✅] [SLO-3 ✅] [SLO-4 ✅]│
├─────────────────────────────────────────────┤
│ Methodology · Quarterly archive · API · v1.6│
└─────────────────────────────────────────────┘
```

### 7.2 `/scenarios/sN-xxx.html` — Per-scenario page

| Section | Content |
|---|---|
| Header | Scenario ID + description + tier + duration |
| Latest verdict | Big colour-coded badge + verdict glyph + p99 |
| Tolerance table | 6 metrics × (target / observed / verdict) |
| 30-day sparkline | SVG inline, no JS |
| Last 30 runs | Table : date / verdict / p95 / link to `summary.json` |
| Methodology link | Anchor to `/methodology.html#sN` |
| Baseline link | `/baselines/v{chart}/L2/{scenario}.json` |

### 7.3 `/methodology.html`

Generated from `docs/EFE_A1_PHASE2_PR21_ARCHITECTURE.md` via
pulldown-cmark.  Sections preserved :
  - § 6 Scenario catalogue
  - § 7 Per-scenario design
  - § 8 Chaos fault library
  - § 11 Result format
  - § 12 Comparison + verdict engine
  - § 13 Baseline management

### 7.4 `/quarterly-archive.html`

Table of all quarterly evidence :
| Quarter | JSON | PDF | Bundle | Verifier command |
|---|---|---|---|---|
| Q1-2026 | `Q1-2026.json` | `Q1-2026.pdf` | `Q1-2026.pdf.bundle` | `efe-bench-harness evidence verify Q1-2026.pdf` |
| Q2-2026 | … | … | … | … |

---

## 8. Badge taxonomy (11 frozen SVGs)

| File | Width | Content | Verdict source |
|---|---|---:|---|
| `/badges/scenario-S01.svg` | dynamic | `S01: ✅ 8.2ms` | latest run for S01 |
| `/badges/scenario-S02.svg` | dynamic | `S02: ✅ 12.1ms` | latest run for S02 |
| `/badges/scenario-S03.svg` | dynamic | `S03: ⚠ 145ms` | latest run for S03 |
| `/badges/scenario-S04.svg` | dynamic | `S04: ✅ 89ms` | latest run for S04 |
| `/badges/scenario-S05.svg` | dynamic | `S05: ✅ 92ms` | latest run for S05 |
| `/badges/scenario-S06.svg` | dynamic | `S06: ✅ 78ms` | latest run for S06 |
| `/badges/scenario-S07.svg` | dynamic | `S07: ✅ 101ms` | latest run for S07 |
| `/badges/scenario-S08.svg` | dynamic | `S08: ✅ 142ms` | latest run for S08 |
| `/badges/scenario-S09.svg` | dynamic | `S09: ✅ 99ms` | latest run for S09 |
| `/badges/scenario-S10.svg` | dynamic | `S10: ✅ pass` | latest run for S10 |
| `/badges/overall.svg` | dynamic | `bench: ✅ pass` | aggregate (worst-of) |

### 8.1 Colour palette (AD8)

```
pass         #3fb950   green
warn         #d29922   amber
fail         #da3633   red
no_baseline  #8b949e   grey
```

### 8.2 Byte-stability (AD2)

The SVG generator is deterministic.  Two runs with identical (verdict, p95)
produce identical bytes.  Tested via `golden_svg.rs` (11 fixtures).

---

## 9. Trend graph generator (sparkline)

```
SVG dimensions : 200 × 40 (no axis labels)
X axis        : 30 days, oldest left, today right
Y axis        : p95 latency in ms (auto-scaled per scenario)
Colour        : same as verdict colour
Dashed segment : day with no run (AD13)
Sample point   : 4×4 px filled circle for each day
```

Implementation : `src/publish/trend/mod.rs` + `bucket.rs`.

---

## 10. JSON API contract (frozen v1.6 § 24.3)

### 10.1 `/api/current.json`

```jsonc
{
  "schema_version": 1,
  "as_of": "2026-05-16T16:00:00Z",
  "chart_version": "0.6.0",
  "harness_version": "0.2.0",
  "run_id": "01J3F7XGGYM5NZ0CWY3Q7RH3JE",
  "git_sha": "abc1234",
  "overall_verdict": "pass",
  "scenarios": {
    "S01_baseline_100qps_5min": {
      "verdict": "pass",
      "p50_ms": 4.1,
      "p95_ms": 8.2,
      "p99_ms": 12.1,
      "error_rate": 0.0001,
      "throughput_qps": 99.8,
      "tolerance_used": { "p95_ms_max_delta_pct": 10 }
    },
    // … S02-S10
  },
  "stability_contract_version": "1.6"
}
```

### 10.2 `/api/history.json` (last 30 daily entries)

```jsonc
{
  "schema_version": 1,
  "entries": [
    {
      "date": "2026-05-16",
      "url": "/history/2026-05-16.json",
      "sha256": "8e91…",     // AD19 — tamper-evidence
      "overall_verdict": "pass",
      "errata_for": null      // AD52 — set to original date when this is a rev
    },
    // …
  ]
}
```

### 10.3 `/api/scenarios.json`

```jsonc
{
  "schema_version": 1,
  "scenarios": {
    "S01_baseline_100qps_5min": {
      "description": "5-min baseline sanity at 100 QPS — quick CI gate",
      "duration": "5m",
      "tier": "quick",
      "chaos_count": 0,
      "latest_verdict": "pass",
      "page": "/scenarios/s01-baseline.html",
      "badge": "/badges/scenario-S01.svg"
    },
    // …
  }
}
```

### 10.4 `/api/baselines.json`

```jsonc
{
  "schema_version": 1,
  "baselines": {
    "v0.6.0": {
      "released_at": "2026-04-01T00:00:00Z",
      "scenarios": [
        { "id": "S01_baseline_100qps_5min", "url": "/baselines/v0.6.0/L2/S01_baseline_100qps_5min.json" },
        // …
      ]
    }
  }
}
```

### 10.5 `/api/version.json`

```jsonc
{
  "schema_version": 1,
  "harness_version": "0.2.0",
  "chart_version": "0.6.0",
  "git_sha": "abc1234",
  "stability_contract": "1.6",
  "build_date": "2026-05-16",
  "published_at": "2026-05-16T16:30:00Z"
}
```

---

## 11. mdbook integration + theme

### 11.1 `book.toml`

```toml
[book]
title = "EFE Public Bench"
authors = ["EFE Platform Team"]
language = "en"
src = "src"

[output.html]
theme = "theme"            # custom theme (no JS)
default-theme = "light"
preferred-dark-theme = "light"
no-section-label = true
git-repository-url = "https://github.com/efe-ai/efe/tree/main/bench"

# T125 mitigation — no client-side JS.
playground.copyable = false
playground.copy-js  = false
search.enable       = false
```

### 11.2 Custom theme (`bench/publish/theme/`)

Forks the default mdbook theme to :
  - Remove `searcher.js` + `book.js`
  - Remove the copy-button
  - Remove the dark-theme toggle
  - Replace the chrome to match EFE brand (Phase-2 stub : minimal CSS)

### 11.3 Page generation flow

```
bench/publish/src/*.md     ← markdown source
       ↓ (mdbook build)
bench/publish/book/*.html  ← static HTML output
       ↓ (publisher copies)
gh-pages branch            ← serves at https://efe.github.io/bench/
```

---

## 12. Quarterly compliance evidence

### 12.1 Two parallel chains (PR #22 + PR #24)

Both chains share the same cosign keyless OIDC identity regex
(extended in AD25).  The auditor's flow is :

```bash
# Verify Q2-2026 SLO evidence (PR #22)
efe-slo-rollup evidence verify slos/v0.6.0/quarterly/Q2-2026.pdf

# Verify Q2-2026 bench evidence (PR #24)
efe-bench-harness evidence verify bench/quarterly/Q2-2026.pdf
```

Both commands accept either PDF (the identity regex matches either
workflow).  This streamlines the compliance review.

### 12.2 PDF layout (8 sections, ~ 12 pages)

| Section | Content |
|---|---|
| 1. Cover | Period · chart version · audit-trail · Rekor entry |
| 2. Methodology | Reference to PR #21 arch doc + STABILITY_CONTRACT v1.6 |
| 3. Per-scenario summary (S01–S10) | 10 sub-sections : verdict + tolerance + 90-day p95 |
| 4. Chaos events log | Every chaos fault applied in the quarter |
| 5. Baseline changes | Diff of `bench_baselines/v*` files committed in the quarter |
| 6. Errata | Any ERRATUM published (AD52) |
| 7. Cross-chain reference | Pointer to PR #22 SLO evidence Q2-2026 |
| 8. Signature page | Cosign + Rekor UUID + JSON SHA256 |

### 12.3 Cosign chain (AD24–AD28)

```
github-workflow (bench-publish-quarterly.yml)
       │
       ├── OIDC ──► Fulcio (sigstore CA)
       │      └─ ephemeral cert
       │
       ├── cosign sign-blob --bundle Q2-2026.pdf.bundle Q2-2026.pdf
       │
       └── Rekor (transparency log) ◄── append entry
                  └─ public verifier : `efe-bench-harness evidence verify`
```

---

## 13. PR #22 SLO cross-link

### 13.1 Sidebar badges (AD33)

The overview page sidebar embeds 4 SLO badges via stable URL :

```html
<aside class="slo-sidebar">
  <h3>SLO compliance</h3>
  <a href="https://efe.github.io/slos/"><img src="https://efe.github.io/slos/badges/slo-1-availability.svg"></a>
  <a href="https://efe.github.io/slos/"><img src="https://efe.github.io/slos/badges/slo-2-latency-p95.svg"></a>
  <a href="https://efe.github.io/slos/"><img src="https://efe.github.io/slos/badges/slo-3-recovery-time.svg"></a>
  <a href="https://efe.github.io/slos/"><img src="https://efe.github.io/slos/badges/slo-4-replication-lag.svg"></a>
</aside>
```

### 13.2 Optional JSON embed (AD34)

When CLI flag `--embed-slos` is set, the publisher fetches
`https://efe.github.io/slos/current.json` at build time and embeds a
subset under `current.json.slo_snapshot`.  Default OFF.

---

## 14. PR #23 smoke cross-link

### 14.1 Overview header (AD35)

```html
<header class="bench-header">
  <a href="https://efe.github.io/smoke/"><img src="https://efe.github.io/smoke/badge.svg" alt="smoke"></a>
  <img src="/badges/overall.svg" alt="bench overall">
</header>
```

Two badges side-by-side : smoke (merge gate health) + bench (perf health).
Customer sees both at a glance.

---

## 15. Crate extension `bench/harness/src/publish/`

Module map :

```rust
// bench/harness/src/publish/mod.rs

pub mod badge;     // 11 byte-stable SVG generators
pub mod trend;     // 30-day sparkline
pub mod site;      // mdbook orchestration
pub mod api;       // 5 JSON serialisers
pub mod quarterly; // signed evidence
pub mod crosslink; // SLO + smoke embedding
pub mod deploy;    // gh-pages push

pub struct Publisher {
    pub source: PathBuf,            // bench/publish/src/
    pub output: PathBuf,             // gh-pages target
    pub bench_results: PathBuf,      // bench/results/
    pub bench_baselines: PathBuf,    // bench_baselines/
    pub chart_version: String,
    pub embed_slos: bool,
    pub embed_smoke: bool,
}

impl Publisher {
    pub async fn publish(&self) -> BenchResult<PublishReport> { … }
    pub async fn publish_quarterly(&self, quarter: &str) -> BenchResult<()> { … }
    pub fn dry_run(&self) -> BenchResult<DryRunReport> { … }
}
```

The publisher is **idempotent** (AD37) — re-running with identical
inputs produces byte-identical output.

---

## 16. CLI design

### 16.1 `efe-bench-harness publish` (extended)

```
USAGE :
  efe-bench-harness publish [OPTIONS]

OPTIONS (frozen v1.6 § 24.5) :

PR #21 inherited :
  --output <PATH>              gh-pages target directory
  --source <PATH>              mdbook source (default = bench/publish/src)

PR #24 new (7 flags) :
  --bench-results <PATH>       Directory of bench/results/{run_id}/{scenario}/summary.json (default = bench/results)
  --bench-baselines <PATH>     Directory of bench_baselines/v*/L2/*.json (default = bench_baselines)
  --chart-version <VERSION>    Chart version under publish (default = detect from latest baseline)
  --embed-slos                 Embed PR #22 SLO snapshot in /api/current.json (default = off)
  --embed-smoke                Embed PR #23 smoke badge in overview header (default = on)
  --dry-run                    Write to tempdir + verify layout (no gh-pages push)
  --history-window-days <N>    Number of days of history to expose (default = 30)
```

### 16.2 `efe-bench-harness publish-quarterly`

```
USAGE :
  efe-bench-harness publish-quarterly [OPTIONS]

OPTIONS :
  --period <QUARTER>           Q1-2026 / Q2-2026 / … (required)
  --chart-version <VERSION>    Chart version (required)
  --output <PATH>              gh-pages target
  --cosign-identity <URI>      OIDC issuer (default = ambient GH OIDC)
  --cosign-disabled            Skip signing (CI uses keyless OIDC ; local debug)
  --dry-run                    No PDF write, no signing
```

---

## 17. CI workflows

### 17.1 `bench-publish.yml`

```yaml
name: bench-publish

on:
  push:
    branches: [main]
  workflow_run:
    workflows: ["bench-full", "bench-release"]
    types: [completed]
  schedule:
    - cron: "0 * * * *"   # hourly safety-net

concurrency:
  group: bench-publish
  cancel-in-progress: false   # AD31

permissions:
  contents: write             # gh-pages
  id-token: write             # OIDC

jobs:
  publish:
    runs-on: ubuntu-22.04
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install mdbook
        run: cargo install mdbook --version "^0.4" --locked
      - name: Build harness
        run: cargo build --release -p efe-bench-harness
      - name: Generate public site
        run: |
          ./target/release/efe-bench-harness publish \
              --chart-version ${{ vars.BENCH_CHART_VERSION }} \
              --output /tmp/bench-pages \
              --embed-smoke
      - name: Deploy to gh-pages
        uses: peaceiris/actions-gh-pages@v4
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: /tmp/bench-pages
          destination_dir: bench
          keep_files: true
          force_orphan: false
```

### 17.2 `bench-publish-quarterly.yml`

Triggered cron `0 5 1 1,4,7,10 *` (Q boundary + 1 day grace).  Uses
cosign keyless OIDC + Rekor.  Opens a PR adding the signed evidence.

---

## 18. gh-pages branch protection

See Appendix E for the **full repo-settings configuration** (must be
applied by `@efe/release-eng` admin before this PR merges).

Key rules :
  - Force-push : **disabled** (T116, T122).
  - Signed commits : **required** (T116).
  - CODEOWNERS for `bench/**` and `slos/**` : `@efe/release-eng`.
  - PR review : **2 approvals** required for non-CI edits.
  - Linear history : **enforced**.

---

## 19. STABILITY_CONTRACT v1.6 § 24

```markdown
## 24. Bench public-page surface — frozen v1.6 (PR #24)

### 24.1  11 badge file names (closed taxonomy)
   `/badges/scenario-S01.svg` … `/badges/scenario-S10.svg`
   `/badges/overall.svg`

### 24.2  10 scenario page URLs
   `/scenarios/s01-baseline.html` … `/scenarios/s10-slo-validation.html`

### 24.3  5 JSON API endpoints
   `/api/current.json` `/api/history.json` `/api/scenarios.json`
   `/api/baselines.json` `/api/version.json`
   (each carries `schema_version: 1` ; forward-compat on unknown keys)

### 24.4  Quarterly evidence path layout
   `/quarterly/Q{1..4}-{yyyy}.{json,pdf,pdf.bundle}`

### 24.5  CLI surface (7 new flags on `publish` + new `publish-quarterly`)
   See PR #24 § 16.

### 24.6  Site-URL invariants
   - kebab-case lowercase
   - no query strings
   - explicit file extensions
   - no 301/302 redirects

### 24.7  gh-pages branch protection
   See PR #24 Appendix E.

### 24.8  Cosign keyless OIDC identity regex (shared with PR #22 v1.4 § 22.8)
   `^https://github\.com/efe-ai/efe/\.github/workflows/(slo-quarterly|bench-publish-quarterly)\.yml@refs/.+$`
```

---

## 20. Security model + threat coverage

| ID | Threat | Severity | Mitigation |
|---|---|---|---|
| T116 | gh-pages defacement via leaked `GITHUB_TOKEN` | SEV-2 | Branch protection (AD29) + signed commits + CODEOWNERS |
| T117 | Cross-tenant data leak via public badge URL | SEV-2 | AD42 — allow-list of fields ; no tenant data in JSON API |
| T118 | SVG injection via baseline numeric values | SEV-3 | AD43 — numeric values HTML-escaped + XML-escaped before SVG interpolation |
| T119 | PR comment injection via badge filename | SEV-3 | AD44 — filenames frozen v1.6 § 24.1, no operator-controlled interpolation |
| T120 | Public page reveals internal IP / hostname | SEV-2 | AD42 — allow-list of fields ; lint rule rejects RFC-1918 / private hostnames in JSON output |
| T121 | Quarterly evidence revocation handling | SEV-3 | AD52 — ERRATUM flow ; original PDF preserved + Rekor entry remains for audit |
| T122 | gh-pages force-push race on concurrent CI | SEV-3 | AD31 — `concurrency.cancel-in-progress: false` ; sequential publish |
| T123 | Public CDN cache poisoning | SEV-2 | AD32 — pre-deploy ratchet ; gh-pages is git-backed, not CDN-backed |
| T124 | Stale badge served as fresh | SEV-3 | AD54 — `cache-control: no-cache` on /badges/*.svg |
| T125 | Phishing via public methodology page | SEV-2 | AD22 (CSP) + AD20 (no JS) + AD45 (canonical URL meta + OG tags) |

---

## 21. Acceptance criteria (AC1 – AC38)

### 21.1 Site generation

  - **AC1** — `efe-bench-harness publish --dry-run` emits 11 badges, 10 scenario pages, 5 JSON files, 1 overview, 1 methodology, 1 chaos, 1 quarterly archive.
  - **AC2** — `efe-bench-harness publish` is idempotent (two runs ⇒ byte-identical output).
  - **AC3** — Generated SVGs match golden bytes (11 fixtures).
  - **AC4** — Generated JSON files match golden bytes (5 fixtures).
  - **AC5** — Generated HTML contains no `<script>` tags.
  - **AC6** — CSP header present in all HTML files.

### 21.2 Frozen surface (STABILITY v1.6 § 24)

  - **AC7** — 11 badge file names match § 24.1 exactly.
  - **AC8** — 10 scenario page URLs match § 24.2 exactly.
  - **AC9** — 5 JSON API endpoints match § 24.3 exactly.
  - **AC10** — Quarterly path matches § 24.4 exactly.
  - **AC11** — CLI surface matches § 24.5 (7 new flags + new subcommand).
  - **AC12** — Site URL invariants enforced (kebab-case, no query strings, explicit extensions).

### 21.3 Cross-link integrity

  - **AC13** — SLO sidebar embeds 4 PR #22 badges via stable URL.
  - **AC14** — Smoke badge embedded in overview header (when `--embed-smoke`).
  - **AC15** — `--embed-slos` fetches `https://efe.github.io/slos/current.json` and embeds under `current.json.slo_snapshot`.

### 21.4 Trend + sparkline

  - **AC16** — 30-day sparkline SVG is 200×40 dimensions.
  - **AC17** — Missing-day handling uses dashed segment.
  - **AC18** — Sparkline byte-stable across two runs.

### 21.5 Quarterly evidence

  - **AC19** — `publish-quarterly` produces PDF + JSON + bundle.
  - **AC20** — PDF embeds SHA256 of JSON sidecar in metadata.
  - **AC21** — `evidence verify` accepts the bench Q-PDF using the shared OIDC identity regex.
  - **AC22** — Cross-chain verifier : `efe-slo-rollup evidence verify` also accepts the bench PDF (and vice-versa).

### 21.6 ERRATA flow

  - **AC23** — Re-publishing a day with `--errata-for 2026-05-15` writes `/history/2026-05-15-rev2.json` and appends to `/api/history.json` with `replaces: "2026-05-15"`.
  - **AC24** — Original day-file is preserved (not deleted).

### 21.7 CI integration

  - **AC25** — `bench-publish.yml` runs on push:main + after bench-full / bench-release completion.
  - **AC26** — Hourly cron acts as safety net.
  - **AC27** — `concurrency.cancel-in-progress: false` prevents race.
  - **AC28** — gh-pages deploy uses `peaceiris/actions-gh-pages@v4` with `keep_files: true`.
  - **AC29** — `bench-publish-quarterly.yml` triggers on cron `0 5 1 1,4,7,10 *`.
  - **AC30** — Quarterly workflow opens a PR adding the signed evidence (not auto-merge).

### 21.8 Security

  - **AC31** — `--deny-pii` feature compile-time-fails if any forbidden hostname/IP pattern appears in JSON API output.
  - **AC32** — Numeric values XML-escaped before SVG interpolation (T118).
  - **AC33** — gh-pages branch protection rules enforced via repo settings (verified via `gh api repos/efe-ai/efe/branches/gh-pages/protection`).
  - **AC34** — CSP header present in deployed HTML (verified post-deploy via curl).

### 21.9 Operability

  - **AC35** — `efe-bench-harness publish --dry-run --output /tmp` succeeds without network.
  - **AC36** — `efe-bench-harness --version` reports v1.6 § 24.
  - **AC37** — Mdbook build smoke test passes (verifies SUMMARY.md structure).
  - **AC38** — Frozen v1.6 § 24 test suite passes (8 tests).

---

## 22. Tests strategy

| Layer | Path | Count | Purpose |
|---|---|---|---|
| Golden SVG | `tests/golden_svg.rs` | 11 | Byte-stable badges (10 scenarios + overall) |
| Golden JSON | `tests/golden_json.rs` | 5 | Each `/api/*.json` matches fixture |
| Mdbook smoke | `tests/mdbook_smoke.rs` | 3 | SUMMARY.md valid ; 10 scenario pages present ; HTML has no `<script>` |
| Crosslink | `tests/crosslink_test.rs` | 6 | SLO badge URL embedded ; smoke badge URL embedded ; `--embed-slos` flow |
| Trend sparkline | `tests/trend_property.rs` | 6 | proptest : 30-day window edge cases (missing days, all-pass, all-fail) |
| Quarterly | `tests/quarterly_test.rs` | 5 | PDF + cosign + Rekor verify |
| ERRATA | `tests/errata_test.rs` | 3 | AD52/AD53 — original preserved + replacement linked |
| Frozen v1.6 | `tests/stability_contract_v16.rs` | 8 | Locks the v1.6 § 24 surface |
| CLI golden | `tests/publish_cli_golden.rs` | 7 | 7 new flags surface frozen |
| **Total** | | **~ 54 new tests** | |

---

## 23. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Cosign keyless OIDC fails | LOW | HIGH | Fallback to manual signing + alarm |
| gh-pages branch corrupted | LOW | HIGH | Branch protection + Rekor log preserves history |
| mdbook build flakes | MED | LOW | Hourly cron retries |
| JSON schema drift undetected | LOW | HIGH | Golden tests + frozen v1.6 |
| Quarterly evidence revoked | LOW | MED | ERRATA flow (AD52) — original preserved |
| Customer-side cache stale | MED | LOW | AD54 — `cache-control: no-cache` on badges |
| Public methodology becomes phishing target | LOW | MED | AD22 (CSP) + AD20 (no JS) + canonical URL meta |

---

## 24. Rollback strategy

### 24.1 Code rollback

`git revert` the merge commit ; CI workflows remain harmless (no new
deployments).  Existing gh-pages content unaffected.

### 24.2 Data rollback (gh-pages)

- **Daily files** (`/history/*.json`) : write-once, no rollback needed.
- **Badges + current.json** : auto-replaced on next publish run.
- **Quarterly evidence** : signed + on Rekor ; cannot rollback ; use ERRATA flow (AD52).

### 24.3 Branch-protection rollback

If an emergency unblock is needed, release-eng admin temporarily
relaxes branch protection (logged in GitHub audit) + fixes the
underlying issue + restores protection.

---

## 25. Sequence diagrams

### 25.1 Publish flow (on push:main)

```
push:main ──► bench-publish.yml
              │
              ├── build harness (~15 s)
              ├── install mdbook (~10 s)
              ├── efe-bench-harness publish (~30 s)
              │     ├── read bench/results/*/summary.json
              │     ├── read bench_baselines/v*/L2/*.json
              │     ├── render 11 badges
              │     ├── render 30-day sparklines
              │     ├── render 10 scenario pages (mdbook)
              │     ├── render 5 JSON API files
              │     └── verify gh-pages ratchet
              ├── peaceiris/actions-gh-pages (~5 s)
              └── done (~60 s total)
```

### 25.2 Quarterly evidence flow

```
cron 0 5 1 Q-month *
       │
       ▼
bench-publish-quarterly.yml
       │
       ├── efe-bench-harness publish-quarterly --period Q2-2026
       │     ├── aggregate 90-day data
       │     ├── render PDF + JSON sidecar
       │     ├── embed SHA256 in PDF metadata
       │     ├── cosign sign-blob → Rekor entry
       │     └── peaceiris/actions-gh-pages (upload PDF + bundle)
       │
       └── peter-evans/create-pull-request (opens PR adding the evidence)
```

### 25.3 Defacement recovery flow

```
Monitoring detects unauthorised gh-pages commit
       │
       ▼
SRE on-call pages release-eng
       │
       ├── Identify unauthorised commit (`git log gh-pages`)
       ├── Revert to last known-good commit
       ├── Force-push (logged in GitHub audit)
       ├── Restore branch protection if it was relaxed
       └── File post-mortem within 14 days
```

---

## 26. Reviewer checklist

| Check | Status |
|---|---|
| 11 badge file names frozen in v1.6 § 24.1 | ☐ |
| 10 scenario page URLs frozen in v1.6 § 24.2 | ☐ |
| 5 JSON API endpoints frozen in v1.6 § 24.3 | ☐ |
| Quarterly path frozen in v1.6 § 24.4 | ☐ |
| 7 new CLI flags frozen in v1.6 § 24.5 | ☐ |
| 4 site-URL invariants enforced | ☐ |
| gh-pages branch protection rules applied | ☐ |
| Cosign identity regex extended (shared with PR #22) | ☐ |
| All HTML pages free of `<script>` tags | ☐ |
| CSP header in deployed HTML | ☐ |
| 54 new tests green | ☐ |
| Frozen v1.6 § 24 test suite passes | ☐ |
| Cross-chain verifier accepts both SLO and bench PDFs | ☐ |
| mdbook builds successfully | ☐ |
| `--dry-run` succeeds without network | ☐ |
| Clippy `-D warnings` clean | ☐ |
| Release build green | ☐ |
| All 10 threats T116-T125 have explicit mitigations referenced | ☐ |

---

## 27. Appendix A — CLI flag reference

(alphabetical, with default + envvar + frozen-or-not)

```
--bench-baselines <PATH>       (no def) env: EFE_BENCH_BASELINES         (frozen)
--bench-results <PATH>         (no def) env: EFE_BENCH_RESULTS           (frozen)
--chart-version <VERSION>      (no def) env: EFE_CHART_VERSION           (frozen)
--cosign-disabled              false                                    (frozen)
--cosign-identity <URI>        (no def) env: COSIGN_IDENTITY             (frozen)
--dry-run                      false                                    (frozen)
--embed-slos                   false                                    (frozen)
--embed-smoke                  true                                     (frozen)
--history-window-days <N>      30                                       (frozen)
--output <PATH>                (no def)                                  (frozen)
--period <QUARTER>             (no def — for publish-quarterly)          (frozen)
--source <PATH>                bench/publish/src                         (frozen)
```

---

## 28. Appendix B — Frozen URL inventory

(See § 6 — every path is locked v1.6 § 24)

---

## 29. Appendix C — JSON API schema (annotated)

See `bench/schemas/{current,history,scenarios,baselines,version}-v1.json`
(committed alongside the implementation).

---

## 30. Appendix D — Badge SVG template

(byte-stable template ; see § 8 for content rules)

---

## 31. Appendix E — gh-pages branch protection rules

Repository settings (applied by `@efe/release-eng` admin) :

```
Branches → gh-pages → Branch protection rule :
  ✅ Require a pull request before merging
     ✅ Require approvals : 2
     ✅ Require review from Code Owners
  ✅ Require status checks to pass
     ✅ Require branches to be up to date
  ✅ Require signed commits
  ✅ Require linear history
  ✅ Do not allow bypassing
  ❌ Allow force pushes (DISABLED)
  ❌ Allow deletions (DISABLED)
  Restrict who can push : @efe-ai/release-eng
```

CODEOWNERS file (`.github/CODEOWNERS`) :

```
bench/**                @efe-ai/release-eng @efe-ai/perf-eng
slos/**                 @efe-ai/release-eng @efe-ai/sre
smoke/**                @efe-ai/release-eng
.github/workflows/**    @efe-ai/release-eng @efe-ai/security
```

---

## 32. Appendix F — Phase-3 follow-ups (G.1 – G.12)

| # | Title | Brief |
|---|---|---|
| G.1 | Extract `bench/publish/` sibling crate | When LOC > 3 500 |
| G.2 | Cross-chart-minor regression scenarios | New scenarios per chart-minor |
| G.3 | Cross-version baseline interpolation | Visual diff with confidence bands |
| G.4 | Multi-language methodology pages | FR / DE / JA |
| G.5 | Per-customer badge attribution | Embed metadata |
| G.6 | Customer-uploadable comparison runs | Compare-my-cluster |
| G.7 | Bench-as-a-Service (hosted) | Multi-tenant portal |
| G.8 | HSM-signed evidence | Phase-3 hardening |
| G.9 | Customer-facing branded landing | UX-designed |
| G.10 | Real-time WebSocket bench stream | Live feed |
| G.11 | Edge worker CDN | Cloudflare front |
| G.12 | Public bench API rate-limiting | Edge-worker enforced |

---

## End of architecture

This document is **frozen** — any change to badge file names, page
URLs, JSON API endpoints, quarterly path, CLI flags, or gh-pages
protection rules requires :

1. A new architecture PR (e.g. PR #26 or later),
2. A bump of STABILITY_CONTRACT major version,
3. A migration plan covering existing gh-pages content + Rekor entries.

The implementation (commits 1–12 of § 3) follows this spec exactly.
Any divergence is a contract violation and must be filed as an issue
referencing this doc.
