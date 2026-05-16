# Audit budget tracker

**Status** : live cumulative log
**Process invariant** : I.6 (PR #25 § 5)
**Frozen at** : STABILITY_CONTRACT v1.7 § 25.3
**Owner** : `@efe-ai/release-eng` (writes) ; engineering-leadership (reads aggregates)
**Cadence** : updated per PR at merge ; recalibration review after every 3 Phase-3 PRs

This file is the cumulative source-of-truth for audit budget data
across PRs.  It lives in the repo (markdown, not Jira) so it survives
tooling changes and is grep-able by any team member.

> **Hard rule** : the tracker captures ESTIMATES (at PR-open time) and
> ACTUALS (at merge time).  Don't backfill estimates retroactively —
> that defeats the calibration loop.

---

## 1. Format

Each row is a PR.  Columns :

| Column | Meaning | Filled by |
|---|---|---|
| `PR` | PR number (link if possible) | author at open |
| `Title` | Short PR title | author at open |
| `Impl est (d)` | Estimated implementation days | author at open |
| `Audit est (d)` | Estimated audit days (per I.1) | author at open |
| `Audit % est` | `audit_est / impl_est * 100`, rounded | author at open |
| `Actual impl (d)` | Real implementation days at merge | author at merge |
| `Actual audit (d)` | Real audit days at merge | author at merge |
| `Actual %` | `actual_audit / actual_impl * 100`, rounded | author at merge |
| `Defects found` | Count of D-codes in lessons file | author at merge |
| `Lessons file` | Link to `docs/lessons/PR-N-lessons.md` | author at merge |
| `Verdict` | pass / partial / fail (per § 4) | release-eng at merge |
| `Notes` | One-line context if needed | any |

### 1.1 Calibration anchor

**PR #24 (the reference)** :
- Implementation : 20 days
- Audit (estimated at open) : 8 days (40 %)
- Audit (actual) : 30 days (150 %)
- Defects found : 67
- Verdict : **outlier** — first PR with explicit audit process, both
  implementing the change AND learning the audit pattern simultaneously

**Phase-3 steady-state target** : 30 % audit overhead.

**Calibration triggers** :
- After every 3 Phase-3 PRs : release-eng computes the median % of
  the last 3 actuals.  If it diverges by > 15 percentage points from
  the 30 % target, propose a new target.
- Recalibration is a PR to this file's "Calibration anchor" subsection
  + the [`docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`](../roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md) § 5.2.

---

## 2. Cumulative table

| PR | Title | Impl est (d) | Audit est (d) | Audit % est | Actual impl | Actual audit | Actual % | Defects | Lessons file | Verdict | Notes |
|---|---|---|---|---|---|---|---|---|---|---|---|
| [#16](https://github.com/efe-ai/efe/pull/16) | Workspace foundations | — | — | — | — | — | — | — | (pre-process) | grandfathered | Pre-I.1 |
| [#17](https://github.com/efe-ai/efe/pull/17) | Loadgen v0 | — | — | — | — | — | — | — | (pre-process) | grandfathered | Pre-I.1 |
| [#18](https://github.com/efe-ai/efe/pull/18) | Cluster-provisioner v0 (inline) | — | — | — | — | — | — | — | (pre-process) | grandfathered | Superseded by #23 |
| [#19](https://github.com/efe-ai/efe/pull/19) | Scenario YAML schema | — | — | — | — | — | — | — | (pre-process) | grandfathered | Pre-I.1 |
| [#20](https://github.com/efe-ai/efe/pull/20) | Loadgen profile YAML | — | — | — | — | — | — | — | (pre-process) | grandfathered | Pre-I.1 |
| [#21](https://github.com/efe-ai/efe/pull/21) | Public-bench harness | — | — | — | — | — | — | — | (pre-process) | grandfathered | Pre-I.1 |
| [#22](https://github.com/efe-ai/efe/pull/22) | SLO portal | — | — | — | — | — | — | — | (pre-process) | grandfathered | Pre-I.1 |
| [#23](https://github.com/efe-ai/efe/pull/23) | Cluster-provisioner extraction | — | — | — | — | — | — | — | (pre-process) | grandfathered | Byte-compat shim |
| [#24](https://github.com/efe-ai/efe/pull/24) | Publish bench results | 20 | 8 | 40 | 20 | 30 | **150** | **67** | [pr24-lessons-learned.md](../lessons/pr24-lessons-learned.md) | **outlier** | First PR with explicit audit ; calibration anchor |
| [#25](https://github.com/efe-ai/efe/pull/25) | Process changes I.1+I.2+I.6 | 5 | 2 | 40 | 5 | 2 | 40 | 8 | [PR-25-lessons.md](../lessons/PR-25-lessons.md) | **pass** | First PR under new rules ; 8 defects vs PR #24's 67 (10× reduction) ; ratio met estimate exactly |
| #26 | _placeholder — first Phase-3 PR under new rules_ | _est_ | _est_ | _est_ | _filled_ | _filled_ | _filled_ | _count_ | _link_ | _filled_ | |
| #27 | _placeholder_ | _est_ | _est_ | _est_ | _filled_ | _filled_ | _filled_ | _count_ | _link_ | _filled_ | |
| #28 | _placeholder_ | _est_ | _est_ | _est_ | _filled_ | _filled_ | _filled_ | _count_ | _link_ | _filled_ | |

> **Placeholders** : rows for #26-#28 are pre-allocated so the author
> doesn't forget to add a row.  Delete the placeholders or replace
> with real data at open / merge time.

---

## 3. How to update — per-PR procedure

### 3.1 At PR open (author)

1. Add a new row to § 2 with your PR number + title.
2. Fill : `Impl est (d)`, `Audit est (d)`, `Audit % est`.
3. The 4 actual columns + defects + lessons file + verdict stay
   empty until merge.
4. Commit the update **in the SAME PR** as a tiny diff against this
   file — that way the row exists in the PR itself.

### 3.2 During the audit (author)

No tracker update needed.  Update the lessons file (`docs/lessons/PR-N-lessons.md`)
incrementally per the audit prompt.

### 3.3 At merge time (author, before pressing merge)

1. Compute `Actual impl (d)` : days between first commit and merge,
   subtracting weekends / OOO.
2. Compute `Actual audit (d)` : sum of time blocks dedicated to audit
   passes (estimate from your timeline — be honest, not aspirational).
3. Compute `Actual %` : `Actual audit / Actual impl * 100`.
4. Count `Defects found` : grep the lessons file for `^| D-` rows in
   § 6 ; that's the count.
5. Link the lessons file.
6. Push the update as the last commit before the merge button.

### 3.4 At merge time (release-eng)

1. Set `Verdict` per § 4 below.
2. If verdict is `fail`, open a follow-up issue to re-audit within 30
   days.

---

## 4. Verdict criteria

The verdict is set by release-eng (not the author) based on :

- **pass** : `Actual %` ≤ 50 % AND `Defects found` < 20.  The audit
  was efficient and converged at typical cost.
- **partial** : `Actual %` 50–100 % OR `Defects found` 20–66.  The
  audit was costlier than steady-state but caught real defects.
- **fail** : `Actual %` > 100 % OR `Defects found` ≥ 67.  The audit
  approached / matched PR #24's outlier signature ; trigger an
  I-stream re-examination per roadmap § 7.
- **outlier** : reserved for PR #24 only (the calibration anchor).
- **grandfathered** : PRs ≥ #24 are pre-I.1 ; rows are empty.

---

## 5. Recalibration log

After each batch of 3 Phase-3 PRs, release-eng files a recalibration
entry here.

### 5.1 PR #26-#28 batch (TBD)

- Median Actual % : _to be computed_
- Decision : maintain / adjust target
- Rationale : _to be written_
- New target (if changed) : _to be set_
- Roadmap PR updating § 5.2 : _link_

### 5.2 PR #29-#31 batch (TBD)

_pending_

### 5.3 PR #32-#34 batch (TBD)

_pending_

---

## 6. Aggregate statistics (auto-updatable section)

A simple shell command can regenerate this section :

```bash
# Run from repo root.
echo "Updated : $(date -u -Iseconds)"
echo "Total PRs (I.1 era) : $(awk -F'|' '/^\| #2[5-9]|^\| #3[0-9]/ {n++} END {print n}' docs/process/audit-budget-tracker.md)"
# ... etc.
```

> Phase-3 G.15 will replace this manual snippet with a CI job that
> updates these aggregates automatically.  Until then : update by
> hand after each batch.

### 6.1 Live numbers (last updated : 2026-05-16, post-PR #25 merge)

| Metric | Value |
|---|---|
| PRs in tracker | 10 (9 grandfathered + 1 anchor + 1 first-of-Phase-3) |
| PRs under I.1 (i.e. with real audit data) | 2 (PR #24 anchor, PR #25 first Phase-3) |
| Mean Actual % (I.1 era, n=2) | 95 % (still pre-steady-state ; PR #24 outlier dominates) |
| Median Actual % (I.1 era, n=2) | 95 % (same) |
| Mean Defects found (I.1 era, n=2) | 37.5 (driven by PR #24 = 67 ; PR #25 = 8) |
| Defect-count reduction PR #24 → PR #25 | 10× (67 → 8) |
| Audit-overhead reduction PR #24 → PR #25 | 3.75× (150 % → 40 %) |
| Outlier ratio (PR #24 vs target) | 150 / 30 = 5× |
| Outlier ratio (PR #25 vs target) | 40 / 30 = 1.33× |

**Status** : PR #25 came in at 40 % audit ratio against a 30 % target.
That's a 1.33× outlier, well within the "pass" verdict range and a
strong signal the steady-state target is achievable but may need
adjustment to 35-40 % after 3 more Phase-3 PRs are measured.

Recalibration trigger : after PR #26 + #27 + #28 (3 Phase-3 PRs).
Current recommendation : maintain 30 % target ; revisit at recalibration.

---

## 7. What the tracker does NOT do

- It does NOT replace per-author performance review.  Audit ratios
  are AGGREGATE signals, not individual metrics.
- It does NOT include reviewer time.  A reviewer's hours are tracked
  separately (out-of-band).
- It does NOT include time spent on the "buffer for unforeseen depth"
  line item from the PR template.  That's deliberately separated to
  avoid double-counting.
- It does NOT trigger automated estimation.  Humans estimate ; the
  tracker only ensures the estimate is recorded.

---

## 8. Anti-patterns

### 8.1 "I'll backfill the estimate after I see what it actually took"

This defeats the entire calibration loop.  The estimate MUST be at
open time, BEFORE you know how it'll go.  If you backfill, the median
converges to the actual and the target becomes uninformative.

### 8.2 "Round the actuals down to look better"

Don't.  Release-eng spot-checks at quarterly review by looking at the
PR's first-commit-to-merge timeline.  Caught discrepancies trigger
trust loss + a follow-up audit.

### 8.3 "Skip the row, I'll do it later"

The PR template's `## Audit budget` section is enforced by the
`pr25-i6-audit-budget` status check.  But the TRACKER update is on
the honour system.  Skipping the tracker row silently weakens
Phase-3 recalibration.

### 8.4 "I.1-bypassed PRs don't need a tracker row"

Yes they do.  Add the row, mark `Verdict : bypass`, link the
`i1-bypass-log.md` entry.  Quarterly review uses the tracker count of
bypass rows to enforce the < 25 % bypass-rate ceiling.

---

## 9. References

- PR #25 architecture § 5 (I.6) : [`docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md)
- PR #25 PR template (audit-budget section) : [`.github/pull_request_template.md`](../../.github/pull_request_template.md)
- Status check : `.github/workflows/pr25-process-checks.yml` job `i6-audit-budget`
- Calibration target source : [`docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`](../roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md) § 5.2
- Bypass log (cross-referenced for bypass-rate computation) : [`docs/process/i1-bypass-log.md`](i1-bypass-log.md)
- STABILITY_CONTRACT v1.7 § 25.3 : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md)
