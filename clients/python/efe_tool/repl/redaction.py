"""**Sprint D19 — DT-B-4** — Secret redaction for REPL `:debug` logs.

Anti-leak filter that scrubs JWT bearer tokens, AWS / GCP / Azure /
Stripe / generic API keys from any string before it lands in
the `:debug enable` logs or in any structured event the REPL
emits to stderr.

Why a dedicated module ?

* The REPL is long-running ; an operator can leave `:debug` on
  for hours. A single accidental `print(headers)` containing
  `Authorization: Bearer eyJ...` is a credentials leak →
  RGPD breach + compromised IdP.

* The redaction is **defence-in-depth** — operators are told
  via `LETTER_B §13` to NEVER paste secrets in the REPL ; this
  module catches the cases where they do anyway.

* The patterns are **deny-list-driven** : we redact what we know
  is sensitive, leave the rest. Operators who ship custom secret
  formats can register additional patterns via
  :func:`add_pattern`.

Public API :

  * :func:`redact(text)` — apply all registered patterns ; safe
    on any input (returns input unchanged when no match).
  * :func:`redact_mapping(mapping)` — recursively redact a
    dict / list (used by structured event emission).
  * :func:`add_pattern(name, pattern, replacement)` — extension
    point for app-specific secret shapes.
  * :data:`PATTERNS` — the canonical list (read-only by convention).

Pin tested by :
  * `scripts/tests/test_d19_dt_b4_redaction_in_debug_logs`
"""

from __future__ import annotations

import re
from collections.abc import Mapping as ABCMapping
from typing import Any, Final, Pattern


# ---------------------------------------------------------------------------
#  Patterns
#
#  Order matters : longer / more specific patterns FIRST so they
#  match before a more permissive fallback. Each entry is
#  (name, compiled_pattern, replacement).
#
#  The patterns are deliberately conservative — false positives
#  (over-redacting) are ALWAYS preferred over false negatives
#  (leaking a secret). Operators who hit a false positive can
#  add a pre-redaction allowlist via :func:`add_pattern` with
#  a higher-priority sentinel.
# ---------------------------------------------------------------------------


def _compile(pattern: str, flags: int = 0) -> Pattern[str]:
    return re.compile(pattern, flags)


#: Canonical secret patterns. Each tuple = (name, compiled_pattern, replacement).
#:
#: Names are stable and surface in tests + audit log entries
#: ("redacted_jwt", "redacted_bearer", ...).
_BUILTIN_PATTERNS: Final[tuple[tuple[str, Pattern[str], str], ...]] = (
    # JWT — three base64url segments separated by dots, header always
    # starts with `eyJ` (b64 of `{`). The middle segment can be of
    # arbitrary length ; we cap at 4096 to avoid catastrophic
    # backtracking on adversarial input.
    (
        "jwt",
        _compile(
            r"\beyJ[A-Za-z0-9_\-]{8,4096}"
            r"\.eyJ[A-Za-z0-9_\-]{8,4096}"
            r"\.[A-Za-z0-9_\-]{8,4096}\b"
        ),
        "<REDACTED-JWT>",
    ),
    # Authorization: Bearer <token> — case-insensitive header style.
    # We keep "Bearer " in the output so logs remain parseable.
    (
        "bearer",
        _compile(r"(?i)\bbearer\s+[A-Za-z0-9_.\-+/=]{16,}"),
        "Bearer <REDACTED>",
    ),
    # AWS access key id — `AKIA` + 16 chars (regex from AWS doc).
    (
        "aws_access_key",
        _compile(r"\bAKIA[0-9A-Z]{16}\b"),
        "<REDACTED-AWS-KEY>",
    ),
    # AWS secret access key — opportunistic ; matches when the
    # caller writes ``aws_secret_access_key=...`` style envs.
    (
        "aws_secret_kv",
        _compile(
            r"(?i)(aws_secret_access_key|aws_secret_key)\s*[=:]\s*"
            r"[A-Za-z0-9/+=]{30,60}"
        ),
        r"\1=<REDACTED-AWS-SECRET>",
    ),
    # OpenAI / Anthropic style API keys (sk-..., sk-ant-...).
    (
        "openai_key",
        _compile(r"\bsk-(ant-)?[A-Za-z0-9_\-]{16,}\b"),
        "<REDACTED-API-KEY>",
    ),
    # Stripe live + test keys.
    (
        "stripe_key",
        _compile(r"\b(sk|rk|pk)_(live|test)_[A-Za-z0-9]{16,}\b"),
        "<REDACTED-STRIPE-KEY>",
    ),
    # GitHub personal access tokens (classic + fine-grained).
    (
        "github_pat",
        _compile(r"\bghp_[A-Za-z0-9]{36}\b|\bgithub_pat_[A-Za-z0-9_]{50,}\b"),
        "<REDACTED-GITHUB-PAT>",
    ),
    # GCP service-account-style JSON keys (private_key field).
    (
        "gcp_private_key",
        _compile(
            r'"private_key"\s*:\s*"-----BEGIN [A-Z ]+ PRIVATE KEY-----[^"]+"',
        ),
        '"private_key": "<REDACTED-GCP-PRIVATE-KEY>"',
    ),
    # Generic ``X-API-Key: ...`` / ``X-Auth-Token: ...`` header style.
    (
        "x_api_key_header",
        _compile(
            r"(?im)^(\s*x-(?:api-key|auth-token|access-token))\s*:\s*\S{16,}"
        ),
        r"\1: <REDACTED-HEADER>",
    ),
    # Catch-all for env-style secrets : ``FOO_TOKEN=value`` /
    # ``BAR_API_KEY=value`` / ``BAZ_SECRET=value``. Match common
    # suffixes only, on the value half of the assignment.
    (
        "env_kv_secret",
        _compile(
            r"(?i)\b([A-Z][A-Z0-9_]*?(?:TOKEN|API_?KEY|SECRET|PASSWORD|PASSWD))"
            r"\s*[=:]\s*([\"']?)([A-Za-z0-9_\-+/.]{12,})\2"
        ),
        r"\1=<REDACTED>",
    ),
)


# Mutable list — additional patterns registered via add_pattern().
PATTERNS: list[tuple[str, Pattern[str], str]] = list(_BUILTIN_PATTERNS)


def reset_patterns() -> None:
    """Restore the builtin patterns. Useful in test teardown."""
    global PATTERNS
    PATTERNS = list(_BUILTIN_PATTERNS)


def add_pattern(name: str, pattern: str, replacement: str, *, flags: int = 0) -> None:
    """Register a custom redaction pattern.

    The new entry is APPENDED — applied after the builtins. Operators
    who need to override a builtin must :func:`reset_patterns` and
    re-register their replacement.

    Args:
        name: stable identifier surfaced in audit + tests.
        pattern: regex source.
        replacement: replacement string (supports ``\\1``-style backrefs).
        flags: optional ``re`` flags.

    Raises:
        ValueError: if ``name`` clashes with an existing entry.
    """
    if any(n == name for n, _, _ in PATTERNS):
        raise ValueError(f"redaction pattern named {name!r} already exists")
    PATTERNS.append((name, _compile(pattern, flags), replacement))


# ---------------------------------------------------------------------------
#  Core redaction
# ---------------------------------------------------------------------------


def redact(text: str | None) -> str:
    """Apply every registered pattern to ``text`` and return the
    redacted version.

    **Audit fix M8-A4 + M8-A5** — explicit type contract :
      * ``str``   → returns the redacted string (or unchanged if
        no pattern matched).
      * ``None``  → returns ``""`` (logging-friendly fallback).
      * other type → raises ``TypeError`` with a clear message.
        Pre-audit the function silently passed non-str inputs
        (e.g. ``redact(0)`` returned ``0``), masking developer
        bugs that would later crash deeper in the call stack.

    Safety guarantees :
      * the output is **always** safe to log on stderr ;
      * no allocation when no pattern matches against a non-empty
        string (the regex `.sub` returns the same string
        identity, and the for-loop's reassignment is a no-op).
    """
    if text is None:
        return ""
    if not isinstance(text, str):
        raise TypeError(
            f"redact() expects str | None, got {type(text).__name__} "
            f"({text!r:.50}). Cast explicitly upstream — silent "
            f"coercion masks log-injection bugs."
        )
    if text == "":
        return text
    out = text
    for _name, regex, repl in PATTERNS:
        out = regex.sub(repl, out)
    return out


def redact_mapping(obj: Any) -> Any:
    """Recursively :func:`redact` every string leaf of a JSON-like
    object.

    Used by the structured-event emission path : the REPL log
    formatter calls this before serialising to ndjson, so every
    `:debug enable` line is scrubbed regardless of which key
    happens to carry the secret.

    Behaviour :
      * ``str``   → :func:`redact`.
      * ``Mapping`` → new ``dict`` with each value recursively
        redacted (keys are left unchanged — leaking a key name
        like ``api_key`` is acceptable, leaking the value is not).
      * ``list`` / ``tuple`` → new ``list`` / ``tuple`` with each
        element recursively redacted.
      * Any other type → returned as-is.
    """
    if isinstance(obj, str):
        return redact(obj)
    if isinstance(obj, ABCMapping):
        return {k: redact_mapping(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [redact_mapping(v) for v in obj]
    if isinstance(obj, tuple):
        return tuple(redact_mapping(v) for v in obj)
    return obj


# ---------------------------------------------------------------------------
#  Test introspection
# ---------------------------------------------------------------------------


def builtin_pattern_names() -> tuple[str, ...]:
    """Stable tuple of builtin pattern names (anti-regression).

    Pin tested : adding/removing a builtin requires a deliberate
    test update, surfaced in the diff.
    """
    return tuple(name for name, _, _ in _BUILTIN_PATTERNS)


__all__ = [
    "add_pattern",
    "builtin_pattern_names",
    "PATTERNS",
    "redact",
    "redact_mapping",
    "reset_patterns",
]
