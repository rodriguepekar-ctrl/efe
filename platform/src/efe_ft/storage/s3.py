"""Two implementations of :class:`BlobStore`.

* :class:`LocalBlobStore` — writes under ``<root>/<key>``. Used in
  tests and standalone dev.
* :class:`S3BlobStore` — lazy-imports ``boto3`` if present, else
  raises at construction.

Keys may contain ``/`` — local paths are created on demand.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
from typing import Protocol, runtime_checkable

from efe_ft.errors import StorageError


@runtime_checkable
class BlobStore(Protocol):
    """Narrow surface all storage backends implement."""

    async def put(self, key: str, blob: bytes) -> bytes:
        """Write ``blob`` and return its sha256 checksum."""

    async def get(self, key: str) -> bytes: ...
    async def delete(self, key: str) -> None: ...
    async def exists(self, key: str) -> bool: ...


class LocalBlobStore:
    """Filesystem-backed store — good for tests and single-node dev."""

    def __init__(self, root: str | os.PathLike[str]) -> None:
        self._root = Path(root)
        self._root.mkdir(parents=True, exist_ok=True)

    def _path(self, key: str) -> Path:
        p = self._root / key
        p.parent.mkdir(parents=True, exist_ok=True)
        return p

    async def put(self, key: str, blob: bytes) -> bytes:
        path = self._path(key)
        try:
            # Write to a temp file then rename for atomicity.
            tmp = path.with_suffix(path.suffix + ".tmp")
            tmp.write_bytes(blob)
            tmp.replace(path)
        except OSError as exc:
            raise StorageError(f"local store put failed: {exc}") from exc
        return hashlib.sha256(blob).digest()

    async def get(self, key: str) -> bytes:
        path = self._path(key)
        if not path.exists():
            raise StorageError(f"key not found: {key}")
        return path.read_bytes()

    async def delete(self, key: str) -> None:
        path = self._path(key)
        if path.exists():
            path.unlink()

    async def exists(self, key: str) -> bool:
        return self._path(key).exists()


class S3BlobStore:
    """boto3-backed store. Instantiation fails if boto3 is missing."""

    def __init__(self, bucket: str, region: str, prefix: str = "") -> None:
        try:
            import boto3
        except ImportError as exc:  # pragma: no cover — optional dep
            raise StorageError(
                "boto3 is required for S3BlobStore. Install efe-ft[s3]."
            ) from exc
        self._bucket = bucket
        self._prefix = prefix.rstrip("/")
        # Use a synchronous boto3 client behind asyncio.to_thread.
        self._client = boto3.client("s3", region_name=region)

    def _key(self, key: str) -> str:
        return f"{self._prefix}/{key}" if self._prefix else key

    async def put(self, key: str, blob: bytes) -> bytes:
        import asyncio

        def _call() -> None:
            self._client.put_object(
                Bucket=self._bucket, Key=self._key(key), Body=blob,
            )

        await asyncio.to_thread(_call)
        return hashlib.sha256(blob).digest()

    async def get(self, key: str) -> bytes:
        import asyncio

        def _call() -> bytes:
            resp = self._client.get_object(
                Bucket=self._bucket, Key=self._key(key),
            )
            body: bytes = resp["Body"].read()
            return body

        return await asyncio.to_thread(_call)

    async def delete(self, key: str) -> None:
        import asyncio

        def _call() -> None:
            self._client.delete_object(
                Bucket=self._bucket, Key=self._key(key),
            )

        await asyncio.to_thread(_call)

    async def exists(self, key: str) -> bool:
        import asyncio

        def _call() -> bool:
            try:
                self._client.head_object(
                    Bucket=self._bucket, Key=self._key(key),
                )
                return True
            except self._client.exceptions.ClientError:
                return False

        return await asyncio.to_thread(_call)


def build_store(
    *,
    backend: str,
    local_root: str,
    s3_bucket: str | None = None,
    s3_region: str = "eu-west-3",
    s3_prefix: str = "efe-ft",
) -> BlobStore:
    if backend == "local":
        return LocalBlobStore(local_root)
    if backend == "s3":
        if not s3_bucket:
            raise StorageError("s3_bucket is required for S3BlobStore")
        return S3BlobStore(s3_bucket, s3_region, s3_prefix)
    raise StorageError(f"unknown backend {backend!r}")


__all__ = ["BlobStore", "LocalBlobStore", "S3BlobStore", "build_store"]
