"""Layer registry — orthogonal experimentation axes.

M08 lives on **Google's overlapping-experiment-infrastructure** model
(Tang et al. 2010): each layer is a routing axis along which a user is
assigned independently. Running one experiment per layer means two
experiments on different layers can coexist *without* statistical
interference (invariant **I4**).

The MVP ships with three default layers:

* ``prompt``  — system prompt / few-shot / template changes.
* ``model``   — weights swap (e.g. Llama-7B → Mistral-7B).
* ``postproc`` — output filter / retry / safety rewrite.

Each layer is defined by a ``(name, salt)`` pair where the salt feeds
the MurmurHash3 input so that two layers with distinct salts produce
statistically independent hash streams.

Salt rotation: changing a layer's salt invalidates every cached
assignment inside that layer — used to "purge the epoch" if a hash
contamination is suspected.
"""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass

from efe_rollout.config import LayerSettings
from efe_rollout.errors import UnknownLayer


@dataclass(frozen=True, slots=True)
class Layer:
    name: str
    salt: str


class LayerRegistry:
    """Resolves layer names → salts + enforces the known-layers policy.

    When :attr:`allow_custom_layers` is False (production default
    mode), attempting to resolve an unknown layer raises
    :class:`UnknownLayer` immediately.
    """

    __slots__ = ("_allow_custom", "_by_name")

    def __init__(
        self,
        layers: Iterable[Layer],
        *,
        allow_custom_layers: bool = True,
    ) -> None:
        self._by_name: dict[str, Layer] = {
            layer.name: layer for layer in layers
        }
        self._allow_custom: bool = bool(allow_custom_layers)

    # ---- factories ------------------------------------------------------

    @classmethod
    def from_settings(cls, settings: LayerSettings) -> LayerRegistry:
        return cls(
            (Layer(name=s.name, salt=s.salt) for s in settings.layers),
            allow_custom_layers=settings.allow_custom_layers,
        )

    @classmethod
    def default(cls) -> LayerRegistry:
        return cls.from_settings(LayerSettings())

    # ---- queries --------------------------------------------------------

    def __contains__(self, name: object) -> bool:
        return isinstance(name, str) and name in self._by_name

    def names(self) -> tuple[str, ...]:
        return tuple(sorted(self._by_name))

    def get(self, name: str) -> Layer:
        if name in self._by_name:
            return self._by_name[name]
        if self._allow_custom:
            # Custom layers still need a salt — synthesize deterministically.
            return Layer(name=name, salt=f"custom::{name}")
        raise UnknownLayer(
            f"layer {name!r} not registered (known layers: "
            f"{sorted(self._by_name)})",
        )

    def salt(self, name: str) -> str:
        return self.get(name).salt

    def register(self, layer: Layer) -> None:
        """Install a new layer after construction. Useful in tests and
        when ``allow_custom_layers=False`` admins enable a new axis."""
        self._by_name[layer.name] = layer

    def rotate(self, name: str, new_salt: str) -> None:
        """Swap a layer's salt — invalidates every cached assignment.

        Callers are expected to flush Redis ``rollout:assign:*`` keys
        for the tenant in the same breath.
        """
        if name not in self._by_name:
            raise UnknownLayer(
                f"cannot rotate unknown layer {name!r}",
            )
        self._by_name[name] = Layer(name=name, salt=new_salt)


__all__ = ["Layer", "LayerRegistry"]
