# SDK Quartet Contract Test Runner

Sprint **Φ-C-SDK Completion (D30)** — artefacts #11-#17.
Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 5.

## Mission

Verify the **D-SDK-1 doctrine** : for a triplet `(text, catalog_id, seed)`
fixed, the 4 SDKs (Python + Go + TS + Java) return **bit-identical**
`Verdict` objects on 8 identifying fields.

Failure mode = SLO-SDK-CONTRACT violation = CI red.

## Quickstart (local, mock mode)

```bash
cd sdk/contract-runner
# Generate fixtures (= idempotent, deterministic)
python3 generate_fixtures.py

# Run all 4 runners + comparator
EFE_CONTRACT_MOCK=1 docker-compose up --abort-on-container-exit \
    --exit-code-from runner-compare

# Inspect output
cat shared/contract.json | jq .slo_compliance
```

Expected output : `measured_pass_count: 400, measured_expected_count: 400`.

## Architecture

```
┌──────────────────┐   ┌──────────────────┐   ┌──────────────────┐   ┌──────────────────┐
│  runner-py       │   │  runner-go       │   │  runner-ts       │   │  runner-java     │
│  (Python 3.12)   │   │  (Go 1.22)       │   │  (Node 20)       │   │  (JDK 21)        │
│                  │   │                  │   │                  │   │                  │
│  reads:          │   │  reads:          │   │  reads:          │   │  reads:          │
│   /fixtures/     │   │   /fixtures/     │   │   /fixtures/     │   │   /fixtures/     │
│  writes:         │   │  writes:         │   │  writes:         │   │  writes:         │
│   /shared/out_py │   │   /shared/out_go │   │   /shared/out_ts │   │   /shared/out_*  │
└─────────┬────────┘   └─────────┬────────┘   └─────────┬────────┘   └─────────┬────────┘
          │                      │                      │                      │
          └──────────────────────┴──────────┬───────────┴──────────────────────┘
                                            │
                                            ▼
                                  ┌─────────────────────┐
                                  │   runner-compare    │
                                  │   (compare.py)      │
                                  │                     │
                                  │   asserts:          │
                                  │   - bit-identity on │
                                  │     8 fields        │
                                  │   - confidence Δ <  │
                                  │     1e-6            │
                                  │                     │
                                  │   writes:           │
                                  │    /out/contract.json│
                                  └─────────────────────┘
```

## Mock vs Live mode

| Mode | `EFE_CONTRACT_MOCK` | Backend | Use |
|------|---------------------|---------|-----|
| Mock | `1` | none ; each runner replicates the D25 stub formula | Local dev, CI |
| Live | `0` (or unset) | `efe-server:50051` (= dockerised) | Pre-release |

The mock mode is bit-identical to the live mode by construction
(= the D25 stub formula is the same in Rust). Live mode validates
the wire format and gRPC stack.

## Adding a new SDK to the quartet

1. Implement a runner under `runners/<sdk>/`.
2. Add a service entry in `docker-compose.yml`.
3. Add the SDK to `compare.py` `--<sdk>` argument list.
4. Update `SLO-SDK-CONTRACT` `expected_count` (= 100 × N SDKs).
5. Add a pin test in `scripts/tests/test_polyglot_sdk_structure.py`.

## SLO

`bench_baselines/v0.6.0/sdk_slos.yaml::SLO-SDK-CONTRACT` :

```yaml
- id: SLO-SDK-CONTRACT
  comparator: eq
  threshold_pass_count: 400
  expected_count: 400
  source_bench: contract
```

## Runbook

See `docs/runbook/sdk_contract_test_failure.md` for the diagnostic flow
when a contract test fails.
