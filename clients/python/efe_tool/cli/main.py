"""Top-level entry point for the ``efe-admin`` / ``efe`` CLI.

Responsibilities:

* Parse ``argv`` via :mod:`efe_tool.cli.parser`.
* Resolve configuration via :mod:`efe_tool.cli.config`.
* Set up the structured logger via :mod:`efe_tool.cli.logging`.
* Dispatch to the requested subcommand in :mod:`efe_tool.cli.commands`.
* Catch every exception and map it to a deterministic exit code via
  :mod:`efe_tool.cli.errors`.

The whole file is a thin shell around the plumbing modules. The real
interesting code lives in those modules.
"""

from __future__ import annotations

import sys
from typing import Sequence

from .config import ResolvedConfig, resolve_config
from .errors import (
    CliError,
    InterruptedError,
    UsageError,
)
from .logging import Logger, setup_logging
from .output import render_error
from .parser import build_parser


# ---------------------------------------------------------------------------
#  Dispatch table
# ---------------------------------------------------------------------------


def _dispatch(cmd: str, cfg: ResolvedConfig, log: Logger) -> int:
    """Route ``cmd`` to the right ``commands.<name>.run`` function.

    The import is *lazy* so that a ``classify`` call doesn't pay the
    cost of importing the ``eval`` runner's ``importlib`` dance, for
    example.
    """
    if cmd == "classify":
        from .commands import classify
        return classify.run(cfg, log)
    if cmd == "compile-catalog":
        from .commands import compile_catalog
        return compile_catalog.run(cfg, log)
    if cmd == "validate":
        from .commands import validate
        return validate.run(cfg, log)
    if cmd == "healthcheck":
        from .commands import healthcheck
        return healthcheck.run(cfg, log)
    if cmd == "eval":
        from .commands import eval_cmd
        return eval_cmd.run(cfg, log)
    if cmd == "version":
        from .commands import version as version_cmd
        return version_cmd.run(cfg, log)
    if cmd == "completions":
        from .commands import completions
        return completions.run(cfg, log)
    if cmd == "repl":
        from .commands import repl
        return repl.run(cfg, log)
    if cmd == "scenario":
        from .commands import scenario as scenario_cmd
        return scenario_cmd.run(cfg, log)
    if cmd == "catalog":
        from .commands import catalog as catalog_cmd
        return catalog_cmd.run(cfg, log)
    raise UsageError(f"unknown subcommand: {cmd!r}")


# ---------------------------------------------------------------------------
#  main
# ---------------------------------------------------------------------------


def main(argv: Sequence[str] | None = None) -> int:
    """Parse, resolve, dispatch, handle. Never raises."""
    # 1. Parse.
    try:
        parser = build_parser()
        raw_args = parser.parse_args(argv)
    except UsageError as e:
        # Parse failure — no logger yet; render a text error.
        render_error(e, format="text")
        return e.exit_code
    except SystemExit as e:
        # argparse emits SystemExit for --help / --version / internal
        # bugs. Preserve its code rather than masking it.
        return int(e.code) if e.code is not None else 0

    # 2. Resolve config.
    try:
        cfg = resolve_config(raw_args)
    except CliError as e:
        render_error(e, format="text")
        return e.exit_code

    # 3. Set up logging.
    log = setup_logging(cfg)

    # 4. Dispatch.
    try:
        code = _dispatch(cfg.command, cfg, log)
        return int(code)
    except CliError as e:
        render_error(e, format=cfg.log_format, pretty=cfg.pretty,
                     request_id=cfg.request_id)
        # Also log one terminal event so ``--log-format json`` users
        # see a single ndjson line summarising the failure.
        log.error(
            "cli_failed",
            error_code=e.error_code,
            exit_code=e.exit_code,
            message=e.message,
        )
        return e.exit_code
    except KeyboardInterrupt:
        err = InterruptedError("interrupted by user (SIGINT)")
        render_error(err, format=cfg.log_format)
        return err.exit_code
    except Exception as e:  # pragma: no cover — last-resort guard
        # Unexpected — this is a genuine bug. Surface it as exit 126
        # and include the exception type so ops can file a report.
        bug = CliError(f"unexpected error: {type(e).__name__}: {e}", cause=e)
        render_error(bug, format=cfg.log_format)
        log.error("cli_bug", error=str(e), type=type(e).__name__)
        return bug.exit_code


# ---------------------------------------------------------------------------
#  Entry points
# ---------------------------------------------------------------------------


def cli_entry() -> None:  # pragma: no cover — trivial wrapper
    """Module-level entry used by setuptools `console_scripts`."""
    raise SystemExit(main())


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())


__all__ = ["main", "cli_entry"]
