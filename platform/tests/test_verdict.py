"""Tests for verdict interpretation + dissolution."""

from __future__ import annotations

import pytest

from efe_platform.swarm.dissolution import (
    centroid_statistics,
    decode_output,
)
from efe_platform.swarm.protocol import (
    DecodedPayload,
    ExecuteResponse,
    Verdict,
)
from efe_platform.swarm.verdict import analyze_verdict


def _response(verdict: Verdict, confidence: float = 0.8) -> ExecuteResponse:
    return ExecuteResponse(
        swarm_id="s",
        request_id="r",
        payload=DecodedPayload(text="ok"),
        confidence=confidence,
        verdict=verdict,
        rounds_executed=2,
        total_ms=12.0,
        alignment_delta=0.1,
    )


# ---------------------------------------------------------------------------
#  Verdict → report
# ---------------------------------------------------------------------------


def test_converged_high_confidence_ok() -> None:
    r = analyze_verdict(_response(Verdict.CONVERGED, 0.9))
    assert r.confident is True
    assert r.severity == "ok"
    assert r.retryable is False


def test_converged_low_confidence_warn() -> None:
    r = analyze_verdict(_response(Verdict.CONVERGED, 0.2))
    assert r.confident is False
    assert r.severity == "warn"
    assert r.retryable is True


def test_continue_warns() -> None:
    r = analyze_verdict(_response(Verdict.CONTINUE))
    assert r.severity == "warn"
    assert r.retryable is True


def test_divergence_is_error() -> None:
    r = analyze_verdict(_response(Verdict.DIVERGENCE))
    assert r.severity == "error"
    assert r.confident is False


def test_mode_collapse_is_error() -> None:
    r = analyze_verdict(_response(Verdict.MODE_COLLAPSE))
    assert r.severity == "error"


def test_unspecified_escalates() -> None:
    r = analyze_verdict(_response(Verdict.UNSPECIFIED))
    assert r.severity == "error"
    assert r.retryable is True  # server may be flaky


def test_from_proto_cast_safety() -> None:
    assert Verdict.from_proto(0) is Verdict.UNSPECIFIED
    assert Verdict.from_proto(2) is Verdict.CONVERGED
    assert Verdict.from_proto(9999) is Verdict.UNSPECIFIED  # unknown wire


# ---------------------------------------------------------------------------
#  Dissolution
# ---------------------------------------------------------------------------


def test_decode_text_payload() -> None:
    out = decode_output(DecodedPayload(text="hello"))
    assert out.kind == "text"
    assert out.as_text() == "hello"


def test_decode_centroid_payload() -> None:
    out = decode_output(DecodedPayload(centroid=(0.1, 0.2, 0.3)))
    assert out.kind == "centroid"
    assert out.as_centroid() == (0.1, 0.2, 0.3)


def test_decode_mixed_raises() -> None:
    with pytest.raises(ValueError, match="exactly one"):
        DecodedPayload(text="x", centroid=(1.0,))


def test_as_text_on_centroid_raises() -> None:
    out = decode_output(DecodedPayload(centroid=(0.1,)))
    with pytest.raises(ValueError):
        out.as_text()


def test_centroid_statistics_shape() -> None:
    s = centroid_statistics((1.0, 2.0, 3.0, 4.0))
    assert s.dim == 4
    assert s.mean == pytest.approx(2.5)
    assert s.stddev > 0
    assert s.l2_norm > 0


def test_centroid_statistics_rejects_nan() -> None:
    with pytest.raises(ValueError, match="non-finite"):
        centroid_statistics((1.0, float("nan")))


def test_centroid_statistics_rejects_empty() -> None:
    with pytest.raises(ValueError, match="empty"):
        centroid_statistics(())
