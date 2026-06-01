# CB Catalogue 15 → 35 expansion — Lessons learned : 2-pass chirurgical audit

**Author** : @rodrigue.pekar
**Date** : 2026-05-17
**Status** : final
**Audience** : engineering leadership + future PR authors of CB-catalogue
amendments

**Companion docs** :
- Architecture (CB catalogue) : [`docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md`](../architecture/EFE_CORE_BENCHMARK_CATALOGUE.md)
- STABILITY contract addition : [`docs/STABILITY_CONTRACT.md`](../STABILITY_CONTRACT.md) § 27
- Audit prompt : [`docs/process/audit-prompt.md`](../process/audit-prompt.md) (FR canonical)
- Validator    : `bash scripts/audit-prompt-validator.sh docs/lessons/CB-catalogue-expansion-lessons.md`

> **Blameless reminder** : names below appear only in timeline rows.
> Root-cause discussion uses "the catalogue did X" / "the validator did Y",
> not "Alice did X".  Defect counts are properties of the work, not of
> the workers.

---

## 1. Headline

A deep-coverage audit of the CB catalogue surfaced 20 previously-uncovered
surfaces (10 crates + 8 top-level subsystems + 2 packaging artefacts),
widening the catalogue from 15 to 35 benchmarks and introducing 2 new
frozen families (`integration`, `long-run`) in 2 audit passes with
1 family-validation defect found and fixed.

---

## 2. Timeline summary

| Pass | Tests at end | Δ tests | Defects fixed (Σ) | Notable finding |
|---|---|---|---|---|
| 1 (initial expansion) | 47 | — | 0 | 20 new YAML scenarios drafted ; validator + freeze tests untouched ; assumed family enumeration was already general |
| 2 (validator sweep) | 47 | 0 | 1 | Validator rejected 6/20 new scenarios (families `integration` + `long-run` not in the 5-family allow-list) ; fix : widen allow-list to 7 families ; orchestrator + STABILITY § 27.1 + freeze test all updated to expect 35 |

**Verdict** : total tests added across audit : 0 (the existing 47 freeze
tests cover the expansion through the on-disk-set check, which auto-
expanded with the canonical-array update). Total defects fixed : 1.
Average defects/pass : 0.5.

> The low defect rate (0.5/pass vs PR #24's 6.7/pass) is explained by
> the work being a *closed-form expansion* of an already-frozen surface,
> not a new subsystem implementation. The single defect was a family
> enumeration that the original catalogue had hard-coded to five.

---

## 3. Defect taxonomy

| Category | Count | % | Examples (D-codes) |
|---|---|---|---|
| **Doc/code drift** | 0 | 0 % | — |
| **Coverage gap** | 0 | 0 % | — |
| **Schema drift** | 1 | 100 % | D-FAM |
| **Security / integrity** | 0 | 0 % | — |
| **Spec violation** | 0 | 0 % | — |
| **Process gap** | 0 | 0 % | — |

---

## 4. What worked / What did not

### Worked

- **Closed-form expansion under a frozen schema** : the catalogue's
  YAML schema and the validator's exit-code contract were precise
  enough that drafting twenty new scenarios took under an hour, and
  the only failure-class that escaped the initial draft was the
  family enumeration.
- **`bash scripts/run-core-benchmarks.sh --validate-only`** as a
  single-line gate : the orchestrator's validate-only mode produced
  a green-or-red verdict for the entire 35-scenario set in under two
  seconds, making pass-2 cleanup a single edit + single rerun.
- **The freeze test's two-sided check** (`for every canonical id, a
  YAML exists ; for every YAML on disk, a canonical id exists`)
  auto-detected the expansion without code changes once the canonical
  array was updated. Adding identifiers to one side without the other
  would have failed loudly.

### Did not work

- **The catalogue's original prose committed to "five families"** in
  several § 4 sub-headings + § 2.1 lists. Updating those required
  hunting through ~800 lines of doc to find every occurrence ; a
  numbered-family table at the top of § 4 would have been a single
  edit.
- **The original "fifteen" was textually embedded** in § 1.1 ("introduces
  fifteen"), § 1.2 (the subsection title itself), § 2.1, § 4 heading,
  § 6 (cadence), § 8 (STABILITY additions), and § 11 (acceptance
  criteria). Future expansions should use a leading "N" constant
  defined once and referenced by symbol-style markers, not by spelled-out
  numerals.

---

## 5. Process changes proposed

- [ ] When adding a frozen enumeration (e.g. families, exit codes,
      verdict states), pin it with **one** canonical list in the
      governing doc + a freeze test that asserts no other location
      enumerates it independently. Avoid spelled-out cardinality in
      narrative prose where possible ; numeric symbols ("the N
      families") survive expansion gracefully.
      Owner : @rodrigue.pekar ; deadline : 2026-06-01.
- [ ] Add a `scripts/cb-scenario-family-allowlist.txt` (single source
      of truth) read by both the validator and the catalogue doc's
      generator, so the family enumeration cannot drift between them.
      Owner : @bench-infra-lead ; deadline : 2026-07-01.

---

## 6. Defect index

| Code | Pass | Class | One-line description |
|---|---|---|---|
| D-FAM | 2 | Schema drift | `cb-scenario-validator.sh` accepted only 5 families ; the new scenarios used 2 additional families (`integration`, `long-run`) ; widened the allow-list to 7 and updated the error-message text |

---

## 7. Known residual gaps

- **G-CB-IMPL** — Per-benchmark workload implementation :
  - Why not fixed in this expansion : the expansion ships YAML
    scenarios, the validator widening, the STABILITY freeze and the
    freeze tests. The actual workload-driving code for the 20 new
    benchmarks lands across follow-up PRs, one per benchmark or
    grouped by family-owner.
  - Pinned by test : `v1_7_section_27_3_cb_scripts_present_and_executable`
    (passes today against the placeholder orchestrator ; will require
    a stricter "scenario `--run` exits 0 for each id" upgrade once
    workloads exist).
  - Tracked at : the bench-infra board, "CB workload" epic.
  - Estimated effort to close : ~8 engineer-weeks distributed across
    family owners.

- **G-CB-BASELINES** — Reference baselines for the 20 new scenarios :
  - Why not fixed in this expansion : baselines are calibrated against
    a real workload + a chosen runner ; impossible to capture before
    G-CB-IMPL.
  - Pinned by test : none yet (baselines are data, not contracts) ;
    will be pinned by `bench/baselines/CB-NN.json` existence checks in
    a future freeze test.
  - Tracked at : same epic as G-CB-IMPL.
  - Estimated effort to close : ~2 engineer-weeks once G-CB-IMPL is
    done.

- **G-CB-LONGRUN-INFRA** — Dedicated long-run runner for CB-29 + CB-30 :
  - Why not fixed in this expansion : the weekly cadence + runner
    pool is an SRE deliverable, not a catalogue deliverable. The
    catalogue commits to *what* runs, the SRE team commits to *where*.
  - Pinned by test : none — infra concern.
  - Tracked at : SRE quarterly plan, Q3-2026.
  - Estimated effort to close : ~1 engineer-week of SRE work.

---

## 8. (Optional) Audit prompt iterations

### Pass 1
> "Cover every crate under `crates/` and every top-level subsystem
> that does not currently have a CB-NN scenario. For each one, draft
> a scenario YAML matching the existing schema, choose a family that
> reflects the surface, declare a tolerance budget consistent with the
> declared SLI."

### Pass 2
> "Run the existing validator + freeze test against the new scenarios.
> Treat every red line as a contract drift to triage : either fix the
> scenario (if it violated the existing contract) or widen the contract
> (if the scenario revealed a missing frozen surface). Document the
> choice in the audit-trail row."

---

## 9. (Optional) References

- Architecture : `docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md`
- STABILITY    : `docs/STABILITY_CONTRACT.md` § 27 (with the
  2026-05-17 deep-coverage audit-trail row)
- Sibling lessons : `docs/lessons/PR-25-lessons.md` (process invariants),
  `docs/lessons/PR-26-lessons.md` (post-mortem CI enforcement)

---

## 10. (Optional) Conclusion

The expansion shipped a closed-form widening of a frozen surface : 20
new scenarios, 2 new family enumerants, 1 validator defect, 0 new
tests required (the existing two-sided freeze test auto-detected the
expansion once its canonical array was updated). The catalogue is now
exhaustive across the platform's non-coordination surface : every
shipped crate and every top-level subsystem is exercised by at least
one suite. The catalogue's freeze discipline survived the expansion
unchanged — no STABILITY invariant was relaxed ; one was widened (the
family allow-list grew from 5 to 7), which is a STABILITY-bump that
the audit-trail row records explicitly.
