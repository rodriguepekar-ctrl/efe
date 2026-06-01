"""Tests for the async path."""

from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

import efe

pytestmark = pytest.mark.asyncio


async def test_aclassify_returns_verdict(
    bundle_path: Path, catalog_path: Path
) -> None:
    verdict = await efe.aclassify_zero_copy(
        "hello async",
        catalog_id="router-v1",
        bundle_path=str(bundle_path),
        catalog_path=str(catalog_path),
        seed=42,
    )
    assert isinstance(verdict, efe.Verdict)
    assert verdict.label


async def test_aclassify_concurrent(
    bundle_path: Path, catalog_path: Path
) -> None:
    """Multiple concurrent aclassify calls should all return."""
    texts = ["hello", "world", "foo", "bar", "baz"]
    coros = [
        efe.aclassify_zero_copy(
            t,
            catalog_id="router-v1",
            bundle_path=str(bundle_path),
            catalog_path=str(catalog_path),
            seed=42,
        )
        for t in texts
    ]
    verdicts = await asyncio.gather(*coros)
    assert len(verdicts) == len(texts)
    for v in verdicts:
        assert isinstance(v, efe.Verdict)


async def test_aclassify_propagates_errors(
    bundle_path: Path, catalog_path: Path
) -> None:
    """Empty text should raise ConfigError through the async path."""
    with pytest.raises(efe.ConfigError):
        await efe.aclassify_zero_copy(
            "",
            catalog_id="router-v1",
            bundle_path=str(bundle_path),
            catalog_path=str(catalog_path),
        )
