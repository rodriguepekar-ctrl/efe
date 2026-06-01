"""Public result types returned by the Mode-1 bridge."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Mapping


@dataclass(frozen=True)
class SubScore:
    """Per-class cosine similarity with the projected centroid.

    Useful for:
        * debugging ("why did the swarm pick X over Y?")
        * calibration ("how close was the runner-up?")
        * re-ranking in downstream logic
    """

    label: str
    cosine: float


@dataclass(frozen=True)
class ClassifyResult:
    """Outcome of one swarm classification.

    Attributes:
        label: The chosen class name (from the catalog) or ``"uncertain"``
            when ``reject=True``.
        confidence: Swarm convergence confidence in ``[0, 1]``.
            Distinct from ``top_cosine`` which is the post-projection
            cosine with the chosen anchor.
        alignment_delta: L2 distance between the raw centroid and its
            projection onto the alignment cone. A high value signals
            that the problem was outside the catalog's coverage.
        top_cosine: Cosine similarity between the projected centroid
            and the chosen anchor. ``None`` when ``reject=True`` for
            a non-alignment reason.
        rounds: Number of fusion rounds executed by the swarm.
        reject: ``True`` when the result should be treated as
            ``uncertain`` by the calling LLM (confidence below the
            catalog's reject threshold, or alignment breach).
        swarm_id: Server-side UUID of the swarm that produced this
            verdict.
        request_id: Client-side UUID used for idempotency + audit
            correlation.
        text_hash: SHA-256 hex of the normalised input text. Safe to
            log (no PII leak). Useful for caching and deduplication.
        catalog_version: The catalog version that produced the anchors.
        sub_scores: Optional per-class sub-scores (populated when
            ``classify(..., return_sub_scores=True)``).
        total_ms: Round-trip time as measured on the server side.
    """

    label: str
    confidence: float
    alignment_delta: float
    rounds: int
    reject: bool
    swarm_id: str
    request_id: str
    text_hash: str
    catalog_version: str
    top_cosine: float | None = None
    sub_scores: tuple[SubScore, ...] | None = None
    total_ms: float = 0.0
    extra: Mapping[str, Any] = field(default_factory=dict)

    @property
    def trustworthy(self) -> bool:
        """Convenience: not rejected AND alignment within bounds."""
        return not self.reject and self.alignment_delta <= 0.5

    def as_tool_result(self) -> dict[str, Any]:
        """JSON-serialisable dict for returning to an LLM tool call.

        Minimal surface — hides internal IDs and sub-scores by default.
        """
        return {
            "label": self.label,
            "confidence": round(self.confidence, 4),
            "rounds": self.rounds,
            "alignment_delta": round(self.alignment_delta, 4),
            "reject": self.reject,
        }

    def as_debug_dict(self) -> dict[str, Any]:
        """Verbose dict for logging / debugging — includes all fields."""
        return {
            "label": self.label,
            "confidence": round(self.confidence, 6),
            "alignment_delta": round(self.alignment_delta, 6),
            "rounds": self.rounds,
            "reject": self.reject,
            "top_cosine": round(self.top_cosine, 6) if self.top_cosine is not None else None,
            "swarm_id": self.swarm_id,
            "request_id": self.request_id,
            "text_hash": self.text_hash,
            "catalog_version": self.catalog_version,
            "total_ms": round(self.total_ms, 3),
            "sub_scores": [
                {"label": s.label, "cosine": round(s.cosine, 6)} for s in (self.sub_scores or ())
            ],
        }
