<!--
═══════════════════════════════════════════════════════════════════════
  CANONICAL TEMPLATE for docs/lessons/PR-<N>-lessons.md

  Required by process invariant I.1 (STABILITY_CONTRACT v1.7 § 25.1).

  How to use :
    1. cp docs/lessons/_template.md docs/lessons/PR-<N>-lessons.md
    2. Replace every <PLACEHOLDER> with your real content.
    3. Run : bash scripts/audit-prompt-validator.sh docs/lessons/PR-<N>-lessons.md
       (the canonical file itself MUST validate clean ; see process_invariants.rs)
    4. Commit + push.

  The 7 sections below are MANDATORY in this order with these EXACT headers.
  The validator regex anchors on "## N. <prefix>" where prefix is :
    1. Headline
    2. Timeline summary
    3. Defect taxonomy
    4. What worked
    5. Process changes proposed
    6. Defect index
    7. Known residual gaps

  Anything beyond the 7 (extra context, appendices) goes AFTER section 7,
  numbered 8+ and freely titled.

  Golden reference example : docs/lessons/pr24-lessons-learned.md
  Architecture rationale   : docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 3
═══════════════════════════════════════════════════════════════════════
-->

# PR #<N> — Lessons learned : <K>-pass chirurgical audit

**Author** : @<github-handle>
**Date** : <YYYY-MM-DD>
**Status** : final | draft (mark as draft only while WIP ; merge requires final)
**Audience** : engineering leadership + future PR authors

**Companion docs** :
- Architecture (if applicable) : path looks like `docs/architecture/EFE_A1_PHASE<P>_PR<N>_ARCHITECTURE.md` — replace `<P>` and `<N>` with the real phase + PR number and convert to a real markdown link.
- Audit prompt : [`docs/process/audit-prompt.md`](../process/audit-prompt.md) (FR canonical)
- Validator    : `bash scripts/audit-prompt-validator.sh docs/lessons/PR-<N>-lessons.md`

> **Blameless reminder** : names below appear only in timeline rows.
> Root-cause discussion uses "the system did X" / "the process did Y",
> not "Alice did X".  Defect counts are properties of the work, not of
> the workers.

---

## 1. Headline

<ONE sentence : what is this PR, how many audit passes did it take, how many
defects were found.>

**Example (PR #24)** :
> A 10-pass chirurgical audit on a single PR found 67 defects in code that
> was, at the start of every pass, sincerely claimed to be "100 % done" by
> the same engineer running the audit.

---

## 2. Timeline summary

A row per audit pass.  The validator requires ≥ 1 row.  Tests-at-end is
the TOTAL passing test count after the pass (not the delta).

| Pass | Tests at end | Δ tests | Defects fixed (Σ) | Notable finding |
|---|---|---|---|---|
| 1 (initial) | <count> | — | <count> | <one-line description> |
| 2 | <count> | <±N> | <count> | <description> |
| ... | ... | ... | ... | ... |

**Verdict** : total tests added across audit : <N>.  Total defects
fixed : <N>.  Average defects/pass : <ratio>.

> If the audit converged at pass 1 (no defects beyond the first sweep),
> still record ONE row with "no defects found post-implementation".
> This is rare ; PR #24 had 6.7 defects/pass sustained across 10 passes.

---

## 3. Defect taxonomy

Bucket the defects into the canonical 6 categories.  Adjust counts so
they sum to the total in § 2.

| Category | Count | % | Examples (D-codes) |
|---|---|---|---|
| **Doc/code drift** | <N> | <%> | <D-codes that fit this bucket> |
| **Coverage gap** | <N> | <%> | <...> |
| **Schema drift** | <N> | <%> | <...> |
| **Security / integrity** | <N> | <%> | <...> |
| **Spec violation** | <N> | <%> | <...> |
| **Process gap** | <N> | <%> | <...> |

> If a defect doesn't fit any bucket, add a new category here and propose
> it as an addition to `docs/process/sot-pattern.md` § "Defect taxonomy
> reference" in § 5 below.

---

## 4. What worked / What did not

Honest, blameless.  Aim for 3-5 bullets per side.  Specific > abstract.

### Worked

- <Specific tool, process step, or technique that helped>
- <...>
- <...>

### Did not work

- <Specific failure pattern observed.  Use "the X did Y" framing>
- <...>
- <...>

> Common failure patterns from PR #24 (cite if applicable) :
> - "Tests written from MENTAL MODEL of behaviour are unreliable"
> - "Sincere 'done' verdicts were demonstrated wrong within hours"
> - "Self-reported coverage gave false confidence"

---

## 5. Process changes proposed

What did THIS audit reveal that should change for FUTURE PRs?  If
nothing, write "None — PR #25 process worked as designed."

- [ ] <Proposed change 1 ; owner : <name> ; deadline : <date>>
- [ ] <Proposed change 2>

These items feed into the I-stream of the roadmap
(`docs/roadmap/EFE_A1_PHASE2_ROADMAP_REFRESH.md` § 2.3 or successor).

If you propose a change to STABILITY_CONTRACT § 25 (the frozen process
invariants), call it out HERE with rationale ; require engineering-
leadership sign-off before merge.

---

## 6. Defect index

The complete grep-able catalogue.  The validator requires ≥ 1 row.
Use the convention `D-<code>` where `<code>` is sequentially-assigned
within the PR (e.g. D-1, D-2, ... or D-A, D-B, ...).

| Code | Pass | Class | One-line description |
|---|---|---|---|
| D-1 | 1 | <category from § 3> | <description> |
| D-2 | 1 | <...> | <...> |
| ... | ... | ... | ... |

> **Minimum-viable row** : even if the audit found ZERO real defects,
> add `| D-0 | 1 | None | Audit converged at pass 1, no findings |`.
> This proves the audit happened.

---

## 7. Known residual gaps

The validator requires this section to have a non-empty body — even
if the body is "None — audit converged."

Items deliberately NOT fixed in this PR but tracked for follow-up :

- **G-<code>** — <gap title> :
  - Why not fixed in this PR : <one sentence>
  - Pinned by test : <test name in tests/...> (the test FAILS when the gap closes — making the fix visible)
  - Tracked at : <link to roadmap / issue>
  - Estimated effort to close : <weeks>

- **G-<code>** — <next gap> :
  - ...

If genuinely no gaps remain :

> None — audit converged at pass <N>.  All residual zones explicitly
> enumerated in § 5 of pass <N>'s self-report.

---

<!-- Sections 8+ are OPTIONAL.  Use them for context that doesn't fit
     the 7 mandatory sections : appendices, detailed timelines, raw
     audit-prompt iterations, screenshots, etc.  These do NOT need to
     follow any schema. -->

## 8. (Optional) Audit prompt iterations

Per-pass excerpt of how the audit prompt evolved.  Useful if you tuned
the prompt and want future authors to see what worked.

### Pass 1
> <the prompt or instruction used>

### Pass 2
> <...>

---

## 9. (Optional) References

- <Link to architecture doc>
- <Link to roadmap entry>
- <Link to related lessons files>
- <Link to PRs that this PR builds on>

---

## 10. (Optional) Conclusion

<A 1-paragraph closing statement.  The PR #24 conclusion is the gold
reference for tone : honest about cost, honest about residuals, no
victory-lap framing.>
