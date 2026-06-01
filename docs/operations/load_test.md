# EFE — Load test reproducibility

> **Audience** : engineer who wants to run the D9 ghz load test
> harness against a local or in-cluster `efe-server`.
> **Sprint D9 deliverable**.

---

## What this gives you

Four scenarios — `smoke`, `sustained`, `stress`, `spike` — each
mapped to one or more SLO contracts:

| Scenario | Duration | Concurrency | Target RPS | SLO |
|----------|---------:|-----------:|-----------:|-----|
| `smoke` | 30 s | 1 | 1 | go/no-go (no SLO) |
| `sustained` | 5 min | 4 | ~10 | `D9-LOAD-1`, `-2` |
| `stress` | 2 min | 16 | ~30 | `D9-LOAD-3` |
| `spike` | 90 s | 1→16 step | step | `D9-LOAD-4` |

Output:
* `target/load/load_test_bench.json` — flat eval-bench-format,
  ingestible by `scripts/parse_criterion_report.py`.
* `target/load/load_test_summary.md` — markdown PR-comment friendly.
* `target/load/ghz_<scenario>_report.json` — raw ghz JSON per scenario.

---

## Quick run (local, against a release binary)

```bash
# 1. Build the server.
cargo build --release -p efe-server

# 2. Start it without auth + without idempotency cache (D9.A2).
EFE_LISTEN_ADDR=127.0.0.1:50051 \
EFE_AUTH_ENABLED=false \
EFE_IDEMPOTENCY_TTL_MS=0 \
RUST_LOG=info \
  ./target/release/efe-server &
SERVER_PID=$!

# 3. Wait for bind.
for _ in $(seq 1 60); do nc -z 127.0.0.1 50051 && break ; sleep 1 ; done

# 4. Run all four scenarios.
bash bench/load/run_load_test.sh all \
  --endpoint 127.0.0.1:50051 \
  --output-dir target/load

# 5. Inspect.
cat target/load/load_test_summary.md

# 6. Stop the server.
kill "$SERVER_PID"
```

---

## Run inside an existing cluster

```bash
# 1. Port-forward.
kubectl port-forward -n efe svc/efe 50051:50051 &
PF_PID=$!

# 2. Run scenarios — the same script works against any TCP endpoint.
bash bench/load/run_load_test.sh sustained \
  --endpoint 127.0.0.1:50051 \
  --output-dir target/load

# 3. Stop the port-forward.
kill "$PF_PID"
```

> ⚠ **Don't run `stress` or `spike` against a production cluster
> without coordination** — they will saturate replicas and trigger
> SLO alerts. Use a staging install or scale a single isolated
> replica.

---

## Why the `--no-cache` choice matters (D9.A2)

The `efe-server` has a 10 000-entry LRU cache (idempotency layer).
With our 5 fixed fixtures, the second pass through any client
hits the cache → measured latency = **cache lookup**, not
**Mode-1 compute**.

The `EFE_IDEMPOTENCY_TTL_MS=0` env var disables the cache on the
server side ; this gives the load test a real measurement of
compute throughput.

In production you want the cache (it's a feature). In capacity
benchmarking you want it off. The `run_load_test.sh` script
documents this in its header but doesn't set the env — it's the
**caller**'s job to pass it when starting the server.

---

## SLO interpretation

After the run, the bench-format output contains one entry per
`load_<scenario>::<metric>` key. The aggregator
(`scripts/parse_criterion_report.py`) ingests these and evaluates
the four `D9-LOAD-*` SLOs (cf. supplement §1).

| SLO | Pass criterion | Math justification |
|-----|----------------|-------------------|
| D9-LOAD-1 | `load_sustained::p99_ns ≤ 5 s` | log-normal CI sample sizing — n ~3 000 in 5 min gives ±25 % IC on p99 |
| D9-LOAD-2 | `load_sustained::error_ratio ≤ 1 %` | binomial CI — visible at n=3 000 |
| D9-LOAD-3 | `load_stress::throughput_rps ≥ 5` | Little's law — single replica caps at 6.4 RPS, HPA scales beyond |
| D9-LOAD-4 | `load_spike::recovery_seconds ≤ 30` | drain time = Q · S / K with K=2 minimum |

---

## Customising the scenarios

Each scenario lives in `bench/load/ghz_<name>.json`. Feel free to
edit the JSON — but understand the implications:

* Increasing `duration` of `sustained` gives a tighter p99 IC but
  longer CI runtime. The trade-off curve is in supplement §1.2.
* Reducing `concurrency` of `stress` softens the saturation test.
* The `data-file` points at `fixtures/sample_request.json` — five
  pre-baked `ExecuteRequest` JSON payloads. Add more for diversity
  (see `clients/python/efe_tool/scenarios/builtin/` for ideas).

---

## Hooking into CI

The bench-load CI workflow (`.github/workflows/bench-load.yml`)
runs `smoke + sustained` on every PR touching `bench/load/` or
`crates/efe-server/`. The job is `continue-on-error: true` until
3 consecutive main-branch runs PASS — then a separate PR flips it
to fail-closed.

---

## See also

* Architecture: [`docs/bench/d9_docker_helm_load_architecture.md` § 5](../bench/d9_docker_helm_load_architecture.md)
* Math: [`docs/bench/d9_deep_analysis_supplement.md` § 1](../bench/d9_deep_analysis_supplement.md)
* Soak test (24 h): [`bench/soak/`](../../bench/soak/)
* SLO definitions: [`scripts/parse_criterion_report.py`](../../scripts/parse_criterion_report.py) (search `SLO-D9-LOAD`)
