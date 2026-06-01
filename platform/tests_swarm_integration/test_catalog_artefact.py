"""Catalog bridge + artefact syncer."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone

import pytest

from efe_swarm_integration.artefact.digest import compute_sha256
from efe_swarm_integration.artefact.manifest import (
    ArtefactManifest,
    seal_manifest,
    verify_manifest_seal,
)
from efe_swarm_integration.artefact.syncer import (
    ArtefactSyncer,
)
from efe_swarm_integration.catalog.bridge import CatalogBridge
from efe_swarm_integration.catalog.compiler import CatalogCompiler
from efe_swarm_integration.catalog.store import InMemoryConeBindingStore
from efe_swarm_integration.catalog.yaml_loader import (
    CatalogYaml,
    EmbedderYaml,
    LabelYaml,
)
from efe_swarm_integration.catalog.validator import validate_catalog_yaml
from efe_swarm_integration.errors import (
    ArtefactDigestMismatch,
    CatalogCompilationFailed,
    ConeRefNotBound,
    PiiDetectedInPrompt,
)
from efe_swarm_integration.types import ConeRef


# ---------------------------------------------------------------------------
# Catalog bridge
# ---------------------------------------------------------------------------


def test_bridge_resolve_unknown_raises() -> None:
    async def scenario() -> None:
        bridge = CatalogBridge(store=InMemoryConeBindingStore())
        with pytest.raises(ConeRefNotBound):
            await bridge.resolve(tenant_id="t", feature_id="f")

    asyncio.run(scenario())


def test_bridge_register_and_resolve() -> None:
    async def scenario() -> None:
        bridge = CatalogBridge(store=InMemoryConeBindingStore())
        ref = ConeRef("t", "k", 1)
        await bridge.register_binding(
            tenant_id="t",
            feature_id="f",
            task_id=None,
            cone_ref=ref,
            embedder_family="st",
            embedder_version="2",
            anchor_count=3,
            labels=("fraud", "billing", "tech"),
            now=datetime(2026, 4, 23, tzinfo=timezone.utc),
        )
        got = await bridge.resolve(tenant_id="t", feature_id="f")
        assert got == ref
        labels = await bridge.labels_for(tenant_id="t", feature_id="f")
        assert labels == ("fraud", "billing", "tech")

    asyncio.run(scenario())


def test_catalog_validator_rejects_pii() -> None:
    catalog = CatalogYaml(
        catalog_id="c",
        version=1,
        tenant="acme",
        task="triage",
        embedder=EmbedderYaml("st", "m", "1"),
        dim=4,
        labels=(
            LabelYaml(
                label="fraud",
                anchors=("call alice@example.com for fraud",),
            ),
        ),
    )

    def linter(text: str) -> list[str]:
        if "@" in text:
            return ["email"]
        return []

    with pytest.raises(PiiDetectedInPrompt):
        validate_catalog_yaml(catalog, pii_linter=linter)


def test_catalog_compiler_happy() -> None:
    class FakeEmbedder:
        async def embed(self, *texts: str) -> list[tuple[float, ...]]:
            return [(0.1, 0.2, 0.3, 0.4)] * len(texts)

    async def scenario() -> None:
        catalog = CatalogYaml(
            catalog_id="c",
            version=1,
            tenant="acme",
            task="triage",
            embedder=EmbedderYaml("st", "m", "1"),
            dim=4,
            labels=(
                LabelYaml("fraud", ("anchor fraud 1", "anchor fraud 2")),
                LabelYaml("billing", ("anchor billing 1",)),
            ),
        )
        compiler = CatalogCompiler(embedder=FakeEmbedder())
        spec, ordered = await compiler.compile(catalog, created_by="alice")
        assert spec.tenant == "acme"
        assert len(ordered) == 3  # 2 fraud + 1 billing
        assert ordered == ["fraud", "fraud", "billing"]

    asyncio.run(scenario())


def test_catalog_compiler_rejects_dim_mismatch() -> None:
    class BadEmbedder:
        async def embed(self, *texts: str) -> list[tuple[float, ...]]:
            return [(0.1, 0.2)] * len(texts)  # wrong dim

    async def scenario() -> None:
        catalog = CatalogYaml(
            catalog_id="c",
            version=1,
            tenant="t",
            task="k",
            embedder=EmbedderYaml("st", "m", "1"),
            dim=4,
            labels=(LabelYaml("a", ("anchor",)),),
        )
        compiler = CatalogCompiler(embedder=BadEmbedder())
        with pytest.raises(CatalogCompilationFailed):
            await compiler.compile(catalog, created_by="alice")

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Artefact sync
# ---------------------------------------------------------------------------


def test_manifest_seal_roundtrip() -> None:
    digest = compute_sha256(b"hello")
    m = ArtefactManifest(
        manifest_id="m1",
        tenant_id="t1",
        artefact_id="a1",
        digest=digest,
        dim=4,
        tokenizer_family="bpe",
        source_bucket="src",
        target_bucket="dst",
        synced_at=datetime(2026, 4, 23, tzinfo=timezone.utc),
    )
    sealed = seal_manifest(m)
    assert verify_manifest_seal(sealed)


def test_artefact_syncer_detects_digest_mismatch() -> None:
    class FakeSource:
        async def iter_chunks(self, *, bucket: str, key: str):
            async def gen():
                yield b"different content"
            return gen()

    class FakeTarget:
        async def put(self, *, bucket: str, key: str, chunks):
            list(chunks)

    async def scenario() -> None:
        syncer = ArtefactSyncer(
            source=FakeSource(), target=FakeTarget(), target_bucket="dst"
        )
        expected = compute_sha256(b"hello")  # wrong digest
        with pytest.raises(ArtefactDigestMismatch):
            await syncer.sync(
                tenant_id="t",
                artefact_id="a",
                source_bucket="src",
                source_key="k",
                expected_digest=expected,
                dim=4,
            )

    asyncio.run(scenario())


def test_artefact_syncer_happy() -> None:
    payload = b"some-artifact-bytes"

    class FakeSource:
        async def iter_chunks(self, *, bucket: str, key: str):
            async def gen():
                yield payload
            return gen()

    class FakeTarget:
        def __init__(self) -> None:
            self.put_calls: list[tuple[str, str]] = []

        async def put(self, *, bucket: str, key: str, chunks):
            self.put_calls.append((bucket, key))
            list(chunks)

    async def scenario() -> None:
        tgt = FakeTarget()
        syncer = ArtefactSyncer(
            source=FakeSource(), target=tgt, target_bucket="dst"
        )
        outcome = await syncer.sync(
            tenant_id="t",
            artefact_id="a",
            source_bucket="src",
            source_key="k",
            expected_digest=compute_sha256(payload),
            dim=4,
        )
        assert outcome.ok
        assert outcome.manifest is not None
        assert verify_manifest_seal(outcome.manifest)
        assert tgt.put_calls == [("dst", "t/a")]

    asyncio.run(scenario())
