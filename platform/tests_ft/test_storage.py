"""Tests for the blob-store abstraction (LocalBlobStore)."""

from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

from efe_ft.errors import StorageError
from efe_ft.storage import LocalBlobStore, build_store


async def test_put_get_roundtrip(tmp_path: Path) -> None:
    store = LocalBlobStore(tmp_path / "blobs")
    checksum = await store.put("tenant/a/key.npy", b"payload-bytes")
    assert checksum == hashlib.sha256(b"payload-bytes").digest()
    got = await store.get("tenant/a/key.npy")
    assert got == b"payload-bytes"


async def test_exists(tmp_path: Path) -> None:
    store = LocalBlobStore(tmp_path / "blobs")
    assert not await store.exists("nope")
    await store.put("yes", b"x")
    assert await store.exists("yes")


async def test_delete(tmp_path: Path) -> None:
    store = LocalBlobStore(tmp_path / "blobs")
    await store.put("k", b"v")
    assert await store.exists("k")
    await store.delete("k")
    assert not await store.exists("k")
    # Deleting a missing key is a no-op.
    await store.delete("ghost")


async def test_get_missing_raises(tmp_path: Path) -> None:
    store = LocalBlobStore(tmp_path / "blobs")
    with pytest.raises(StorageError):
        await store.get("missing")


async def test_put_overwrites(tmp_path: Path) -> None:
    store = LocalBlobStore(tmp_path / "blobs")
    await store.put("k", b"first")
    await store.put("k", b"second")
    assert await store.get("k") == b"second"


async def test_build_store_local(tmp_path: Path) -> None:
    store = build_store(backend="local", local_root=str(tmp_path / "z"))
    assert isinstance(store, LocalBlobStore)


async def test_build_store_unknown_backend() -> None:
    with pytest.raises(StorageError, match="unknown backend"):
        build_store(backend="alien", local_root="/tmp/x")


async def test_build_store_s3_without_bucket() -> None:
    with pytest.raises(StorageError, match="s3_bucket"):
        build_store(backend="s3", local_root="/tmp/x")
