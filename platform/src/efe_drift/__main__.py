"""``python -m efe_drift`` — forwards to :func:`efe_drift.cli.main`."""

from __future__ import annotations

import sys

from efe_drift.cli import main

if __name__ == "__main__":  # pragma: no cover
    sys.exit(main(sys.argv[1:]))
