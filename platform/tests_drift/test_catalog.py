"""Feature catalog — FeatureSpec validation + registry CRUD."""

from __future__ import annotations

import pytest

from efe_drift.catalog import FeatureCatalog, FeatureSpec
from efe_drift.errors import (
    DuplicateFeature,
    FeatureNotFound,
    InvalidBinningStrategy,
    UnknownExtractor,
    ValidationError,
)
from efe_drift.extraction import build_default_registry


def test_feature_spec_validates_required_fields():
    with pytest.raises(ValidationError):
        FeatureSpec(
            tenant_id="",
            task_id="t",
            name="x",
            kind="numeric",
            extractor="y",
            binning_strategy="equifrequency",
        )


def test_feature_spec_rejects_incompatible_binning_for_kind():
    with pytest.raises(InvalidBinningStrategy):
        # categorical cannot use equifrequency
        FeatureSpec(
            tenant_id="acme",
            task_id="summarize",
            name="language",
            kind="categorical",
            extractor="lang_detect",
            binning_strategy="equifrequency",
        )


def test_feature_spec_natural_key_tuple():
    spec = FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length",
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
        version=2,
    )
    assert spec.natural_key == ("acme", "summarize", "prompt_length", 2)


@pytest.mark.asyncio
async def test_catalog_register_and_get():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    spec = FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length_tokens",
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
    )
    entry = await catalog.register(spec)
    assert entry.name == "prompt_length_tokens"
    assert entry.enabled
    # Get by id + name.
    same = catalog.get(entry.feature_id)
    assert same.feature_id == entry.feature_id
    got = catalog.get_by_name(
        tenant_id="acme", task_id="summarize",
        name="prompt_length_tokens",
    )
    assert got.feature_id == entry.feature_id


@pytest.mark.asyncio
async def test_catalog_duplicate_refused():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    spec = FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length",
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
    )
    await catalog.register(spec)
    with pytest.raises(DuplicateFeature):
        await catalog.register(spec)


@pytest.mark.asyncio
async def test_catalog_allows_new_version_of_same_name():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    base = dict(
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length",
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
    )
    await catalog.register(FeatureSpec(**base, version=1))
    await catalog.register(FeatureSpec(**base, version=2))
    latest = catalog.get_by_name(
        tenant_id="acme", task_id="summarize",
        name="prompt_length",
    )
    assert latest.version == 2


@pytest.mark.asyncio
async def test_catalog_unknown_extractor_rejected():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    spec = FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="weird",
        kind="numeric",
        extractor="does_not_exist",
        binning_strategy="equifrequency",
    )
    with pytest.raises(UnknownExtractor):
        await catalog.register(spec)


@pytest.mark.asyncio
async def test_catalog_disable_and_enable():
    extractors = build_default_registry()
    catalog = FeatureCatalog(extractor_lookup=extractors)
    spec = FeatureSpec(
        tenant_id="acme",
        task_id="summarize",
        name="prompt_length",
        kind="numeric",
        extractor="token_length",
        binning_strategy="equifrequency",
    )
    entry = await catalog.register(spec)
    disabled = await catalog.disable(entry.feature_id)
    assert not disabled.enabled
    enabled = await catalog.enable(entry.feature_id)
    assert enabled.enabled


@pytest.mark.asyncio
async def test_catalog_not_found():
    catalog = FeatureCatalog()
    with pytest.raises(FeatureNotFound):
        catalog.get("missing")
    with pytest.raises(FeatureNotFound):
        catalog.get_by_name(
            tenant_id="a", task_id="b", name="c",
        )
