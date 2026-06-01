"""Detector registry — name → :class:`DriftDetector` lookup.

Enables runtime configuration of which detectors are active per
deployment / tenant. The default registry is produced by
:func:`build_default_registry` and covers all six MVP detectors.
"""

from __future__ import annotations

from efe_drift.detection.base import DriftDetector, DriftMethod
from efe_drift.detection.chi_squared import ChiSquaredDetector
from efe_drift.detection.classifier import ClassifierDriftDetector
from efe_drift.detection.ks import KsDetector
from efe_drift.detection.mmd import MmdDetector
from efe_drift.detection.psi import JsDivergenceDetector, PsiDetector
from efe_drift.detection.wasserstein import WassersteinDetector


class DetectorRegistry:
    """Method-name → detector lookup."""

    __slots__ = ("_by_method",)

    def __init__(self) -> None:
        self._by_method: dict[DriftMethod, DriftDetector] = {}

    def register(self, detector: DriftDetector) -> None:
        if detector.method in self._by_method:
            raise ValueError(
                f"detector for method {detector.method!r} already registered",
            )
        self._by_method[detector.method] = detector

    def get(self, method: DriftMethod) -> DriftDetector:
        if method not in self._by_method:
            raise KeyError(f"no detector for method {method!r}")
        return self._by_method[method]

    def has(self, method: DriftMethod) -> bool:
        return method in self._by_method

    def methods(self) -> tuple[DriftMethod, ...]:
        return tuple(sorted(self._by_method))

    def for_kind(self, kind: str) -> tuple[DriftDetector, ...]:
        return tuple(
            d for d in self._by_method.values()
            if kind in d.supported_kinds()
        )


def build_default_registry() -> DetectorRegistry:
    reg = DetectorRegistry()
    reg.register(PsiDetector())
    reg.register(JsDivergenceDetector())
    reg.register(KsDetector())
    reg.register(ChiSquaredDetector())
    reg.register(WassersteinDetector())
    reg.register(MmdDetector())
    reg.register(ClassifierDriftDetector())
    return reg


__all__ = ["DetectorRegistry", "build_default_registry"]
