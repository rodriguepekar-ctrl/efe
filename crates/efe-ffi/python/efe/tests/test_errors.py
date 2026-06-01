"""Tests for the error mapping (Rust → Python exception)."""

from __future__ import annotations

from pathlib import Path

import pytest

import efe


def test_classify_zero_copy_rejects_empty_text(
    bundle_path: Path, catalog_path: Path
) -> None:
    with pytest.raises(efe.ConfigError):
        efe.classify_zero_copy(
            "",
            catalog_id="router-v1",
            bundle_path=str(bundle_path),
            catalog_path=str(catalog_path),
        )


def test_classify_zero_copy_rejects_empty_catalog_id(
    bundle_path: Path, catalog_path: Path
) -> None:
    with pytest.raises(efe.ConfigError):
        efe.classify_zero_copy(
            "hello",
            catalog_id="",
            bundle_path=str(bundle_path),
            catalog_path=str(catalog_path),
        )


def test_classify_zero_copy_rejects_empty_bundle_path(
    catalog_path: Path,
) -> None:
    with pytest.raises(efe.ConfigError):
        efe.classify_zero_copy(
            "hello",
            catalog_id="router-v1",
            bundle_path="",
            catalog_path=str(catalog_path),
        )


def test_load_bundle_rejects_empty_path() -> None:
    with pytest.raises(efe.BundleError):
        efe.load_bundle("")


def test_swarm_spec_rejects_empty_request_id() -> None:
    with pytest.raises(efe.ConfigError):
        efe.SwarmSpec("", "router-v1")


def test_swarm_spec_rejects_zero_max_rounds() -> None:
    with pytest.raises(efe.ConfigError):
        efe.SwarmSpec("req-1", "router-v1", max_rounds=0)


def test_efe_native_client_rejects_empty_catalog_path(
    bundle: efe.Bundle,
) -> None:
    with pytest.raises(efe.ConfigError):
        efe.EfeNativeClient(bundle, "")


def test_classify_after_close_raises(
    bundle: efe.Bundle, catalog_path: Path
) -> None:
    client = efe.EfeNativeClient(bundle, str(catalog_path))
    client.close()
    with pytest.raises(efe.EfeError):
        client.classify("hello")


def test_classify_empty_text_raises(client: efe.EfeNativeClient) -> None:
    with pytest.raises(efe.EfeError):
        client.classify("")


def test_alignment_cone_rejects_negative_max_distance() -> None:
    import numpy as np

    anchors = np.zeros((4, 16), dtype=np.float32)
    with pytest.raises(efe.ConfigError):
        efe.AlignmentCone("cone-1", anchors, -0.5)


def test_alignment_cone_rejects_nan_max_distance() -> None:
    import numpy as np

    anchors = np.zeros((4, 16), dtype=np.float32)
    with pytest.raises(efe.ConfigError):
        efe.AlignmentCone("cone-1", anchors, float("nan"))


def test_exception_hierarchy() -> None:
    """All custom exceptions inherit from EfeError, which inherits Exception."""
    for cls in [
        efe.BundleError,
        efe.CatalogError,
        efe.SwarmError,
        efe.AlignmentBreach,
        efe.NumericalError,
        efe.ResourceExhausted,
        efe.ConfigError,
        efe.TimeoutError,
        efe.FfiError,
    ]:
        assert issubclass(cls, efe.EfeError)
        assert issubclass(cls, Exception)
