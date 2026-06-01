"""Single source of truth for :mod:`efe_rollout` semver.

Kept isolated so ``from efe_rollout.version import __version__`` never
drags in the rest of the package (useful for packaging tools).
"""

from __future__ import annotations

__version__ = "0.1.0"

__all__ = ["__version__"]
