"""SLO-FFI-DROPIN — EfeNativeClient == EfeClient (gRPC) verdict equivalence.

Sprint **Φ-C-FFI Completion (D27)** — artefact #15.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 15 + § 20.

## Mission

Vérifier que ``EfeNativeClient`` (PyO3) produit le même verdict que
``EfeClient`` (gRPC) sur 1+ requests fixées. Cette propriété autorise
le drop-in migration (= un dev change deux lignes d'import et son code
continue à fonctionner pareil).

## Skip strategy

Le test SKIPS si :

* ``efe_tool`` (= la lib gRPC client) n'est pas installée, OU
* ``EFE_HOST`` env var n'est pas définie (= pas de gRPC backend joignable).

Sur les runners production / staging, ces deux conditions sont vraies
→ le test enforce. Sur un fresh clone, le test skip gracefully.

## Output

```json
{
  "slo_compliance": {
    "SLO-FFI-DROPIN": {
      "measured_pass_count": 1,
      "measured_expected_count": 1
    }
  }
}
```

Le test n'exécute qu'une seule comparaison pour limiter la surface
d'erreur du gRPC backend en CI. Un test plus exhaustif (= 10
fixtures) sera ajouté en D29 quand le gRPC ↔ native cluster sera
provisionné.
"""

from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path

import pytest

import efe

GRPC_AVAILABLE = importlib.util.find_spec("efe_tool") is not None
HOST_ENV_SET = bool(os.environ.get("EFE_HOST"))

FIXED_TEXT = "I want to dispute the charge"
FIXED_CATALOG = "router-v1"
FIXED_SEED = 42


@pytest.mark.skipif(
    not GRPC_AVAILABLE,
    reason="efe_tool gRPC client not installed",
)
@pytest.mark.skipif(
    not HOST_ENV_SET,
    reason="EFE_HOST env var not set (= no gRPC backend reachable)",
)
def test_native_and_grpc_produce_same_verdict(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """SLO-FFI-DROPIN — same verdict via PyO3 and via gRPC."""
    from efe_tool import EfeClient  # type: ignore[import-not-found]

    # gRPC path.
    grpc = EfeClient.from_env()
    grpc_verdict = grpc.classify(
        FIXED_TEXT,
        catalog=FIXED_CATALOG,
        seed=FIXED_SEED,
    )

    # Native path.
    native = efe.EfeNativeClient(bundle, str(catalog_path), seed=FIXED_SEED)
    try:
        native_verdict = native.classify(FIXED_TEXT)
    finally:
        native.close()

    diff: list[str] = []
    if grpc_verdict.label != native_verdict.label:
        diff.append(
            f"label : grpc={grpc_verdict.label!r}, native={native_verdict.label!r}"
        )
    if abs(grpc_verdict.confidence - native_verdict.confidence) > 1e-6:
        diff.append(
            f"confidence : grpc={grpc_verdict.confidence}, "
            f"native={native_verdict.confidence}, "
            f"Δ={abs(grpc_verdict.confidence - native_verdict.confidence)}"
        )
    if grpc_verdict.swarm_id != native_verdict.swarm_id:
        diff.append(
            f"swarm_id : grpc={grpc_verdict.swarm_id}, "
            f"native={native_verdict.swarm_id}"
        )

    passed = len(diff) == 0

    # Emit SLO compliance JSON.
    out_dir = Path("target") / "ffi"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "dropin.json").write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-FFI-DROPIN": {
                        "measured_pass_count": 1 if passed else 0,
                        "measured_expected_count": 1,
                        "diff": diff,
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )

    assert passed, f"SLO-FFI-DROPIN violated:\n  " + "\n  ".join(diff)


@pytest.mark.skipif(
    GRPC_AVAILABLE,
    reason="this skip-pin only runs in environments without efe_tool",
)
def test_dropin_test_is_skipped_when_grpc_unavailable() -> None:
    """Meta : when efe_tool is absent the dropin SLO is expected SKIP.

    Important : we still emit a compliance JSON marker so check_slo.py
    sees ``measured_pass_count: 0, measured_expected_count: 0`` rather
    than treating SLO-FFI-DROPIN as "absent" — that lets a sysadmin
    run check_slo.py without efe_tool and see "SKIPPED (no gRPC)"
    rather than a generic "no data".
    """
    out_dir = Path("target") / "ffi"
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "dropin.json").write_text(
        json.dumps(
            {
                "slo_compliance": {
                    "SLO-FFI-DROPIN": {
                        "measured_pass_count": 0,
                        "measured_expected_count": 0,
                        "skip_reason": "efe_tool gRPC client not installed",
                    }
                }
            },
            indent=2,
        )
        + "\n"
    )
