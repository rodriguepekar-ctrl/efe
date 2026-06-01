# Runbook : bench-publish incident response

**Owner** : `@efe-ai/release-eng`
**Pager rotation** : PagerDuty service `efe-bench-publish-prod`
**Last reviewed** : 2026-05-16 (post PR #24 10-pass audit)
**Next review** : 2026-08-15 (quarterly cadence)

This runbook is the **master entry point** for any incident affecting
the public bench-publish pipeline (PR #24, STABILITY_CONTRACT v1.6
§ 24).  Sub-runbooks for two specific incident classes are :

- [`bench-publish-defacement.md`](bench-publish-defacement.md) — T116 / T122 (branch protection)
- [`bench-publish-quarterly.md`](bench-publish-quarterly.md) — T121 (evidence revocation / errata)

When in doubt, follow this master runbook first — it routes to the
specific sub-runbook in Phase 1.

---

## 0. TL;DR for the on-call

```
1. Confirm the alert is real    →   Phase 1
2. Classify severity (P0..P3)   →   Phase 2
3. Stabilise / contain          →   Phase 3
4. Mitigate / roll forward      →   Phase 4
5. Communicate stakeholders     →   Phase 5
6. Post-mortem within 14 days   →   Phase 6
```

Critical invariants you MUST preserve at all times :

- **Past gh-pages history is NEVER deleted** (AD30 — auditors hold pins).
- **Past quarterly evidence is NEVER deleted** — only superseded via ERRATA.
- **Branch protection on gh-pages is NEVER permanently disabled** —
  only a time-boxed bypass with `bench-publish-defacement.md` Phase 2.
- **You do not `--no-verify` a pre-commit hook, do not `--cosign-disabled`
  in production, do not skip the ratchet check.**  If a contract test
  fails you fix the offending change, not the test.

---

## 1. Detection and confirmation

### 1.1 How an incident surfaces

| Channel | What you see | Likely class |
|---|---|---|
| PagerDuty alert `bench-publish-failed` | CI workflow non-zero | Build / deploy |
| External report (Twitter, email) | "Your bench page shows X" | Defacement, SHA mismatch |
| Auditor email | "Q2-2026 PDF won't verify" | Evidence revocation |
| GitHub status page | sigstore / Rekor outage | Cosign signing blocked |
| Internal Slack `#release-eng` | "Hourly cron stuck" | Concurrency lock |
| Customer-success ticket | "Badge URL 404s" | Page slug drift (D-U) |

### 1.2 Confirm before acting

Before declaring an incident, run these three checks (≤ 3 minutes) :

```bash
# 1. Is the live site reachable + ratchet-passing?
curl -fsSL https://efe.github.io/bench/api/version.json | jq

# 2. Do the 11 badges all exist?
for f in scenario-S0{1..9}.svg scenario-S10.svg overall.svg; do
    curl -fsSI "https://efe.github.io/bench/badges/${f}" | head -1
done

# 3. Can `evidence verify` succeed on the latest quarterly PDF?
LATEST_Q=$(curl -fsSL https://efe.github.io/bench/api/version.json \
    | jq -r '.chart_version' | xargs -I{} echo "Q$( (($(date -u +%-m)-1)/3 + 1 == 1 ? 4 : ($(date -u +%-m)-1)/3))-$(date -u +%Y)")
./target/release/efe-bench-harness evidence verify \
    "bench/v$(jq -r '.chart_version' /tmp/version.json)/quarterly/${LATEST_Q}.pdf"
```

If all three succeed → **stand down, no incident.**

---

## 2. Severity classification

| Severity | Definition | Time-to-mitigate target | Examples |
|---|---|---|---|
| **P0** | Public-bench unreachable OR signed evidence verifiably tampered | 30 min | gh-pages branch wiped, cosign sig forged-looking |
| **P1** | Bench data wrong but page reachable | 4 h | overall.svg shows green when latest run was fail |
| **P2** | Single scenario page 404 / stale | 24 h | 6 of 10 scenario URLs broken (the D-U regression) |
| **P3** | Cosmetic / non-functional drift | 7 d | smoke badge missing, CSP warning in console |

> Honest disclosure : **D-U** (6 of 10 scenario page URLs 404'd) shipped to
> production briefly between v0.5.x and the audit catch in pass 5.  Treat
> any "click-through scenario page 404" report as a P2 until proven otherwise.

---

## 3. Stabilisation (always run first)

### 3.1 Lock the deploy pipeline

```bash
# Disable the hourly cron + push trigger so the broken state isn't
# re-deployed while you investigate.  Re-enable in Phase 4.
gh workflow disable bench-publish --repo efe-ai/efe
gh workflow disable bench-publish-quarterly --repo efe-ai/efe
```

### 3.2 Snapshot the broken state

```bash
# Pull the current gh-pages content into an evidence dir so the
# post-mortem can reference what shipped.  NEVER overwrite this snapshot.
mkdir -p incident-evidence/$(date -u +%Y%m%d-%H%M)
git clone --branch gh-pages \
    --depth 1 \
    https://github.com/efe-ai/efe.git \
    incident-evidence/$(date -u +%Y%m%d-%H%M)/gh-pages-snapshot
```

### 3.3 Snapshot the running workflows

```bash
gh run list --workflow=bench-publish --limit 20 \
    > incident-evidence/$(date -u +%Y%m%d-%H%M)/recent-runs.txt
gh run list --workflow=bench-publish-quarterly --limit 5 \
    >> incident-evidence/$(date -u +%Y%m%d-%H%M)/recent-runs.txt
```

---

## 4. Mitigation — route to the right sub-runbook

| Incident class | Sub-runbook | Phase to enter |
|---|---|---|
| gh-pages defaced | [`bench-publish-defacement.md`](bench-publish-defacement.md) | Phase 2 |
| Past evidence needs revocation | [`bench-publish-quarterly.md`](bench-publish-quarterly.md) | Phase 1 (errata) or Phase 2 (retraction) |
| Embedded SHA mismatch on a fresh PDF | § 4.1 below |
| Cosign keyless OIDC unavailable | § 4.2 below |
| Rekor transparency log unreachable | § 4.3 below |
| Deploy stuck (concurrency lock) | § 4.4 below |
| 6-of-10 page slug 404 (D-U regression) | § 4.5 below |
| 11-badge count drift | § 4.6 below |

### 4.1 Embedded SHA mismatch on a fresh PDF

**Symptom** : `evidence verify` reports
`embedded SHA mismatch : pdf=X, json=Y`.

```bash
# Step 1 : compute the JSON SHA yourself.
sha256sum bench/vX.Y.Z/quarterly/Q2-2026.json

# Step 2 : extract the PDF marker by hand.
strings bench/vX.Y.Z/quarterly/Q2-2026.pdf \
    | grep -E '(EFE_BENCH_EVIDENCE_JSON_SHA256|EFE_EVIDENCE_JSON_SHA256)'

# Step 3 : if PDF SHA != JSON SHA, the JSON was modified AFTER signing.
#         → tamper or accidental edit.  Treat as P0.
#         → DO NOT re-sign in place.  Issue an ERRATUM
#           (see bench-publish-quarterly.md Phase 1).
```

### 4.2 Cosign keyless OIDC unavailable

**Symptom** : `bench-publish-quarterly.yml` workflow fails at
`cosign sign-blob` step with
`failed to get fulcio cert : ... 502 / 503 / connect refused`.

This is a sigstore.dev outage (track on https://status.sigstore.dev).

**Action** :
1. Do NOT re-run with `--cosign-disabled` in CI (would publish an
   unsigned evidence PDF — silent integrity regression).
2. Pause the workflow.  Wait for sigstore recovery (typical : < 1 h).
3. Re-trigger : `gh workflow run bench-publish-quarterly --field period=Q2-2026`.
4. If outage > 6 h, escalate to legal/compliance — the quarterly
   evidence may need to be issued late with a documented incident note.

### 4.3 Rekor transparency log unreachable

**Symptom** : `cosign sign-blob` succeeds, but `evidence verify` later
fails with `Rekor inclusion proof : connection refused`.

The signed bundle is still valid offline (it contains the inclusion
proof).  Issue : new verifications can't re-fetch the log entry.

**Action** : verify offline with `cosign verify-blob --insecure-ignore-tlog`
(only as a one-off operator override — NOT in CI).  Wait for Rekor
recovery, then re-verify.  Document the outage in the next quarterly
report's "Operational notes" section.

### 4.4 Deploy stuck (concurrency lock)

**Symptom** : `gh run list --workflow=bench-publish` shows runs in
`queued` state piling up.

**Cause** : `concurrency.group: bench-publish` + `cancel-in-progress: false`
(AD31 / T122) means a hung run blocks all followers.

```bash
# Step 1 : identify the hung run.
gh run list --workflow=bench-publish --limit 5

# Step 2 : if the head run is genuinely stuck, cancel it explicitly.
gh run cancel <run-id>

# Step 3 : the next queued run takes over.  Verify it completes :
gh run watch <next-run-id>
```

> NEVER set `cancel-in-progress: true` to "fix" this — it would
> re-introduce the T122 race that AD31 fixed.

### 4.5 Six-of-ten page slug 404 (D-U regression)

**Symptom** : every link from `index.html` to `scenarios/s04-chaos-participant-loss.html`
(etc.) returns 404.  Auditor or customer click-through fails.

**Root cause shape** : the aggregator's `scenarios.json::page` field
points to a slug that doesn't match the on-disk filename.  Historical
defect : the slug function was `parts[0]+"-"+parts[1]`, producing
`s04-chaos` instead of `s04-chaos-participant-loss`.

```bash
# Step 1 : confirm the drift.
curl -fsSL https://efe.github.io/bench/api/scenarios.json \
    | jq -r '.scenarios | to_entries[] | "\(.key): \(.value.page)"'
# Compare against the actual files :
for f in scenarios/s01-*.html scenarios/s10-*.html ; do
    curl -fsSI "https://efe.github.io/bench/${f}" | head -1
done

# Step 2 : the fix is to re-run publish from a harness ≥ v0.2.0
#         (D-U fix in pass 5 of audit).  Verify the version.json :
curl -fsSL https://efe.github.io/bench/api/version.json | jq .harness_version

# Step 3 : if harness_version < 0.2.0 (impossible post-audit, but
#         documented), force a re-publish.
gh workflow run bench-publish
```

### 4.6 11-badge count drift

**Symptom** : ratchet step fails with
`FAIL — expected 11 badges, got N`.

**Cause** : `BADGE_FILENAMES` constant drifted in code.  Per
STABILITY_CONTRACT v1.6 § 24.1, this is a contract violation — fix
the offending PR, not the ratchet.

---

## 5. Communication

### 5.1 Stakeholder matrix

| Audience | Channel | When |
|---|---|---|
| Internal release-eng | Slack `#release-eng` | T+0 (alert acknowledged) |
| Internal exec | Email `eng-leadership@` | T+1 h if P0 / P1 |
| Customer success | Slack `#customer-success` | T+2 h if any auditor will see it |
| External auditors | Email + signed retraction | Per quarterly runbook Phase 3 |
| Legal counsel | PagerDuty escalation | If retraction issued |
| Compliance officer | Email | Within 7 d of resolution |

### 5.2 Templates

#### Slack `#release-eng` initial acknowledgement (≤ 5 minutes after page)

```
:warning: bench-publish incident detected
- Time   : <UTC timestamp>
- Source : <PagerDuty / external report / monitoring>
- Surface: <what the user sees>
- Sev    : P? (under investigation)
- On-call: <you>
- Runbook: docs/runbooks/bench-publish-incident-response.md
- War room : <link to incident channel>
```

#### Customer-success update (T+2 h if relevant)

```
Hi customer-success,

We detected a defect in the public bench page at <UTC time>.
Affected surface : <single sentence>.
Customer impact  : <single sentence — be honest>.
Mitigation       : <what's underway / done>.
Auditor action   : <none / re-verify Q2-2026.pdf when notified>.

Next update at T+4 h or on resolution, whichever sooner.
— Release-Eng
```

---

## 6. Post-mortem — the closing step

**As of PR #26 (G.15), the post-mortem is the formally enforced
closing step of every incident.**  The previous paragraph-level
description has been superseded by the canonical schema below.

### 6.1 The deadline

The deadline window depends on severity (frozen under STABILITY
§ 26.4) :

| Severity | Window | Counted from |
|----------|--------|--------------|
| S1 | 7 calendar days  | incident issue's closure |
| S2 | 14 calendar days | incident issue's closure |
| S3 | 30 calendar days | incident issue's closure |

Deadlines falling on weekends or recognised public holidays
(`docs/process/efe-business-calendar.md`) shift to the next business
day at 23:59:59 UTC.

### 6.2 The filename

Create the file at `docs/postmortems/INC-YYYY-NNN.md`, where
`INC-YYYY-NNN` is the incident identifier you assigned at incident
open time (the first line of the incident issue's body must read
`Incident ID: INC-YYYY-NNN`).

For multi-incident post-mortems covering N closely-related
incidents, use `docs/postmortems/INC-YYYY-NNN+MMM.md` with the
identifiers `+`-separated (see STABILITY § 26.5.2).

### 6.3 The canonical schema

The post-mortem MUST follow the eleven-section canonical template
at `docs/postmortems/_template.md`. Each section's heading text,
the section order, and the YAML frontmatter contract are all frozen
under STABILITY § 26.1 and § 26.2.

A fully-worked example is at
`docs/postmortems/_VALIDATOR_REFERENCE.md`. Copy this file and
replace the content with your incident ; the resulting file will
pass the validator on the first attempt.

The eleven sections, in canonical order, are :

1. Summary (3-4 sentences)
2. Timeline (UTC timestamps, ≥ 2 rows)
3. Root cause (one paragraph, direct and unambiguous)
4. Contributing factors
5. What went well
6. What went wrong
7. Action items (≥ 1 row, each with owner + Linear + due date)
8. Prevention (the permanent changes)
9. Customer impact
10. Compliance ramifications (every checkbox ticked OR annotated N/A)
11. Cross-references

### 6.4 The validator

Before opening the PR with your post-mortem, run :

```
bash scripts/postmortem-validator.sh docs/postmortems/INC-YYYY-NNN.md
```

A clean exit (code 0) means the post-mortem will pass CI. The six
non-zero exit codes (2, 3, 4, 5, 6, 7) each correspond to a specific
class of structural problem ; see STABILITY § 26.3 for the
contract.

### 6.5 The deadline workflow

A nightly job in `.github/workflows/pr26-postmortem-checks.yml`
checks every closed incident issue against its deadline. If a
deadline expires without a corresponding post-mortem file, the
workflow opens a tracking issue assigned to the incident commander.

The tracking issue closes automatically once the post-mortem file
lands on `main`. If the tracking issue remains open for more than
thirty days, a secondary escalation issue is opened, assigned to
the engineering lead.

### 6.6 The bypass mechanism

Three bypass classes are recognised :

- **Class 1 : MISLABEL** — the issue was not actually an operational
  incident.
- **Class 2 : MERGED** — the issue is a duplicate of another with
  an in-progress post-mortem.
- **Class 3 : DRILL** — the issue is a scripted drill (label
  `drill` on the issue).

Class 1 and Class 2 require an entry in
`docs/process/postmortem-bypass-log.md` and a `[post-mortem-bypass]`
comment closing the tracking issue. Class 3 is automatic.

**Hard rule** : the post-mortem is blameless. Names appear only in
the on-call timeline ; root-cause discussion uses "the system did X"
not "Alice did X". The validator does NOT enforce this rule — it is
a quarterly-review judgement — but the canonical template's
introduction reminds every author of it.

---

## 7. References

- PR #24 § 17 — workflow architecture
- PR #24 § 20 (T100, T116, T118, T121, T122, T125) — threats addressed
- PR #24 AD30, AD31, AD32 — immutability, sequential publish, ratchet
- STABILITY_CONTRACT v1.6 § 24 — frozen surface
- PR #24 audit lessons : [`docs/lessons/pr24-lessons-learned.md`](../lessons/pr24-lessons-learned.md)
- 10-pass audit defects index : same file § "Defect taxonomy"
- Drills : [`docs/drills/`](../drills/)

## 8. Honest disclosure

This runbook is informed by the 10-pass chirurgical audit that found
**67 defects / coverage gaps** in PR #24 before it shipped publicly.
The remaining residual risks documented in lessons-learned § "Known
Phase-2 gaps" can each surface as an incident — the table in § 4
above lists the most likely ones first.

If you are paged for an incident class **not** listed in § 4, do not
improvise.  Open `#release-eng` and ask for a runbook update before
acting.  An undocumented incident class is itself a Phase-2 gap.
