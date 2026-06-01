# Runbook — FFI Wheel Load Failure

> **Sprint Φ-C-FFI Completion (D28)** — artefact #28.
> Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 9.

## Symptômes

| Indicateur | Action immediate |
|---|---|
| `import efe` lève ImportError | § Diagnostic 1 |
| `import efe` lève `OSError: cannot load shared library` | § Diagnostic 1 |
| `import efe` réussit mais `efe.version()` AttributeError | § Diagnostic 2 |
| `import efe` prend > 5 s (SLO-FFI-COLD-START ≤ 100 ms p95) | § Diagnostic 3 |
| `efe_ffi_wheel_load_seconds` p99 > 1.0 s sur Grafana | § Diagnostic 3 |

## § Diagnostic 1 : ImportError / OSError

### Causes communes (ordre de fréquence)

1. **Wheel installée pour mauvaise arch** (= aarch64 wheel sur x86_64).
   ```bash
   python -c "import platform; print(platform.machine())"  # x86_64 vs aarch64
   pip show efe | grep Location
   file "$(python -c 'import efe; print(efe.__file__)')"
   ```
   * Mitigation : `pip install --force-reinstall --no-cache-dir efe`.

2. **manylinux trop ancien pour le runtime** (= glibc 2.17 sur Ubuntu 24.04 OK,
   sur Alpine ne charge pas).
   ```bash
   ldd --version  # check glibc
   ```
   * Mitigation : utiliser le wheel `musllinux` (si publié) ou un base
     image avec glibc ≥ 2.17 (Debian 11+, Ubuntu 20.04+, RHEL 8+).

3. **Wheel signée par Sigstore mais cert expired** (rare).
   ```bash
   cosign verify-blob --certificate efe.pem --signature efe.sig efe-*.whl
   ```

4. **Wheel corrompue au transfert** :
   ```bash
   sha256sum dist/efe-*.whl
   # Compare to bench_baselines/v0.6.0/ffi_wheel_attestations.json
   ```

## § Diagnostic 2 : AttributeError post-import

### Cause unique

Le `#[pymodule]` `efe_native()` n'a pas registré toutes les fonctions
attendues. Pattern : un dev a ajouté une fonction Rust mais oublié
le `m.add_function!`.

### Diagnostic

```bash
python -c "import efe._native as n; print(dir(n))"
# Doit lister : version, load_bundle, classify_zero_copy, aclassify_zero_copy,
# Verdict, Bundle, ..., 10 exceptions
```

Comparer à la liste canonique :

```bash
pytest scripts/tests/test_efe_ffi_structure.py::test_phi_c_ffi_lib_rs_registers_canonical_surface -v
```

### Mitigation

Re-build avec le commit qui ajoute le `m.add_function!` manquant.
Le pin test bloque le merge à l'avenir.

## § Diagnostic 3 : Cold start lent

### Hypothèses

1. **Disk cold** : la wheel est sur un FS lent (NFS, EBS old-gen).
   * Mitigation : `pip install --target=/dev/shm/efe efe` (en RAM).

2. **`sys.path` long** : Python scanne ` ~/.local/lib/python*/site-packages`
   + le venv + les eggs → 100ms+ avant `import efe`.
   * Diagnostic : `python -X importtime -c 'import efe' 2>/tmp/importtime.txt`
   * Mitigation : `python -S` (= no `site` module), réorganiser sys.path.

3. **`numpy` cold init** : la première fois que numpy charge un BLAS
   provider, ça peut prendre 100-200 ms.
   * Mitigation : `import numpy` avant de mesurer.

4. **Tokio runtime init lent** : 2 workers spawn = quelques ms, mais
   sur container CPU-limited ça peut grimper.
   * Diagnostic : `RUST_LOG=tokio=debug python -c 'import efe'`
   * Mitigation : reduce tokio workers (`EFE_FFI_TOKIO_WORKERS=1`).

5. **Sigstore validation à l'import** (= future post-D29).
   * Diagnostic : `EFE_DISABLE_SIGSTORE_VERIFY=1 python -c 'import efe'`
   * Mitigation : skip-verify pour cold-path use cases.

### Diagnostic step-by-step

```bash
# 1. Lance le test cold start strict
pytest crates/efe-ffi/python/efe/tests/test_cold_start.py -v -s
# Output attendu : p95 < 100 ms

# 2. Si fail, profile detail
python -X importtime -c 'import efe' 2>&1 | sort -k 1 -n | tail -20
# Identifie les modules qui prennent > 10 ms

# 3. Check Prometheus histogram
curl http://localhost:9100/metrics | grep 'efe_ffi_wheel_load_seconds'
# Look at the p99 bucket — should be < 0.1 s
```

## Mitigation immediate (= user-facing)

Si un user signale `pip install efe ; import efe` lent ou cassé :

```bash
# 1. Diagnose
ssh <user_host>
python --version
python -c "import platform; print(platform.machine())"
pip show efe

# 2. Force-reinstall correct wheel
pip uninstall efe -y
pip install --force-reinstall --no-cache-dir efe==<known_good>

# 3. Si toujours cassé, file un ticket avec la sortie de :
python -X importtime -c 'import efe' 2>&1 > /tmp/importtime.txt
attach /tmp/importtime.txt to ticket
```

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Single-user error (= wheel reinstall fix) | self-service | n/a |
| Multi-user error (= same wheel) | #efe-support Slack | 4 h |
| All users on a Python version | #efe-incidents PagerDuty | 1 h |
| Wheel publish blocked en CI | #efe-build | 24 h |

## Liens

* Archi § 9 : `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-FFI-COLD-START`, `SLO-FFI-CROSS-PY`
* Test : `crates/efe-ffi/python/efe/tests/test_cold_start.py`
* Instrument : `efe_ffi_wheel_load_seconds{python_version}`
