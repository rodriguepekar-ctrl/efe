"""Value objects mirroring the ``efe.v1`` protobuf surface.

All types are immutable (``frozen=True, slots=True``). They have a
1-to-1 mapping with the protobuf messages in ``protos/efe/v1/*.proto``
but live in pure-Python so that :

* the module is importable without ``grpcio`` / ``protobuf`` deps,
* the rest of the platform never manipulates wire-level objects,
* unit tests run without generated ``*_pb2.py`` files.

The :mod:`efe_swarm_integration.contract` adapter is the only place
allowed to round-trip these types to/from their protobuf counterparts.
"""

from __future__ import annotations

import enum
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Literal

__all__ = [
    "AlignmentCone",
    "Anchor",
    "BreachReport",
    "CanonicalCone",
    "ConeEvolutionOp",
    "ConeHandle",
    "ConeMetadata",
    "ConeRef",
    "ConeSpec",
    "ConvergenceThresholds",
    "DecodeMode",
    "DecodedOutput",
    "DescribeSpecReply",
    "ExecuteRequest",
    "ExecuteResponse",
    "PopulationEntry",
    "PopulationMix",
    "ResourceBudget",
    "ServerEvent",
    "StreamChunk",
    "SwarmSpec",
    "TemplateCode",
    "TextDelta",
    "VerdictBundle",
    "VerdictEnum",
    "build_swarm_id",
]


# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------


class TemplateCode(str, enum.Enum):
    """MVP templates from ``efe_core::types::TemplateCode``."""

    NaSem = "na-sem"
    NaLog = "na-log"
    NaCrit = "na-crit"

    def as_u8(self) -> int:
        """Stable integer representation matching the Rust enum."""
        return {TemplateCode.NaSem: 0, TemplateCode.NaLog: 1, TemplateCode.NaCrit: 2}[self]


class VerdictEnum(str, enum.Enum):
    UNSPECIFIED = "unspecified"
    CONTINUE = "continue"
    CONVERGED = "converged"
    DIVERGENCE = "divergence"
    MODE_COLLAPSE = "mode_collapse"


# ---------------------------------------------------------------------------
# Population mix
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class PopulationEntry:
    template: TemplateCode
    count: int

    def __post_init__(self) -> None:
        if self.count <= 0:
            raise ValueError("PopulationEntry.count must be > 0")


@dataclass(frozen=True, slots=True)
class PopulationMix:
    entries: tuple[PopulationEntry, ...]

    def __post_init__(self) -> None:
        if not self.entries:
            raise ValueError("PopulationMix needs at least one entry")
        # deduplicate by template — policy is to fail on duplicates
        seen: set[TemplateCode] = set()
        for e in self.entries:
            if e.template in seen:
                raise ValueError(f"duplicate template {e.template}")
            seen.add(e.template)

    @property
    def total(self) -> int:
        return sum(e.count for e in self.entries)

    @property
    def template_count(self) -> int:
        return len(self.entries)

    @staticmethod
    def default_classifier() -> "PopulationMix":
        """Balanced NaSem/NaLog/NaCrit mix for Mode 1 classification."""
        return PopulationMix(
            entries=(
                PopulationEntry(TemplateCode.NaSem, 64),
                PopulationEntry(TemplateCode.NaLog, 32),
                PopulationEntry(TemplateCode.NaCrit, 16),
            )
        )


# ---------------------------------------------------------------------------
# Anchors / Alignment cone
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class Anchor:
    values: tuple[float, ...]

    def __post_init__(self) -> None:
        if not self.values:
            raise ValueError("Anchor must have at least one dimension")
        if any(not _is_finite(v) for v in self.values):
            raise ValueError("Anchor values must be finite floats")

    def dim(self) -> int:
        return len(self.values)


@dataclass(frozen=True, slots=True)
class AlignmentCone:
    """Inline cone — used when the consumer doesn't have a registered
    ``ConeRef`` yet (first boot / tests)."""

    anchors: tuple[Anchor, ...]
    max_projection_distance: float

    def __post_init__(self) -> None:
        if not self.anchors:
            raise ValueError("AlignmentCone needs at least one anchor")
        if not (self.max_projection_distance > 0.0 and _is_finite(self.max_projection_distance)):
            raise ValueError("max_projection_distance must be positive and finite")
        dim = self.anchors[0].dim()
        for a in self.anchors[1:]:
            if a.dim() != dim:
                raise ValueError("all anchors must share the same dimension")

    def dim(self) -> int:
        return self.anchors[0].dim()


@dataclass(frozen=True, slots=True)
class ConeRef:
    """Reference to a cone stored in the M9 alignment registry."""

    tenant: str
    task: str
    version: int

    def __post_init__(self) -> None:
        if not self.tenant or not self.task:
            raise ValueError("ConeRef requires tenant and task")
        if self.version < 1:
            raise ValueError("ConeRef.version must be >= 1")

    @property
    def key(self) -> str:
        return f"{self.tenant}/{self.task}@v{self.version}"


@dataclass(frozen=True, slots=True)
class ConeMetadata:
    created_at: datetime
    created_by: str
    description: str = ""
    parent_key: str | None = None
    evolution_op: str | None = None
    labels: tuple[tuple[str, str], ...] = ()

    def __post_init__(self) -> None:
        if self.created_at.tzinfo is None:
            raise ValueError("ConeMetadata.created_at must be tz-aware")


@dataclass(frozen=True, slots=True)
class CanonicalCone:
    """Server-canonical form of a cone ; returned by
    :class:`admin.AdminService.GetCone`."""

    cone: AlignmentCone
    metadata: ConeMetadata


@dataclass(frozen=True, slots=True)
class ConeHandle:
    """Identifier + canonical form of a registered cone."""

    key: str  # "tenant/task@vN"
    canonical: CanonicalCone


@dataclass(frozen=True, slots=True)
class ConeEvolutionOp:
    """Sum type mirroring ``efe.v1.admin.ConeEvolutionOp``."""

    kind: Literal[
        "append_anchor",
        "remove_anchor_idx",
        "replace_anchor",
        "update_radius",
        "scale_anchors",
        "union_with_key",
    ]
    append_anchor: Anchor | None = None
    remove_anchor_idx: int | None = None
    replace_anchor: tuple[int, Anchor] | None = None
    update_radius: float | None = None
    scale_anchors: float | None = None
    union_with_key: str | None = None

    def __post_init__(self) -> None:
        # Exactly one variant payload must be set.
        set_fields = [
            self.append_anchor is not None,
            self.remove_anchor_idx is not None,
            self.replace_anchor is not None,
            self.update_radius is not None,
            self.scale_anchors is not None,
            self.union_with_key is not None,
        ]
        if sum(set_fields) != 1:
            raise ValueError("ConeEvolutionOp requires exactly one variant payload")


# ---------------------------------------------------------------------------
# Convergence thresholds
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ConvergenceThresholds:
    epsilon_variance: float = 1e-3
    epsilon_gradient: float = 1e-4
    epsilon_stability: float = 1e-3
    stability_window: int = 3

    def __post_init__(self) -> None:
        if self.epsilon_variance <= 0.0:
            raise ValueError("epsilon_variance must be > 0")
        if self.epsilon_gradient <= 0.0:
            raise ValueError("epsilon_gradient must be > 0")
        if self.epsilon_stability <= 0.0:
            raise ValueError("epsilon_stability must be > 0")
        if self.stability_window < 1:
            raise ValueError("stability_window must be >= 1")


# ---------------------------------------------------------------------------
# Resource budget
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ResourceBudget:
    max_wall_clock_ms: int = 500
    max_energy_wh: float = 5.0
    max_hbm_bytes: int = 6 * 1024**3
    max_recursion_depth: int = 0

    def __post_init__(self) -> None:
        if self.max_wall_clock_ms <= 0:
            raise ValueError("max_wall_clock_ms must be > 0")
        if self.max_energy_wh <= 0.0:
            raise ValueError("max_energy_wh must be > 0")
        if self.max_hbm_bytes <= 0:
            raise ValueError("max_hbm_bytes must be > 0")
        if self.max_recursion_depth < 0:
            raise ValueError("max_recursion_depth must be >= 0")


# ---------------------------------------------------------------------------
# Decode mode
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class DecodeMode:
    kind: Literal["text", "pass_through"] = "pass_through"
    max_tokens: int | None = None

    def __post_init__(self) -> None:
        if self.kind == "text" and (self.max_tokens is None or self.max_tokens <= 0):
            raise ValueError("DecodeMode.text requires a positive max_tokens")


# ---------------------------------------------------------------------------
# Decoded output
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class DecodedOutput:
    kind: Literal["text", "raw_centroid"]
    text: str | None = None
    raw_centroid: tuple[float, ...] | None = None

    def __post_init__(self) -> None:
        if self.kind == "text" and self.text is None:
            raise ValueError("DecodedOutput.text requires text payload")
        if self.kind == "raw_centroid" and self.raw_centroid is None:
            raise ValueError("DecodedOutput.raw_centroid requires values")


# ---------------------------------------------------------------------------
# SwarmSpec (the wire-level spec)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SwarmSpec:
    """Python mirror of ``efe.v1.SwarmSpec``."""

    dim_latent: int
    problem_embedding: tuple[float, ...]
    population: PopulationMix
    alignment_cone: AlignmentCone | None
    alignment_cone_ref: ConeRef | None
    max_rounds: int = 8
    convergence: ConvergenceThresholds = field(default_factory=ConvergenceThresholds)
    budget: ResourceBudget = field(default_factory=ResourceBudget)
    swarm_id: str = ""  # empty → server generates

    def __post_init__(self) -> None:
        if self.dim_latent < 1:
            raise ValueError("dim_latent must be >= 1")
        if len(self.problem_embedding) != self.dim_latent:
            raise ValueError(
                f"problem_embedding length {len(self.problem_embedding)} "
                f"does not match dim_latent {self.dim_latent}"
            )
        if any(not _is_finite(x) for x in self.problem_embedding):
            raise ValueError("problem_embedding contains NaN or Inf")
        if self.max_rounds < 1:
            raise ValueError("max_rounds must be >= 1")
        if (self.alignment_cone is None) == (self.alignment_cone_ref is None):
            raise ValueError(
                "SwarmSpec requires exactly one of alignment_cone or alignment_cone_ref"
            )


@dataclass(frozen=True, slots=True)
class ExecuteRequest:
    """Python mirror of ``efe.v1.ExecuteRequest``."""

    spec: SwarmSpec
    decode_mode: DecodeMode = field(default_factory=DecodeMode)
    request_id: str = ""  # empty → server fills
    raw_prompt: str | None = None
    deadline: datetime | None = None

    def __post_init__(self) -> None:
        if self.deadline is not None and self.deadline.tzinfo is None:
            raise ValueError("ExecuteRequest.deadline must be tz-aware")


# ---------------------------------------------------------------------------
# ExecuteResponse / VerdictBundle
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class BreachReport:
    cone_key: str
    swarm_id: str
    observed_delta: float
    cone_radius: float
    timestamp: datetime
    context: str = ""

    def __post_init__(self) -> None:
        if self.timestamp.tzinfo is None:
            raise ValueError("BreachReport.timestamp must be tz-aware")


@dataclass(frozen=True, slots=True)
class ExecuteResponse:
    """Raw server response (wire-level)."""

    swarm_id: str
    request_id: str
    output: DecodedOutput
    confidence: float
    verdict: VerdictEnum
    rounds_executed: int
    total_ms: float
    alignment_delta: float
    breach: BreachReport | None = None


@dataclass(frozen=True, slots=True)
class VerdictBundle:
    """Reconciled, business-friendly wrapper around :class:`ExecuteResponse`."""

    swarm_id: str
    request_id: str
    verdict: VerdictEnum
    decoded: DecodedOutput
    confidence: float
    alignment_delta: float
    rounds_executed: int
    total_ms: float
    breach: BreachReport | None = None

    def is_converged(self) -> bool:
        return self.verdict == VerdictEnum.CONVERGED


# ---------------------------------------------------------------------------
# Streaming types
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class TextDelta:
    text: str


@dataclass(frozen=True, slots=True)
class ServerEvent:
    kind: str
    message: str = ""
    labels: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True, slots=True)
class StreamChunk:
    """Python mirror of ``efe.v1.StreamChunk``. Exactly one of the
    three payloads is set."""

    payload_kind: Literal["text_delta", "server_event", "final"]
    text_delta: TextDelta | None = None
    server_event: ServerEvent | None = None
    final: ExecuteResponse | None = None

    def __post_init__(self) -> None:
        n = sum(
            x is not None
            for x in (self.text_delta, self.server_event, self.final)
        )
        if n != 1:
            raise ValueError("StreamChunk requires exactly one payload set")


# ---------------------------------------------------------------------------
# DescribeSpec reply
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class DescribeSpecReply:
    valid: bool
    errors: tuple[str, ...] = ()
    hbm_bytes_estimate: int = 0
    rounds_expected: int = 0


# ---------------------------------------------------------------------------
# ConeSpec — admin-side payload used by CatalogBridge
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class ConeSpec:
    tenant: str
    task: str
    version: int
    cone: AlignmentCone
    created_by: str
    description: str = ""
    labels: tuple[tuple[str, str], ...] = ()

    def __post_init__(self) -> None:
        if self.version < 1:
            raise ValueError("ConeSpec.version must be >= 1")


# ---------------------------------------------------------------------------
# Utility helpers
# ---------------------------------------------------------------------------


def _is_finite(x: float) -> bool:
    # Python floats : NaN != NaN and inf == inf, so we explicitly check.
    return x == x and x not in (float("inf"), float("-inf"))


def build_swarm_id(seed: str | None = None) -> str:
    """Deterministic UUIDv4-like string for tests ; random in production."""
    if seed is not None:
        import hashlib

        return hashlib.sha256(seed.encode()).hexdigest()[:32]
    import uuid

    return uuid.uuid4().hex


# ---------------------------------------------------------------------------
# Utility : default spec builder
# ---------------------------------------------------------------------------


def default_spec_for_classify(
    *,
    embedding: tuple[float, ...],
    cone_ref: ConeRef,
    budget_ms: int = 500,
    max_rounds: int = 8,
) -> SwarmSpec:
    """Convenience builder for Mode 1 classification."""
    return SwarmSpec(
        dim_latent=len(embedding),
        problem_embedding=embedding,
        population=PopulationMix.default_classifier(),
        alignment_cone=None,
        alignment_cone_ref=cone_ref,
        max_rounds=max_rounds,
        budget=ResourceBudget(max_wall_clock_ms=budget_ms),
    )


# Re-export Any so users can type dicts with extra metadata.
_ = Any
