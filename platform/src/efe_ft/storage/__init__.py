"""Blob storage abstraction — local filesystem or S3.

The interface is intentionally minimal:
    put(key, bytes) -> checksum
    get(key) -> bytes
    delete(key)
    exists(key) -> bool

Layer-switch via :class:`LocalBlobStore` or :class:`S3BlobStore`.
Tests and dev use the local store; production points to S3.
"""

from __future__ import annotations

from efe_ft.storage.s3 import BlobStore, LocalBlobStore, S3BlobStore, build_store

__all__ = ["BlobStore", "LocalBlobStore", "S3BlobStore", "build_store"]
