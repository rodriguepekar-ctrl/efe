"""End-to-end test for EfeClient.classify.

Uses a `FakeEmbedder` + a mocked gRPC stub so the whole Python
pipeline is exercised without needing sentence-transformers or a
running EFE server.
"""

from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock

import numpy as np
import pytest

from efe_tool.catalog import AnchorCatalog
from efe_tool.client import EfeClient
from efe_tool.errors import BridgeError
from efe_tool.types import ClassifyResult


class FakeEmbedder:
    """Deterministic embedder: SHA-256 → stable slot. Dim large enough
    to keep the collision probability negligible on tiny test corpora.

    Using SHA-256 (not ``hash()``) makes the embedding reproducible
    across Python interpreter restarts — ``hash()`` randomises its
    string hashing by default (PYTHONHASHSEED)."""

    def __init__(self, dim: int = 64) -> None:
        self._dim = dim

    def encode(self, text: str) -> np.ndarray:
        import hashlib
        v = np.zeros(self._dim, dtype=np.float32)
        h = hashlib.sha256(text.encode("utf-8")).digest()
        idx = int.from_bytes(h[:4], "big") % self._dim
        v[idx] = 1.0
        return v

    def encode_batch(self, texts):
        return np.stack([self.encode(t) for t in texts])

    def dim(self) -> int:
        return self._dim

    def fingerprint(self) -> str:
        return f"sentence-transformers/test-fake@{self._dim}"


@pytest.fixture
def catalog(tmp_path: Path) -> AnchorCatalog:
    path = tmp_path / "cat.yaml"
    path.write_text(
        """
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata: { name: demo, version: "1.0.0" }
embedder: { family: sentence-transformers, model: test-fake, dim: 64 }
cone_policy: { max_projection_distance: 0.5 }
thresholds: { confidence_high: 0.85, confidence_medium: 0.65 }
classes:
  - name: alpha
    canonical_sentences: ["aaa"]
  - name: beta
    canonical_sentences: ["bbb"]
  - name: gamma
    canonical_sentences: ["ccc"]
"""
    )
    return AnchorCatalog.load(path, embedder=FakeEmbedder(dim=64))


def _make_fake_execute_response(*, confidence: float, payload_vec: list[float] | None,
                                 alignment_delta: float, rounds: int = 8):
    """Hand-build a fake ExecuteResponse proto."""
    from efe_tool._proto.efe.v1 import efe_pb2, types_pb2

    if payload_vec is not None:
        out = types_pb2.DecodedOutput(
            raw=types_pb2.RawCentroid(values=payload_vec)
        )
    else:
        out = types_pb2.DecodedOutput(text="unexpected text")
    return efe_pb2.ExecuteResponse(
        swarm_id="test-swarm",
        request_id="test-req",
        output=out,
        confidence=confidence,
        verdict=types_pb2.Verdict.VERDICT_CONVERGED,
        rounds_executed=rounds,
        total_ms=42.0,
        alignment_delta=alignment_delta,
    )


def _client_with_stub(embedder, stub_mock) -> EfeClient:
    client = EfeClient(endpoint="unused:0", embedder=embedder, timeout_s=1.0)
    # Inject the mocked stub directly — bypass gRPC channel.
    client._stub = stub_mock  # noqa: SLF001
    client._channel = MagicMock()  # noqa: SLF001
    return client


def test_classify_returns_label_when_confidence_high(catalog: AnchorCatalog) -> None:
    stub = MagicMock()
    # Route the centroid to slot 0 (match the "alpha" anchor hash).
    # The FakeEmbedder encodes "aaa" -> index hash("aaa") % 8.
    alpha_vec = FakeEmbedder(dim=64).encode("aaa").tolist()
    stub.Execute.return_value = _make_fake_execute_response(
        confidence=0.92, payload_vec=alpha_vec, alignment_delta=0.1,
    )
    client = _client_with_stub(FakeEmbedder(dim=64), stub)
    result = client.classify("aaa", catalog)
    assert isinstance(result, ClassifyResult)
    assert result.label == "alpha"
    assert result.reject is False
    assert 0.9 < result.confidence < 1.0
    assert result.top_cosine is not None and result.top_cosine > 0.99
    assert result.sub_scores and len(result.sub_scores) == 3
    # Server-side request was built + sent.
    stub.Execute.assert_called_once()


def test_classify_rejects_when_confidence_low(catalog: AnchorCatalog) -> None:
    stub = MagicMock()
    alpha_vec = FakeEmbedder(dim=64).encode("aaa").tolist()
    stub.Execute.return_value = _make_fake_execute_response(
        confidence=0.40,  # below medium threshold (0.65)
        payload_vec=alpha_vec,
        alignment_delta=0.1,
    )
    client = _client_with_stub(FakeEmbedder(dim=64), stub)
    result = client.classify("aaa", catalog)
    assert result.reject is True
    assert result.label == "uncertain"


def test_classify_rejects_on_alignment_breach(catalog: AnchorCatalog) -> None:
    stub = MagicMock()
    alpha_vec = FakeEmbedder(dim=64).encode("aaa").tolist()
    stub.Execute.return_value = _make_fake_execute_response(
        confidence=0.92,
        payload_vec=alpha_vec,
        alignment_delta=0.9,  # > catalog.max_projection_distance (0.5)
    )
    client = _client_with_stub(FakeEmbedder(dim=64), stub)
    result = client.classify("aaa", catalog)
    assert result.reject is True
    assert result.label == "uncertain"
    assert result.alignment_delta > 0.5


def test_classify_rejects_on_non_raw_payload(catalog: AnchorCatalog) -> None:
    stub = MagicMock()
    stub.Execute.return_value = _make_fake_execute_response(
        confidence=0.92, payload_vec=None, alignment_delta=0.1,
    )
    client = _client_with_stub(FakeEmbedder(dim=64), stub)
    result = client.classify("aaa", catalog)
    assert result.reject is True
    assert result.label == "uncertain"


def test_embedder_mismatch_rejected(tmp_path: Path) -> None:
    # Build a catalog compiled with dim=8 embedder…
    path = tmp_path / "c.yaml"
    path.write_text(
        """
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata: { name: cc, version: "1.0.0" }
embedder: { family: sentence-transformers, model: test-fake, dim: 64 }
classes:
  - { name: a, canonical_sentences: [alpha] }
  - { name: b, canonical_sentences: [beta] }
"""
    )
    catalog = AnchorCatalog.load(path, embedder=FakeEmbedder(dim=64))
    # …but try to classify with a dim=4 embedder.

    class OtherEmbedder(FakeEmbedder):
        def fingerprint(self) -> str:
            return "sentence-transformers/other@4"

    stub = MagicMock()
    client = _client_with_stub(OtherEmbedder(dim=4), stub)
    with pytest.raises(BridgeError, match="embedder mismatch"):
        client.classify("alpha", catalog)


def test_classify_cache_hit_skips_server(catalog: AnchorCatalog) -> None:
    stub = MagicMock()
    alpha_vec = FakeEmbedder(dim=64).encode("aaa").tolist()
    stub.Execute.return_value = _make_fake_execute_response(
        confidence=0.92, payload_vec=alpha_vec, alignment_delta=0.1,
    )
    client = _client_with_stub(FakeEmbedder(dim=64), stub)
    client.classify("aaa", catalog)
    client.classify("aaa", catalog)
    # Second call hits the local LRU cache → server called only once.
    stub.Execute.assert_called_once()
    stats = client.cache_stats()
    assert stats["hits"] == 1 and stats["misses"] == 1


def test_classify_rejects_empty_text(catalog: AnchorCatalog) -> None:
    client = _client_with_stub(FakeEmbedder(dim=64), MagicMock())
    with pytest.raises(BridgeError, match="non-empty"):
        client.classify("   ", catalog)


def test_classify_propagates_budget_to_spec(catalog: AnchorCatalog) -> None:
    """Fix #1: the client must inject a ResourceBudget > server default 500ms."""
    stub = MagicMock()
    alpha_vec = FakeEmbedder(dim=64).encode("aaa").tolist()
    stub.Execute.return_value = _make_fake_execute_response(
        confidence=0.92, payload_vec=alpha_vec, alignment_delta=0.1,
    )
    client = _client_with_stub(FakeEmbedder(dim=64), stub)
    client.classify("aaa", catalog, budget_wall_clock_ms=8_000)
    # Inspect the request that was sent.
    args, kwargs = stub.Execute.call_args
    sent_req = args[0]
    assert sent_req.spec.budget.max_wall_clock_ms == 8_000
