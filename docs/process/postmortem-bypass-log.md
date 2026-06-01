# Post-mortem bypass log

**Status** : live audit log (append-only)
**Process invariant** : G.15 bypass procedure (PR #26 § 5.4)
**Frozen at** : STABILITY_CONTRACT v1.7 § 26.6
**Owner** : `@efe-ai/release-eng` (writes entries) ; engineering-leadership (reviews quarterly)
**Cadence** : append per bypass ; review every Q

This file records every incident issue that bypassed the post-mortem
requirement under the canonical schema (G.15). The bypass is a
deliberate escape hatch for three narrow classes ; this log makes
the escape hatch AUDITED so it doesn't become routine.

> **Read me first** : the bypass procedure is detailed in
> [`docs/architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md)
> § 5.4. Do NOT bypass without reading that section. The bypass is
> NOT a way to "skip a post-mortem on a tight schedule" — it is for
> mislabeled non-incidents, merged incidents, and drill exercises
> ONLY.

---

## 1. Bypass classes

Per the architecture, three and only three bypass classes are accepted :

### 1.1 Class 1 : `MISLABEL`

**When acceptable** : an issue was labeled `incident` and subsequently
closed, but on inspection the issue was not an operational incident
in the runbook sense. Examples : an issue mistakenly labeled by a
webhook integration, an issue that turned out to be a feature
request mis-triaged.

**Procedure** :
1. The engineering lead closes the tracking issue with a comment
   beginning `[post-mortem-bypass]` followed by a one-paragraph
   justification.
2. The engineering lead removes the `incident` label from the
   original issue (to prevent future re-flagging).
3. A row is appended to the table in § 2 of this file at the next
   business day.

### 1.2 Class 2 : `MERGED`

**When acceptable** : an issue was opened as an incident in good
faith but turned out to be a duplicate or subset of another
incident that already has a post-mortem in progress. The bypass is
the right outcome because two post-mortems for one underlying
event would dilute the lessons.

**Procedure** :
1. The engineering lead closes the tracking issue with a comment
   beginning `[post-mortem-bypass]` and a link to the surviving
   incident's post-mortem file.
2. The original issue is cross-referenced from the surviving
   incident's post-mortem in its § 11 Cross-references section.
3. A row is appended to the table in § 2 of this file at the next
   business day.

### 1.3 Class 3 : `DRILL`

**When acceptable** : the issue was opened as part of an intentional
drill (the quarterly defacement drill, the annual revocation drill,
the G.15 rehearsal drill, or any future drill).

**Procedure** :
- Drill issues are marked with the `drill` label IN ADDITION TO
  `incident`, set at issue-open time by the drill commander.
- The workflow at `.github/workflows/pr26-postmortem-checks.yml`
  detects the `drill` label automatically and exempts the issue
  from the deadline check.
- **No manual entry in this log is required for Class 3.** The
  workflow records the exemption in its own structured log
  (`docs/process/postmortem-status.md`).

---

## 2. Bypass log table (Class 1 + Class 2 only)

Class 3 (drill) bypasses are recorded automatically by the workflow
and do NOT appear in the table below.

The expected rate of Class 1 + Class 2 bypasses combined is no more
than two per quarter. A rate above this threshold triggers an
explicit conversation at the next master-runbook review.

| Date | Incident ID | Class | Engineer | One-paragraph justification | Closing comment link |
|------|-------------|-------|----------|-----------------------------|---------------------|
| _(no entries yet — log opened with PR #26)_ | | | | | |

---

## 3. Quarterly review checklist

The quarterly master-runbook review (§ 25.4 of the STABILITY
contract) explicitly examines this log. The review verifies :

- [ ] Each entry's class assignment was correct (Class 1 / Class 2).
- [ ] Each entry's justification paragraph is concrete and specific,
      not generic.
- [ ] No entry should retroactively have been a real post-mortem.
- [ ] The combined Class 1 + Class 2 rate for the quarter is ≤ 2.
      If > 2, the review opens a discussion of whether the deadline
      algorithm needs adjustment, the team's labeling discipline
      needs improvement, or the bypass mechanism itself is being
      abused.

The review's outcome is documented in the corresponding quarterly
post-mortem summary file under `docs/process/`.

---

_Frozen under STABILITY § 26.6 ; changes to the bypass-class set or
to the log's structural schema require a STABILITY-bump PR. Entries
are append-only and never removed (an entry whose subject is later
recharacterised is amended via an additional row, not by editing
the original)._
