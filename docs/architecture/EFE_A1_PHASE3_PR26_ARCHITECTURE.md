# EFE-A1 Phase 3 — PR #26 architecture : G.15 post-mortem template CI-enforced

**Status** : DRAFT (architecture review pending Phase 3 kickoff 2026-09-15)
**Phase** : 3 (Q4-2026 critical-path window)
**Type** : Process + light automation (zero-LOC change to the harness binary, ~600 LOC across CI + scripts + docs)
**Roadmap item** : G.15 — "Post-mortem template CI-enforced (new)"
**Owner** : `@efe-ai/release-eng` (primary) + `@efe-ai/sre` (template owner)
**Authors** : release-eng + SRE leadership
**Reviewers** : 2 minimum ; one MUST be the SRE on-call lead, one MUST be a Phase-2 PR author familiar with the I.1 schema
**Date** : 2026-05-17

**Companion docs** :
- Lessons (this PR's own retrospective, will exist post-merge) : `docs/lessons/PR-26-lessons.md`
- Previous architecture doc (the immediate predecessor) : [`EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](EFE_A1_PHASE3_PR25_ARCHITECTURE.md)
- Roadmap : [`docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`](../roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md)
- Stability contract : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md) § 25 (process invariants) + § 26 (new, this PR)
- Existing post-mortem template : [`docs/postmortems/_template.md`](../postmortems/_template.md) (11 sections, written before this PR ; this PR FREEZES it as the canonical schema)
- Audit-prompt validator (sibling artefact, reference implementation) : [`scripts/audit-prompt-validator.sh`](../../scripts/audit-prompt-validator.sh)
- Master incident runbook : [`docs/runbooks/bench-publish-incident-response.md`](../runbooks/bench-publish-incident-response.md)
- Bypass log (existing pattern this PR extends) : [`docs/process/i1-bypass-log.md`](../process/i1-bypass-log.md)

---

## 0. TL;DR

PR #26 closes G.15, the last open process-discipline item in Phase 3
Q4-2026 and the prerequisite for I.7 (the formal post-mortem invariant
that lands in Q1-2027). The PR ships the automated enforcement that
EFE's existing post-mortem template has been waiting for since the
template itself was written six months ago.

Concretely, PR #26 produces five durable artefacts and one operational
artefact :

1. **The post-mortem template is frozen.** The 11-section template at
   `docs/postmortems/_template.md` is locked into the STABILITY contract
   under a new § 26.1, with the same six-chart-minor freeze horizon as
   every other v1.7 surface. Any change to the template's section set
   becomes a STABILITY-bump PR.
2. **A new validator script,** `scripts/postmortem-validator.sh`,
   following the exact shape of the existing audit-prompt validator
   (stable exit-code contract, colourised output, identical CLI), checks
   that any file under `docs/postmortems/` (except `_template.md`)
   carries the frozen schema and a non-degenerate body.
3. **A new CI workflow,** `.github/workflows/pr26-postmortem-checks.yml`,
   runs the validator in two distinct modes : a *file-level* mode that
   validates every post-mortem file already in the repo on every PR ;
   and a *deadline-level* mode that runs nightly and checks, for every
   GitHub issue closed with the `incident` label, that a corresponding
   post-mortem file has been published within the deadline window
   defined by § 26.4.
4. **A frontmatter contract.** Every post-mortem file MUST carry a
   structured YAML frontmatter block (incident ID, severity, detected /
   mitigated / resolved timestamps, author, reviewers, linked GitHub
   issue, status). The frontmatter is the machine-readable cross-link
   between the incident issue and the post-mortem file. Its schema is
   frozen under § 26.2.
5. **A bypass log.** A new `docs/process/postmortem-bypass-log.md`
   captures every legitimate exception (mislabeled non-incidents,
   merged incidents, drill-class incidents) under the same pattern as
   the existing I.1 bypass log.
6. **A drill rehearsal.** Within seven calendar days of PR #26 merging,
   the SRE team runs a simulated incident in the staging environment,
   deliberately delays the post-mortem file beyond the deadline, and
   verifies that the deadline-mode workflow opens a tracking issue
   exactly as designed. The rehearsal output is appended to PR #26's
   lessons file as the first empirical validation of the work.

PR #26 adds **zero LOC** to the harness binary and adds approximately
**600 LOC** across :

- `scripts/postmortem-validator.sh` (~280 LOC, sibling of the
  audit-prompt validator)
- `.github/workflows/pr26-postmortem-checks.yml` (~120 LOC, three
  jobs)
- `docs/postmortems/_template.md` (modified — adds machine-readable
  frontmatter ; ~25 net LOC added)
- `docs/postmortems/_VALIDATOR_REFERENCE.md` (~80 LOC, an example file
  that the validator's positive-path test exercises)
- `docs/process/postmortem-bypass-log.md` (new, ~60 LOC initial schema)
- `docs/STABILITY_CONTRACT.md` § 26 (~80 LOC, new section)
- `bench/harness/tests/postmortem_schema.rs` (~140 LOC, validator
  positive + negative tests in Rust)
- `bench/harness/tests/stability_contract_v17.rs` (~30 LOC added —
  freeze tests for § 26.1 through § 26.6)
- `docs/runbooks/bench-publish-incident-response.md` (~40 LOC modified
  — adds the post-PM step to the existing runbook)
- `docs/lessons/PR-26-lessons.md` (the retrospective, ~300 LOC, written
  during the audit)

**Success criterion** : within thirty days of the rehearsal drill,
exactly two real or simulated incidents are processed end to end under
the PR #26 workflow without manual override. If three or more bypasses
are recorded in the first thirty days, the audit reopens and we treat
the validator's heuristics as miscalibrated.

---

## 1. Why this PR exists

### 1.1 The gap PR #26 closes

EFE has, since approximately 2025-Q4, possessed a structured
post-mortem template at `docs/postmortems/_template.md`. The template
was written carefully by the SRE leadership team in response to a
specific set of incidents in late 2025 where the lack of structured
post-mortems made it hard for the engineering organisation to learn
from operational pain. The template's eleven sections are themselves
the product of substantial thought about what makes a post-mortem
useful to a reader six months after the incident, and the template
has been in informal use since it was written.

The template has, however, never been enforced. An incident can be
declared, paged out, mitigated, resolved, and closed on the GitHub
issue tracker without any post-mortem ever being written. The SRE
team has, in practice, written post-mortems for the most severe
incidents on a voluntary basis ; lower-severity incidents have
typically not received the same treatment, and even the severe ones
have sometimes been written without using the canonical template.
The result is that EFE's incident-learning surface is uneven : the
incidents that produced post-mortems have generated durable lessons,
and the incidents that did not have not.

This is the gap G.15 closes. PR #26 institutionalises the post-mortem
as a structural property of the incident-response process rather than
a voluntary practice. After PR #26 lands, the closure of any incident
issue starts a deadline clock that, if it expires without a
corresponding post-mortem being committed under the canonical
schema, causes the verification pipeline to open a tracking issue that
the team must explicitly address.

### 1.2 Why now

G.15 was identified as a Phase-3 work item in PR #25's lessons file,
specifically in the "Known residual gaps" section. The reasoning at
the time was that PR #25 had institutionalised three process
invariants targeting *release-time* defects (I.1 audit prompt, I.2
single source of truth, I.6 audit budget), but had left untouched
the *operational-incident* class of defects. A team that ships
audit-grade software but loses the lessons of every operational
incident remains one half of an undisciplined organisation.

The decision to schedule G.15 in Q4-2026 (the first month of Phase
3) rather than later was driven by two specific considerations. The
first is that G.15 is a prerequisite for I.7 (the formal post-mortem
invariant scheduled for Q1-2027), which moves the post-mortem
discipline from "the workflow opens an issue if you miss the
deadline" to "the verification pipeline fails the merge of any PR
in the affected component until the post-mortem lands." I.7 cannot
ship without G.15 in place, and the team wants I.7 to land early
enough in Q1-2027 to have empirical data on its effect before the
Phase-3 mid-cycle review. The second consideration is that the
post-mortem-template work itself is small enough to be a one-week
sprint, which makes it the natural opener of Phase 3's
process-discipline work : it builds momentum on a high-leverage
item without consuming the team's bandwidth for the more
expensive G-stream items that follow.

### 1.3 The empirical motivation

A small audit conducted by the SRE lead during the PR #25 audit
window identified, among the open GitHub issues at the time, **nine
incident-labelled issues that had been closed without a corresponding
post-mortem file existing in the repository**. The nine spanned a
range of severities (one Sev1, three Sev2, five Sev3) and a range of
ages (the oldest closed eight months before the audit). The audit
also identified two post-mortem files in the repository whose section
structure did not match the canonical template, both written by
individuals who were no longer on the team at the time of the audit.

These eleven combined data points (nine missing post-mortems, two
non-canonical post-mortems) define the empirical magnitude of the
problem G.15 is solving. The author of the present document considers
this magnitude small enough that the work is comfortably within a
single-week scope, and large enough that ignoring it would compound
into a substantially larger problem over Phase 3.

### 1.4 What G.15 does NOT do

G.15 enforces the existence and structural validity of post-mortem
files. It does NOT enforce post-mortem quality, post-mortem
correctness, or that the action items in a post-mortem are actually
completed within their due dates. Each of these would be a useful
discipline, and each is in scope for a future PR (most likely I.7's
companion or a successor item), but each requires substantially more
design than G.15's structural enforcement does and is therefore
explicitly out of scope.

The author acknowledges that an unscrupulous team could, in
principle, satisfy G.15's structural validator while writing
post-mortems whose content is hollow. The team's working assumption is
that the cost of writing a hollow post-mortem that nonetheless passes
the validator is roughly the same as the cost of writing a real one,
and that the social pressure inside a small engineering team makes
hollow post-mortems socially expensive. If empirical evidence emerges
that this assumption is wrong, the team will revisit.

---

## 2. Scope

### 2.1 In scope (this PR)

1. Freeze the existing 11-section post-mortem template under STABILITY
   § 26.1, including the section order, the section count, and the
   minimum content requirements for each section.
2. Add a machine-readable YAML frontmatter contract to the canonical
   template, freeze it under § 26.2.
3. Write `scripts/postmortem-validator.sh`, modelled directly on the
   existing `scripts/audit-prompt-validator.sh`, with a stable exit-code
   contract frozen under § 26.3.
4. Define the deadline algorithm that translates an incident issue's
   closure time into a calendar deadline for the corresponding
   post-mortem file. Freeze the algorithm under § 26.4.
5. Define the file-naming convention that links a post-mortem file to
   its corresponding incident issue. Freeze the convention under § 26.5.
6. Build `.github/workflows/pr26-postmortem-checks.yml` implementing
   three jobs : file-level validation on every PR, deadline-level
   nightly check, and an aggregation job that produces the
   single-line summary the team sees on the daily status report.
7. Write `bench/harness/tests/postmortem_schema.rs` with positive and
   negative tests for the validator, sibling of the existing
   `audit_prompt_schema.rs` test file but specialised for the
   post-mortem schema.
8. Add freeze tests for § 26.1 through § 26.6 inside
   `bench/harness/tests/stability_contract_v17.rs`, ensuring any
   surface change to the post-mortem contract fails CI before review.
9. Create `docs/process/postmortem-bypass-log.md` as the
   bypass-tracking file, sibling of the existing
   `docs/process/i1-bypass-log.md`.
10. Update the incident-response runbook at
    `docs/runbooks/bench-publish-incident-response.md` to incorporate
    the new post-mortem step as the closing action of every incident.
11. Run the rehearsal drill described in § 11 and append its outcome
    to `docs/lessons/PR-26-lessons.md` before this PR is merged.

### 2.2 Explicitly deferred to I.7 (Phase 3 Q1-2027)

1. Hard CI gate that fails the merge of any PR touching the affected
   component until the post-mortem lands. PR #26 only opens a tracking
   issue ; PR #27 (I.7) makes the gate refusing.
2. Post-mortem content-quality heuristics (e.g. "timeline has ≥ 5
   entries", "action items have ≥ 3 entries with explicit owners").
   The validator in PR #26 enforces structural presence ; the content
   heuristics will be calibrated against the first quarter of
   real-world post-mortems produced under PR #26.
3. Cross-correlation between post-mortems and the audit-budget tracker
   that PR #25 introduced, so that operational incidents feed into the
   release-engineering audit-time budget.
4. Action-item due-date tracking (a CI job that opens reminder issues
   when a post-mortem's action item passes its due date without being
   marked complete).
5. A public-facing post-mortem digest published as part of the
   quarterly evidence chain (an attractive feature, deferred because
   the customer-facing implications of publishing post-mortems are
   themselves a STABILITY-frozen surface and the design conversation
   has not yet happened).

### 2.3 Explicitly deferred to later phases

1. Integration with external incident-management systems (PagerDuty,
   Opsgenie, etc.). PR #26 relies entirely on GitHub-native primitives
   (issues, labels, comments, the Actions runtime). External
   integration is a Phase-4 J-stream item, gated on a customer who
   actually requires it.
2. Multi-language post-mortems (the canonical template is English
   only ; multi-language is a Phase-4 J.5 federated-identity-adjacent
   item).
3. Automated severity classification of incidents. PR #26 trusts the
   `severity:S1|S2|S3` label set by the incident commander ; automated
   classification is an interesting research direction but not on the
   roadmap.

---

## 3. Vocabulary

The remainder of this document uses the following terms with the
following precise meanings. Several of these terms also appear in
the STABILITY contract under § 26.

**Incident.** An operational event that requires the immediate
attention of the on-call engineer and is tracked in the GitHub issue
tracker with the `incident` label set. The label is set either
automatically by the incident-response runbook's escalation playbook
or manually by the on-call engineer when an issue is reclassified as
an incident after initial triage. An incident has a severity (one of
`S1`, `S2`, `S3`) assigned at incident-creation time and revisable
during the incident's lifetime. The severity is set via a label of
the form `severity:S1`, `severity:S2`, or `severity:S3` on the issue.

**Incident issue.** The specific GitHub issue tracking an incident.
Every incident issue has, at a minimum, the `incident` label and one
of the three severity labels. Most also have additional labels
identifying the affected component (`component:bench-portal`,
`component:slo-portal`, etc.). The issue's lifecycle is `open → closed`,
with the closure time being the start of the post-mortem deadline
clock defined in § 26.4.

**Post-mortem file.** A markdown file in the `docs/postmortems/`
directory whose filename follows the convention defined in § 26.5 and
whose content follows the schema defined in § 26.1 and § 26.2. Exactly
one post-mortem file corresponds to each incident issue, except in the
explicit multi-incident case described in § 26.5.2.

**Canonical template.** The reference post-mortem template at
`docs/postmortems/_template.md`. This file is not itself a
post-mortem ; it is the schema definition. The validator MUST recognise
the template as non-violating ; the deadline workflow MUST exclude
the template from its post-mortem-existence check.

**Validator.** The executable at `scripts/postmortem-validator.sh`
that reads a post-mortem file and returns a structured exit code
indicating whether the file conforms to the canonical schema. The
validator's exit codes are frozen under § 26.3.

**Deadline window.** The duration between an incident issue's closure
time and the latest moment at which the corresponding post-mortem file
must exist in the repository to avoid a workflow violation. The
duration depends on the incident's severity, per § 26.4.

**Deadline violation.** The condition in which a closed incident
issue's deadline window has expired without the corresponding
post-mortem file existing under the canonical schema. A deadline
violation triggers the workflow's tracking-issue-opening behaviour
defined in § 5.3.

**Bypass.** An explicit, logged exception to the post-mortem
requirement for a specific incident. Bypasses are reserved for the
small number of cases described in § 6 and are logged in
`docs/process/postmortem-bypass-log.md`. Bypasses are NOT a routine
operational practice ; a team that issues more than two bypasses in a
calendar quarter is required to justify the rate at the next
quarterly master-runbook review.

**Tracking issue.** The auto-opened GitHub issue produced by the
deadline-mode workflow when it detects a deadline violation. The
tracking issue links to the original incident issue, identifies the
expected post-mortem filename, and is assigned to the original
incident's commander. The tracking issue is closed when the
corresponding post-mortem file is committed.

**Rehearsal drill.** The simulated incident the team runs within
seven days of PR #26 merging, to validate that the workflow behaves
as designed under realistic conditions. The drill's outcome is
appended to the PR's lessons file as empirical evidence that the
work landed correctly.

**The eleven sections.** The eleven section headings of the canonical
template, frozen under § 26.1. They are : (1) Summary, (2) Timeline,
(3) Root cause, (4) Contributing factors, (5) What went well, (6)
What went wrong, (7) Action items, (8) Prevention, (9) Customer
impact, (10) Compliance ramifications, (11) Cross-references. The
order is invariant.

**Frontmatter.** The YAML block at the head of every post-mortem
file, frozen under § 26.2, that carries the machine-readable
metadata linking the file to its incident issue. The frontmatter is
parsed by the validator and by the deadline workflow.

---

## 4. Architecture overview

PR #26 is organised into three loosely-coupled subsystems that
together produce the enforcement behaviour. The subsystems are
described in this section at a 30,000-foot level ; § 5 covers each
of them in detail.

The first subsystem is the **schema layer**. It comprises the
canonical post-mortem template, the YAML frontmatter contract, and
the file-naming convention. The schema layer defines what a
post-mortem file must look like for the validator to accept it.
Everything else in PR #26 depends on the schema layer being stable,
which is why the schema layer is what STABILITY § 26.1, § 26.2, and
§ 26.5 freeze.

The second subsystem is the **validator layer**. It comprises the
`scripts/postmortem-validator.sh` script and its companion
Rust-integration test file `bench/harness/tests/postmortem_schema.rs`.
The validator reads a post-mortem file and returns a structured exit
code indicating whether the file conforms to the schema layer.
Everything else in PR #26 depends on the validator being correct,
which is why the test surface for the validator is large (positive
test cases for every valid shape plus negative test cases for every
exit code).

The third subsystem is the **enforcement layer**. It comprises the
`.github/workflows/pr26-postmortem-checks.yml` workflow with its
three jobs, the deadline algorithm at § 26.4, and the tracking-issue
mechanism. The enforcement layer is where the validator is invoked
against real-world post-mortem files and where deadline violations
are surfaced to the team. The enforcement layer is the only part of
PR #26 that interacts with GitHub's issue tracker and the only part
that performs any external action.

The relationship between the three layers is strictly hierarchical.
The schema layer knows about no other layer. The validator layer
depends on the schema layer but does not know about the enforcement
layer. The enforcement layer depends on both layers below it. This
hierarchy is deliberate : it means that the schema layer can be
exercised by hand (a developer can open the canonical template and
read it), the validator layer can be exercised in isolation (a
developer can run the validator script on any local markdown file),
and the enforcement layer can be exercised in CI (the workflow runs
on every PR). Each layer's tests do not depend on the layers above it,
which keeps the test surface tractable.

The diagram below (described in words, since this document is
intentionally diagram-free for review-tool compatibility) shows the
flow of information through the three layers on the two operational
paths PR #26 supports.

On the *file-level path* (triggered by any PR that touches the
`docs/postmortems/` directory), the workflow's first job invokes the
validator against every file in that directory except `_template.md`.
The validator reads each file, parses the frontmatter, checks the
section headings against the canonical eleven, checks the timeline
table has at least two non-header rows, checks the action-items table
has at least one non-header row, and returns a structured exit code.
The job aggregates the exit codes and either passes the check or
emits a structured failure message identifying each non-conforming
file.

On the *deadline-level path* (triggered nightly at 03:00 UTC via the
workflow's cron trigger), the workflow's second job queries GitHub's
API for every issue with the `incident` label that was closed within
the lookback window (default : 30 calendar days, configurable). For
each such issue, the job computes the deadline window from the
issue's severity and closure time, checks whether a post-mortem file
exists under the expected filename, and if the deadline has expired
without the file existing, the job ensures a tracking issue is open
for the violation. The tracking-issue-opening is idempotent : if a
tracking issue already exists for this incident, the job does not
open a duplicate.

The third job, the aggregation job, runs only after the first two
have completed. It produces a single-line summary of the day's
state (number of post-mortems present, number of deadline violations,
number of bypasses active) and writes the summary into a
public-facing file at `docs/process/postmortem-status.md`. The file
is auto-committed by the workflow under the same change-control
policy that governs the rest of the public-facing site (signed
commits, force-push disabled, CODEOWNERS review on changes).

---

## 5. Detailed design

### 5.1 The schema layer

#### 5.1.1 The eleven sections

The canonical template defines eleven sections. The set, the order,
and the heading text are all part of the frozen schema. Any change
to the section set is a STABILITY-bump PR.

The eleven sections are :

1. **Summary** — 3-4 sentences of plain English describing what
   happened. Defines abbreviations. Audience : anyone in the
   organisation who needs to know that this incident happened, including
   non-engineers.
2. **Timeline** — UTC timestamps in a markdown table. Columns : Time,
   Event. Minimum two rows (the first signal and the all-clear), but
   in practice typically five to fifteen.
3. **Root cause** — one paragraph, direct and unambiguous. If multiple
   causes contributed, the section lists them in order of contribution
   and cross-links the supporting evidence (logs, dashboards, code
   references).
4. **Contributing factors** — bullet list of organisational, process,
   or systemic factors that made the root cause more likely to occur
   or more painful when it did.
5. **What went well** — bullet list of specific actions that helped
   contain or resolve the incident. Important for organisational
   morale and for capturing positive patterns to repeat.
6. **What went wrong** — bullet list of specific gaps in detection,
   diagnosis, communication, mitigation, or recovery.
7. **Action items** — markdown table with columns : Action, Owner,
   Linear, Due, Status. Minimum one row. Every action has a single
   accountable person, a Linear issue ID, a due date, and an
   acceptance criterion.
8. **Prevention** — bullet list of the highest-leverage controls that
   prevent recurrence : new SLOs, new pin tests, new runbook sections,
   new CI gates, architectural changes. Each item documents its
   decision criteria so future reviewers can audit whether the
   prevention worked.
9. **Customer impact** — markdown table with columns : Tenant, Surface,
   Duration, Customers impacted, Communication sent. Can have zero
   rows for incidents that did not affect customers ; the section
   itself is non-optional.
10. **Compliance ramifications** — checklist of compliance-related
    actions (DPO notification, GDPR Art. 33 assessment, AI Act Art. 9
    register update, supervisory authority notification, trust center
    update). Each checkbox is either ticked or explicitly N/A.
11. **Cross-references** — bullet lists of runbooks consulted,
    dashboards used, ADRs proposed, releases involved.

The validator enforces the presence of each of the eleven section
headings, in the exact order specified, with the exact heading text
specified. The heading text is matched against a regular expression
that tolerates trailing whitespace, level-2 versus level-3 heading
syntax (i.e. `##` vs `###`), and the optional addition of an
em-dash-introduced caveat after the heading text. The regex is
defined in § 5.2.4.

The validator does NOT enforce that the content under each heading
is non-empty in any deep sense — that's reserved for I.7's
content-quality heuristics — except for two sections whose
emptiness is so destructive that the validator does check : the
Timeline (which must have at least two non-header rows) and the
Action items (which must have at least one non-header row).

#### 5.1.2 The frontmatter

Every post-mortem file MUST begin with a YAML frontmatter block of
the following exact shape :

```yaml
---
incident_id: INC-2026-NNN
severity: S1 | S2 | S3
detected_at: YYYY-MM-DDTHH:MM:SSZ
mitigated_at: YYYY-MM-DDTHH:MM:SSZ
resolved_at: YYYY-MM-DDTHH:MM:SSZ
author: github-username
reviewers:
  - github-username-1
  - github-username-2
github_issue: https://github.com/efe-ai/efe/issues/NNNN
status: DRAFT | REVIEW | PUBLISHED
---
```

Each field is mandatory. The validator parses the YAML and rejects
the file if any field is missing, if any field's value does not match
its expected shape (the validator does light schema validation on
each field), or if the YAML block itself is malformed.

The field semantics are :

- `incident_id` — the unique identifier of the incident, in the
  format `INC-YYYY-NNN` where YYYY is the calendar year of the
  incident's detection and NNN is a sequential number scoped to the
  year. The identifier is assigned by the incident-response runbook
  at the moment the incident is opened, and it is also the basis of
  the post-mortem filename (§ 26.5).
- `severity` — one of `S1`, `S2`, or `S3`, matching the severity
  label set on the incident issue. The deadline algorithm at § 26.4
  uses this field.
- `detected_at` — the UTC timestamp of the first signal (alert,
  ticket, monitor) that identified the incident. Format is ISO 8601
  with the trailing `Z` mandatory.
- `mitigated_at` — the UTC timestamp at which the customer-facing
  impact ended. May equal `resolved_at` for incidents where mitigation
  and resolution happened simultaneously.
- `resolved_at` — the UTC timestamp at which the root cause was fully
  addressed (as distinct from `mitigated_at` for incidents where
  initial mitigation was a workaround and full resolution came later).
- `author` — the GitHub username of the engineer responsible for
  writing the post-mortem. Typically the incident commander, but
  occasionally delegated.
- `reviewers` — list of at least two GitHub usernames who have
  reviewed and approved the post-mortem. The validator enforces the
  minimum count of two. At least one of the reviewers MUST be a
  CODEOWNER of the affected component, but the validator does not
  enforce this constraint (a future I.7 work item may).
- `github_issue` — the full URL of the corresponding GitHub issue.
  The deadline workflow uses this field to verify that the
  post-mortem covers the incident it claims to cover.
- `status` — one of `DRAFT`, `REVIEW`, `PUBLISHED`. A
  post-mortem in `DRAFT` status counts as existing for the purposes
  of the deadline check (the file is present), but is flagged in
  the aggregation report as not yet finalised.

#### 5.1.3 The filename convention

Post-mortem files follow the convention :

```
docs/postmortems/<incident_id>.md
```

where `<incident_id>` matches the `incident_id` field of the
frontmatter. Therefore, given an incident issue with the assigned
identifier `INC-2026-007`, the corresponding post-mortem file MUST
be at `docs/postmortems/INC-2026-007.md`.

The filename is the primary key the deadline workflow uses to verify
the existence of a post-mortem. It does NOT use the file's content,
because checking the file's content would require parsing every
markdown file in the directory on every workflow run, which is
operationally expensive. The filename-based lookup is O(1) per
incident issue.

There is one structured exception to the filename convention,
described in § 26.5.2 : *multi-incident post-mortems*, in which a
single post-mortem file covers two or more related incidents. Such
a file's name takes the form `INC-2026-007+009.md` (with the
incident identifiers concatenated by `+`), and the frontmatter's
`incident_id` field becomes a list rather than a scalar. The
validator and the deadline workflow both recognise this exception.

#### 5.1.4 The reference example file

Alongside the canonical template, PR #26 ships
`docs/postmortems/_VALIDATOR_REFERENCE.md`. This file is a fictional
but fully-formed post-mortem against the frozen schema. It serves
three purposes : it is the validator's positive-test fixture
(`postmortem_schema.rs::validator_accepts_canonical_reference`), it
is the documentation example any new author can copy when writing
their first real post-mortem, and it is the reference the deadline
workflow uses in its self-test to verify that a known-good file is
recognised.

The reference file's incident ID is `INC-2026-000` (the zero
suffix is reserved for the reference example and never used for
real incidents). The reference file's content is deliberately
plausible : it describes a fictional but realistic incident in
which the bench portal's quarterly publish workflow failed because
the cosign keyless OIDC token expired mid-pipeline, the team
detected the failure within nine minutes, mitigated by manual
retry, and resolved by adjusting the workflow's token refresh
policy. The reference file is approximately eighty lines of
markdown and is referenced from the canonical template's
introduction as a worked example.

### 5.2 The validator layer

#### 5.2.1 Script structure

`scripts/postmortem-validator.sh` is implemented as a single bash
script following the exact structural conventions of the existing
`scripts/audit-prompt-validator.sh`. The script is approximately
280 lines including documentation, has no external dependencies
beyond bash, awk, and grep (all present on every supported
platform), and is callable both locally by developers and from the
CI workflow.

The script is structured as five sections in source order :

1. **Documentation block** — a ~50-line header comment describing
   the script's purpose, its invocation, its exit codes (the stable
   contract under § 26.3), and references to the architecture
   document and the STABILITY contract.
2. **Helper functions** — colourised output helpers
   (`log_err`, `log_warn`, `log_ok`, `log_info`), TTY-detection
   logic for the colour codes, and the usage-printing function.
3. **Argument parsing** — accepts exactly one argument, the path to
   the post-mortem file under validation. Validates that the
   argument is present, that the path exists, that the path points
   to a regular file, and that the file is non-empty.
4. **Validation checks** — six structured checks, each producing a
   specific exit code if it fails. The checks are described in
   § 5.2.2 below.
5. **Success summary** — if all six checks pass, the script prints
   a structured success summary including the incident ID, the
   severity, the number of timeline entries, the number of action
   items, and the file's status.

The script's stable contract is its exit-code surface (§ 26.3).
The internal organisation of the validation checks can change
between minor versions of the script, but the exit codes cannot.

#### 5.2.2 The six validation checks

The validator runs six checks in sequence. Each check produces a
specific exit code if it fails ; if a check fails, the validator
exits immediately without running the subsequent checks. The order
of the checks is deliberate : the cheaper checks run first so that
a structurally broken file is rejected without needing to parse the
expensive sections.

**Check 1 : path validity.** The argument was provided, the path
exists, the path is a regular file, and the file is non-empty. Exit
code on failure : 2.

**Check 2 : frontmatter parses.** The file begins with a `---`
delimiter followed by valid YAML followed by a closing `---`
delimiter. Every mandatory field is present, and every field's value
matches its expected shape (regex check for incident_id, enum check
for severity, ISO-8601 check for the three timestamps, plain-string
check for author, list-of-strings check for reviewers, URL check for
github_issue, enum check for status). Exit code on failure : 3.

**Check 3 : eleven sections present, in order.** The file contains
exactly the eleven canonical section headings, in the exact order
specified, with the heading text matching the regex defined in
§ 5.2.4. Exit code on failure : 4.

**Check 4 : timeline has ≥ 2 non-header rows.** Between the
"Timeline" heading and the next heading, the file contains a
markdown table with at least two rows of content beyond the header
and the separator. Exit code on failure : 5.

**Check 5 : action items have ≥ 1 non-header row.** Between the
"Action items" heading and the next heading, the file contains a
markdown table with at least one row of content beyond the header
and the separator. Exit code on failure : 6.

**Check 6 : compliance checklist is fully addressed.** The
"Compliance ramifications" section's checklist has every item
either ticked (`- [x]`) or explicitly marked as not applicable (the
text "N/A" appears on the same line). No checkbox may be left in
the un-addressed state (`- [ ]` with no N/A annotation). Exit code
on failure : 7.

#### 5.2.3 The stable exit-code contract

The validator's exit codes are the validator's API. They are frozen
under STABILITY § 26.3 for the next six chart-minor versions. Any
change to the exit codes is a STABILITY-bump PR.

The complete exit-code table :

- `0` — validation passed.
- `2` — file argument missing, path does not exist, path is not a
  regular file, or file is empty.
- `3` — frontmatter missing, malformed YAML, or any mandatory field
  missing or of wrong shape.
- `4` — one or more of the eleven canonical sections is missing, the
  section order is wrong, or a section heading does not match the
  canonical text.
- `5` — the Timeline section has fewer than two non-header rows.
- `6` — the Action items section has fewer than one non-header row.
- `7` — one or more of the Compliance ramifications checkboxes is
  unaddressed.

The validator never returns exit code 1 (reserved for unexpected
runtime errors such as a broken bash interpreter). Exit codes 8 and
above are reserved for future check additions ; until I.7 lands,
they are not used.

The choice of exit codes 2–7 rather than 1–6 is deliberate. Exit
code 1 is the conventional "general failure" code in unix shell
conventions, and we deliberately leave it unused so that a script
that wraps the validator and catches its exit code can distinguish
between "the validator itself failed" (exit 1) and "the validator
found a structured violation" (exit 2-7).

#### 5.2.4 The section-heading regex

The validator matches section headings against the following regex
shape (illustrated for the "Timeline" section ; analogous regexes
are used for the other ten) :

```
^##+[[:space:]]+2\.[[:space:]]+Timeline\b
```

The regex is anchored to the start of a line, allows `##` or `###`
or deeper heading levels, requires at least one whitespace
character between the heading marker and the section number,
matches the literal `2.` as the section number, requires at least
one whitespace after the period, matches the literal `Timeline`,
and uses a word boundary (`\b`) to allow trailing content (e.g. an
em-dash and a caveat) without rejecting the heading.

The eleven regexes are stored as a bash array in the validator
script and matched in order. The order check (Check 3 in § 5.2.2)
ensures that section *N* appears before section *N + 1* in the
file ; the absence check is performed at the same time, by failing
the order check if a regex fails to match anywhere in the file.

The deliberate choice of allowing both `##` and `###` heading
levels reflects the fact that the canonical template uses `##` for
section headings, but some authors prefer to nest the post-mortem
under a top-level `#` heading and use `###` for sections. Both
shapes are valid ; the validator does not enforce the heading depth.

#### 5.2.5 The Rust integration test

`bench/harness/tests/postmortem_schema.rs` is a Rust integration
test file that exercises the validator against a comprehensive set
of fixtures. The file is approximately 140 LOC and contains the
following tests :

- `validator_accepts_canonical_reference` — runs the validator
  against `docs/postmortems/_VALIDATOR_REFERENCE.md` and asserts
  exit code 0.
- `validator_rejects_missing_file` — runs the validator with a
  non-existent path and asserts exit code 2.
- `validator_rejects_empty_file` — creates a temporary empty file
  and asserts exit code 2.
- `validator_rejects_missing_frontmatter` — creates a temporary
  file with no frontmatter and asserts exit code 3.
- `validator_rejects_malformed_yaml` — creates a temporary file
  with YAML that fails to parse and asserts exit code 3.
- For each of the nine mandatory frontmatter fields, a
  corresponding test (`validator_rejects_missing_incident_id`,
  etc.) that omits exactly that field and asserts exit code 3.
- `validator_rejects_invalid_severity` — uses a severity value
  outside the `S1|S2|S3` enum and asserts exit code 3.
- For each of the eleven canonical sections, a corresponding test
  (`validator_rejects_missing_summary`, etc.) that omits exactly
  that section and asserts exit code 4.
- `validator_rejects_wrong_section_order` — re-orders two
  sections and asserts exit code 4.
- `validator_rejects_empty_timeline` — produces a file with a
  Timeline section containing only the header and separator and
  asserts exit code 5.
- `validator_rejects_empty_action_items` — analogous for action
  items, exit code 6.
- `validator_rejects_unaddressed_compliance` — produces a file
  with a `- [ ]` checkbox lacking N/A annotation and asserts exit
  code 7.

Each negative test uses Rust's `tempfile` crate to create a
throwaway file, write the malformed content, and invoke the
validator via `std::process::Command`. The exit code of the
spawned process is asserted with `assert_eq!`. The total runtime
of all tests in this file is well under one second on standard
CI hardware.

### 5.3 The enforcement layer

#### 5.3.1 The workflow's three jobs

`.github/workflows/pr26-postmortem-checks.yml` defines three jobs.
They are described in this section in the order in which the
workflow runs them.

**Job 1 : `validate-files`**. Triggered by any PR that modifies a
file under `docs/postmortems/` (matched via the workflow's
`paths` filter on push and pull_request events). The job checks
out the repository, then iterates over every file in
`docs/postmortems/` (excluding `_template.md`,
`_VALIDATOR_REFERENCE.md`, and any `_*.md` file matching the
underscore-prefixed convention reserved for non-incident
files). For each file, the job invokes
`scripts/postmortem-validator.sh` and captures the exit code. The
job succeeds if every invocation returns exit code 0 ; it fails
otherwise, with the failure message identifying each
non-conforming file and its exit code. The job's timeout is set
to two minutes, which is more than ample for the worst-case fleet
size we anticipate over the freeze horizon.

**Job 2 : `check-deadlines`**. Triggered by the workflow's cron
schedule (daily at 03:00 UTC). The job uses the GitHub CLI to
query the issue tracker for every issue with the `incident` label
that is in the closed state and was closed within the lookback
window (default 30 calendar days, configurable via the workflow's
`lookback_days` input). For each such issue, the job extracts the
incident ID from the issue's first body line (the runbook requires
the first line to be of the form `Incident ID: INC-YYYY-NNN`),
computes the deadline using the algorithm at § 26.4, and checks
whether `docs/postmortems/<incident_id>.md` exists. If the file
does not exist and the deadline has expired, the job checks for
an existing tracking issue (identified by a `tracking-for: INC-YYYY-NNN`
label) and, if no such tracking issue exists, opens one with the
following properties : title `[POST-MORTEM OVERDUE] INC-YYYY-NNN`,
body containing a link to the original incident issue and the
expected post-mortem filename, labels `post-mortem-overdue` and
`tracking-for:INC-YYYY-NNN`, and assigned to the original
incident's commander (extracted from the original issue's
`assignee` field or, if absent, from the first comment's author).

**Job 3 : `aggregate`**. Triggered by the successful completion
of both Job 1 and Job 2. The job collects the outputs of the two
preceding jobs and writes a single-line summary into
`docs/process/postmortem-status.md`. The summary's format is
fixed : a YAML block at the top with the date of the run, the
number of post-mortems present, the number of open tracking
issues, and the number of active bypasses ; followed by a
markdown table listing the active tracking issues with their
ages. The job then commits the updated file to the repository
under the signed-commit policy. The aggregation job is the only
job that performs a write to the repository ; the validation jobs
are read-only.

#### 5.3.2 The deadline algorithm

The deadline algorithm computes, given an incident issue's
severity and closure time, the latest UTC timestamp at which the
corresponding post-mortem file must exist for the workflow to
consider the deadline met.

The algorithm is :

```
deadline(severity, closure_time):
    base_window_days = match severity:
        S1 → 7 calendar days
        S2 → 14 calendar days
        S3 → 30 calendar days
    deadline = closure_time + base_window_days
    if deadline falls on a Saturday or Sunday:
        deadline = next Monday at 23:59:59 UTC
    if deadline falls on an EFE-recognised public holiday:
        deadline = next business day at 23:59:59 UTC
    return deadline
```

The base window choices reflect the operational intuition that
high-severity incidents warrant faster follow-through (because the
lessons are most expensive to lose) while low-severity incidents
can afford a longer window (because they are less time-sensitive
and the post-mortem author may need time to coordinate with
adjacent teams).

The weekend and holiday adjustments are deliberate. An incident
closed on a Saturday afternoon would, under naive arithmetic,
have a deadline falling on a subsequent weekend day. The
adjustment shifts the deadline to the next business day, which
both reflects the team's actual working pattern and avoids the
appearance of an unforgiving deadline. The list of recognised
public holidays is maintained in
`docs/process/efe-business-calendar.md`, which PR #26 ships as
the initial version covering Swedish national holidays plus the
European-Christmas and European-Easter clusters. Customers in
other geographies would, in a future PR, contribute their local
holiday lists.

The algorithm has one additional edge case : an incident whose
severity is changed after closure. If the severity on the
incident issue is increased post-closure (e.g. the team
discovered the impact was worse than initially assessed), the
deadline is recomputed against the higher severity, which
shortens the window. If the severity is decreased post-closure,
the deadline is recomputed against the lower severity, which
extends the window. The algorithm always uses the *current*
severity label at the moment the deadline check runs, not the
severity at closure. This is to ensure that the team has the
flexibility to reclassify incidents in light of subsequent
information without being penalised by an outdated deadline.

#### 5.3.3 The tracking-issue lifecycle

When the deadline workflow detects a deadline violation, it
opens a tracking issue with the structure described in § 5.3.1.
The tracking issue's lifecycle is then governed by the following
rules, all of which are implemented within the workflow itself :

- The tracking issue remains open until the corresponding
  post-mortem file appears in the repository (detected by Job 1
  running successfully against the new file).
- When the post-mortem file appears, the next nightly run of
  Job 2 detects the file's existence and closes the tracking
  issue with a comment linking to the post-mortem.
- If the tracking issue is open for more than thirty calendar
  days, the workflow opens a *secondary* tracking issue,
  assigned to the engineering lead, with the title `[POST-MORTEM
  ESCALATION] INC-YYYY-NNN` and a body explaining that the
  original tracking issue has not been resolved within thirty
  days. The escalation is a hard signal that the team's
  post-mortem discipline is slipping ; it is discussed at the
  next quarterly master-runbook review.
- A tracking issue may be closed manually by an engineering lead
  with the `[post-mortem-bypass]` label set on the closing
  comment. This is the bypass mechanism described in § 6 and is
  logged accordingly.

The tracking issue's content is templated, with the template
stored in the workflow YAML itself rather than in a separate
file. The templating choice is deliberate : it keeps the
workflow self-contained and makes the workflow's behaviour
easier to audit (a reviewer can read the workflow's YAML and see
exactly what the tracking issue will look like).

### 5.4 The bypass mechanism

#### 5.4.1 When a bypass is legitimate

A bypass is the deliberate, logged exception to the post-mortem
requirement for a specific incident. PR #26 recognises three
legitimate bypass classes ; any bypass outside these classes is
treated as a process violation and discussed at the next
quarterly review.

**Class 1 : mislabeled non-incident.** An issue was labeled
`incident` and subsequently closed, but on inspection the issue
was not an operational incident in the sense the runbook means.
Examples : an issue mistakenly labeled by a webhook integration,
an issue that turned out to be a feature request mis-triaged. The
bypass is recorded with the class `MISLABEL` and a one-paragraph
justification.

**Class 2 : merged-into-another-incident.** An issue was opened
as an incident in good faith but turned out to be a duplicate or
subset of another incident that already has a post-mortem in
progress. The bypass is recorded with the class `MERGED` and a
link to the surviving incident's post-mortem.

**Class 3 : drill-class.** The issue was opened as part of an
intentional drill (the quarterly defacement drill, the annual
revocation drill, the rehearsal drill that PR #26 itself
introduces, or any future drill). Drill issues are marked with
the `drill` label in addition to `incident`, and the workflow
recognises the `drill` label as automatically exempting the
issue from the post-mortem requirement. No manual bypass log
entry is needed for drill-class exemptions ; the workflow logs
them automatically.

#### 5.4.2 The bypass-log file

`docs/process/postmortem-bypass-log.md` is the durable record of
every Class 1 and Class 2 bypass ever issued. The file follows
the same shape as the existing `docs/process/i1-bypass-log.md` :
a YAML frontmatter block followed by a markdown table of
bypass entries.

The frontmatter contains the file's owner (`@efe-ai/release-eng`)
and the file's review cadence (quarterly, at the master-runbook
review). The table has columns : Date, Incident ID, Class,
Engineer who applied the bypass, One-paragraph justification,
Link to the closing comment on the tracking issue. The validator
at `scripts/postmortem-validator.sh` does NOT validate the
bypass log ; the bypass log is reviewed manually at the
quarterly review.

The quarterly review's role is twofold : to verify that every
bypass was legitimately Class 1 or Class 2, and to check whether
the bypass rate is increasing. A bypass rate above two per
quarter triggers an explicit conversation at the next review
about whether the deadline algorithm needs adjustment, whether
the team's labelling discipline needs improvement, or whether
the bypass mechanism itself is being abused.

### 5.5 STABILITY contract additions

PR #26 adds a new section § 26 to `docs/STABILITY_CONTRACT.md`,
freezing the surfaces introduced by this PR for the standard
six-chart-minor horizon. The new section's structure is :

- **§ 26.1** — The eleven canonical sections (set, order, heading
  text). Locked by `stability_contract_v17.rs::pm_canonical_sections`.
- **§ 26.2** — The frontmatter schema (field names, field shapes,
  field semantics). Locked by `stability_contract_v17.rs::pm_frontmatter_schema`.
- **§ 26.3** — The validator's exit-code contract (codes 0, 2, 3,
  4, 5, 6, 7). Locked by
  `stability_contract_v17.rs::pm_validator_exit_codes`.
- **§ 26.4** — The deadline algorithm (severity → window, weekend
  adjustment, holiday adjustment, severity-change semantics).
  Locked by `stability_contract_v17.rs::pm_deadline_algorithm`.
- **§ 26.5** — The filename convention (`docs/postmortems/INC-YYYY-NNN.md`)
  and the multi-incident exception. Locked by
  `stability_contract_v17.rs::pm_filename_convention`.
- **§ 26.6** — The bypass mechanism (three classes, the
  bypass-log location). Locked by
  `stability_contract_v17.rs::pm_bypass_classes`.

Each lock is a Rust integration test that loads the corresponding
artefact (the template, the validator script, the workflow YAML,
the bypass log) and asserts that the surface matches the frozen
expectation. A drift in any of the six surfaces fails CI on the
PR that introduces the drift, well before review.

The six new freeze tests bring the total v1.7 STABILITY test
count from 35 to 41, and the platform's total test surface from
440 to approximately 446. The exact final count will be
documented in PR #26's lessons file once the implementation is
complete.

---

## 6. The interaction between PR #26 and PR #25

PR #26 is the second PR in Phase 3, following directly from PR #25
(the institutionalisation of I.1, I.2, I.6). The two PRs share a
common pattern : both add structural automation that enforces a
specific kind of discipline against the team's future work. It is
worth describing in this section how the two PRs interact, because
the interaction is what produces the cumulative effect that the
investor brief and the lessons files refer to.

PR #25 introduced three invariants at *release time* : every
substantial release carries a retrospective, no two locations
declare overlapping facts, every release declares its audit budget
up front. PR #26 introduces one invariant at *incident time* :
every closed incident issue carries a corresponding post-mortem
under the canonical schema within the severity-appropriate deadline.

The release-time invariants and the incident-time invariant compose
in the following way. The release retrospective (PR #25's first
rule) captures the lessons of a *planned* event : the lessons of
shipping a new release. The post-mortem (PR #26's invariant)
captures the lessons of an *unplanned* event : the lessons of an
operational incident. Together, the two surfaces cover the team's
entire learning surface : every meaningful event in the team's
working life produces a structured retrospective document, either
the lessons file from PR #25 or the post-mortem file from PR #26.

The two kinds of documents are deliberately different in shape, for
the deliberately different audiences they serve. The lessons file
serves the team itself, retrospectively, as the calibration anchor
for future releases. The post-mortem serves the broader
organisation, including non-engineering stakeholders, as the
durable explanation of what an incident was, why it happened, and
what changed because of it. The two shapes reflect the two
audiences ; we did not attempt to merge them into a single
document, and we are unlikely to do so in the foreseeable future.

The validator scripts for the two surfaces are deliberately
implemented in parallel. `scripts/audit-prompt-validator.sh` from
PR #25 and `scripts/postmortem-validator.sh` from PR #26 share the
same colourised-output helpers, the same exit-code-stability
discipline, the same documentation-block structure, the same
TTY-detection logic. The parallelism is not accidental : we want
new engineers reading either validator to recognise the pattern
immediately, and we want future validators (for I.7's content
heuristics, for hypothetical Q1 work) to follow the same shape.
The shape itself is becoming an internal convention worth
preserving.

The CI workflows for the two surfaces are likewise structurally
parallel. `pr25-process-checks.yml` and `pr26-postmortem-checks.yml`
share the same job-naming conventions, the same timeout patterns,
the same colour-coded status outputs in the CI summary. A future
engineer adding a third or fourth process-discipline workflow will
have two reference examples to follow rather than only one, and
the convention will be that the third workflow follows the
established shape rather than inventing a new one.

The STABILITY contract sections § 25 (added by PR #25) and § 26
(added by PR #26) sit together in the contract and are reviewed
together at every quarterly contract review. The two sections
together constitute the contract's *process surface* — the part
of the platform's stable promises that govern the team's working
practice rather than the platform's external behaviour.

---

## 7. Test surface

PR #26 introduces, alters, or removes the following tests. The
total net addition is approximately 170 LOC of new test code, with
a few minor additions to existing files.

### 7.1 New : `bench/harness/tests/postmortem_schema.rs`

This file is described in detail in § 5.2.5. It contains
approximately 25 individual test functions, each exercising one
specific behaviour of the validator. The file is approximately
140 LOC, depends only on the standard library plus `tempfile`
(already in the workspace's dependency closure), and runs in
well under one second.

### 7.2 Added : freeze tests in `bench/harness/tests/stability_contract_v17.rs`

Six new test functions are added to the existing v1.7 stability
file, one per § 26 subsection, as listed in § 5.5. The functions
each load the corresponding artefact and assert that its surface
matches the frozen expectation. The additions are approximately
30 LOC.

### 7.3 Added : the `pr26-postmortem-checks.yml` workflow's self-test

The workflow itself contains a self-test job that runs once at
PR-creation time and verifies that the validator can be invoked
against the reference example file (`_VALIDATOR_REFERENCE.md`)
and returns exit code 0. The self-test guards against shipping a
workflow that is itself broken. The job is approximately 15 LOC
of YAML embedded inside the workflow.

### 7.4 Modified : the `audit-prompt-validator.sh` documentation block

A small modification (~5 LOC) to the audit-prompt validator's
documentation block adds a cross-reference to the post-mortem
validator, so that a reader of either file finds the other. The
modification is cosmetic but contributes to the parallel-validator
pattern described in § 6.

### 7.5 Manual test : the rehearsal drill

The rehearsal drill described in § 11 is itself a manual test,
albeit not an automated one. The drill verifies that the
deadline workflow opens a tracking issue when expected, that the
tracking issue contains the expected fields, that the issue is
closed when the post-mortem appears, and that the aggregation
job records the drill correctly in `postmortem-status.md`. The
drill is not run from the CI pipeline ; it is run by hand by the
SRE lead and the release-engineering lead together, with the
output captured in the lessons file.

### 7.6 Total impact on the test count

- Before PR #26 : 440 tests passing.
- After PR #26 : approximately 466 tests passing (440 + ~25 from
  `postmortem_schema.rs` + 6 from the new freeze tests + 1
  workflow self-test test).

The platform's test count thus grows by approximately six percent
in this PR. The growth rate is in line with the budget tracker's
expectation for a discipline-class PR.

---

## 8. Migration and backfill

### 8.1 Pre-PR-26 post-mortems

Two post-mortem files exist in the repository at the time of this
writing whose section structure does not conform to the canonical
template (as identified in the SRE audit referenced in § 1.3).
These two files are explicitly grandfathered : they will continue
to live at their existing paths, the validator will be configured
to skip them via an explicit allow-list at the top of the
validator script, and the long-term plan is to rewrite them under
the canonical schema during Q1-2027 as a low-priority background
task.

The allow-list mechanism is structurally similar to the existing
PR #24 lessons file allow-list in the audit-prompt validator (the
PR #24 lessons file is grandfathered because it predates the I.1
schema). The allow-list lives at the top of the validator script
in a comment-delimited block, and any addition to it requires the
PR to explicitly document the justification.

### 8.2 Pre-PR-26 incidents

Approximately nine incidents closed in the period 2025-Q4 through
2026-Q2 are missing post-mortems (as identified in § 1.3). The
question of whether to backfill post-mortems for these incidents
is non-trivial. The author of this document has weighed the two
options and recommends the following hybrid approach :

- The single Sev1 incident from this period MUST be backfilled
  with a full post-mortem written under the canonical schema. The
  backfill is scheduled for Q4-2026 and will be authored by the
  original incident commander.
- The three Sev2 incidents from this period SHOULD be backfilled
  if the original incident commanders agree to write them. If a
  commander declines, the SRE lead authors a *condensed* post-mortem
  capturing the available evidence, and the file is marked with
  a `status: PUBLISHED_RETROSPECTIVE` value distinguishing it
  from incidents post-mortemed in real time.
- The five Sev3 incidents from this period are NOT required to be
  backfilled. They are documented in the bypass log under the
  class `PRE_PR26_S3` (a fourth bypass class introduced
  specifically for this backfill, then retired once the backfill
  is complete).

The backfill effort is approximately one engineer-week in total
and is scheduled in the second half of Q4-2026, after PR #26
itself is in production. The backfill is not part of PR #26's
scope ; PR #26 only establishes the policy.

### 8.3 The deadline workflow's lookback window

The deadline-mode workflow's default lookback window is 30
calendar days, which means that on the day PR #26 lands, the
workflow's first nightly run will check incidents closed in the
preceding 30 days. To avoid the workflow opening tracking issues
for the incidents identified in § 8.2 (which we are handling via
the backfill mechanism), the workflow's first 60 days of
operation use a 0-day lookback window. After 60 days, the
lookback is restored to 30 days, by which time the backfill is
complete and the team is fully in steady state.

The transient lookback configuration is implemented via the
workflow's `lookback_days` input, which defaults to 0 for the
first 60 days (controlled by an `if` condition checking the
current date against a constant) and 30 thereafter. The constant
is documented in the workflow YAML and is itself a STABILITY-
frozen value once PR #26 lands.

---

## 9. Operational concerns

### 9.1 Runbook integration

The existing master incident-response runbook at
`docs/runbooks/bench-publish-incident-response.md` is amended by
PR #26 to add a final closing step. The step reads, in the
runbook's voice :

> **Closing step : write the post-mortem.** Within the window
> specified by your incident's severity (S1 : 7 days, S2 : 14
> days, S3 : 30 days), create a file at
> `docs/postmortems/<incident-id>.md` following the canonical
> template at `docs/postmortems/_template.md`. The file must
> validate cleanly under
> `scripts/postmortem-validator.sh <path>`. The
> validation is run automatically on every PR that touches the
> directory, and the deadline is checked nightly. If you cannot
> meet the deadline, request a bypass at the next standup with
> a one-paragraph justification, and the engineering lead will
> classify the request as a Class 1, Class 2, or non-legitimate
> bypass.

The runbook amendment is approximately 40 LOC of markdown and is
itself reviewed under the I.2 invariant from PR #25 (no parallel
declarations : the canonical statement of the post-mortem
requirement lives in the STABILITY contract § 26, and the runbook
links to it rather than restating it).

### 9.2 Quarterly review participation

The post-mortem discipline becomes a standing item on the
quarterly master-runbook review starting with the first review
after PR #26 ships. The review's post-mortem segment covers, in
this order : (a) the number of incidents that occurred in the
quarter, broken down by severity ; (b) the number of post-mortems
that met their deadlines ; (c) the number of bypasses by class ;
(d) any escalations (secondary tracking issues opened, per
§ 5.3.3) ; (e) a brief discussion of the most operationally
significant post-mortem written in the quarter.

The review's output is documented in a quarterly summary file
under `docs/process/quarterly-postmortem-summary-<quarter>.md`,
which is itself the kind of structured-prose document the team
maintains. The file is not subject to validator enforcement (it
is not a post-mortem file, just a summary). It is part of the
team's institutional memory, available to new hires as part of
their onboarding reading.

### 9.3 Drill cadence

PR #26 establishes a new quarterly drill : the *post-mortem
drill*. The drill simulates an incident in the staging
environment (with no production impact), runs the full incident
response process, deliberately misses the post-mortem deadline,
and verifies that the deadline workflow opens a tracking issue.
The drill is performed by a rotating team member each quarter,
not always the same person, to ensure that the discipline is
distributed.

The first drill is the rehearsal drill described in § 11, which
runs within seven days of PR #26 merging. Subsequent drills are
scheduled in line with the existing quarterly cadence : last
Friday of each calendar quarter, with the explicit goal of
testing the drift behaviour of the workflow over time.

### 9.4 Onboarding integration

New engineers joining the team are expected to read the
canonical template, the validator script, and the reference
example as part of their first-week onboarding. The onboarding
documentation at `docs/onboarding/contributor-guide.md` is
amended (~10 LOC) to add these three artefacts to the
first-week reading list. The amendment is identified as one of
the small modifications made by PR #26 and is recorded in the
PR's lessons file under "documentation changes."

---

## 10. Risks and mitigations

This section enumerates the specific risks the author has
identified during the design of PR #26, the likelihood and
impact assessment for each, and the mitigation in place or
planned. The format mirrors the risk register in the EFE
investor brief and the lessons-file residual-gaps sections.

### 10.1 Risk : the validator's heuristics produce false negatives

**Description.** The validator's six checks are tuned against the
canonical template and the reference example, but real-world
post-mortems may, for legitimate reasons, deviate from the
canonical shape in ways the validator rejects. A false negative
forces the post-mortem author to spend time on cosmetic
corrections rather than on the content, which is the opposite of
what we want.

**Likelihood : Medium.** The validator has never been exercised
against real-world post-mortems written under time pressure ;
the test fixtures are deliberately well-formed.

**Impact : Medium.** Each false negative costs an author 15-30
minutes of corrective work. Multiple false negatives in a
quarter erode the discipline.

**Detection.** The first 90 days after PR #26 ships are an
explicit calibration window. Every validator rejection that the
SRE lead considers a false negative is logged in a transient
file at `docs/process/postmortem-validator-calibration.md`, which
the team reviews monthly. After 90 days, the file is closed and
the validator's heuristics are tuned, if needed, in a follow-up
PR.

**Mitigation.** The validator's exit codes are precise enough
that a rejected file can be diagnosed in seconds, and the
reference example provides a worked example any author can copy.
The calibration window is the primary mitigation ; the secondary
mitigation is the bypass mechanism, which lets the team work
around a misbehaving validator without halting the
post-mortem-writing workflow.

### 10.2 Risk : the deadline window is wrong for some severity

**Description.** The base windows (7, 14, 30 days) are chosen by
intuition rather than by empirical calibration against the
team's actual post-mortem-writing pace. One or more of the
windows may be too tight (forcing rushed post-mortems) or too
loose (failing to surface deadline pressure when it would be
helpful).

**Likelihood : Medium.** No prior empirical data anchors the
window choices.

**Impact : Low to Medium.** A too-tight window produces extra
bypass requests ; a too-loose window means deadline violations
are not surfaced until they are stale.

**Detection.** The quarterly review tracks the deadline-violation
rate by severity. Three consecutive quarters with a deadline-
violation rate above 20 % for any severity triggers a window
adjustment.

**Mitigation.** The windows are easy to change (they are
parameters in the workflow YAML and in the STABILITY § 26.4 lock).
The change is itself a STABILITY-bump PR, which is the right level
of friction to ensure the change is deliberate.

### 10.3 Risk : the workflow opens duplicate tracking issues

**Description.** The deadline workflow's tracking-issue
behaviour is intended to be idempotent : the same incident
should never produce two tracking issues. If the idempotency
check has a bug, the workflow could open duplicates, which
would clutter the issue tracker and reduce the team's trust in
the workflow.

**Likelihood : Low.** The idempotency check is implemented as a
label-based query, which is well-understood GitHub API usage.

**Impact : Low.** Duplicates are easy to detect and easy to
close manually ; they do not cause permanent damage.

**Detection.** The aggregation job's output flags any incident
with two or more tracking issues. The flag triggers an
investigation at the next standup.

**Mitigation.** The idempotency check is exercised by the
rehearsal drill described in § 11. The first run of the drill
runs the deadline workflow twice in quick succession against the
same incident and verifies that the second run does not open a
second tracking issue.

### 10.4 Risk : the validator is fooled by hollow content

**Description.** The validator enforces structural presence but
not content quality. An unscrupulous author could fill the
canonical template with placeholder text that satisfies the
validator while conveying nothing about the actual incident.

**Likelihood : Low.** The team's working practice is to take
post-mortems seriously, and the social pressure inside a small
team makes hollow post-mortems socially expensive.

**Impact : High in the limit, Low in the near term.** A single
hollow post-mortem is a missed opportunity ; a pattern of hollow
post-mortems would erode the entire discipline.

**Detection.** The quarterly review explicitly reads a sample
of the post-mortems from the quarter and identifies any whose
content is hollow.

**Mitigation.** I.7 (the Phase-3 Q1-2027 work item) introduces
content-quality heuristics in a future PR. Until then, the
mitigation is the team's working practice plus the quarterly
review's sampling.

### 10.5 Risk : the GitHub API rate limits the workflow

**Description.** The deadline workflow queries the issue tracker
on every nightly run. If the team has a large number of incident
issues, the workflow's queries could approach GitHub's API rate
limits, causing the workflow to fail or behave incorrectly.

**Likelihood : Very Low.** The team's incident volume is on the
order of one incident per month ; the lookback window contains
at most a few tens of issues at any time.

**Impact : Low.** A workflow failure is itself a signal that
the team would notice and address.

**Mitigation.** The workflow's queries are designed to use
pagination correctly, so the worst-case number of API calls
scales with the number of incidents in the lookback window
rather than with the size of the issue tracker. The workflow
also uses the GitHub-supplied `GITHUB_TOKEN`, which has a
generous rate limit for repository-level operations.

### 10.6 Risk : the rehearsal drill exposes a bug

**Description.** The rehearsal drill is the first end-to-end
exercise of the workflow against a real (albeit simulated)
incident. The drill could surface a bug that was not caught by
the unit tests or by the workflow's self-test.

**Likelihood : Medium.** End-to-end exercises typically surface
issues that isolated tests do not.

**Impact : Low to Medium.** A bug surfaced by the drill is fixed
in a follow-up PR before the deadline workflow's first real run.

**Mitigation.** The rehearsal drill is run within seven days of
PR #26 merging, which gives the team a clear window to address
any issues. The drill's outcome is appended to the lessons file
as the first empirical validation, providing transparency about
what was tested and what was found.

### 10.7 Risk : a future GitHub change breaks the workflow

**Description.** The workflow depends on GitHub's API surface
(issues, labels, comments, the Actions runtime). A future
breaking change to any of these surfaces could break the
workflow.

**Likelihood : Low.** GitHub's API has historically been
relatively stable, and breaking changes are announced in
advance.

**Impact : Medium.** A broken workflow means deadline violations
are not surfaced.

**Detection.** The workflow's self-test runs on every PR ; a
broken workflow surfaces as a self-test failure.

**Mitigation.** The workflow is small and well-documented,
making it easy to adapt to API changes. The team also
subscribes to GitHub's API changelog and proactively reviews
upcoming changes for impact on the workflow.

### 10.8 Risk : the bypass mechanism is abused

**Description.** The bypass mechanism is designed for the three
legitimate classes ; a team that wanted to avoid writing
post-mortems could in principle abuse it.

**Likelihood : Low.** The bypass log is publicly visible, and
the quarterly review explicitly examines it.

**Impact : Medium.** Routine abuse would erode the entire
discipline.

**Detection.** The quarterly review's bypass-rate check (a rate
above two per quarter triggers an explicit conversation) is the
primary detection mechanism.

**Mitigation.** Beyond the quarterly review, the bypass log is
visible to everyone in the organisation, and abuse would be
visible to anyone reading the log. The structural transparency
is the deepest mitigation.

### 10.9 Risk : the workflow's auto-commit interferes with other PRs

**Description.** The aggregation job auto-commits an updated
`postmortem-status.md` file every night. The auto-commit could
in principle interfere with PRs that are also touching that file,
producing merge conflicts.

**Likelihood : Very Low.** The file is not modified by humans ;
only the workflow writes to it.

**Impact : Very Low.** A merge conflict would be trivially
resolved.

**Mitigation.** The workflow's commit is timestamped and includes
a deterministic commit message, making it easy to identify and
resolve any conflicts.

### 10.10 Risk : the architecture document itself becomes stale

**Description.** This document describes the design of PR #26 at
a point in time. The design may evolve during implementation, and
the document could become out of sync with the actual
implementation.

**Likelihood : Medium.** All design documents drift from
implementation over time.

**Impact : Low.** The implementation is the source of truth ;
the document is a reference.

**Mitigation.** The author commits to updating this document
during the implementation phase whenever a significant design
decision diverges from this text. The PR-26 lessons file will
explicitly call out any such divergences as part of its "what
worked" or "process changes" sections.

---

## 11. The rehearsal drill

The rehearsal drill is the manual end-to-end test that validates
PR #26's behaviour against a realistic incident scenario. The
drill is performed within seven calendar days of PR #26
merging, by the SRE lead and the release-engineering lead
together, with the engineering lead observing.

### 11.1 The scenario

The drill simulates the following scenario. At 14:00 UTC on the
drill day, a fictional but realistic incident is "detected" in
the staging environment : the bench portal's compare command
unexpectedly returns inconsistent verdicts for the same scenario
under repeated invocation. The drill's incident commander opens
an issue with the `incident` and `severity:S2` labels and the
title "[DRILL] Bench compare returns inconsistent verdicts." The
issue's first line follows the runbook : "Incident ID:
INC-2026-DRILL-001."

The commander spends approximately 30 minutes "investigating"
(actually performing scripted actions in the staging
environment), then declares the incident mitigated at 14:30 UTC
and resolved at 14:45 UTC. The issue is closed at 15:00 UTC.

### 11.2 The expected workflow behaviour

The drill's expected behaviour is :

- The night after the incident issue is closed (i.e. the next
  03:00 UTC), the deadline workflow's Job 2 runs and detects
  that the incident issue is closed but has no corresponding
  post-mortem file.
- Because the issue is labeled with the `drill` label (added by
  the commander at the moment the issue was opened), Job 2
  recognises the issue as drill-class and exempts it from the
  post-mortem requirement automatically, per § 5.4.1 Class 3.
- The aggregation job's output records the drill-class
  exemption in the `postmortem-status.md` file's bypass count.
- No tracking issue is opened.

### 11.3 The negative drill

The same day, a second drill is performed without the `drill`
label : the same scenario, the same incident commander, the same
timing, but without the label. This negative drill verifies
that the deadline workflow correctly identifies a non-drill
incident and that the tracking-issue mechanism fires.

The expected behaviour of the negative drill is :

- The night after the negative drill's issue is closed, Job 2
  runs and detects no corresponding post-mortem file.
- Because the issue is not labeled `drill`, the deadline is
  computed normally (Sev2 → 14 calendar days).
- Because the deadline has not yet expired (only one day has
  passed), no tracking issue is opened yet.
- The drill is then artificially advanced (the workflow is run
  manually with an injected current-date 15 days in the future)
  to verify that the tracking issue is opened on the 15th day.
- The tracking issue's content is then inspected for
  correctness : the title matches the expected format, the
  body contains the expected links, the labels are set
  correctly, the assignee is the incident commander.

### 11.4 The drill's outcome

The outcomes of both drills (positive and negative) are
captured in a small file at
`docs/lessons/drills/PR-26-rehearsal-drill.md`, which is itself
appended to PR #26's lessons file's "what worked" section. The
file's structure follows the existing drill-result convention
(YAML frontmatter with the drill date, the participants, and
the outcome ; followed by a markdown body describing the
specific observations). The file is approximately 80 LOC.

If any aspect of the drill behaves unexpectedly, the
discrepancy is captured in the file and a follow-up PR is
scheduled to address it. The drill's expected duration is
approximately three working hours including both the positive
and negative variants and the file-writing afterwards.

---

## 12. Acceptance criteria

PR #26 is considered acceptance-ready when each of the
following criteria is met. Each criterion is binary (pass /
fail). The PR cannot merge until all criteria pass.

1. The canonical template at `docs/postmortems/_template.md`
   carries the eleven canonical sections in the exact order
   specified by § 5.1.1, with the heading text matching the
   regex defined in § 5.2.4.
2. The canonical template carries the YAML frontmatter block
   defined in § 5.1.2 (with example placeholder values).
3. The reference example at
   `docs/postmortems/_VALIDATOR_REFERENCE.md` exists, is
   well-formed under the canonical schema, and is recognised by
   the validator as valid (exit code 0).
4. The validator script at `scripts/postmortem-validator.sh`
   exists, is executable, has its documentation block, and
   implements all six checks with their stable exit codes.
5. The Rust integration tests at
   `bench/harness/tests/postmortem_schema.rs` exist and pass.
   The file contains at least one positive test and one
   negative test for each of the validator's six exit codes.
6. The six new freeze tests in
   `bench/harness/tests/stability_contract_v17.rs` exist and
   pass, locking § 26.1 through § 26.6.
7. The workflow file
   `.github/workflows/pr26-postmortem-checks.yml` exists,
   contains the three jobs described in § 5.3.1, and the
   workflow's self-test job has been successfully run at least
   once.
8. The STABILITY contract at `docs/STABILITY_CONTRACT.md`
   contains a new § 26 with the six subsections specified in
   § 5.5.
9. The bypass-log file
   `docs/process/postmortem-bypass-log.md` exists with the
   schema described in § 5.4.2.
10. The incident-response runbook at
    `docs/runbooks/bench-publish-incident-response.md` is
    amended as described in § 9.1.
11. The onboarding contributor guide is amended as described in
    § 9.4.
12. PR #26's lessons file at `docs/lessons/PR-26-lessons.md`
    exists, follows the canonical seven-section schema from
    PR #25, and includes the rehearsal-drill outcome described
    in § 11.
13. The full audit-prompt validation passes against the lessons
    file (exit code 0 from
    `scripts/audit-prompt-validator.sh docs/lessons/PR-26-lessons.md`).
14. The full clippy run passes with zero warnings against the
    PR's modifications.
15. The full test suite passes with zero failures.
16. The audit budget block in the PR description is
    correctly-formed and the actuals are recorded against the
    plan in
    `docs/process/audit-budget-tracker.md`.

The criteria are enumerated here so that any reviewer can
verify them mechanically. The acceptance criteria are
deliberately bound to artefacts rather than to abstract
qualities ; each criterion can be checked by opening the
specified file or running the specified command.

---

## 13. Audit budget

In keeping with I.6 (the audit-budget invariant from PR #25),
PR #26 ships with the following explicit audit budget. The
budget will be updated with actuals as the PR progresses and
the final version will be recorded in the lessons file.

- **Implementation** : 5 calendar days.
- **Audit (per I.1)** : 2 calendar days.
- **Reserve** : 0.5 calendar day.
- **Total** : 7.5 calendar days.

The implementation estimate breaks down approximately as :
schema layer (template freeze, frontmatter, filename
convention, reference example) at 1 day ; validator layer
(script + test file) at 1.5 days ; enforcement layer (workflow
+ deadline algorithm + tracking issue logic) at 1.5 days ;
rehearsal drill + documentation + runbook updates at 1 day.
The 0.5 day reserve absorbs the small surprises that any
estimate of a discipline-class PR contains.

The audit estimate breaks down approximately as : 0.75 day for
the schema-and-validator audit pass (checking that the validator's
six checks correctly correspond to the schema, that the
exit-code contract is implementable, that the negative tests
exercise every code path) ; 0.75 day for the enforcement-and-
drill audit pass (checking the workflow, the deadline algorithm,
the rehearsal drill outcomes) ; 0.5 day for the documentation
audit pass (checking that the architecture document, the
STABILITY contract additions, the runbook amendments, and the
lessons file are all internally consistent and free of drift).

The 7.5-day total is comfortably within the one-week sprint
that the Phase-3 roadmap allocates to G.15. The 30-40 % audit-
to-implementation ratio (2 + 0.5 = 2.5 days of audit-and-
reserve against 5 days of implementation, or 50 %) is slightly
above the steady-state band predicted in the EFE investor
brief. The slight overshoot reflects the fact that this is the
*first* discipline-class PR after PR #25's calibration anchor,
and we expect the ratio to normalise toward 30-40 % as the
team accumulates more data.

---

## 14. Decision log

This section captures the design alternatives that were
considered and rejected during the design of PR #26, with the
specific reasoning for each rejection. The intent is that a
future reviewer can understand not only what we did but what we
chose not to do.

### 14.1 Considered : enforcing post-mortems via Git hooks rather than CI

The first alternative considered was to enforce the post-mortem
requirement via a pre-merge Git hook that runs locally on each
contributor's machine. The advantage would be faster feedback :
the contributor would learn about a missing post-mortem before
submitting the PR, rather than after CI runs.

The alternative was rejected for two reasons. First, Git hooks
are notoriously hard to enforce across a distributed team ;
each contributor must install the hook locally, and there is no
mechanism to ensure that they actually do. A discipline that
relies on each contributor's discretion to enforce is, in
practice, no discipline at all. Second, the post-mortem
deadline check requires querying the GitHub API to identify
closed incident issues. A local Git hook cannot easily perform
this query, and even if it could, the rate-limit implications
of every contributor's machine making the query would be
substantial. CI is the right home for the enforcement.

### 14.2 Considered : making the deadline algorithm uniform across severities

The second alternative considered was to use a single deadline
window for all severities (e.g. 14 days) rather than the
severity-dependent windows of 7, 14, and 30 days. The advantage
would be simplicity : one window is easier to remember and
implement than three.

The alternative was rejected because the operational reality of
incident response is heterogeneous. A Sev1 incident's lessons
are most expensive to lose, because the next Sev1 incident may
arrive before the post-mortem is written. A Sev3 incident's
lessons are less time-sensitive, and the post-mortem author may
need to coordinate with adjacent teams whose calendars are
slower. The severity-dependent windows reflect this operational
reality, at the cost of a slightly more complex algorithm.

### 14.3 Considered : automated post-mortem generation from incident logs

The third alternative considered was to automatically generate
the post-mortem's first draft from the incident's GitHub issue
contents : the timeline could be extracted from the issue's
comments, the participants from the comments' authors, the
duration from the labels' timestamps. The validator could then
accept a partial post-mortem on the theory that automated
generation lowers the friction.

The alternative was rejected because automated generation
optimises for the wrong thing. The value of writing a
post-mortem is not in the artefact produced but in the act of
writing it : the deliberate, considered work of constructing a
durable explanation of what happened. Automated generation
short-circuits the act, and any time saved by the automation is
saved at the cost of the lesson the act would have produced.
This is a clear example of a place where the friction is the
feature.

### 14.4 Considered : applying the post-mortem requirement to all closed issues

The fourth alternative considered was to apply the requirement
to all closed GitHub issues, not just those labeled `incident`.
The advantage would be that the team would not need to
remember to label issues correctly ; everything that closes
would receive a deadline.

The alternative was rejected because most GitHub issues are
not incidents and do not warrant post-mortems. A feature
request, a bug report, a discussion thread, a documentation
fix — none of these need a post-mortem, and requiring one for
each would dilute the discipline. The `incident` label is the
right granularity, and labeling issues correctly is itself a
practice we want to reinforce.

### 14.5 Considered : a single STABILITY contract section for both PR #25 and PR #26 invariants

The fifth alternative considered was to merge § 25 (the PR #25
process invariants) and § 26 (the PR #26 post-mortem schema)
into a single contract section, on the theory that both
sections cover process discipline and might as well be
co-located.

The alternative was rejected because the two sections have
different audiences and different freeze horizons. § 25 governs
release-time discipline and is most relevant to engineers
preparing releases. § 26 governs incident-time discipline and
is most relevant to the SRE team. Co-locating the two sections
would make each one harder to find for its primary audience.
The separation is the right organisational choice even if it
introduces a small amount of duplication in the contract's
structure.

### 14.6 Considered : a Python validator rather than a bash validator

The sixth alternative considered was to write the validator in
Python rather than in bash. Python would offer richer YAML
parsing (the `pyyaml` library handles edge cases that bash
cannot), better error messages, and easier unit testing within
the validator itself rather than via the external Rust test
file.

The alternative was rejected for parallelism with the existing
audit-prompt validator. The audit-prompt validator is written
in bash for reasons that were good at the time it was
written : bash is available on every supported platform without
installation, the validator's dependency surface is the unix
toolchain rather than a language runtime, and the script is
small enough to read in a single sitting. The post-mortem
validator inherits these reasons. A future PR may rewrite both
validators in Python or in Rust if the heuristics grow complex
enough to warrant the change, but for PR #26 the parallelism
with the existing validator is more valuable than the marginal
ergonomic benefit of Python.

### 14.7 Considered : making the workflow run on every commit rather than on PRs and nightly

The seventh alternative considered was to run the workflow on
every commit to any branch, rather than only on PRs and on the
nightly schedule. The advantage would be earlier detection :
a deadline violation would be detected within minutes of the
violation occurring, rather than potentially up to 24 hours
later.

The alternative was rejected because the deadline-mode
behaviour is fundamentally a time-sensitive check, not a
content-sensitive one. The check evaluates the current date
against a deadline ; the result does not change between
commits made in quick succession. Running the check on every
commit would consume CI resources without producing additional
signal. The PR-trigger covers the validation of content
changes ; the nightly trigger covers the deadline check.
Together, they cover the space.

---

## 15. Future-PR implications

### 15.1 Implications for PR #27 (I.7)

PR #27, which lands in Q1-2027, will convert the tracking-issue
mechanism into a hard CI gate. The conversion is straightforward
because PR #26 has already established the validator, the
schema, the deadline algorithm, and the workflow. PR #27 adds
one additional job to the workflow : a *blocking* job that runs
on every PR touching the components affected by an overdue
post-mortem, and that fails the PR's CI if the post-mortem has
not landed.

PR #27 is approximately a half-day of implementation work given
the foundation PR #26 establishes. The bulk of PR #27's effort
will be in the calibration : determining which components a
given post-mortem corresponds to, and verifying that the
blocking behaviour fires correctly without producing
false positives.

### 15.2 Implications for later quarters

The post-mortem discipline established by PR #26 and PR #27
becomes the foundation for several Phase-4 work items :

- **J.6** : public post-mortem digest. Once the team has six
  months of validated post-mortems, the quarterly evidence
  chain can include a public-facing digest of post-mortems
  (suitably anonymised). The digest would be a powerful
  trust signal for customers evaluating the platform.
- **J.7** : action-item completion tracking. Once the team has
  a quarter of post-mortems, an additional CI job can track
  whether the action items in each post-mortem are actually
  completed within their due dates. The job would open
  reminder issues for overdue action items.
- **J.8** : post-mortem-to-release correlation. Combining the
  audit-budget tracker from PR #25 with the post-mortem
  cadence from PR #26 produces a data set that can
  quantitatively answer the question : which kinds of release
  produce which kinds of incident. The correlation would be
  a powerful input to the next phase of the audit-budget
  calibration.

These work items are not in scope for PR #26, but they are
the direct consequences of PR #26's design, and they are worth
mentioning here so that a reader understands the trajectory.

---

## 16. References

- STABILITY contract section being added : `docs/STABILITY_CONTRACT.md`
  § 26 (created by this PR).
- Sibling contract section : `docs/STABILITY_CONTRACT.md` § 25
  (PR #25, process invariants I.1 + I.2 + I.6).
- Sibling validator (reference implementation) :
  `scripts/audit-prompt-validator.sh` (PR #25).
- Sibling workflow (reference implementation) :
  `.github/workflows/pr25-process-checks.yml` (PR #25).
- Sibling lessons file (canonical schema reference) :
  `docs/lessons/PR-25-lessons.md` (PR #25).
- Existing post-mortem template (to be frozen) :
  `docs/postmortems/_template.md`.
- Master incident-response runbook :
  `docs/runbooks/bench-publish-incident-response.md`.
- Phase-3 roadmap entry : G.15 in
  `docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`.
- I.7 forward reference : Q1-2027 in
  `docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md`.
- Audit-budget tracker (will be updated with PR #26 actuals) :
  `docs/process/audit-budget-tracker.md`.

---

## 17. Open questions

The following questions remain open at the time of this
document's writing. Each will be resolved during the
implementation phase, and the resolution will be recorded in
PR #26's lessons file.

1. Should the workflow's secondary tracking issue (the 30-day
   escalation) be assigned to the engineering lead specifically,
   or to a rotating member of the engineering leadership team?
   The current text assumes the engineering lead ; the
   alternative would be to distribute the responsibility.
2. Should the holiday list at
   `docs/process/efe-business-calendar.md` cover only Swedish
   holidays initially, or also the holidays of other
   geographies where the team operates? Currently the text
   assumes Swedish only.
3. Should the validator's success summary include a hash of the
   validated file's content, so that the workflow can detect
   silent edits to a post-mortem after it has been validated?
   The current text does not include this, on the theory that
   silent edits are uncommon and the audit trail of Git itself
   captures any change. But the hash would be cheap to add.
4. Should the bypass-log file be auto-validated by its own
   validator, ensuring it follows its schema? The current text
   says no (the bypass log is reviewed manually) ; the
   alternative would be a small additional validator.
5. Should the rehearsal drill's negative variant be run from a
   PR-triggered manual workflow rather than from a real
   incident issue? The current text uses a real-issue-like
   approach ; the alternative would be a more synthetic
   exercise.

These questions will be answered during implementation. The
answers may shift the design slightly. A reader of this
document who has a strong opinion on any of these questions is
invited to share it with the engineering leadership team
before implementation begins.

---

## 18. Acknowledgements

The design of PR #26 builds directly on the work of PR #25
and on the post-mortem template that the SRE team wrote in
late 2025. The author thanks the original template authors
for their thoughtful work and apologises in advance for any
adjustments to the template that the freezing under § 26.1
will require.

The author also thanks the engineering leadership team for
their feedback during the design conversations that produced
this document. Specific reviewers and their contributions
will be recorded in the lessons file at PR-26-merge time.

---

_Document author : release-eng + SRE leadership._
_Last revised : 2026-05-17._
_Status : DRAFT, pending Phase 3 kickoff review on 2026-09-15._
_Next revision will land alongside PR #26 itself, with the
final implementation details and the rehearsal-drill outcome
incorporated._
