# Quarterly publish runbook

**Status** : LIVE
**Owner**  : @bench-infra-lead (primary) + @release-eng-lead (cosign segment)
**Cadence** : quarterly (Q1 cible 28 mars, Q2 cible 27 juin, Q3 cible 25 septembre, Q4 cible 19 décembre)
**Last revision** : 2026-08-07 (PR #27 closure ceremony — added § 5.7 weekly canary)

---

## 1. Purpose

The quarterly publish is the public-facing release of EFE
bench-platform data : a signed bundle (PDF + JSON + cosign signature)
covering 90 days of metrics across the 35 CB scenarios + S-suite +
cluster-compat matrix.

This runbook documents the **standard sequence** for the real publish
event AND the **dry-run** sequence for pre-flight validation.

---

## 2. Pre-flight checks

Before any publish or dry-run :

1. **Workspace freeze** : `git status` clean, `git log --oneline -1`
   recorded.  No untracked files.
2. **Full test suite green** : `cargo test --workspace --quiet` exits
   0 with zero failures.  Test count delta vs previous publish should
   be ≥ 0 (no regression).
3. **Pinning canary** : `cargo test --test g11_round_trip` 5x
   consecutive PASS.

---

## 3. Bundle generation

```bash
bash scripts/quarterly-publish.sh \
    --quarter YYYY-QN \
    --output /tmp/quarterly-YYYY-QN-bundle \
    [--dry-run --no-push]   # for dry-run mode
```

Stages :

1. Load scenario catalogue (35 entries).
2. Aggregate post-G.11 NDJSON over 90 days.
3. Render JSON report.
4. Render PDF report.
5. Compute manifest hash (sha256).

Expected duration : ~ 40 min on `ubuntu-latest`.

---

## 4. Signature segment (release-eng owned)

```bash
cosign sign-blob \
    --bundle /tmp/quarterly-YYYY-QN-bundle/cosign-bundle.json \
    --output-signature /tmp/quarterly-YYYY-QN-bundle/quarterly-report-YYYY-QN.sig \
    --output-certificate /tmp/quarterly-YYYY-QN-bundle/quarterly-report-YYYY-QN.pem \
    --yes \
    /tmp/quarterly-YYYY-QN-bundle/quarterly-report-YYYY-QN.sha256
```

Verify :
```bash
cosign verify-blob \
    --bundle /tmp/quarterly-YYYY-QN-bundle/cosign-bundle.json \
    --certificate /tmp/quarterly-YYYY-QN-bundle/quarterly-report-YYYY-QN.pem \
    --certificate-identity-regexp "https://github.com/.*efe.*" \
    --certificate-oidc-issuer-regexp "https://token.actions.githubusercontent.com" \
    /tmp/quarterly-YYYY-QN-bundle/quarterly-report-YYYY-QN.sha256
# Expected: "Verified OK"
```

---

## 5. Post-publish verification

### 5.1 `tolerance_used` field presence (JSON)

```bash
jq -r '.scenarios[].metrics[] | select(.tolerance_used != null) | .tolerance_used.source' \
    quarterly-report-YYYY-QN.json | sort | uniq -c
```

Expected distribution :
- `yaml` : majority (60-70 % of rows)
- `runtime_override` : ~ 20 %
- `gpu_runner_override` : ~ 5-7 %
- `backfilled_yaml` / `backfilled_runtime_override` : decreasing over time
- `no_tolerance_defined` : with `tolerance_used = null` (separate query)
- `pre_g11_unbackfilled` : **MUST be 0** (any non-zero count is an alert)

### 5.2 PDF rendering sample

Sample 12 cells in the metrics table.  Each must render `<value> <unit>`
with the correct direction symbol (`≤`, `≥`, `in [a, b]`, `=`).
Compact-form integer rendering for whole-number values
(e.g. `42 count` not `42.0 count`).

### 5.3 Audit-reader round-trip

```bash
bash bench/harness/scripts/pretend-audit-reader.sh \
    quarterly-report-YYYY-QN.json --explain --limit 50
```

Expected : 100 % parse rate, no ghost rows (empty `tolerance_used`
combined with non-null `tolerance.source` — the historical G.11 bug).

### 5.4 Idempotency check

Re-run the entire publish pipeline with identical inputs.  Expected :
byte-identical bundle hash + byte-identical cosign signature payload
(apart from the `timestamp` field which is by design wall-clock).

---

## 6. Real publish vs dry-run

| Aspect | Real publish | Dry-run |
|--------|--------------|---------|
| `--dry-run` flag | absent | present |
| `--no-push` flag | absent | present |
| Cosign Rekor entry | created | NOT created |
| CDN upload | yes | no |
| URL | `bench.kovant.io/quarterly/YYYY-QN/` | none — `/tmp/...` only |
| Cleanup at end | retained for 5 years | `rm -rf` immediately |

The S11 quarterly dry-run within the S9-S12 monitoring window of any
PR closing a propriété-forte-non-régressable gap is a **mandatory**
validation gate per audit-prompt § 9 (post-merge monitoring rituals,
S9-S12 model).

---

## 7. Operational rituals

### 7.1 Pre-quarterly checklist (T-7 days)

- [ ] Workspace freeze plan announced in `#bench-platform` Slack
- [ ] Backfill batches verified complete for last 90 days
- [ ] All G-XX in active list reviewed for relevance to surface
- [ ] Bench-infra lead + release-eng lead calendar-blocked for
      publish day

### 7.2 Publish day timeline (typical 4h window)

- T+0:00  pre-flight checks
- T+0:30  bundle generation start
- T+1:10  bundle generation done, PDF rendering start
- T+1:32  PDF rendering done, sample verification start
- T+1:45  sample verification done, cosign signature start
- T+1:53  signature done, audit-reader round-trip
- T+1:57  round-trip done, idempotency re-run start
- T+2:35  idempotency re-run done
- T+2:45  retention check / CDN upload (real publish only)
- T+3:00  announcement in `#bench-public` Slack

### 7.3 Post-publish monitoring

- T+24h   alert-channel sweep
- T+72h   first external-user feedback collected
- T+7d    first soak check : are there any rows where downstream
          consumers report `tolerance_used` parsing errors? expected 0

---

## 5.7 Weekly canary (post-PR-#27, post-2026-08-07)

**Added** : 2026-08-07 during the PR #27 closure ceremony, step 6
(16:45 UTC).
**Rationale** : the daily pinning ritual (5x per day) is appropriate
for the 4-week S9-S12 surveillance window but not sustainable as
long-term lights-out monitoring.  This section formalises the
weekly canary that replaces it.

### Schedule

Every Vendredi 09:15 UTC, the bench-infra on-call (or delegate)
executes :

```bash
for i in 1 2 3 4 5; do
  cargo test --test g11_round_trip --quiet || exit 1
done
echo "weekly canary 5x PASS — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
```

### Failure handling

If the canary fails (exit code non-zero) :

1. **Immediate** : capture the failing test output verbatim ; do not
   re-run silently.  Re-run **once** to disambiguate flake vs
   genuine regression.
2. **If re-run also fails** : page bench-infra lead via PagerDuty
   service `bench-platform-prod`.  Treat as P2 incident.  Open
   issue with title `weekly canary FAIL <date>` and labels
   `bench`, `regression`, `g.11-followup`.
3. **If re-run passes** : record flake in `#bench-incidents` channel
   with timestamp + first-attempt output.  Schedule a follow-up
   investigation within 48h.

### Cumulative tally

Maintained in `docs/runbooks/weekly-canary-log.md` (created at first
canary execution, Vendredi 14 août 2026 09:15 UTC).  One row per
week.  Cumulative count exposed to bench-infra leadership monthly.

### Discontinuation criteria

The weekly canary may be discontinued if and only if :
- 12 consecutive months of green canary runs (52 / 52 weekly PASS),
- AND no new G.11-class regression detected by any other surface
  (CI nightly, soak, drill).

Discontinuation requires a STABILITY-bump PR with a triple sign-off
mirroring the PR #27 closure ceremony format.

---

## 8. References

- Closure ceremony PR #27 : `docs/lessons/monitoring/PR-27-closure-2026-08-07.md`
- Audit-prompt § 9 (post-merge monitoring rituals) : `docs/process/audit-prompt.md`
- Pinning test : `bench/harness/tests/g11_round_trip.rs`
- Quarterly bundle generator : `scripts/quarterly-publish.sh`
- Cosign keyless OIDC docs : <https://docs.sigstore.dev/cosign/sign/>
- Master gap tracker : `docs/gaps/master_tracker.md`

---

_Last revision : 2026-08-07 (PR #27 closure ceremony, added § 5.7)._
