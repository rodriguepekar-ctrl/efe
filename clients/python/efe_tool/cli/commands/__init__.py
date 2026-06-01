"""Subcommand implementations for the ``efe-admin`` CLI.

Each module exports a single :func:`run` coroutine of the shape::

    def run(cfg: ResolvedConfig, logger: Logger) -> int:
        ...

``run`` must return an exit code (``0`` on success), or raise a
:class:`efe_tool.cli.errors.CliError` subclass. Anything else is a
programming bug caught by the top-level dispatcher and surfaced as
exit 126.

See ``LETTER_A_ARCHITECTURE.md`` §5.3.
"""

from __future__ import annotations

__all__: list[str] = []
