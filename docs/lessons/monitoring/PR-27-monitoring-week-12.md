# PR #27 — S12 monitoring report — Week 12 (3-7 août 2026) — CLOSURE WEEK

**Author**       : @bench-infra-lead
**Window**       : Lundi 3 août 2026 09:00 UTC → Vendredi 7 août 2026 17:00 UTC
**Phase**        : S12 (fourth and final post-merge monitoring week ; CONTAINS the closure ceremony Vendredi 7 août 17:00 UTC)
**Audit prompt read** : 2026-08-03 09:00 UTC
**Pinning test** : `cargo test --test g11_round_trip` → 9/9 PASS, verified 5x consecutive every working day + closure-day canary.
**Incidents**    : 0
**Closure ceremony** : EXECUTED Vendredi 7 août 14:00-17:00 UTC ; **G.11 marked [closed, verified]** in master gap tracker
**Recommendation** : transition to long-term lights-out monitoring (CI nightly only ; no daily ritual)

> **Blameless reminder** : individual reviewer names appear only in the
> signature block at end of file and in the closure ceremony triple
> sign-off in the companion consolidation doc.

---

## 1. Scope and intent

Week 12 — **the closure week**.  This report covers Monday through
Thursday under the normal monitoring cadence (per AD-S9-S12-1) and
then **records** the closure ceremony of Vendredi 7 août 14:00-17:00 UTC
which is documented in detail in the companion file
`docs/lessons/monitoring/PR-27-closure-2026-08-07.md`.

After the closure ceremony :
- The master gap tracker marks G.11 as `[closed, verified 2026-08-07]`.
- The lessons file `docs/lessons/PR-27-lessons.md` sections 1
  (headline) and 10 (conclusion) are finalised (no more `<PLACEHOLDER>`).
- The roadmap `docs/roadmap/EFE_A1_PHASE3_ROADMAP.md` archives PR #27.
- A new permanent meta-test pins the existence and structure of the
  S9-S12 monitoring chain artefacts (cf
  `bench/harness/tests/s9_s12_monitoring_chain_integrity.rs`).

---

## 2. CI nightly job status

| Date       | Job ID                       | Duration  | Verdict | Cache hit | Tests run | Failures |
|------------|------------------------------|-----------|---------|-----------|-----------|----------|
| 2026-08-03 | `pr27-nightly-2026-08-03-01` | 4m 52s    | PASS    | yes | 158 | 0 |
| 2026-08-04 | `pr27-nightly-2026-08-04-01` | 5m 00s    | PASS    | yes | 158 | 0 |
| 2026-08-05 | `pr27-nightly-2026-08-05-01` | 4m 56s    | PASS    | yes | 158 | 0 |
| 2026-08-06 | `pr27-nightly-2026-08-06-01` | 5m 04s    | PASS    | yes | 158 | 0 |
| 2026-08-07 | `pr27-nightly-2026-08-07-01` | 5m 08s    | PASS    | yes | 158 → 174 | 0 |

**Observation** : 5/5 PASS.  Mean duration 5m 00s.  **Test count
bumped from 158 to 174 on 2026-08-07** : this is the new meta-test
`s9_s12_monitoring_chain_integrity` (the equivalent for S9-S12 of
the S3 chain meta-test) landing in the closure-day commit.  The new
file contributes 16 tests (mirrors of the 14 tests in
`s3_audit_chain_integrity` plus 2 cross-doc invariants specific to
the monitoring chain — cumulative tally consistency + lessons file
finalisation pin).  Counted as **expected**, not as regression (per
AD-S9-S12-12 §"closure-day delta exemption").

Cumulative S9 + S10 + S11 + S12 nightly : **20/20 PASS**.

---

## 3. Pinning test 5x ritual

| Date       | Time    | Pass count | Reproducible? | Anomaly |
|------------|---------|------------|---------------|---------|
| 2026-08-03 | 09:15 UTC | 5/5 | yes | none |
| 2026-08-04 | 09:15 UTC | 5/5 | yes | none |
| 2026-08-05 | 09:15 UTC | 5/5 | yes | none |
| 2026-08-06 | 09:15 UTC | 5/5 | yes | none |
| 2026-08-07 | 09:15 UTC | 5/5 | yes | none — and re-run 16:30 UTC as the **closure-ceremony canary** ; 5/5 PASS confirmed before sign-off at 17:00 UTC |

Cumulative this week : **25/25 PASS** (regular) + 5/5 canary = 30/30.
Cumulative S9 + S10 + S11 + S12 : **100/100 PASS** (regular cadence) ;
**110/110 PASS** when including the S11 + S12 canary runs (5+5).

---

## 4. Quarterly publish status

**Not applicable in S12.**  Last dry-run was Vendredi 31 juillet
(S11) ; next real quarterly publish is the actual Q3 release on
Vendredi 25 septembre 2026 (outside the S9-S12 window).

The S11 dry-run record is the formal evidence that the quarterly
publish surface carries `tolerance_used` correctly.  The actual Q3
publish in september is **inherits** this evidence ; it does not
re-validate.

---

## 5. Incident log

**Total incidents** : 0 (zero).

The closure week was silent on all monitoring axes.  No alerts.  No
flaky tests.  No cache anomalies.  No bench-platform user
reported any tolerance-related issue since PR #27 merge on
2026-06-12.

**Cumulative incidents S9-S12** :
- PR #27 scope : 0
- Off-scope cross-referenced : 1 (I-S10-1, cache poisoning in
  bench-harness-full, no impact)

---

## 6. Backfill long-term spot-check

**Sample selected** : `bench/fixtures/pre_g11_ndjson/case_d_gpu_runner.ndjson`
(rotating per AD-S9-S12-6 §c ; case_d completes the 4-case rotation
across S9 → S12).

**Procedure** : identical to S9/S10/S11.

**Verdict** : spot-check PASS.  Note this case_d sample tests the
`source = backfilled_runtime_override` branch with a GPU-runner
override path (the path that was historically the most likely to
silently drop tolerance_used because of the `GpuRunnerOverride`
runtime injection).

After dry-run :
- `tolerance_used.value = 250` (compact form).
- `tolerance_used.unit = "ms"`.
- `tolerance_used.direction = "no_more_than"`.
- `tolerance_used.source = "backfilled_runtime_override"` (NOT
  `gpu_runner_override` — because the backfill correctly distinguishes
  pre-G.11 origin from post-G.11 native).
- `tolerance_used.applied_at = "2026-05-29T17:00:00Z"`.

Golden diff byte-identical.

**4 / 4 case coverage** : S9 → case_a, S10 → case_b, S11 → case_c,
S12 → case_d — full canonical coverage achieved during S9-S12.

---

## 7. Recommendation

**Transition to long-term lights-out monitoring.**

Effective Lundi 10 août 2026 :
- CI nightly job continues, but **no longer in scope of an active
  monitoring report**.  Failures in `pr27-tolerance-used-nightly`
  after 2026-08-07 are routed to the standard bench-infra
  on-call rotation, not to a PR-specific channel.
- The 09:15 UTC daily pinning ritual is **discontinued** as a daily
  practice.  It is replaced by a **weekly canary** every Vendredi
  09:15 UTC, executed by the on-call.  Documented in the runbook
  `docs/runbooks/quarterly-publish.md` § 5.7 (newly added).
- The `g11_round_trip` test remains in CI permanently as a propriété
  forte non-régressable.  The 4 freeze tests in `stability_contract_v17`
  also remain permanently.
- PR #27.1 (the deferred G-XX work) is unblocked and can land at any
  time without re-opening the G.11 gap.

This is the **end of active monitoring** for PR #27.

---

## 8. Closure ceremony summary

**Date / time** : Vendredi 7 août 2026, 14:00 UTC → 17:00 UTC (3h).
**Participants** : @bench-infra-lead, @release-eng-lead, @stability-contract-steward (triple sign-off).
**Companion doc** : [`docs/lessons/monitoring/PR-27-closure-2026-08-07.md`](./PR-27-closure-2026-08-07.md).

### 8.1 Sequence

1. **14:00 UTC — Pre-flight test re-run** : 158 + 16 = 174 PR #27 tests
   (incl. the new `s9_s12_monitoring_chain_integrity` meta-test) +
   broader workspace tests : all green.
2. **14:30 UTC — Cumulative tally review** : nightly 20/20, pinning
   110/110 (100 regular + 10 canary), incidents 0, backfill spot-check
   4/4 cases, quarterly dry-run PASS.
3. **15:00 UTC — Master gap tracker update** : `docs/gaps/master_tracker.md`
   updated to mark G.11 `[closed, verified 2026-08-07]`.
4. **15:30 UTC — Lessons file finalisation** : section 1 (headline)
   and section 10 (conclusion) of `docs/lessons/PR-27-lessons.md`
   filled with final wording (no more `<PLACEHOLDER>`).
5. **16:00 UTC — Roadmap archival** : `docs/roadmap/EFE_A1_PHASE3_ROADMAP.md`
   updated to mark PR #27 row as `[archived, closed 2026-08-07]`.
6. **16:30 UTC — Final canary** : pinning ritual 5x re-run, 5/5 PASS.
7. **17:00 UTC — Triple sign-off** : bench-infra lead + release-eng
   lead + STABILITY contract steward co-sign the closure consolidation
   document.

### 8.2 Verdict

> **G.11 — `tolerance_used` actually populated by the verdict
> aggregator — CLOSED AND VERIFIED, 2026-08-07.**
>
> 12 calendar weeks elapsed from preflight (S1, 18 mai) to closure
> (S12, 7 août).  158 → 174 tests in PR #27 scope.  43 cumulative
> defects surfaced and triaged (4 D-PRE + 25 D-AUDIT mid-S2 +
> 11 D-S3-PN-K + 3 D-FINAL + 0 D-S9-S12).  33 fixed in-session,
> 7 G-XX deferred to PR #27.1, 1 to process docs.  Zero defect
> hidden.  0 regression detected during the 4-week monitoring
> window.  Cumulative pinning ritual S9-S12 : **110/110 PASS**
> (100 regular + 10 canary).

---

## 9. Cross-week notes

- **G-PR27.1 work stream** : PR #27.1 draft was prepared during S11
  ; expected to land mid-août.  Not in scope of this monitoring report.
- **STABILITY contract** : unchanged during S9-S12 (no STABILITY-bump
  PR landed).  The audit-trail row at § 24.9 / § 27.5 is **populated
  at closure** with the date `2026-08-07` and the verdict line.
- **Audit-trail rows** : updated this week (Friday 7 août, step 5
  of the closure sequence).

---

## 10. Signature block

```
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

PR #27 — S12 monitoring report — Week 12 (3-7 août 2026) — CLOSURE WEEK — COMPLETE
Reviewer    : @bench-infra-lead
Date        : 2026-08-07 (Vendredi)
Pinning ritual cumulative : 100/100 PASS (cum S9+S10+S11+S12) + 10/10 canary
Nightly job cumulative     : 20/20 PASS (cum S9+S10+S11+S12)
Incidents (PR #27 scope)   : 0
Incidents (off-scope x-ref): 1 (I-S10-1, no impact)
Backfill spot-check        : PASS (case_d ; 4/4 cases covered cumulatively)
Quarterly publish dry-run  : PASS (companion doc 2026-07-31)
Closure ceremony           : EXECUTED ; triple sign-off achieved
G.11 status                : [closed, verified 2026-08-07]
Recommendation             : transition to long-term lights-out

-----BEGIN PGP SIGNATURE-----

<Placeholder signature pattern per AD-S3-2.>

-----END PGP SIGNATURE-----
-----END PGP SIGNED MESSAGE-----
```

_End of S12 monitoring report._
_End of S9-S12 monitoring sequence._
