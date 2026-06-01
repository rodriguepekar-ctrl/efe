# PR #27 — S11 monitoring report — Week 11 (27-31 juillet 2026)

**Author**       : @bench-infra-lead
**Window**       : Lundi 27 juillet 2026 09:00 UTC → Vendredi 31 juillet 2026 17:00 UTC
**Phase**        : S11 (third post-merge monitoring week ; INCLUDES quarterly publish dry-run)
**Audit prompt read** : 2026-07-27 09:00 UTC
**Pinning test** : `cargo test --test g11_round_trip` → 9/9 PASS, verified 5x consecutive every working day.
**Incidents**    : 0
**Quarterly publish dry-run** : EXECUTED Vendredi 31 juillet 14:00-16:30 UTC ; **PASS** (see § 4 + companion doc)
**Recommendation** : continue surveillance ; quarterly publish surface validated end-to-end

> **Blameless reminder** : individual reviewer names appear only in the
> signature block at end of file.

---

## 1. Scope and intent

Week 11 — third post-merge week.  The pivotal week of S9-S12 :
it includes the **quarterly publish dry-run** on Vendredi 31
juillet, which is the first end-to-end validation that PR #27's
`tolerance_used` field flows correctly through the public-facing
quarterly bundle (PDF + JSON + signed manifest).

The dry-run is documented in detail in the companion file
`docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md`.
This monitoring report cross-references but does not duplicate.

---

## 2. CI nightly job status

| Date       | Job ID                       | Duration  | Verdict | Cache hit | Tests run | Failures |
|------------|------------------------------|-----------|---------|-----------|-----------|----------|
| 2026-07-27 | `pr27-nightly-2026-07-27-01` | 4m 53s    | PASS    | yes | 158 | 0 |
| 2026-07-28 | `pr27-nightly-2026-07-28-01` | 4m 57s    | PASS    | yes | 158 | 0 |
| 2026-07-29 | `pr27-nightly-2026-07-29-01` | 5m 02s    | PASS    | yes | 158 | 0 |
| 2026-07-30 | `pr27-nightly-2026-07-30-01` | 4m 51s    | PASS    | yes | 158 | 0 |
| 2026-07-31 | `pr27-nightly-2026-07-31-01` | 5m 06s    | PASS    | yes | 158 | 0 |

**Observation** : 5/5 PASS.  Mean duration 4m 58s.  Cache stable.
Test count 158 unchanged.

Cumulative S9 + S10 + S11 nightly : **15/15 PASS**.

---

## 3. Pinning test 5x ritual

| Date       | Time    | Pass count | Reproducible? | Anomaly |
|------------|---------|------------|---------------|---------|
| 2026-07-27 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-28 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-29 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-30 | 09:15 UTC | 5/5 | yes | none |
| 2026-07-31 | 09:15 UTC | 5/5 | yes | none — note : also re-run 09:50 UTC as the canary before the quarterly publish dry-run kickoff at 14:00 UTC ; 5/5 PASS confirmed there too |

Cumulative this week : **25/25 PASS** (regular) + 5/5 extra canary = 30/30.
Cumulative S9 + S10 + S11 : **75/75 PASS** (regular cadence).

---

## 4. Quarterly publish status

**Date / time** : Vendredi 31 juillet 2026, 14:00 UTC → 16:30 UTC (2h30).
**Status this week** : DRY-RUN EXECUTED (this is the S11 week when the
quarterly publish dry-run is scheduled per AD-S9-S12-5).  See § 4.1
summary below and the companion doc for full transcript.
**Operator** : @bench-infra-lead, paired with @release-eng-lead for the cosign signature segment.
**Companion doc** : [`docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md`](./PR-27-quarterly-dry-run-2026-07-31.md).

### 4.1 Summary

| Stage                                         | Result | Duration | Notes                                    |
|-----------------------------------------------|--------|----------|------------------------------------------|
| 1. Freeze snapshot                            | PASS   | 4m       | 158 PR #27 tests + 1,847 broader tests all green |
| 2. Quarterly bundle generation (PDF + JSON)   | PASS   | 38m      | Bundle size 247 MB ; byte-identical against last quarter's hash for non-G.11 sections |
| 3. `tolerance_used` field present in JSON     | PASS   | —        | grep confirms 100 % rows populated (per quarterly publish runbook § 5.4) |
| 4. PDF rendering preserves `tolerance_used`   | PASS   | 22m      | Manual sample : 12 / 12 sampled tables render the field correctly with `<value> <unit>` formatting |
| 5. Cosign signature (dry-run = no push)       | PASS   | 8m       | `cosign sign-blob --output-signature=quarterly-dry-2026-07-31.sig` produced 96-byte signature ; Rekor entry would have been created if `--push` had been set |
| 6. Audit-reader CLI round-trip                | PASS   | 4m       | `pretend_audit_reader` parses 100 % of rows ; `tolerance.source` distribution as expected (post-G.11 rows : 6 `yaml`, 2 `runtime_override`, 1 `gpu_runner_override`, 0 `no_tolerance_defined` ; backfilled rows : `backfilled_yaml` + `backfilled_runtime_override`) |
| 7. Idempotency check (re-run #2 same input)   | PASS   | 35m      | Byte-identical bundle hash, byte-identical cosign signature payload (deterministic by AD-D5-1) |

**Verdict** : DRY-RUN PASS.  No regression.  The quarterly publish
surface correctly carries `tolerance_used` end-to-end.

### 4.2 Cross-reference with companion doc

See [`PR-27-quarterly-dry-run-2026-07-31.md`](./PR-27-quarterly-dry-run-2026-07-31.md)
for :
- Raw command transcripts (with timestamps).
- The 12-sample PDF cell verification table.
- The Rekor staging-server response payload (mocked, since `--push`
  was not enabled).
- The full audit-reader output (truncated to first 50 rows + last
  10 rows per AD-S9-S12-9 §"transcript discipline").

---

## 5. Incident log

**Total incidents** : 0 (zero).

The quarterly publish dry-run executed cleanly.  No alerts fired.
No cache poisoning in any workflow.  Slack channels remained quiet
(apart from the routine "dry-run kickoff" + "dry-run complete"
announcements at 14:00 and 16:30).

---

## 6. Backfill long-term spot-check

**Sample selected** : `bench/fixtures/pre_g11_ndjson/case_c_no_tolerance_defined.ndjson`
(rotating per AD-S9-S12-6 §c).

**Procedure** : identical to S9/S10.

**Verdict** : spot-check PASS.  Note this case_c sample tests the
`source = no_tolerance_defined` branch (not backfilled), which is a
distinct sub-path of the backfill semantics : backfill correctly
**skips** this row rather than fabricating a synthetic tolerance.

After dry-run :
- `tolerance_used = null` (value).
- `tolerance.source = "no_tolerance_defined"`.
- `applied_at = null`.
- Double-implication invariant satisfied.

Golden diff byte-identical against `bench/fixtures/backfill_golden_diff.txt`
section for case_c.

---

## 7. Recommendation

**Continue surveillance.  No rollback signal.**
**Quarterly publish dry-run validates the full G.11 surface.**

Specific implication of this week's dry-run :
- The single biggest risk before S12 closure was the possibility that
  `tolerance_used` would somehow get lost in the bundle-generation
  pipeline (e.g. by a PDF template not referencing the field, or by
  the JSON serialiser stripping null values inconsistently).  The
  dry-run validates none of these failure modes occurred.
- One observation : the PDF rendering at stage 4 took 22 min on this
  dry-run, which is +3 min vs Q2 dry-run.  Diff explainable by the
  +1 column in the metric table (the new `Tolerance` column carrying
  `<value> <unit>` formatting).  Within budget.

No corrective action required.  Continue to S12 (week 12, 3-7 août
2026) which is the **closure week**.

---

## 8. Cross-week notes

- **G-PR27.1 work stream** : 1 PR #27.1 candidate (G-PR27.1-COSIGN-INSTALL)
  was prepared in draft this week but not merged ; expected to land
  during S12 alongside the broader PR #27.1 PR.  Not in scope for
  monitoring.
- **STABILITY contract** : unchanged.
- **Audit-trail rows** : § 24.9 and § 27.5 `[PENDING]` remain
  pending until S12 closure ceremony (next week).

---

## 9. Signature block

```
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

PR #27 — S11 monitoring report — Week 11 (27-31 juillet 2026) — COMPLETE
Reviewer    : @bench-infra-lead
Date        : 2026-07-31 (Vendredi)
Pinning ritual cumulative : 75/75 PASS (cum S9+S10+S11) + 5/5 canary
Nightly job cumulative     : 15/15 PASS (cum S9+S10+S11)
Incidents                  : 0
Backfill spot-check        : PASS (case_c)
Quarterly publish dry-run  : PASS (companion doc 2026-07-31)
Recommendation             : continue surveillance ; full surface validated

-----BEGIN PGP SIGNATURE-----

<Placeholder signature pattern per AD-S3-2.>

-----END PGP SIGNATURE-----
-----END PGP SIGNED MESSAGE-----
```

_End of S11 monitoring report._
