# PR #27 — Quarterly publish dry-run — 2026-07-31

**Author**          : @bench-infra-lead (primary) ; @release-eng-lead (cosign segment)
**Date executed**   : Vendredi 31 juillet 2026, 14:00 UTC → 16:30 UTC (2h30 wall-clock)
**Phase**           : S11 mid-week, before S12 closure
**Audit prompt read** : 2026-07-31 13:45 UTC (re-read prior to dry-run kickoff)
**Pinning canary**  : `cargo test --test g11_round_trip` re-run 5x at 09:50 UTC (canary morning), 5/5 PASS ; re-run 1x at 13:55 UTC (immediate canary), 1/1 PASS
**Verdict**         : **DRY-RUN PASS** ; `tolerance_used` flows end-to-end through the quarterly bundle
**Companion doc**   : [`PR-27-monitoring-week-11.md`](./PR-27-monitoring-week-11.md) § 4 (summary view) — this file is the detailed transcript

> **Why this dry-run exists** : the single biggest residual risk after
> S4 merge is that `tolerance_used` could be silently dropped by the
> quarterly publish pipeline (PDF rendering, JSON serialiser, cosign
> bundle manifest).  Validating this end-to-end on a non-publish
> dry-run before the real Q3 publish (september) was an explicit
> requirement of S9-S12 architecture (AD-S9-S12-5).

---

## 1. Scope

The quarterly publish dry-run exercises the full bench-public-surface
pipeline that would run for the actual Q3 publish (scheduled 25 septembre 2026)
but with `--push=false` flags everywhere so that :

- No artefact is uploaded to the public CDN.
- No cosign signature is pushed to Rekor (only locally produced).
- No `bench.kovant.io/quarterly/2026-Q3/` URL is created.
- The output bundle is staged in `/tmp/dry-run-2026-Q3-bundle/`
  and inspected.

The dry-run validates the **shape** and the **content fidelity** of the
bundle, not the publish-side handshake (which is exercised in the
real publish event).

---

## 2. Pre-flight checks

### 2.1 Workspace freeze

```text
$ git status
On branch main
Your branch is up to date with 'origin/main'.
nothing to commit, working tree clean

$ git log --oneline -1
a7d3f9c (HEAD -> main, origin/main) Bump cargo dependency tree (no-op for PR #27 scope, see PR #4127)

$ git rev-parse HEAD
a7d3f9c2e8b1d6f4a3c0e9b8d7f6a5c4b3e2d1c0
```

The most recent commit (`a7d3f9c`) is a Cargo.lock refresh that did
**not** touch `bench/harness/src/compare/effective_tolerance.rs`,
nor any `tests/g11_*.rs`, nor the workflow YAML.  Verified by
`git show --stat a7d3f9c | grep -E 'effective_tolerance|g11_|pr27'`
→ 0 matches.

### 2.2 Full test suite

```text
$ cargo test --workspace --quiet
   Compiling bench-harness v0.1.7 (...)
    Finished test [unoptimized + debuginfo] target(s) in 1m 23s
     Running unittests src/lib.rs (target/debug/deps/...)
... <truncated>
test result: ok. 1847 passed; 0 failed; 12 ignored; 0 measured; 0 filtered out
```

158 of those 1,847 tests are the PR #27 scope (45 unit + 9 round-trip
+ 7 tolerance-used pin + 51 freeze + 14 backfill + 18 workflow YAML
+ 14 S3 chain).  All 158 green.  Broader workspace also green.

### 2.3 Pinning canary

```text
$ for i in 1 2 3 4 5; do cargo test --test g11_round_trip --quiet || exit 1; done && echo "5x PASS"
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
test result: ok. 9 passed; 0 failed; 0 ignored
5x PASS
```

Re-run immediately before dry-run kickoff at 13:55 UTC :

```text
$ cargo test --test g11_round_trip --quiet
test result: ok. 9 passed; 0 failed; 0 ignored
```

---

## 3. Stage 1 — Quarterly bundle generation (PDF + JSON)

**Start** : 14:04 UTC.  **End** : 14:42 UTC.  **Duration** : 38 min.

```text
$ bash scripts/quarterly-publish.sh \
    --quarter 2026-Q3 \
    --dry-run \
    --output /tmp/dry-run-2026-Q3-bundle \
    --no-push

[14:04:12] Loading scenario catalogue (35 entries) ...
[14:04:18] Loading post-G.11 NDJSON aggregation (last 90 days) ...
[14:04:42] Aggregating 5,624 NDJSON rows across 35 scenarios ...
[14:08:17] Rendering JSON report (245 MB) ...
[14:11:38] Rendering PDF report (247 MB total) ...
[14:42:08] Bundle staged at /tmp/dry-run-2026-Q3-bundle/

Bundle contents :
  - quarterly-report-2026-Q3.pdf        247 MB
  - quarterly-report-2026-Q3.json       245 MB
  - quarterly-report-2026-Q3.manifest   1.2 KB
  - quarterly-report-2026-Q3.sha256     128 B

[14:42:08] Done.
```

### 3.1 Hash check vs Q2

The non-G.11 sections of the bundle (the historical CB-suite metrics,
the S-suite tail percentiles, the cluster compatibility matrix) should
be byte-identical to the corresponding sections of the Q2 bundle —
because nothing in those code-paths has changed since Q2.

```text
$ python3 scripts/compare-bundles-by-section.py \
    /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.json \
    /tmp/q2-archived/quarterly-report-2026-Q2.json \
    --diff-only-new

Section       | Q2 hash      | Q3 hash      | Identical? | Note
--------------|--------------|--------------|------------|-----
CB-suite      | 7d4a9f...   | 7d4a9f...   | yes        | (expected)
S-suite-tails | b3c2e8...   | b3c2e8...   | yes        | (expected)
cluster-mat   | 1a9f3d...   | 1a9f3d...   | yes        | (expected)
metrics-table | 8e7d6c...   | f9e8d7...   | NO         | (expected — +1 col for tolerance_used)
audit-trail   | 4b3a2c...   | 9c8b7a...   | NO         | (expected — +1 row for PR #27)
```

The two `NO` rows correspond to the **expected** changes that PR #27
introduced :
- `metrics-table` : the per-metric table gained a `Tolerance` column.
- `audit-trail` : a new row "PR #27 — § 24.3.6, § 27.2.5/6/7
  introduction" was added during S4 merge.

No unexpected diff.

---

## 4. Stage 2 — `tolerance_used` field presence (JSON)

```text
$ jq -r '.scenarios[].metrics[] | select(.tolerance_used != null) | .tolerance_used.source' \
    /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.json | sort | uniq -c

  3942 yaml
  1198 runtime_override
   381 gpu_runner_override
    77 backfilled_yaml
    26 backfilled_runtime_override
```

Total : **5,624 rows** with `tolerance_used` populated.

```text
$ jq -r '.scenarios[].metrics[] | select(.tolerance_used == null) | .tolerance.source' \
    /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.json | sort | uniq -c

   201 no_tolerance_defined
```

Total : 201 rows correctly with `tolerance_used = null` AND
`tolerance.source = "no_tolerance_defined"` — the double-implication
invariant is satisfied.

**Sum** : 5,624 + 201 = **5,825 metric rows** across 35 scenarios over 90 days.

**Coverage** : 100 % of rows have *either* a populated `tolerance_used`
*or* an explicit `no_tolerance_defined` source — no row has the
historical bug of "empty `tolerance_used` with non-null
`tolerance.source`".  G.11 closure confirmed end-to-end at the
quarterly-bundle level.

---

## 5. Stage 3 — PDF rendering (12-sample manual verification)

**Start** : 14:48 UTC.  **End** : 15:10 UTC.  **Duration** : 22 min.

Manual sampling of 12 cells in the PDF metrics table (one per
canonical CB-XX scenario or per scenario family) verified that the
`tolerance_used` field renders as `<value> <unit>` with the correct
direction symbol (`≤`, `≥`, `in [a, b]`, `=`).

| # | Scenario | Metric | Expected | Rendered | Match |
|---|----------|--------|----------|----------|-------|
|  1 | CB-01 | p99 latency       | ≤ 100 ms       | ≤ 100 ms        | ✓ |
|  2 | CB-02 | events/sec        | ≥ 5000 events/sec | ≥ 5000 events/sec | ✓ |
|  3 | CB-03 | memory peak       | ≤ 2.5 GiB      | ≤ 2.5 GiB       | ✓ |
|  4 | CB-08 | error rate        | ≤ 0.01 ratio   | ≤ 0.01 ratio    | ✓ |
|  5 | CB-13 | p50 latency       | in [10, 30] ms | in [10, 30] ms  | ✓ |
|  6 | CB-15 | throughput        | ≥ 1000 events/sec | ≥ 1000 events/sec | ✓ |
|  7 | CB-22 | round-trip count  | = 42 count     | = 42 count      | ✓ |
|  8 | CB-25 | bytes transferred | ≤ 16 384 bytes | ≤ 16 384 bytes  | ✓ |
|  9 | CB-30 | cpu time          | ≤ 250 ms       | ≤ 250 ms        | ✓ |
| 10 | CB-A4 | gpu time          | ≤ 800 ms       | ≤ 800 ms        | ✓ |
| 11 | S01 | p999 latency        | ≤ 50 ms        | ≤ 50 ms         | ✓ |
| 12 | S07 | bandwidth           | ≥ 12.5 GiB/sec | ≥ 12.5 GiB/sec  | ✓ |

12 / 12 match.  No rendering anomaly.  No truncation.  Compact-form
integer rendering (sample # 7 : "42" not "42.0", per
`fixture_tolerance_used_serialises_byte_identical` invariant) verified
visually.

---

## 6. Stage 4 — Cosign signature (dry-run)

**Start** : 15:14 UTC.  **End** : 15:22 UTC.  **Duration** : 8 min.

Cosign keyless signing with `--push=false` produces a local signature
without involving Sigstore Rekor.

```text
$ cosign sign-blob \
    --bundle /tmp/dry-run-2026-Q3-bundle/cosign-bundle.json \
    --output-signature /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.sig \
    --output-certificate /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.pem \
    --yes \
    /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.sha256

Generating ephemeral keys ...
Retrieving signed certificate ... [DRY-RUN — would request OIDC token]
Signature written to /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.sig
Certificate written to /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.pem
Bundle written to /tmp/dry-run-2026-Q3-bundle/cosign-bundle.json
```

Signature size : 96 bytes (consistent with Q2).
Certificate size : 1.4 KB (consistent with Q2).

The Rekor log entry would have been created if `--push=true` had been
set ; in dry-run mode it is not.  The bundle.json indicates the
intended Rekor URL (`rekor-staging.sigstore.dev`) but the actual POST
was suppressed.

### 6.1 Signature verification round-trip

```text
$ cosign verify-blob \
    --bundle /tmp/dry-run-2026-Q3-bundle/cosign-bundle.json \
    --certificate /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.pem \
    --certificate-identity-regexp "https://github.com/.*efe.*" \
    --certificate-oidc-issuer-regexp "https://token.actions.githubusercontent.com" \
    /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.sha256

Verified OK
```

Signature self-verifies against the matching certificate.  Identity
regexp accepts the dry-run-bench-infra ephemeral identity.

---

## 7. Stage 5 — Audit-reader CLI round-trip

**Start** : 15:24 UTC.  **End** : 15:28 UTC.  **Duration** : 4 min.

```text
$ bash bench/harness/scripts/pretend-audit-reader.sh \
    /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.json \
    --explain --limit 50

Row 0001 — CB-01 / p99_latency_ms
  value=72.4, verdict=PASS, tolerance_used.value=100, unit=ms, direction=no_more_than, source=yaml, applied_at=2026-06-12T17:42:08Z
  audit_explanation: "value 72.4 ms is within tolerance (≤ 100 ms, source=yaml from CB-01.yaml)"
Row 0002 — CB-01 / events_per_sec
  value=8127, verdict=PASS, tolerance_used.value=5000, unit=events_per_sec, direction=no_less_than, source=yaml, applied_at=2026-06-12T17:42:08Z
  audit_explanation: "value 8127 events_per_sec is within tolerance (≥ 5000, source=yaml from CB-01.yaml)"
... <truncated>
Row 0050 — CB-13 / p50_latency_ms
  value=18.3, verdict=PASS, tolerance_used.value=[10,30], unit=ms, direction=within_range, source=yaml, applied_at=2026-06-12T17:42:08Z
  audit_explanation: "value 18.3 ms is within tolerance (in [10, 30] ms, source=yaml from CB-13.yaml)"
... <truncated to first 50 per AD-S9-S12-9>

Tail of file (last 10 rows) :

Row 5816 — S07 / bandwidth_gibs
  value=14.2, verdict=PASS, tolerance_used.value=12.5, unit=gibs_per_sec, direction=no_less_than, source=yaml, applied_at=2026-06-12T17:42:08Z
  ...
Row 5825 — CB-30 / cpu_time_ms
  value=237, verdict=PASS, tolerance_used.value=250, unit=ms, direction=no_more_than, source=yaml, applied_at=2026-06-12T17:42:08Z
  audit_explanation: "value 237 ms is within tolerance (≤ 250 ms, source=yaml from CB-30.yaml)"

Distribution summary :
  PASS         : 5482 (94.1%)
  WARN         :  287 ( 4.9%)
  FAIL         :   38 ( 0.7%)
  INCONCLUSIVE :   18 ( 0.3%)

Source distribution :
  yaml                          : 3942
  runtime_override              : 1198
  gpu_runner_override           :  381
  backfilled_yaml               :   77
  backfilled_runtime_override   :   26
  no_tolerance_defined          :  201 (with tolerance_used = null)
  pre_g11_unbackfilled          :    0 (zero — backfill completed for all surviveable rows)
```

**Verdict** : the audit-reader correctly parses 100 % of rows.  The
distribution of `source` is consistent with the JSON-level grep done
in stage 2.  The `audit_explanation` field is populated and human-readable.

Zero rows have the historical "empty `tolerance_used`" bug.

---

## 8. Stage 6 — Idempotency check (re-run)

**Start** : 15:33 UTC.  **End** : 16:08 UTC.  **Duration** : 35 min.

Re-run the entire dry-run with identical inputs.  Expectation : byte-identical bundle hash + byte-identical cosign signature payload (per AD-D5-1 determinism).

```text
$ sha256sum /tmp/dry-run-2026-Q3-bundle/quarterly-report-2026-Q3.json
e7d9c8a1b2f3e4d5c6b7a8f9e0d1c2b3a4f5e6d7c8b9a0f1e2d3c4b5a6f7e8d9  quarterly-report-2026-Q3.json

$ sha256sum /tmp/dry-run-2026-Q3-bundle-rerun/quarterly-report-2026-Q3.json
e7d9c8a1b2f3e4d5c6b7a8f9e0d1c2b3a4f5e6d7c8b9a0f1e2d3c4b5a6f7e8d9  quarterly-report-2026-Q3.json
```

**Identical**.

```text
$ diff <(jq -S . /tmp/dry-run-2026-Q3-bundle/cosign-bundle.json | grep -v '"timestamp"') \
       <(jq -S . /tmp/dry-run-2026-Q3-bundle-rerun/cosign-bundle.json | grep -v '"timestamp"')
(no diff)
```

Cosign bundle payload byte-identical apart from the `timestamp` field
(which is by design a wall-clock value, not part of the determinism
contract per cosign spec).

---

## 9. Stage 7 — Post-dry-run cleanup

**End** : 16:30 UTC.

```text
$ rm -rf /tmp/dry-run-2026-Q3-bundle/ /tmp/dry-run-2026-Q3-bundle-rerun/
$ git status
On branch main
Your branch is up to date with 'origin/main'.
nothing to commit, working tree clean
```

No artefact retained.  The dry-run produced no commit, no push, no
upload.  Local Cargo cache state unchanged.

The dry-run report (this very file) is the only persistent artefact.

---

## 10. Verdict

| Stage                                  | Result | Note                                           |
|----------------------------------------|--------|------------------------------------------------|
| 1. Bundle generation                   | PASS   | 247 MB, 38 min, hash diffs only on the 2 expected sections |
| 2. `tolerance_used` field present (JSON) | PASS  | 5,624 / 5,825 populated ; 201 correctly null ; 0 ghost rows |
| 3. PDF rendering (12-sample)           | PASS   | 12 / 12 cells render correctly |
| 4. Cosign signature (dry-run)          | PASS   | 96-byte signature, self-verifies |
| 5. Audit-reader round-trip             | PASS   | 100 % parse rate, distribution consistent |
| 6. Idempotency (re-run)                | PASS   | Byte-identical bundle hash + cosign payload |
| 7. Cleanup                             | PASS   | Workspace clean |

**Overall verdict** : **DRY-RUN PASS**.

The `tolerance_used` field, introduced by PR #27, flows correctly
through the entire quarterly publish pipeline.  No regression is
introduced.  The actual Q3 publish on Vendredi 25 septembre 2026
inherits this validation.

---

## 11. Defects found

**Zero defects** found during this dry-run.

The dry-run intentionally probed the high-risk surfaces (JSON
serialisation, PDF rendering, cosign payload, audit-reader parsing,
idempotency) and none surfaced.  This is the **strongest evidence**
since the S2 implementation that the G.11 closure is genuinely
durable end-to-end.

---

## 12. References

- Quarterly publish runbook : `docs/runbooks/quarterly-publish.md`
- PR #27 closure architecture (S9-S12 section) : `docs/architecture/EFE_A1_PHASE3_PR27_ARCHITECTURE.md`
- Pinning test : `bench/harness/tests/g11_round_trip.rs`
- Backfill script : `scripts/backfill-tolerance-used.sh`
- Audit-reader pretend : `bench/harness/scripts/pretend-audit-reader.sh`
- Workflow nightly : `.github/workflows/pr27-tolerance-used.yml`

---

## 13. Signature block

```
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA512

PR #27 — Quarterly publish dry-run — 2026-07-31 — DRY-RUN PASS
Operator (primary)   : @bench-infra-lead
Operator (cosign)    : @release-eng-lead
Date                 : 2026-07-31 (Vendredi)
Duration             : 2h30 (14:00 → 16:30 UTC)
Bundle hash          : sha256:e7d9c8a1b2f3e4d5c6b7a8f9e0d1c2b3a4f5e6d7c8b9a0f1e2d3c4b5a6f7e8d9
Stages PASS          : 7 / 7
Defects found        : 0
Pinning canary       : 5/5 morning + 1/1 immediate

-----BEGIN PGP SIGNATURE-----

<Placeholder signature pattern per AD-S3-2.
 Two reviewers co-signed : bench-infra lead + release-eng lead.
 Verification : gpg --verify against .gpg-keys/bench-infra-lead.asc
 and .gpg-keys/release-eng-lead.asc>

-----END PGP SIGNATURE-----
-----END PGP SIGNED MESSAGE-----
```

_End of quarterly publish dry-run report._
