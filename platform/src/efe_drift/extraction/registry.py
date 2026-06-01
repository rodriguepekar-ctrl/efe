"""Extractor registry — central lookup for named extractors.

The registry is a thin wrapper over a dict :

* Callers register concrete extractors under string names at process
  boot (``token_length``, ``lang_detect``, ``sbert_minilm``, etc.).
* Callers retrieve them by name via :meth:`get`.
* :meth:`has` is the hook used by
  :class:`~efe_drift.catalog.FeatureCatalog` to verify at register
  time that the declared extractor exists.

Thread safety is not needed — registration happens during process
boot before the asyncio loop is spinning.
"""

from __future__ import annotations

from collections.abc import Iterator

from efe_drift.errors import ExtractorMissing
from efe_drift.extraction.base import FeatureExtractor


class ExtractorRegistry:
    """Name → :class:`FeatureExtractor` mapping."""

    __slots__ = ("_by_name",)

    def __init__(self) -> None:
        self._by_name: dict[str, FeatureExtractor] = {}

    # ---- mutation ------------------------------------------------------

    def register(self, extractor: FeatureExtractor) -> None:
        if not extractor.name or not extractor.name.strip():
            raise ValueError("extractor name must be non-empty")
        if extractor.name in self._by_name:
            raise ValueError(
                f"extractor {extractor.name!r} already registered",
            )
        self._by_name[extractor.name] = extractor

    def unregister(self, name: str) -> None:
        self._by_name.pop(name, None)

    def clear(self) -> None:
        self._by_name.clear()

    # ---- query ---------------------------------------------------------

    def has(self, name: str) -> bool:
        return name in self._by_name

    def get(self, name: str) -> FeatureExtractor:
        if name not in self._by_name:
            raise ExtractorMissing(
                f"extractor {name!r} not registered — known extractors: "
                f"{sorted(self._by_name)}",
            )
        return self._by_name[name]

    def __iter__(self) -> Iterator[FeatureExtractor]:
        return iter(self._by_name.values())

    def __len__(self) -> int:
        return len(self._by_name)

    def names(self) -> tuple[str, ...]:
        return tuple(sorted(self._by_name))


__all__ = ["ExtractorRegistry"]
