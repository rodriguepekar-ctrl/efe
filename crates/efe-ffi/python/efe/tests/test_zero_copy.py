"""Tests for the numpy zero-copy interop."""

from __future__ import annotations

import numpy as np

import efe


def test_bundle_encode_returns_numpy_float32(bundle: efe.Bundle) -> None:
    arr = bundle.encode("hello")
    assert isinstance(arr, np.ndarray)
    assert arr.dtype == np.float32
    assert arr.shape == (bundle.dim,)


def test_bundle_encode_l2_norm_is_one(bundle: efe.Bundle) -> None:
    arr = bundle.encode("hello world")
    norm = float(np.linalg.norm(arr))
    assert abs(norm - 1.0) < 1e-4, f"L2 norm {norm} != 1.0"


def test_bundle_encode_deterministic(bundle: efe.Bundle) -> None:
    a = bundle.encode("same text")
    b = bundle.encode("same text")
    np.testing.assert_array_equal(a, b)


def test_bundle_encode_different_text_yields_different_vec(bundle: efe.Bundle) -> None:
    a = bundle.encode("hello")
    b = bundle.encode("world")
    assert not np.array_equal(a, b)


def test_swarm_spec_set_embedding_accepts_numpy() -> None:
    spec = efe.SwarmSpec("req-1", "router-v1", seed=42)
    arr = np.zeros(384, dtype=np.float32)
    spec.set_embedding(arr)
    assert spec.embedding_dim() == 384


def test_swarm_spec_builder_with_numpy_embedding() -> None:
    arr = np.ones(384, dtype=np.float32) * 0.1
    spec = (
        efe.SwarmSpec.builder()
        .request_id("req-1")
        .catalog_id("router-v1")
        .embedding(arr)
        .seed(42)
        .max_rounds(10)
        .build()
    )
    assert spec.embedding_dim() == 384
    assert spec.seed == 42
    assert spec.max_rounds == 10
