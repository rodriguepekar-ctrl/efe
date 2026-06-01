"""Decode the raw :class:`ExecuteResponse` into a business result."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Sequence

from efe_swarm_integration.errors import (
    AlignmentBreach,
    SwarmDivergence,
    SwarmModeCollapse,
    UncertainAlignment,
)
from efe_swarm_integration.reconciler.confidence import compute_confidence
from efe_swarm_integration.reconciler.nearest_anchor import nearest_anchor_label
from efe_swarm_integration.types import (
    ExecuteResponse,
    VerdictBundle,
    VerdictEnum,
)

__all__ = ["ClassifyResponse", "VerdictReconciler", "reconcile"]


@dataclass(frozen=True, slots=True)
class ClassifyResponse:
    """Business-facing result for a Mode 1 classification."""

    label: str | None
    label_idx: int | None
    confidence: float
    alignment_delta: float
    swarm_id: str
    request_id: str
    verdict: VerdictEnum
    reasons: tuple[str, ...] = field(default_factory=tuple)

    @property
    def is_uncertain(self) -> bool:
        return self.label is None


def reconcile(
    response: ExecuteResponse,
    *,
    cone_radius: float,
    uncertain_threshold_ratio: float = 0.8,
    raise_on_terminal: bool = False,
) -> VerdictBundle:
    """Package an :class:`ExecuteResponse` into a :class:`VerdictBundle`.

    Raises :class:`SwarmDivergence` / :class:`SwarmModeCollapse` only if
    ``raise_on_terminal`` is True. Otherwise the caller inspects the
    ``verdict`` field of the returned bundle.
    """
    if raise_on_terminal and response.verdict == VerdictEnum.DIVERGENCE:
        raise SwarmDivergence(
            f"swarm diverged for request {response.request_id}",
            detail=response.__dict__,
        )
    if raise_on_terminal and response.verdict == VerdictEnum.MODE_COLLAPSE:
        raise SwarmModeCollapse(
            f"swarm mode collapse for request {response.request_id}",
            detail=response.__dict__,
        )

    # Optional breach check.
    breach = None
    if cone_radius > 0.0 and response.alignment_delta > cone_radius:
        if raise_on_terminal:
            raise AlignmentBreach(
                f"alignment delta {response.alignment_delta:.4f} > cone radius {cone_radius:.4f}"
            )

    confidence = compute_confidence(
        alignment_delta=response.alignment_delta,
        cone_radius=cone_radius,
        server_reported_confidence=response.confidence,
    )

    return VerdictBundle(
        swarm_id=response.swarm_id,
        request_id=response.request_id,
        verdict=response.verdict,
        decoded=response.output,
        confidence=confidence,
        alignment_delta=response.alignment_delta,
        rounds_executed=response.rounds_executed,
        total_ms=response.total_ms,
        breach=breach,
    )


class VerdictReconciler:
    """Stateful reconciler that can also produce :class:`ClassifyResponse`
    when a label vocabulary is known."""

    __slots__ = ("_uncertain_ratio",)

    def __init__(self, uncertain_threshold_ratio: float = 0.8) -> None:
        if not 0.0 <= uncertain_threshold_ratio <= 1.0:
            raise ValueError("uncertain_threshold_ratio must be in [0, 1]")
        self._uncertain_ratio = uncertain_threshold_ratio

    def to_classify(
        self,
        bundle: VerdictBundle,
        *,
        labels: Sequence[str],
        anchors: Sequence[Sequence[float]] | None = None,
        cone_radius: float,
    ) -> ClassifyResponse:
        reasons: list[str] = []
        if bundle.verdict in (VerdictEnum.DIVERGENCE, VerdictEnum.MODE_COLLAPSE):
            reasons.append(f"verdict:{bundle.verdict.value}")
            return ClassifyResponse(
                label=None,
                label_idx=None,
                confidence=0.0,
                alignment_delta=bundle.alignment_delta,
                swarm_id=bundle.swarm_id,
                request_id=bundle.request_id,
                verdict=bundle.verdict,
                reasons=tuple(reasons),
            )

        # If output is an explicit text the server has already decoded
        # into the label domain.
        if bundle.decoded.kind == "text" and bundle.decoded.text:
            label = bundle.decoded.text
            try:
                idx = labels.index(label)
            except ValueError:
                idx = -1
            return ClassifyResponse(
                label=label,
                label_idx=idx if idx >= 0 else None,
                confidence=bundle.confidence,
                alignment_delta=bundle.alignment_delta,
                swarm_id=bundle.swarm_id,
                request_id=bundle.request_id,
                verdict=bundle.verdict,
                reasons=("text_decode",),
            )

        # Raw centroid : do nearest-anchor locally if anchors supplied.
        if bundle.decoded.kind == "raw_centroid":
            centroid = bundle.decoded.raw_centroid or ()
            if anchors is not None:
                try:
                    label, idx, _ = nearest_anchor_label(
                        centroid, anchors=anchors, labels=list(labels)
                    )
                except ValueError as exc:
                    reasons.append(f"nearest_anchor_failed:{exc}")
                    label, idx = None, None
            else:
                label, idx = None, None
                reasons.append("no_anchors_provided")

            # Uncertainty gate.
            if cone_radius > 0.0:
                ratio = bundle.alignment_delta / cone_radius
                if ratio > self._uncertain_ratio:
                    label = None
                    idx = None
                    reasons.append(f"uncertain:ratio={ratio:.3f}")

            return ClassifyResponse(
                label=label,
                label_idx=idx,
                confidence=bundle.confidence,
                alignment_delta=bundle.alignment_delta,
                swarm_id=bundle.swarm_id,
                request_id=bundle.request_id,
                verdict=bundle.verdict,
                reasons=tuple(reasons) if reasons else ("centroid_decoded",),
            )

        # Fallback
        reasons.append("unknown_output_kind")
        return ClassifyResponse(
            label=None,
            label_idx=None,
            confidence=bundle.confidence,
            alignment_delta=bundle.alignment_delta,
            swarm_id=bundle.swarm_id,
            request_id=bundle.request_id,
            verdict=bundle.verdict,
            reasons=tuple(reasons),
        )
