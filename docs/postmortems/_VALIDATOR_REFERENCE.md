<!--
  Reference post-mortem — fictional but fully-formed.

  Purpose :
    * Validator positive-path fixture.  Exercised by
      bench/harness/tests/postmortem_schema.rs::validator_accepts_canonical_reference.
    * Documentation example for new authors.  Copy this file into
      docs/postmortems/INC-YYYY-NNN.md and replace the content with
      your incident.
    * Workflow self-test target.  The PR-trigger of
      .github/workflows/pr26-postmortem-checks.yml invokes the
      validator against this file once to confirm the validator is
      working end-to-end before validating any real post-mortems.

  The incident described below is FICTIONAL.  No real customer was
  affected ; no real production data was lost.  The narrative is
  realistic so that the example is useful, not so that it deceives
  any reader into thinking the incident actually happened.

  The incident ID `INC-2026-000` is reserved by convention for the
  reference example and is never assigned to a real incident.

  Frozen under STABILITY § 26.1 + § 26.2 (frontmatter + section set).
  This file is skipped by the deadline workflow because its filename
  starts with `_`.
-->

---
incident_id: INC-2026-000
severity: S2
detected_at: 2026-03-14T09:07:00Z
mitigated_at: 2026-03-14T09:31:00Z
resolved_at: 2026-03-14T10:18:00Z
author: release-eng-bot
reviewers:
  - sre-lead
  - bench-infra-lead
github_issue: https://github.com/efe-ai/efe/issues/0
status: PUBLISHED
---

# Postmortem — Quarterly publish workflow OIDC token expiry

> **Status** : PUBLISHED
> **Severity** : Sev2
> **Detected at (UTC)** : 2026-03-14 09:07
> **Mitigated at (UTC)** : 2026-03-14 09:31
> **Resolved at (UTC)** : 2026-03-14 10:18
> **Author** : release-eng-bot
> **Reviewers** : sre-lead, bench-infra-lead
> **PagerDuty incident** : (none — automated workflow failure)
> **Linear epic** : EFE-DRILL-000

> **This is the reference example.** It is a fictional but
> realistic post-mortem written under the canonical schema. Real
> post-mortems should be copied from this file and rewritten in
> place. The incident below did not actually happen ; nothing in
> the public bench portal or in the customer-facing chain was
> ever broken in the way described.

---

## 1. Summary (= 3-4 sentences)

The quarterly publish workflow, which produces the signed quarterly
evidence PDF for the public bench portal, failed mid-pipeline on
the morning of 14 March 2026. The failure was caused by the OIDC
token that the workflow uses to sign the PDF expiring before the
signature step could complete, which produced a hard error in the
cosign step that aborted the pipeline before any artefact was
published. No customer-facing publication occurred ; no
publicly-visible artefact was corrupted. The team detected the
failure within nine minutes via the workflow-failure pager,
mitigated by manual retry within twenty-four minutes, and
implemented a permanent fix the same afternoon by adjusting the
token refresh policy.

---

## 2. Timeline (= UTC timestamps)

| Time | Event |
|------|-------|
| `09:00` | Quarterly publish workflow triggered automatically by cron. |
| `09:07` | Pager fires on workflow failure ; the failing step is identified as the cosign signing step. |
| `09:09` | On-call engineer acknowledges the page ; opens the workflow run log. |
| `09:14` | Root cause hypothesis : OIDC token used by the cosign step has expired before the step started, because an earlier step in the workflow ran longer than expected. |
| `09:22` | Hypothesis confirmed via the token's `exp` claim in the workflow's structured log. |
| `09:25` | Mitigation : on-call engineer manually re-runs the workflow with no changes. The fresh token completes the signing step successfully. |
| `09:31` | Mitigation confirmed — the workflow's manual rerun produces a valid quarterly artefact and publishes it correctly. |
| `09:45` | Investigation of the longer-than-expected earlier step begins. |
| `10:11` | Investigation identifies the slow step : the SBOM-generation step grew slower because of a transitive dependency that doubled in size since the last quarterly run. |
| `10:18` | All-clear confirmed and broadcast to the team. |
| `14:30` | Permanent fix shipped : the cosign step now requests a freshly-issued token immediately before signing, rather than relying on the token issued at workflow start. |

---

## 3. Root cause

The OIDC token issued to the quarterly publish workflow at workflow
start has a fixed 15-minute lifetime. The workflow's preceding
steps (SBOM generation, dependency snapshotting, schema validation,
PDF rendering) had, over the preceding two quarters, accumulated
enough additional work that the cumulative pre-signing duration
crossed the 15-minute boundary for the first time, causing the
token to be expired by the time the cosign step ran. The cosign
step then failed with the GitHub Actions OIDC error message
`token has expired` and exited non-zero, aborting the publish.

There was no mistake in the cosign step or in the signing logic
itself ; the cosign step worked exactly as designed. The error was
in the workflow's implicit assumption that the OIDC token issued
at workflow start would still be valid when the signing step
reached it. This assumption had been true through the workflow's
entire history until the duration of the preceding steps crossed
the token's lifetime.

---

## 4. Contributing factors

- The OIDC token's 15-minute lifetime is set by GitHub Actions and
  is not configurable per-workflow ; the workflow had no early-warning
  signal that the cumulative pre-signing duration was approaching
  the boundary.
- The SBOM-generation step's runtime is sensitive to the size of
  the dependency tree, which is itself sensitive to changes in
  third-party crates over which the team has no direct control.
- The quarterly publish runs only four times a year, so the
  duration-creep was not visible on a tighter cadence ; the
  per-run duration had been creeping upward over multiple
  quarters but the team did not have a monitor on it.
- The cosign error message was technically correct but did not
  immediately suggest the root cause ; the on-call engineer
  initially suspected a network issue with the Sigstore endpoint
  before reading the token's `exp` claim.

---

## 5. What went well

- The workflow-failure pager fired within seven minutes of the
  scheduled start time, which is well within the team's target
  detection-time SLO for automated workflow failures.
- The on-call engineer's acknowledgement and initial investigation
  were both completed within nine minutes of the page, also well
  within target.
- The manual retry path worked exactly as the runbook described,
  with no surprises and no need for unusual privileges.
- The customer-facing public bench portal showed no broken state
  at any point ; the failure was contained entirely within the CI
  workflow.
- The permanent fix was implementable the same afternoon and did
  not require a STABILITY-bump (it adjusted the workflow's
  internal step order, not any frozen surface).

---

## 6. What went wrong

- The cosign error message did not immediately surface the OIDC
  token's `exp` claim, which would have made the root cause
  identifiable in seconds rather than minutes.
- The team had no monitor on the quarterly workflow's per-step
  duration, so the drift toward the 15-minute boundary was
  invisible until it caused a failure.
- The first attempted mitigation (the manual rerun) succeeded only
  because the fresh token had time to complete the signing step ;
  if the SBOM step had been even slower on the rerun, the failure
  would have recurred.
- The runbook section on quarterly publish failures was last
  updated in late 2025 and did not mention OIDC-token-expiry as a
  possible failure mode.

---

## 7. Action items (= Linear-tracked)

| Action | Owner | Linear | Due | Status |
|--------|-------|--------|-----|--------|
| Move the cosign step's token request to immediately before signing | release-eng | EFE-2026-114 | 2026-03-14 | DONE |
| Add a per-step duration monitor to the quarterly publish workflow with an alert when any step approaches the OIDC token lifetime | bench-infra | EFE-2026-115 | 2026-03-28 | DONE |
| Update the runbook section "Quarterly publish failures" to enumerate OIDC-token-expiry as a known failure class with the diagnostic command for reading the token's `exp` claim | release-eng | EFE-2026-116 | 2026-03-21 | DONE |
| Add an automated CI gate that fails the workflow if any pre-signing step exceeds 10 minutes (early-warning for the 15-minute token boundary) | release-eng | EFE-2026-117 | 2026-04-04 | DONE |
| Open an upstream conversation with GitHub Actions about extending the OIDC token lifetime for long-running workflows, or about providing a token-refresh primitive | engineering-lead | EFE-2026-118 | 2026-04-30 | IN-PROGRESS |

---

## 8. Prevention (= what we change permanently)

The four shipped action items above are the prevention. The
operational change is that the cosign step now requests a freshly-
issued OIDC token immediately before signing, rather than relying
on the token issued at workflow start. The change is small (about
six lines of YAML) but it eliminates the entire class of failure
mode this incident exemplified, because the token can no longer be
expired by the time the signing step reaches it.

The supporting changes are : the per-step duration monitor that
gives the team early warning if the pre-signing duration starts
approaching the new (tighter) 10-minute internal budget ; the
updated runbook that surfaces OIDC-token-expiry as a known failure
mode for any future on-call engineer ; and the upstream
conversation with GitHub Actions that explores whether the
underlying 15-minute lifetime can be relaxed for workflows of this
shape. The combination of the four changes converts a previously
invisible drift into a tracked operational parameter.

**Decision criteria for future review** : the prevention is
considered effective if no further OIDC-token-expiry failures
occur over the next eight quarterly publish runs (i.e. through
Q1-2028). If a failure recurs despite the four changes, the team
revisits whether the workflow itself needs to be restructured into
two separate workflows (one for SBOM generation, one for signing)
that each fit comfortably within the token lifetime.

---

## 9. Customer impact

| Tenant | Surface | Duration | Customers impacted | Communication sent |
|--------|---------|----------|--------------------|--------------------|
| (none) | quarterly publish workflow | 24 min (failed→retried) | 0 | none |

No customer-facing impact. The quarterly evidence PDF was
published on its normal schedule once the manual rerun completed.
External auditors who consume the quarterly artefacts would have
seen no discrepancy.

---

## 10. Compliance ramifications

- [x] DPO notified (≤ 24 h) — N/A : no personal data involved.
- [x] Breach assessment under GDPR Art. 33 — N/A : no personal data involved.
- [x] AI Act Art. 9 risk register updated — N/A : no AI Act high-risk use case touched.
- [x] Supervisory authority notification required ? (yes / no) — N/A : no supervisory authority's scope was touched.
- [x] Customer-facing trust center entry updated — N/A : no customer-visible event to disclose.

The incident was fully contained within the team's internal CI
infrastructure and did not touch any regulated control surface.

---

## 11. Cross-references

- Runbook(s) consulted :
  - `docs/runbooks/bench-publish-incident-response.md` § "Quarterly publish failures"
- Dashboards used :
  - GitHub Actions workflow-run log for the failed run
  - Sigstore Rekor inclusion-proof inspector for the eventual successful run
- ADRs proposed : none (the fix is an in-workflow restructuring, not an architectural change).
- Releases involved :
  - The cosign-step adjustment shipped as part of the next minor release of the bench-publish workflow.
  - The duration monitor shipped as part of the same release.

---

_This is the reference example for the canonical post-mortem
schema. Authors writing their first real post-mortem should copy
this file into `docs/postmortems/INC-YYYY-NNN.md` and replace the
content with their incident, preserving the section structure and
the YAML frontmatter._

_Frozen under STABILITY § 26.1 + § 26.2._
