"""Dataset curation — builds a signed manifest.

This is a *planning* module : it constructs a :class:`DatasetManifest`
description that downstream (M05) will materialise. Actual row-level
deduplication / shuffling / PII scrub happen in M05 ; here we only
produce the plan + the HMAC seal so later stages can verify integrity.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any

from efe_scheduling.hashing import (
    DOMAIN_TAG_DATASET_SEAL,
    DOMAIN_TAG_DATASET_SEED,
    canonical_json,
    hmac_seal,
)

__all__ = [
    "CuratorInputs",
    "CuratorOutcome",
    "DatasetManifest",
    "build_dataset_manifest",
]


@dataclass(frozen=True, slots=True)
class CuratorInputs:
    tenant_id: str
    task_id: str | None
    training_cycle_id: str
    window_end: datetime
    window_days: int = 30
    stratify_keys: tuple[str, ...] = ("task_id",)
    dedup_keys: tuple[str, ...] = ("prompt_hash", "response_hash")
    max_rows: int = 200_000
    pii_profile: str = "default"

    def __post_init__(self) -> None:
        if self.window_end.tzinfo is None:
            raise ValueError("window_end must be tz-aware")
        if self.window_days <= 0:
            raise ValueError("window_days > 0")
        if self.max_rows <= 0:
            raise ValueError("max_rows > 0")


@dataclass(frozen=True, slots=True)
class DatasetManifest:
    tenant_id: str
    task_id: str | None
    training_cycle_id: str
    window_start: datetime
    window_end: datetime
    stratify_keys: tuple[str, ...]
    dedup_keys: tuple[str, ...]
    max_rows: int
    pii_profile: str
    seed: bytes
    seal: bytes

    def to_dict(self) -> dict[str, Any]:
        return {
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "training_cycle_id": self.training_cycle_id,
            "window_start": self.window_start.astimezone(timezone.utc).isoformat(),
            "window_end": self.window_end.astimezone(timezone.utc).isoformat(),
            "stratify_keys": list(self.stratify_keys),
            "dedup_keys": list(self.dedup_keys),
            "max_rows": self.max_rows,
            "pii_profile": self.pii_profile,
            "seed": self.seed.hex(),
            "seal": self.seal.hex(),
        }


@dataclass(frozen=True, slots=True)
class CuratorOutcome:
    manifest: DatasetManifest
    size_estimate: int
    warnings: list[str] = field(default_factory=list)


def build_dataset_manifest(inputs: CuratorInputs) -> CuratorOutcome:
    window_start = inputs.window_end - timedelta(days=inputs.window_days)

    # Deterministic seed derived from training_cycle_id.
    seed_payload = {
        "tenant_id": inputs.tenant_id,
        "task_id": inputs.task_id,
        "training_cycle_id": inputs.training_cycle_id,
    }
    seed = hmac_seal(DOMAIN_TAG_DATASET_SEED, seed_payload)

    core_payload = {
        "tenant_id": inputs.tenant_id,
        "task_id": inputs.task_id,
        "training_cycle_id": inputs.training_cycle_id,
        "window_start": window_start.astimezone(timezone.utc).isoformat(),
        "window_end": inputs.window_end.astimezone(timezone.utc).isoformat(),
        "stratify_keys": list(inputs.stratify_keys),
        "dedup_keys": list(inputs.dedup_keys),
        "max_rows": inputs.max_rows,
        "pii_profile": inputs.pii_profile,
        "seed": seed.hex(),
    }
    seal = hmac_seal(DOMAIN_TAG_DATASET_SEAL, core_payload)

    manifest = DatasetManifest(
        tenant_id=inputs.tenant_id,
        task_id=inputs.task_id,
        training_cycle_id=inputs.training_cycle_id,
        window_start=window_start,
        window_end=inputs.window_end,
        stratify_keys=inputs.stratify_keys,
        dedup_keys=inputs.dedup_keys,
        max_rows=inputs.max_rows,
        pii_profile=inputs.pii_profile,
        seed=seed,
        seal=seal,
    )

    warnings: list[str] = []
    size_estimate = inputs.max_rows  # placeholder — real estimate done by M05
    if inputs.window_days > 90:
        warnings.append("window_days > 90 : dataset may be stale")
    _ = canonical_json  # keep import
    return CuratorOutcome(manifest=manifest, size_estimate=size_estimate, warnings=warnings)
