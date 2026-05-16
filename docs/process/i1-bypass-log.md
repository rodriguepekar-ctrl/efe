# I.1 bypass log

**Status** : live audit log (append-only)
**Process invariant** : I.1 bypass procedure (PR #25 § 3.5)
**Frozen at** : STABILITY_CONTRACT v1.7 § 25.1
**Owner** : `@efe-ai/release-eng` (writes entries) ; engineering-leadership (reviews quarterly)
**Cadence** : append per bypass ; review every Q

This file records every PR that bypassed I.1 (the mandatory audit
prompt + lessons file).  The bypass is a deliberate escape hatch for
two narrow cases ; this log makes the escape hatch AUDITED so it
doesn't become routine.

> **Read me first** : the bypass procedure is detailed in
> [`docs/process/audit-prompt.md`](audit-prompt.md) § 5.  Do NOT bypass
> without reading that section.  The bypass is NOT a way to "skip
> audit on a tight schedule" — it is for security hotfixes and
> mechanical refactors ONLY.

---

## 1. Bypass classes

Per the architecture, two and only two bypass tags are accepted :

### 1.1 `[I.1-bypass]` — security hotfix

**When acceptable** :
- A CVE must ship in hours, not days.
- An external audit / compliance officer mandates an immediate patch.
- An on-call incident requires a code change before a runbook fix
  suffices.

**Procedure (from audit-prompt.md § 5.1)** :
1. PR title prefixed `[I.1-bypass]` (before the Conventional Commit prefix).
2. Row in this log filled at PR-open time.
3. Merged via `gh pr merge --admin` by engineering-leadership.
4. Follow-up PR within 7 days with the full lessons file + a D-code
   "post-merge audit findings".

### 1.2 `[I.1-refactor]` — mechanical refactor

**When acceptable** :
- Diff is ≥ 95 % whitespace + import reordering (verifiable via
  `git diff --shortstat`).
- `cargo fix --allow-no-vcs` generated the diff.
- A formatter (`cargo fmt`) was applied.
- A mass-rename via `sed -i` / IDE refactor with no behaviour change.

**Procedure (from audit-prompt.md § 5.2)** :
1. PR title prefixed `[I.1-refactor]`.
2. Row in this log filled at PR-open time.
3. A release-eng reviewer (NOT the author) confirms the diff shape.
4. No follow-up PR required.

---

## 2. Bypass entries

Append-only.  DO NOT delete rows ; if an entry was filed in error,
add a correction row that supersedes the original (cite the original
PR number).

| Date (UTC) | PR | Tag | Author | Reason | Reviewer (refactor only) | Follow-up PR | Status |
|---|---|---|---|---|---|---|---|
| _no entries yet — log initialised by PR #25 on 2026-05-16_ | | | | | | | |

### 2.1 Field reference

- **Date (UTC)** : `YYYY-MM-DD` of bypass approval (merge time).
- **PR** : link to the bypassed PR.
- **Tag** : `[I.1-bypass]` or `[I.1-refactor]`.
- **Author** : `@github-handle` of the PR author.
- **Reason** : ≤ 25 words, factual.  Examples :
  - "CVE-2026-XXXX — directory traversal in summary-path resolver"
  - "cargo fmt applied across 47 files ; diff 99.2% whitespace"
- **Reviewer (refactor only)** : `@github-handle` of the release-eng
  reviewer who confirmed the diff shape.  Empty for security hotfixes
  (admin merge).
- **Follow-up PR** : link to the post-merge lessons-file PR for the
  `[I.1-bypass]` class.  Empty for `[I.1-refactor]`.
- **Status** :
  - `pending follow-up` : security hotfix merged, lessons-file PR not yet open.
  - `complete` : security hotfix has its follow-up lessons file landed.
  - `refactor closed` : refactor merged, no follow-up needed.
  - `corrected` : superseded by a later row (cite PR number).

---

## 3. Quarterly review checklist

Release-eng reviews this log every quarter (aligned with quarterly
drills — see [`docs/drills/`](../drills/)).

### 3.1 What to measure

For the trailing 90-day window :

| Metric | Calculation | Target |
|---|---|---|
| Bypass count | Rows added in window | n/a (raw count) |
| Total qualifying PRs | Count of PRs ≥ 1000 LOC + harness src touch from `audit-budget-tracker.md` | n/a |
| Bypass rate | bypass_count / total_qualifying * 100 | **< 25 %** |
| `[I.1-bypass]` follow-up compliance | % of security bypasses with completed follow-up PR ≤ 7 days | **100 %** |
| `[I.1-refactor]` reviewer-confirmed | % of refactor bypasses with a non-author reviewer signed off | **100 %** |

### 3.2 Action triggers

- **Bypass rate ≥ 25 %** : the bypass mechanism is being abused or the
  base rule is too strict.  Engineering-leadership decides between :
  - Tighten bypass procedure (stricter `[I.1-bypass]` justification template).
  - Relax I.1 threshold (e.g. 1500 LOC instead of 1000).
  - Replace I.1 with a different process.

- **Follow-up compliance < 100 %** : the follow-up PRs aren't landing
  on time.  Mandate that the next `[I.1-bypass]` author wait for the
  prior follow-up to land before opening a new bypass.

- **Reviewer-confirmed < 100 %** : authors are self-approving refactor
  bypasses.  Add a status check that asserts the reviewer is not the
  author.

### 3.3 Review report

After each quarterly review, release-eng files a 1-page report at
`docs/process/i1-bypass-review-YYYY-QN.md` with :

```markdown
# I.1 bypass review : YYYY-QN

- Window : YYYY-MM-DD to YYYY-MM-DD
- Bypass count : N
- Qualifying PRs : N
- Bypass rate : N %
- Target met : yes / no
- Action items : [if any]
```

---

## 4. Anti-patterns

### 4.1 "I'll add the lessons file in the next sprint"

For `[I.1-bypass]` (security hotfix), the follow-up is **7 days** —
not "next sprint."  If you can't commit to 7 days, choose a different
mitigation path (revert, feature flag, etc.) instead of bypassing.

### 4.2 "It's a refactor that also fixes a bug"

If your diff is "mostly whitespace + one behaviour change", the
behaviour change part is NOT a refactor.  Open TWO PRs : a
`[I.1-refactor]` for the whitespace, and a normal I.1-compliant PR
for the behaviour change.

### 4.3 "I'm the only person who can review my own refactor"

If you're the only person who can review a refactor, that's a team-
risk signal (bus factor) ; flag to engineering-leadership.  Don't use
self-approval as a workaround.

### 4.4 "The bypass log is internal — I don't need to fill it carefully"

This file is PUBLIC.  Auditors (SOC 2, ISO 27001) read it during
compliance reviews to assess the discipline of our development
process.  A sparse or hand-wavy bypass entry is a red flag.

---

## 5. Example entries (for reference only — not real bypasses)

These are NOT real bypasses ; they show what well-filled rows look
like.  Real entries replace these examples in § 2.

### 5.1 Example : security hotfix (hypothetical)

```
| 2026-07-12 | #34 | [I.1-bypass] | @alice | CVE-2026-31337 — TOCTOU race in scenario-id validator | — | #36 | complete |
```

### 5.2 Example : refactor (hypothetical)

```
| 2026-08-22 | #41 | [I.1-refactor] | @bob | cargo fmt applied workspace-wide ; 98.7% whitespace | @charlie | — | refactor closed |
```

### 5.3 Example : correction (hypothetical)

```
| 2026-09-04 | #48 | [I.1-refactor] | @dave | sed-rename 'OLD_NAME' -> 'NEW_NAME' workspace-wide | @eve | — | corrected by #51 |
| 2026-09-05 | #51 | [I.1-bypass] | @dave | Correction : #48 was misclassified as refactor — actually a behaviour change (the rename touched a CLI flag). Re-running audit. | — | #53 | complete |
```

---

## 6. References

- Bypass procedure : [`docs/process/audit-prompt.md`](audit-prompt.md) § 5
- I.1 enforcement workflow : [`.github/workflows/pr25-process-checks.yml`](../../.github/workflows/pr25-process-checks.yml) job `i1-audit-prompt`
- Budget tracker (cross-ref for bypass-rate denominator) : [`docs/process/audit-budget-tracker.md`](audit-budget-tracker.md)
- Architecture § 3.5 : [`docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md)
- STABILITY_CONTRACT v1.7 § 25.1 : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md)
- Quarterly drill cadence : [`docs/drills/`](../drills/)
