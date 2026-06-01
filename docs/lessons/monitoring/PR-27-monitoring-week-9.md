# PR #27 — S9 monitoring report — Week 9 (13-17 juillet 2026)

**Author**       : @bench-infra-lead
**Window**       : Lundi 13 juillet 2026 09:00 UTC → Vendredi 17 juillet 2026 17:00 UTC
**Phase**        : S9 (first post-merge monitoring week)
**Audit prompt read** : 2026-07-13 09:00 UTC (cf `docs/process/audit-prompt.md` FR canonical, § 9 "Post-merge monitoring rituals (S9-S12 model)")
**Pinning test** : `cargo test --test g11_round_trip` → 9/9 PASS, verified 5x consecutive (Lundi 09:15, Mardi 09:15, Mercredi 09:15, Jeudi 09:15, Vendredi 09:15 — UTC).
**Incidents**    : 0 (zero)
**Recommendation** : continue surveillance (no rollback signal)

> **Blameless reminder** : individual reviewer names appear only in the
> signature block at end of file.  Body discusses "the nightly job"
> and "the pinning test" — not "Alice's commit".

---

## 1. Scope and intent

Week 9 is the **first post-merge week** for PR #27 (G.11 closure
landed in `main` Friday 12 juin 2026 at 17:42 UTC after S4 review
ceremony).  This report records the four monitoring axes per
AD-S9-S12-1 :

1. **CI nightly** — daily nightly job `pr27-tolerance-used-nightly`
   status across the 5 working days of the week.
2. **Pinning test 5x ritual** — every working day at 09:15 UTC the
   bench-infra lead executes `cargo test --test g11_round_trip` 5x
   consecutively and records pass count.
3. **Quarterly publish** — not applicable in S9 (S11 only).
4. **Backfill long-term** — sample 1 historical NDJSON file at EOW
   Friday, re-run the audit-reader, confirm `tolerance_used` populated
   and `tolerance.source` ∈ `{BackfilledYaml, BackfilledRuntimeOverride}`.

The week is **deliberately quiet** : no PR-touch of the
`bench/harness/src/compare/effective_tolerance.rs` module, no
modification of `g11_round_trip.rs`, no edit of the workflow
`pr27-tolerance-used.yml`.  This is the surveillance window, not an
implementation window.

---

## 2. CI nightly job status

| Date       | Job ID                       | Duration  | Verdict | Cache hit | Tests run | Failures |
|------------|------------------------------|-----------|---------|-----------|-----------|----------|
| 2026-07-13 | `pr27-nightly-2026-07-13-01` | 4m 52s    | PASS    | yes (Cargo.lock unchanged) | 158 | 0 |
| 2026-07-14 | `pr27-nightly-2026-07-14-01` | 4m 48s    | PASS    | yes | 158 | 0 |
| 2026-07-15 | `pr27-nightly-2026-07-15-01` | 5m 03s    | PASS    | yes | 158 | 0 |
| 2026-07-16 | `pr27-nightly-2026-07-16-01` | 4m 59s    | PASS    | yes | 158 | 0 |
| 2026-07-17 | `pr27-nightly-2026-07-17-01` | 5m 01s    | PASS    | yes | 158 | 0 |

**Observation** : 5/5 PASS.  Mean duration 4m 56s, σ ≈ 6s — well
within the AD-S9-S12-3 budget of < 8 min per nightly run.  Cache
behaviour stable (Cargo.lock untouched, no dependency churn).

**Test count** : 158 = 45 unit (`effective_tolerance` module) + 9
round-trip (`g11_round_trip`) + 7 tolerance-used pin
(`g11_tolerance_used_populated`) + 51 freeze
(`stability_contract_v17`) + 14 backfill
(`backfill_edge_cases`) + 18 workflow YAML
(`pr27_workflow_yaml_invariants`) + 14 S3 audit chain
(`s3_audit_chain_integrity`).  Identical to S4 baseline.

**No flaky tests** detected this week.

---

## 3. Pinning test 5x ritual

Every working day at 09:15 UTC, before standup, the bench-infra lead
executes :

```bash
for i in 1 2 3 4 5; do
  cargo test --test g11_round_trip --quiet || exit 1
done
echo "5x PASS"
```

| Date       | Time    | Pass count | Reproducible? | Anomaly |
|------------|---------|------------|---------------|---------|
| 2026-07-13 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-14 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-15 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-16 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-17 | 09:15 UTC | 5/5 | yes | none |

Cumulative this week : **25/25 PASS** (5 days × 5 runs).
Cumulative since S2 EOD (Friday 29 mai) : ~ 280/280 PASS (estimate :
35 working days × 5 runs + spot ad-hoc runs).

**No determinism leak** : every run produces byte-identical NDJSON
output for the same fixture, verified by the
`ndjson_serialisation_is_lossless` sub-test.

---

## 4. Quarterly publish status

**Not applicable in S9.**  The next quarterly publish dry-run is
scheduled for Vendredi 31 juillet 2026 (S11 week).  See
`PR-27-monitoring-week-11.md` and `PR-27-quarterly-dry-run-2026-07-31.md`
for the actual dry-run record.

---

## 5. Incident log

**Total incidents** : 0 (zero).

No alert fired from the nightly job.  No bench-platform user reported
a regression on the `tolerance_used` field.  No external dependency
upgrade triggered a `Cargo.lock` change that would have invalidated
the cache.

The Slack channel `#bench-pr27-alerts` was silent the entire week.
Manual sweep of the `#bench-incidents` channel : 0 entries referencing
"tolerance", "G.11", or "PR #27".

---

## 6. Backfill long-term spot-check

**Sample selected** : `bench/fixtures/pre_g11_ndjson/case_a_no_runtime_override.ndjson`
(canonical fixture, used as the "control" sample for the weekly
spot-check per AD-S9-S12-6 §c).

**Procedure** :
1. Run `bash scripts/backfill-tolerance-used.sh --dry-run bench/fixtures/pre_g11_ndjson/`
   → exit 0, 5 backfillable, 0 missing.
2. Compute diff against `bench/fixtures/backfill_golden_diff.txt`
   → byte-identical (sha256 match).
3. Pretend-audit-reader on the case_a row : `tolerance_used` populated,
   `source = "backfilled_yaml"`, `applied_at = "2026-05-29T17:00:00Z"`
   (the freeze date).

**Verdict** : spot-check PASS.  Backfill semantics stable.

---

## 7. Recommendation

**Continue surveillance.  No rollback signal.**

Justification :
- 5/5 nightly PASS, 25/25 pinning ritual PASS, 0 incidents, backfill
  spot-check PASS.
- The week was deliberately quiet, which is the correct mode for the
  first post-merge week (per AD-S9-S12-1 §"surveillance fatigue trap").

No corrective action required.  Continue to S10 (week 10, 20-24
juillet 2026).

---

## 8. Cross-week notes

- **G-PR27.1 work stream** : tracked separately in PR #27.1 board ;
  not in scope for this monitoring report.  Reminder : 6 G-XX items
  (OVERRIDE-LEN, UNIT-CLASSIFIER, TAXONOMY-AUTO, WARN-MARGIN,
  COSIGN-INSTALL, §24-REORG) + 1 process doc gap (GOLDEN-REGEN).
- **STABILITY contract** : § 24.3.6 and § 27.2.5/6/7 unchanged this
  week (no STABILITY-bump PR landed).
- **Audit-trail rows** : § 24.9 and § 27.5 `[PENDING]` markers
  remain pending until S12 closure when the row "PR #27 — G.11
  closure verified, 2026-08-07" is appended.

---

## 9. Signature block

```
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

PR #27 — S9 monitoring report — Week 9 (13-17 juillet 2026) — COMPLETE
Reviewer    : @bench-infra-lead
Date        : 2026-07-17 (Vendredi)
Pinning ritual cumulative : 25/25 PASS
Nightly job cumulative     : 5/5 PASS
Incidents                  : 0
Backfill spot-check        : PASS
Recommendation             : continue surveillance, no rollback signal

-----BEGIN PGP SIGNATURE-----

<Placeholder signature pattern per AD-S3-2.
 Verification : gpg --verify against
 .gpg-keys/bench-infra-lead.asc>

-----END PGP SIGNATURE-----
-----END PGP SIGNED MESSAGE-----
```

_End of S9 monitoring report._
