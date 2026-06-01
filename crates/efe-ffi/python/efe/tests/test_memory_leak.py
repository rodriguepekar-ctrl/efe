"""SLO-FFI-MEMORY-DRIFT — RSS ≤ 5 % drift after 10 000 classify.

Sprint **Φ-C-FFI Completion (D27)** — artefact #12.
Cf. ``docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md`` § 8.

## Mission

Détecter une fuite mémoire dans la chaîne Python → Rust → Python.
Exécute 10 000 ``classify_zero_copy``, mesure RSS pré/post, et
gate à ≤ 5 % drift.

## Output

```json
{
  "slo_compliance": {
    "SLO-FFI-MEMORY-DRIFT": {
      "measured_ratio": 0.018,
      "rss_baseline_bytes": 89_456_640,
      "rss_final_bytes": 91_086_336
    }
  }
}
```
"""

from __future__ import annotations

import gc
import json
import os
from pathlib import Path

import pytest

import efe

# Skip if `psutil` is not installed — the test is informational on
# minimal environments. CI nightly installs psutil unconditionally.
psutil = pytest.importorskip("psutil")


N_WARMUP = 1_000
N_MEASURE = 10_000

# SLO threshold : 5 % drift.
THRESHOLD_RATIO = 0.05


@pytest.fixture(scope="module")
def proc() -> "psutil.Process":
    return psutil.Process(os.getpid())


@pytest.mark.bench
def test_no_leak_after_10000_classify(
    bundle: efe.Bundle,
    catalog_path: Path,
    proc: "psutil.Process",
) -> None:
    """SLO-FFI-MEMORY-DRIFT : RSS drift ≤ 5 % after 10 000 classify."""

    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        # Warmup : N_WARMUP calls to stabilise allocator state +
        # tokenizer caches.
        for i in range(N_WARMUP):
            client.classify(f"warmup {i % 10}")
        gc.collect()
        rss_baseline = proc.memory_info().rss

        # Measure : N_MEASURE calls.
        for i in range(N_MEASURE):
            # Rotate among 100 texts so any per-text caching gets
            # exercised across the rotation.
            client.classify(f"measure {i % 100}")
        gc.collect()
        rss_final = proc.memory_info().rss

        drift_bytes = rss_final - rss_baseline
        drift_ratio = drift_bytes / max(rss_baseline, 1)
        print(
            f"\n[memory_leak] baseline={rss_baseline / 2**20:.1f} MiB ; "
            f"final={rss_final / 2**20:.1f} MiB ; drift={drift_ratio*100:.2f} %"
        )

        # Emit SLO compliance JSON.
        out_dir = Path("target") / "ffi"
        out_dir.mkdir(parents=True, exist_ok=True)
        (out_dir / "memory_leak.json").write_text(
            json.dumps(
                {
                    "slo_compliance": {
                        "SLO-FFI-MEMORY-DRIFT": {
                            "measured_ratio": drift_ratio,
                            "rss_baseline_bytes": rss_baseline,
                            "rss_final_bytes": rss_final,
                            "n_iterations": N_MEASURE,
                        }
                    }
                },
                indent=2,
            )
            + "\n"
        )

        assert drift_ratio <= THRESHOLD_RATIO, (
            f"SLO-FFI-MEMORY-DRIFT violated : drift {drift_ratio*100:.2f} % > {THRESHOLD_RATIO*100:.0f} %"
        )
    finally:
        client.close()


def test_verdict_refcount_stable(
    bundle: efe.Bundle,
    catalog_path: Path,
) -> None:
    """Pin test : long-lived objects (bundle, client) don't gain extra refs.

    If the FFI layer accidentally retains references to its arguments
    via `Py::clone` instead of `PyRef::borrow`, the refcount of the
    user-side `bundle` / `client` objects climbs by 1 per classify
    call → eventual memory leak.

    100 calls is enough to make a 1-per-call leak observable.
    """
    import sys

    client = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    try:
        # Force-collect to settle baseline refcounts.
        gc.collect()
        rc_bundle = sys.getrefcount(bundle)
        rc_client = sys.getrefcount(client)

        for _ in range(100):
            v = client.classify("hello world")
            # Drop the verdict immediately ; if it leaked an Arc
            # to the bundle, the next refcount would climb.
            del v
        gc.collect()

        rc_bundle_after = sys.getrefcount(bundle)
        rc_client_after = sys.getrefcount(client)
        # Allow ±1 jitter (= depends on the test harness internals,
        # not on our crate).
        assert abs(rc_bundle_after - rc_bundle) <= 1, (
            f"bundle refcount drifted : before {rc_bundle}, after {rc_bundle_after}"
        )
        assert abs(rc_client_after - rc_client) <= 1, (
            f"client refcount drifted : before {rc_client}, after {rc_client_after}"
        )
    finally:
        client.close()
