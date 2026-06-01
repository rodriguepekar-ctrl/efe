"""Randomness abstraction — injectable :class:`Randomness` Protocol.

Jitter, stratification, tie-breaking and seed generation all go through
this protocol so tests are fully reproducible.

Principle **P5** (randomness injected) forbids direct ``random.*`` or
``secrets.*`` calls outside this module.
"""

from __future__ import annotations

import random
import secrets
from typing import Protocol, runtime_checkable

__all__ = ["FakeRandomness", "Randomness", "SystemRandomness"]


@runtime_checkable
class Randomness(Protocol):
    """Protocol for injectable randomness sources."""

    def random(self) -> float:  # pragma: no cover
        """Return uniform in [0, 1)."""
        ...

    def uniform(self, a: float, b: float) -> float:  # pragma: no cover
        """Return uniform in [a, b]."""
        ...

    def randint(self, a: int, b: int) -> int:  # pragma: no cover
        """Return integer in [a, b] inclusive."""
        ...

    def token_bytes(self, n: int) -> bytes:  # pragma: no cover
        """Return n cryptographically-strong random bytes."""
        ...


class SystemRandomness:
    """Production randomness. ``token_bytes`` uses :mod:`secrets`."""

    __slots__ = ("_rng",)

    def __init__(self) -> None:
        # Use SystemRandom for stronger seeds than the default Mersenne twister.
        self._rng = random.SystemRandom()

    def random(self) -> float:
        return self._rng.random()

    def uniform(self, a: float, b: float) -> float:
        return self._rng.uniform(a, b)

    def randint(self, a: int, b: int) -> int:
        return self._rng.randint(a, b)

    def token_bytes(self, n: int) -> bytes:
        if n < 0:
            raise ValueError("token_bytes requires n >= 0")
        return secrets.token_bytes(n)


class FakeRandomness:
    """Deterministic randomness for tests, backed by a seeded ``Random``."""

    __slots__ = ("_rng", "_seed")

    def __init__(self, seed: int = 0xEFE5_CEDE) -> None:
        self._seed = int(seed)
        self._rng = random.Random(self._seed)

    @property
    def seed(self) -> int:
        return self._seed

    def reseed(self, seed: int) -> None:
        self._seed = int(seed)
        self._rng = random.Random(self._seed)

    def random(self) -> float:
        return self._rng.random()

    def uniform(self, a: float, b: float) -> float:
        return self._rng.uniform(a, b)

    def randint(self, a: int, b: int) -> int:
        return self._rng.randint(a, b)

    def token_bytes(self, n: int) -> bytes:
        if n < 0:
            raise ValueError("token_bytes requires n >= 0")
        return self._rng.getrandbits(n * 8).to_bytes(n, "big")
