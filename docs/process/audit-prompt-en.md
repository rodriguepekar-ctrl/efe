# Audit prompt — English version

**Status** : translation of the canonical FR version
**FR canonical** : [`docs/process/audit-prompt.md`](audit-prompt.md)
**Frozen at** : STABILITY_CONTRACT v1.7 § 25.1
**Last updated** : 2026-05-16

This is a faithful English translation of the chirurgical audit prompt
that PR #25 (process invariant I.1) makes mandatory for PRs ≥ 1000 LOC
touching the harness source tree.

> **Source-of-truth note** : if this file and the FR version drift,
> the FR version (`audit-prompt.md`) wins.  The original prompt was
> calibrated empirically on PR #24 in French ; the English translation
> is operational only.

---

## 1. The prompt

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

Copy this verbatim into your audit tool (Claude Code, ChatGPT, or any
other LLM).  Don't paraphrase : every sentence was empirically
calibrated on PR #24.

---

## 2. When to use this prompt

### 2.1 Mandatory cases (I.1 enforcement)

The CI status check `pr25-i1-audit-prompt` requires a
`docs/lessons/PR-<N>-lessons.md` file when :

- The PR has a net delta ≥ 1000 LOC, **AND**
- The PR touches at least one file under :
  - `bench/harness/src/`
  - `crates/*/src/`
  - `tools/loadgen/src/`

In these cases, **you MUST** apply this prompt before requesting review.
The lessons file is the output of that application.

### 2.2 Optional cases (self-imposed)

You MAY apply this prompt even when I.1 doesn't require it, if :

- You touch a high-blast-radius zone (cosign, gh-pages deploy, branch
  protection, evidence generation).
- The PR is short but fixes a security defect.
- You introduce a new external dependency.
- You modify a file that the PR #24 10-pass audit already catalogued
  as "drift surface" (see `docs/lessons/pr24-lessons-learned.md` § 4 —
  embed_smoke surface, scenario page slugs, cosign regex, etc.).

### 2.3 When NOT to use

- Pure refactor (whitespace / formatter) — use the `[I.1-refactor]`
  tag instead (see § 5).
- Urgent security hotfix — use `[I.1-bypass]` (see § 5).
- Changes < 50 LOC purely under docs / test fixtures.

---

## 3. How to execute the prompt

### 3.1 With Claude Code

```bash
# Inside the repo, open a Claude Code session.
# Paste the prompt EXACTLY as in § 1.
# Claude iterates until you say "enough" or "stop".
```

For each pass, Claude :
1. Enumerates the un-audited zones at the top of its response.
2. Finds ≥ 1 defect + codes the fix + writes the test.
3. Updates `docs/lessons/PR-<N>-lessons.md` with the new D-code.
4. WAITS for your confirmation before moving to the next pass (the
   prompt says "Repeat until the user says 'stop'").

### 3.2 With another LLM

Same formulation.  The prompt is calibrated to work on any modern LLM
(GPT-4, Claude 3+, Gemini Pro).  Only adapt the file references to
your workspace.

### 3.3 Without an LLM (solo human)

The prompt also works as a **mental checklist**.  A human reviewer
who literally follows the 5 steps (rather than skim-reading) will
find defects they would have missed.

For a solo audit :
- Reserve an uninterrupted time block (1-3h depending on PR size).
- Print the prompt on paper or pin a sticky-note.
- Type your output into the lessons file as you go.
- Stop only when you can honestly write "audit converged" in § 7
  Known residual gaps.

---

## 4. Anti-patterns observed on PR #24

This section catalogues the behaviours the prompt corrects.  If you
catch yourself doing one of these during your audit, restart the
current pass.

### 4.1 "I tested it, it's fine"

**Symptom** : you write tests that validate your MENTAL MODEL of the
code instead of validating its actual behaviour.

**Concrete PR #24 case** : the initial D-T test asserted that `.+$`
rejected trailing junk after `refs/`.  The test failed in reality
because `.+` is greedy.  The code worked — the test was wrong.

**Correction** : read the code BEFORE writing the test.  Assert what
the code DOES, not what you BELIEVE it does.

### 4.2 "It's 100 % done"

**Symptom** : you declare "done" after 1-2 passes because you don't
find anything obvious anymore.

**Concrete PR #24 case** : between passes 1 and 9, the engineer
sincerely declared "100 % done" 9 times.  Each time, the next pass
found 4-11 non-trivial defects.

**Correction** : the prompt says "Repeat until the user says stop".
Don't stop on your own.  If you're solo, ask a buddy to tell you "stop".

### 4.3 "This zone doesn't need auditing"

**Symptom** : you skip part of the diff because it "doesn't seem
critical".

**Concrete PR #24 case** : `escape_pdf_text` seemed trivial.  Audit
pass 7 (D-AL) discovered the production body contained `§`
(non-ASCII) → mojibake `Â§` in every auditor PDF.  Not critical in
theory ; embarrassing in practice.

**Correction** : enumerate ALL zones touched by the diff.  The prompt
says "Explicitly enumerate the zones you have NOT audited" — use that
list so you don't forget anything.

### 4.4 "The defect is too small to document"

**Symptom** : you fix a defect but don't give it a D-code, don't add
it to the § 6 table in the lessons file.

**Concrete PR #24 case** : without the D-code, the defect doesn't
exist in the audit trail.  Future PRs can't reference it.  The
statistical categorisation in § 3 is skewed.

**Correction** : EVERY fix observes the rule "1 fix = 1 D-code in the
table".  Including one-line bugs.  Especially one-line bugs (they're
the most prone to re-introduction).

### 4.5 "I'll document later"

**Symptom** : you fix 10 defects then write the lessons file at end
of day.

**Concrete PR #24 case** : the memory of defects found in pass 2 is
hazy by the time you're writing in pass 5.  D-codes lack diagnostic
precision.

**Correction** : update `docs/lessons/PR-<N>-lessons.md` IMMEDIATELY
after each fix.  The prompt says "Document the defect ... in the
lessons file" — step 4 of 5.

---

## 5. Bypass tags

You can bypass I.1 in exactly two cases.  Both require a permanent
justification entry in `docs/process/i1-bypass-log.md`.

### 5.1 `[I.1-bypass]` — security hotfix

Use when :
- A CVE must ship in hours, not days.
- A patch is forced by compliance (external audit finds a hole).

Procedure :
1. Prefix the PR title with `[I.1-bypass]` (before the Conventional
   Commit prefix).
2. Add a row to `docs/process/i1-bypass-log.md` justifying.
3. Ask an engineering-leadership admin to merge via
   `gh pr merge --admin`.
4. **Within 7 days** : open a follow-up PR with the full lessons file
   and a D-code "post-merge audit findings".

### 5.2 `[I.1-refactor]` — mechanical refactor

Use when :
- The diff is ≥ 95 % whitespace + import reordering.
- `cargo fix --allow-no-vcs` generated the diff.
- A formatter (`cargo fmt`) was applied.

Procedure :
1. Prefix the PR title with `[I.1-refactor]`.
2. Justify with a screenshot or snippet of `git diff --shortstat`.
3. A release-eng reviewer (not the author) confirms.
4. No lessons file required ; no follow-up PR.

### 5.3 Quarterly review

Release-eng reviews `i1-bypass-log.md` every quarter :
- **< 25 % bypass rate** on qualifying PRs : OK.
- **≥ 25 %** : triggers a re-design of the rule.  Either the prompt
  is too strict, or the culture is dodging the audit.

---

## 6. Expected response examples

### 6.1 Pass 1 (initial audit)

```
## Pass 1 — initial audit

Zones not yet audited (explicitly enumerated) :
- bench/harness/src/publish/aggregator.rs (new module)
- bench/harness/src/publish/api/current.rs (modified)
- tests/aggregator_real_disk.rs (new)

Defect found : D-1
  - Class : Coverage gap
  - Symptom : `aggregator::from_disk` doesn't log when a summary.json
    is corrupt — the operator can't distinguish "no data" from "JSON
    corrupted".
  - Fix : add `tracing::warn!` with scenario_id + path + error.
  - Pinning test : `d_o_aggregator_warns_on_corrupt_summary` in
    tests/aggregator_real_disk.rs.

I updated docs/lessons/PR-26-lessons.md :
  - § 2 : added row "Pass 1 | tests=412 | +1 | 1 | D-1 logging gap"
  - § 6 : added row D-1.
  - § 3 : Coverage gap = 100 % (1/1).

Continue to pass 2 ?
```

### 6.2 Pass N (convergence)

```
## Pass N — final pass

Zones not yet audited : none.  I've covered all modified files + all
call sites of touched functions.

Defect found : none this pass.

Recommendation : declare audit converged.  I updated § 7 of the
lessons file : "None — audit converged at pass N."

Ready for human review.
```

---

## 7. Empirical calibration

The prompt was calibrated on PR #24 :

| Metric | Value |
|---|---|
| Passes to convergence | 10 |
| Total defects found | 67 |
| Average defects / pass | 6.7 (sustained, did NOT decline) |
| Test count delta | +155 (200 → 355) |
| Defect categories covered | 6/6 (doc drift, coverage, schema, security, spec, process) |

For Phase-3 PRs (with I.1 process in place from day 0), the empirical
target is :

| Metric | Target |
|---|---|
| Passes to convergence | 2-3 |
| Defects found | < 20 |
| Audit budget vs implementation | 30 % (vs 150 % for PR #24) |

If PR #26 emerges with ≥ 67 defects, the roadmap (§ 7) triggers a
full re-examination of I-stream — the process change didn't take.

---

## 8. References

- Canonical lessons file : [`docs/lessons/pr24-lessons-learned.md`](../lessons/pr24-lessons-learned.md)
- Lessons template : [`docs/lessons/_template.md`](../lessons/_template.md)
- Validator : `bash scripts/audit-prompt-validator.sh <path>`
- SOT pattern (I.2 companion) : [`docs/process/sot-pattern.md`](sot-pattern.md)
- Budget tracker (I.6 companion) : [`docs/process/audit-budget-tracker.md`](audit-budget-tracker.md)
- Bypass log : [`docs/process/i1-bypass-log.md`](i1-bypass-log.md)
- PR #25 architecture : [`docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md`](../architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md)
- STABILITY_CONTRACT v1.7 § 25 : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md)
