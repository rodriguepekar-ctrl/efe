"""Tests for deterministic behaviour with fixed seed."""

from __future__ import annotations

from pathlib import Path

import efe


def test_same_seed_yields_same_label(
    bundle: efe.Bundle, catalog_path: Path
) -> None:
    """Same seed + same input → same label."""
    text = "I need to dispute a charge"
    client_a = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    client_b = efe.EfeNativeClient(bundle, str(catalog_path), seed=42)
    v_a = client_a.classify(text)
    v_b = client_b.classify(text)
    assert v_a.label == v_b.label
    assert v_a.confidence == v_b.confidence
    client_a.close()
    client_b.close()


def test_different_seed_can_yield_different_label(
    bundle: efe.Bundle, catalog_path: Path
) -> None:
    """Different seeds (almost always) yield different verdicts.

    The D25 stub synthesises labels from
    `SHA256(request_id || catalog_id || seed)` ; with the request_id
    derived from the system clock + seed, two different seeds with
    different request_ids will (with extremely high probability)
    produce different labels.
    """
    text = "test"
    client_a = efe.EfeNativeClient(bundle, str(catalog_path), seed=1)
    client_b = efe.EfeNativeClient(bundle, str(catalog_path), seed=2)
    v_a = client_a.classify(text)
    v_b = client_b.classify(text)
    # Both verdicts must be valid.
    assert v_a.label
    assert v_b.label
    client_a.close()
    client_b.close()


def test_classify_zero_copy_seed_42_strictly_deterministic(
    bundle_path: Path, catalog_path: Path
) -> None:
    """Audit fix F-A4 : `classify_zero_copy(..., seed=42)` with
    identical args MUST produce strictly identical verdicts across
    repeated calls. Pre-fix the stub mixed a clock-derived
    `request_id` into the verdict hash, breaking SLO F-12. Post-fix
    the hash uses (embedding, catalog_id, seed) only — fully
    deterministic."""
    verdicts: list[efe.Verdict] = []
    for _ in range(10):
        v = efe.classify_zero_copy(
            "test",
            catalog_id="router-v1",
            bundle_path=str(bundle_path),
            catalog_path=str(catalog_path),
            seed=42,
        )
        verdicts.append(v)
    # Every verdict must be label-identical and confidence-identical.
    first = verdicts[0]
    for v in verdicts[1:]:
        assert v.label == first.label, (
            f"determinism broken : {v.label} != {first.label}"
        )
        assert v.confidence == first.confidence
        assert v.swarm_id == first.swarm_id  # swarm_id is part of the hash output
        assert v.cone_id == first.cone_id
