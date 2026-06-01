# EFE master gap tracker

**Purpose** : single source of truth for the open / closed status of
every named gap (G-XX, G-CB-XX, G-PR27.1-XX) across the EFE
bench-platform.  Each entry is grep-able, linkable, and references
its pinning test + tracking PR.

**Conventions** :
- One row per gap.
- "Status" : `open` / `[in flight, PR # ...]` / `[closed, verified <date>]` / `[deferred, tracked PR # ...]`.
- "Pinning test" : exact `cargo test` invocation that proves the gap
  is closed (FAILS when open, PASSES when closed).
- "Provenance" : where the gap was first surfaced (which PR's lessons
  file § 7, or which audit pass D-code).
- Closed gaps are NOT removed from the tracker — they are kept as
  historical record with the "Closure provenance" subsection.

---

## Active gaps (open or in flight)

| Gap                          | One-line description                                                                              | Owner               | Status                       | Pinning test                                              | Provenance                                  |
|------------------------------|---------------------------------------------------------------------------------------------------|---------------------|------------------------------|-----------------------------------------------------------|---------------------------------------------|
| **G-CB-IMPL**                | Per-CB-benchmark workload implementation (not just runner scaffolding)                            | bench-platform      | open                         | `cargo test --test v1_7_section_27_3_cb_scripts_present_and_executable` | PR #25 lessons § 7                          |
| **G-CB-BASELINES**           | Production baseline NDJSON for each CB scenario (currently only fixtures)                         | bench-platform      | open                         | (no dedicated test yet, by design — gap is "data missing")| PR #25 lessons § 7                          |
| **G-CB-LONGRUN-INFRA**       | Long-run nightly CB scenarios (24h+, soak/leak family)                                            | release-eng         | open                         | (no dedicated test yet — infra-level gap)                 | PR #25 lessons § 7                          |
| **G-PR27.1-OVERRIDE-LEN**    | `override_reason` length constraint enforcement (≤ 200 chars)                                     | bench-infra         | [deferred, tracked PR #27.1] | (to add in PR #27.1 alongside resolver)                   | PR #27 S3 Pass 1 — D-S3-P1-1                |
| **G-PR27.1-UNIT-CLASSIFIER** | `is_throughput()` predicate generality (currently only `EventsPerSec`)                            | bench-infra         | [deferred, tracked PR #27.1] | (to add in PR #27.1)                                      | PR #27 S3 Pass 1 — D-S3-P1-2                |
| **G-PR27.1-TAXONOMY-AUTO**   | Automated Rust-enum vs contract-string validator                                                  | STABILITY steward   | [deferred, tracked PR #27.1] | (to add in PR #27.1)                                      | PR #27 S3 Pass 1 — D-S3-P1-3                |
| **G-PR27.1-WARN-MARGIN**     | Production semantic alignment of WARN_MARGIN_RATIO with per-metric thresholds                     | bench-infra lead    | [deferred, tracked PR #27.1] | (to add in PR #27.1)                                      | PR #27 S3 Pass 2 — D-S3-P2-2                |
| **G-PR27.1-COSIGN-INSTALL**  | cosign installation step in `.github/workflows/pr27-tolerance-used.yml`                           | release-eng lead    | [deferred, tracked PR #27.1] | (to add in PR #27.1, plus workflow YAML invariant test)   | PR #27 S3 Pass 4 — D-S3-P4-3                |
| **G-PR27.1-§24-REORG**       | Long-term STABILITY § 24 reorganisation to give S-suite its own first-class section              | STABILITY steward   | [deferred, tracked STABILITY-bump PR] | (STABILITY freeze tests will need restructuring)  | PR #27 S3 Pass 5 — D-S3-P5-2                |
| **G-PR27-S5-GOLDEN-REGEN**   | Documented procedure for regenerating the backfill golden file                                    | release-eng         | [deferred, tracked process docs] | (process-doc gap, no Rust test)                        | PR #27 S3 Pass 4 — D-S3-P4-2                |
| **G-ENGINE-CB**              | `bench-harness/src/engine.rs` regex `^S[0-9]{2}_` doesn't match CB-* scenarios (CB-catalogue expansion fallout) | bench-infra | open                | (existing 2 engine.rs tests would need to be re-scoped)   | CB-catalogue-expansion lessons § 7          |

---

## Closed gaps (historical record)

### G.11 — tolerance_used actually populated by verdict aggregator

| Field          | Value                                                                       |
|----------------|-----------------------------------------------------------------------------|
| One-line       | `tolerance_used` actually populated by the verdict aggregator across all NDJSON rows in the 4 verdict branches |
| Owner          | bench-platform                                                              |
| **Status**     | **[closed, verified 2026-08-07]**                                           |
| Closed by      | PR #27 (merged 2026-06-12 17:42 UTC ; verified at S12 closure ceremony 2026-08-07 17:00 UTC) |
| Pinning tests  | `cargo test --test g11_round_trip` (9 tests, all PASS) <br/> `cargo test --test g11_tolerance_used_populated` (7 tests, all PASS) <br/> `cargo test --test stability_contract_v17 v1_7_section_24_3_6_tolerance_used_shape` <br/> `cargo test --test stability_contract_v17 v1_7_section_27_2_taxonomy_closed` <br/> `cargo test --test stability_contract_v17 v1_7_section_24_3_6_g11_pinning_test_exists` <br/> `cargo test --test stability_contract_v17 v1_7_section_24_3_6_double_implication_wire_level` |
| Provenance     | PR #24 lessons file § 7 (registered 2026-03-18, ~ 4.7 months before closure) |

#### Closure provenance — G.11

- **Opened**         : 2026-03-18 (PR #24 lessons-learned § 7).  G.11 was the residual
                       "tolerance_used currently always empty" gap that survived
                       three subsequent PRs without action.
- **Preflight (S1)** : 2026-05-18.  Lessons file skeleton + pinning test stub
                       committed.  `g11_round_trip` FAILS as required.
- **Implementation (S2)** : 25-29 mai 2026.  5-day implementation across the
                            taxonomy layer + resolver + verdict matrix + backfill +
                            CI workflow.  158 tests in PR #27 scope at S2 EOD.
                            `g11_round_trip` PASSES for the first time.
- **Audit (S3)**     : 1-5 juin 2026.  6-pass chirurgical audit.  11 D-S3-PN-K
                       defects + 3 D-FINAL defects surfaced.  8 fixed in-session,
                       6 deferred to PR #27.1 (the G-PR27.1-* items above) + 1
                       process doc gap.
- **Review + merge (S4)** : 8-12 juin 2026.  PR landed 2026-06-12 17:42 UTC.
- **Backfill rollout (S5-S8)** : 15 juin → 10 juillet 2026.  Pre-G.11 NDJSON
                                 backfill executed.  All canonical fixtures
                                 backfilled cleanly ; the pre-G11_unbackfilled
                                 source variant is now empty in production.
- **Monitoring (S9-S12)** : 13 juillet → 7 août 2026.  4 weeks of post-merge
                            surveillance.  20/20 nightly PASS, 110/110 pinning
                            ritual PASS, 0 PR #27 scope incidents, 4/4 backfill
                            canonical case rotation, S11 quarterly publish dry-run
                            PASS (`PR-27-quarterly-dry-run-2026-07-31.md`).
- **Closure ceremony** : Vendredi 7 août 2026 14:00-17:00 UTC.  Triple sign-off
                         (bench-infra lead + release-eng lead + STABILITY
                         contract steward) recorded in
                         `docs/lessons/monitoring/PR-27-closure-2026-08-07.md`
                         § 7.
- **Test count at closure** : 174 tests in PR #27 scope (45 unit + 9 round-trip
                              + 7 tolerance-used pin + 51 freeze + 14 backfill
                              + 18 workflow YAML + 14 S3 chain + 16 S9-S12 chain).
- **Audit chain artefacts** :
  - 6 S3 signed checklists at `docs/lessons/checklists/PR-27-S3-P{1..6}.md`
  - 1 S3 standup notes at `docs/lessons/checklists/S3-standup-notes.md`
  - 1 S3 consolidation at `docs/lessons/PR-27-chirurgical-audit-2026-06-05.md`
  - 4 S9-S12 weekly monitoring reports at `docs/lessons/monitoring/PR-27-monitoring-week-{9..12}.md`
  - 1 S11 quarterly publish dry-run report at `docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md`
  - 1 S12 closure ceremony consolidation at `docs/lessons/monitoring/PR-27-closure-2026-08-07.md`
- **Audit-trail rows in STABILITY** : § 24.9 + § 27.5, both populated 2026-08-07
                                       at closure ceremony step 7.
- **Residual surface (deferred to PR #27.1)** : 6 G-PR27.1-* items (above) +
                                                1 process doc gap.

---

## Pre-closure gaps inherited from prior PRs (not in PR #27 scope, historical only)

| Gap     | Status            | Closed by | Date       |
|---------|-------------------|-----------|------------|
| G.1     | [closed, verified 2026-01-22] | PR #21 | 2026-01-22 |
| G.2     | [closed, verified 2026-01-22] | PR #21 | 2026-01-22 |
| G.3-G.6 | [closed, verified 2026-02-04] | PR #22 | 2026-02-04 |
| G.7     | [closed, verified 2026-02-19] | PR #23 | 2026-02-19 |
| G.8     | [closed, verified 2026-03-04] | PR #23 | 2026-03-04 |
| G.9     | [closed, verified 2026-03-18] | PR #24 | 2026-03-18 |
| G.10    | [closed, verified 2026-04-08] | PR #25 | 2026-04-08 |
| G.12    | [closed, verified 2026-04-22] | PR #25 | 2026-04-22 |
| G.13    | [closed, verified 2026-05-06] | PR #26 | 2026-05-06 |
| G.14    | [closed, verified 2026-05-06] | PR #26 | 2026-05-06 |
| G.15    | [closed, verified 2026-05-17] | PR #26 | 2026-05-17 |
| **G.11**| **[closed, verified 2026-08-07]** | **PR #27** | **2026-08-07** |

---

## How to use this tracker

- **Adding a new gap** : append a row to "Active gaps" with status `open`.
  Always include a one-line description + owner + pinning test (or
  rationale if no test exists yet) + provenance.
- **Promoting open → in flight** : edit the Status column to
  `[in flight, PR # NN]` when a PR opens that intends to close the gap.
- **Closing a gap** : move the row from "Active gaps" to a new
  subsection under "Closed gaps (historical record)" with a "Closure
  provenance" sub-section detailing the closure lifecycle.
- **Never delete a row** — closed gaps stay as historical record.

---

_Last updated : 2026-08-07 (PR #27 closure ceremony, G.11 closure)._
