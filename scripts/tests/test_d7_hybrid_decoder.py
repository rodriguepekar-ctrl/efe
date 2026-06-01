"""Sprint D7 anti-regression tests for the hybrid decoder *policy*.

The hybrid classifier is split into a deterministic policy
(`HybridClassifier._combine`) and an RPC-driven shell
(`HybridClassifier.classify`). These tests pin **the policy** : every
branch of the truth table that the architecture document spells out
must produce the documented outcome.

We deliberately do NOT exercise the RPC path here ; that is covered
by `tests/integration_grpc.rs` once the hybrid path is activated in a
follow-up sprint (cf. architecture D7 §16.2).

Run with :
    python3 -m pytest scripts/tests/test_d7_hybrid_decoder.py -v

Or standalone :
    python3 scripts/tests/test_d7_hybrid_decoder.py
"""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_CLIENT = _ROOT / "clients" / "python"
sys.path.insert(0, str(_CLIENT))

from efe_tool.integrations.hybrid_decoder import (  # noqa: E402
    HybridClassifier,
    HybridResult,
    _raw_cosine_predict,
)


# --------------------------------------------------------------------- #
# Lightweight stand-ins for AnchorCatalog / EfeClient.
# --------------------------------------------------------------------- #


class _ClassDef:
    def __init__(self, name: str, n_sentences: int = 1):
        self.name = name
        self.canonical_sentences = tuple(f"sent-{i}" for i in range(n_sentences))


class _Catalog:
    def __init__(self, anchors: np.ndarray, classes):
        self.anchors = anchors
        self.classes = classes
        self.dim = int(anchors.shape[1])
        self.embedder_fingerprint = "test/embedder@1"
        self.name = "test"
        self.version = "1.0.0"


def _make_catalog():
    """4-class catalog with one row per class, mutually orthogonal."""
    classes = [_ClassDef("fraud"), _ClassDef("bug"), _ClassDef("question"), _ClassDef("billing")]
    anchors = np.eye(4, dtype=np.float32)
    return _Catalog(anchors=anchors, classes=classes)


def _make_classifier():
    return HybridClassifier(client=None, catalog=_make_catalog())  # type: ignore[arg-type]


# --------------------------------------------------------------------- #
# Test : raw cosine prediction
# --------------------------------------------------------------------- #


def test_raw_cosine_predicts_argmax_anchor():
    cat = _make_catalog()
    # Embedding pointing at "billing" (index 3).
    vec = np.array([0.0, 0.0, 0.0, 1.0], dtype=np.float32)
    label, top, runner = _raw_cosine_predict(vec, cat)
    assert label == "billing"
    assert top == 1.0
    assert runner == 0.0
    print("OK : raw cosine picks argmax anchor")


def test_raw_cosine_handles_zero_norm():
    cat = _make_catalog()
    vec = np.zeros(4, dtype=np.float32)
    label, top, runner = _raw_cosine_predict(vec, cat)
    assert label == "uncertain"
    assert top == 0.0
    assert runner == 0.0
    print("OK : zero-norm input degrades gracefully")


# --------------------------------------------------------------------- #
# Test : decision policy (each branch of the truth table)
# --------------------------------------------------------------------- #


def test_combine_agree_returns_max_confidence():
    h = _make_classifier()
    out = h._combine(
        raw_label="fraud", raw_top=0.92, raw_runner=0.30,
        swarm_label="fraud", swarm_conf=0.71,
        extra={},
    )
    assert out.label == "fraud"
    assert out.path == "agree"
    assert abs(out.confidence - 0.92) < 1e-6  # max of the two
    print("OK : agreement → max confidence")


def test_combine_disagree_raw_decisive_overrides_swarm():
    h = _make_classifier()
    out = h._combine(
        raw_label="fraud", raw_top=0.78, raw_runner=0.30,  # decisive (>0.55, gap big)
        swarm_label="billing", swarm_conf=0.55,             # below low-conf threshold
        extra={},
    )
    assert out.label == "fraud"
    assert out.path == "raw_overrides_swarm"
    assert out.confidence == 0.78
    print("OK : decisive raw overrides unsure swarm")


def test_combine_disagree_swarm_high_conf_overrides_raw():
    h = _make_classifier()
    out = h._combine(
        raw_label="bug", raw_top=0.45, raw_runner=0.40,     # not decisive
        swarm_label="fraud", swarm_conf=0.91,                # very high swarm
        extra={},
    )
    assert out.label == "fraud"
    assert out.path == "swarm_overrides_raw"
    assert out.confidence == 0.91
    print("OK : very-high-conf swarm overrides middling raw")


def test_combine_disagree_both_middling_returns_reject():
    h = _make_classifier()
    out = h._combine(
        raw_label="fraud", raw_top=0.50, raw_runner=0.45,   # raw not decisive (margin too small)
        swarm_label="billing", swarm_conf=0.78,              # not high-conf
        extra={},
    )
    assert out.label == "uncertain"
    assert out.path == "reject"
    assert out.confidence == 0.78  # the higher of the two
    print("OK : both signals middling → reject")


def test_combine_agree_uncertain_does_not_collapse_to_uncertain_label():
    """If raw says 'uncertain' AND swarm says 'uncertain', we still
    surface the uncertainty rather than pretend agreement on a class."""
    h = _make_classifier()
    out = h._combine(
        raw_label="uncertain", raw_top=0.0, raw_runner=0.0,
        swarm_label="uncertain", swarm_conf=0.0,
        extra={},
    )
    assert out.label == "uncertain"
    assert out.path == "reject"
    print("OK : both uncertain → reject (no synthetic agreement)")


def test_combine_handles_nan_inputs():
    h = _make_classifier()
    out = h._combine(
        raw_label="fraud", raw_top=float("nan"), raw_runner=0.0,
        swarm_label="fraud", swarm_conf=float("nan"),
        extra={},
    )
    # NaN must be sanitised to 0 — neither side is "decisive".
    assert out.label == "fraud"
    assert out.path == "agree"
    assert out.confidence == 0.0
    print("OK : NaN inputs are sanitised, never propagated")


# --------------------------------------------------------------------- #
# Test : constructor validates thresholds
# --------------------------------------------------------------------- #


def test_constructor_rejects_low_high_swap():
    cat = _make_catalog()
    try:
        HybridClassifier(
            client=None,  # type: ignore[arg-type]
            catalog=cat,
            swarm_low_confidence=0.90,
            swarm_high_confidence=0.50,
        )
    except ValueError as e:
        assert "low" in str(e) and "high" in str(e)
        print("OK : low > high constructor swap rejected")
        return
    raise AssertionError("constructor should have rejected low > high")


def test_constructor_rejects_out_of_range_thresholds():
    cat = _make_catalog()
    for kw in [
        {"raw_high_margin": 1.5},
        {"swarm_high_confidence": -0.1},
        {"raw_top_gap": 2.0},
    ]:
        try:
            HybridClassifier(client=None, catalog=cat, **kw)  # type: ignore[arg-type]
        except ValueError:
            continue
        raise AssertionError(f"constructor should have rejected {kw}")
    print("OK : out-of-range thresholds rejected")


# --------------------------------------------------------------------- #
# Standalone runner
# --------------------------------------------------------------------- #


def main() -> int:
    tests = [
        test_raw_cosine_predicts_argmax_anchor,
        test_raw_cosine_handles_zero_norm,
        test_combine_agree_returns_max_confidence,
        test_combine_disagree_raw_decisive_overrides_swarm,
        test_combine_disagree_swarm_high_conf_overrides_raw,
        test_combine_disagree_both_middling_returns_reject,
        test_combine_agree_uncertain_does_not_collapse_to_uncertain_label,
        test_combine_handles_nan_inputs,
        test_constructor_rejects_low_high_swap,
        test_constructor_rejects_out_of_range_thresholds,
    ]
    failures = 0
    for t in tests:
        try:
            t()
        except AssertionError as e:
            print(f"FAIL : {t.__name__} — {e}")
            failures += 1
        except Exception as e:
            print(f"ERROR : {t.__name__} — {type(e).__name__}: {e}")
            failures += 1
    if failures:
        print(f"\n{failures}/{len(tests)} tests failed")
        return 1
    print(f"\nAll {len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
