# Runbook — OPS master gate failed

> **Severity** : critical
> **Owner** : SRE on-call
> **Linked SLOs** : `SLO-OPS-AVAILABILITY`, `SLO-OPS-E2E-P99-MODE1`,
> `SLO-OPS-CANARY-ROLLBACK-RATE`, `SLO-OPS-LIFECYCLE-INVARIANTS`,
> `SLO-OPS-CLOUD-PARITY`, `SLO-OPS-MTTR`
> **Alerts firing** : `OpsAvailabilityFastBurn1h`, `OpsMasterGateFailNightly`,
> `OpsCanaryStuck`, `OpsInvalidTenantTransition`, …
> **CI workflow** : `.github/workflows/ops-master-gate.yml`
> **Postmortem template** : [`docs/postmortems/_template.md`](../postmortems/_template.md)

---

## 0. TL;DR

The master gate (= consolidation of 105 surface SLOs into 18 master
SLOs) tripped. **A release is blocked** until either :

1. The underlying surface failure is fixed and re-measured PASS, **or**
2. A DT (deferred ticket) is opened and the gate is re-run with
   `--tolerate-skip-with-dt`.

Doctrine **anti-Φ-D.A1** forbids silent skips ; never patch the gate
to ignore a SLO without an explicit DT row.

---

## 1. Triage (≤ 5 min)

```bash
# Pull the latest artefact
gh run download --name ops-master-report --dir /tmp/ops
cat /tmp/ops/master_report.md | head -60
```

Then locate the failing master row :

```bash
jq '.master[] | select(.rolled_status=="FAIL")' /tmp/ops/master_report.json
```

For each FAIL row :

- `id`                : which master SLO is red.
- `severity`          : critical → page ; major → ticket Sev2.
- `children_count`    : how many surface SLOs feed this master row.
- `fail_examples`     : first 3 failing surface SLOs.

---

## 2. Recovery branches

### 2.1 If the failure is on `SLO-OPS-AVAILABILITY`

Run `kubectl get pods -A | grep -v Running` in every region to confirm
the outage scope. Page if cross-region (= Sev1). Single-region :

```bash
# Manual canary rollback for the latest release.
helm history efe-stack -n efe-prod | head -5
helm rollback efe-stack <PREVIOUS_REV> -n efe-prod
```

### 2.2 If the failure is on `SLO-OPS-E2E-P99-MODE1`

Likely cause : GPU OOM or kernel regression.

```bash
kubectl -n efe-prod logs deploy/efe-server --tail=200 \
    | grep -E 'OOM|CUDA|fusion'
```

Pin `kernel_version` to the previous tag in `values-prod.yaml` and
`helm upgrade` ; flag a DT for the regression.

### 2.3 If the failure is on `SLO-OPS-LIFECYCLE-INVARIANTS`

A tenant attempted an invalid FSM transition. The trigger
`tg_tenants_lifecycle_audit` already raised. To inspect :

```sql
SELECT tenant_id, lifecycle_state, lifecycle_attempts,
       lifecycle_last_error, lifecycle_saga_id
FROM tenants
WHERE lifecycle_attempts > 1
  AND lifecycle_state IN ('provisioning_pg','provisioning_redis',
                          'migrating_schema','seeding_fixtures',
                          'compliance_pending','erasure_in_progress');
```

For stuck sagas, use the admin override (Sev2 audit entry generated) :

```bash
efe-admin tenant fsm-force \
    --tenant <slug> \
    --to provisioning_pg \
    --rationale "stuck after CI rollback ; valid retry edge"
```

### 2.4 If the failure is on `SLO-OPS-CANARY-ROLLBACK-RATE`

≥ 10 % rollback rate in 90 days is a release-pipeline signal, not a
production outage. Open a Linear epic ; do **not** page.

```bash
gh run list --workflow ops-master-gate.yml --limit 30
```

Examine the last 10 rollbacks and group by failure category in the
postmortem template.

### 2.5 If the failure is on `SLO-OPS-CLOUD-PARITY`

Variance > 20 % between best and worst cloud. Likely a config drift in
one cloud's `values-prod-<cloud>.yaml`. Diff :

```bash
diff <(yq . charts/efe-stack/values-prod-aws.yaml) \
     <(yq . charts/efe-stack/values-prod-gcp.yaml)
```

Open a fix PR ; defer the gate with a DT only if a 1-week SLA can be
held.

---

## 3. Decision tree

```
                 master gate FAIL
                        │
              ┌─────────┴─────────┐
              │                   │
        SILENT-SKIP             FAIL (real)
              │                   │
          → never tolerate ─── page Sev1 if critical
          → file DT + reopen      │
          → re-run with -tolerate ↓
                            mitigate via §2.x
                                  │
                            verify-fix re-run
                                  │
                              gate PASS
                                  │
                            postmortem (≤ 48 h)
```

---

## 4. Verification of recovery

1. Re-run the workflow :
   `gh workflow run ops-master-gate.yml -f branch=main`
2. Wait for the `Hard gate` step to print `Master gate : ✅ PASS`.
3. Compare the new `master_report.md` against the previous one — look
   for any regressed SLO that now skips.

---

## 5. Postmortem

If the gate failed in **production** (= not in a PR), open the
postmortem template within 24 h and submit within 48 h. The
postmortem is tracked by `SLO-OPS-POSTMORTEM-VELOCITY` ; missing one
is a Sev2 by itself.

---

## 6. Cross-references

- [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md) § 8 (master SLO consolidation)
- [`scripts/ops/master_gate_check.py`](../../scripts/ops/master_gate_check.py)
- [`scripts/ops/consolidate_all_slos.py`](../../scripts/ops/consolidate_all_slos.py)
- [`bench_baselines/v0.9.0/ops_master_slos.yaml`](../../bench_baselines/v0.9.0/ops_master_slos.yaml)
- [`docs/postmortems/_template.md`](../postmortems/_template.md)

---

## 7. Sprint A1 P1 PR #4 — meta-swarm SLO suite

The master gate consumes the 7 `SLO-A1-META-*` surface SLOs defined in
[`bench_baselines/v0.10.0/meta_swarm_slos.yaml`](../../bench_baselines/v0.10.0/meta_swarm_slos.yaml).
They are produced by `bash scripts/ops/run_suite.sh a1_meta_swarm`
which runs the `three_swarms_kovant_latency` Criterion bench (`--quick`)
and harvests both the criterion estimates tree AND the sidecar
`meta-swarm-extras.json`.

### Common A1 failures

| Offender                                 | Investigation entry-point                              |
|------------------------------------------|--------------------------------------------------------|
| `SLO-A1-META-LATENCY-P99` FAIL           | Reproduce locally : `cargo bench -p efe-meta-swarm --bench three_swarms_kovant_latency`. Compare `target/criterion/.../estimates.json` median + std_dev vs the runner expectation. The 100 ms budget is per [EFE_A1_PHASE1_ARCHITECTURE.md][1] § 9. |
| `SLO-A1-META-CONVERGENCE-ITERS` FAIL     | A regression of `iteration::coupled::CoupledIteration::step_once` likely broke contractivity. Re-run `cargo test -p efe-meta-swarm --offline` ; PR #3 has 12 integration proptests pinning this. |
| `SLO-A1-META-DETERMINISM` FAIL           | A new RNG/clone path leaked non-determinism. Audit `Tensor::pseudo_random` callers and any `rayon::par_iter` ordering. |
| `SLO-A1-META-NON-REGRESSION` FAIL        | Cumulative perf drift > 10 % vs `last_p99_observed_ms`. Either revert or update the baseline at next release (see [EFE_A1_BENCH_TRACKING.md][2] § 4). |

### Soft-launch contract

During the first **3 main runs** post-merge, both the
`bench-meta-swarm-a1` job (in `bench.yml`) and the "A1 meta-swarm
master gate" step (in `ops-master-gate.yml`) are
`continue-on-error: true`.  If you see a `SLO-A1-META-*` row in the
master report during this window, treat it as a **warning only** —
the gate will not block release.  Once stabilised, a follow-up PR
flips them off.  See [EFE_A1_BENCH_TRACKING.md][2] § 6 for the flip
procedure.

[1]: ../EFE_A1_PHASE1_ARCHITECTURE.md
[2]: ../EFE_A1_BENCH_TRACKING.md
