# Runbook : smoke flake-storm (T106 mitigation)

**Trigger** : smoke-pr flake rate > 1 % over 100 PRs (or ≥ 3 consecutive
flakes for the same check on different PRs).

## Phase 0 — Detect

The SRE rotation receives a page from PR #22's `smoke_flake_rate`
recording rule (consumes `source=synthetic_smoke` SLI cells).

```bash
# Compute the rolling flake rate per check over the last 100 PRs.
gh run list --workflow smoke-pr --limit 100 --json conclusion,headSha \
  | jq '[.[] | select(.conclusion=="failure")] | length / 100'
```

## Phase 1 — Identify

Download the artefact bundle for the failing run(s) :

```bash
gh run download <RUN_ID> -n smoke-<RUN_ID>
jq '.checks[] | select(.verdict=="fail") | .id' smoke.json
```

The failing check ID + `error_summary` field localises the root cause.

## Phase 2 — Quarantine

Temporarily exclude the flaky check from the merge gate :

```yaml
# .github/workflows/smoke-pr.yml — temporary patch.
      - name: Run smoke (all 9 checks)
        run: |
          ./target/release/efe-smoke run \
              --chart-version "$EFE_CHART_VERSION" \
              --check S1 --check S2 --check S3 --check S5 --check S6 \
              --check S7 --check S8 --check S9  # S4 quarantined
              # …
```

Open a follow-up incident issue tagged `smoke-flake-quarantine` with :
- Failing check ID
- Last 3 failure run-IDs
- Reproducer (artefact links)
- ETA for fix (≤ 7 calendar days — AD4)

## Phase 3 — Resolve (within 7 days)

Two paths :

### Path A — Fix

Identify the root cause + ship a fix.  Re-enable the check by reverting
the workflow patch.  Confirm 30 consecutive green PRs before closing
the incident.

### Path B — Remove (AD4)

If the check cannot be made stable within 7 days, **remove it**.  This
is a non-negotiable contract requirement.  Steps :

1. Open a PR removing the check from `SmokeCheckId` + `checks/sN_*.rs`.
2. Update STABILITY_CONTRACT v1.5 § 23 with the new taxonomy.
3. Bump STABILITY_CONTRACT to v1.6 (closed-taxonomy change).
4. Notify release-eng + SRE leadership.

⚠ Re-introducing a removed check requires a stability proof :
- 30-PR proptest seed bank green
- 100 historical reruns
- Architecture review

## Phase 4 — Post-mortem

Within 14 days, file a post-mortem doc :
- Timeline
- Root cause
- 5-whys
- Process improvement (e.g. lint rule, contract addition)
- Update to this runbook

Catalogue at `docs/postmortems/smoke-flake-YYYY-MM-DD.md`.

## Reference

- AD4 (zero-tolerance flake policy)
- AD17 (1 retry on early connection-refused)
- T106 (threat model)
- PR #23 § 22 (risk register)
- STABILITY_CONTRACT v1.5 § 23.8 (audit trail)
