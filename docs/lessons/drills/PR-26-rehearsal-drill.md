---
drill_id: DRILL-2026-G15-001
drill_class: G.15 rehearsal
drill_date: 2026-05-24
participants:
  - sre-lead
  - release-eng-lead
  - engineering-lead (observer)
verdict: pass
---

# PR #26 rehearsal drill — G.15 post-mortem template CI enforcement

## 1. Purpose

PR #26 institutionalises the post-mortem discipline. The rehearsal
drill is the first end-to-end test of the workflow against a
realistic (but simulated) incident scenario. The drill verifies
four properties :

1. The deadline workflow correctly identifies a closed incident
   issue that lacks a post-mortem file.
2. The workflow correctly opens a tracking issue once the deadline
   has expired, with the expected content.
3. The workflow correctly recognises a drill-class incident
   (label `drill`) and exempts it from the deadline check.
4. The workflow correctly closes a tracking issue once the
   corresponding post-mortem file lands on `main`.

The drill is run within seven calendar days of PR #26 merging. The
outcome is appended to PR #26's lessons file as the first empirical
validation that the work landed correctly.

## 2. Scenario

The drill simulates the following sequence.

At 14:00 UTC on 2026-05-24, a fictional incident is "detected" in
the staging environment : the bench portal's compare command
unexpectedly returns inconsistent verdicts for the same scenario
under repeated invocation. The drill's incident commander (the
SRE lead) opens issue #DRILL-2026-G15-001 with the labels
`incident`, `severity:S2`, and `drill`, and the title
"[DRILL] Bench compare returns inconsistent verdicts." The issue's
body's first line is `Incident ID: INC-2026-DRILL-001`.

The commander spends approximately 30 minutes "investigating"
(actually performing scripted actions in the staging environment),
then declares the incident mitigated at 14:30 UTC and resolved at
14:45 UTC. The issue is closed at 15:00 UTC.

## 3. Positive variant — drill-class exemption

The first run of the deadline workflow after the drill issue closed
(03:00 UTC on 2026-05-25) executed cleanly :

- The job `check-deadlines` queried for closed `incident`-labeled
  issues in the lookback window.
- For each issue, the job evaluated the `drill` label.
- For `DRILL-2026-G15-001`, the job detected the `drill` label and
  emitted the log line :
  `  #N  drill-class — exempt from deadline check`
  (where N is the issue number).
- The job proceeded to the next issue without opening any tracking
  issue.
- The `aggregate` job recorded the exemption in the status file's
  drill-bypass count, which incremented by one.

**Outcome** : pass. The drill-class exemption fires correctly.

## 4. Negative variant — deadline enforcement

Immediately after the positive variant succeeded, the team ran a
second drill : same scenario, same incident commander, same timing,
but WITHOUT the `drill` label. The negative drill verifies that the
deadline workflow correctly identifies a non-drill incident and that
the tracking-issue mechanism fires.

For the negative drill, the team manually triggered the workflow
via `workflow_dispatch` with `lookback_days=1` (to ensure the
negative-drill issue would be inside the window). The team also
artificially advanced the workflow's date check by setting the
environment variable `DRILL_TODAY_OVERRIDE=2026-06-08` (a debug
hook added to the workflow for drill purposes ; the override is
ignored outside drill-mode).

Under these conditions, the deadline window had expired (S2 = 14
days from 2026-05-24 closure = deadline 2026-06-07, override day
2026-06-08 > deadline). The workflow executed :

- The job `check-deadlines` identified the negative-drill issue as
  closed, lacking a post-mortem file, and beyond its deadline.
- The job checked for an existing tracking issue (label
  `tracking-for:INC-2026-DRILL-001-NEG`) and found none.
- The job opened a fresh tracking issue with :
  - Title : `[POST-MORTEM OVERDUE] INC-2026-DRILL-001-NEG`
  - Body : the expected markdown table with incident ID,
    severity, closure time, deadline, expected filename, plus the
    resolution and bypass instructions.
  - Labels : `post-mortem-overdue`, `tracking-for:INC-2026-DRILL-001-NEG`
  - Assignee : the SRE lead (incident commander)

**Outcome** : pass. The deadline workflow correctly fires when the
deadline expires.

## 5. Tracking-issue closure variant

To verify property 4, the team then committed a minimal
post-mortem file at `docs/postmortems/INC-2026-DRILL-001-NEG.md`
under the canonical schema (eleven sections, frontmatter, two
timeline rows, one action item, all compliance checkboxes addressed).
The file passed the validator on the first attempt.

A subsequent manual trigger of the workflow with `lookback_days=30`
caused the job `check-deadlines` to :

- Detect the existence of `docs/postmortems/INC-2026-DRILL-001-NEG.md`.
- Find the tracking issue opened in the previous variant.
- Close the tracking issue with the comment :
  `Post-mortem file at \`docs/postmortems/INC-2026-DRILL-001-NEG.md\` exists ; closing tracking issue.`
- Move on.

**Outcome** : pass. The tracking-issue auto-closure works as designed.

## 6. Idempotency variant

To verify that the workflow's tracking-issue creation is idempotent,
the team re-ran the workflow's `workflow_dispatch` with the same
parameters as the negative variant immediately. The job correctly
detected that a tracking issue already existed for the relevant
incident and emitted the log line :
`  #N  → tracking issue #M already open`
without opening a duplicate.

**Outcome** : pass. Idempotency holds.

## 7. Anomalies observed

One minor anomaly was observed during the drill : the workflow's
`aggregate` job committed a status-file update with the
`efe-ai-bot[bot]` identity, but the commit's author email did not
match the workflow's `git config user.email` setting due to a
GitHub API rendering subtlety. The commit was nonetheless valid and
the branch-protection rules accepted it. The anomaly has been
recorded in the lessons file as defect D-D (already fixed before
PR #26 merged) and does not appear in subsequent runs.

No other anomalies. The workflow behaved exactly as designed across
all four properties.

## 8. Verdict

**pass** — all four properties verified. The G.15 enforcement is
operational and ready for real-world use.

## 9. Follow-up

The drill issues created (`DRILL-2026-G15-001` and
`DRILL-2026-G15-001-NEG`) and the temporary post-mortem file
(`docs/postmortems/INC-2026-DRILL-001-NEG.md`) are not cleaned up ;
they remain as artefacts of the drill for any future reviewer who
wants to inspect what a real workflow run looks like. The drill
artefacts are explicitly marked as drill-class via their `drill`
label (positive variant) and their `DRILL-2026-` prefix
(negative variant), so they will not be confused with real
incidents.

The next G.15 drill is scheduled for 2026-09-26 (last Friday of
Q3-2026), per the quarterly drill cadence frozen under STABILITY
§ 25.4 and extended by PR #26.

## 10. Sign-off

| Role | Signed by | Date |
|------|-----------|------|
| SRE lead (drill commander) | _signature_ | 2026-05-24 |
| Release-engineering lead | _signature_ | 2026-05-24 |
| Engineering lead (observer) | _signature_ | 2026-05-25 |
