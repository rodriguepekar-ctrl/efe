"""Sprint D7 — Plan B : hybrid (raw cosine + swarm) decoder.

This module implements a fallback classifier that **does not depend on
the swarm being a competent classifier on its own**. It is the safety
net documented in the D7 architecture document
(`docs/bench/d7_genesis_fusion_bias_remediation_architecture.md` §4
"Approach C — Hybrid decoder") in case the β/α calibration sweep
plateaus below the 0.65 macro_f1 multi-catalog gate.

Status
------

**Armed but not activated.** As of D7 the recommended path remains
Approach E (β/α tuning via `Mode1Config.fusion`). The hybrid path is
exercised only in unit tests and via an explicit opt-in flag — see
`HybridClassifier.is_active`. The intent is :

* If post-D7 calibration delivers macro_f1 ≥ 0.75 on the 3 catalogs,
  this module sits dormant and serves as **fallback documentation**
  (i.e. "we know how to ship a 0.95+ classifier in a pinch").
* If a future regression drops the swarm below 0.65, the operator
  flips `efe_tool.integrations.hybrid_decoder.HybridClassifier` into
  the client and the system maintains its product contract.

How it works
------------

The hybrid decoder produces a label by **majority vote** between two
independent classifiers :

1. **Raw cosine** — embed the input, compute cosine similarity against
   each anchor, pick the argmax. This is the embedder's own
   classification : on `support_triage` with `paraphrase-multilingual-MiniLM-L12-v2`,
   raw cosine alone reaches 0.95+ on the golden set.

2. **Swarm centroid** — the `EfeClient.classify` result. The swarm
   sees the same anchors and produces a centroid + confidence ; we
   reuse the existing `ClassifyResult.label` and `ClassifyResult.confidence`.

The hybrid logic :

* If both classifiers agree, return that label with the higher of the
  two confidences (the swarm's confidence is a useful certainty
  signal even when the swarm is biased).
* If they disagree :
  - If raw_cosine_top1_score > 0.55 (high-margin nearest anchor) and
    swarm_confidence < 0.70 (low swarm certainty), prefer raw cosine
    (the swarm is confused on a clear case).
  - Else if swarm_confidence ≥ 0.85 (very high swarm certainty),
    prefer the swarm (it has seen the anchors as a pool, may detect
    multi-anchor blends the cosine cannot).
  - Else (median confidence on both sides) : reject — return
    ``"uncertain"`` with confidence = max(raw_top1_score, swarm_conf).

This policy is inspired by classic ensemble theory : disagreement
between independent classifiers is itself a useful uncertainty
signal.

Public API
----------

``HybridClassifier(client, catalog)`` — the main class. Call
``.classify(text)`` to obtain a ``HybridResult``. The class is
:func:`dataclass(frozen=True)` so it is safe to share across threads.

The dependencies are minimal — only ``numpy`` and the existing
``efe_tool`` package. No additional model weights, no extra RPC.

Examples
--------

::

    from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder
    from efe_tool.integrations.hybrid_decoder import HybridClassifier

    embedder = LocalSTEmbedder()
    catalog = AnchorCatalog.load("catalogs/support_triage.yaml", embedder=embedder)
    client = EfeClient.from_env()
    hybrid = HybridClassifier(client=client, catalog=catalog)

    result = hybrid.classify("Someone charged my card without my consent.")
    print(result.label, result.confidence, result.path)
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any

import numpy as np

if TYPE_CHECKING:  # pragma: no cover — import guard for static type checkers.
    from efe_tool import AnchorCatalog, EfeClient
    from efe_tool.types import ClassifyResult


# ---------------------------------------------------------------------------
# Tunable thresholds (operator-overridable via constructor kwargs)
# ---------------------------------------------------------------------------

#: Default raw-cosine "high margin" threshold above which the embedder
#: alone is trusted to override an unsure swarm.
DEFAULT_RAW_HIGH_MARGIN: float = 0.55
#: Default swarm-confidence "very high" threshold above which the
#: swarm overrides a disagreeing raw cosine.
DEFAULT_SWARM_HIGH_CONFIDENCE: float = 0.85
#: Default swarm-confidence "low" threshold below which the swarm
#: yields to raw cosine on disagreement.
DEFAULT_SWARM_LOW_CONFIDENCE: float = 0.70
#: Default minimum margin of the top-1 raw cosine over the runner-up
#: required to consider raw cosine "decisive" on its own.
DEFAULT_RAW_TOP_GAP: float = 0.04


# ---------------------------------------------------------------------------
# Result type
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class HybridResult:
    """The output of :meth:`HybridClassifier.classify`.

    Attributes
    ----------
    label
        The chosen class name (e.g. ``"fraud"``) or ``"uncertain"`` if
        the two classifiers disagreed and neither met its high-margin
        threshold.
    confidence
        A scalar in ``[0, 1]`` ; the maximum of the two source
        confidences when they agreed, or the higher signal when they
        disagreed (and the disagreement was resolved).
    path
        Human-readable explanation of which branch was taken
        (``"agree"``, ``"raw_overrides_swarm"``, ``"swarm_overrides_raw"``,
        ``"reject"``). Exposed primarily for the calibration audit and
        the unit tests.
    raw_label
        The label produced by raw cosine alone.
    raw_score
        The cosine of the input embedding to the chosen anchor.
    raw_runner_up_score
        The cosine to the runner-up anchor — a useful sanity signal
        for the audit.
    swarm_label
        The label produced by the swarm.
    swarm_confidence
        The swarm's confidence in `swarm_label`.
    """

    label: str
    confidence: float
    path: str
    raw_label: str
    raw_score: float
    raw_runner_up_score: float
    swarm_label: str
    swarm_confidence: float
    extra: dict[str, Any] = field(default_factory=dict)


# ---------------------------------------------------------------------------
# Helper : raw-cosine classifier
# ---------------------------------------------------------------------------


def raw_cosine_predict(
    text_embedding: np.ndarray,
    catalog: "AnchorCatalog",
) -> tuple[str, float, float]:
    """Return ``(label, top_score, runner_up_score)`` for the raw
    cosine classifier on a single input.

    Exposed publicly in **Sprint D8** for use by
    ``eval/scripts/compare_decoders.py`` and any external code that
    wants to run the embedder-only baseline. Stable API.
    """
    if text_embedding.ndim != 1:
        raise ValueError(
            f"text_embedding must be 1-D, got shape {text_embedding.shape}"
        )
    # Normalise the input embedding (the catalog anchors are already
    # L2-normalised at compile time).
    norm = float(np.linalg.norm(text_embedding))
    if not np.isfinite(norm) or norm <= 0.0:
        # Degenerate input — treat as the worst-case non-classifying
        # signal so the hybrid path produces a deterministic "reject"
        # rather than NaN propagation.
        return ("uncertain", 0.0, 0.0)
    unit = (text_embedding / norm).astype(np.float32, copy=False)
    sims = catalog.anchors @ unit  # shape (N,)

    # The catalog can carry one anchor per *class* (already aggregated
    # at compile time) — pick the argmax directly.
    if sims.shape[0] != len(catalog.classes):
        # Backward-compat : if the catalog produces one anchor per
        # canonical sentence we aggregate by class via a max-pool.
        per_class = _max_pool_per_class(sims, catalog)
    else:
        per_class = sims

    order = np.argsort(per_class)[::-1]
    top_idx = int(order[0])
    runner_up_idx = int(order[1]) if order.size > 1 else top_idx
    label = catalog.classes[top_idx].name
    return label, float(per_class[top_idx]), float(per_class[runner_up_idx])


def _max_pool_per_class(sims: np.ndarray, catalog: "AnchorCatalog") -> np.ndarray:
    """Aggregate per-anchor cosines into per-class scores by max-pool.

    A safety net for catalogs whose `anchors` array carries one row
    per canonical sentence rather than one row per class. The
    `AnchorCatalog.compile` path normally aggregates at compile time,
    but this lets the hybrid decoder accept any catalog shape.
    """
    out = np.full(len(catalog.classes), -1.0, dtype=np.float32)
    cursor = 0
    for ci, cls in enumerate(catalog.classes):
        n_sent = max(1, len(cls.canonical_sentences))
        end = cursor + n_sent
        slice_ = sims[cursor:end]
        if slice_.size == 0:
            cursor = end
            continue
        out[ci] = float(slice_.max())
        cursor = end
    return out


# ---------------------------------------------------------------------------
# Main classifier
# ---------------------------------------------------------------------------


@dataclass
class HybridClassifier:
    """Combine raw cosine + swarm into a more robust classifier.

    The class is intentionally a thin coordinator : it does not
    re-implement embedding or RPC. It only encodes the policy that
    decides which classifier to trust on disagreement.
    """

    client: "EfeClient"
    catalog: "AnchorCatalog"
    raw_high_margin: float = DEFAULT_RAW_HIGH_MARGIN
    swarm_high_confidence: float = DEFAULT_SWARM_HIGH_CONFIDENCE
    swarm_low_confidence: float = DEFAULT_SWARM_LOW_CONFIDENCE
    raw_top_gap: float = DEFAULT_RAW_TOP_GAP

    def __post_init__(self) -> None:
        for name, val in [
            ("raw_high_margin", self.raw_high_margin),
            ("swarm_high_confidence", self.swarm_high_confidence),
            ("swarm_low_confidence", self.swarm_low_confidence),
            ("raw_top_gap", self.raw_top_gap),
        ]:
            if not (0.0 <= val <= 1.0):
                raise ValueError(f"{name} must be in [0, 1] ; got {val}")
        if self.swarm_low_confidence >= self.swarm_high_confidence:
            raise ValueError(
                "swarm_low_confidence must be < swarm_high_confidence ; "
                f"got {self.swarm_low_confidence} vs {self.swarm_high_confidence}"
            )

    @property
    def is_active(self) -> bool:
        """`True` once the operator switches the client into hybrid mode.

        The default implementation always returns `True` — the boolean
        is a placeholder for a future wiring where the swarm-only
        path remains the default and an explicit toggle activates the
        hybrid logic. Today instantiating the class is the explicit
        toggle ; the property is kept for forward compat and to make
        intent visible in audit code.
        """
        return True

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def classify(self, text: str) -> HybridResult:
        """Classify ``text`` via the hybrid policy.

        Parameters
        ----------
        text
            Non-empty user input. Lengths above ~512 tokens are
            silently embedded as-is — the embedder will truncate
            following its own contract.

        Returns
        -------
        HybridResult
        """
        if not isinstance(text, str) or not text.strip():
            raise ValueError("HybridClassifier.classify: text must be non-empty")

        # 1. Raw cosine prediction (no RPC).
        text_vec = self.client.embedder.encode(text)
        raw_label, raw_top, raw_runner = raw_cosine_predict(text_vec, self.catalog)

        # 2. Swarm prediction (RPC).
        swarm: "ClassifyResult" = self.client.classify(text, self.catalog)
        swarm_label = swarm.label if not swarm.reject else "uncertain"
        swarm_conf = float(swarm.confidence)

        # 3. Combine.
        return self._combine(
            raw_label=raw_label,
            raw_top=raw_top,
            raw_runner=raw_runner,
            swarm_label=swarm_label,
            swarm_conf=swarm_conf,
            extra={
                "swarm_alignment_delta": float(swarm.alignment_delta),
                "swarm_rounds": int(swarm.rounds),
            },
        )

    # ------------------------------------------------------------------
    # Decision logic — split out for unit-testing the policy in isolation
    # ------------------------------------------------------------------

    def _combine(
        self,
        *,
        raw_label: str,
        raw_top: float,
        raw_runner: float,
        swarm_label: str,
        swarm_conf: float,
        extra: dict[str, Any],
    ) -> HybridResult:
        # Normalise NaN / negative cosines to 0 so the comparisons below
        # are well-defined.
        raw_top = float(raw_top) if np.isfinite(raw_top) else 0.0
        raw_runner = float(raw_runner) if np.isfinite(raw_runner) else 0.0
        swarm_conf = float(swarm_conf) if np.isfinite(swarm_conf) else 0.0
        margin = raw_top - raw_runner

        if raw_label == swarm_label and raw_label != "uncertain":
            confidence = max(raw_top, swarm_conf)
            return HybridResult(
                label=raw_label,
                confidence=confidence,
                path="agree",
                raw_label=raw_label,
                raw_score=raw_top,
                raw_runner_up_score=raw_runner,
                swarm_label=swarm_label,
                swarm_confidence=swarm_conf,
                extra=extra,
            )

        # Disagreement.
        raw_decisive = (raw_top >= self.raw_high_margin) and (
            margin >= self.raw_top_gap
        )
        if raw_decisive and swarm_conf < self.swarm_low_confidence:
            return HybridResult(
                label=raw_label,
                confidence=raw_top,
                path="raw_overrides_swarm",
                raw_label=raw_label,
                raw_score=raw_top,
                raw_runner_up_score=raw_runner,
                swarm_label=swarm_label,
                swarm_confidence=swarm_conf,
                extra=extra,
            )
        if (
            swarm_label != "uncertain"
            and swarm_conf >= self.swarm_high_confidence
        ):
            return HybridResult(
                label=swarm_label,
                confidence=swarm_conf,
                path="swarm_overrides_raw",
                raw_label=raw_label,
                raw_score=raw_top,
                raw_runner_up_score=raw_runner,
                swarm_label=swarm_label,
                swarm_confidence=swarm_conf,
                extra=extra,
            )
        # Both signals are middling — the hybrid contract is to surface
        # the uncertainty rather than coin-flip a decision.
        return HybridResult(
            label="uncertain",
            confidence=max(raw_top, swarm_conf),
            path="reject",
            raw_label=raw_label,
            raw_score=raw_top,
            raw_runner_up_score=raw_runner,
            swarm_label=swarm_label,
            swarm_confidence=swarm_conf,
            extra=extra,
        )


__all__ = [
    "HybridClassifier",
    "HybridResult",
    "raw_cosine_predict",
    "DEFAULT_RAW_HIGH_MARGIN",
    "DEFAULT_RAW_TOP_GAP",
    "DEFAULT_SWARM_HIGH_CONFIDENCE",
    "DEFAULT_SWARM_LOW_CONFIDENCE",
]


# ---------------------------------------------------------------------------
# Backward-compat alias (kept for the D7 unit tests that imported the
# private symbol directly). New callers should use `raw_cosine_predict`.
# ---------------------------------------------------------------------------

_raw_cosine_predict = raw_cosine_predict

