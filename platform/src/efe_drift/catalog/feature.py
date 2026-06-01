"""Feature specification — input shape for :class:`FeatureCatalog`.

A :class:`FeatureSpec` is the *declaration* of a monitored feature —
it names the extractor responsible for producing the value, the
binning strategy used at baseline time, and the retention policy.

The spec is **versioned** : once a feature is in use, evolving its
extractor or kind requires a new version (``version+1``). The old
version remains readable for as long as baselines reference it.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from efe_drift.errors import (
    InvalidBinningStrategy,
    ValidationError,
)
from efe_drift.records import BinningStrategy, FeatureKind

_VALID_KINDS: frozenset[FeatureKind] = frozenset({
    "numeric", "categorical", "embedding", "binary",
})

_VALID_BINNINGS: frozenset[BinningStrategy] = frozenset({
    "equifrequency", "equiwidth", "custom", "none",
})

# For categorical / embedding / binary kinds some binning strategies
# make no sense — we refuse the spec up-front.
_ALLOWED_BINNING_BY_KIND: dict[FeatureKind, frozenset[BinningStrategy]] = {
    "numeric":     frozenset({"equifrequency", "equiwidth", "custom"}),
    "categorical": frozenset({"none"}),
    "embedding":   frozenset({"none"}),
    "binary":      frozenset({"none", "custom"}),
}


@dataclass(frozen=True, slots=True)
class FeatureSpec:
    """Immutable declaration of a feature to monitor.

    ``tenant_id`` + ``task_id`` + ``name`` + ``version`` is the natural
    key. A :class:`~efe_drift.catalog.registry.FeatureCatalog` refuses
    to register two entries with the same key.
    """

    tenant_id: str
    task_id: str
    name: str
    kind: FeatureKind
    extractor: str
    binning_strategy: BinningStrategy = "none"
    version: int = 1
    expected_cardinality: int | None = None
    retention_days: int = 90
    notes: str | None = None

    def __post_init__(self) -> None:
        if not self.tenant_id.strip():
            raise ValidationError("tenant_id must be non-empty")
        if not self.task_id.strip():
            raise ValidationError("task_id must be non-empty")
        if not self.name.strip():
            raise ValidationError("feature name must be non-empty")
        if not self.extractor.strip():
            raise ValidationError("extractor must be non-empty")
        if self.kind not in _VALID_KINDS:
            raise ValidationError(
                f"kind {self.kind!r} not in {sorted(_VALID_KINDS)}",
            )
        if self.binning_strategy not in _VALID_BINNINGS:
            raise InvalidBinningStrategy(
                f"binning_strategy {self.binning_strategy!r} not in "
                f"{sorted(_VALID_BINNINGS)}",
            )
        allowed = _ALLOWED_BINNING_BY_KIND[self.kind]
        if self.binning_strategy not in allowed:
            raise InvalidBinningStrategy(
                f"kind={self.kind!r} requires binning_strategy in "
                f"{sorted(allowed)}, got {self.binning_strategy!r}",
            )
        if self.version < 1:
            raise ValidationError(
                f"version must be ≥ 1, got {self.version}",
            )
        if self.retention_days < 1:
            raise ValidationError(
                f"retention_days must be ≥ 1, got {self.retention_days}",
            )
        if (
            self.expected_cardinality is not None
            and self.expected_cardinality < 2
        ):
            raise ValidationError(
                "expected_cardinality, if set, must be ≥ 2",
            )

    @property
    def natural_key(self) -> tuple[str, str, str, int]:
        return (self.tenant_id, self.task_id, self.name, self.version)

    def to_dict(self) -> dict[str, Any]:
        return {
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "name": self.name,
            "kind": self.kind,
            "extractor": self.extractor,
            "binning_strategy": self.binning_strategy,
            "version": self.version,
            "expected_cardinality": self.expected_cardinality,
            "retention_days": self.retention_days,
            "notes": self.notes,
        }


__all__ = ["FeatureSpec"]
