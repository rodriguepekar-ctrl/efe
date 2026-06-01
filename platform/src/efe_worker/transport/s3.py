"""Blob store — snapshot fetch + artifact push.

Three concrete implementations:

* :class:`FakeBlobStore` — in-memory dict of ``{uri: bytes}`` for
  tests; also tracks every ``put`` / ``get`` / ``list`` call so
  assertions are easy.
* :class:`LocalBlobStore` — filesystem-backed, accepts ``file://...``
  and plain path URIs. Used for local dev + CI without MinIO.
* :class:`S3BlobStore` — real S3/MinIO via :mod:`boto3`. Guarded
  behind a lazy import so the worker can still boot in environments
  where boto3 is missing (tests, CI).

All three implement the :class:`BlobStore` protocol.

Every ``get_with_sha256`` call verifies the downloaded bytes against
the caller-supplied expected digest; on mismatch we raise
:class:`S3Checksum` so the driver can abort cleanly.
"""

from __future__ import annotations

import hashlib
import os
import shutil
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Protocol

from efe_worker.errors import S3Checksum, S3KeyNotFound, StorageError
from efe_worker.hashing import coerce_hex


class BlobStore(Protocol):
    async def get(self, uri: str) -> bytes:  # pragma: no cover
        ...

    async def put(self, uri: str, data: bytes) -> None:  # pragma: no cover
        ...

    async def exists(self, uri: str) -> bool:  # pragma: no cover
        ...

    async def list_prefix(
        self, prefix_uri: str,
    ) -> list[str]:  # pragma: no cover
        ...

    async def delete(self, uri: str) -> None:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# shared helper — checksum-verifying get
# ---------------------------------------------------------------------------


async def get_with_sha256(
    store: BlobStore, uri: str, expected: Any,
) -> bytes:
    """Fetch a blob and enforce its sha256 matches ``expected``.

    ``expected`` accepts hex str, raw 32-byte digest, or bytearray.
    """
    exp_hex = coerce_hex(expected)
    data = await store.get(uri)
    observed = hashlib.sha256(data).hexdigest()
    if observed != exp_hex:
        raise S3Checksum(
            f"checksum mismatch for {uri!r}",
            detail={
                "uri": uri,
                "expected": exp_hex,
                "observed": observed,
            },
        )
    return data


# ---------------------------------------------------------------------------
# Fake (tests)
# ---------------------------------------------------------------------------


@dataclass
class FakeBlobStore:
    """In-memory blob store.

    URIs are opaque keys; anything goes (``s3://``, ``file://``, bare
    strings). The fake keeps:

    * ``data`` — keyed bytes
    * ``put_calls``, ``get_calls``, ``delete_calls`` — audit lists
      used by tests to assert call ordering.
    * ``fail_put``, ``fail_get`` — toggle synthetic failures.
    """

    data: dict[str, bytes] = field(default_factory=dict)
    put_calls: list[str] = field(default_factory=list)
    get_calls: list[str] = field(default_factory=list)
    delete_calls: list[str] = field(default_factory=list)
    fail_put: bool = False
    fail_get: bool = False

    async def get(self, uri: str) -> bytes:
        self.get_calls.append(uri)
        if self.fail_get:
            raise StorageError("fake store configured to fail on get")
        if uri not in self.data:
            raise S3KeyNotFound(f"not in fake store: {uri!r}")
        return self.data[uri]

    async def put(self, uri: str, data: bytes) -> None:
        self.put_calls.append(uri)
        if self.fail_put:
            raise StorageError("fake store configured to fail on put")
        self.data[uri] = bytes(data)

    async def exists(self, uri: str) -> bool:
        return uri in self.data

    async def list_prefix(self, prefix_uri: str) -> list[str]:
        return sorted(k for k in self.data if k.startswith(prefix_uri))

    async def delete(self, uri: str) -> None:
        self.delete_calls.append(uri)
        self.data.pop(uri, None)


# ---------------------------------------------------------------------------
# Local FS
# ---------------------------------------------------------------------------


class LocalBlobStore:
    """Filesystem-backed store.

    URIs have the form ``file:///absolute/path`` or ``/absolute/path``
    (the leading ``file://`` is stripped). All paths are resolved
    relative to :attr:`root` if they are not absolute.
    """

    def __init__(self, root: str | Path) -> None:
        self._root = Path(root).expanduser().resolve()
        self._root.mkdir(parents=True, exist_ok=True)

    def _resolve(self, uri: str) -> Path:
        path = uri
        if path.startswith("file://"):
            path = path[len("file://"):]
        p = Path(path)
        if not p.is_absolute():
            p = self._root / p
        return p

    async def get(self, uri: str) -> bytes:
        p = self._resolve(uri)
        if not p.is_file():
            raise S3KeyNotFound(f"not a file: {uri!r}")
        try:
            return p.read_bytes()
        except OSError as exc:
            raise StorageError(
                f"read failed for {uri!r}: {exc}",
            ) from exc

    async def put(self, uri: str, data: bytes) -> None:
        p = self._resolve(uri)
        p.parent.mkdir(parents=True, exist_ok=True)
        try:
            p.write_bytes(data)
        except OSError as exc:
            raise StorageError(
                f"write failed for {uri!r}: {exc}",
            ) from exc

    async def exists(self, uri: str) -> bool:
        return self._resolve(uri).exists()

    async def list_prefix(self, prefix_uri: str) -> list[str]:
        p = self._resolve(prefix_uri)
        if p.is_dir():
            base = p
        else:
            base = p.parent
        if not base.exists():
            return []
        out: list[str] = []
        for child in _walk(base):
            # represent as file:// URI
            out.append(f"file://{child}")
        return sorted(out)

    async def delete(self, uri: str) -> None:
        p = self._resolve(uri)
        try:
            if p.is_dir():
                shutil.rmtree(p)
            elif p.exists():
                p.unlink()
        except OSError as exc:
            raise StorageError(
                f"delete failed for {uri!r}: {exc}",
            ) from exc


def _walk(root: Path) -> Iterator[Path]:
    for dirpath, _dirnames, filenames in os.walk(root):
        for fn in filenames:
            yield Path(dirpath) / fn


# ---------------------------------------------------------------------------
# Real S3 (lazy)
# ---------------------------------------------------------------------------


class S3BlobStore:
    """Real S3 / MinIO client. Lazy-imports boto3."""

    def __init__(
        self,
        *,
        endpoint_url: str | None = None,
        region: str = "eu-west-3",
    ) -> None:
        try:
            import boto3
        except ImportError as exc:  # pragma: no cover — optional dep
            raise StorageError(
                "boto3 is required for S3BlobStore",
                detail={"install": "pip install boto3"},
            ) from exc
        self._boto3 = boto3
        self._endpoint = endpoint_url
        self._region = region
        self._client: Any | None = None

    def _ensure(self) -> Any:
        if self._client is None:
            self._client = self._boto3.client(
                "s3",
                endpoint_url=self._endpoint,
                region_name=self._region,
            )
        return self._client

    @staticmethod
    def _split(uri: str) -> tuple[str, str]:
        if not uri.startswith("s3://"):
            raise StorageError(f"not an s3:// uri: {uri!r}")
        rest = uri[len("s3://"):]
        if "/" not in rest:
            raise StorageError(f"s3 uri missing key: {uri!r}")
        bucket, _, key = rest.partition("/")
        return bucket, key

    async def get(self, uri: str) -> bytes:  # pragma: no cover — network
        bucket, key = self._split(uri)
        client = self._ensure()
        import asyncio
        try:
            resp = await asyncio.to_thread(
                client.get_object, Bucket=bucket, Key=key,
            )
            return bytes(resp["Body"].read())
        except Exception as exc:
            raise StorageError(f"S3 get failed for {uri!r}: {exc}") from exc

    async def put(
        self, uri: str, data: bytes,
    ) -> None:  # pragma: no cover — network
        bucket, key = self._split(uri)
        client = self._ensure()
        import asyncio
        await asyncio.to_thread(
            client.put_object, Bucket=bucket, Key=key, Body=data,
        )

    async def exists(self, uri: str) -> bool:  # pragma: no cover — network
        bucket, key = self._split(uri)
        client = self._ensure()
        import asyncio
        try:
            await asyncio.to_thread(
                client.head_object, Bucket=bucket, Key=key,
            )
            return True
        except Exception:
            return False

    async def list_prefix(
        self, prefix_uri: str,
    ) -> list[str]:  # pragma: no cover — network
        bucket, key = self._split(prefix_uri)
        client = self._ensure()
        import asyncio
        resp = await asyncio.to_thread(
            client.list_objects_v2, Bucket=bucket, Prefix=key,
        )
        out: list[str] = []
        for row in resp.get("Contents") or []:
            out.append(f"s3://{bucket}/{row['Key']}")
        return sorted(out)

    async def delete(self, uri: str) -> None:  # pragma: no cover — network
        bucket, key = self._split(uri)
        client = self._ensure()
        import asyncio
        await asyncio.to_thread(
            client.delete_object, Bucket=bucket, Key=key,
        )


__all__ = [
    "BlobStore",
    "FakeBlobStore",
    "LocalBlobStore",
    "S3BlobStore",
    "get_with_sha256",
]
