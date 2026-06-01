"""Snapshot ingestion.

A snapshot is produced by M01 and shipped to the worker as:

* a ``manifest.json`` at some S3 URI,
* N parquet shards referenced inside the manifest with a sha256 digest
  per shard,
* a ``content_hash`` that is the sha256 of the sorted list of shard
  sha256s (defensive check against manifest tampering).

The worker's contract: ``fetch_snapshot(manifest_uri, blob_store)``
returns a fully-materialised :class:`Snapshot` with every row in
memory. We keep this simple for the MVP — dataset sizes top out in
the low millions of rows and each row is short text.

For parquet we rely on ``pyarrow`` *only if present*; tests install
a lightweight "fake-parquet" mode that stores JSON lines instead.
"""

from __future__ import annotations

import hashlib
import io
import json
from collections.abc import Iterable
from dataclasses import dataclass, field
from typing import Any

from efe_worker.errors import (
    CorruptDataset,
    DataError,
    MissingDependency,
    SnapshotMissing,
    StorageError,
)
from efe_worker.hashing import coerce_hex, sha256_hex
from efe_worker.transport.s3 import BlobStore

# ---------------------------------------------------------------------------
# dataclasses
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ShardRef:
    uri: str
    rows: int
    sha256: str  # hex


@dataclass(frozen=True, slots=True)
class PartitionSpec:
    train: float = 0.8
    val: float = 0.1
    holdout: float = 0.1
    seed: int = 42


@dataclass(frozen=True, slots=True)
class SnapshotManifest:
    snapshot_id: str
    tenant_id: str
    task_id: str
    n_samples: int
    label_balance: dict[str, int]
    content_hash: str  # hex
    shards: tuple[ShardRef, ...]
    partitioning: PartitionSpec = field(default_factory=PartitionSpec)

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> SnapshotManifest:
        try:
            shards = tuple(
                ShardRef(
                    uri=str(s["uri"]),
                    rows=int(s["rows"]),
                    sha256=str(s["sha256"]).lower(),
                )
                for s in raw["shards"]
            )
        except (KeyError, TypeError, ValueError) as exc:
            raise CorruptDataset(
                f"malformed manifest shards: {exc}",
            ) from exc
        part_raw = raw.get("partitioning") or {}
        partitioning = PartitionSpec(
            train=float(part_raw.get("train", 0.8)),
            val=float(part_raw.get("val", 0.1)),
            holdout=float(part_raw.get("holdout", 0.1)),
            seed=int(part_raw.get("seed", 42)),
        )
        try:
            return cls(
                snapshot_id=str(raw["snapshot_id"]),
                tenant_id=str(raw["tenant_id"]),
                task_id=str(raw["task_id"]),
                n_samples=int(raw["n_samples"]),
                label_balance=dict(raw.get("label_balance") or {}),
                content_hash=str(raw["content_hash"]).lower(),
                shards=shards,
                partitioning=partitioning,
            )
        except (KeyError, TypeError, ValueError) as exc:
            raise CorruptDataset(
                f"malformed manifest: {exc}",
            ) from exc


@dataclass(frozen=True, slots=True)
class Row:
    text: str
    label: str | None
    meta: dict[str, Any] = field(default_factory=dict)


@dataclass
class Snapshot:
    manifest: SnapshotManifest
    rows: list[Row]

    @property
    def n_rows(self) -> int:
        return len(self.rows)

    @property
    def labels(self) -> tuple[str, ...]:
        """Sorted unique labels (excludes None)."""
        seen = {r.label for r in self.rows if r.label is not None}
        return tuple(sorted(seen))

    def partition(
        self,
    ) -> tuple[list[Row], list[Row], list[Row]]:
        """Split rows into (train, val, holdout) deterministically.

        Uses a seeded random for reproducibility. The split respects
        the fractions in :class:`PartitionSpec` — sum is normalised
        to 1.0.
        """
        import random

        total = (
            self.manifest.partitioning.train
            + self.manifest.partitioning.val
            + self.manifest.partitioning.holdout
        )
        if total <= 0:
            raise DataError("partition fractions must be positive")
        t = self.manifest.partitioning.train / total
        v = self.manifest.partitioning.val / total
        rng = random.Random(self.manifest.partitioning.seed)
        indices = list(range(self.n_rows))
        rng.shuffle(indices)
        n = self.n_rows
        n_train = round(n * t)
        n_val = round(n * v)
        train = [self.rows[i] for i in indices[:n_train]]
        val = [self.rows[i] for i in indices[n_train : n_train + n_val]]
        holdout = [self.rows[i] for i in indices[n_train + n_val :]]
        return train, val, holdout


# ---------------------------------------------------------------------------
# fetch + verify
# ---------------------------------------------------------------------------


async def fetch_snapshot(
    manifest_uri: str,
    blob_store: BlobStore,
    *,
    verify_shard_sha256: bool = True,
) -> Snapshot:
    """Download + parse + verify a full snapshot.

    On any checksum mismatch we raise :class:`CorruptDataset`, which
    the driver classifies as a non-retryable DATA error (the run goes
    FAILED, never retries).
    """
    # 1. manifest
    try:
        raw = await blob_store.get(manifest_uri)
    except StorageError as exc:
        raise SnapshotMissing(
            f"manifest not readable at {manifest_uri!r}: {exc}",
        ) from exc
    try:
        parsed = json.loads(raw.decode("utf-8"))
    except json.JSONDecodeError as exc:
        raise CorruptDataset(
            f"manifest at {manifest_uri!r} is not valid JSON: {exc}",
        ) from exc
    manifest = SnapshotManifest.from_dict(parsed)

    # 2. verify content_hash = sha256(sorted shard sha256s joined by \n)
    joined = "\n".join(sorted(s.sha256 for s in manifest.shards))
    recomputed = sha256_hex(joined)
    if recomputed != manifest.content_hash:
        raise CorruptDataset(
            f"content_hash mismatch for {manifest.snapshot_id!r}",
            detail={
                "expected": manifest.content_hash,
                "observed": recomputed,
            },
        )

    # 3. fetch + decode each shard
    rows: list[Row] = []
    for shard in manifest.shards:
        blob = await blob_store.get(shard.uri)
        if verify_shard_sha256:
            observed = hashlib.sha256(blob).hexdigest()
            if observed != shard.sha256:
                raise CorruptDataset(
                    f"shard sha256 mismatch at {shard.uri!r}",
                    detail={
                        "expected": coerce_hex(shard.sha256),
                        "observed": observed,
                    },
                )
        for r in _decode_shard(blob, shard.uri):
            rows.append(r)

    if len(rows) != manifest.n_samples:
        raise CorruptDataset(
            f"row count mismatch: manifest says {manifest.n_samples}, "
            f"shards yielded {len(rows)}",
        )
    return Snapshot(manifest=manifest, rows=rows)


# ---------------------------------------------------------------------------
# shard decoders
# ---------------------------------------------------------------------------


def _decode_shard(blob: bytes, uri: str) -> Iterable[Row]:
    """Decode one shard based on its extension (parquet | jsonl)."""
    # Fast path: JSON-lines — ``uri`` ends in ``.jsonl`` or begins with
    # the magic ``{"text"`` byte prefix. Used by tests + dev fixtures.
    if uri.endswith(".jsonl") or (blob.lstrip().startswith(b"{")):
        yield from _decode_jsonl(blob)
        return
    # Otherwise assume parquet.
    try:
        import pyarrow.parquet as pq
    except ImportError as exc:  # pragma: no cover — optional dep
        raise MissingDependency(
            "pyarrow is required to read parquet shards "
            "(or use .jsonl fixtures for tests).",
        ) from exc
    table = pq.read_table(io.BytesIO(blob))
    data = table.to_pydict()
    texts = data.get("text") or data.get("content") or []
    labels = data.get("label") or [None] * len(texts)
    metas = data.get("meta") or [{}] * len(texts)
    for t, lab, m in zip(texts, labels, metas, strict=False):
        meta = dict(m) if isinstance(m, dict) else {}
        yield Row(
            text=str(t),
            label=None if lab is None else str(lab),
            meta=meta,
        )


def _decode_jsonl(blob: bytes) -> Iterable[Row]:
    for line in blob.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line.decode("utf-8"))
        except json.JSONDecodeError as exc:
            raise CorruptDataset(
                f"bad jsonl line: {exc}",
            ) from exc
        yield Row(
            text=str(obj.get("text", "")),
            label=(None if obj.get("label") is None
                   else str(obj["label"])),
            meta=dict(obj.get("meta") or {}),
        )


# ---------------------------------------------------------------------------
# encoder helpers for tests — build a valid manifest + shards in-memory
# ---------------------------------------------------------------------------


def build_synthetic_snapshot(
    *,
    tenant_id: str = "acme",
    task_id: str = "routing",
    snapshot_id: str = "sn-1",
    rows: Iterable[tuple[str, str | None]] | None = None,
    seed: int = 42,
) -> tuple[dict[str, bytes], str]:
    """Build a fake snapshot as a dict of ``{uri: bytes}``.

    Returns the blob dict and the manifest URI. Callers feed the dict
    into a :class:`FakeBlobStore` and pass the manifest URI to
    :func:`fetch_snapshot`. Used extensively in tests.
    """
    import random as _rand

    if rows is None:
        rnd = _rand.Random(seed)
        labels = ["a", "b", "c"]
        rows = [
            (f"sample {i} text", rnd.choice(labels))
            for i in range(24)
        ]
    rows_list = list(rows)
    # one shard per 10 rows
    shard_blobs: dict[str, bytes] = {}
    shard_refs: list[dict[str, Any]] = []
    chunk = 10
    for idx, i in enumerate(range(0, len(rows_list), chunk)):
        batch = rows_list[i : i + chunk]
        lines = []
        for text, lab in batch:
            lines.append(json.dumps(
                {"text": text, "label": lab},
                ensure_ascii=False,
            ))
        blob = ("\n".join(lines) + "\n").encode("utf-8")
        uri = f"file:///snap/{snapshot_id}/shard-{idx:03d}.jsonl"
        shard_blobs[uri] = blob
        shard_refs.append({
            "uri": uri,
            "rows": len(batch),
            "sha256": hashlib.sha256(blob).hexdigest(),
        })
    label_balance: dict[str, int] = {}
    for _, lab in rows_list:
        if lab is None:
            continue
        label_balance[lab] = label_balance.get(lab, 0) + 1
    joined = "\n".join(sorted(s["sha256"] for s in shard_refs))
    content_hash = sha256_hex(joined)
    manifest = {
        "snapshot_id": snapshot_id,
        "tenant_id": tenant_id,
        "task_id": task_id,
        "n_samples": len(rows_list),
        "label_balance": label_balance,
        "content_hash": content_hash,
        "shards": shard_refs,
        "partitioning": {
            "train": 0.7, "val": 0.15, "holdout": 0.15, "seed": seed,
        },
    }
    manifest_uri = f"file:///snap/{snapshot_id}/manifest.json"
    shard_blobs[manifest_uri] = json.dumps(manifest).encode("utf-8")
    return shard_blobs, manifest_uri


__all__ = [
    "PartitionSpec",
    "Row",
    "ShardRef",
    "Snapshot",
    "SnapshotManifest",
    "build_synthetic_snapshot",
    "fetch_snapshot",
]
