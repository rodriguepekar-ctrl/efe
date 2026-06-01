#!/usr/bin/env python3
"""
scripts/gen_fixtures.py — produce the 3 realistic SwarmSpec fixtures
used by the Bench Foundation sprint.

Outputs (regenerate by running):

    scripts/fixtures/realistic_small.json
    scripts/fixtures/realistic_medium.json
    scripts/fixtures/realistic_large.json

Each spec uses one-hot orthogonal anchors at evenly-spaced indices,
which keeps anchor cosine distance maximal (=> avoids triggering the
I5 anchor-de-duplication invariant of `alignment::policy`).

Run: `python3 scripts/gen_fixtures.py`
"""

from __future__ import annotations
import json
import os
from typing import Any

# Available template codes — serde encodes them as PascalCase variant
# names (the human-readable kebab-case form is only used by the
# `TemplateCode::as_str()` accessor, NOT by serde Deserialize).
VALID_TEMPLATES = ("NaSem", "NaLog", "NaCrit")

OUT_DIR = os.path.join(os.path.dirname(__file__), "fixtures")


def one_hot(dim: int, index: int) -> list[float]:
    """Create a one-hot vector of given dim with `1.0` at `index`."""
    v = [0.0] * dim
    v[index] = 1.0
    return v


def evenly_spaced_anchors(dim: int, n_anchors: int) -> list[list[float]]:
    """Generate `n_anchors` orthogonal unit vectors of dimension `dim`,
    placed at evenly-spaced indices `[0, dim/n_anchors, 2*dim/n_anchors, ...]`.

    For the alignment policy, this guarantees pairwise cosine = 0
    (well below the typical de-dup threshold of 0.99).
    """
    if n_anchors > dim:
        raise ValueError(
            f"n_anchors ({n_anchors}) cannot exceed dim ({dim}) "
            "for orthogonal one-hot encoding"
        )
    step = dim // n_anchors
    return [one_hot(dim, i * step) for i in range(n_anchors)]


def build_spec(
    dim: int,
    population: list[tuple[str, int]],
    n_anchors: int,
    max_rounds: int,
    max_projection_distance: float,
) -> dict[str, Any]:
    """Build a complete SwarmSpec dict ready for JSON serialization.

    Format note: matches `efe_core::types::SwarmSpec` serde derives.
    `PopulationMix` is `#[serde(transparent)]` over
    `Vec<(TemplateCode, u32)>`, so populations are `[[code, count]]`
    pairs (NOT `[{template_code, count}]` objects). `AlignmentCone`
    serializes anchors as plain `Vec<Vec<f32>>`.
    """
    anchors = evenly_spaced_anchors(dim, n_anchors)
    return {
        "spec": {
            "dim_latent": dim,
            "problem_embedding": [0.0] * dim,
            "population": [[code, count] for (code, count) in population],
            "max_rounds": max_rounds,
            "alignment_cone": {
                "anchors": anchors,
                "max_projection_distance": max_projection_distance,
            },
        }
    }


def write_json(path: str, data: dict[str, Any]) -> None:
    """Write `data` to `path` with stable, indent=2 formatting."""
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=False)
        fh.write("\n")


def main() -> None:
    os.makedirs(OUT_DIR, exist_ok=True)

    # ------------------------------------------------------------
    # SMALL — typical Mode 1 simple request
    #   8 agents, dim=128, 4 anchors, 8 rounds
    #   Latents: 8 × 128 × 4 B = 4 KB
    #   Anchors: 4 × 128 × 4 B = 2 KB
    #   Target SLO: p95 < 50 ms
    # ------------------------------------------------------------
    small = build_spec(
        dim=128,
        population=[("NaSem", 4), ("NaLog", 4)],
        n_anchors=4,
        max_rounds=8,
        max_projection_distance=0.30,
    )
    write_json(os.path.join(OUT_DIR, "realistic_small.json"), small)

    # ------------------------------------------------------------
    # MEDIUM — Mode 1 with deeper reflection
    #   32 agents, dim=256, 8 anchors, 8 rounds
    #   Latents: 32 × 256 × 4 B = 32 KB
    #   Target SLO: p95 < 200 ms
    # ------------------------------------------------------------
    medium = build_spec(
        dim=256,
        population=[("NaSem", 16), ("NaLog", 8), ("NaCrit", 8)],
        n_anchors=8,
        max_rounds=8,
        max_projection_distance=0.25,
    )
    write_json(os.path.join(OUT_DIR, "realistic_medium.json"), medium)

    # ------------------------------------------------------------
    # LARGE — industrial / stress
    #   128 agents, dim=256, 16 anchors, 8 rounds
    #   Latents: 128 × 256 × 4 B = 128 KB
    #   Target SLO: p95 < 1 s
    # ------------------------------------------------------------
    large = build_spec(
        dim=256,
        population=[("NaSem", 64), ("NaLog", 32), ("NaCrit", 32)],
        n_anchors=16,
        max_rounds=8,
        max_projection_distance=0.20,
    )
    write_json(os.path.join(OUT_DIR, "realistic_large.json"), large)

    # Summary
    sizes = [
        ("small", small),
        ("medium", medium),
        ("large", large),
    ]
    print(f"Generated 3 fixtures in {OUT_DIR}/")
    for name, spec in sizes:
        s = spec["spec"]
        n_agents = sum(p[1] for p in s["population"])
        n_anchors = len(s["alignment_cone"]["anchors"])
        latent_kb = (n_agents * s["dim_latent"] * 4) / 1024.0
        anchor_kb = (n_anchors * s["dim_latent"] * 4) / 1024.0
        print(
            f"  realistic_{name}.json:"
            f" dim={s['dim_latent']:>3},"
            f" N={n_agents:>3},"
            f" anchors={n_anchors:>2},"
            f" rounds={s['max_rounds']},"
            f" latents={latent_kb:>5.1f} KB,"
            f" anchors={anchor_kb:>4.1f} KB"
        )


if __name__ == "__main__":
    main()
