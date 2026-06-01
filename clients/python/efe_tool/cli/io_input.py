"""Text input resolution for the ``classify`` subcommand.

Exactly one of three sources must be chosen:

* ``--text "..."``       — inline string
* ``--text-file PATH``   — UTF-8 file
* ``--stdin``            — read until EOF

Auto-detection: if none of the flags is set **and** stdin is not a
TTY, ``--stdin`` is implied. A TTY stdin without an explicit flag
raises :class:`UsageError` (we refuse to silently block).

Validation:

* UTF-8 strict (no replacement characters).
* ``MAX_TEXT_LENGTH`` upper bound.
* Warns above ``WARN_TEXT_LENGTH``.
* Sensitive-pattern detection warns the user when classifying what
  looks like a credential — see :func:`_warn_if_sensitive`.
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path
from typing import Any, Iterable, Optional, TextIO

from .errors import UsageError
from .metadata import (
    MAX_TEXT_FILE_BYTES,
    MAX_TEXT_LENGTH,
    WARN_TEXT_LENGTH,
)


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def read_text_input(
    *,
    text: Optional[str] = None,
    text_file: Optional[Path] = None,
    use_stdin: bool = False,
    stdin: Optional[TextIO] = None,
    logger: Any = None,
) -> str:
    """Resolve and validate the text to classify.

    Args:
        text:        Inline text, from ``--text``.
        text_file:   Path from ``--text-file``.
        use_stdin:   Explicit ``--stdin`` flag.
        stdin:       Override for tests. Defaults to :data:`sys.stdin`.
        logger:      Optional CLI logger to receive warnings.

    Raises:
        UsageError:  on ambiguous combination, empty input, out-of-bounds
                     length, or non-UTF-8 decoding.
    """
    effective_stdin = stdin if stdin is not None else sys.stdin

    sources_provided = _count_sources(text, text_file, use_stdin)
    if sources_provided == 0:
        # Auto-mode: accept stdin iff not a TTY.
        if _stream_is_tty(effective_stdin):
            raise UsageError(
                "no text source provided",
                hint=(
                    "pass --text, --text-file, or --stdin "
                    "(stdin is a TTY, we won't block silently)"
                ),
            )
        return _read_from_stdin(effective_stdin, logger=logger)

    if sources_provided > 1:
        raise UsageError(
            "conflicting text sources",
            hint="pick exactly one of --text, --text-file, or --stdin",
        )

    if text is not None:
        _warn_if_sensitive(text, source="--text (visible in `ps`)", logger=logger)
        return _validate_text(text, source="--text", logger=logger)

    if text_file is not None:
        return _read_from_file(text_file, logger=logger)

    # use_stdin
    return _read_from_stdin(effective_stdin, logger=logger)


# ---------------------------------------------------------------------------
# Internals
# ---------------------------------------------------------------------------


def _count_sources(
    text: Optional[str],
    text_file: Optional[Path],
    use_stdin: bool,
) -> int:
    return sum(
        1 for v in (text, text_file, use_stdin)
        if (v is not None if not isinstance(v, bool) else v)
    )


def _read_from_file(path: Path, *, logger: Any) -> str:
    if not path.exists():
        raise UsageError(
            f"--text-file not found: {path}",
            hint="check the path; relative paths are resolved against cwd",
        )
    try:
        size = path.stat().st_size
    except OSError as e:
        raise UsageError(f"cannot stat --text-file: {e}")
    if size > MAX_TEXT_FILE_BYTES:
        raise UsageError(
            f"--text-file too large ({size} bytes > "
            f"{MAX_TEXT_FILE_BYTES} bytes)",
            hint="split the input or raise MAX_TEXT_FILE_BYTES",
        )
    try:
        raw = path.read_text(encoding="utf-8")
    except UnicodeDecodeError as e:
        raise UsageError(
            f"--text-file is not valid UTF-8: {e}",
            hint="convert the file to UTF-8 before classifying",
        )
    return _validate_text(raw, source=f"--text-file:{path.name}", logger=logger)


def _read_from_stdin(stream: TextIO, *, logger: Any) -> str:
    # Best-effort: reconfigure to UTF-8 strict on Python ≥ 3.10 if the
    # stream exposes it. This protects Windows consoles set to cp1252.
    reconfig = getattr(stream, "reconfigure", None)
    if callable(reconfig):
        try:
            reconfig(encoding="utf-8", errors="strict")
        except Exception:  # pragma: no cover
            pass
    # Bound the read to ``MAX_TEXT_FILE_BYTES`` + 1 characters so a
    # malicious / accidental ``cat /dev/zero | efe-admin classify …``
    # fails fast instead of OOMing. The +1 lets us detect overflow
    # without doing a second read.
    limit = MAX_TEXT_FILE_BYTES + 1
    try:
        raw = stream.read(limit)
    except UnicodeDecodeError as e:
        raise UsageError(
            f"stdin is not valid UTF-8: {e}",
            hint="pipe UTF-8 bytes only",
        )
    if not raw:
        raise UsageError(
            "stdin closed with no content",
            hint="echo 'your text' | efe-admin classify --stdin --catalog ...",
        )
    if len(raw) >= limit:
        raise UsageError(
            f"stdin too large (>= {MAX_TEXT_FILE_BYTES} bytes)",
            hint="send a shorter fragment or use --text-file with chunked input",
        )
    return _validate_text(raw, source="stdin", logger=logger)


def _validate_text(raw: str, *, source: str, logger: Any) -> str:
    # Strip a trailing newline (common with `echo` and heredocs) but
    # preserve internal whitespace.
    text = raw.rstrip("\n\r")
    if not text.strip():
        raise UsageError(
            f"{source}: text is empty (whitespace-only)",
            hint="provide a non-empty string",
        )
    length = len(text)
    if length > MAX_TEXT_LENGTH:
        raise UsageError(
            f"{source}: text too long ({length} > {MAX_TEXT_LENGTH} "
            f"characters)",
            hint="classify shorter fragments or raise MAX_TEXT_LENGTH",
        )
    if length > WARN_TEXT_LENGTH and logger is not None:
        logger.warn(
            "text_large",
            source=source,
            length=length,
            warn_threshold=WARN_TEXT_LENGTH,
        )
    return text


# ---------------------------------------------------------------------------
# Sensitive-content heuristic
# ---------------------------------------------------------------------------


_SENSITIVE_PATTERNS: tuple[re.Pattern[str], ...] = (
    re.compile(r"(?i)\bpassword\b\s*[:=]"),
    re.compile(r"(?i)\bapi[_-]?key\b\s*[:=]"),
    re.compile(r"(?i)\btoken\b\s*[:=]"),
    re.compile(r"(?i)\bsecret\b\s*[:=]"),
    re.compile(r"\b[A-Za-z0-9_\-]{32,}\b"),   # long opaque token
    re.compile(r"\b\d{13,19}\b"),              # card-like number
)


def _warn_if_sensitive(text: str, *, source: str, logger: Any) -> None:
    if logger is None:
        return
    hit = next(
        (p.pattern for p in _SENSITIVE_PATTERNS if p.search(text)),
        None,
    )
    if hit is None:
        return
    logger.warn(
        "sensitive_text_on_cli",
        source=source,
        reason=(
            "the text appears to contain credentials or card numbers; "
            "prefer --stdin to avoid leaking to shell history and `ps`."
        ),
        pattern=hit,
    )


def _stream_is_tty(stream: Any) -> bool:
    isatty = getattr(stream, "isatty", None)
    if not callable(isatty):
        return False
    try:
        return bool(isatty())
    except Exception:  # pragma: no cover
        return False


# ---------------------------------------------------------------------------
# Public surface
# ---------------------------------------------------------------------------


def redact(text: str, *, limit: int = 24) -> str:
    """Produce a log-safe, length-preserving preview of ``text``.

    Used by subcommands that want to log a short sentinel without
    leaking the full payload.
    """
    n = len(text)
    if n == 0:
        return "<<empty>>"
    if n <= limit:
        return f"<<len={n}>>"
    head = text[: limit // 2].replace("\n", "\\n")
    return f"<<len={n} head={head!r}>>"


__all__ = ["read_text_input", "redact"]
