"""SLO-FFI-DETERMINISM — 100 identical runs for (text, seed) fixed.

Sprint **Φ-C-FFI Completion (D27)** — artefact #14.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 19 (D15).

## Mission

Une fois ``seed`` fixé et ``text`` fixé, l'output ``Verdict`` doit
être **bit-identique** à chaque appel. Exécute 100 classify sur la
même paire ``(text, seed)`` et vérifie l'égalité stricte de :

* ``label`` (str)
* ``cone_id`` (str)
* ``rounds_executed`` (u32)
* ``swarm_id`` (str)
* ``catalog_version`` (str)
* ``confidence`` (f32, exact equality — pas d'epsilon ici)

Le champ ``latency_ms`` est ignoré.

## Output

```json
{
  "slo_compliance": {
    "SLO-FFI-DETERMINISM": {
      "measured_pass_count": 100,
      "measured_expected_count": 100
    }
  }
}
```
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import efe

N_RUNS = 100

# Fixture text + seed qui exercent le SHA-256 mix du stub D25.
FIXED_TEXT = "I want to dispute the charge on my account"
FIXED_SEED = 42


def test_determinism_strict_100_runs(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """SLO-FFI-DETERMINISM — 100 runs with same (text, seed) → all identical."""

    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=FIXED_SEED)
    try:
        first: efe.Verdict | None = None
        passes = 0
        mismatches: list[str] = []

        for i in range(N_RUNS):
            verdict = client.classify(FIXED_TEXT)
            if first is None:
                first = verdict
                passes += 1
                continue
            if (
                verdict.label == first.label
                and verdict.cone_id == first.cone_id
                and verdict.rounds_executed == first.rounds_executed
                and verdict.swarm_id == first.swarm_id
                and verdict.catalog_version == first.catalog_version
                # confidence exact equality (= same SHA-256 → same u32 → same f32)
                and verdict.confidence == first.confidence
            ):
                passes += 1
            else:
                mismatches.append(
                    f"iter={i}: label={verdict.label!r} conf={verdict.confidence} "
                    f"swarm={verdict.swarm_id!r}"
                )

        # Emit SLO compliance JSON.
        out_dir = Path("target") / "ffi"
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "determinism.json").write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-FFI-DETERMINISM": {
                            "measured_pass_count": passes,
                            "measured_expected_count": N_RUNS,
                            "mismatches": mismatches[:10],
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )

        assert passes == N_RUNS, (
            f"SLO-FFI-DETERMINISM violated : {passes}/{N_RUNS} passes.\n"
            f"First 10 mismatches: {mismatches[:10]}"
        )
    finally:
        client.close()


def test_different_seed_yields_different_verdict(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """Sanity : changing the seed must change the swarm_id.

    If two different seeds produced the same verdict, the seed
    parameter would be ignored — that's a regression.
    """
    a = efe.EfeNativeClient(bundle, str(catalog_path), seed=42).classify("hello")
    b = efe.EfeNativeClient(bundle, str(catalog_path), seed=43).classify("hello")
    assert a.swarm_id != b.swarm_id, (
        f"different seeds produced same swarm_id: {a.swarm_id}"
    )


def test_different_text_yields_different_verdict(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """Sanity : changing the text must change the swarm_id."""
    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        a = client.classify("text alpha")
        b = client.classify("text beta")
        assert a.swarm_id != b.swarm_id, (
            f"different texts produced same swarm_id: {a.swarm_id}"
        )
    finally:
        client.close()
