"""SLO-FFI-EQUIVALENCE — 100 fixtures bit-identical against frozen baseline.

Sprint **Φ-C-FFI Completion (D27)** — artefact #11.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 7.

## Mission

Charge ``fixtures/numerical_equiv.json`` (100 fixtures + verdict
attendu pré-calculé par ``scripts/bench/freeze_numerical_equiv.py``)
et vérifie que ``efe.classify_zero_copy(text, catalog_id, seed)``
retourne **bit-identique** au snapshot pour les champs identifiants :

* ``label`` (str)
* ``cone_id`` (str)
* ``rounds_executed`` (u32)
* ``swarm_id`` (str)
* ``catalog_version`` (str)

Le champ ``confidence`` est comparé avec une tolérance ε = 1e-6
(= f32 epsilon ; les arrondis sur la division u32/u32::MAX peuvent
varier de 1 ulp entre plateformes).

Le champ ``latency_ms`` est **ignoré** — non-déterministe par nature.

## Output JSON pour check_slo.py

Le test écrit ``target/ffi/numerical_equiv.json`` :

```json
{
  "slo_compliance": {
    "SLO-FFI-EQUIVALENCE": {
      "measured_pass_count": 100,
      "measured_expected_count": 100,
      "label_mismatches": [],
      "confidence_mismatches": []
    }
  }
}
```

→ ``check_slo.py`` (extension Φ-C-FFI) lit
``measured_pass_count`` + ``measured_expected_count`` et gate.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

import efe

FIXTURES_PATH = Path(__file__).parent / "fixtures" / "numerical_equiv.json"
CATALOG_PATH = Path(__file__).parent / "fixtures" / "catalog.json"

# Tolérance de comparaison confidence : bit-identique post-Φ-C-FFI.B2.
#
# **Audit fix Φ-C-FFI.B6** : pre-fix la tolérance était 1e-6 pour
# absorber le drift f64 (Python) vs f32 (Rust). Avec la fix B2 dans
# `freeze_numerical_equiv.py` (numpy f32 + linear sum), les deux
# côtés calculent en f32 fidèlement → la confidence est bit-identique
# après le cast f32 → Python float (= f64).
#
# Comparaison stricte `==` garantit que toute future régression
# (= changement de formule ou de précision) casse le test
# immédiatement, plutôt qu'un drift silencieux sub-epsilon.
CONFIDENCE_EPSILON = 0.0  # strict bit-identity


@pytest.fixture(scope="module")
def fixtures_payload() -> dict[str, Any]:
    """Load the 100 frozen fixtures + their expected verdicts."""
    if not FIXTURES_PATH.exists():
        pytest.skip(
            f"fixture file missing: {FIXTURES_PATH} "
            "(run `python scripts/bench/freeze_numerical_equiv.py`)"
        )
    return json.loads(FIXTURES_PATH.read_text())


@pytest.fixture(scope="module")
def catalog_path_str() -> str:
    if not CATALOG_PATH.exists():
        pytest.skip(f"catalog fixture missing: {CATALOG_PATH}")
    return str(CATALOG_PATH)


def _compare_verdict(
    fixture: dict[str, Any], verdict: efe.Verdict
) -> tuple[bool, list[str]]:
    """Return (passed, diagnostics_list)."""
    diag: list[str] = []
    expected = fixture["expected"]
    if verdict.label != expected["label"]:
        diag.append(
            f"label mismatch : got {verdict.label!r}, expected {expected['label']!r}"
        )
    if CONFIDENCE_EPSILON == 0.0:
        # **Audit fix B6** : strict bit-identity check post-B2 freeze.
        if verdict.confidence != expected["confidence"]:
            diag.append(
                f"confidence mismatch (strict) : got {verdict.confidence!r}, "
                f"expected {expected['confidence']!r}"
            )
    elif abs(verdict.confidence - expected["confidence"]) > CONFIDENCE_EPSILON:
        diag.append(
            f"confidence mismatch : got {verdict.confidence}, "
            f"expected {expected['confidence']} (Δ={abs(verdict.confidence - expected['confidence'])})"
        )
    if verdict.cone_id != expected["cone_id"]:
        diag.append(f"cone_id mismatch : got {verdict.cone_id!r}, expected {expected['cone_id']!r}")
    if verdict.rounds_executed != expected["rounds_executed"]:
        diag.append(
            f"rounds mismatch : got {verdict.rounds_executed}, "
            f"expected {expected['rounds_executed']}"
        )
    if verdict.swarm_id != expected["swarm_id"]:
        diag.append(f"swarm_id mismatch : got {verdict.swarm_id!r}, expected {expected['swarm_id']!r}")
    if verdict.catalog_version != expected["catalog_version"]:
        diag.append(
            f"catalog_version mismatch : got {verdict.catalog_version!r}, "
            f"expected {expected['catalog_version']!r}"
        )
    return (len(diag) == 0, diag)


def test_numerical_equivalence_100_fixtures(
    bundle: efe.Bundle,
    catalog_path_str: str,
    fixtures_payload: dict[str, Any],
    tmp_path_factory: pytest.TempPathFactory,
) -> None:
    """SLO-FFI-EQUIVALENCE — 100 fixtures must pass bit-identical."""

    fixtures = fixtures_payload["fixtures"]
    n = len(fixtures)
    assert n == 100, f"expected 100 fixtures, got {n}"

    bundle_path = bundle.fingerprint  # placeholder ; classify_zero_copy
    # takes a path string. The conftest bundle is a synthetic one ; we
    # need an actual path arg. Strategy : use the test fixtures dir as
    # a synthetic bundle path (it canonicalises to a real dir → cache
    # key probe succeeds → load_bundle stub produces a valid PyBundle).
    bp = str(FIXTURES_PATH.parent)

    passes = 0
    label_mismatches: list[str] = []
    confidence_mismatches: list[str] = []
    other_mismatches: list[str] = []

    for fixture in fixtures:
        verdict = efe.classify_zero_copy(
            fixture["text"],
            catalog_id=fixture["catalog_id"],
            bundle_path=bp,
            catalog_path=catalog_path_str,
            seed=fixture["seed"],
        )
        ok, diag = _compare_verdict(fixture, verdict)
        if ok:
            passes += 1
        else:
            for d in diag:
                if d.startswith("label mismatch"):
                    label_mismatches.append(f"{fixture['id']}: {d}")
                elif d.startswith("confidence mismatch"):
                    confidence_mismatches.append(f"{fixture['id']}: {d}")
                else:
                    other_mismatches.append(f"{fixture['id']}: {d}")

    # Emit SLO compliance JSON for check_slo.py.
    out_dir = Path("target") / "ffi"
    out_dir.mkdir(parents=True, exist_ok=True)
    compliance = {
        "slo_compliance": {
            "SLO-FFI-EQUIVALENCE": {
                "measured_pass_count": passes,
                "measured_expected_count": n,
                "label_mismatches": label_mismatches,
                "confidence_mismatches": confidence_mismatches,
                "other_mismatches": other_mismatches,
            }
        }
    }
    (out_dir / "numerical_equiv.json").write_text(
        json.dumps(compliance, indent=2, ensure_ascii=False) + "\n"
    )

    if passes != n:
        # Diagnostic verbeux pour debugging.
        sample = (label_mismatches + confidence_mismatches + other_mismatches)[:10]
        msg = (
            f"\nSLO-FFI-EQUIVALENCE violated : {passes}/{n} passes.\n"
            f"First 10 mismatches:\n"
            + "\n".join(f"  - {s}" for s in sample)
        )
        pytest.fail(msg)


def test_fixture_file_has_100_entries(
    fixtures_payload: dict[str, Any],
) -> None:
    """Pin test : the fixture file must declare 100 entries."""
    assert fixtures_payload["count"] == 100, (
        f"fixture count drift : declared {fixtures_payload['count']}, expected 100"
    )
    assert len(fixtures_payload["fixtures"]) == 100, (
        f"fixture list length drift : got {len(fixtures_payload['fixtures'])}, expected 100"
    )


def test_fixture_ids_are_unique(
    fixtures_payload: dict[str, Any],
) -> None:
    """Pin test : each fixture has a unique id."""
    ids = [f["id"] for f in fixtures_payload["fixtures"]]
    assert len(ids) == len(set(ids)), "duplicate fixture ids detected"


def test_fixture_texts_are_unique(
    fixtures_payload: dict[str, Any],
) -> None:
    """Pin test : each fixture has a unique text (= meaningful coverage)."""
    texts = [f["text"] for f in fixtures_payload["fixtures"]]
    # Allowed duplicates : the deliberate "Repeated text " * 5 case.
    n_unique = len(set(texts))
    assert n_unique >= 99, (
        f"fixture texts not unique enough : {n_unique}/100 distinct"
    )
