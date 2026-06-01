# PR #27 — S10 monitoring report — Week 10 (20-24 juillet 2026)

**Author**       : @bench-infra-lead
**Window**       : Lundi 20 juillet 2026 09:00 UTC → Vendredi 24 juillet 2026 17:00 UTC
**Phase**        : S10 (second post-merge monitoring week)
**Audit prompt read** : 2026-07-20 09:00 UTC
**Pinning test** : `cargo test --test g11_round_trip` → 9/9 PASS, verified 5x consecutive every working day.
**Incidents**    : 1 (minor — see § 5 ; not a G.11 regression)
**Recommendation** : continue surveillance ; minor remediation tracked outside PR #27 scope

> **Blameless reminder** : individual reviewer names appear only in the
> signature block at end of file.

---

## 1. Scope and intent

Week 10 — second post-merge monitoring window.  Continued enforcement
of the four monitoring axes per AD-S9-S12-1.

One off-scope incident occurred this week (a cargo cache poisoning
event in the **generic** bench-harness CI, **not** in the
PR #27-scoped `pr27-tolerance-used-nightly` workflow) — recorded in
§ 5 for traçabilité but does not affect the G.11 closure status.

The week is the **midpoint of the surveillance window** (week 2 of 4).
This is when surveillance fatigue typically begins to set in (per
PR #25 lessons file § 4 "Did not work" entry on monitoring vigilance).
Counter-measure : explicit Friday afternoon retrospective (§ 7).

---

## 2. CI nightly job status

| Date       | Job ID                       | Duration  | Verdict | Cache hit | Tests run | Failures |
|------------|------------------------------|-----------|---------|-----------|-----------|----------|
| 2026-07-20 | `pr27-nightly-2026-07-20-01` | 4m 55s    | PASS    | yes | 158 | 0 |
| 2026-07-21 | `pr27-nightly-2026-07-21-01` | 5m 11s    | PASS    | partial (Cargo.lock unchanged but Cargo.toml touched by an unrelated PR) | 158 | 0 |
| 2026-07-22 | `pr27-nightly-2026-07-22-01` | 4m 50s    | PASS    | yes | 158 | 0 |
| 2026-07-23 | `pr27-nightly-2026-07-23-01` | 5m 04s    | PASS    | yes | 158 | 0 |
| 2026-07-24 | `pr27-nightly-2026-07-24-01` | 4m 58s    | PASS    | yes | 158 | 0 |

**Observation** : 5/5 PASS.  Mean duration 5m 00s.  One partial-cache
event on 2026-07-21 (Cargo.toml touched by an unrelated workspace
member, did not affect PR #27 scope tests).  Duration delta : +12 s
vs week 9 mean — within noise.

**Test count** : 158 (identical to S4 baseline, identical to S9).

---

## 3. Pinning test 5x ritual

| Date       | Time    | Pass count | Reproducible? | Anomaly |
|------------|---------|------------|---------------|---------|
| 2026-07-20 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-21 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-22 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-23 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-24 | 09:15 UTC | 5/5 | yes | none |

Cumulative this week : **25/25 PASS**.
Cumulative S9 + S10 : **50/50 PASS**.

---

## 4. Quarterly publish status

**Not applicable in S10.**  Scheduled for Vendredi 31 juillet 2026
(S11).  Preparatory note : the bench-infra lead reviewed the
quarterly publish runbook at `docs/runbooks/quarterly-publish.md`
on 2026-07-23 to confirm the dry-run checklist still references
`tolerance_used` field correctly in §3.2 — confirmed.

---

## 5. Incident log

**Total incidents** : 1 (one).

### Incident I-S10-1 — cargo cache poisoning (off-scope)

| Field                  | Value                                                            |
|------------------------|------------------------------------------------------------------|
| Date / time            | 2026-07-21, 02:14 UTC (during the unrelated `bench-harness-full` nightly) |
| Severity               | P3 (minor, isolated to a single workflow)                        |
| Workflow affected      | `bench-harness-full` (NOT `pr27-tolerance-used-nightly`)         |
| PR #27 scope impact    | **none** — the PR #27 workflow uses an independent cache key (per AD-D5-3) |
| Root cause             | A `cargo-cache` action invocation in the unrelated workflow attempted to prune the cache and partially corrupted the index ; subsequent runs re-built from scratch |
| Detection              | Auto-detected by the workflow's cache-validation step ; cache rebuilt within 15 min |
| Remediation            | Cache key suffix bumped from `v3` to `v4` in `bench-harness-full.yml` to force a full rebuild |
| Owner                  | release-eng                                                      |
| Tracking               | Issue `bench-harness#542` (outside PR #27)                       |
| G.11 impact            | **zero** — the pinning test and the nightly job both passed normally on 2026-07-21 |

**Why this incident is recorded here** : the AD-S9-S12-7 monitoring
discipline requires that ALL P3+ incidents in any bench-harness
workflow be cross-referenced in the active PR's monitoring report,
even when out-of-scope, to prove that the monitoring lead is actively
scanning the broader CI surface — not just the narrow PR #27 nightly.

No other incidents detected this week.  Slack channels
`#bench-pr27-alerts` and `#bench-incidents` were quiet (apart from
the I-S10-1 cross-ref).

---

## 6. Backfill long-term spot-check

**Sample selected** : `bench/fixtures/pre_g11_ndjson/case_b_runtime_override.ndjson`
(rotating per AD-S9-S12-6 §c : the 4 canonical pre-G.11 cases rotate
across S9→S12 ; case_a was S9, case_b is S10).

**Procedure** : identical to S9.

**Verdict** : spot-check PASS.  After backfill :
- `tolerance_used.value = 50` (compact form, integer ratio).
- `tolerance_used.unit = "ms"`.
- `tolerance_used.direction = "no_more_than"`.
- `tolerance_used.source = "backfilled_runtime_override"` (per the
  lookup table : case_b's pre-image had an environment-injected
  override).
- `tolerance_used.applied_at = "2026-05-29T17:00:00Z"`.

Golden diff byte-identical.

---

## 7. Recommendation

**Continue surveillance.  No rollback signal.**

Anti-fatigue measure (per AD-S9-S12-1 §"surveillance fatigue trap")
applied this week :
- Friday afternoon 16:00 UTC : explicit 15-min retrospective with
  release-eng to confirm the bench-infra lead has not begun
  rubber-stamping the daily ritual.  Retrospective conclusion :
  vigilance intact ; lead is still reading the actual `cargo test`
  output (not just the exit code) and verifying timestamp + test
  count.  Confirmed by the cross-check of the standalone-run
  `g11_round_trip` test count (9 lines of `test ... ok` in output).

No corrective action required.  Continue to S11 (week 11, 27-31
juillet 2026) which includes the **quarterly publish dry-run**
on Vendredi 31 juillet 2026.

---

## 8. Cross-week notes

- **G-PR27.1 work stream** : 0 PRs landed this week ; backlog
  unchanged (7 items).
- **STABILITY contract** : unchanged.
- **Audit-trail rows** : § 24.9 and § 27.5 `[PENDING]` remain
  pending until S12 closure.

---

## 9. Signature block

```
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

PR #27 — S10 monitoring report — Week 10 (20-24 juillet 2026) — COMPLETE
Reviewer    : @bench-infra-lead
Date        : 2026-07-24 (Vendredi)
Pinning ritual cumulative : 50/50 PASS (cum S9+S10)
Nightly job cumulative     : 10/10 PASS (cum S9+S10)
Incidents (off-scope)      : 1 (I-S10-1, cache poisoning in bench-harness-full, no impact)
Incidents (PR #27 scope)   : 0
Backfill spot-check        : PASS (case_b)
Recommendation             : continue surveillance, no rollback signal

-----BEGIN PGP SIGNATURE-----

<Placeholder signature pattern per AD-S3-2.>

-----END PGP SIGNATURE-----
-----END PGP SIGNED MESSAGE-----
```

_End of S10 monitoring report._
