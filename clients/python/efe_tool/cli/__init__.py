"""``efe_tool.cli`` — the command-line interface for EFE Mode 1.

This package exposes a single public symbol, :func:`main`, which is
wired to the ``efe-admin`` and ``efe`` entry points in
``pyproject.toml``::

    [project.scripts]
    efe-admin = "efe_tool.cli:main"
    efe       = "efe_tool.cli:main"

Internal layout (see ``LETTER_A_ARCHITECTURE.md``):

* :mod:`efe_tool.cli.main`     — entry point, top-level dispatch.
* :mod:`efe_tool.cli.parser`   — argparse tree.
* :mod:`efe_tool.cli.config`   — layered configuration resolution.
* :mod:`efe_tool.cli.errors`   — exit-code-mapped exception taxonomy.
* :mod:`efe_tool.cli.logging`  — structured stderr logger.
* :mod:`efe_tool.cli.io_input` — text source resolution.
* :mod:`efe_tool.cli.output`   — JSON / text / table formatters.
* :mod:`efe_tool.cli.metadata` — constants, defaults, env mapping.
* :mod:`efe_tool.cli.commands` — per-subcommand implementations.

All import boundaries above are **unidirectional**: ``commands``
imports from the others, never the reverse.
"""

from __future__ import annotations

from .main import main

__all__ = ["main"]
