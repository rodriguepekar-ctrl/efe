"""Validated builder for :class:`SwarmSpec`.

The bot does not construct ``SwarmSpec`` directly — it funnels every
request through :class:`SpecBuilder`, which:

* Enforces **cross-field invariants** proto3 cannot express (population
  sums to a reasonable total, embedding dim matches ``dim_latent``,
  exactly one cone source is set, budget is consistent with deadline).

* Centralizes **safe defaults** drawn from :class:`PlatformConfig`
  so callers don't have to remember every field.

* Converts user-friendly inputs (e.g. a list of Python ``float``s) into
  the ``tuple`` shapes the neutral dataclasses require.

The builder is immutable — each mutator returns a new builder. This
keeps call sites clean and makes the object safe to pass between
coroutines without risk of late mutation.
"""

from __future__ import annotations

import math
from collections.abc import Iterable, Mapping, Sequence
from dataclasses import replace
from typing import cast

from efe_platform.errors import SwarmInvalidSpec
from efe_platform.swarm.protocol import (
    AnchorRef,
    ConeRef,
    ConvergenceThresholds,
    PopulationEntry,
    ResourceBudget,
    SwarmSpec,
    TemplateCode,
)

# ---------------------------------------------------------------------------
#  Constants
# ---------------------------------------------------------------------------

#: Population defaults — a balanced MVP mix of semantic, logic, and
#: critique agents. Sum is 1024 which is the canonical swarm size.
_DEFAULT_POPULATION: tuple[tuple[TemplateCode, int], ...] = (
    ("na-sem", 512),
    ("na-log", 384),
    ("na-crit", 128),
)

_VALID_TEMPLATES: frozenset[str] = frozenset({"na-sem", "na-log", "na-crit"})

_MIN_LATENT_DIM = 16
_MAX_LATENT_DIM = 65_536
_MIN_POPULATION_TOTAL = 32
_MAX_POPULATION_TOTAL = 16_384
_MIN_MAX_ROUNDS = 1
_MAX_MAX_ROUNDS = 64


# ---------------------------------------------------------------------------
#  Builder
# ---------------------------------------------------------------------------


class SpecBuilder:
    """Fluent, immutable builder producing a :class:`SwarmSpec`.

    Typical usage::

        spec = (
            SpecBuilder()
            .with_problem_embedding(vec, dim_latent=384)
            .with_default_population()
            .with_cone_ref(ConeRef("acme", "support", 2))
            .with_budget_ms(3_500)
            .build()
        )
    """

    __slots__ = (
        "_budget",
        "_cone_anchors",
        "_cone_ref",
        "_convergence",
        "_dim_latent",
        "_max_projection_distance",
        "_max_rounds",
        "_population",
        "_problem_embedding",
        "_swarm_id",
    )

    def __init__(self) -> None:
        self._problem_embedding: tuple[float, ...] | None = None
        self._dim_latent: int | None = None
        self._population: tuple[PopulationEntry, ...] = tuple(
            PopulationEntry(t, c) for t, c in _DEFAULT_POPULATION
        )
        self._max_rounds: int = 8
        self._convergence: ConvergenceThresholds = ConvergenceThresholds()
        self._budget: ResourceBudget = ResourceBudget()
        self._cone_anchors: tuple[AnchorRef, ...] | None = None
        self._cone_ref: ConeRef | None = None
        self._max_projection_distance: float = 1.0
        self._swarm_id: str | None = None

    # ------------------------------------------------------- Immutable clone

    def _clone(self) -> SpecBuilder:
        new = SpecBuilder()
        new._problem_embedding = self._problem_embedding
        new._dim_latent = self._dim_latent
        new._population = self._population
        new._max_rounds = self._max_rounds
        new._convergence = self._convergence
        new._budget = self._budget
        new._cone_anchors = self._cone_anchors
        new._cone_ref = self._cone_ref
        new._max_projection_distance = self._max_projection_distance
        new._swarm_id = self._swarm_id
        return new

    # ---------------------------------------------------------- Mutators

    def with_problem_embedding(
        self, vec: Sequence[float], *, dim_latent: int | None = None
    ) -> SpecBuilder:
        """Set the problem embedding; dim_latent defaults to len(vec)."""
        if not vec:
            raise SwarmInvalidSpec("problem_embedding must not be empty")
        dim = dim_latent if dim_latent is not None else len(vec)
        if dim != len(vec):
            raise SwarmInvalidSpec(
                f"dim_latent={dim} mismatches embedding length={len(vec)}"
            )
        if not _MIN_LATENT_DIM <= dim <= _MAX_LATENT_DIM:
            raise SwarmInvalidSpec(
                f"dim_latent must be in [{_MIN_LATENT_DIM},{_MAX_LATENT_DIM}], got {dim}"
            )
        # NaN / Inf / non-finite rejection — the Rust server expects finite f32.
        for i, v in enumerate(vec):
            if not math.isfinite(float(v)):
                raise SwarmInvalidSpec(f"non-finite value at embedding[{i}]")
        n = self._clone()
        n._problem_embedding = tuple(float(x) for x in vec)
        n._dim_latent = dim
        return n

    def with_population(
        self, entries: Iterable[tuple[str, int]] | Mapping[str, int]
    ) -> SpecBuilder:
        """Override the population mix."""
        iterable: Iterable[tuple[str, int]]
        iterable = entries.items() if isinstance(entries, Mapping) else entries
        normalized: list[PopulationEntry] = []
        total = 0
        seen: set[str] = set()
        for t, c in iterable:
            if t not in _VALID_TEMPLATES:
                raise SwarmInvalidSpec(
                    f"unknown template {t!r}; valid: {sorted(_VALID_TEMPLATES)}"
                )
            if t in seen:
                raise SwarmInvalidSpec(f"duplicate template {t!r} in population mix")
            seen.add(t)
            normalized.append(PopulationEntry(cast(TemplateCode, t), int(c)))
            total += int(c)
        if not _MIN_POPULATION_TOTAL <= total <= _MAX_POPULATION_TOTAL:
            raise SwarmInvalidSpec(
                f"population total must be in "
                f"[{_MIN_POPULATION_TOTAL},{_MAX_POPULATION_TOTAL}], got {total}"
            )
        n = self._clone()
        n._population = tuple(normalized)
        return n

    def with_default_population(self) -> SpecBuilder:
        """Reset to the canonical 512/384/128 mix (explicit intent)."""
        n = self._clone()
        n._population = tuple(PopulationEntry(t, c) for t, c in _DEFAULT_POPULATION)
        return n

    def with_max_rounds(self, n_rounds: int) -> SpecBuilder:
        if not _MIN_MAX_ROUNDS <= n_rounds <= _MAX_MAX_ROUNDS:
            raise SwarmInvalidSpec(
                f"max_rounds must be in [{_MIN_MAX_ROUNDS},{_MAX_MAX_ROUNDS}], got {n_rounds}"
            )
        n = self._clone()
        n._max_rounds = n_rounds
        return n

    def with_convergence(self, thresholds: ConvergenceThresholds) -> SpecBuilder:
        n = self._clone()
        n._convergence = thresholds
        return n

    def with_budget(self, budget: ResourceBudget) -> SpecBuilder:
        n = self._clone()
        n._budget = budget
        return n

    def with_budget_ms(self, wall_clock_ms: int) -> SpecBuilder:
        """Shortcut to set just the wall-clock leg of the budget."""
        n = self._clone()
        n._budget = replace(self._budget, max_wall_clock_ms=wall_clock_ms)
        return n

    def with_cone_anchors(
        self, anchors: Iterable[Sequence[float]], *, max_projection_distance: float = 1.0
    ) -> SpecBuilder:
        """Set an inline cone (array of anchors)."""
        refs = tuple(AnchorRef(tuple(float(x) for x in a)) for a in anchors)
        if not refs:
            raise SwarmInvalidSpec("cone must contain at least one anchor")
        dims = {a.dim() for a in refs}
        if len(dims) != 1:
            raise SwarmInvalidSpec(f"anchor dimensions must all match, got {dims}")
        if max_projection_distance <= 0:
            raise SwarmInvalidSpec("max_projection_distance must be > 0")
        n = self._clone()
        n._cone_anchors = refs
        n._cone_ref = None
        n._max_projection_distance = max_projection_distance
        return n

    def with_cone_ref(self, ref: ConeRef) -> SpecBuilder:
        """Reference a registered cone by canonical key."""
        n = self._clone()
        n._cone_ref = ref
        n._cone_anchors = None
        return n

    def with_swarm_id(self, swarm_id: str) -> SpecBuilder:
        if not swarm_id:
            raise SwarmInvalidSpec("swarm_id must be non-empty")
        n = self._clone()
        n._swarm_id = swarm_id
        return n

    # ------------------------------------------------------------- Build

    def build(self) -> SwarmSpec:
        """Produce the immutable :class:`SwarmSpec`, or raise.

        Validates:
        - problem embedding is set,
        - exactly one cone source is set,
        - anchor dim matches dim_latent (if inline cone),
        - budget is consistent with max_rounds.
        """
        if self._problem_embedding is None or self._dim_latent is None:
            raise SwarmInvalidSpec("problem_embedding is required")
        if (self._cone_anchors is None) == (self._cone_ref is None):
            raise SwarmInvalidSpec(
                "exactly one of cone_anchors or cone_ref must be set"
            )
        if self._cone_anchors is not None:
            cone_dim = self._cone_anchors[0].dim()
            if cone_dim != self._dim_latent:
                raise SwarmInvalidSpec(
                    f"cone anchors dim={cone_dim} != dim_latent={self._dim_latent}"
                )
        # Rough sanity: budget must allow >=1ms per round.
        min_budget = self._max_rounds
        if self._budget.max_wall_clock_ms < min_budget:
            raise SwarmInvalidSpec(
                f"budget wall_clock_ms={self._budget.max_wall_clock_ms} "
                f"too small for max_rounds={self._max_rounds}"
            )
        return SwarmSpec(
            problem_embedding=self._problem_embedding,
            dim_latent=self._dim_latent,
            population=self._population,
            max_rounds=self._max_rounds,
            convergence=self._convergence,
            budget=self._budget,
            alignment_cone=self._cone_anchors,
            alignment_cone_ref=self._cone_ref,
            max_projection_distance=self._max_projection_distance,
            swarm_id=self._swarm_id,
        )


__all__ = ["SpecBuilder"]
