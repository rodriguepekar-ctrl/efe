"""Wire-neutral dataclasses that mirror the ``efe.v1`` proto messages.

Why duplicate the proto shapes?
-------------------------------

1. The rest of the platform should not depend on generated gRPC stubs.
   Unit tests, audit writers, bot planners never need ``ClassDescriptor``
   — they need plain, typed, JSON-serializable Python values.

2. The proto package may evolve (adding fields, changing defaults);
   keeping a narrow **adapter layer** here lets us absorb the change
   in a single place.

3. We enforce *semantic* invariants (non-empty population, positive
   latent dim, deadline within a sane bound) that proto3 cannot.

Every dataclass here is :func:`dataclasses.frozen`. Convert-to-proto
happens inside :mod:`efe_platform.swarm.client`; convert-from-proto
happens when decoding the ``ExecuteResponse``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import IntEnum
from typing import Any, Literal

# ---------------------------------------------------------------------------
#  Enumerations
# ---------------------------------------------------------------------------


class Verdict(IntEnum):
    """Mirrors ``efe.v1.Verdict``.

    The integer values intentionally match the proto enum so the wire
    adapter can cast cheaply. *Do not reorder*.
    """

    UNSPECIFIED = 0
    CONTINUE = 1
    CONVERGED = 2
    DIVERGENCE = 3
    MODE_COLLAPSE = 4

    @classmethod
    def from_proto(cls, v: int) -> Verdict:
        """Safe cast that defaults unknown wire values to ``UNSPECIFIED``.

        This protects us against forward-compat bumps where the server
        adds a new verdict variant the platform doesn't understand yet.
        """
        try:
            return cls(v)
        except ValueError:
            return cls.UNSPECIFIED


TemplateCode = Literal["na-sem", "na-log", "na-crit"]
"""MVP set of nano-agent template identifiers.

The Rust core defines these as a ``#[non_exhaustive]`` enum; the
literal union here is the MVP subset exposed through ``efe-platform``.
Post-MVP templates (``na-math``, ``na-code``, ``na-caus``,
``na-mem``, ``na-align``, ``na-med``) will be added as a minor-bump
extension of the Literal.
"""


# ---------------------------------------------------------------------------
#  Value objects — build up to SwarmSpec
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class PopulationEntry:
    """A ``(template, count)`` pair, validated as positive."""

    template_code: TemplateCode
    count: int

    def __post_init__(self) -> None:
        if self.count <= 0:
            raise ValueError(f"count must be > 0, got {self.count}")


@dataclass(frozen=True, slots=True)
class ConvergenceThresholds:
    """Four knobs driving when the arbiter declares ``CONVERGED``.

    Values are cross-validated by :class:`SpecBuilder`; defaults here
    are the "safe" MVP settings mirrored from the Rust-side config.
    """

    epsilon_variance: float = 1e-2
    epsilon_gradient: float = 1e-3
    epsilon_stability: float = 1e-3
    stability_window: int = 3

    def __post_init__(self) -> None:
        for name in ("epsilon_variance", "epsilon_gradient", "epsilon_stability"):
            v = getattr(self, name)
            if not 0 < v < 1:
                raise ValueError(f"{name} must be in (0,1), got {v}")
        if self.stability_window <= 0:
            raise ValueError("stability_window must be > 0")


@dataclass(frozen=True, slots=True)
class ResourceBudget:
    """Hard ceilings enforced by the Rust arbiter for a single request."""

    max_wall_clock_ms: int = 5_000
    max_energy_wh: float = 0.10
    max_hbm_bytes: int = 512 * 1024 * 1024  # 512 MiB
    max_recursion_depth: int = 4

    def __post_init__(self) -> None:
        if self.max_wall_clock_ms <= 0:
            raise ValueError("max_wall_clock_ms must be > 0")
        if self.max_energy_wh < 0:
            raise ValueError("max_energy_wh must be >= 0")
        if self.max_hbm_bytes <= 0:
            raise ValueError("max_hbm_bytes must be > 0")
        if self.max_recursion_depth <= 0:
            raise ValueError("max_recursion_depth must be > 0")


@dataclass(frozen=True, slots=True)
class AnchorRef:
    """Embedding vector pinned to a semantic class."""

    values: tuple[float, ...]

    def __post_init__(self) -> None:
        if not self.values:
            raise ValueError("anchor values must be non-empty")

    def dim(self) -> int:
        return len(self.values)


@dataclass(frozen=True, slots=True)
class ConeRef:
    """Reference to a cone stored in the registry (tenant/task@version)."""

    tenant: str
    task: str
    version: int

    def __post_init__(self) -> None:
        if not self.tenant:
            raise ValueError("tenant must be non-empty")
        if not self.task:
            raise ValueError("task must be non-empty")
        if self.version < 1:
            raise ValueError("version must be >= 1")

    def key(self) -> str:
        return f"{self.tenant}/{self.task}@v{self.version}"


@dataclass(frozen=True, slots=True)
class SwarmSpec:
    """Full specification of a swarm execution.

    This is the value the bot assembles via :class:`SpecBuilder` and
    hands to :meth:`SwarmClient.execute`. It is serialization-ready
    (``dataclasses.asdict`` produces something JSON-dumpable).
    """

    problem_embedding: tuple[float, ...]
    dim_latent: int
    population: tuple[PopulationEntry, ...]
    max_rounds: int
    convergence: ConvergenceThresholds
    budget: ResourceBudget
    # One of the two cone sources must be set; enforced by SpecBuilder.
    alignment_cone: tuple[AnchorRef, ...] | None = None
    alignment_cone_ref: ConeRef | None = None
    max_projection_distance: float = 1.0
    swarm_id: str | None = None  # server fills if unset


# ---------------------------------------------------------------------------
#  Execute I/O
# ---------------------------------------------------------------------------


DecodeModeKind = Literal["text", "pass_through"]


@dataclass(frozen=True, slots=True)
class DecodeMode:
    """Output decoding policy.

    ``kind="text"`` requires :attr:`max_tokens`. ``kind="pass_through"``
    leaves the centroid unchanged for downstream post-processing.
    """

    kind: DecodeModeKind = "text"
    max_tokens: int = 128

    def __post_init__(self) -> None:
        if self.kind == "text" and self.max_tokens <= 0:
            raise ValueError("max_tokens must be > 0 in text mode")


@dataclass(frozen=True, slots=True)
class ExecuteRequest:
    """Client-side view of the ``Execute`` RPC input."""

    spec: SwarmSpec
    raw_prompt: str = ""
    request_id: str | None = None
    decode_mode: DecodeMode = field(default_factory=DecodeMode)
    deadline_ms: int | None = None  # absolute deadline override


@dataclass(frozen=True, slots=True)
class DecodedPayload:
    """Either decoded text or raw centroid values — never both."""

    text: str | None = None
    centroid: tuple[float, ...] | None = None

    def __post_init__(self) -> None:
        if (self.text is None) == (self.centroid is None):
            raise ValueError("exactly one of text/centroid must be set")

    def is_text(self) -> bool:
        return self.text is not None


@dataclass(frozen=True, slots=True)
class ExecuteResponse:
    """Client-side view of the ``Execute`` RPC output."""

    swarm_id: str
    request_id: str
    payload: DecodedPayload
    confidence: float
    verdict: Verdict
    rounds_executed: int
    total_ms: float
    alignment_delta: float


@dataclass(frozen=True, slots=True)
class SwarmOutcome:
    """High-level summary of an :class:`ExecuteResponse`.

    The bot/audit layer use :class:`SwarmOutcome` instead of the raw
    :class:`ExecuteResponse` because the outcome is the minimal,
    opinionated view: it fuses verdict + confidence + payload kind
    into a single object that fits cleanly in JSON transcripts.
    """

    request_id: str
    swarm_id: str
    verdict: Verdict
    confidence: float
    text: str | None
    centroid_dim: int | None
    rounds_executed: int
    total_ms: float
    alignment_delta: float

    @classmethod
    def from_response(cls, resp: ExecuteResponse) -> SwarmOutcome:
        return cls(
            request_id=resp.request_id,
            swarm_id=resp.swarm_id,
            verdict=resp.verdict,
            confidence=resp.confidence,
            text=resp.payload.text,
            centroid_dim=(
                len(resp.payload.centroid) if resp.payload.centroid is not None else None
            ),
            rounds_executed=resp.rounds_executed,
            total_ms=resp.total_ms,
            alignment_delta=resp.alignment_delta,
        )

    def to_dict(self) -> dict[str, Any]:
        return {
            "request_id": self.request_id,
            "swarm_id": self.swarm_id,
            "verdict": self.verdict.name,
            "confidence": round(self.confidence, 4),
            "text": self.text,
            "centroid_dim": self.centroid_dim,
            "rounds_executed": self.rounds_executed,
            "total_ms": round(self.total_ms, 2),
            "alignment_delta": round(self.alignment_delta, 4),
        }


__all__ = [
    "AnchorRef",
    "ConeRef",
    "ConvergenceThresholds",
    "DecodeMode",
    "DecodeModeKind",
    "DecodedPayload",
    "ExecuteRequest",
    "ExecuteResponse",
    "PopulationEntry",
    "ResourceBudget",
    "SwarmOutcome",
    "SwarmSpec",
    "TemplateCode",
    "Verdict",
]
