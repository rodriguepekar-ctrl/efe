"""``python -m efe_rollout`` — forwards to :func:`efe_rollout.cli.main`."""

from __future__ import annotations

import sys

from efe_rollout.cli import main

if __name__ == "__main__":  # pragma: no cover
    sys.exit(main(sys.argv[1:]))
