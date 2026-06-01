"""Module entry point — enables ``python -m efe_registry``."""

from __future__ import annotations

from efe_registry.cli import main

if __name__ == "__main__":
    raise SystemExit(main())
