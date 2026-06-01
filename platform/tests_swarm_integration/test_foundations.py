"""Types + errors + hashing + config."""

from __future__ import annotations

import copy
import pickle
from datetime import datetime, timezone

import pytest

from efe_swarm_integration.config import DEFAULTS, MAX_DIM, MAX_POPULATION
from efe_swarm_integration.errors import (
    ContractSnapshotMismatch,
    SchemaViolation,
    SwarmIntegrationError,
)
from efe_swarm_integration.hashing import (
    DOMAIN_TAG_ARTEFACT_SYNC,
    DOMAIN_TAG_CONE_BINDING,
    DOMAIN_TAG_SWARM_IDEMPOTENCY,
    canonical_json,
    hmac_seal,
    verify_seal,
    is_using_dev_key,
    set_key,
)
from efe_swarm_integration.types import (
    AlignmentCone,
    Anchor,
    ConeRef,
    ConvergenceThresholds,
    DecodeMode,
    ExecuteRequest,
    PopulationEntry,
    PopulationMix,
    ResourceBudget,
    SwarmSpec,
    TemplateCode,
    build_swarm_id,
    default_spec_for_classify,
)


def test_template_code_stable_u8() -> None:
    assert TemplateCode.NaSem.as_u8() == 0
    assert TemplateCode.NaLog.as_u8() == 1
    assert TemplateCode.NaCrit.as_u8() == 2


def test_population_mix_default_classifier() -> None:
    mix = PopulationMix.default_classifier()
    assert mix.total == 112
    assert mix.template_count == 3


def test_population_mix_rejects_duplicates() -> None:
    with pytest.raises(ValueError):
        PopulationMix(
            entries=(
                PopulationEntry(TemplateCode.NaSem, 10),
                PopulationEntry(TemplateCode.NaSem, 5),
            )
        )


def test_anchor_dim_check() -> None:
    a = Anchor(values=(1.0, 2.0, 3.0))
    b = Anchor(values=(1.0, 2.0))
    with pytest.raises(ValueError):
        AlignmentCone(anchors=(a, b), max_projection_distance=0.3)


def test_alignment_cone_requires_positive_radius() -> None:
    a = Anchor(values=(1.0,))
    with pytest.raises(ValueError):
        AlignmentCone(anchors=(a,), max_projection_distance=0.0)


def test_cone_ref_key_format() -> None:
    ref = ConeRef(tenant="acme", task="triage", version=3)
    assert ref.key == "acme/triage@v3"


def test_swarm_spec_validates_embedding_len() -> None:
    with pytest.raises(ValueError):
        SwarmSpec(
            dim_latent=4,
            problem_embedding=(1.0, 2.0),
            population=PopulationMix.default_classifier(),
            alignment_cone=None,
            alignment_cone_ref=ConeRef("t", "k", 1),
        )


def test_swarm_spec_requires_cone_source() -> None:
    with pytest.raises(ValueError):
        SwarmSpec(
            dim_latent=2,
            problem_embedding=(0.0, 0.0),
            population=PopulationMix.default_classifier(),
            alignment_cone=None,
            alignment_cone_ref=None,
        )


def test_default_spec_for_classify_roundtrip() -> None:
    ref = ConeRef("t", "k", 1)
    spec = default_spec_for_classify(
        embedding=(0.1, 0.2, 0.3, 0.4), cone_ref=ref, budget_ms=200
    )
    assert spec.dim_latent == 4
    assert spec.budget.max_wall_clock_ms == 200


def test_build_swarm_id_deterministic_with_seed() -> None:
    assert build_swarm_id("foo") == build_swarm_id("foo")


def test_hmac_tags_distinct() -> None:
    a = hmac_seal(DOMAIN_TAG_SWARM_IDEMPOTENCY, {"k": 1})
    b = hmac_seal(DOMAIN_TAG_ARTEFACT_SYNC, {"k": 1})
    c = hmac_seal(DOMAIN_TAG_CONE_BINDING, {"k": 1})
    assert a != b != c


def test_hmac_seal_roundtrip() -> None:
    payload = {"a": 1, "b": [1, 2, 3]}
    seal = hmac_seal(DOMAIN_TAG_ARTEFACT_SYNC, payload)
    assert verify_seal(DOMAIN_TAG_ARTEFACT_SYNC, payload, seal)


def test_hmac_seal_rejects_unknown_tag() -> None:
    with pytest.raises(ValueError):
        hmac_seal(b"efe-other/tag/v1\x00", {"x": 1})


def test_set_key_flips_dev_flag() -> None:
    set_key(b"x" * 32)
    assert not is_using_dev_key()


def test_canonical_json_stable() -> None:
    a = canonical_json({"b": 1, "a": 2})
    b = canonical_json({"a": 2, "b": 1})
    assert a == b


def test_defaults_config_sane() -> None:
    assert DEFAULTS.transport.mtls_required is True
    assert DEFAULTS.max_inflight_per_tenant > 0
    assert MAX_DIM > 0
    assert MAX_POPULATION > 0
