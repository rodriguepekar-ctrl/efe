"""efe_compare_bundles_quality (M20) — pin tests."""
from __future__ import annotations

import json

import pytest

from m20_helpers import make_compare_bundles_response




@pytest.fixture
def fake_backend_with_classify_batch(fake_backend):
    """Augment FakeBackend with classify_batch + sample_gold_sentences."""

    async def classify_batch(*, catalog, version, sentences, auth):
        # Deterministic : every sentence gets label "X" with conf 0.8
        return [
            {"label": "X", "confidence": 0.8 if version == "v1.0.0" else 0.6}
            for _ in sentences
        ]

    async def sample_gold_sentences(*, catalog, n, auth):
        return [f"sentence-{i}" for i in range(n)]

    fake_backend.classify_batch = classify_batch
    fake_backend.sample_gold_sentences = sample_gold_sentences
    return fake_backend


@pytest.mark.asyncio
async def test_compare_bundles_basic_path(
    m20_dispatcher, fake_backend_with_classify_batch, fake_sampling_session,
):
    fake_sampling_session.next_response = make_compare_bundles_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_compare_bundles_quality",
        args={
            "catalog": "support-routing",
            "version_a": "v1.0.0",
            "version_b": "v1.1.0",
            "test_sentences": ["a", "b", "c"],
        },
    )
    assert result.is_error is False
    assert result.content["recommendation"] in (
        "promote", "investigate", "do_not_promote",
    )


@pytest.mark.asyncio
async def test_compare_bundles_rejects_same_versions(
    m20_dispatcher, fake_backend_with_classify_batch, fake_sampling_session,
):
    result = await m20_dispatcher.dispatch(
        tool_name="efe_compare_bundles_quality",
        args={
            "catalog": "x",
            "version_a": "v1.0.0",
            "version_b": "v1.0.0",
        },
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_compare_bundles_samples_when_no_sentences(
    m20_dispatcher, fake_backend_with_classify_batch, fake_sampling_session,
):
    fake_sampling_session.next_response = make_compare_bundles_response()
    result = await m20_dispatcher.dispatch(
        tool_name="efe_compare_bundles_quality",
        args={
            "catalog": "x",
            "version_a": "v1.0.0",
            "version_b": "v1.1.0",
            "sample_size": 10,
        },
    )
    assert result.is_error is False
    assert result.content["n_sentences_compared"] == 10


@pytest.mark.asyncio
async def test_compare_bundles_uses_provided_sentences(
    m20_dispatcher, fake_backend_with_classify_batch, fake_sampling_session,
):
    fake_sampling_session.next_response = make_compare_bundles_response()
    sentences = ["s1", "s2", "s3", "s4"]
    result = await m20_dispatcher.dispatch(
        tool_name="efe_compare_bundles_quality",
        args={
            "catalog": "x",
            "version_a": "v1.0.0",
            "version_b": "v1.1.0",
            "test_sentences": sentences,
        },
    )
    assert result.content["n_sentences_compared"] == 4


@pytest.mark.asyncio
async def test_compare_bundles_validates_recommendation(
    m20_dispatcher, fake_backend_with_classify_batch, fake_sampling_session,
):
    fake_sampling_session.next_response = json.dumps({
        "accuracy_delta_pp": 1.0,
        "divergence_examples": [],
        "summary": "x" * 30,
        "recommendation": "ship_it",  # invalid enum
    })
    result = await m20_dispatcher.dispatch(
        tool_name="efe_compare_bundles_quality",
        args={
            "catalog": "x",
            "version_a": "v1.0.0",
            "version_b": "v1.1.0",
            "test_sentences": ["s1"],
        },
    )
    assert result.is_error is True


@pytest.mark.asyncio
async def test_compare_bundles_emits_audit_event(
    m20_dispatcher, fake_backend_with_classify_batch, fake_sampling_session,
    audit_emitter,
):
    audit_emitter.enabled = False
    fake_sampling_session.next_response = make_compare_bundles_response()
    before = len(audit_emitter.recent(limit=500))
    await m20_dispatcher.dispatch(
        tool_name="efe_compare_bundles_quality",
        args={
            "catalog": "x",
            "version_a": "v1.0.0",
            "version_b": "v1.1.0",
            "test_sentences": ["s"],
        },
    )
    after = len(audit_emitter.recent(limit=500))
    assert after == before + 1


@pytest.mark.asyncio
async def test_compare_bundles_blocks_when_feature_disabled(
    m20_off_dispatcher, fake_sampling_session,
):
    fake_sampling_session.next_response = make_compare_bundles_response()
    result = await m20_off_dispatcher.dispatch(
        tool_name="efe_compare_bundles_quality",
        args={
            "catalog": "x",
            "version_a": "v1.0.0",
            "version_b": "v1.1.0",
            "test_sentences": ["s"],
        },
    )
    assert result.is_error is True
