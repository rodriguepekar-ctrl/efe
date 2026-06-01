"""CLI entry point for `efe-cli`.

Sprint 0: just prints version. Sprint 6 (PR-053) adds `solve`, `bench`, etc.
"""

from __future__ import annotations

import sys

from efe import __version__


def main(argv: list[str] | None = None) -> int:
    """Execute the CLI."""
    args = argv if argv is not None else sys.argv[1:]
    if args and args[0] in ("-v", "--version"):
        print(f"efe {__version__}")
        return 0
    print(f"efe {__version__} (Sprint 0 stub - `solve` available Sprint 6)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
