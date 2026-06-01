# M26 Bench Scenarios

Production-scale benches required by the M26 GA criteria.

| Scenario | Scale | Duration | Pass criteria |
|---|---|---|---|
| `bench_50tenants_5concurrent` | 50 × 5 = 250 microVMs | 4 h | provisioning P99 < 5 s, failure rate < 1 %, 0 escapes |
| `bench_approval_storm` | 100 simultaneous approvals | 30 min | all within SLA, no token race wins |
| `bench_cost_cap_thrash` | 50 runs × $0.01 cap | 10 min | all → COST_CAPPED < 60 s, no overspend > 5 % |
| `bench_audit_chain_under_load` | 1000 events/s | 1 h | append P99 < 50 ms, verify OK |
| `bench_pod_crash_recovery` | controller crash every 30 s | 30 min | reaper recovers all orphans < 5 min |

Run :

```bash
EFE_API_BASE=https://staging.efe.example.com \
EFE_BENCH_TOKEN=<token> \
pytest bench/m26/ -v
```

Outputs : `bench_baselines/v2.1.0/baseline.json` (compared against M25
baseline ; ≥ 10 % regression blocks the release).
