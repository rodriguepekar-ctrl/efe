# Runbook — FFI GIL Contention

> **Sprint Φ-C-FFI Completion (D28)** — artefact #26.
> Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 13.

## Symptômes

| Indicateur | Seuil d'alerte | Implication |
|---|---|---|
| `efe_ffi_gil_held_seconds{op="classify"}` p99 > 0.05 s | WARN | GIL tenu > 50 ms par classify |
| `efe_ffi_gil_released_seconds / efe_ffi_classify_seconds` < 0.70 | CRIT | Le ratio GIL release tombe sous 70 % |
| Latence apparente de Python threads concurrents 10× attendue | CRIT | Threads cooperate-bound |
| Bench `test_gil_release.py::test_gil_released_4thread_speedup_proxy` < 3.0× | CRIT | SLO-FFI-GIL-RELEASED violated |

## Hypothèses chirurgicales

1. **A : Une opération PyO3 oublie le `py.allow_threads`**
   * Probabilité : haute si un nouveau `#[pyfunction]` a été mergé récemment.
   * Vérification : `git log --since='1 week ago' --grep '#\[pyfunction\]' crates/efe-ffi/src/`
   * Mitigation : Wrap la heavy compute dans `py.allow_threads(move || {...})`.

2. **B : Le `py.allow_threads` re-acquire le GIL pour appeler PyO3 helpers**
   * Probabilité : moyenne. Pattern : appel `Python::with_gil` inside `allow_threads`.
   * Vérification : `grep -rn 'with_gil' crates/efe-ffi/src/ | grep -v test`
   * Mitigation : Réorganiser pour minimiser le `with_gil` ré-entrant.

3. **C : `tokio::task::spawn_blocking` thread-pool saturated**
   * Probabilité : moyenne sur les paths async.
   * Vérification : `RUST_LOG=tokio=debug` et grep `blocking_threads`.
   * Mitigation : Augmenter le worker pool tokio (env `EFE_FFI_TOKIO_WORKERS=8`).

4. **D : Pin GIL via objets PyObject longue durée**
   * Probabilité : faible. Pattern : `Bound<'py, PyAny>` stockés dans une struct.
   * Vérification : `grep -rn 'Bound<' crates/efe-ffi/src/ | grep -v fn`
   * Mitigation : Stocker `Py<PyAny>` au lieu de `Bound`.

## Procédure de diagnostic

### Étape 1 : Reproduire localement

```bash
# Boot maturin develop si pas déjà
cd crates/efe-ffi && maturin develop --release

# Lance le test GIL release strict
pytest python/efe/tests/test_gil_release.py -v -s

# Output attendu — speedup ≥ 3.0× (= R ≈ 0.87)
# Output crit — speedup < 2.0× (= R < 0.50)
```

### Étape 2 : Profile détaillé

```bash
# Enregistre les histogrammes GIL pendant 30 s de charge
curl http://localhost:9100/metrics | grep 'efe_ffi_gil' > /tmp/gil_baseline.txt
# (... lance N classify ...)
curl http://localhost:9100/metrics | grep 'efe_ffi_gil' > /tmp/gil_post.txt
diff /tmp/gil_baseline.txt /tmp/gil_post.txt
```

### Étape 3 : Bisect le commit fautif

```bash
git bisect start
git bisect bad HEAD
git bisect good <last_known_green_tag>
# Au moment où le bisect demande, lance :
pytest crates/efe-ffi/python/efe/tests/test_gil_release.py::test_gil_released_4thread_speedup_proxy
# git bisect good / bad selon résultat
```

## Mitigation immediate (= revert + ship)

Si l'incident est en prod et qu'un revert immédiat est nécessaire :

```bash
# Revert le commit identifié
git revert <commit_sha>
# Rebuild + redeploy
cd crates/efe-ffi && maturin build --release
# Publish la wheel patched
gh release create v<patch>.1 dist/*.whl
```

## Long-term : éviter les régressions

* Le pin test `test_gil_release.py::test_gil_released_4thread_speedup_proxy`
  est en **PR-gate** (ffi-test.yml). Il bloque tout merge qui dégrade le
  ratio sous 3.0×.
* Ajouter une alerte Prometheus `FFI_GIL_HELD_HIGH` (cf. § 12 archi)
  qui pend WARN dès que le ratio held/total dépasse 30 % sur 5 min.

## Escalation

| Sévérité | Qui notifier | Délai |
|---|---|---|
| WARN (held > 30 % sur 5 min) | #efe-perf Slack | 24 h |
| CRIT (test_gil_release fail) | #efe-incidents PagerDuty | 1 h |
| CRIT (latence p99 user > 10×) | #efe-incidents + Eng lead | immediate |

## Liens

* Architecture : `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 13
* SLO : `bench_baselines/v0.6.0/ffi_slos.yaml::SLO-FFI-GIL-RELEASED`
* Test : `crates/efe-ffi/python/efe/tests/test_gil_release.py`
* Instrument : `efe_ffi_gil_held_seconds`, `efe_ffi_gil_released_seconds`
