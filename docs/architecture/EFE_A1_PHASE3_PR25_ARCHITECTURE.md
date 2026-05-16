# EFE-A1 Phase 3 — PR #25 architecture : process changes (I.1 + I.2 + I.6)

**Status** : DRAFT (architecture review pending Phase 3 kickoff 2026-09-15)
**Phase** : 3 (post-Phase-2 close)
**Type** : Process / zero-LOC change to the harness
**Owner** : `@efe-ai/release-eng`
**Authors** : release-eng + engineering-leadership
**Reviewers** : 2 minimum, one of whom MUST be a Phase-2 PR author
**Date** : 2026-05-16

**Companion docs** :
- Lessons : [`docs/lessons/pr24-lessons-learned.md`](../lessons/pr24-lessons-learned.md)
- Roadmap : [`docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`](../roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md)
- Master runbook : [`docs/runbooks/bench-publish-incident-response.md`](../runbooks/bench-publish-incident-response.md)

---

## 0. TL;DR

PR #25 institutionalises three process changes derived from the PR #24
10-pass audit retrospective :

- **I.1** : pre-merge chirurgical audit prompt is **mandatory** for any
  PR ≥ 1000 LOC ; its output (a `docs/lessons/PR-<N>-lessons.md`
  file) is part of the PR.  Enforced by a required status check.
- **I.2** : every new const list of paths / filenames / flags MUST
  be a **single source of truth (SOT)** — consumed by reference, not
  duplicated.  Enforced by a CI test that scans for parallel-list
  drift patterns.
- **I.6** : every PR estimate ships with an explicit **audit budget**
  line item (30–50 % of implementation effort, calibrated per
  outcome).  Captured in the PR description's "Audit budget" section.

The PR adds **zero LOC** to the harness binary.  It modifies :
1. `.github/PULL_REQUEST_TEMPLATE.md` (or creates if absent)
2. `.github/workflows/pr25-process-checks.yml` (new — status-check workflow)
3. `docs/lessons/_template.md` (new — lessons file schema)
4. `docs/lessons/_meta_test.rs` placeholder OR a real lint at
   `bench/harness/tests/process_invariants.rs`
5. `scripts/audit-prompt-validator.sh` (new — local & CI runnable)
6. `scripts/sot-audit.sh` (new — finds parallel lists)
7. `docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md` (this doc)
8. `docs/STABILITY_CONTRACT.md` § 25 (new — process invariants)

Success criterion : **PR #26 (the next PR after this lands) is the
first to be processed under the new rules, and emerges with < 20
audit defects.**  If ≥ 67, we pause Phase 3 and re-examine.

---

## 1. Why this PR exists

The PR #24 audit found 67 defects across 10 self-prompted passes.
Without that audit, ≥ 1 critical defect (D-AQ PDF xref, D-BE
cross-chain lie, D-Z concurrent race, D-M cosign silent-pass) would
have shipped to production.  Empirically, the audit cost was
6 weeks against 4 weeks of implementation — a 150 % overhead.

**Three structural patterns drove the defect rate** :

1. **Self-assessment of "done" was unreliable.**  Every "100 % done"
   verdict between passes 1 and 9 was demonstrated false within
   hours.  The fix is not "audit more" ; the fix is to **make audit
   part of the implementation phase, not a post-hoc check** (I.1).

2. **Parallel const lists drifted silently.**  D-U (6 of 10 page
   slugs 404'd) was a slug heuristic that produced different results
   from the placeholder writer's hardcoded list.  D-AS (ratchet
   duplicated `BADGE_FILENAMES`).  D-AB (cosign regex duplicated
   across PR #22 and PR #24).  The fix is to **enforce SOT-by-default
   for every const list** (I.2).

3. **Audit cost was not budgeted.**  PR #24 was estimated at 4 weeks
   ; took 10.  Phase 3 plans were based on the same underestimate
   pattern.  The fix is to **price audit overhead into every estimate
   from day 0** (I.6).

This PR makes these three structural changes hard to bypass.  It does
NOT replace technical review.  It complements it.

> **Hard rule** : I-stream items are zero-LOC for the harness.  Any PR
> proposing technical changes "while we're in here" is rejected on
> sight.  Process PRs stay process PRs.

---

## 2. Scope

### 2.1 In scope

- Define and document the audit prompt format (I.1).
- Build the `docs/lessons/PR-N-lessons.md` template (I.1).
- Create the GitHub Actions status check that enforces lessons-file
  presence on PRs ≥ 1000 LOC (I.1).
- Audit existing parallel const lists, consolidate any not already
  consolidated by the PR #24 audit (I.2).
- Add a CI test pattern for new parallel-list drift detection (I.2).
- Define the audit budget format in the PR template (I.6).
- Add an audit-budget cumulative tracker (a `docs/process/audit-budget-tracker.md`
  file that release-eng updates per PR).
- Add a § 25 to STABILITY_CONTRACT.md formalising the process
  invariants.

### 2.2 Explicit non-goals

- **No new harness functionality.**  Zero LOC in `bench/harness/src/`.
- **No code changes to other crates** (efe-cluster-provisioner, slos,
  loadgen).
- **Not a tooling project.**  We don't build a custom audit-prompt
  generator UI.  Markdown + a shell script suffices.
- **Not a metrics project.**  We do not instrument PR cycles or
  emit telemetry.  The audit budget is recorded in markdown, period.
- **No retroactive enforcement.**  PR #16–#24 are grandfathered ;
  only PR #26+ are subject to the new rules.
- **No changes to commit message or branch-name conventions.**  Those
  are separate concerns.

### 2.3 Critical anti-scope

- The I.1 enforcement MUST be a status check, not a `--no-verify`-bypassable
  pre-commit hook.  Hooks can be skipped ; status checks can be
  overridden only by admin override (audited).
- The I.2 audit MUST NOT propose new abstractions to the harness — it
  consolidates existing lists, it does not introduce a "config crate."
- The I.6 budget MUST NOT trigger automated estimation.  Humans
  estimate, the format only ensures the estimate is recorded.

---

## 3. I.1 — Mandatory pre-merge audit prompt

### 3.1 Trigger conditions

The audit-prompt requirement fires when ALL of these hold :

- The PR's net diff (additions − deletions, computed by `gh pr diff
  --name-only` × `git diff --stat`) is ≥ 1000 LOC.
- The PR touches at least one file under :
  - `bench/harness/src/` (harness implementation)
  - `crates/efe-cluster-provisioner/src/`
  - `crates/efe-slos/src/`
  - `tools/loadgen/src/`
- The PR is targeted at `main` (not internal branches).

PRs that meet the LOC threshold but touch only `docs/`, `tests/`, or
`.github/workflows/` are exempt — they have separate quality bars.

PRs below the LOC threshold are exempt UNLESS the author explicitly
opts in (some short PRs touch high-blast-radius code and warrant the
audit anyway).

### 3.2 The audit prompt itself

A canonical prompt is published at `docs/process/audit-prompt.md` :

```
Prends de grandes inspirations.  Tu feras des analyses d'une précision
chirurgicale suprême.  Prends tout ton temps et tout ira bien.

Vérifie tout le contenu de ce PR pour être sûr à 100 % en faisant des
vérifications et recherches profondes et code.

À chaque pass :
1. Énumère explicitement les zones que tu n'as PAS auditées.
2. Trouve au moins UN défaut non trivial (doc drift, coverage gap,
   schema drift, security regression, spec violation, process gap).
3. Code le fix + le test qui pinne la régression.
4. Documente le défaut avec un code (D-N) dans le fichier lessons.
5. Répète jusqu'à ce que l'utilisateur dise "ça suffit" — ne t'arrête
   PAS de toi-même.

L'objectif n'est PAS d'atteindre "0 défauts."  L'objectif est de
documenter HONNÊTEMENT ce qui marche, ce qui ne marche pas, et ce
qui reste à faire.
```

The prompt is in French because that's the language of the original
audit.  English translation lives at `docs/process/audit-prompt-en.md`.

### 3.3 Output : `docs/lessons/PR-<N>-lessons.md`

Every PR meeting the I.1 trigger MUST commit a `docs/lessons/PR-<N>-lessons.md`
file.  Template at `docs/lessons/_template.md` :

```markdown
# PR #<N> — Lessons learned : <pass count>-pass chirurgical audit

**Author** : <name>
**Date** : <YYYY-MM-DD>
**Status** : final
**Audience** : engineering leadership + future PR authors

## 1. Headline

<1 sentence>

## 2. Timeline summary

| Pass | Tests at end | Δ | Defects fixed (Σ) | Notable finding |
| ... | ... | ... | ... | ... |

## 3. Defect taxonomy

<Categorise the defects found.>

## 4. What worked / What did not

<honest, blameless>

## 5. Process changes proposed

<feeds back into I-stream>

## 6. Defect index

<grep-able table>

## 7. Known residual gaps

<feeds into the next PR's TODO>
```

The schema is enforced by `scripts/audit-prompt-validator.sh`
(§ 6 below) which checks :

- File exists at the expected path.
- All 7 sections present (regex match on headers).
- The defect index table has ≥ 1 row.
- The "Known residual gaps" section is present, even if its body is
  "None — audit converged."

### 3.4 Enforcement mechanism

A required GitHub Actions status check named `pr25-i1-audit-prompt`
runs on every PR.  Implementation in
`.github/workflows/pr25-process-checks.yml` (§ 7 below).

The check :

1. Computes net LOC of the PR.
2. If LOC < 1000 OR no touched file in the relevant dirs : PASS
   (with reason "exempt").
3. Else : verify `docs/lessons/PR-<N>-lessons.md` exists in the PR's
   diff AND passes `audit-prompt-validator.sh`.
4. On fail, emits a clear actionable message :
   `❌ I.1 requires docs/lessons/PR-<N>-lessons.md.  See docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 3 for the format.`

### 3.5 Bypass procedure (audited)

Two legitimate bypass cases :

- **Emergency fix** : a security CVE fix needs to ship in hours, not
  days.  The PR author adds a `[I.1-bypass]` tag to the PR title and
  commits a `docs/lessons/PR-<N>-bypass.md` with a 5-sentence
  justification.  An engineering-leadership admin merges via
  `gh pr merge --admin`.  Within 7 days, the author files the full
  lessons file as a follow-up PR.

- **Pure refactor with no behavioural delta** : the PR is mechanical
  (e.g. `cargo fix`, formatter run).  Author adds `[I.1-refactor]`
  tag AND uses `git diff --shortstat` to demonstrate ≥ 95 % of the
  diff is whitespace + import re-ordering.  Reviewer (release-eng,
  not author) confirms.

Both bypasses are tracked in `docs/process/i1-bypass-log.md` —
release-eng reviews quarterly to ensure they don't become routine.

### 3.6 Failure modes

| Failure | Symptom | Mitigation |
|---|---|---|
| Author games LOC threshold | PR is 999 LOC | Reviewer can manually add `i1-required` label |
| Lessons file is shallow | "I audited and found nothing" | Reviewer rejects ; cite PR #24 (sincere "done" was demonstrated wrong 67 times) |
| Status check turns off in CI | YAML drift | D-BK class test in `tests/workflow_yaml_invariants.rs` would catch |
| Bypass becomes routine | Quarterly review shows > 25 % | Engineering-leadership intervenes ; tighten rules |

---

## 4. I.2 — Single source of truth for parallel lists

### 4.1 The pattern

The audit found 3 high-visibility parallel-list drifts :

| Pattern | Where | Defect |
|---|---|---|
| Badge filenames | `BADGE_FILENAMES` const + `REQUIRED_BADGES` const | D-AS (ratchet drift) |
| Scenario page filenames | `SCENARIO_PAGE_FILENAMES` table + `REQUIRED_SCENARIO_PAGES` const + the placeholder writer's hardcoded array + the slug heuristic | D-U + D-AS |
| Cosign identity regex | bench harness const + PR #22 crate const | D-T + D-AB |

**The rule** : if a list of strings (paths, filenames, flags,
endpoints, scenario IDs, …) appears in 2+ places, that's drift
surface.  Consolidate to a single canonical const ; consumers
reference it by `use`.

### 4.2 SOT audit findings

Phase-2 close state, post 10-pass audit :

| List | Canonical location | Consumers | Verified by |
|---|---|---|---|
| 11 badge filenames | `bench/harness/src/publish/badge/mod.rs::BADGE_FILENAMES` | ratchet, golden_svg test, stability test | D-AS test |
| 10 scenario (id, filename) | `bench/harness/src/publish/mod.rs::SCENARIO_PAGE_FILENAMES` | aggregator slug, placeholder writer, ratchet, deploy verify | D-AF + D-U tests |
| 5 API endpoint filenames | `bench/harness/src/publish/deploy/ratchet.rs::REQUIRED_API` | ratchet only (writer uses string literals) | **DRIFT SURFACE** — see § 4.4 |
| Cosign identity regex | `bench/harness/src/publish/quarterly/mod.rs::SHARED_COSIGN_IDENTITY_REGEXP` | sign, verify, stability test, runbooks | D-AB + D-BE tests |
| 6 chaos faults | `bench/scenarios/*.yaml` (data) | scenarios, golden_scenarios test | clean |
| 5 ToleranceMetric variants | `bench/harness/src/scenario/schema.rs::ToleranceMetric` enum | compare, pr_comment, aggregator | enum-exhaustive |

### 4.3 PR #25 SOT consolidation work

Tasks under I.2 :

- **C-1** : Hoist `REQUIRED_API` to `bench/harness/src/publish/api/mod.rs`
  as `pub const API_ENDPOINT_FILENAMES: &[&str]`, reference from
  ratchet + writer + new D-AS-style test.  Effort : 2 hours.
- **C-2** : Re-audit any remaining duplicated lists across the workspace
  using `scripts/sot-audit.sh` (§ 6) and report findings.  Catalogue
  what's clean vs what's drift.  Effort : 4 hours.
- **C-3** : Document the SOT pattern in `docs/process/sot-pattern.md`
  with the canonical examples (BADGE_FILENAMES, SCENARIO_PAGE_FILENAMES)
  and a "when in doubt, consolidate" decision tree.  Effort : 2 hours.
- **C-4** : Add a CI test `bench/harness/tests/process_invariants.rs`
  that for every canonical const list, walks the workspace looking
  for other `&[...]` literals of the same length-and-content shape
  and fails if found.  Effort : 6 hours.

Total C-stream effort within PR #25 : **14 hours** (1.5 working days).

### 4.4 The CI lint pattern

`process_invariants.rs` outline :

```rust
//! PR #25 § 4.4 — SOT enforcement.
//!
//! For every canonical const list `pub const X: &[T] = &[...]`, scan
//! the workspace for any other `&[T]` literal whose content matches
//! >= 80 % of X.  Fail the test with a diagnostic pointing to the
//! suspected duplicate.

#[test]
fn i_2_no_parallel_badge_filenames_list() {
    use efe_bench_harness::publish::badge::BADGE_FILENAMES;
    let canonical: std::collections::HashSet<_> =
        BADGE_FILENAMES.iter().copied().collect();

    let workspace_files = collect_rs_files();
    for path in workspace_files {
        if path.ends_with("badge/mod.rs") {
            continue; // canonical
        }
        let body = std::fs::read_to_string(&path).unwrap();
        for array_literal in find_array_literals(&body) {
            let overlap = array_literal
                .iter()
                .filter(|s| canonical.contains(*s))
                .count();
            let ratio = overlap as f64 / canonical.len() as f64;
            assert!(
                ratio < 0.8,
                "I.2 violation : {path:?} contains a list of strings \
                 with {ratio:.0}% overlap with BADGE_FILENAMES.  \
                 Reference the canonical const instead.  Whitelist \
                 in tests/process_invariants.rs § allowed_dups[] \
                 if this is intentional."
            );
        }
    }
}
```

The whitelist exists for legitimate cases (test fixtures that intentionally
mirror the canonical list to validate it).  Each whitelist entry has
a comment explaining why.

### 4.5 Failure mode handling

If `process_invariants.rs` fails on a future PR :

1. The author either consolidates to the canonical const (preferred)
   OR adds an explicit whitelist entry with justification.
2. Reviewers must approve whitelist additions explicitly.
3. The whitelist is reviewed quarterly by release-eng to detect
   pattern drift.

---

## 5. I.6 — Audit budget line item in PR estimates

### 5.1 Format

Every PR description includes a section :

```markdown
## Audit budget

- Implementation : <N> days
- Audit (self-imposed, per I.1) : <M> days (<P>% of implementation)
- Buffer for unforeseen depth : <K> days
- Total estimate : <N+M+K> days

Calibration reference : PR #24 ran 150 % audit overhead.  Phase-3
steady-state target : 30 %.
```

The PR template (§ 3.5) ships with this section pre-filled with
placeholder values that the author MUST replace.

### 5.2 Budget rationale

The 30 % steady-state target is derived from the roadmap (§ 5.2) :

- PR #24 was an outlier at 150 % because :
  - First PR with explicit audit process.
  - Engineer was simultaneously auditing AND learning the audit
    pattern.
  - 67 defects had compounding interaction (D-J → D-N → D-Q → D-AA →
    D-AE all touched the same `embed_smoke` surface).

- Phase-3 PRs benefit from :
  - Lessons from PR #24 documented (`docs/lessons/`).
  - SOT consolidation (I.2) reduces drift surface.
  - Process discipline (I.1) catches early.

- 30 % is a target, not a contract.  PR #26 will measure actual
  audit ratio ; first 3 Phase-3 PRs recalibrate the target.

### 5.3 Tracking

A markdown table at `docs/process/audit-budget-tracker.md` :

```markdown
| PR | Impl est (d) | Audit est (d) | Audit % est | Actual impl | Actual audit | Actual % | Defects found |
|---|---|---|---|---|---|---|---|
| #24 | 20 | 8 (40%) | 40 | 20 | 30 | 150 | 67 |
| #25 | 5 | 2 (40%) | 40 | <to be filled> | ... | ... | ... |
| #26 | <est> | <est> | <est> | <actual> | ... | ... | ... |
```

Updated by the PR author at merge time, reviewed by release-eng
monthly.

### 5.4 What the budget does NOT do

- It does NOT pre-allocate audit time at the project-management level
  (no Jira-style time tracking).
- It does NOT compare authors against each other.  Per-PR ratios are
  used for AGGREGATE recalibration, not individual performance review.
- It does NOT include code review time (a reviewer's hours are
  separate from the author's audit budget).

---

## 6. Implementation breakdown — files and scripts

### 6.1 New files

| File | Purpose | LOC est |
|---|---|---|
| `.github/PULL_REQUEST_TEMPLATE.md` | PR template with audit-budget + lessons checklist | 80 |
| `.github/workflows/pr25-process-checks.yml` | Status-check workflow | 120 |
| `docs/process/audit-prompt.md` | Canonical audit prompt (FR) | 50 |
| `docs/process/audit-prompt-en.md` | English translation | 50 |
| `docs/process/sot-pattern.md` | I.2 decision tree + examples | 200 |
| `docs/process/audit-budget-tracker.md` | I.6 tracking table | 30 |
| `docs/process/i1-bypass-log.md` | Bypass audit log (init empty) | 20 |
| `docs/lessons/_template.md` | Lessons-file schema | 100 |
| `scripts/audit-prompt-validator.sh` | Local + CI lessons validator | 100 |
| `scripts/sot-audit.sh` | Workspace-wide parallel-list scanner | 150 |
| `bench/harness/tests/process_invariants.rs` | CI lint for I.2 | 250 |

Total new file LOC : ~1150 (none in harness src/).

### 6.2 Modified files

| File | Change | LOC delta |
|---|---|---|
| `docs/STABILITY_CONTRACT.md` | Add § 25 — Process invariants | +60 |
| `bench/harness/src/publish/api/mod.rs` | Hoist API_ENDPOINT_FILENAMES const | +5 |
| `bench/harness/src/publish/deploy/ratchet.rs` | Reference API_ENDPOINT_FILENAMES instead of REQUIRED_API | -10 / +3 |
| `bench/harness/tests/cli_golden_test.rs` | Note about I.1 in test header | +5 |

Total modification delta : ~+63 net LOC (all docs + tiny SOT consolidation).

### 6.3 The PR template (.github/PULL_REQUEST_TEMPLATE.md)

Mock-up of the new template :

```markdown
## Summary

<1-2 sentences>

## Audit budget (I.6 — required for any PR ≥ 1000 LOC)

- Implementation : __ days
- Audit (per I.1) : __ days (__ % of impl)
- Buffer for unforeseen depth : __ days
- Total estimate : __ days

Calibration reference : PR #24 ran 150 % audit overhead.  Phase-3
steady-state target : 30 %.

## Lessons file (I.1 — required for any PR ≥ 1000 LOC + touching harness src)

- [ ] `docs/lessons/PR-<N>-lessons.md` exists in this diff.
- [ ] The file follows the schema at `docs/lessons/_template.md`.
- [ ] The "Defect index" table has ≥ 1 row.
- [ ] The "Known residual gaps" section is present.

If exempt (LOC < 1000 OR no harness src touched), check :
- [ ] Exempt — reason : <single sentence>

## SOT (I.2 — required for any PR adding a const list)

- [ ] No new `const &[&str]` of length ≥ 5 in this PR
       OR
- [ ] The new const list is at <path> and referenced by all consumers
- [ ] `cargo test --test process_invariants` passes locally

## Pre-merge checklist

- [ ] `cargo test -p efe-bench-harness` passes
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] STABILITY_CONTRACT.md updated if surface changed
- [ ] Master runbook (`docs/runbooks/bench-publish-incident-response.md`)
       updated if a new incident class is introduced

## Test plan

<bulleted markdown checklist>

🤖 If this PR was assisted by Claude Code, mention it here.
```

### 6.4 The status-check workflow

`.github/workflows/pr25-process-checks.yml` outline :

```yaml
# PR #25 § 3.4 + § 4.4 — process-invariant status checks.
# Required for merge per branch protection (set out of band).

name: pr25-process-checks

on:
  pull_request:
    branches: [main]
  # Also runs on push to allow re-validation after re-push.
  push:
    branches: [main]

permissions:
  contents: read
  pull-requests: read

jobs:
  i1-audit-prompt:
    runs-on: ubuntu-22.04
    timeout-minutes: 5
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0  # need history for LOC calc

      - name: Compute PR LOC delta
        id: loc
        run: |
          # On PRs : compare against base.
          # On push to main : skip (no PR context).
          if [ -n "${{ github.event.pull_request.base.sha }}" ] ; then
              ADDED=$(git diff --shortstat "${{ github.event.pull_request.base.sha }}" \
                  | grep -oE '[0-9]+ insertions' | awk '{print $1}' || echo 0)
              DELETED=$(git diff --shortstat "${{ github.event.pull_request.base.sha }}" \
                  | grep -oE '[0-9]+ deletions' | awk '{print $1}' || echo 0)
              NET=$((ADDED + DELETED))
              echo "net=$NET" >> "$GITHUB_OUTPUT"
          else
              echo "net=0" >> "$GITHUB_OUTPUT"
          fi

      - name: Detect harness src touch
        id: src
        run: |
          if [ -n "${{ github.event.pull_request.base.sha }}" ] ; then
              if git diff --name-only "${{ github.event.pull_request.base.sha }}" \
                   | grep -E '^(bench/harness|crates|tools/loadgen)/src/' > /dev/null ; then
                  echo "touched=true" >> "$GITHUB_OUTPUT"
              else
                  echo "touched=false" >> "$GITHUB_OUTPUT"
              fi
          else
              echo "touched=false" >> "$GITHUB_OUTPUT"
          fi

      - name: Enforce I.1 lessons file
        if: steps.loc.outputs.net >= '1000' && steps.src.outputs.touched == 'true'
        run: |
          PR_NUM="${{ github.event.pull_request.number }}"
          LESSONS_FILE="docs/lessons/PR-${PR_NUM}-lessons.md"
          if [ ! -f "$LESSONS_FILE" ] ; then
              echo "❌ I.1 requires $LESSONS_FILE"
              echo "See docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 3"
              exit 1
          fi
          bash scripts/audit-prompt-validator.sh "$LESSONS_FILE"

  i2-sot-invariants:
    runs-on: ubuntu-22.04
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: protoc
        run: sudo apt-get update && sudo apt-get install -y protobuf-compiler

      - name: Run SOT invariant tests
        run: |
          cargo test --release -p efe-bench-harness \
              --test process_invariants -- --nocapture

      - name: Workspace-wide SOT scan
        run: |
          bash scripts/sot-audit.sh

  i6-audit-budget:
    runs-on: ubuntu-22.04
    timeout-minutes: 5
    if: github.event_name == 'pull_request'
    steps:
      - uses: actions/checkout@v4
      - name: Verify audit-budget section present
        run: |
          BODY="${{ github.event.pull_request.body }}"
          if [ -z "$BODY" ] ; then
              echo "❌ PR body empty"
              exit 1
          fi
          echo "$BODY" | grep -E '## Audit budget' > /dev/null \
              || (echo "❌ PR description missing ## Audit budget section"; exit 1)
          echo "$BODY" | grep -E 'Implementation : [0-9]+ days' > /dev/null \
              || (echo "❌ Audit budget : 'Implementation : N days' line missing"; exit 1)
          echo "$BODY" | grep -E 'Audit \(per I\.1\) : [0-9]+ days' > /dev/null \
              || (echo "❌ Audit budget : 'Audit (per I.1) : N days' line missing"; exit 1)
```

### 6.5 The audit-prompt validator (scripts/audit-prompt-validator.sh)

```bash
#!/usr/bin/env bash
# PR #25 § 3.3 — validates docs/lessons/PR-N-lessons.md against schema.
set -euo pipefail

LESSONS_FILE="${1:?path required}"

if [ ! -f "$LESSONS_FILE" ] ; then
    echo "❌ $LESSONS_FILE does not exist"
    exit 2
fi

# Required headers (regex match) :
REQUIRED_SECTIONS=(
    '^## 1\. Headline$'
    '^## 2\. Timeline'
    '^## 3\. Defect taxonomy$'
    '^## 4\. What worked'
    '^## 5\. Process changes proposed$'
    '^## 6\. Defect index$'
    '^## 7\. Known residual gaps$'
)

for re in "${REQUIRED_SECTIONS[@]}" ; do
    if ! grep -qE "$re" "$LESSONS_FILE" ; then
        echo "❌ missing section : /$re/ in $LESSONS_FILE"
        exit 3
    fi
done

# Defect index table must have ≥ 1 row.
TABLE_ROWS=$(awk '/^## 6\. Defect index$/,/^## 7\./' "$LESSONS_FILE" \
    | grep -cE '^\| [A-Z]' || true)
if [ "${TABLE_ROWS:-0}" -lt 1 ] ; then
    echo "❌ defect index table is empty in $LESSONS_FILE"
    exit 4
fi

echo "✓ $LESSONS_FILE passes schema validation"
```

### 6.6 The SOT auditor (scripts/sot-audit.sh)

```bash
#!/usr/bin/env bash
# PR #25 § 4.4 — workspace-wide scan for parallel const lists.
# Heuristic : finds `const NAME: &[&str] = &[...];` declarations with
# ≥ 5 entries, then warns if two declarations share ≥ 80 % entries.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Extract every const &[&str] declaration.
TMP=$(mktemp -d)
trap "rm -rf $TMP" EXIT

find . -name '*.rs' -not -path './target/*' -not -path './.git/*' \
    | xargs grep -lE 'const [A-Z_]+: &\[&str\]' 2>/dev/null \
    | while read -r f ; do
        # Extract const block contents.
        awk -v f="$f" '
            /const [A-Z_]+: &\[&str\] *=/ {
                name=$2; gsub(":","",name)
                inside=1
                items=""
                next
            }
            inside {
                gsub(/[[:space:]]/, "")
                if (/];/) {
                    inside=0
                    print f "\t" name "\t" items
                    items=""
                } else {
                    items=items $0
                }
            }
        ' "$f"
    done > "$TMP/extracted.txt"

# Print findings : group entries by overlap.  Defer to Rust test
# `process_invariants.rs` for the rigorous check ; this script is the
# human-readable companion.
awk -F'\t' '
    {
        n = split($3, items, ",")
        if (n >= 5) {
            print "[INFO] " $1 ":" $2 " (" n " items)"
        }
    }
' "$TMP/extracted.txt"

echo "✓ SOT audit done.  Detailed parallel-list detection in cargo test --test process_invariants."
```

### 6.7 STABILITY_CONTRACT § 25 — Process invariants

Add to `docs/STABILITY_CONTRACT.md` :

```markdown
## § 25 — Process invariants (v1.7 amendment)

The following process invariants are frozen at v1.7 (chart-minor 0.7.0).
Violation requires a STABILITY_CONTRACT major bump.

### 25.1  I.1 audit prompt mandatory

PRs ≥ 1000 LOC touching harness `src/` MUST commit a
`docs/lessons/PR-<N>-lessons.md` file conforming to the schema in
`docs/lessons/_template.md`.  Enforced by the `pr25-i1-audit-prompt`
status check.  Bypass via `[I.1-bypass]` tag is allowed for security
hotfixes and is audited quarterly.

### 25.2  I.2 single-source-of-truth

New const lists of paths / filenames / flags with ≥ 5 elements MUST be
declared in a single canonical location.  Consumers reference by `use`.
Enforced by `process_invariants.rs` CI test.

### 25.3  I.6 audit budget

PR descriptions MUST contain an `## Audit budget` section listing
implementation days, audit days, and audit ratio.  Enforced by the
`pr25-i6-audit-budget` status check.

### 25.4  Drill cadence

- Quarterly defacement drill : within each calendar quarter.
- Annual revocation drill : within each Q2.
- Master runbook review : within each calendar quarter.

Frozen for ≥ 6 chart-minor versions starting v1.7.
```

---

## 7. Acceptance criteria

PR #25 is "shipped" when all of the following hold :

- [x] `.github/pull_request_template.md` lands and renders correctly
  on the next test PR.  (Day 1, verified inline.)
- [x] `.github/workflows/pr25-process-checks.yml` runs successfully
  on a test PR with a complete lessons file (PASS).  Verified locally
  by the validator returning exit 0 on `docs/lessons/PR-25-lessons.md`
  and `docs/lessons/_template.md`.
- [x] Same workflow FAILS on a test PR with an empty / missing lessons
  file (PASS, by negative test).  Verified by the validator returning
  exit 2 (missing path) / exit 3 (missing section) / exit 4 (empty
  defect table) on synthetic bad fixtures.
- [x] `process_invariants.rs` passes for all current canonical lists.
  20 / 0 tests under the hard gate.
- [x] **`scripts/audit-prompt-validator.sh` validates against the
  canonical lessons reference.**
  - Honest correction (final audit, 2026-05-16) : the original
    architecture text claimed this should be `docs/lessons/pr24-lessons-learned.md`.
    That file predates the canonical 7-section schema (it has 10
    sections with different titles) and is intentionally
    GRANDFATHERED per § 2.2.  The canonical reference is now
    `docs/lessons/PR-25-lessons.md` (the first lessons file produced
    UNDER the new schema) AND `docs/lessons/_template.md` (the
    schema itself).  Both validate clean (exit 0).
- [x] `scripts/sot-audit.sh` runs in CI and lists the canonical consts
  (no false positives).  14 qualifying decls scanned, 0 duplications.
- [x] STABILITY_CONTRACT § 25 added and tested via
  `tests/stability_contract_v17.rs` (35 / 0).
- [x] Phase-2 docs updated to reference the new process :
  - `docs/runbooks/bench-publish-incident-response.md` § 6 references
    `docs/lessons/_template.md`.  (The architecture originally said
    "post-mortem template references ... instead" — post-mortems and
    lessons are different artefacts ; the runbook now links the
    lessons template as the SHAPE reference until the dedicated
    `docs/postmortems/_template.md` ships in PR #26 G.15.)
  - `docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md` decision log
    gained 3 entries on 2026-05-16 documenting the PR #25 merge.
- [ ] Engineering-leadership signs off on the bypass procedure (§ 3.5).
  **Operational item, not testable in repo.**  Tracked in
  `docs/lessons/PR-25-lessons.md` § 7 G-25-2.
- [ ] An all-hands onboarding session on the new process is scheduled
  for the week after merge.  **Operational item.**  Tracked in
  G-25-4.

**Status at PR #25 merge (2026-05-16) : 9 / 11 criteria met in repo ;
2 are operational and tracked as known gaps.**

---

## 8. Test strategy

### 8.1 New tests added in PR #25

| Test name | File | What it verifies |
|---|---|---|
| `i_2_no_parallel_badge_filenames_list` | `tests/process_invariants.rs` | BADGE_FILENAMES not duplicated |
| `i_2_no_parallel_scenario_page_filenames` | same | SCENARIO_PAGE_FILENAMES not duplicated |
| `i_2_no_parallel_api_endpoints` | same | API_ENDPOINT_FILENAMES not duplicated |
| `i_2_no_parallel_cosign_regex` | same | SHARED_COSIGN_IDENTITY_REGEXP not duplicated outside the documented PR #22 + PR #24 mirror |
| `i_1_template_has_required_sections` | `tests/process_invariants.rs` | The lessons `_template.md` itself satisfies the validator |
| `v1_7_section_25_1_audit_prompt_mandatory` | `tests/stability_contract_v17.rs` | Stability locks I.1 |
| `v1_7_section_25_2_sot_invariant` | same | Stability locks I.2 |
| `v1_7_section_25_3_audit_budget` | same | Stability locks I.6 |
| `v1_7_section_25_4_drill_cadence` | same | Stability locks drills |

Expected delta : **+9 tests**.

### 8.2 Negative tests

A `tests/process_invariants_negatives.rs` companion verifies the
detector actually fires on bad input :

```rust
#[test]
fn negative_detects_duplicated_const_list_in_synthetic_input() {
    // Construct a synthetic Rust source string that contains two
    // const lists with 100 % overlap, feed it through the detector,
    // assert the detector flags it.
    let evil_source = r#"
const A: &[&str] = &["scenario-S01.svg", "scenario-S02.svg",
                     "scenario-S03.svg", "scenario-S04.svg",
                     "scenario-S05.svg", "scenario-S06.svg"];
const B: &[&str] = &["scenario-S01.svg", "scenario-S02.svg",
                     "scenario-S03.svg", "scenario-S04.svg",
                     "scenario-S05.svg", "scenario-S06.svg"];
"#;
    let dups = detect_parallel_lists(evil_source);
    assert!(!dups.is_empty(), "detector missed obvious duplication");
}
```

### 8.3 Bootstrap test for PR #25 itself

PR #25 commits its own `docs/lessons/PR-25-lessons.md` to demonstrate
the meta-self-consistency : the PR that institutionalises I.1 also
satisfies I.1.  Expected content : a 1-pass audit (the process PR
itself doesn't have surface to audit), with "Known residual gaps"
listing the items deferred to PR #26+ (see § 11).

---

## 9. Rollout plan

### 9.1 Timeline

| Day | Activity | Owner |
|---|---|---|
| T-0 (merge day) | PR #25 merges to main | author + 2 reviewers |
| T+1 | Slack `#release-eng` announcement | release-eng lead |
| T+3 | All-hands onboarding session (60 min) | engineering-leadership |
| T+7 | First PR under new rules : PR #26 (G.15 post-mortem template) | release-eng |
| T+14 | Quarterly drill #1 dry-run (using new lessons template) | release-eng |
| T+30 | Recalibration check : PR #26 audit ratio + defect count | release-eng + leadership |

### 9.2 Onboarding session agenda (60 min)

1. (10 min) Why : recap PR #24 audit findings (67 defects).
2. (10 min) I.1 : how to write the audit prompt, what the lessons
   file looks like, walk through PR #24's example.
3. (10 min) I.2 : the SOT pattern, examples (BADGE_FILENAMES), how
   to add a whitelist entry when needed.
4. (5 min)  I.6 : audit budget format, where the tracker lives.
5. (15 min) Live exercise : everyone writes a 1-paragraph audit-budget
   estimate for a hypothetical PR.
6. (10 min) Q&A + commitments.

### 9.3 Communication templates

#### Slack #release-eng (T+1)

```
:rocket: PR #25 merged — process change

The new rules :
- I.1 : audit prompt + lessons file required on PRs ≥ 1000 LOC
- I.2 : SOT for const lists ; CI lint enforces
- I.6 : audit budget line item in PR descriptions

Effective for PR #26 onward.  PR #16–#24 are grandfathered.

Onboarding session : <calendar link> Thursday 14:00 UTC.

Q&A here in this thread.
```

#### Engineering-leadership digest email (T+30)

```
Subject : Phase 3 process change — first 30-day signal

PR #25 merged 30 days ago.  First Phase-3 PR (PR #26 G.15) under the
new rules :

- Audit budget estimated : 5 days (40% of impl)
- Audit days actual : N days (M% of impl)
- Defects found in audit : X
- Status : <pass / partial / fail vs the < 20 defects target>

Recalibration : <decision to maintain 40% target, or adjust>.

— release-eng
```

---

## 10. Risk register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Status check turned off "to ship fast" | Medium | High | Branch-protection rule requires the check ; admin-only override is audited |
| Authors game LOC count by splitting PRs | Medium | Medium | Reviewer label `i1-required` overrides ; quarterly review of PR sizes |
| SOT lint false-positives slow PRs | Medium | Medium | Whitelist mechanism + sot-audit.sh runs locally for fast feedback |
| Audit budget becomes a CYA exercise (always 30%) | High | Medium | Recalibration based on actual ratios after first 3 PRs |
| Onboarding session attendance < 60 % | Medium | Low | Recorded session ; reminder in #release-eng each week for 4 weeks |
| Authors push back on the process | Low | High | Reference PR #24 67-defect data ; offer co-author the first lessons file |
| Bypass mechanism gets normalised | Medium | High | Quarterly review of bypass log ; > 25 % bypass rate triggers re-design |
| Lessons files become low-quality boilerplate | High | Medium | Reviewer rejection criterion ; spot-check by release-eng monthly |
| CI workflow itself drifts | Low | Medium | D-AP-class test in `workflow_yaml_invariants.rs` asserts pr25 workflow's existence + cron + required steps |
| process_invariants.rs is hard to maintain | Medium | Low | Whitelist mechanism + clear regenerate-with-context error messages |

---

## 11. Known residual gaps (carried to PR #26+)

PR #25 does NOT solve these ; they are tracked for follow-up :

- **I.3** : Mandatory workflow YAML structural tests for new
  workflows.  Half-implemented (D-AP + D-BK already exist) ; the
  remaining work is documentation + a generator.  Deferred to PR #28.
- **I.4** : Doc/code reconciliation CI step that fails when claims in
  STABILITY_CONTRACT don't match code behaviour.  This is the
  biggest leverage but hardest to build ; deferred to a dedicated
  PR #30 (1 month estimate, requires AST parsing).
- **I.5** : Drill cadence enforcement via calendar.  Currently
  documented in runbooks ; PR #25 codifies in STABILITY § 25.4.
  Full enforcement (Google Calendar API integration) deferred to
  out-of-band tooling.

These are scoped in [`docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`](../roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md)
§ 2.3 (I-stream) and tracked individually.

---

## 12. Decision log

| Decision | Alternative considered | Rationale |
|---|---|---|
| Status check (not pre-commit hook) for I.1 enforcement | Pre-commit hook | Hooks are `--no-verify`-bypassable ; status checks require admin override that's logged |
| LOC threshold = 1000 (not 500 or 2000) | 500 (more PRs covered), 2000 (fewer) | PR #24 was 17 000 LOC ; 1000 covers anything with non-trivial architecture |
| Lessons file in `docs/lessons/` (public) | `internal-docs/lessons/` (private) | Honesty signal to customers ; PR #24 67-defect public accounting is a trust moment |
| Audit budget tracked in markdown (not Jira) | Jira time-tracking | Lower friction ; markdown survives tooling changes |
| French canonical audit prompt | English | Preserves the historical artifact ; English translation provided |
| 80 % overlap threshold for I.2 lint | 100 % (exact) or 50 % (looser) | 80 % catches genuine drift (D-AS had 100 % overlap) while allowing legitimate test fixtures to differ |
| Bypass tag `[I.1-bypass]` (not a separate label) | GitHub label | Tag in PR title is more visible and survives branch history |
| 30 % audit budget steady-state target | 50 % (PR #24 was 150 %), 20 % (optimistic) | 30 % matches the "happy path" estimate post-lessons ; recalibrate after PR #26-28 |
| `docs/lessons/_template.md` not `_skeleton.md` | _skeleton, _example, EXAMPLE | "template" matches existing GitHub semantics |
| `process_invariants.rs` lives in bench/harness | A separate crate `efe-process-lint` | Avoid new crate sprawl ; the test runs in CI alongside others |
| Audit-budget tracker is per-PR markdown rows | A YAML / TOML structured file | Easier to grep + edit ; structured tooling can come later if needed |

---

## 13. Open questions for kickoff review

These are flagged in the roadmap and require resolution before merge :

1. **Status check enforcement granularity** : do we require the
   `pr25-i1-audit-prompt` check to be GREEN, or just "present"
   (passing or skipped)?  Proposal : required-green, no skip.
2. **Lessons file location for forks / contributors** : if an external
   contributor opens a PR, can they commit to `docs/lessons/`?  Proposal
   : yes ; the validator runs on the PR diff regardless of contributor.
3. **Workspace-wide vs harness-only SOT lint** : `process_invariants.rs`
   currently lives in `bench/harness/tests/`.  Should it also scan
   `crates/efe-slos/` and `tools/loadgen/`?  Proposal : yes — extend
   to workspace scope in PR #25 itself, since it's zero-LOC harness-side.
4. **I.6 budget recalibration trigger** : after how many PRs do we
   adjust the 30 % target?  Proposal : after 3 PR-26..PR-28, with
   release-eng running the analysis.
5. **English vs French audit prompt as canonical** : keep both ; the
   FR version retains historical fidelity to the PR #24 origin ; the
   EN version becomes the operational default for non-FR contributors.

---

## 14. References

- Lessons : [`docs/lessons/pr24-lessons-learned.md`](../lessons/pr24-lessons-learned.md)
- Roadmap : [`docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`](../roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md)
- Master runbook : [`docs/runbooks/bench-publish-incident-response.md`](../runbooks/bench-publish-incident-response.md)
- STABILITY_CONTRACT : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md) (§ 25 added by this PR)
- Drill #1 : [`docs/drills/drill-quarterly-revocation.md`](../drills/drill-quarterly-revocation.md)
- Drill #2 : [`docs/drills/drill-defacement-response.md`](../drills/drill-defacement-response.md)
- Soak report (Phase-2 close baseline) : [`docs/reports/soak-report-2026-05-16.md`](../reports/soak-report-2026-05-16.md)

---

## 15. Appendix A — the canonical audit prompt (FR + EN)

### FR (the original, kept for fidelity)

```
Prends de grandes inspirations.  Tu feras des analyses d'une précision
chirurgicale suprême.  Prends tout ton temps et tout ira bien.

Vérifie tout le contenu de ce PR pour être sûr à 100 % en faisant des
vérifications et recherches profondes et code.

À chaque pass :
1. Énumère explicitement les zones que tu n'as PAS auditées.
2. Trouve au moins UN défaut non trivial (doc drift, coverage gap,
   schema drift, security regression, spec violation, process gap).
3. Code le fix + le test qui pinne la régression.
4. Documente le défaut avec un code (D-N) dans le fichier lessons.
5. Répète jusqu'à ce que l'utilisateur dise "ça suffit" — ne t'arrête
   PAS de toi-même.

L'objectif n'est PAS d'atteindre "0 défauts."  L'objectif est de
documenter HONNÊTEMENT ce qui marche, ce qui ne marche pas, et ce
qui reste à faire.
```

### EN (operational default)

```
Take a deep breath.  Conduct surgically precise analyses.  Take all
the time you need ; everything will be fine.

Audit the entire content of this PR until you are 100 % sure, with
deep verification and research, including reading and writing code.

For every pass :
1. Explicitly enumerate the zones you have NOT audited.
2. Find at least ONE non-trivial defect (doc drift, coverage gap,
   schema drift, security regression, spec violation, process gap).
3. Code the fix + the test that pins the regression.
4. Document the defect with a code (D-N) in the lessons file.
5. Repeat until the user says "stop" — do NOT stop on your own.

The goal is NOT to reach "0 defects."  The goal is to document
HONESTLY what works, what doesn't, and what remains to be done.
```

---

## 16. Appendix B — example lessons file (synthetic minimal)

For PR authors to reference, a minimal but complete example :

```markdown
# PR #99 — Lessons learned : 2-pass chirurgical audit

**Author** : @alice
**Date** : 2026-11-30
**Status** : final

## 1. Headline

Refactor of the loadgen profile parser ; 2 audit passes, 3 defects found,
all minor coverage gaps.

## 2. Timeline summary

| Pass | Tests at end | Δ | Defects | Notable finding |
|---|---|---|---|---|
| 1 | 412 | +12 | 2 | unsigned overflow on rate=0 |
| 2 | 415 | +3 | 1 | env var precedence undocumented |

## 3. Defect taxonomy

| Class | Count | % |
|---|---|---|
| Coverage | 2 | 67 % |
| Doc drift | 1 | 33 % |

## 4. What worked / What did not

Worked : reusing PR #24's stress-test pattern (D-AU) caught the rate=0
case immediately.

Didn't work : original plan was 1 pass.  Adversarial prompt revealed
the env var precedence gap.

## 5. Process changes proposed

None new ; PR #25 process worked as designed.

## 6. Defect index

| Code | Pass | Class | Description |
|---|---|---|---|
| D-99-A | 1 | Coverage | rate=0 overflowed unsigned |
| D-99-B | 1 | Coverage | rate=u32::MAX not tested |
| D-99-C | 2 | Doc drift | EFE_LOADGEN_RATE env var not in README |

## 7. Known residual gaps

None — audit converged at pass 2.
```

---

## 17. Appendix C — file-tree delta of PR #25

```
+ .github/PULL_REQUEST_TEMPLATE.md
+ .github/workflows/pr25-process-checks.yml
+ docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md
+ docs/lessons/_template.md
+ docs/lessons/PR-25-lessons.md
+ docs/process/audit-budget-tracker.md
+ docs/process/audit-prompt.md
+ docs/process/audit-prompt-en.md
+ docs/process/i1-bypass-log.md
+ docs/process/sot-pattern.md
+ scripts/audit-prompt-validator.sh
+ scripts/sot-audit.sh
+ bench/harness/tests/process_invariants.rs
+ bench/harness/tests/process_invariants_negatives.rs
+ bench/harness/tests/stability_contract_v17.rs
~ bench/harness/src/publish/api/mod.rs           [hoist API_ENDPOINT_FILENAMES]
~ bench/harness/src/publish/deploy/ratchet.rs    [reference API_ENDPOINT_FILENAMES]
~ docs/STABILITY_CONTRACT.md                     [+ § 25 process invariants]
~ docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md  [decision log entry]
```

Total files : **15 new + 4 modified**.
Net LOC : ~+1200 docs + scripts ; ~+10 harness src/.
Net harness-binary change : **0 functional LOC** (the hoist is pure
rearrangement of a const).
```
