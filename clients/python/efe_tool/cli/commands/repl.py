"""Dispatcher for ``efe-admin repl`` subcommand.

Bridges the CLI's :class:`ResolvedConfig` into the REPL entry point
:func:`efe_tool.repl.run_session`. Keeps the CLI command thin: this
file adds nothing on top of the REPL except parameter plumbing.
"""

from __future__ import annotations

from typing import Any

from ..config import ResolvedConfig
from ..logging import Logger


def run(cfg: ResolvedConfig, log: Logger) -> int:
    """Entry point called by ``cli.main._dispatch`` when ``cfg.command == 'repl'``."""
    from efe_tool.repl import run_session

    return run_session(
        cfg,
        log=log,
        history_path=cfg.history_path,
        history_enabled=not cfg.no_history,
        prompt_template=cfg.prompt_template or "EFE {catalog}> ",
    )


__all__ = ["run"]
