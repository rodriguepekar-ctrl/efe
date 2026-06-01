"""Concrete :class:`LlmProvider` implementations.

* :class:`MockProvider` — deterministic, test-only, no I/O.
* :class:`ClaudeProvider` — wraps ``anthropic`` SDK (optional dep).
* :class:`OpenAIProvider` — wraps ``openai`` SDK (optional dep).

The optional providers import their SDK lazily inside the constructor
so the base ``efe-platform`` install doesn't require them.
"""

from __future__ import annotations

from efe_platform.bot.providers.mock import MockProvider

__all__ = ["MockProvider"]

# Lazy exports
try:
    from efe_platform.bot.providers.claude import ClaudeProvider  # noqa: F401

    __all__.append("ClaudeProvider")
except ImportError:  # pragma: no cover — optional dep
    pass

try:
    from efe_platform.bot.providers.openai import OpenAIProvider  # noqa: F401

    __all__.append("OpenAIProvider")
except ImportError:  # pragma: no cover — optional dep
    pass
