<!--
  PR #26 lessons file.

  Frozen schema : the canonical seven-section lessons schema from
  PR #25 / STABILITY § 25.1.  Validated by
  scripts/audit-prompt-validator.sh.

  This file will be updated during the audit phase of PR #26.  The
  current version is the "Day 0" draft written at PR-open time.
-->

# PR #26 lessons — G.15 post-mortem template CI enforcement

## 1. Headline

PR #26 closes G.15 by adding the automated enforcement of the
existing post-mortem template. Six artefacts ship together : the
template's freezing under STABILITY § 26.1, a YAML frontmatter
contract, a 350-line bash validator with a stable seven-code exit
contract, a 460-line CI workflow with three jobs, ~140 lines of
Rust integration tests covering positive and negative paths, and
the six new freeze tests added to the v1.7 STABILITY suite. The PR
adds no LOC to the harness binary and approximately 1 100 LOC
across scripts, workflows, tests, and documentation.

Empirical anchor : 8 defects across 2 audit passes, at the same
inspection rigour as PR #25. The PR comes in at the bottom of the
predicted Phase-3 steady-state defect band (8 to 30), confirming
the eight-fold reduction signal from PR #25 holds for the second
process-discipline PR in Phase 3.

## 2. Timeline summary

| Pass | Date | Tests at end | Δ | Defects | Notable |
|------|------|--------------|---|---------|---------|
| 1 (Day 1 — schema + validator) | 2026-05-17 | 466 | +26 | 4 | Validator's compliance-checklist heuristic mis-fired on the reference example until the N/A annotation rule was made anywhere-on-line. |
| 2 (Day 2 — workflow + tests + freeze + first audit) | 2026-05-18 | 472 | +6 | 4 | Workflow YAML parse fixed twice (heredoc body lines starting at column 1 violated YAML block-scalar indentation in two separate jobs). Rehearsal drill executed cleanly. |
| 3 (Day 3 — round-2 audit + contract-vs-impl drift fix) | 2026-05-18 | 482 | +10 | 1 | Round-2 audit caught D-J : STABILITY § 26.4 promised weekend + holiday shifts in the deadline algorithm, but the workflow's inline `date + N days` did pure calendar arithmetic.  Extracted to `scripts/postmortem-deadline.sh` (testable from Rust) and added 9 canonical input/output tests.  Freeze test § 26.4 now also asserts script existence + executability + doc consistency. |

Total defects across three passes : 9, distributed as detailed in § 6.

## 3. Defect taxonomy

| Class | PR #25 baseline | PR #26 | Δ |
|-------|------------------|--------|---|
| doc/code drift       | 2 | 1 | −1 |
| coverage gap         | 3 | 2 | −1 |
| schema drift         | 0 | 0 | 0 |
| security / integrity | 0 | 0 | 0 |
| spec violation       | 1 | 2 | +1 |
| process gap          | 2 | 4 | +2 |

The spec-violation count rises to 2 (vs PR #25's 1) because the
round-2 audit caught D-J — a contract-vs-implementation drift in
the deadline-shift algorithm that the round-1 audit missed.  The
defect was real (the workflow's deadline computation did not
honour STABILITY § 26.4's promises about weekend + holiday shifts)
but the round-1 freeze tests verified only the contract's
textual presence, not the workflow's behavioural conformance.
Round-2 fixed this by extracting the algorithm to a separate script
and tightening the freeze test to assert script existence +
executability + textual consistency.

The process-gap count rises to four because PR #26 is itself a
process PR and its own process surface has more gaps to close than
a typical PR. See § 6 for the specific D-codes.

## 4. What worked

Several practices from PR #25 transferred cleanly to PR #26 and are
worth recording as repeatable patterns.

The parallel-validator pattern (`postmortem-validator.sh`
structurally identical to `audit-prompt-validator.sh`) saved
approximately one engineer-day of design conversation. The author
of the new validator did not need to invent any structural choice ;
each section of the new script has a corresponding section in the
sibling, and any reviewer reading either file can immediately spot
where they parallel each other.

The pattern of writing the reference example BEFORE the validator
(rather than after) caught two heuristic errors at the moment they
were introduced. The reference example serves as the unit test for
the validator while the validator is being written, which closes
the loop between schema-design and validator-correctness.

The decision to use the `read_workspace_file` helper from the v17
freeze test file, rather than introducing a new helper, kept the
freeze-test surface internally consistent. Reviewers reading either
the § 25 or § 26 freeze tests see the same pattern of file-loading
and assertion shape.

The use of `printf '%s\n'` rather than heredocs for multi-line
content embedded inside YAML block scalars (specifically inside
the workflow's `run:` blocks) avoided two bugs : the YAML-parsing
errors that the first two attempts produced (lines starting at
column 1 inside a heredoc violating the block-scalar indentation
rule) and a class of latent escaping bugs that the heredoc form
would have produced under unusual input.

## 5. Process changes proposed

Three minor process changes emerge from PR #26's audit, none of
which warrant a new STABILITY-bump :

The first is that the parallel-validator pattern should become an
explicit reference in the audit-prompt documentation. New process
PRs should be encouraged to follow the pattern when they introduce
a new validator. This is a documentation-only change ; no
automation enforces it.

The second is that the workflow self-test (the job that invokes
the validator against the reference example as the first action in
the workflow) should be made a convention for all future process
workflows. The self-test caught zero defects in PR #26 itself but
would have caught any subsequent regression in the validator that
made it past the integration tests but failed end-to-end.

The third is that the cold-start period for the deadline workflow's
lookback window (default 0 for 60 days post-merge) is worth
documenting as a general pattern for any future workflow that
checks historical state. The pattern is cheap to implement and
prevents a class of "tracking issues opened for incidents the team
chose to handle via backfill" problems.

## 6. Defect index

| Code | Class | Location | One-line fix |
|------|-------|----------|--------------|
| D-A | process gap | `docs/process/postmortem-bypass-log.md` schema | Added the explicit quarterly-review checklist in § 3 so reviewers know what to verify. |
| D-B | coverage gap | `bench/harness/tests/postmortem_schema.rs` | Added `validator_accepts_minimal_timeline` (boundary at exactly 2 rows) — the original draft tested only ≥ 3. |
| D-C | doc/code drift | `docs/STABILITY_CONTRACT.md` § 26.4 | The deadline algorithm's weekend-shift wording in the contract did not match the workflow's actual implementation ; aligned both to "next Monday at 23:59:59 UTC". |
| D-D | process gap | `.github/workflows/pr26-postmortem-checks.yml` job `aggregate` | Added the no-op cmp check so the workflow does not produce an empty commit when the status file is unchanged. |
| D-E | spec violation | `scripts/postmortem-validator.sh` exit-code documentation | The original draft omitted the explanation that exit code 1 is reserved ; added explicitly so future maintainers don't fill that slot accidentally. |
| D-F | coverage gap | `docs/postmortems/_VALIDATOR_REFERENCE.md` compliance section | Original draft left the compliance checkboxes as `- [ ]` without N/A annotation, which failed Check 6 ; updated to `- [x] ... N/A` form. |
| D-G | process gap | `docs/process/postmortem-bypass-log.md` quarterly review | Added the explicit "rate ≤ 2 per quarter" threshold so the review has a concrete trigger. |
| D-H | process gap | `docs/runbooks/bench-publish-incident-response.md` § 6 | The previous § 6 hand-wave description of post-mortem requirements was superseded by the canonical schema reference ; rewrote § 6 to point at the validator instead. |
| D-J | spec violation | `.github/workflows/pr26-postmortem-checks.yml` deadline computation | STABILITY § 26.4 promised weekend + holiday shifts but the workflow's `date + N days` did only calendar arithmetic.  Extracted algorithm into `scripts/postmortem-deadline.sh` ; wired the workflow to invoke the script ; added 9 canonical input/output tests + tightened the § 26.4 freeze test to assert script existence + executability + doc consistency.  Caught by round-2 audit (lens : implementation-vs-contract). |

No critical or high-severity defects were identified.  All nine
defects above were caught during the three-pass audit and fixed
before merge.  D-J in particular illustrates why round-2 audits
matter : textual freeze tests caught no drift, but the round-2
lens that compares contract promises to implementation behaviour
caught the gap.

## 7. Known residual gaps

The following items are deliberately left open and scheduled for a
later PR. None are urgent ; all are tracked.

**G.15-residual-1 : the validator does not enforce content quality.**
The validator checks structural presence (the eleven sections, the
non-empty timeline and action-items tables, the addressed
compliance checklist). It does NOT check that the content under
each heading is substantive. A team that wanted to game the
discipline could write hollow post-mortems that satisfy the
validator. We have accepted this gap because the social pressure
inside the team makes hollow post-mortems socially expensive ;
PR #27 (I.7) will add content-quality heuristics calibrated against
the first quarter of real-world post-mortems.

**G.15-residual-2 : the workflow does not fail PR merges on overdue
post-mortems.** The workflow opens a tracking issue ; it does not
block the PR itself. PR #27 (I.7) will convert the tracking issue
into a hard CI gate that fails the merge of any PR in the affected
component until the post-mortem lands.

**G.15-residual-3 : action items are not auto-tracked for completion.**
The action items in a post-mortem have due dates, but no automation
reminds the team when an action item is overdue. This is part of
Phase-4 J.7 (action-item completion tracking).

**G.15-residual-4 : the pre-PR-26 incidents have not yet been
backfilled.** Nine incidents closed between 2025-Q4 and 2026-Q2
predate G.15 and have no post-mortems. The backfill plan in the
architecture doc § 8.2 schedules the work for Q4-2026 ; it is
explicitly out of scope for PR #26 itself.

**G.15-residual-5 : the deadline workflow runs once per day.** The
24-hour granularity means a deadline that expires at 04:00 UTC will
not trigger a tracking issue until the next 03:00 UTC run, almost
24 hours later. The lag has been accepted as not worth fixing ;
the team's incident-response cadence does not require sub-day
resolution on the post-mortem deadline check.

**G.15-residual-6 : the public-facing post-mortem digest is not
yet produced.** A digest of post-mortems suitable for customer
consumption would be a strong trust signal but it requires careful
design (anonymisation, redaction, customer communication). This is
Phase-4 J.6 and is gated on the team having six months of validated
post-mortems to draw from.

These six residual gaps are documented exhaustively in the PR's
internal residual-gaps tracker and reviewed at every Phase-3
quarterly review.
