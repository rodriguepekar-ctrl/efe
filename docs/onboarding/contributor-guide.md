# Contributor guide — first-week reading + ongoing discipline

**Audience** : engineers joining the team for the first time, plus
returning maintainers refreshing on the process discipline.
**Maintainer** : `@efe-ai/release-eng`
**Last revised** : 2026-05-17 (PR #26, adding G.15 post-mortem
materials).

This file is the entry point for anyone joining the team's
day-to-day engineering work. It points at the canonical artefacts
new contributors are expected to absorb in their first week, and at
the ongoing discipline they will be expected to participate in
thereafter.

The file is short by design. The actual content lives in the
artefacts it links to ; this is the index, not the book.

---

## 1. First-week reading (mandatory)

The following artefacts MUST be read by every new contributor
during their first week on the team. The list is small ; each
entry is short or skimmable. The expectation is that a new
contributor finishes the list within five working days.

### 1.1 STABILITY contract

[`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md). Read § 22
onwards. Sections § 24, § 25, and § 26 are the most-referenced
sections in day-to-day work. The earlier sections (§ 1 — § 21)
can be read on a slower cadence.

### 1.2 PR #25 architecture — the three process rules

[`docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md).
The institutionalisation of I.1 (audit prompt mandatory), I.2
(single source of truth), and I.6 (audit budget). Every PR ≥ 1 000
LOC follows these rules.

### 1.3 PR #26 architecture — the post-mortem rule

[`docs/architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md).
The G.15 institutionalisation of post-mortems. Every closed
incident issue corresponds to a post-mortem under a deadline window.

### 1.4 Post-mortem canonical template

[`docs/postmortems/_template.md`](../postmortems/_template.md). The
eleven-section schema every post-mortem follows. Read once at
onboarding ; reference it again when you write your first
post-mortem.

### 1.5 Post-mortem reference example

[`docs/postmortems/_VALIDATOR_REFERENCE.md`](../postmortems/_VALIDATOR_REFERENCE.md).
A fully-worked example post-mortem written under the canonical
schema. The single best resource for understanding what a
high-quality post-mortem looks like.

### 1.6 Audit-prompt validator script

[`scripts/audit-prompt-validator.sh`](../../scripts/audit-prompt-validator.sh).
The validator that enforces I.1. Read the documentation block at
the top of the script ; you do not need to read the implementation
unless you are modifying the validator itself.

### 1.7 Post-mortem validator script

[`scripts/postmortem-validator.sh`](../../scripts/postmortem-validator.sh).
The validator that enforces G.15. Same advice as for the audit-
prompt validator : read the doc block, skip the implementation
unless you are modifying it.

### 1.8 Incident-response runbook

[`docs/runbooks/bench-publish-incident-response.md`](../runbooks/bench-publish-incident-response.md).
What to do when you are paged for an incident. § 6 in particular
documents the post-mortem closing step.

---

## 2. The 30-minute walkthrough

Within the first month of a new contributor's first non-trivial
PR, a senior contributor schedules a 30-minute synchronous
walkthrough. The walkthrough covers :

- The three rules of PR #25 (I.1 audit prompt, I.2 SOT, I.6 audit
  budget) and how they applied to the new contributor's PR.
- The G.15 post-mortem rule and what would happen if an incident
  occurred during the contributor's on-call rotation.
- Any specific area-of-the-codebase context the contributor needs
  but did not absorb during the first-week reading.

The walkthrough is documented in the team's internal log under
`docs/process/onboarding-walkthrough-log.md` (not in this file,
because the log contains specific contributor names).

---

## 3. Ongoing discipline

After the first week and the first walkthrough, the contributor is
expected to participate in the team's ongoing discipline at the
following rhythm :

### 3.1 Per-PR

Every PR the contributor opens follows the three rules of PR #25.
The pre-merge audit-prompt validator and the audit-budget tracker
update are part of the PR's "Definition of Done" ; a PR that lacks
either does not merge.

### 3.2 Per-incident

If the contributor is paged for an incident during their on-call
rotation, they are the incident commander for that incident. The
post-mortem closing step is theirs to complete within the deadline
window appropriate to the severity. The post-mortem is reviewed
by at least one other contributor before merging.

### 3.3 Quarterly

The contributor attends the quarterly master-runbook review (last
Friday of each calendar quarter). The review covers the rolling
state of the audit-budget tracker, the post-mortem discipline
(bypass rates, deadline violations, escalations), and any
drift in process discipline that has surfaced during the quarter.

---

## 4. How to get help

If you are stuck on anything in the first week, the right
escalation path is :

1. Ask the contributor who opened your last PR review (they are by
   convention your "first-week buddy").
2. If they are unavailable, open a question in the team's
   coordination channel with the prefix `[new-contributor]`. Any
   maintainer will respond.
3. If your question concerns the process discipline specifically
   (an unclear rule, a validator error you cannot resolve, a
   conflict between two pieces of documentation), the
   engineering-lead is the designated person to talk to.

There are no stupid questions during the first week. Asking is
preferred to guessing.

---

## 5. Owner

This file is owned by `@efe-ai/release-eng` and reviewed at the
quarterly master-runbook review. Suggestions for additions or
revisions are welcome at any time ; the file is intentionally short
and additions are added thoughtfully rather than reflexively.

The reading list in § 1 will grow as new process artefacts ship.
The current list reflects the state of the team's discipline as of
2026-05-17 (PR #26).
