"""Bidi stream placeholder.

Declared so the public API is stable ; the actual ``bidi`` RPC is
out of scope for MVP.
"""

from __future__ import annotations

from dataclasses import dataclass

__all__ = ["BidiStreamHandle"]


@dataclass(frozen=True, slots=True)
class BidiStreamHandle:
    """Reserved type for future bidirectional streaming support."""

    stream_id: str

    def is_supported(self) -> bool:
        return False
