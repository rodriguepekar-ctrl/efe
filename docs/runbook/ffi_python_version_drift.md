# Runbook — FFI Python Version Drift

> **Sprint Φ-C-FFI Completion (D28)** — artefact #27.
> Cf. `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 11 + § 16.

## Symptômes

| Indicateur | Implication |
|---|---|
| `ImportError: undefined symbol: _Py_NotImplementedStruct` | Wheel ABI3 cassée — symbole non-limited utilisé |
| `RuntimeError: type 'Verdict' is not initialized` | Maturin metadata désynchronisée vs runtime |
| `EFE_FFI_WHEEL_LOAD_SECONDS` augmente sur une Python version | Drift de coût d'init cross-version |
| Bench `SLO-FFI-CROSS-PY` < 4/4 passes | Au moins une Python version casse |
| User reports : "ça marchait sur 3.10 mais plus sur 3.13" | ABI3 incomplet OU bump CPython breaking |

## Diagnostic ABI3 chirurgical

### Étape 1 : Scan symbol-level

```bash
# Identifier la wheel offending
ls -la crates/efe-ffi/dist/
# Run ABI3 verify
python scripts/bench/verify_abi3.py crates/efe-ffi/dist/
```

Si le script reporte un symbole `_Py*` blacklisted, c'est un import
de fonction CPython **non-limited**. Localiser :

```bash
nm -D --undefined-only crates/efe-ffi/dist/efe-X.Y.Z-cp310-abi3-*.whl |
    grep '_Py_'
```

### Étape 2 : Identifier la dep qui introduit

```bash
# Build le wheel avec debug + map des symboles aux crates
maturin build --release --strip=false -- --emit=asm -C link-args=-Wl,-Map=/tmp/efe_ffi.map
grep '_Py_NotImplementedStruct' /tmp/efe_ffi.map
```

Souvent : un bump de version `pyo3`, `numpy`, ou `pyo3-async-runtimes`
introduit un nouveau symbole non-limited.

### Étape 3 : Pin la dep coupable

```toml
# crates/efe-ffi/Cargo.toml
pyo3 = "=0.21.0"  # avant : "0.21"
```

Re-build, re-test :

```bash
maturin build --release
python scripts/bench/verify_abi3.py crates/efe-ffi/dist/
```

## Cas "Python 3.X bumped → break"

Quand CPython publie une release majeure (3.13 → 3.14) :

1. **Lancer la matrix test** dès la première CPython RC :
   ```bash
   pyenv install 3.14-dev
   pyenv local 3.14-dev
   pip install efe
   python -c "import efe; print(efe.classify_zero_copy('hi', ...))"
   ```

2. **Si erreur ABI** : ouvrir un ticket `PYO3-COMPAT-3.14` et bumper
   `pyo3 = "0.22"` (ou la version qui supporte 3.14).

3. **Re-build matrix** : tag `vX.Y.Z+pre-3.14` et publish.

4. **Reporter PR-gate** : extend la matrix `ffi-test.yml` pour
   inclure 3.14 (= `python: ["3.10", "3.11", "3.12", "3.13", "3.14"]`).

## Cas "wheel locale OK, prod casse"

* Vérifier `ffi-publish.yml` step `verify_abi3` : était-il vert au tag ?
* Comparer la wheel locale (= `maturin develop --release`) avec celle
  publishée (= `maturin build --release --manylinux 2_17`) — la
  différence est manylinux container (= glibc bound).
* Si le manylinux container introduit un symbole, bumper manylinux
  baseline ou pin la version exacte du container.

## Mitigation immediate

```bash
# Yank la wheel défectueuse
twine delete efe==X.Y.Z --repository <private_pypi>
# OU plus safe : publish une 0.X.Y.post1 fixe
maturin build --release  # post-fix
twine upload dist/*.whl
```

## Long-term

* Pin test `test_phi_c_ffi_check_slo_parses_threshold_pass_count` 
  enforce le pattern Φ-D.A1 anti-régression.
* Sigstore Rekor lookup : auditable trail de quelle wheel était
  publiée à quelle date → on peut reverter publish.
* SBOM CycloneDX : permet d'identifier exactement quelle version
  d'une dep transitive est embeddée.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Wheel casse localement | dev concerné | 4 h |
| Wheel casse en CI publish | #efe-build Slack | 24 h |
| Wheel casse en prod (= user impact) | #efe-incidents PagerDuty | 1 h |

## Liens

* Archi § 11 : `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`
* SLO : `SLO-FFI-ABI3`, `SLO-FFI-CROSS-PY`
* Scripts : `scripts/bench/verify_abi3.py`
* PEP 384 (Limited API) : <https://peps.python.org/pep-0384/>
* PyO3 ABI3 docs : <https://pyo3.rs/v0.21.0/building_and_distribution#abi3>
