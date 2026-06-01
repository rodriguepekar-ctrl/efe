#!/usr/bin/env python3
"""
scripts/gen_fixtures_v2.py — Sprint D1 realistic fixtures.

Produces three fixtures `realistic_{small,medium,large}_v2.json` with
**non-degenerate problem embeddings** so the production
`HeuristicMetaAnalyzer` produces a realistic task DAG (rather than a
uniform softmax which it does on `[0.0; dim]` inputs).

Differences vs `gen_fixtures.py` (v1):
- `problem_embedding` is now a Box-Muller pseudo-Gaussian vector
  (seed = 42), scaled to ‖x‖₂ ≈ √dim · 0.1 ≈ 1.13 for dim=128.
- Anchors are unchanged (still one-hot orthogonal at evenly-spaced
  indices) — they are not the bottleneck and stable indices keep the
  alignment::policy I5 invariant happy.
- Population mix unchanged (same NaSem/NaLog/NaCrit balance).

Run: `python3 scripts/gen_fixtures_v2.py`

The bench `crates/efe-server/benches/real_strategies_e2e.rs` consumes
these v2 fixtures via `efe_server::bench::common::load_realistic_v2_spec`.
"""

from __future__ import annotations
import json
import math
import os
from typing import Any

VALID_TEMPLATES = ("NaSem", "NaLog", "NaCrit")
OUT_DIR = os.path.join(os.path.dirname(__file__), "fixtures")


# ----------------------------------------------------------------
# Pseudo-random Gaussian via Box-Muller, fully deterministic
# ----------------------------------------------------------------


class _Pcg32:
    """Minimal PCG-XSH-RR 32-bit generator (deterministic across all
    Python versions). 64-bit internal state, returns 32-bit unsigned
    integers. Reference: https://www.pcg-random.org/."""

    _MULT = 6364136223846793005
    _INC = 1442695040888963407
    _MASK64 = (1 << 64) - 1
    _MASK32 = (1 << 32) - 1

    def __init__(self, seed: int) -> None:
        self.state = (seed + self._INC) & self._MASK64
        # advance once to mix
        _ = self.next_u32()

    def next_u32(self) -> int:
        oldstate = self.state
        self.state = (oldstate * self._MULT + self._INC) & self._MASK64
        xorshifted = (((oldstate >> 18) ^ oldstate) >> 27) & self._MASK32
        rot = (oldstate >> 59) & 31
        return ((xorshifted >> rot) | (xorshifted << ((-rot) & 31))) & self._MASK32

    def next_unit(self) -> float:
        """Return a `float` in (0, 1) — strictly avoids 0 and 1 so the
        Box-Muller `log(u)` never blows up."""
        # +1 nudges off zero ; / (max+2) keeps result strictly < 1.
        return (self.next_u32() + 1) / (self._MASK32 + 2)


def gaussian_embedding(dim: int, seed: int = 42, scale: float = 0.1) -> list[float]:
    """Generate `dim` Gaussian samples via Box-Muller. Each pair of
    uniforms produces 2 Gaussians; we pick the cosine branch only for
    determinism and discard the sine branch (cheaper to reason about
    on review).

    With `scale = 0.1` the resulting vector has expected
    `‖x‖₂² ≈ dim · 0.01`, so `‖x‖₂ ≈ √dim · 0.1` (≈ 1.13 at dim=128,
    ≈ 1.6 at dim=256). Bounded magnitude keeps cosine similarities
    against orthogonal anchors well-distributed in `[-0.9, +0.9]`,
    which is what the bench wants to drive
    `HeuristicMetaAnalyzer::decompose` through realistic discrimination.
    """
    rng = _Pcg32(seed)
    out: list[float] = []
    while len(out) < dim:
        u1 = rng.next_unit()
        u2 = rng.next_unit()
        z = math.sqrt(-2.0 * math.log(u1)) * math.cos(2.0 * math.pi * u2)
        out.append(z * scale)
    return out


def one_hot(dim: int, idx: int) -> list[float]:
    v = [0.0] * dim
    v[idx] = 1.0
    return v


def evenly_spaced_anchors(dim: int, n_anchors: int) -> list[list[float]]:
    if n_anchors > dim:
        raise ValueError("n_anchors > dim")
    step = dim // n_anchors
    return [one_hot(dim, i * step) for i in range(n_anchors)]


# ----------------------------------------------------------------
# Spec composition
# ----------------------------------------------------------------


def build_spec(
    *,
    dim: int,
    population: list[tuple[str, int]],
    n_anchors: int,
    max_rounds: int,
    max_projection_distance: float,
    seed: int,
) -> dict[str, Any]:
    return {
        "spec": {
            "dim_latent": dim,
            "problem_embedding": gaussian_embedding(dim, seed=seed),
            "population": [[code, count] for (code, count) in population],
            "max_rounds": max_rounds,
            "alignment_cone": {
                "anchors": evenly_spaced_anchors(dim, n_anchors),
                "max_projection_distance": max_projection_distance,
            },
        }
    }


def write_json(path: str, data: dict[str, Any]) -> None:
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2, sort_keys=False)
        fh.write("\n")


def main() -> None:
    os.makedirs(OUT_DIR, exist_ok=True)

    # Sprint D7 — `max_projection_distance` harmonised at 0.50 across
    # all sizes to match the production catalog (`catalogs/support_triage.yaml`).
    # The pre-D7 values (0.30 / 0.25 / 0.20) were calibrated against
    # the old aggressive α₀=0.50 fusion schedule that pulled the
    # centroid hard onto the cone. With D7's softer α₀=0.30 the
    # centroid stays closer to the input — the production cone radius
    # of 0.50 is the meaningful test boundary, not the synthetic
    # tighter values that only worked because the old schedule
    # over-projected.
    sizes = [
        ("small_v2", dict(
            dim=128,
            population=[("NaSem", 4), ("NaLog", 4)],
            n_anchors=4,
            max_rounds=8,
            max_projection_distance=0.50,
            seed=42,
        )),
        ("medium_v2", dict(
            dim=256,
            population=[("NaSem", 16), ("NaLog", 8), ("NaCrit", 8)],
            n_anchors=8,
            max_rounds=8,
            max_projection_distance=0.50,
            seed=42,
        )),
        ("large_v2", dict(
            dim=256,
            population=[("NaSem", 64), ("NaLog", 32), ("NaCrit", 32)],
            n_anchors=16,
            max_rounds=8,
            max_projection_distance=0.50,
            seed=42,
        )),
    ]

    print(f"Generated 3 fixtures in {OUT_DIR}/")
    for name, params in sizes:
        spec = build_spec(**params)
        path = os.path.join(OUT_DIR, f"realistic_{name}.json")
        write_json(path, spec)
        s = spec["spec"]
        emb = s["problem_embedding"]
        emb_norm = math.sqrt(sum(v * v for v in emb))
        n_agents = sum(p[1] for p in s["population"])
        print(
            f"  realistic_{name}.json:"
            f" dim={s['dim_latent']:>3},"
            f" N={n_agents:>3},"
            f" anchors={len(s['alignment_cone']['anchors']):>2},"
            f" rounds={s['max_rounds']},"
            f" ‖embedding‖₂={emb_norm:>4.2f}"
        )


if __name__ == "__main__":
    main()
