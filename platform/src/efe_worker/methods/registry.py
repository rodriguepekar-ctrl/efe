"""Method registry — maps canonical names to classes.

The registry is populated eagerly at import time for the 5 built-in
methods, and can be extended at runtime via :func:`register_method`
(useful for tests that want to plug in fakes).
"""

from __future__ import annotations

from typing import Any

from efe_worker.errors import UnknownRecipe
from efe_worker.methods.base import TrainingMethod

#: Canonical name → factory (class or callable that returns an instance).
METHOD_REGISTRY: dict[str, Any] = {}


def register_method(name: str, factory: Any) -> None:
    """Register ``name`` → ``factory``. Overwrites silently."""
    if not name:
        raise ValueError("method name must be non-empty")
    METHOD_REGISTRY[name] = factory


def resolve_method(
    name: str, hyperparameters: dict[str, Any] | None = None,
) -> TrainingMethod:
    """Instantiate a method by name."""
    try:
        factory = METHOD_REGISTRY[name]
    except KeyError as exc:
        known = sorted(METHOD_REGISTRY)
        raise UnknownRecipe(
            f"unknown method {name!r}; known: {known}",
        ) from exc
    instance: TrainingMethod = factory(hyperparameters or {})
    return instance


def _bootstrap() -> None:
    """Import the built-in methods so they register themselves."""
    # Local imports to avoid heavy side-effects at top-level.
    from efe_worker.methods.classif import ClassifMethod
    from efe_worker.methods.distill import DistillMethod
    from efe_worker.methods.infonce import InfoNCEMethod
    from efe_worker.methods.lora import LoRAMethod
    from efe_worker.methods.triplet import TripletMethod

    register_method("infonce", InfoNCEMethod)
    register_method("triplet", TripletMethod)
    register_method("classif", ClassifMethod)
    register_method("lora", LoRAMethod)
    register_method("distill", DistillMethod)


_bootstrap()


__all__ = ["METHOD_REGISTRY", "register_method", "resolve_method"]
