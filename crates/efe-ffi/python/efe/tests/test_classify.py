"""Tests for the classify path."""

from __future__ import annotations

from pathlib import Path

import efe


def test_classify_returns_verdict(client: efe.EfeNativeClient) -> None:
    verdict = client.classify("I want to dispute the charge")
    assert isinstance(verdict, efe.Verdict)
    assert verdict.label
    assert 0.0 <= verdict.confidence <= 1.0
    assert verdict.rounds_executed >= 1
    assert verdict.swarm_id.startswith("swarm-")


def test_classify_zero_copy_one_shot(
    bundle_path: Path, catalog_path: Path
) -> None:
    verdict = efe.classify_zero_copy(
        "hello world",
        catalog_id="router-v1",
        bundle_path=str(bundle_path),
        catalog_path=str(catalog_path),
        seed=42,
    )
    assert isinstance(verdict, efe.Verdict)
    assert verdict.catalog_version


def test_classify_to_dict_roundtrip(client: efe.EfeNativeClient) -> None:
    verdict = client.classify("test")
    d = verdict.to_dict()
    assert d["label"] == verdict.label
    assert d["confidence"] == verdict.confidence
    assert d["rounds_executed"] == verdict.rounds_executed


def test_verdict_repr_is_useful(client: efe.EfeNativeClient) -> None:
    verdict = client.classify("test")
    repr_str = repr(verdict)
    assert "Verdict(" in repr_str
    assert verdict.label in repr_str


def test_verdict_equality_after_classify(
    bundle: efe.Bundle, catalog_path: Path
) -> None:
    """Two clients with the same seed → identical verdict for same text."""
    client_a = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    client_b = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    text = "I need help"
    v_a = client_a.classify(text)
    v_b = client_b.classify(text)
    # Label + cone_id + catalog_version should match (the latency
    # field varies so we don't compare it directly).
    assert v_a.label == v_b.label
    assert v_a.cone_id == v_b.cone_id
    assert v_a.catalog_version == v_b.catalog_version
    client_a.close()
    client_b.close()
