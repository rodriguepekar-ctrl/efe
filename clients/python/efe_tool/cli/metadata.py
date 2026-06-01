"""Static constants for the ``efe-admin`` CLI.

Everything here is importable at zero cost — no heavy dependency,
no side effect. This module is allowed to be imported at the top of
``cli.main`` before any argument parsing.

The public surface:

    * :data:`SCHEMA_VERSION` — the contract version of the JSON
      document we emit on stdout (see ``LETTER_A_ARCHITECTURE.md`` §9).
    * :data:`DEFAULTS` — authoritative defaults for every resolved
      configuration key. The resolver (``cli.config``) consults this
      dict when neither a flag nor an env var has been provided.
    * :data:`ENV_KEYS` — the canonical mapping ``config_key → env var``.

Invariants
----------

* ``DEFAULTS`` and ``ENV_KEYS`` are kept in strict lockstep. If you add
  a new configuration key, add it to *both* or neither.
* Changes to any default are **non-breaking** provided that
  :data:`SCHEMA_VERSION` is not incremented.
* ``SCHEMA_VERSION`` is an integer. Increment by +1 on a breaking
  change to the stdout JSON. Document the break in ``CHANGELOG.md``.

This module never imports anything outside the standard library.
"""

from __future__ import annotations

import os
from typing import Final, Mapping


# ---------------------------------------------------------------------------
# Program identity
# ---------------------------------------------------------------------------

#: Canonical program name as invoked from a shell. Kept in strict lockstep
#: with ``pyproject.toml``'s ``[project.scripts]`` entry.
PROG_CANONICAL: Final[str] = "efe-admin"

#: Short alias exposed alongside ``efe-admin``. Both point to
#: ``efe_tool.cli:main``.
PROG_ALIAS: Final[str] = "efe"

#: One-line description reused by argparse, help and man page.
PROG_DESCRIPTION: Final[str] = (
    "EFE — Essaim Fluide Éphémère — Mode-1 command-line client."
)

#: Tagline used at the top of ``--help``.
PROG_TAGLINE: Final[str] = (
    "Classify, compile, validate, and probe EFE catalogs from a shell."
)


# ---------------------------------------------------------------------------
# Output schema version
# ---------------------------------------------------------------------------

#: Version of the JSON document emitted on stdout by ``classify``,
#: ``healthcheck``, ``compile-catalog`` and ``validate``.
#:
#: The consumer is expected to gate on this value:
#:
#:     result = subprocess.run(..., capture_output=True)
#:     doc = json.loads(result.stdout)
#:     if doc["schema_version"] != 1:
#:         raise RuntimeError("unsupported CLI schema version")
#:
#: Breaking changes (removed or renamed fields, changed types) require
#: a +1 bump and an entry in ``CHANGELOG.md``.
SCHEMA_VERSION: Final[int] = 1


# ---------------------------------------------------------------------------
# Text input limits
# ---------------------------------------------------------------------------

#: Maximum size (in Unicode code points) of a text to classify. Above
#: this threshold, the CLI raises a ``UsageError``. Chosen to bound
#: the cost of an embedder pass on CPU.
MAX_TEXT_LENGTH: Final[int] = 100_000

#: Soft threshold above which a warning is emitted. The classification
#: still proceeds.
WARN_TEXT_LENGTH: Final[int] = 10_000

#: Maximum size (in bytes) of a file passed via ``--text-file``. Slightly
#: larger than MAX_TEXT_LENGTH to account for multibyte UTF-8.
MAX_TEXT_FILE_BYTES: Final[int] = 4 * MAX_TEXT_LENGTH


# ---------------------------------------------------------------------------
# Configuration defaults
# ---------------------------------------------------------------------------

#: Authoritative defaults for every configuration key.
#:
#: Every key here MUST appear in :data:`ENV_KEYS` (possibly with
#: ``None`` meaning "no env override").
DEFAULTS: Final[Mapping[str, object]] = {
    # Network
    "endpoint": "127.0.0.1:50051",
    "tls": False,
    "tls_ca": None,
    "jwt": None,
    "tenant": None,
    # Catalog
    "catalog_path": None,
    "embedder_model": "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2",
    # Classification tuning
    "budget_ms": 5_000,
    "timeout_s": 10.0,
    "max_rounds": 16,
    "strict": False,
    # Output
    "format": "json",
    "pretty": False,
    "with_diagnostics": False,
    # Logging
    "log_level": "info",
    "log_format": "text",
    "quiet": False,
    "no_color": False,
    "request_id": None,      # auto-generated if absent
    "progress": "auto",      # "auto" | "always" | "never"
}


#: Canonical mapping ``config_key → env var``.
#:
#: A value of ``None`` means the key has no env override (it's
#: always set explicitly or defaulted). The special case ``no_color``
#: is mapped to the standard ``NO_COLOR`` convention (any non-empty
#: value counts as truthy, as per https://no-color.org/).
ENV_KEYS: Final[Mapping[str, str | None]] = {
    "endpoint": "EFE_ENDPOINT",
    "tls": "EFE_TLS",
    "tls_ca": "EFE_TLS_CA",
    "jwt": "EFE_JWT",
    "tenant": "EFE_TENANT",
    "catalog_path": "EFE_CATALOG_PATH",
    "embedder_model": "EFE_EMBEDDER",
    "budget_ms": "EFE_BUDGET_MS",
    "timeout_s": "EFE_TIMEOUT_S",
    "max_rounds": "EFE_MAX_ROUNDS",
    "strict": "EFE_STRICT",
    "format": "EFE_OUTPUT_FORMAT",
    "pretty": None,
    "with_diagnostics": None,
    "log_level": "EFE_LOG_LEVEL",
    "log_format": "EFE_LOG_FORMAT",
    "quiet": None,
    "no_color": "NO_COLOR",
    "request_id": "EFE_REQUEST_ID",
    "progress": None,
}


# ---------------------------------------------------------------------------
# Enum domains
# ---------------------------------------------------------------------------

LOG_LEVELS: Final[tuple[str, ...]] = ("debug", "info", "warn", "error")
LOG_FORMATS: Final[tuple[str, ...]] = ("text", "json")
OUTPUT_FORMATS: Final[tuple[str, ...]] = ("json", "text", "table")
PROGRESS_MODES: Final[tuple[str, ...]] = ("auto", "always", "never")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def parse_bool_env(raw: str | None) -> bool | None:
    """Parse a boolean env var using the same rules as :mod:`argparse`'s
    ``store_true`` + a tolerant truthy/falsy interpretation.

    Returns ``None`` if ``raw`` is ``None`` or the empty string (meaning
    "no override"). Raises ``ValueError`` on ambiguous content so the
    resolver can surface it as a :class:`UsageError`.
    """
    if raw is None or raw == "":
        return None
    norm = raw.strip().lower()
    if norm in {"1", "true", "yes", "on", "y", "t"}:
        return True
    if norm in {"0", "false", "no", "off", "n", "f"}:
        return False
    raise ValueError(f"ambiguous boolean: {raw!r}")


def no_color_from_env() -> bool:
    """Implement the ``NO_COLOR`` convention.

    Per https://no-color.org/, the mere presence of ``NO_COLOR`` in the
    environment — irrespective of its value, even empty — disables
    color output. We honour that exact semantic here.
    """
    return "NO_COLOR" in os.environ
