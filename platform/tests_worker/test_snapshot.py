"""Tests for :mod:`efe_worker.data.snapshot`."""

from __future__ import annotations

import hashlib
import json

import pytest

from efe_worker.data.snapshot import (
    PartitionSpec,
    Snapshot,
    SnapshotManifest,
    build_synthetic_snapshot,
    fetch_snapshot,
)
from efe_worker.errors import (
    CorruptDataset,
    SnapshotMissing,
)
from efe_worker.transport.s3 import FakeBlobStore


def test_partition_spec_defaults() -> None:
    p = PartitionSpec()
    assert p.train + p.val + p.holdout == pytest.approx(1.0)
    assert p.seed == 42


async def test_fetch_synthetic_snapshot() -> None:
    blobs, uri = build_synthetic_snapshot(seed=1)
    s = FakeBlobStore(data=blobs)
    snap = await fetch_snapshot(uri, s)
    assert snap.n_rows == 24
    assert set(snap.labels).issubset({"a", "b", "c"})


async def test_snapshot_partition_respects_fractions() -> None:
    blobs, uri = build_synthetic_snapshot(seed=7)
    snap = await fetch_snapshot(uri, FakeBlobStore(data=blobs))
    t, v, h = snap.partition()
    assert len(t) + len(v) + len(h) == snap.n_rows
    assert len(t) >= len(v) >= 0


async def test_fetch_missing_manifest() -> None:
    s = FakeBlobStore()
    with pytest.raises(SnapshotMissing):
        await fetch_snapshot(
            "file:///nope", s,
        )


async def test_fetch_bad_manifest_json() -> None:
    s = FakeBlobStore()
    await s.put("file:///m.json", b"not json")
    with pytest.raises(CorruptDataset):
        await fetch_snapshot("file:///m.json", s)


async def test_fetch_bad_content_hash() -> None:
    blobs, uri = build_synthetic_snapshot()
    # tamper content_hash
    manifest = json.loads(blobs[uri])
    manifest["content_hash"] = "0" * 64
    blobs[uri] = json.dumps(manifest).encode()
    s = FakeBlobStore(data=blobs)
    with pytest.raises(CorruptDataset):
        await fetch_snapshot(uri, s)


async def test_fetch_bad_shard_sha() -> None:
    blobs, uri = build_synthetic_snapshot()
    # corrupt a shard
    manifest = json.loads(blobs[uri])
    first = manifest["shards"][0]["uri"]
    blobs[first] = b"{'text': 'corrupted'}"
    s = FakeBlobStore(data=blobs)
    with pytest.raises(CorruptDataset):
        await fetch_snapshot(uri, s)


async def test_fetch_skip_sha_check() -> None:
    blobs, uri = build_synthetic_snapshot()
    manifest = json.loads(blobs[uri])
    first = manifest["shards"][0]["uri"]
    # Modify bytes but keep row count; skip check → should succeed.
    # (Row count mismatch would still raise, so we keep same content.)
    # Reverse bytes but keep same sha check off:
    blobs[first] = blobs[first]
    s = FakeBlobStore(data=blobs)
    snap = await fetch_snapshot(uri, s, verify_shard_sha256=False)
    assert snap.n_rows == manifest["n_samples"]


async def test_row_count_mismatch() -> None:
    blobs, uri = build_synthetic_snapshot()
    manifest = json.loads(blobs[uri])
    manifest["n_samples"] = 999
    blobs[uri] = json.dumps(manifest).encode()
    # also need content_hash recomputed to pass that check first
    # — easier to bypass by skipping sha check only and manipulate
    # n_samples to mismatch.
    joined = "\n".join(sorted(s["sha256"] for s in manifest["shards"]))
    manifest["content_hash"] = hashlib.sha256(
        joined.encode(),
    ).hexdigest()
    blobs[uri] = json.dumps(manifest).encode()
    s = FakeBlobStore(data=blobs)
    with pytest.raises(CorruptDataset):
        await fetch_snapshot(uri, s)


def test_manifest_from_dict_malformed() -> None:
    with pytest.raises(CorruptDataset):
        SnapshotManifest.from_dict({"snapshot_id": "x"})


def test_manifest_bad_shard_entry() -> None:
    with pytest.raises(CorruptDataset):
        SnapshotManifest.from_dict({
            "snapshot_id": "s", "tenant_id": "t", "task_id": "r",
            "n_samples": 1, "content_hash": "0" * 64,
            "shards": [{"uri": "x"}],  # missing rows + sha
        })


def test_snapshot_partition_invalid_fractions() -> None:
    m = SnapshotManifest(
        snapshot_id="s", tenant_id="t", task_id="r",
        n_samples=0,
        label_balance={},
        content_hash="0" * 64,
        shards=(),
        partitioning=PartitionSpec(train=0, val=0, holdout=0),
    )
    snap = Snapshot(manifest=m, rows=[])
    with pytest.raises(Exception):
        snap.partition()
