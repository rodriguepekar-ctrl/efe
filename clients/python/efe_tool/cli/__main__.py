"""Support ``python -m efe_tool.cli`` invocation.

Delegates to :func:`efe_tool.cli.main.main` so that tests and
debugging sessions can run the CLI without relying on the console
script installation.
"""

from __future__ import annotations

import sys

from .main import main


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
