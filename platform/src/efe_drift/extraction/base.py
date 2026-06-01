"""Feature extractor base types.

A :class:`FeatureExtractor` is a callable-like object that turns a
:class:`~efe_drift.records.InferenceEvent` into a zero-or-one
:class:`~efe_drift.records.FeatureValue`.

Design notes :

* Extractors are **deterministic** given an event — no wall-clock, no
  RNG. Tests rely on this.
* Extractors are **side-effect-free** from the caller's perspective :
  they must not touch the network, the DB, or the disk (beyond the
  one-time model loading done in their ``__init__``).
* Extractors MAY lazy-load heavy artefacts (sentence-transformers
  weights, fasttext language models) the first time they see an
  event. Subsequent calls reuse the cached artefact.
* When the event lacks the signal the extractor needs, the extractor
  returns ``None`` — this is not an error, just "nothing to sample
  for this feature on this event".

The typed factory helpers at the bottom of the file (:func:`numeric`,
:func:`categorical`, :func:`binary`, :func:`embedding`) construct
:class:`FeatureValue` instances with the correct shape for the chosen
``kind``. Callers that build :class:`FeatureValue` by hand are welcome
too, but the factories catch the most common mistake (passing a
string to ``numeric_value``).
"""

from __future__ import annotations

from typing import Protocol, runtime_checkable

from efe_drift.errors import ExtractorTypeMismatch
from efe_drift.records import FeatureKind, FeatureValue, InferenceEvent


@runtime_checkable
class FeatureExtractor(Protocol):
    """Pure function ``InferenceEvent → FeatureValue | None``.

    Implementations declare :attr:`name` (the catalog key) and
    :attr:`kind` (the feature kind they produce). The properties are
    read-only — this lets concrete classes expose them as dataclass
    fields, :func:`property` getters, or class attributes without
    tripping mypy's Protocol-variance rules.
    """

    @property
    def name(self) -> str:  # pragma: no cover
        ...

    @property
    def kind(self) -> FeatureKind:  # pragma: no cover
        ...

    def extract(
        self, event: InferenceEvent,
    ) -> FeatureValue | None:  # pragma: no cover
        ...


# ---------------------------------------------------------------------------
# Typed factories — always the right shape for the given kind
# ---------------------------------------------------------------------------


def numeric(feature_name: str, value: float | int | None) -> FeatureValue | None:
    if value is None:
        return None
    return FeatureValue(
        kind="numeric",
        feature_name=feature_name,
        numeric_value=float(value),
    )


def binary(feature_name: str, value: bool | None) -> FeatureValue | None:
    if value is None:
        return None
    return FeatureValue(
        kind="binary",
        feature_name=feature_name,
        numeric_value=1.0 if value else 0.0,
    )


def categorical(
    feature_name: str, value: str | None,
) -> FeatureValue | None:
    if value is None or not value.strip():
        return None
    return FeatureValue(
        kind="categorical",
        feature_name=feature_name,
        categorical_value=value.strip(),
    )


def embedding(
    feature_name: str, vec: tuple[float, ...] | list[float] | None,
) -> FeatureValue | None:
    if vec is None:
        return None
    frozen = tuple(float(x) for x in vec)
    if not frozen:
        return None
    return FeatureValue(
        kind="embedding",
        feature_name=feature_name,
        embedding=frozen,
    )


def assert_extractor_produces(
    fv: FeatureValue, expected_kind: FeatureKind,
) -> None:
    """Guardrail — typically called by the ingestion pipeline.

    If an extractor claims ``kind = "numeric"`` but returns a
    categorical :class:`FeatureValue` we raise so the mismatch is
    caught *before* persisting a corrupt row.
    """
    if fv.kind != expected_kind:
        raise ExtractorTypeMismatch(
            f"extractor produced kind={fv.kind!r} but catalog "
            f"declared {expected_kind!r} for feature "
            f"{fv.feature_name!r}",
        )


__all__ = [
    "FeatureExtractor",
    "assert_extractor_produces",
    "binary",
    "categorical",
    "embedding",
    "numeric",
]
