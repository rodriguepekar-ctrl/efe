# Runbook — FFI Performance Regression

> **Sprint Φ-C-FFI Completion (D28)** — artefact #29.
> Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 14 + § 20.

## Symptômes

| Indicateur | Implication |
|---|---|
| `SLO-FFI-LATENCY-P99` fail en nightly | p99 > 40 ms |
| `SLO-FFI-SPEEDUP` fail | native ≤ 5× gRPC violation |
| `efe_ffi_classify_seconds` p99 augmente > 50 % sur 7 jours | drift cumulé |
| User reports : "ça rame depuis le dernier deploy" | Suspect un commit récent |
| `efe_ffi_bundle_cache_hits_total{outcome="miss"}` rate > 50 % | Cache invalidé trop souvent |

## Investigation chirurgicale

### Étape 1 : Quantifier l'amplitude

```bash
# Pull les dernières mesures
curl http://prometheus.efe.svc/api/v1/query?query='histogram_quantile(0.99,sum(rate(efe_ffi_classify_seconds_bucket[5m])) by (le))' | jq

# Compare à la baseline (snapshot pre-regression)
diff <(curl ...baseline) <(curl ...now)
```

### Étape 2 : Identifier le bottleneck par étage

L'archi § 4.1 décompose `T_classify = T_marshal_in + T_kernel + T_marshal_out`.

```bash
# Histograms par étage
curl http://prom/api/v1/query?query='efe_ffi_marshal_in_seconds_sum / efe_ffi_marshal_in_seconds_count'
curl http://prom/api/v1/query?query='efe_ffi_kernel_seconds_sum / efe_ffi_kernel_seconds_count' 
curl http://prom/api/v1/query?query='efe_ffi_marshal_out_seconds_sum / efe_ffi_marshal_out_seconds_count'
```

Trouver le ratio coupable :

```text
T_marshal_in:  10 µs  → 1 ms   (×100 regression sur marshal — investigate marshal path)
T_kernel:      30 ms  → 30 ms  (stable)
T_marshal_out: 5 µs   → 5 µs   (stable)
```

### Étape 3 : Bisect via Criterion

```bash
# Criterion bench history
ls target/criterion/end_to_end_stub_no_python/history/

# Compare last green to current
cargo bench -p efe-ffi --bench ffi_overhead -- --save-baseline pre-regress
git checkout <last_known_green>
cargo bench -p efe-ffi --bench ffi_overhead -- --save-baseline green
# Now compare
cargo bench -p efe-ffi --bench ffi_overhead -- --baseline green
```

### Étape 4 : Bisect git pour cibler le commit

```bash
git bisect start
git bisect bad HEAD
git bisect good <last_green_tag>
# Auto-script :
git bisect run sh -c "
  maturin develop --release -m crates/efe-ffi/Cargo.toml &&
  pytest crates/efe-ffi/python/efe/tests/bench/native_vs_grpc.py::test_native_p99_under_budget
"
```

## Hypothèses fréquentes

### H1 : Cache hit rate effondré

```bash
curl http://prom/api/v1/query?query='rate(efe_ffi_bundle_cache_hits_total{outcome="hit"}[5m]) / rate(efe_ffi_bundle_cache_hits_total[5m])'
# Si < 0.80, le cache est sub-effective
```

Causes :
* `bundle_path` varie par appel (= chemins avec timestamp).
* Le bundle est swap`d fréquemment (= ops déploient toutes les 10 min).
* `LRU_CAPACITY = 4` insuffisant pour le workload (= 5+ bundles distincts).

Mitigation : Stabiliser les paths côté ops, ou bumper `LRU_CAPACITY`
(decision D16 § 19).

### H2 : GIL contention introduite

Cf. `ffi_gil_contention.md` runbook.

### H3 : Dep transitive bumpée → SIMD perdu

Une dep (`numpy`, `pyo3`, `tokio`) bumpée peut désactiver un path
optimisé. Diagnostic :

```bash
# Check transitive lockfile diff
git diff <last_green_tag> -- Cargo.lock | grep -A 2 -B 2 'numpy\|pyo3\|tokio'
```

### H4 : Tokio runtime saturated (path async)

```bash
RUST_LOG=tokio=debug python -c '
import asyncio, efe
async def main():
    for _ in range(1000):
        await efe.aclassify_zero_copy("hi", "router-v1", "/etc/efe/bundles/active", "/etc/efe/catalogs/router-v1.json")
asyncio.run(main())
' 2>&1 | grep -c 'blocking_threads' # high count → saturation
```

Mitigation : `EFE_FFI_TOKIO_WORKERS=4` (default 2).

### H5 : pinned host pool fragmenté (GPU path)

Si `--features cuda` + GPU path actif :

```bash
curl http://prom/api/v1/query?query='efe_gpu_pinned_host_pool_bytes'
# Si > 1 GiB sur un workload qui devrait tenir en 200 MiB, fragmentation suspectée
```

Mitigation : `note_arena_reset` doit être appelé à chaque request fin
(cf. `EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § Φ-D.B1).

## Mitigation immediate

```bash
# 1. Revert le commit identifié par bisect
git revert <commit_sha>
git push origin main

# 2. Trigger un wheel re-publish
gh workflow run ffi-publish.yml -f tag=v<patch>.1

# 3. Verify la nouvelle wheel passe SLO
gh workflow run ffi-bench.yml -f tag=v<patch>.1
```

## Long-term

* Le `ffi-bench.yml` nightly catch les régressions ≥ 10 % avant
  qu'elles atteignent prod.
* Le Criterion bench `ffi_overhead` history mesure 5 étages
  individuellement → un slowdown par étage est traçable.
* Le pin test `test_phi_c_ffi_check_slo_parses_threshold_pass_count`
  garantit que les SLO restent gating (= jamais silently skipped).

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Bench nightly drift < 10 % | dev concerné par PR | 24 h |
| Bench nightly drift ≥ 10 % | #efe-perf Slack | 24 h |
| Bench tag-gate FAIL | block publish | immediate |
| Prod user-reported slowdown ≥ 2× | #efe-incidents PagerDuty | 1 h |

## Liens

* Archi § 14 : `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-FFI-LATENCY-P99`, `SLO-FFI-SPEEDUP`
* Bench Rust : `crates/efe-ffi/benches/ffi_overhead.rs`
* Bench Python : `crates/efe-ffi/python/efe/tests/bench/native_vs_grpc.py`
* Instruments : `efe_ffi_classify_seconds`, `efe_ffi_kernel_seconds`,
  `efe_ffi_marshal_in_seconds`, `efe_ffi_marshal_out_seconds`,
  `efe_ffi_bundle_cache_hits_total`
