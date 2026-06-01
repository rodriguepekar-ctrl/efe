"""Tests for :mod:`efe_worker.transport.s3`."""

from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

from efe_worker.errors import S3Checksum, S3KeyNotFound, StorageError
from efe_worker.transport.s3 import (
    FakeBlobStore,
    LocalBlobStore,
    get_with_sha256,
)


async def test_fake_put_get() -> None:
    s = FakeBlobStore()
    await s.put("a", b"hello")
    got = await s.get("a")
    assert got == b"hello"
    assert s.put_calls == ["a"]
    assert s.get_calls == ["a"]


async def test_fake_get_missing() -> None:
    s = FakeBlobStore()
    with pytest.raises(S3KeyNotFound):
        await s.get("nope")


async def test_fake_list_prefix() -> None:
    s = FakeBlobStore()
    await s.put("pre/a", b"1")
    await s.put("pre/b", b"2")
    await s.put("other", b"3")
    prefixed = await s.list_prefix("pre/")
    assert prefixed == ["pre/a", "pre/b"]


async def test_fake_delete() -> None:
    s = FakeBlobStore()
    await s.put("x", b"1")
    assert await s.exists("x") is True
    await s.delete("x")
    assert await s.exists("x") is False


async def test_fake_forced_failures() -> None:
    s = FakeBlobStore(fail_put=True)
    with pytest.raises(StorageError):
        await s.put("x", b"1")
    s2 = FakeBlobStore(fail_get=True)
    await s2.put("x", b"1")
    with pytest.raises(StorageError):
        await s2.get("x")


async def test_local_roundtrip(tmp_path: Path) -> None:
    s = LocalBlobStore(tmp_path / "root")
    await s.put("k", b"data")
    got = await s.get("k")
    assert got == b"data"


async def test_local_uri_relative_resolves_under_root(
    tmp_path: Path,
) -> None:
    root = tmp_path / "root"
    s = LocalBlobStore(root)
    await s.put("blobs/x", b"data")
    assert (root / "blobs" / "x").is_file()
    got = await s.get("blobs/x")
    assert got == b"data"


async def test_local_uri_scheme_stripped(tmp_path: Path) -> None:
    """``file://`` prefix is stripped; the remaining path is absolute."""
    root = tmp_path / "root"
    s = LocalBlobStore(root)
    # Use an absolute path already under root so the ``file://`` prefix
    # resolves to an existing file.
    target = root / "inside.bin"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(b"hi")
    got = await s.get(f"file://{target}")
    assert got == b"hi"


async def test_local_get_missing(tmp_path: Path) -> None:
    s = LocalBlobStore(tmp_path / "root")
    with pytest.raises(S3KeyNotFound):
        await s.get("missing")


async def test_local_exists(tmp_path: Path) -> None:
    s = LocalBlobStore(tmp_path / "root")
    await s.put("k", b"v")
    assert await s.exists("k")
    assert not await s.exists("absent")


async def test_local_delete(tmp_path: Path) -> None:
    s = LocalBlobStore(tmp_path / "root")
    await s.put("k", b"v")
    await s.delete("k")
    assert not await s.exists("k")


async def test_get_with_sha256_ok() -> None:
    s = FakeBlobStore()
    data = b"payload"
    await s.put("u", data)
    h = hashlib.sha256(data).hexdigest()
    ok = await get_with_sha256(s, "u", h)
    assert ok == data


async def test_get_with_sha256_mismatch() -> None:
    s = FakeBlobStore()
    await s.put("u", b"payload")
    with pytest.raises(S3Checksum):
        await get_with_sha256(s, "u", "0" * 64)


async def test_local_list_prefix(tmp_path: Path) -> None:
    s = LocalBlobStore(tmp_path / "root")
    (tmp_path / "root" / "foo").mkdir(parents=True, exist_ok=True)
    (tmp_path / "root" / "foo" / "a.txt").write_bytes(b"a")
    (tmp_path / "root" / "foo" / "b.txt").write_bytes(b"b")
    out = await s.list_prefix("foo/")
    assert any(p.endswith("a.txt") for p in out)
    assert any(p.endswith("b.txt") for p in out)
