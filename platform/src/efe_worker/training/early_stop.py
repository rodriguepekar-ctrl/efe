"""Early stopping — simple patience counter.

The training loop calls :meth:`EarlyStopping.step` after each eval
pass. If the chosen ``metric`` hasn't improved for ``patience`` eval
passes, :meth:`should_stop` returns True and the driver breaks out of
the loop.
"""

from __future__ import annotations

from typing import Literal

Mode = Literal["min", "max"]


class EarlyStopping:
    """Tracks best-seen metric; signals when patience is exhausted."""

    def __init__(
        self,
        metric: str,
        *,
        patience: int = 3,
        mode: Mode = "min",
        min_delta: float = 0.0,
    ) -> None:
        if patience < 1:
            raise ValueError("patience must be >= 1")
        if mode not in ("min", "max"):
            raise ValueError(f"unknown mode {mode!r}")
        self.metric = metric
        self.patience = int(patience)
        self.mode: Mode = mode
        self.min_delta = float(min_delta)
        self._best: float | None = None
        self._bad = 0
        self._best_step = -1

    @property
    def best(self) -> float | None:
        return self._best

    @property
    def best_step(self) -> int:
        return self._best_step

    @property
    def bad_count(self) -> int:
        return self._bad

    def _improved(self, value: float) -> bool:
        if self._best is None:
            return True
        if self.mode == "max":
            return value > self._best + self.min_delta
        return value < self._best - self.min_delta

    def step(self, metrics: dict[str, float], *, at_step: int) -> bool:
        """Update state with a new metric reading.

        Returns True if the training loop should stop, else False.
        The ``metric`` key must exist in ``metrics`` — if not, we
        silently skip (no improvement signal, no bad count bump).
        """
        if self.metric not in metrics:
            return self._bad >= self.patience
        value = float(metrics[self.metric])
        if self._improved(value):
            self._best = value
            self._best_step = int(at_step)
            self._bad = 0
            return False
        self._bad += 1
        return self._bad >= self.patience

    def reset(self) -> None:
        self._best = None
        self._bad = 0
        self._best_step = -1


__all__ = ["EarlyStopping", "Mode"]
