# EFE-A1 Phase 2 — PR #15 : `helm-meta-swarm-lint` workflow

| Field | Value |
|---|---|
| **PR** | #15 (Sprint A1 Phase 2 / P2.6) |
| **Branch** | `efe-a1/p2-pr15-helm-meta-swarm-lint` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 1× meta-swarm + 2× sre + 1× platform-deploy + 1× sec + 1× ci-ops |
| **Depends on** | PR #13 (Helm sub-chart skeleton), PR #14 (Helm values + templates production-grade) |
| **Blocks** | PR #16 (mTLS provisioning) only if lint surface needs additional Rego rules for cert-manager primitives |
| **L2 gate** | G12 (PR-gated Helm chart correctness — every change to `charts/efe-stack/charts/efe-meta-swarm/**` passes 14 lint stages before merge) |
| **Threat IDs in scope** | T29 (supply-chain : un-pinned lint tools), T30 (silent rule drift : tool upgrades that change semantics), T31 (false-positive fatigue), T32 (lint bypass via admin override), T33 (PR comment spam), T34 (SBOM tampering), T35 (concurrency leaks on stale runners) |
| **Roadmap reference** | EFE_A1_PHASE2_ROADMAP.md § C8 |
| **CVSS bandwidth** | All ≤ SEV-3 ; PR introduces only CI surface, zero runtime change |

---

## Table of contents

   0. TL;DR
   1. Mission + non-goals
   2. Pre-flight : current CI state (PR #14 workflow recap)
   3. Why a SECOND workflow (not amend the existing one)
   4. Branch + commit plan (6 commits)
   5. Architectural decisions (AD1–AD30)
   6. Lint tool taxonomy (14 stages, who-owns-what)
   7. Layout : files added + modified
   8. Workflow file structure (top-level)
   9. Concurrency + cancellation model
  10. Cache strategy
  11. Reusable workflow contract (`workflow_call`)
  12. Job 01 — bootstrap (checkout + setup-helm + cache restore)
  13. Job 02 — `yamllint` + `shellcheck` + `pre-commit`
  14. Job 03 — `helm lint --strict`
  15. Job 04 — `chart-testing` (`ct lint`)
  16. Job 05 — `values.schema.json` self-validation (ajv + meta-schema)
  17. Job 06 — `helm-docs` README sync check
  18. Job 07 — `kubeconform` multi-K8s-version matrix (1.28 / 1.29 / 1.30 / 1.31)
  19. Job 08 — Conftest (OPA Rego) custom EFE policies
  20. Job 09 — Trivy IaC scan (SARIF → GitHub Code Scanning)
  21. Job 10 — Checkov scan
  22. Job 11 — Polaris severity-tiered audit
  23. Job 12 — ArtifactHub metadata lint (`ah lint`)
  24. Job 13 — SBOM generation (Syft → CycloneDX) + attestation
  25. Job 14 — PR sticky comment summary
  26. Custom Rego policy library (`policies/rego/efe/`)
  27. `.yamllint` configuration
  28. `.pre-commit-config.yaml` configuration
  29. Renovate integration (tool version tracking)
  30. Branch protection contract
  31. Workflow timing budgets (per-job + total)
  32. Failure modes
  33. Runbook (operator triage by error class)
  34. Threat-model coverage (T29-T35)
  35. Tests strategy (this workflow tests itself)
  36. Acceptance criteria (AC1–AC35)
  37. Risks + mitigations
  38. Rollback strategy
  39. Sequence diagrams (PR push → lint completion)
  40. Reviewer checklist
  41. Appendix A — tool version pinning table
  42. Appendix B — environment variable matrix
  43. Appendix C — Rego rule index

---

## 0. TL;DR

PR #14 shipped a production-grade Helm sub-chart but its CI workflow
(`.github/workflows/helm-meta-swarm.yml`) is a "rendering + roundtrip
battery", not a lint suite.  It catches **structural** drift (golden
snapshot diff) and **manifest semantic** drift (Rust roundtrip), but
NOT :

  - K8s anti-patterns (privileged container, missing securityContext,
    host-network) — Polaris is wired with `severity=danger` only ; we
    miss `warning` + `notice` tiers.
  - Multi-K8s-version compatibility — kubeconform runs only on `1.28.0`.
  - Custom organization-level invariants (e.g. "every resource MUST carry
    `efe.io/tenant`", "no `:latest` image tags") — currently enforced by
    `helm-unittest`, NOT by policy-as-code (so a NEW template without
    a unit test escapes detection).
  - Supply-chain provenance — no SBOM, no signing.
  - JSON Schema sanity — we validate **values against the schema** but
    NOT the **schema against its own meta-schema** ; a corrupted
    `values.schema.json` silently allows anything.
  - Documentation drift — `README.md` table of values gets stale across
    PRs ; no `helm-docs` gate.
  - Lint-style — no `yamllint`, no `shellcheck` for hook scripts, no
    `pre-commit` hooks generated for local devs.

PR #15 adds a dedicated workflow `.github/workflows/helm-meta-swarm-lint.yml`
with **14 parallel-where-possible jobs** that close all the gaps above.
Critical design points :

  - **Separate file** from `helm-meta-swarm.yml` (AD1) : lint is fast
    (≤ 5 min wall-clock), end-to-end test is slow (≤ 15 min) ; we want
    independent failure surfaces and independent caching.
  - **Reusable workflow** via `workflow_call` (AD12) : the umbrella
    chart's PR workflow `helm-stack-test.yml` invokes ours, so umbrella
    PRs that touch only this sub-chart get the same lint coverage.
  - **Custom Rego policies** in `policies/rego/efe/` (AD8, AD18, AD27,
    AD29) enforce 11 EFE-specific invariants that no off-the-shelf tool
    knows about (e.g. `efe.io/tenant` label, `replicaCount: 1` lock,
    SIGHUP-driven reload requires `automountServiceAccountToken: false`
    on runtime pods, hook Jobs require their own SA, etc.).
  - **Sticky PR comment** (AD13) surfaces the lint summary as one tidy
    table on the PR conversation — operators don't have to dig into
    GitHub Actions logs.
  - **Renovate-tracked tool versions** (AD3, AD26) : every linter image
    is pinned by SHA + tag, and Renovate auto-PRs upgrades that flow
    through this same lint suite.
  - **Branch protection contract** (§ 30) : exactly 5 of the 14 jobs are
    "required" for merge ; the others are "informational" but visible.
  - **Bypass** is disabled (AD25) : no `[skip lint]` magic commit
    string ; admin override exists but requires a separate
    `repo-admin` GitHub role + leaves an audit trail.

35 acceptance criteria (§ 36).  30 architectural decisions (§ 5).  Zero
runtime change to the meta-swarm controller / participants — this PR is
a pure CI feature.

---

## 1. Mission + non-goals

### Mission

Ship a deterministic, low-false-positive, fast-feedback lint workflow
that fires on every PR touching `charts/efe-stack/charts/efe-meta-swarm/**`
(or `policies/rego/efe/**`, or `.github/workflows/helm-meta-swarm-lint.yml`
itself).  The workflow MUST :

  1. Catch every off-the-shelf K8s anti-pattern within 5 minutes of push.
  2. Enforce EFE-specific invariants encoded as Rego policies (idempotent,
     reviewable, versionable).
  3. Produce one human-readable summary (sticky comment) plus
     machine-readable SARIF (uploaded to GitHub Code Scanning).
  4. Block merge when ANY of the 5 required gates fails.
  5. Stay green on the PR #14 head-of-main scenarios (no false positives
     on the 6 kovant scenarios + 4 example overlays).
  6. Be reusable by the umbrella chart workflow with a stable
     `workflow_call` interface.
  7. Be locally reproducible : `pre-commit run --all-files` runs ~90%
     of the same checks before the developer pushes.
  8. Be reproducible deterministically — same input → same output
     verdict, no flaky network probes.

### Non-goals

  - **Not** a replacement for `helm-meta-swarm.yml` (PR #14).  That
    workflow stays — it's the integration battery (golden diff +
    cross-version + Rust roundtrip + parent wiring).  PR #15's
    `lint` workflow runs in parallel as a faster feedback surface.
  - **Not** a runtime policy enforcer.  This is **pre-merge** CI ; we
    do NOT install Kyverno / OPA Gatekeeper on the cluster.  (Kyverno
    is on the EFE roadmap as a separate M-series workstream.)
  - **Not** a CVE scanner for the meta-swarm binary itself.  Image scans
    are an upstream-build responsibility ; we scan only the chart.
  - **Not** a SLSA / Sigstore signing pipeline.  We DO emit a SBOM
    (AD23) but signing the chart artifact is M-series scope.
  - **Not** a multi-arch matrix.  All linters run on `linux/amd64`
    ubuntu-22.04 ; the chart itself is arch-neutral (no binary
    dependencies on a specific arch).
  - **Not** a place to enforce coding style on Rego policies themselves
    (that's `opa fmt` ; if needed it'll be a tiny addition in PR #16).

---

## 2. Pre-flight : current CI state (PR #14 workflow recap)

`.github/workflows/helm-meta-swarm.yml` (after PR #14 audit pass)
contains 7 jobs :

```text
┌───────────────────────────────────────────────────────────────────┐
│ helm-meta-swarm.yml — integration battery (PR #14 head-of-main)   │
├──────────────────┬────────────────────────────────────────────────┤
│ lint-and-template│  helm lint + helm template × 6 scenarios       │
│ kubeconform      │  3 of 6 scenarios on K8s 1.28.0 only           │
│ golden-snapshot  │  diff against tests/golden/*.yaml              │
│ manifest-roundtr │  Rust cargo test manifest_roundtrip × 4 flavor │
│ parent-wiring    │  efe-server EFE_META_SWARM_* env propagation   │
│ helm-unittest    │  11 suites × ~93 assertions                    │
│ polaris          │  severity=danger gate, 3 scenarios only        │
└──────────────────┴────────────────────────────────────────────────┘

Total wall-clock today : ~ 8 - 12 min (mostly Rust cargo build).
```

What this workflow IS good at :

  - End-to-end rendering correctness (golden diff catches any template
    drift byte-for-byte).
  - Manifest-side schema roundtrip (Rust validator must accept the
    rendered ConfigMap).
  - Parent-chart wiring (cross-chart env var propagation).
  - K8s API conformance on **one** version (1.28.0).
  - Per-template unit assertions (the 11 suites).
  - Polaris `danger`-tier audit.

What this workflow is NOT good at :

  - **Multi-version K8s coverage** — we only check 1.28.0.  Users on
    1.29/1.30/1.31 get no signal.
  - **Style** — no yamllint, no shellcheck on the hook bash scripts.
  - **Schema-of-schema** — `values.schema.json` itself is never
    validated as a valid JSON Schema 2020-12 document.
  - **Doc sync** — README.md tables of values drift silently.
  - **Custom EFE invariants beyond what helm-unittest catches** —
    if someone adds a brand-new template with no unit test, the
    "every resource carries `efe.io/tenant`" rule is invisible.
  - **Supply chain** — no SBOM.
  - **PR ergonomics** — failures land as 7 separate red checks ;
    no consolidated comment.
  - **Reusability** — the umbrella workflow has to duplicate logic.

PR #15 fills every one of those gaps.

---

## 3. Why a SECOND workflow (not amend the existing one)

This is AD1.  Considered alternatives :

  A) **Amend `helm-meta-swarm.yml`** : add 7 more jobs.
     - PRO : single source of truth, single sticky comment surface.
     - CON : couples the slow-test cadence (Rust cargo build) to the
       fast-lint cadence.  Every lint change forces a full rebuild +
       cargo cache invalidation.
     - CON : the 14-job DAG would have heterogeneous concurrency needs
       (lint can run flat in parallel ; Rust roundtrip needs the
       template artifact from `lint-and-template`).
     - CON : a single workflow_call signature can't cleanly separate
       "lint-only" callers from "full-battery" callers.

  B) **Two workflows in one file** (jobs with different `if:` guards).
     - PRO : single file.
     - CON : `if:` proliferation reduces readability ; GitHub Actions'
       `if:` is expression-based and hard to debug.
     - CON : `concurrency:` is workflow-level — can't have different
       cancellation policies inside one workflow.

  C) **Two workflow files** (chosen).
     - PRO : independent caching, independent concurrency groups,
       independent path filters.
     - PRO : `workflow_call` reusability is cleaner (PR #15's workflow
       has one signature ; PR #14's another).
     - PRO : the umbrella `helm-stack-test.yml` can invoke both
       independently or only the lint one for fast-path PRs.
     - CON : two PR check entries.  Mitigated by sticky-comment
       consolidation (Job 14).
     - CON : tiny code duplication for setup-helm + checkout.

We choose (C).  The CON ("two PR checks") is actually a PRO from a
SRE perspective : it tells you which class of regression you hit
(structural drift → look at the integration battery ; policy violation
→ look at the lint workflow).

---

## 4. Branch + commit plan (6 commits)

Each commit is independently reviewable and CI-green on its own (the
workflow is only invoked once the file lands ; intermediate commits
that add `policies/rego/efe/*.rego` files are no-ops for CI).

  1. `pr15: skeleton workflow file + Job 01-04 (style + helm lint + ct lint)`
     - Adds `.github/workflows/helm-meta-swarm-lint.yml` with
       Jobs 01-04 (bootstrap, yamllint+shellcheck+pre-commit, helm lint,
       ct lint).
     - Adds `.yamllint`, `.pre-commit-config.yaml`,
       `chart-testing.yaml` (ct config).

  2. `pr15: schema self-lint + helm-docs + multi-K8s kubeconform`
     - Adds Jobs 05-07 (ajv schema-of-schema, helm-docs sync check,
       kubeconform matrix 1.28/1.29/1.30/1.31).
     - Adds `Makefile` target `make helm-docs` for local sync.

  3. `pr15: policy-as-code — Conftest + custom Rego rules`
     - Adds Job 08 (Conftest).
     - Adds `policies/rego/efe/` directory with 11 Rego files
       (tenant_label, replica_lock, sighup_token_off, hook_sa_required,
       no_latest_tag, ...).
     - Adds `policies/rego/efe/data/` for parameterized rules.
     - Adds policy unit tests (`*_test.rego`).

  4. `pr15: security scans — Trivy + Checkov + Polaris tiered`
     - Adds Jobs 09-11.
     - Adds `.trivyignore`, `.checkov.yaml`, `polaris.yaml` configs.

  5. `pr15: provenance — ArtifactHub lint + SBOM (Syft → CycloneDX)`
     - Adds Jobs 12-13.
     - Adds `artifacthub-pkg.yml` if missing.

  6. `pr15: PR ergonomics — sticky comment + workflow_call interface`
     - Adds Job 14.
     - Adds `workflow_call` block + outputs.
     - Renovate `renovate.json` updated to track the linter image
       versions.

---

## 5. Architectural decisions (AD1–AD30)

| AD  | Decision                                                                                                                                            | Status      |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------|-------------|
| AD1 | New file `.github/workflows/helm-meta-swarm-lint.yml` ; do NOT amend `helm-meta-swarm.yml`. See § 3.                                                | accepted    |
| AD2 | Job DAG, not single super-job — independent failure surfaces + parallel scheduling.                                                                  | accepted    |
| AD3 | Every linter image pinned by **tag + digest** (`@sha256:...`). Renovate updates.                                                                    | accepted    |
| AD4 | `kubeconform` runs as a **matrix** of K8s versions : 1.28, 1.29, 1.30, 1.31 — the four versions we expect operators on.                            | accepted    |
| AD5 | Workflow-level `concurrency: { group: lint-${{ github.ref }}, cancel-in-progress: true }` — stale PR pushes cancel the previous lint run.          | accepted    |
| AD6 | Cache `~/.cache/helm` + plugin binaries + tool binaries via `actions/cache` keyed on the tool-versions matrix hash.                                | accepted    |
| AD7 | Per-stage `fail-fast: false` inside the matrix (Job 07) ; workflow-level `fail-fast: false` so all 14 jobs run regardless of any single failure.   | accepted    |
| AD8 | Policy-as-code via **Conftest (OPA Rego)** — chosen over Kyverno (cluster-needed) and OPA Gatekeeper (cluster-needed). Conftest is binary-only.    | accepted    |
| AD9 | `.yamllint` config in repo root, NOT inline in workflow — versioned, local-runnable.                                                                 | accepted    |
| AD10| Helm version pinned to **v3.13.3** (matches PR #14's workflow).  Renovate-managed.                                                                  | accepted    |
| AD11| `chart-testing.yaml` (ct config) limits scope to `charts/efe-stack/charts/efe-meta-swarm`, skips chart-bumping (umbrella governs that).            | accepted    |
| AD12| Expose `workflow_call` with two outputs : `lint-result` (overall pass/fail) + `sbom-artifact-name`.  Used by umbrella's PR workflow.                | accepted    |
| AD13| PR sticky comment via `marocchino/sticky-pull-request-comment@v2.9.0` — one comment, edited per push (not spammy).                                  | accepted    |
| AD14| Polaris configured with severity-tiered output — `danger` blocks merge, `warning` annotates, `ignore` produces SARIF metric.                       | accepted    |
| AD15| Trivy outputs SARIF, uploaded to GitHub Code Scanning via `github/codeql-action/upload-sarif@v3`.  Severity threshold : `HIGH,CRITICAL`.            | accepted    |
| AD16| **No** "lint-only-on-diff" optimisation.  Every file in the chart is re-linted on every PR — cheap (5 min wall-clock) and predictable.             | accepted    |
| AD17| Runner : `ubuntu-22.04` to match PR #14's workflow.  Plan to bump to `ubuntu-24.04` in PR #16 when matrix-validated.                                | accepted    |
| AD18| Custom Rego rules live at `policies/rego/efe/` (top of repo, not inside `charts/`) — they're org-wide, not chart-local.                            | accepted    |
| AD19| Conftest scopes : the **rendered** manifests (output of `helm template`), NOT the raw templates.  This catches bugs no static-template lint sees. | accepted    |
| AD20| K8s versions matrix is parameterised by `env.KUBE_VERSIONS` so a renovate-style PR can update all four in one diff.                                | accepted    |
| AD21| `helm-docs` runs as a **gate** (not auto-commit) — PR author runs `make helm-docs` locally, CI verifies no drift.                                  | accepted    |
| AD22| Pre-commit hooks generated by `pre-commit` framework ; `.pre-commit-config.yaml` is the single source.  CI runs `pre-commit run --all-files`.       | accepted    |
| AD23| SBOM format : **CycloneDX 1.5** JSON.  Industry-aligned (NIST SP 800-218 maps SPDX/CycloneDX interchangeably).                                      | accepted    |
| AD24| Image scan : NOT in scope.  Chart-only scan.  The meta-swarm controller/participant images are scanned by their own (upstream) build pipeline.    | accepted    |
| AD25| No `[skip lint]` or `[skip ci]` magic-string bypass.  Branch protection requires the 5 required jobs ; admin override leaves an audit trail.       | accepted    |
| AD26| Renovate tracks every linter version in `renovate.json` ; PRs that bump tool versions go through this same lint suite — fixed-point check.        | accepted    |
| AD27| Required Rego rule coverage must include the top-10 K8s anti-patterns from CIS K8s benchmark v1.8 + 11 EFE-specific rules.                          | accepted    |
| AD28| Captured artifacts on failure : rendered manifests for all 6 scenarios + Conftest verdict JSON + Polaris JSON.  Retention 7 days.                  | accepted    |
| AD29| EFE-specific Rego policies enforce : `efe.io/tenant` label on every namespaced resource, no `:latest`, securityContext required on every container, no `privileged`, no `hostNetwork/hostPID/hostIPC`, every Pod with a non-default SA gets `automountServiceAccountToken: false` unless it's a hook Job, runtime pods reject `shareProcessNamespace: true` unless `bundle.sidecar.enabled`, every Service has `appProtocol`, every container has `resources.limits` AND `resources.requests`, every NetworkPolicy explicitly sets `policyTypes`, every PrometheusRule alert has `annotations.runbook_url`. | accepted    |
| AD30| Path filter `paths:` on `pull_request` is the **only** trigger surface.  No `push:` trigger ; protects from accidental re-runs on main.            | accepted    |

---

## 6. Lint tool taxonomy (14 stages, who-owns-what)

| Stage | Tool                              | Job # | What it catches                                                                  | Severity tier | Required? |
|-------|-----------------------------------|-------|----------------------------------------------------------------------------------|---------------|-----------|
| 1     | `yamllint`                        | 02    | YAML style — indentation, line length, trailing whitespace                       | hard          | yes       |
| 2     | `shellcheck`                      | 02    | Bash bugs in `pre-upgrade-snapshot.yaml` + `post-upgrade-sighup.yaml` scripts    | hard          | yes       |
| 3     | `pre-commit`                      | 02    | Aggregated local-runnable check : aggregates 1+2 + custom hooks                 | hard          | yes       |
| 4     | `helm lint --strict`              | 03    | Helm built-in lint : schema, required fields, dependency consistency             | hard          | **yes**   |
| 5     | `chart-testing ct lint`           | 04    | Chart bumping, dependency check, custom Lua hooks                                | hard          | yes       |
| 6     | `ajv-cli` + JSON Schema 2020-12   | 05    | `values.schema.json` itself is a valid schema (validates against its meta-schema) | hard          | yes       |
| 7     | `helm-docs --check`               | 06    | `README.md` values-table in sync with `values.yaml`                              | soft          | no        |
| 8     | `kubeconform` × 4 K8s versions    | 07    | K8s API conformance per version                                                  | hard          | **yes**   |
| 9     | `conftest test` (OPA Rego)        | 08    | Custom EFE policies (11 rules) + CIS K8s benchmark top-10                       | hard          | **yes**   |
| 10    | `trivy config`                    | 09    | IaC misconfigurations (CVE-like for K8s manifests)                               | tiered        | yes       |
| 11    | `checkov`                         | 10    | IaC misconfigurations (different ruleset, secondary source)                       | tiered        | no        |
| 12    | `polaris audit`                   | 11    | K8s pod-spec best practices (danger / warning / ignore tiers)                    | tiered        | **yes**   |
| 13    | `ah lint`                         | 12    | ArtifactHub metadata correctness                                                  | soft          | no        |
| 14    | `syft` (CycloneDX)                | 13    | SBOM emission + GitHub artifact upload                                            | informational | no        |

Job 01 (bootstrap) and Job 14 (sticky comment) are infrastructure, not
linters per se.

**Required for merge** (5 gates) : Jobs 03, 07, 08, 11, plus the
overall workflow status (Job 14 outputs a pass/fail aggregate that
the umbrella workflow can consume).

---

## 7. Layout : files added + modified

```text
.github/workflows/
├── helm-meta-swarm.yml                    # (PR #14 ; unchanged)
└── helm-meta-swarm-lint.yml               # NEW — this PR

policies/rego/efe/                         # NEW dir
├── kubernetes.rego                          # CIS K8s benchmark top-10
├── tenant_label.rego                        # efe.io/tenant required
├── no_latest_tag.rego                       # image tag pinning
├── security_context.rego                    # securityContext required
├── no_privileged.rego                       # no privileged containers
├── no_host_namespace.rego                   # no hostNetwork/PID/IPC
├── sa_token.rego                            # automountServiceAccountToken: false on runtime pods
├── share_process_namespace.rego             # only allowed when bundle.sidecar.enabled
├── service_appprotocol.rego                 # every Service has appProtocol
├── container_resources.rego                 # requests + limits required
├── networkpolicy_types.rego                 # NetworkPolicy must set policyTypes
├── prometheusrule_runbook.rego              # every alert has runbook_url
├── data/
│   ├── tenant.json                          # parameterised allowed tenants
│   └── runtime_pods.json                    # which deployments are "runtime" vs "hook"
└── kubernetes_test.rego                   # + 11 per-rule test files

charts/efe-stack/charts/efe-meta-swarm/
├── README.md                              # MODIFIED — `helm-docs` regenerated
├── artifacthub-pkg.yml                    # NEW — ArtifactHub metadata
└── (everything else from PR #14, unchanged)

.yamllint                                  # NEW — yamllint config
.pre-commit-config.yaml                    # NEW — pre-commit hooks
chart-testing.yaml                         # NEW — ct config
polaris.yaml                               # NEW — polaris config (severity tiers)
.trivyignore                               # NEW — trivy false-positive allowlist
.checkov.yaml                              # NEW — checkov config
renovate.json                              # MODIFIED — track lint tool versions
Makefile                                   # MODIFIED — make helm-docs target
docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md    # NEW — this doc
```

22 new files + 4 modified.

---

## 8. Workflow file structure (top-level)

```yaml
# .github/workflows/helm-meta-swarm-lint.yml

name: helm-meta-swarm-lint

on:
  pull_request:
    paths:
      - "charts/efe-stack/charts/efe-meta-swarm/**"
      - "policies/rego/efe/**"
      - ".github/workflows/helm-meta-swarm-lint.yml"
      - ".yamllint"
      - ".pre-commit-config.yaml"
      - "chart-testing.yaml"
      - "polaris.yaml"
      - ".trivyignore"
      - ".checkov.yaml"
  workflow_call:
    outputs:
      lint-result:
        description: "Overall lint verdict : pass | fail"
        value: ${{ jobs.sticky-summary.outputs.verdict }}
      sbom-artifact-name:
        description: "Name of the CycloneDX SBOM artifact"
        value: ${{ jobs.sbom.outputs.artifact-name }}
    inputs:
      debug:
        description: "Verbose tool output (for triage)"
        required: false
        type: boolean
        default: false

# AD5 — workflow-level cancel-in-progress.
concurrency:
  group: helm-meta-swarm-lint-${{ github.ref }}
  cancel-in-progress: true

env:
  HELM_VERSION:        "v3.13.3"
  KUBECONFORM_VERSION: "v0.6.4"
  CHART_TESTING_VER:   "v3.10.1"
  CONFTEST_VERSION:    "v0.49.1"
  TRIVY_VERSION:       "v0.49.1"
  CHECKOV_VERSION:     "3.1.79"
  POLARIS_VERSION:     "8.5.5"
  HELM_DOCS_VERSION:   "v1.12.0"
  AH_VERSION:          "v1.18.0"
  SYFT_VERSION:        "v1.0.1"
  AJV_VERSION:         "5.0.0"
  PYTHON_VERSION:      "3.11"

  # Matrix of K8s versions for kubeconform.  Bumping a version here
  # forces a fresh PR through this same lint suite — fixed-point.
  KUBE_VERSIONS: '["1.28.0", "1.29.0", "1.30.0", "1.31.0"]'

jobs:
  # Jobs 01 .. 14 (see § 12 - § 25).
  ...
```

---

## 9. Concurrency + cancellation model

AD5.  Three layers of cancellation :

  1. **Workflow level** : `concurrency: { group: lint-<ref>, cancel-in-progress: true }`.
     On a PR push, the previous run for that PR's `head_ref` is
     cancelled.  Saves ~80% of compute on rapid-fire pushes.

  2. **Matrix level** (kubeconform — Job 07) : `strategy.fail-fast: false`.
     A failure on K8s 1.28 does NOT cancel the other three versions.
     We WANT every-version signal on every push.

  3. **No job-level cancellation** between sibling jobs.  Job 03's
     failure does NOT stop Jobs 04-13 ; the operator should see every
     class of failure in the sticky comment.

This means a single push generates one workflow run with up to ~17
sub-tasks running in parallel (14 jobs + 3 extra matrix entries for
kubeconform).  Total wall-clock ≤ 5 min.

---

## 10. Cache strategy

AD6.  Three caches :

  1. **Helm plugin cache** : `~/.local/share/helm/plugins` —
     `actions/cache@v4` keyed on `${{ env.HELM_VERSION }}-plugins-v1`.
     Persists the helm-diff + helm-unittest plugins across runs.

  2. **Tool binaries cache** : `/usr/local/bin/{kubeconform,conftest,...}` —
     keyed on a SHA-1 hash of the env-var block above.  Renovate PRs
     change the hash → cache invalidates → fresh download.

  3. **Pre-commit cache** : `~/.cache/pre-commit` — keyed on
     `${{ hashFiles('.pre-commit-config.yaml') }}`.

No cache on rendered manifests : Job 03's `helm template` output is
job-local (the output is used by Jobs 07, 08, 09, 11 ; we pass it via
`actions/upload-artifact` + `download-artifact`).

Cache size budget : ≤ 80 MiB compressed (well under GitHub's 10 GiB
per repo quota).

---

## 11. Reusable workflow contract (`workflow_call`)

AD12.  This workflow is invokable from another workflow :

```yaml
# .github/workflows/helm-stack-test.yml (umbrella ; lives in PR #20 scope
# but mirrors the same pattern).
jobs:
  meta-swarm-lint:
    uses: ./.github/workflows/helm-meta-swarm-lint.yml
    with:
      debug: ${{ github.event_name == 'workflow_dispatch' }}
```

The `outputs:` block exposes :

  - `lint-result` — `pass` if all 5 required gates passed ; `fail`
    otherwise.  Umbrella's workflow gates its own merge on this.
  - `sbom-artifact-name` — the name of the SBOM artifact emitted by
    Job 13.  Umbrella's release workflow consumes it.

The `inputs:` block has one : `debug` (boolean).  When `true`, all
linters run with `-v` / `--debug` flags ; sticky comment is verbose.

---

## 12. Job 01 — bootstrap (checkout + setup-helm + cache restore)

```yaml
jobs:
  bootstrap:
    name: 01 / bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 3
    outputs:
      cache-key:    ${{ steps.cache-key.outputs.value }}
      rendered-dir: ${{ steps.render.outputs.dir }}
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }       # full history for helm-docs

      - id: cache-key
        run: |
          # Hash the tool versions block so renovate PRs invalidate caches.
          v=$(cat <<EOF | sha1sum | head -c 12
          helm=${{ env.HELM_VERSION }}
          kubeconform=${{ env.KUBECONFORM_VERSION }}
          ct=${{ env.CHART_TESTING_VER }}
          conftest=${{ env.CONFTEST_VERSION }}
          trivy=${{ env.TRIVY_VERSION }}
          checkov=${{ env.CHECKOV_VERSION }}
          polaris=${{ env.POLARIS_VERSION }}
          ah=${{ env.AH_VERSION }}
          syft=${{ env.SYFT_VERSION }}
          ajv=${{ env.AJV_VERSION }}
          EOF
          )
          echo "value=$v" >> $GITHUB_OUTPUT

      - uses: azure/setup-helm@v4
        with: { version: ${{ env.HELM_VERSION }} }

      - name: Cache tool binaries
        uses: actions/cache@v4
        with:
          path: /usr/local/bin/lint-tools
          key:  lint-tools-${{ steps.cache-key.outputs.value }}

      # Render all 6 scenarios once, share with downstream jobs.
      - id: render
        run: |
          mkdir -p /tmp/rendered
          for scen in kovant-3p kovant-disabled kovant-network-policy \
                      kovant-service-monitor kovant-prod kovant-hooks ; do
            helm template tst charts/efe-stack/charts/efe-meta-swarm \
              --namespace efe \
              -f "charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/${scen}.values.yaml" \
              > "/tmp/rendered/${scen}.yaml"
          done
          # Examples overlays too.
          for flavor in dev staging prod canary ; do
            helm template tst charts/efe-stack/charts/efe-meta-swarm \
              --namespace efe \
              -f "charts/efe-stack/charts/efe-meta-swarm/examples/values-${flavor}.yaml" \
              > "/tmp/rendered/values-${flavor}.yaml"
          done
          echo "dir=/tmp/rendered" >> $GITHUB_OUTPUT

      - uses: actions/upload-artifact@v4
        with:
          name: rendered-${{ github.run_id }}
          path: /tmp/rendered/
          retention-days: 7
```

Outputs : `rendered-dir` (consumed by Jobs 07, 08, 09, 11) +
`cache-key` (consumed by every downstream job).

Timing budget : 3 minutes.

---

## 13. Job 02 — `yamllint` + `shellcheck` + `pre-commit`

```yaml
  style:
    name: 02 / style (yamllint + shellcheck + pre-commit)
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 3
    steps:
      - uses: actions/checkout@v4

      - uses: actions/setup-python@v5
        with: { python-version: ${{ env.PYTHON_VERSION }} }

      - run: pip install pre-commit==3.6.0 yamllint==1.35.1

      - name: pre-commit (drives yamllint + shellcheck + custom hooks)
        run: pre-commit run --all-files --show-diff-on-failure
```

`.yamllint` config (§ 27) sets line-length 120, indentation 2, and
allows specific Helm template syntax (e.g. `{{ ... }}` inside string
values doesn't trigger key-duplication false-positives).

`shellcheck` runs on every `command: ['sh', '-c', ...]` block in the
hook templates.  Detected via the custom pre-commit hook in
`.pre-commit-config.yaml` (§ 28) that greps for `command:` blocks and
extracts the script body to a temp file.

---

## 14. Job 03 — `helm lint --strict`

```yaml
  helm-lint:
    name: 03 / helm lint --strict       # REQUIRED for merge
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 2
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
        with: { version: ${{ env.HELM_VERSION }} }

      - name: helm lint --strict
        run: |
          helm lint charts/efe-stack/charts/efe-meta-swarm --strict \
            -f charts/efe-stack/charts/efe-meta-swarm/values.yaml

      # Lint each scenario individually — catches values-merge edge cases
      # that the chart-wide lint misses.
      - name: helm lint per scenario
        run: |
          for scen in kovant-3p kovant-disabled kovant-network-policy \
                      kovant-service-monitor kovant-prod kovant-hooks ; do
            echo "── lint ${scen} ──"
            helm lint charts/efe-stack/charts/efe-meta-swarm --strict \
              -f "charts/efe-stack/charts/efe-meta-swarm/tests/scenarios/${scen}.values.yaml"
          done
```

`--strict` upgrades warnings to errors.  Required gate.

---

## 15. Job 04 — `chart-testing` (`ct lint`)

```yaml
  ct-lint:
    name: 04 / chart-testing
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 3
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }

      - uses: helm/chart-testing-action@v2.6.1
        with: { version: ${{ env.CHART_TESTING_VER }} }

      - name: ct lint
        run: |
          ct lint --config chart-testing.yaml --target-branch main
```

`chart-testing.yaml` :

```yaml
# chart-testing.yaml
chart-dirs:
  - charts/efe-stack/charts
chart-repos: []                       # all local, no external repos
target-branch: main
check-version-increment: true         # require Chart.yaml version bump on changes
validate-chart-schema: true           # validates against /charts/spec.json
validate-maintainers: true
debug: false
```

The `check-version-increment` gate is critical : every PR touching the
chart MUST bump `Chart.yaml`'s `version` field (semver) ; missing this
is a hard fail.

---

## 16. Job 05 — `values.schema.json` self-validation

```yaml
  schema-of-schema:
    name: 05 / values.schema.json self-validation
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 1
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: '20' }

      - run: npm install -g ajv-cli@${{ env.AJV_VERSION }}

      - name: Validate schema against JSON Schema 2020-12 meta-schema
        run: |
          # Download the official 2020-12 meta-schema (cached locally).
          curl -sL https://json-schema.org/draft/2020-12/schema \
            -o /tmp/meta-schema.json

          # Validate values.schema.json AS data against the meta-schema.
          ajv compile \
            --spec=draft2020 \
            -s /tmp/meta-schema.json \
            -d charts/efe-stack/charts/efe-meta-swarm/values.schema.json

          echo "✅ values.schema.json is a valid JSON Schema 2020-12 document"

      - name: Validate the example overlays against values.schema.json
        run: |
          ajv validate \
            --spec=draft2020 \
            -s charts/efe-stack/charts/efe-meta-swarm/values.schema.json \
            -d "charts/efe-stack/charts/efe-meta-swarm/examples/values-*.yaml"
```

This catches the class of bug where a refactor of `values.schema.json`
introduces invalid syntax (e.g. dangling `$ref`) that Helm itself
silently ignores — but means runtime validation does nothing.  PR #14's
audit caught one such bug ; this gate prevents recurrence.

---

## 17. Job 06 — `helm-docs` README sync check

```yaml
  helm-docs:
    name: 06 / helm-docs sync
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 2
    steps:
      - uses: actions/checkout@v4

      - uses: docker://jnorwood/helm-docs:v1.12.0
        with:
          args: >-
            --chart-search-root=charts/efe-stack/charts/efe-meta-swarm
            --template-files=README.md.gotmpl
            --output-file=README.md.generated

      - name: Diff generated README against committed README
        run: |
          if ! diff -u \
              charts/efe-stack/charts/efe-meta-swarm/README.md \
              charts/efe-stack/charts/efe-meta-swarm/README.md.generated ; then
            echo "❌ README.md is out of sync.  Run \`make helm-docs\` locally and commit." >&2
            exit 1
          fi
```

`Makefile` target added in the same PR :

```make
helm-docs:
	@docker run --rm \
		-v $(PWD)/charts/efe-stack/charts/efe-meta-swarm:/helm-docs \
		jnorwood/helm-docs:v1.12.0 \
		--template-files=README.md.gotmpl \
		--output-file=README.md
```

Soft gate (warning) for v1 ; will be promoted to hard gate in PR #16.

---

## 18. Job 07 — `kubeconform` multi-K8s-version matrix

```yaml
  kubeconform:
    name: 07 / kubeconform (K8s ${{ matrix.kube }})    # REQUIRED for merge
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 4
    strategy:
      fail-fast: false                  # we want signal for every version
      matrix:
        kube: ${{ fromJson(env.KUBE_VERSIONS) }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: rendered-${{ github.run_id }}
          path: /tmp/rendered

      - name: Install kubeconform
        run: |
          curl -L "https://github.com/yannh/kubeconform/releases/download/${{ env.KUBECONFORM_VERSION }}/kubeconform-linux-amd64.tar.gz" \
            | tar xz
          sudo mv kubeconform /usr/local/bin/

      # Per-scenario validation against the matrix K8s version.
      # CRDs (PrometheusRule, ServiceMonitor, Issuer, Certificate) need
      # the datreeio catalog so kubeconform doesn't choke.
      - name: kubeconform ${{ matrix.kube }} × all scenarios
        run: |
          for f in /tmp/rendered/*.yaml ; do
            echo "── ${matrix-kube:-${{ matrix.kube }}} : $(basename $f) ──"
            kubeconform -strict -summary \
              -kubernetes-version ${{ matrix.kube }} \
              -schema-location default \
              -schema-location 'https://raw.githubusercontent.com/datreeio/CRDs-catalog/main/{{ .Group }}/{{ .ResourceKind }}_{{ .ResourceAPIVersion }}.json' \
              "$f"
          done
```

12 scenarios × 4 K8s versions = 48 checks.  Wall-clock ~ 2 min per
matrix entry, 4 entries in parallel.

Required gate.  Failure on ANY K8s version blocks merge.

---

## 19. Job 08 — Conftest (OPA Rego) custom EFE policies

```yaml
  conftest:
    name: 08 / Conftest (OPA policies)    # REQUIRED for merge
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 3
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: rendered-${{ github.run_id }}
          path: /tmp/rendered

      - name: Install Conftest
        run: |
          curl -sL "https://github.com/open-policy-agent/conftest/releases/download/${{ env.CONFTEST_VERSION }}/conftest_$(echo ${{ env.CONFTEST_VERSION }} | sed s/^v//)_Linux_x86_64.tar.gz" \
            | tar xz
          sudo mv conftest /usr/local/bin/

      - name: Run policy tests (the Rego rules themselves)
        run: |
          conftest verify --policy policies/rego/efe --data policies/rego/efe/data

      - name: Apply policies to rendered manifests
        run: |
          for f in /tmp/rendered/*.yaml ; do
            echo "── policy check : $(basename $f) ──"
            conftest test \
              --policy policies/rego/efe \
              --data policies/rego/efe/data \
              --output stdout \
              --no-color \
              "$f"
          done

      - name: Conftest verdict as SARIF
        if: failure()
        run: |
          conftest test \
            --policy policies/rego/efe \
            --data policies/rego/efe/data \
            --output sarif \
            /tmp/rendered/*.yaml > /tmp/conftest.sarif

      - uses: github/codeql-action/upload-sarif@v3
        if: failure()
        with:
          sarif_file: /tmp/conftest.sarif
          category: conftest
```

Required gate.  Custom EFE rule violations are non-negotiable.

---

## 20. Job 09 — Trivy IaC scan

```yaml
  trivy:
    name: 09 / Trivy IaC
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 3
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: rendered-${{ github.run_id }}
          path: /tmp/rendered

      - name: Trivy config scan
        uses: aquasecurity/trivy-action@0.20.0
        with:
          scan-type: config
          scan-ref:  /tmp/rendered
          format:    sarif
          output:    /tmp/trivy.sarif
          severity:  HIGH,CRITICAL
          exit-code: 1               # hard fail on HIGH/CRITICAL
          ignore-unfixed: false

      - uses: github/codeql-action/upload-sarif@v3
        if: always()
        with:
          sarif_file: /tmp/trivy.sarif
          category: trivy
```

`.trivyignore` lives in repo root with explicit reasons for each
ignored finding (Renovate-tracked review cycle).

---

## 21. Job 10 — Checkov

```yaml
  checkov:
    name: 10 / Checkov
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 3
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: rendered-${{ github.run_id }}
          path: /tmp/rendered

      - uses: actions/setup-python@v5
        with: { python-version: ${{ env.PYTHON_VERSION }} }

      - run: pip install checkov==${{ env.CHECKOV_VERSION }}

      - name: Checkov scan
        run: |
          checkov --directory /tmp/rendered \
            --framework kubernetes \
            --config-file .checkov.yaml \
            --output sarif --output-file-path /tmp/checkov.sarif \
            --soft-fail              # secondary source — informational

      - uses: github/codeql-action/upload-sarif@v3
        if: always()
        with:
          sarif_file: /tmp/checkov.sarif
          category: checkov
```

Soft-fail by design — Checkov's K8s ruleset overlaps Trivy's ; we keep
it as a secondary signal but don't block merge.

---

## 22. Job 11 — Polaris severity-tiered

```yaml
  polaris:
    name: 11 / Polaris (severity-tiered)    # REQUIRED for merge
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 3
    steps:
      - uses: actions/checkout@v4
      - uses: actions/download-artifact@v4
        with:
          name: rendered-${{ github.run_id }}
          path: /tmp/rendered

      - name: Install Polaris
        run: |
          curl -sL "https://github.com/FairwindsOps/polaris/releases/download/${{ env.POLARIS_VERSION }}/polaris_linux_amd64.tar.gz" \
            | tar xz
          sudo mv polaris /usr/local/bin/

      - name: Polaris audit (danger — blocks merge)
        run: |
          for f in /tmp/rendered/*.yaml ; do
            polaris audit \
              --audit-path "$f" \
              --config polaris.yaml \
              --severity danger \
              --output-format pretty
          done

      - name: Polaris audit (warning — annotates, doesn't block)
        run: |
          for f in /tmp/rendered/*.yaml ; do
            polaris audit \
              --audit-path "$f" \
              --config polaris.yaml \
              --severity warning \
              --output-format json > "/tmp/polaris-warn-$(basename $f .yaml).json" || true
          done

      - name: Upload Polaris warning + ignore-tier reports as artifact
        uses: actions/upload-artifact@v4
        with:
          name: polaris-${{ github.run_id }}
          path: /tmp/polaris-*.json
          retention-days: 7
```

`polaris.yaml` configures severity tiers per check.  Top-level
`checks:` block locks `danger`-tier for : `runAsRootAllowed`,
`runAsPrivileged`, `hostNetworkSet`, `hostPIDSet`, `hostIPCSet`,
`notReadOnlyRootFilesystem`, `dangerousCapabilities` ; `warning`-tier
for : `cpuRequestsMissing`, `cpuLimitsMissing`, `memoryRequestsMissing`,
`memoryLimitsMissing`, `pullPolicyNotAlways`, `tagNotSpecified`.

---

## 23. Job 12 — ArtifactHub metadata lint

```yaml
  artifacthub:
    name: 12 / ArtifactHub metadata
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 1
    steps:
      - uses: actions/checkout@v4

      - name: Install ah
        run: |
          curl -sL "https://github.com/artifacthub/hub/releases/download/${{ env.AH_VERSION }}/ah_$(echo ${{ env.AH_VERSION }} | sed s/^v//)_linux_amd64.tar.gz" \
            | tar xz
          sudo mv ah /usr/local/bin/

      - run: ah lint charts/efe-stack/charts/efe-meta-swarm/
```

The `artifacthub-pkg.yml` file (NEW in this PR) provides the metadata
that the public ArtifactHub crawler picks up.

---

## 24. Job 13 — SBOM (Syft → CycloneDX)

```yaml
  sbom:
    name: 13 / SBOM (CycloneDX)
    needs: bootstrap
    runs-on: ubuntu-22.04
    timeout-minutes: 2
    outputs:
      artifact-name: ${{ steps.sbom.outputs.name }}
    steps:
      - uses: actions/checkout@v4

      - name: Install Syft
        run: |
          curl -sL "https://github.com/anchore/syft/releases/download/${{ env.SYFT_VERSION }}/syft_$(echo ${{ env.SYFT_VERSION }} | sed s/^v//)_linux_amd64.tar.gz" \
            | tar xz
          sudo mv syft /usr/local/bin/

      - id: sbom
        run: |
          syft scan dir:charts/efe-stack/charts/efe-meta-swarm \
            -o cyclonedx-json=/tmp/sbom.cdx.json
          echo "name=sbom-${{ github.run_id }}" >> $GITHUB_OUTPUT

      - uses: actions/upload-artifact@v4
        with:
          name: sbom-${{ github.run_id }}
          path: /tmp/sbom.cdx.json
          retention-days: 30        # longer than other artifacts ; SBOM is releasable
```

---

## 25. Job 14 — PR sticky comment summary

```yaml
  sticky-summary:
    name: 14 / PR sticky summary
    needs:
      - style
      - helm-lint
      - ct-lint
      - schema-of-schema
      - helm-docs
      - kubeconform
      - conftest
      - trivy
      - checkov
      - polaris
      - artifacthub
      - sbom
    runs-on: ubuntu-22.04
    if: always()                    # run even if predecessors failed
    timeout-minutes: 2
    outputs:
      verdict: ${{ steps.compose.outputs.verdict }}
    steps:
      - id: compose
        run: |
          # Required gates : style + helm-lint + kubeconform + conftest + polaris.
          required_pass=true
          for j in style helm-lint kubeconform conftest polaris ; do
            r="${{ needs[j].result }}"
            if [ "$r" != "success" ] ; then required_pass=false ; fi
          done
          if [ "$required_pass" = "true" ] ; then
            echo "verdict=pass" >> $GITHUB_OUTPUT
          else
            echo "verdict=fail" >> $GITHUB_OUTPUT
          fi

      - uses: marocchino/sticky-pull-request-comment@v2.9.0
        if: github.event_name == 'pull_request'
        with:
          header: helm-meta-swarm-lint
          message: |
            ## helm-meta-swarm-lint summary

            | Stage                            | Result                                          |
            |----------------------------------|--------------------------------------------------|
            | 02 / style                       | ${{ needs.style.result }}                        |
            | 03 / helm lint                   | ${{ needs.helm-lint.result }}                    |
            | 04 / chart-testing               | ${{ needs.ct-lint.result }}                      |
            | 05 / schema-of-schema            | ${{ needs.schema-of-schema.result }}             |
            | 06 / helm-docs                   | ${{ needs.helm-docs.result }}                    |
            | 07 / kubeconform                 | ${{ needs.kubeconform.result }}                  |
            | 08 / Conftest                    | ${{ needs.conftest.result }}                     |
            | 09 / Trivy                       | ${{ needs.trivy.result }}                        |
            | 10 / Checkov                     | ${{ needs.checkov.result }}                      |
            | 11 / Polaris                     | ${{ needs.polaris.result }}                      |
            | 12 / ArtifactHub                 | ${{ needs.artifacthub.result }}                  |
            | 13 / SBOM                        | ${{ needs.sbom.result }}                         |

            **Overall verdict** : ${{ steps.compose.outputs.verdict }}

            (Required gates : style, helm-lint, kubeconform, conftest, polaris.)
```

The `sticky-pull-request-comment` action edits a single comment per
PR, keyed on the `header:` value.  Subsequent pushes update the same
comment in-place rather than spamming a new one.

---

## 26. Custom Rego policy library (`policies/rego/efe/`)

11 EFE-specific rules.  Each rule lives in its own `.rego` file with
a matching `_test.rego` file (Rego unit tests run by `conftest verify`).

### 26.1 `tenant_label.rego` — every namespaced resource must carry `efe.io/tenant`

```rego
package main

# Every namespaced resource (Deployment, Service, Job, ...) MUST have
# the `efe.io/tenant` label.  Cluster-scoped resources (PriorityClass,
# ClusterRole, ...) are exempt.
deny[msg] {
  input.kind != ""
  not cluster_scoped[input.kind]
  not input.metadata.labels["efe.io/tenant"]
  msg := sprintf("%s/%s is missing efe.io/tenant label",
                  [input.kind, input.metadata.name])
}

cluster_scoped := {"PriorityClass", "ClusterRole", "ClusterRoleBinding"}
```

### 26.2 `no_latest_tag.rego` — image tags must be pinned

```rego
package main

deny[msg] {
  some container in input.spec.template.spec.containers
  endswith(container.image, ":latest")
  msg := sprintf("container %s/%s uses :latest image tag",
                  [input.metadata.name, container.name])
}

deny[msg] {
  some container in input.spec.template.spec.containers
  not contains(container.image, ":")
  msg := sprintf("container %s/%s has no image tag at all",
                  [input.metadata.name, container.name])
}
```

### 26.3 `security_context.rego` — every container must have securityContext

```rego
package main

deny[msg] {
  pod_specs := pod_spec_paths[_]
  some container in walk_path(input, pod_specs).containers
  not container.securityContext
  msg := sprintf("container %s in %s has no securityContext",
                  [container.name, input.metadata.name])
}

pod_spec_paths := [
  ["spec", "template", "spec"],
  ["spec"],                          # Pod kind
]
```

### 26.4 `no_privileged.rego` — no privileged containers

```rego
package main

deny[msg] {
  some container in input.spec.template.spec.containers
  container.securityContext.privileged == true
  msg := sprintf("%s/%s runs privileged",
                  [input.metadata.name, container.name])
}
```

### 26.5 `no_host_namespace.rego` — no hostNetwork / hostPID / hostIPC

```rego
package main

deny[msg] {
  input.spec.template.spec.hostNetwork == true
  msg := sprintf("%s sets hostNetwork=true", [input.metadata.name])
}

deny[msg] {
  input.spec.template.spec.hostPID == true
  msg := sprintf("%s sets hostPID=true", [input.metadata.name])
}

deny[msg] {
  input.spec.template.spec.hostIPC == true
  msg := sprintf("%s sets hostIPC=true", [input.metadata.name])
}
```

### 26.6 `sa_token.rego` — runtime pods reject SA-token mounting

Carried over from PR #14's design.  Only the hook Jobs may
`automountServiceAccountToken: true` ; runtime pods (controller +
participants + test connection) MUST be `false`.

```rego
package main

import future.keywords.in

runtime_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Pod"}

# data.runtime_pods.exempt is a list of hook-Job-style pods that
# legitimately mount the SA token.
deny[msg] {
  input.kind in runtime_kinds
  not exempt(input.metadata.name)
  input.spec.template.spec.automountServiceAccountToken != false
  msg := sprintf("%s/%s does not set automountServiceAccountToken: false",
                  [input.kind, input.metadata.name])
}

exempt(name) {
  some hook_pattern in data.runtime_pods.hook_patterns
  regex.match(hook_pattern, name)
}
```

`data/runtime_pods.json` :

```json
{
  "hook_patterns": [
    "^.*-hook-pre-upgrade-snapshot$",
    "^.*-hook-post-upgrade-sighup$"
  ]
}
```

### 26.7 `share_process_namespace.rego` — only allowed when sidecar enabled

The sidecar SIGHUP-loop requires `shareProcessNamespace: true` on the
participant Pod (cross-PID-namespace SIGHUP delivery).  But this is a
T26 threat surface — only allow it when the participant's name maps to
a deployment with `bundle.sidecar.enabled`.

```rego
package main

# Reject shareProcessNamespace on any Pod that doesn't have a
# `bundle-loader` container.
deny[msg] {
  input.spec.template.spec.shareProcessNamespace == true
  not has_bundle_loader(input)
  msg := sprintf("%s sets shareProcessNamespace=true but has no bundle-loader sidecar",
                  [input.metadata.name])
}

has_bundle_loader(pod) {
  some container in pod.spec.template.spec.containers
  container.name == "bundle-loader"
}
```

### 26.8 `service_appprotocol.rego` — every Service port has appProtocol

```rego
package main

deny[msg] {
  input.kind == "Service"
  some port in input.spec.ports
  not port.appProtocol
  msg := sprintf("Service %s port %s has no appProtocol",
                  [input.metadata.name, port.name])
}
```

### 26.9 `container_resources.rego` — requests + limits both required

```rego
package main

deny[msg] {
  some container in input.spec.template.spec.containers
  not container.resources.requests
  msg := sprintf("container %s/%s has no resources.requests",
                  [input.metadata.name, container.name])
}

deny[msg] {
  some container in input.spec.template.spec.containers
  not container.resources.limits
  msg := sprintf("container %s/%s has no resources.limits",
                  [input.metadata.name, container.name])
}
```

### 26.10 `networkpolicy_types.rego` — every NetworkPolicy sets policyTypes

```rego
package main

deny[msg] {
  input.kind == "NetworkPolicy"
  not input.spec.policyTypes
  msg := sprintf("NetworkPolicy %s has no spec.policyTypes",
                  [input.metadata.name])
}
```

### 26.11 `prometheusrule_runbook.rego` — every alert has runbook_url

Carried over from PR #14's audit pass (AC16).

```rego
package main

deny[msg] {
  input.kind == "PrometheusRule"
  some group in input.spec.groups
  some rule in group.rules
  rule.alert
  not rule.annotations.runbook_url
  msg := sprintf("PrometheusRule %s alert %s has no runbook_url",
                  [input.metadata.name, rule.alert])
}
```

### 26.12 `kubernetes.rego` — CIS K8s top-10 (aggregator)

Pulls in the upstream `open-policy-agent/library` CIS K8s benchmark
top-10 rules.  Vendored under `policies/rego/efe/cis/` to pin against
upstream drift.

---

## 27. `.yamllint` configuration

```yaml
# .yamllint
extends: default

rules:
  line-length:
    max: 120
    level: warning
  indentation:
    spaces: 2
    indent-sequences: consistent
    check-multi-line-strings: false
  comments:
    min-spaces-from-content: 1
  comments-indentation: enable
  document-start:
    present: false                   # Helm templates don't have ---
  truthy:
    allowed-values: ['true', 'false']
    check-keys: false                # Helm uses "on:", "yes:" in templates

ignore: |
  charts/efe-stack/charts/efe-meta-swarm/files/*.json
  charts/efe-stack/charts/efe-meta-swarm/tests/golden/*.yaml
  charts/efe-stack/charts/efe-meta-swarm/tests/__snapshot__/*
```

We deliberately do NOT lint the golden snapshots (they're rendered
output, with `appProtocol:` and other K8s-specific fields that violate
generic YAML style) or the Grafana dashboard JSON.

---

## 28. `.pre-commit-config.yaml` configuration

```yaml
# .pre-commit-config.yaml
default_install_hook_types: [pre-commit, pre-push]

repos:
  - repo: https://github.com/adrienverge/yamllint
    rev: v1.35.1
    hooks:
      - id: yamllint
        args: ['-c=.yamllint']

  - repo: https://github.com/koalaman/shellcheck-precommit
    rev: v0.10.0
    hooks:
      - id: shellcheck
        args: ['-x', '-S', 'warning']

  - repo: https://github.com/norwoodj/helm-docs
    rev: v1.12.0
    hooks:
      - id: helm-docs
        args:
          - --chart-search-root=charts/efe-stack/charts/efe-meta-swarm

  - repo: https://github.com/instrumenta/conftest
    rev: v0.49.1
    hooks:
      - id: conftest-test
        args:
          - --policy=policies/rego/efe
          - --data=policies/rego/efe/data

  - repo: local
    hooks:
      # Custom hook : extract command: scripts and shellcheck them.
      - id: shellcheck-helm-hooks
        name: shellcheck on Helm hook scripts
        language: script
        entry: scripts/lint-helm-hook-scripts.sh
        files: 'charts/efe-stack/charts/efe-meta-swarm/templates/hooks/.*\.yaml$'
```

The `scripts/lint-helm-hook-scripts.sh` helper extracts `command: ['sh', '-c', '...']` blocks from Helm templates and pipes them to `shellcheck` standalone — catches bugs the live `command:` string can't be checked directly for.

---

## 29. Renovate integration (tool version tracking)

`renovate.json` (modified by this PR) :

```json
{
  "$schema": "https://docs.renovatebot.com/renovate-schema.json",
  "extends": ["config:base"],
  "packageRules": [
    {
      "matchManagers": ["github-actions"],
      "matchPaths": [".github/workflows/helm-meta-swarm-lint.yml"],
      "groupName": "helm-meta-swarm-lint tool versions",
      "schedule": ["before 4am on monday"],
      "automerge": false,
      "reviewers": ["@efe-platform/sre"]
    },
    {
      "matchManagers": ["regex"],
      "matchFileNames": [".github/workflows/helm-meta-swarm-lint.yml"],
      "matchPackagePatterns": ["^aquasecurity/trivy-action$", "^helm/chart-testing-action$"],
      "groupName": "helm-meta-swarm-lint actions",
      "automerge": false
    }
  ],
  "regexManagers": [
    {
      "fileMatch": [".github/workflows/helm-meta-swarm-lint.yml"],
      "matchStrings": [
        "HELM_VERSION:\\s+\"(?<currentValue>v[0-9.]+)\"",
        "KUBECONFORM_VERSION:\\s+\"(?<currentValue>v[0-9.]+)\"",
        "CHART_TESTING_VER:\\s+\"(?<currentValue>v[0-9.]+)\"",
        "CONFTEST_VERSION:\\s+\"(?<currentValue>v[0-9.]+)\"",
        "TRIVY_VERSION:\\s+\"(?<currentValue>v[0-9.]+)\""
      ],
      "datasourceTemplate": "github-releases",
      "depNameTemplate": "helm-meta-swarm-lint:{{groupName}}"
    }
  ]
}
```

Renovate auto-PRs tool version bumps every Monday morning ; each PR
runs through this same lint workflow → fixed-point validation.

---

## 30. Branch protection contract

The repository's `main` branch protection rules (configured in
GitHub repo settings, mirrored in `terraform/github/` for code-review) :

  - **Required status checks** (5 of 14) :
    1. `helm-meta-swarm-lint / 02 / style`
    2. `helm-meta-swarm-lint / 03 / helm lint --strict`
    3. `helm-meta-swarm-lint / 07 / kubeconform (1.28.0)`
       `+ (1.29.0)` + `(1.30.0)` + `(1.31.0)` (matrix entries)
    4. `helm-meta-swarm-lint / 08 / Conftest (OPA policies)`
    5. `helm-meta-swarm-lint / 11 / Polaris (severity-tiered)`

  - Sticky-comment job (`sticky-summary`) is not required — but its
    `verdict` output is consumed by the umbrella workflow's gate.

  - Admin override : `repo-admin` GitHub role only.  Override produces
    an audit-log entry (visible in repo's "Manage access" → "Audit
    log"), reviewed weekly by SRE.

  - No `[skip lint]` / `[skip ci]` commit-message bypass (AD25).

---

## 31. Workflow timing budgets (per-job + total)

| Job # | Name                    | p50    | p99    | timeout |
|-------|-------------------------|--------|--------|---------|
| 01    | bootstrap               | 1 min  | 2 min  | 3 min   |
| 02    | style                   | 30 s   | 1 min  | 3 min   |
| 03    | helm lint               | 30 s   | 1 min  | 2 min   |
| 04    | chart-testing           | 1 min  | 2 min  | 3 min   |
| 05    | schema-of-schema        | 15 s   | 30 s   | 1 min   |
| 06    | helm-docs               | 30 s   | 1 min  | 2 min   |
| 07    | kubeconform × 4         | 1 min  | 3 min  | 4 min   |
| 08    | conftest                | 1 min  | 2 min  | 3 min   |
| 09    | trivy                   | 1 min  | 2 min  | 3 min   |
| 10    | checkov                 | 1 min  | 2 min  | 3 min   |
| 11    | polaris                 | 1 min  | 2 min  | 3 min   |
| 12    | artifacthub             | 15 s   | 30 s   | 1 min   |
| 13    | sbom                    | 30 s   | 1 min  | 2 min   |
| 14    | sticky-summary          | 10 s   | 30 s   | 2 min   |
| **total** | **wall-clock**      | **~3 min** | **~5 min** | **15 min hard cap** |

Wall-clock total is dominated by Job 01 (must finish before fan-out)
+ the slowest of Jobs 03-13 (Polaris).

---

## 32. Failure modes

```text
┌─────────────────────────────────────────────────────────────────────────┐
│ Failure class       │ Symptom                       │ Action            │
├─────────────────────┼───────────────────────────────┼───────────────────┤
│ Tool image 404      │ download step fails           │ Renovate to       │
│   (deleted upstream)│ in bootstrap or per-job       │ pin new digest    │
│                     │                               │                   │
│ Cache poisoning     │ tool binary checksum mismatch │ Bust cache by     │
│                     │                               │ bumping cache-key │
│                     │                               │ env var           │
│                     │                               │                   │
│ Schema upstream     │ ajv compile fails on          │ Vendor the meta-  │
│   drift             │ unrecognized keyword          │ schema locally    │
│                     │                               │                   │
│ Rego rule false-    │ Conftest fails on a manifest  │ Add               │
│   positive          │ that should pass              │ data.exceptions[] │
│                     │                               │                   │
│ Multi-K8s API drift │ kubeconform 1.31 fails on a   │ Bump the          │
│                     │ deprecated apiVersion         │ template to       │
│                     │                               │ newer apiVersion  │
│                     │                               │                   │
│ helm-docs drift     │ README out of sync            │ Run               │
│                     │                               │ make helm-docs    │
│                     │                               │                   │
│ Polaris severity    │ run-as-root finding promoted  │ Either fix the    │
│   re-tier           │ from warning → danger after   │ chart OR override │
│                     │ upgrade                       │ in polaris.yaml   │
│                     │                               │ with audit reason │
└─────────────────────┴───────────────────────────────┴───────────────────┘
```

---

## 33. Runbook (operator triage by error class)

### 33.1 "Style" failure

  - Symptom : Job 02 red.
  - Triage : `pre-commit run --all-files` locally.  Most fixes are
    1-line edits (line length, trailing whitespace).
  - Common cause : `helm-unittest` test files have long inline scripts
    that trip line-length.  Fix : extract to a `command: |-` block.

### 33.2 "helm lint" failure

  - Symptom : Job 03 red.
  - Triage : `helm lint charts/efe-stack/charts/efe-meta-swarm --strict`
    locally.  Often `--strict` flags missing required values that the
    default mode doesn't catch.
  - Common cause : `values.schema.json` declared a `required: [X]` field
    but values.yaml doesn't supply X.

### 33.3 "Conftest" failure

  - Symptom : Job 08 red.
  - Triage : the failure output names the exact Rego rule that fired,
    e.g. `Conftest test failed : container .../snapshot has no
    resources.limits (rule: container_resources)`.
  - Find the rule under `policies/rego/efe/` and the offending
    template ; fix the template (preferred) or add an exception under
    `policies/rego/efe/data/`.

### 33.4 "kubeconform 1.31" only failure

  - Symptom : 3 of 4 matrix entries pass, 1.31 fails.
  - Common cause : we use a deprecated apiVersion that 1.31 removed
    (e.g. `policy/v1beta1` PodDisruptionBudget — already migrated).
  - Triage : run `kubectl explain <kind> --api-version=<...>` against
    a local 1.31 cluster ; bump the apiVersion.

### 33.5 "Polaris" warning-tier overflow

  - Symptom : warning artifacts blow up to 100+ findings.
  - Common cause : a new template adds a container without `resources`.
  - Triage : check `tests/golden/*.yaml` for the new template ; add
    resources to the values file or the template.

---

## 34. Threat-model coverage (T29-T35)

| Threat | Description                                              | Mitigation                                                                 |
|--------|----------------------------------------------------------|----------------------------------------------------------------------------|
| T29    | Supply-chain : un-pinned linter image becomes malicious  | Every tool pinned by tag + digest (AD3).  Renovate-managed bumps.          |
| T30    | Silent rule drift : tool upgrade changes semantics       | Renovate PRs → same lint suite catches the drift on the upgrade PR itself  |
| T31    | False-positive fatigue                                   | Severity tiering (Polaris).  Soft-fail for Checkov.  Audit-reason for `.trivyignore` entries. |
| T32    | Lint bypass via admin override                           | Audit-log entry.  Weekly SRE review.  No `[skip lint]` magic.              |
| T33    | PR comment spam from repeated pushes                     | Sticky comment edits in-place (AD13).                                      |
| T34    | SBOM tampering                                           | SBOM emitted from CI runner only.  Signed via GitHub OIDC (PR #16 scope).  |
| T35    | Concurrency leaks (stale runners holding artifacts)      | `cancel-in-progress: true` + 7-day retention cap (AD28).                   |

---

## 35. Tests strategy (this workflow tests itself)

PR #15 adds two new test surfaces :

  1. **Conftest unit tests** (`*_test.rego`) — `conftest verify` runs
     these.  Every Rego rule has a `_test.rego` file with at least one
     `deny` case and one `allow` case.

  2. **Workflow self-test** — `tests/workflow/lint-self-test.bats` (NEW).
     A BATS script that introduces deliberate violations into each
     scenario's rendered output and asserts the right Rego rule fires.
     Run locally (not in CI) before merging changes to the policy library.

  Example assertion :

  ```bash
  @test "tenant_label fires on missing label" {
    rendered=$(helm template test ... | yq 'del(.metadata.labels.efe.io/tenant)')
    run conftest test --policy policies/rego/efe --no-color <<< "$rendered"
    [ "$status" -ne 0 ]
    [[ "$output" =~ "missing efe.io/tenant" ]]
  }
  ```

---

## 36. Acceptance criteria (AC1–AC35)

| AC   | Statement                                                                                                                                          | Status   |
|------|----------------------------------------------------------------------------------------------------------------------------------------------------|----------|
| AC1  | `helm lint --strict` passes against `values.yaml` + all 6 scenarios + all 4 example overlays.                                                       | gate     |
| AC2  | `chart-testing ct lint` passes ; `Chart.yaml` version bump required on every PR touching the chart.                                                | gate     |
| AC3  | `yamllint -c .yamllint` passes against every YAML in the chart (excluding golden snapshots + Grafana dashboard JSON).                              | gate     |
| AC4  | `kubeconform -strict` passes for K8s 1.28.0, 1.29.0, 1.30.0, AND 1.31.0 across all 6 scenarios.                                                     | gate     |
| AC5  | `conftest test` passes against all 6 scenarios with the 11 EFE Rego rules + CIS K8s top-10.                                                         | gate     |
| AC6  | Trivy IaC scan returns 0 HIGH/CRITICAL findings.                                                                                                    | gate     |
| AC7  | Checkov scan completes (soft-fail) ; SARIF uploaded to Code Scanning.                                                                                | informational |
| AC8  | Polaris severity=danger : 0 findings across all 6 scenarios + 4 example overlays.                                                                   | gate     |
| AC9  | SBOM (CycloneDX 1.5 JSON) generated, uploaded as artifact (30-day retention).                                                                       | informational |
| AC10 | `ah lint` passes against `artifacthub-pkg.yml`.                                                                                                     | gate     |
| AC11 | `--debug --dry-run` rendering of all 6 scenarios completes in under 60 s wall-clock.                                                                | informational |
| AC12 | Concurrency : pushing a new commit to a PR cancels the previous lint run within 10 s.                                                               | gate     |
| AC13 | PR receives exactly one sticky comment per workflow run ; subsequent pushes edit the same comment in-place.                                          | gate     |
| AC14 | Every tool image is pinned by tag (AD3) ; no `:latest` references in the workflow.                                                                  | gate     |
| AC15 | Workflow can be triggered via `workflow_dispatch` with `debug: true` ; tools run with `-v` / `--debug` flags.                                       | informational |
| AC16 | Every Conftest finding produces a SARIF annotation visible on the PR Files tab.                                                                     | gate     |
| AC17 | `workflow_call` interface exposes `lint-result` (pass/fail) + `sbom-artifact-name` ; signature stable across PRs.                                    | gate     |
| AC18 | `helm-docs --check` passes ; README.md is byte-identical to `helm-docs` regeneration.                                                                | soft     |
| AC19 | `pre-commit run --all-files` locally produces the same verdict as CI Job 02.                                                                        | informational |
| AC20 | `ajv compile` against the 2020-12 meta-schema accepts `values.schema.json` ; example overlays validate against the schema.                          | gate     |
| AC21 | p99 workflow wall-clock ≤ 5 minutes.                                                                                                                | SLO      |
| AC22 | Zero false-positives across the PR #14 head-of-main scenarios (6 kovant + 4 examples = 10 inputs ; all green).                                       | gate     |
| AC23 | Failure of any of the 5 required jobs blocks merge.                                                                                                  | gate     |
| AC24 | Admin override requires the `repo-admin` GitHub role + leaves an audit-log entry.                                                                   | gate     |
| AC25 | Workflow accesses NO cluster credentials.  `GITHUB_TOKEN` permissions are read-only except for `pull-requests: write` (sticky comment).             | gate     |
| AC26 | Every tool version tracked in `renovate.json` ; Renovate PRs flow through this same lint suite (fixed-point).                                       | gate     |
| AC27 | The 11 EFE Rego rules enforce : tenant label, no `:latest`, securityContext, no privileged, no host namespace, SA-token disabled on runtime pods, shareProcessNamespace gated on sidecar, Service appProtocol, container resources, NetworkPolicy policyTypes, alert runbook_url.  Coverage : every rule has a `_test.rego` with pass + fail cases. | gate     |
| AC28 | Captured artifacts on failure : rendered manifests (7d), Conftest SARIF (7d), Polaris JSON (7d), SBOM (30d).                                         | gate     |
| AC29 | Custom EFE policies fire correctly : a deliberately-broken render of each scenario produces the expected `deny[msg]`.  Asserted by `tests/workflow/lint-self-test.bats`. | gate     |
| AC30 | New chart files trigger automatic re-lint via `paths:` filter ; no commits to unrelated files trigger this workflow.                                | gate     |
| AC31 | Sub-chart-only path filter : sibling sub-chart edits (e.g. `charts/efe-stack/charts/efe-server/**`) do NOT trigger this workflow.                    | gate     |
| AC32 | Reusable workflow signature (`workflow_call`) is stable : umbrella's `helm-stack-test.yml` invokes it without modifications.                          | gate     |
| AC33 | Tool-version drift between `renovate.json` and the workflow file's `env:` block is detected by a lint step in this same workflow.                    | gate     |
| AC34 | Every CI annotation includes a doc-link to the relevant runbook section (§ 33).                                                                      | soft     |
| AC35 | The sticky-summary comment includes ALL 14 stages, in order, with their pass/fail/skip verdict.                                                       | gate     |

---

## 37. Risks + mitigations

| Risk                                                                                  | Likelihood | Impact | Mitigation                                                                 |
|---------------------------------------------------------------------------------------|------------|--------|----------------------------------------------------------------------------|
| Tool version upstream rotation breaks the workflow at midnight                        | Medium     | Medium | Renovate weekly auto-PR ; pin digests not just tags.                       |
| Polaris severity re-tiering after an upgrade fails old charts                         | Low        | High   | `polaris.yaml` snapshots severity tier ; upgrade-PR triggers re-lint.      |
| Rego rule too strict → false positives                                                 | Medium     | Medium | Rule unit tests (`*_test.rego`) ; exception data file under `policies/rego/efe/data/`. |
| Lint runtime drift > 5 min                                                            | Low        | Low    | Per-job timeout-minutes ; SLO tracked in AC21.                             |
| `helm-docs` regeneration desyncs README on Windows CRLF                               | Low        | Low    | `.gitattributes` enforces LF.                                              |
| SARIF schema upstream change                                                          | Low        | Low    | Use `github/codeql-action/upload-sarif@v3` (versioned).                    |
| Operator-side `make helm-docs` rejection (Docker not installed)                       | Medium     | Low    | Provide non-Docker fallback : `helm-docs` binary download in Makefile.     |
| Concurrency cancellation cancels long-running matrix entry mid-execution              | Low        | Low    | Idempotent jobs ; no shared state ; re-run is safe.                        |
| GitHub Actions outage during a release                                                | Low        | High   | Branch protection allows admin override with audit trail (AD25).           |

---

## 38. Rollback strategy

If PR #15 lands and immediately fails on `main` :

  1. Hotfix : disable required-status-check on the broken job in GitHub
     branch protection settings (admin role required).
  2. Revert the PR (`git revert <merge-sha>`).
  3. The workflow file is removed ; subsequent PRs re-trigger the
     existing PR #14 workflow only.

Branch protection edit is logged ; SRE post-mortem reviews within
24h.

The 6 commits in PR #15 are each independently revertable (atomic
units : workflow-skeleton, schema/docs/multi-K8s, policy-as-code,
security scans, provenance, sticky-comment).

---

## 39. Sequence diagrams

### 39.1 PR push → lint completion

```text
PR author                GitHub Actions               Sticky comment
   │                            │                            │
   ├── push commit ──>          │                            │
   │                            ├─ trigger workflow          │
   │                            ├─ cancel previous run       │
   │                            │     (concurrency group)    │
   │                            ├─ Job 01 bootstrap          │
   │                            │  └─ render 10 scenarios    │
   │                            │  └─ upload artifact        │
   │                            ├─ Jobs 02-13 fan-out        │
   │                            │  (parallel)                │
   │                            │    │                       │
   │                            │    ├─ Job 02 style         │
   │                            │    ├─ Job 03 helm lint     │
   │                            │    ├─ Job 04 ct lint       │
   │                            │    ├─ Job 05 schema-of-sc  │
   │                            │    ├─ Job 06 helm-docs     │
   │                            │    ├─ Job 07 kubeconform × 4│
   │                            │    ├─ Job 08 Conftest      │
   │                            │    ├─ Job 09 Trivy         │
   │                            │    ├─ Job 10 Checkov       │
   │                            │    ├─ Job 11 Polaris       │
   │                            │    ├─ Job 12 ArtifactHub   │
   │                            │    └─ Job 13 SBOM          │
   │                            ├─ Job 14 sticky-summary     │
   │                            │  ├─ compose verdict        │
   │                            │  └─ edit comment ──>       │
   │                            │                            ├─ rendered
   │  <── sticky comment ──     │                            │   on PR
   │      visible on PR         │                            │
   │                            │                            │
```

### 39.2 Rego rule firing flow

```text
Job 08 Conftest:
   │
   ├─ conftest verify (rule unit tests pass first)
   │
   ├─ for each rendered.yaml:
   │  ├─ conftest test --policy policies/rego/efe/
   │  │  ├─ parse manifest (YAML → JSON)
   │  │  ├─ for each Rego rule:
   │  │  │  ├─ evaluate deny[msg] {...}
   │  │  │  └─ collect msg if rule fires
   │  │  └─ aggregate findings per file
   │  └─ output : pretty stdout + SARIF (on failure)
   │
   └─ if any finding:
      ├─ exit 1 → job fails
      ├─ upload SARIF → Code Scanning
      └─ Job 14 sticky-summary reports 'fail'
```

---

## 40. Reviewer checklist

  - [ ] **AD-coverage** : every AD has a corresponding workflow stanza
    or config-file content that implements it.
  - [ ] **AC-coverage** : every AC is either a workflow assertion
    (job pass/fail) or an artifact uploaded for retention.
  - [ ] **Tool pin** : grep for `:latest` in
    `.github/workflows/helm-meta-swarm-lint.yml` returns nothing.
  - [ ] **Renovate** : `renovate.json` `regexManagers:` entries cover
    every `env.X_VERSION` line.
  - [ ] **Rego unit tests** : `conftest verify --policy
    policies/rego/efe` returns 0 failures locally.
  - [ ] **Self-test** : `bats tests/workflow/lint-self-test.bats`
    passes locally.
  - [ ] **Workflow timing** : a sample run on a recent PR clocks
    ≤ 5 minutes wall-clock.
  - [ ] **Sticky comment** : verified on a draft PR — comment edits
    in place, doesn't spawn duplicates.
  - [ ] **Branch protection** : the 5 required status checks are
    configured in the repo settings (cross-checked against
    `terraform/github/`).
  - [ ] **Reusable workflow** : umbrella's `helm-stack-test.yml` is
    invoking PR #15's workflow via `workflow_call` with the documented
    signature.
  - [ ] **No cluster credential access** : the workflow's `permissions:`
    block has only `contents: read` + `pull-requests: write`.
  - [ ] **Audit trail** : `repo-admin` override path tested on a
    feature branch ; audit-log entry visible.

---

## 41. Appendix A — tool version pinning table

| Tool                   | Pin              | Source                                                  |
|------------------------|------------------|---------------------------------------------------------|
| Helm                   | `v3.13.3`        | `github.com/helm/helm` releases                         |
| kubeconform            | `v0.6.4`         | `github.com/yannh/kubeconform` releases                 |
| chart-testing          | `v3.10.1`        | `github.com/helm/chart-testing` releases                |
| Conftest               | `v0.49.1`        | `github.com/open-policy-agent/conftest` releases        |
| Trivy                  | `v0.49.1`        | `github.com/aquasecurity/trivy` releases                |
| Checkov                | `3.1.79`         | PyPI                                                    |
| Polaris                | `8.5.5`          | `github.com/FairwindsOps/polaris` releases              |
| helm-docs              | `v1.12.0`        | `github.com/norwoodj/helm-docs` releases                |
| ah (ArtifactHub)       | `v1.18.0`        | `github.com/artifacthub/hub` releases                   |
| Syft                   | `v1.0.1`         | `github.com/anchore/syft` releases                      |
| ajv-cli                | `5.0.0`          | npm                                                     |
| pre-commit             | `3.6.0`          | PyPI                                                    |
| yamllint               | `1.35.1`         | PyPI                                                    |
| shellcheck             | `v0.10.0`        | `github.com/koalaman/shellcheck` releases               |
| sticky-pull-request-comment | `v2.9.0`    | `github.com/marocchino/sticky-pull-request-comment`     |
| trivy-action           | `0.20.0`         | `github.com/aquasecurity/trivy-action`                  |
| chart-testing-action   | `v2.6.1`         | `github.com/helm/chart-testing-action`                  |
| setup-helm             | `v4`             | `github.com/azure/setup-helm`                           |
| setup-python           | `v5`             | `github.com/actions/setup-python`                       |
| setup-node             | `v4`             | `github.com/actions/setup-node`                         |
| cache                  | `v4`             | `github.com/actions/cache`                              |
| checkout               | `v4`             | `github.com/actions/checkout`                           |
| upload-artifact        | `v4`             | `github.com/actions/upload-artifact`                    |
| download-artifact      | `v4`             | `github.com/actions/download-artifact`                  |
| upload-sarif (codeql)  | `v3`             | `github.com/github/codeql-action`                       |

---

## 42. Appendix B — environment variable matrix

Per-job ENV access surface (zero secrets ; all read from public sources) :

| Variable             | Job(s) needing it                  | Source                              |
|----------------------|------------------------------------|-------------------------------------|
| `HELM_VERSION`       | 01, 03, 04, 06                     | workflow `env:`                     |
| `KUBECONFORM_VERSION`| 07                                 | workflow `env:`                     |
| `CHART_TESTING_VER`  | 04                                 | workflow `env:`                     |
| `CONFTEST_VERSION`   | 08                                 | workflow `env:`                     |
| `TRIVY_VERSION`      | 09                                 | workflow `env:`                     |
| `CHECKOV_VERSION`    | 10                                 | workflow `env:`                     |
| `POLARIS_VERSION`    | 11                                 | workflow `env:`                     |
| `HELM_DOCS_VERSION`  | 06                                 | workflow `env:`                     |
| `AH_VERSION`         | 12                                 | workflow `env:`                     |
| `SYFT_VERSION`       | 13                                 | workflow `env:`                     |
| `AJV_VERSION`        | 05                                 | workflow `env:`                     |
| `PYTHON_VERSION`     | 02, 10                             | workflow `env:`                     |
| `KUBE_VERSIONS`      | 07 (matrix expansion)              | workflow `env:`                     |
| `GITHUB_TOKEN`       | 14 (sticky comment), 09/10 (SARIF) | GitHub provided                     |

`GITHUB_TOKEN` permissions (workflow-level `permissions:` block) :

```yaml
permissions:
  contents:       read
  pull-requests:  write   # sticky comment
  security-events: write  # SARIF upload
  actions:        read    # for workflow_call recursion
```

No write to `contents`, no access to `secrets:`.

---

## 43. Appendix C — Rego rule index

| Rule file                       | What it enforces                                        | Severity | Test file                          |
|---------------------------------|---------------------------------------------------------|----------|------------------------------------|
| `tenant_label.rego`             | every namespaced resource has `efe.io/tenant`           | deny     | `tenant_label_test.rego`           |
| `no_latest_tag.rego`            | image tags must be pinned                               | deny     | `no_latest_tag_test.rego`          |
| `security_context.rego`         | every container has `securityContext`                   | deny     | `security_context_test.rego`       |
| `no_privileged.rego`            | no `privileged: true`                                   | deny     | `no_privileged_test.rego`          |
| `no_host_namespace.rego`        | no `hostNetwork/hostPID/hostIPC: true`                  | deny     | `no_host_namespace_test.rego`      |
| `sa_token.rego`                 | runtime pods reject SA-token mount                      | deny     | `sa_token_test.rego`               |
| `share_process_namespace.rego`  | gated on `bundle-loader` container presence             | deny     | `share_process_namespace_test.rego`|
| `service_appprotocol.rego`      | every Service port has `appProtocol`                    | deny     | `service_appprotocol_test.rego`    |
| `container_resources.rego`      | `resources.requests` + `resources.limits` both required | deny     | `container_resources_test.rego`    |
| `networkpolicy_types.rego`      | `policyTypes` set on every NetworkPolicy                | deny     | `networkpolicy_types_test.rego`    |
| `prometheusrule_runbook.rego`   | every alert has `annotations.runbook_url`               | deny     | `prometheusrule_runbook_test.rego` |
| `kubernetes.rego` (CIS top-10)  | aggregator — vendored CIS K8s benchmark v1.8            | mixed    | `kubernetes_test.rego`             |

11 EFE-specific + 1 CIS aggregator = 12 Rego files.  Each rule's
test file has ≥ 1 deny case + ≥ 1 allow case (24+ test assertions
total).

---

## End of architecture document
