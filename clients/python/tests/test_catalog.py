"""Tests for AnchorCatalog (no network, no embedder model download)."""

from __future__ import annotations

from pathlib import Path

import numpy as np
import pytest

from efe_tool.catalog import AnchorCatalog, ThresholdPolicy
from efe_tool.embedder import Embedder
from efe_tool.errors import CatalogError


class FakeEmbedder:
    """Deterministic in-memory embedder used in tests.

    Maps each unique input string to a one-hot-like vector so anchors
    computed from canonical sentences are easy to predict.
    """

    def __init__(self, dim: int = 4) -> None:
        self._dim = dim
        self._vocab: dict[str, int] = {}

    def encode(self, text: str) -> np.ndarray:
        idx = self._vocab.setdefault(text, len(self._vocab) % self._dim)
        v = np.zeros(self._dim, dtype=np.float32)
        v[idx] = 1.0
        return v

    def encode_batch(self, texts):
        return np.stack([self.encode(t) for t in texts])

    def dim(self) -> int:
        return self._dim

    def fingerprint(self) -> str:
        return f"sentence-transformers/test-fake@{self._dim}"


def _write_catalog_yaml(tmp: Path, model_name: str = "test-fake", dim: int = 4) -> Path:
    path = tmp / "cat.yaml"
    path.write_text(
        f"""
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata:
  name: test_cat
  version: "1.0.0"
embedder:
  family: sentence-transformers
  model: {model_name}
  dim: {dim}
cone_policy:
  max_projection_distance: 0.5
thresholds:
  confidence_high: 0.85
  confidence_medium: 0.65
classes:
  - name: a
    canonical_sentences:
      - "alpha one"
  - name: b
    canonical_sentences:
      - "beta one"
  - name: c
    canonical_sentences:
      - "gamma one"
"""
    )
    return path


def test_loads_valid_catalog(tmp_path: Path) -> None:
    emb = FakeEmbedder(dim=4)
    catalog = AnchorCatalog.load(_write_catalog_yaml(tmp_path), embedder=emb)
    assert catalog.name == "test_cat"
    assert catalog.version == "1.0.0"
    assert catalog.dim == 4
    assert len(catalog.classes) == 3
    assert catalog.anchors.shape == (3, 4)
    # each anchor should be L2-normalised
    for i in range(3):
        assert abs(np.linalg.norm(catalog.anchors[i]) - 1.0) < 1e-5


def test_compiled_cache_roundtrip(tmp_path: Path) -> None:
    emb = FakeEmbedder(dim=4)
    yaml_path = _write_catalog_yaml(tmp_path)
    c1 = AnchorCatalog.load(yaml_path, embedder=emb)
    # Reloading without an embedder must hit the .compiled.json
    c2 = AnchorCatalog.load(yaml_path)
    assert c2.name == c1.name
    np.testing.assert_allclose(c1.anchors, c2.anchors)


def test_embedder_fingerprint_mismatch_refused(tmp_path: Path) -> None:
    yaml_path = _write_catalog_yaml(tmp_path, model_name="test-fake", dim=4)
    # First compile with dim=4.
    AnchorCatalog.load(yaml_path, embedder=FakeEmbedder(dim=4))
    # Now load with an embedder that would produce a different fingerprint
    # (dim mismatch) — must refuse because the catalog declares dim=4 and
    # the cached anchors were built against that.
    # We force recompile by deleting the cache, then try to compile with
    # the wrong embedder.
    (tmp_path / "cat.compiled.json").unlink()
    with pytest.raises(CatalogError):
        AnchorCatalog.load(yaml_path, embedder=FakeEmbedder(dim=8))


def test_rejects_too_similar_classes(tmp_path: Path) -> None:
    # Two classes with identical canonical sentences → cosine 1.0 > 0.95.
    path = tmp_path / "bad.yaml"
    path.write_text(
        """
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata:
  name: bad
  version: "1.0.0"
embedder: {family: sentence-transformers, model: test-fake, dim: 4}
cone_policy: {max_projection_distance: 0.5}
classes:
  - {name: a, canonical_sentences: ["same text"]}
  - {name: b, canonical_sentences: ["same text"]}
"""
    )
    with pytest.raises(CatalogError, match="too similar"):
        AnchorCatalog.load(path, embedder=FakeEmbedder(dim=4))


def test_thresholds_zone() -> None:
    tp = ThresholdPolicy(confidence_high=0.85, confidence_medium=0.65)
    assert tp.zone(0.90) == "trusted"
    assert tp.zone(0.85) == "trusted"  # inclusive lower bound
    assert tp.zone(0.70) == "warn"
    assert tp.zone(0.64) == "reject"


def test_refuses_too_few_classes(tmp_path: Path) -> None:
    path = tmp_path / "one.yaml"
    path.write_text(
        """
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata: {name: one, version: "1.0.0"}
embedder: {family: sentence-transformers, model: test-fake, dim: 4}
classes:
  - {name: lone, canonical_sentences: ["x"]}
"""
    )
    with pytest.raises(CatalogError, match="at least 2 classes"):
        AnchorCatalog.load(path, embedder=FakeEmbedder(dim=4))


def test_class_lookup_helpers(tmp_path: Path) -> None:
    cat = AnchorCatalog.load(_write_catalog_yaml(tmp_path), embedder=FakeEmbedder(dim=4))
    assert cat.class_names() == ("a", "b", "c")
    assert cat.class_by_index(1).name == "b"
    assert cat.class_by_name("c").canonical_sentences == ("gamma one",)
    with pytest.raises(CatalogError):
        cat.class_by_index(99)
    with pytest.raises(CatalogError):
        cat.class_by_name("no-such")


# Make sure FakeEmbedder satisfies the Embedder protocol.
def test_fake_embedder_satisfies_protocol() -> None:
    assert isinstance(FakeEmbedder(), Embedder)
