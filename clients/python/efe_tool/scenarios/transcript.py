"""Versioned transcript schema for Letter-C runs.

A transcript is the full JSON record of one :func:`run_conversation`
invocation: prompt metadata, boot config, messages, tool calls, stats.
It is the *audit artefact* of a scenario run.

See ``LETTER_C_ARCHITECTURE.md`` §14.4 for the canonical shape.

The schema is versioned under ``schema_version`` independently of the
``efe-tool`` package.  A breaking change increments the integer and
requires documentation in ``CHANGELOG.md``.
"""

from __future__ import annotations

import json
import os
import time
from pathlib import Path
from typing import Any, Mapping, Optional


TRANSCRIPT_SCHEMA_VERSION: int = 1


# ---------------------------------------------------------------------------
#  Builders
# ---------------------------------------------------------------------------


def build_transcript(
    *,
    result: Any,                         # ConversationResult
    scenario: Optional[Any] = None,      # Scenario | None
    boot_config: Optional[Mapping[str, Any]] = None,
    started_at: Optional[float] = None,
    ended_at: Optional[float] = None,
    redact_user: bool = False,
) -> dict[str, Any]:
    """Build the JSON-serialisable transcript dict.

    Args:
        result: :class:`efe_tool.bridges.claude.ConversationResult`.
        scenario: :class:`efe_tool.scenarios.Scenario` if applicable.
        boot_config: The resolved boot configuration (arbitrary dict).
        started_at: POSIX timestamp of the run start.
        ended_at: POSIX timestamp of the run end. Defaults to now.
        redact_user: If True, the user's raw text is replaced by a
            length-and-hash placeholder in the ``messages`` array.

    The result contains stable keys for downstream consumers to parse.
    """
    now = time.time()
    started = started_at if started_at is not None else now
    ended = ended_at if ended_at is not None else now

    messages = _project_messages(
        getattr(result, "transcript", []) or [],
        redact_user=redact_user,
    )
    stats = _as_dict(getattr(result, "stats", None))
    tool_calls = [_as_dict(c) for c in getattr(result, "tool_calls", []) or []]

    scenario_section: dict[str, Any] = {}
    if scenario is not None:
        prompt = getattr(scenario, "prompt", None)
        scenario_section = {
            "name": getattr(scenario, "name", None),
            "version": getattr(prompt, "version", None),
            "prompt_hash": getattr(prompt, "hash", None),
            "model": getattr(prompt, "model", None),
            "catalogs": list(getattr(prompt, "catalog_names", ()) or ()),
        }

    return {
        "schema_version": TRANSCRIPT_SCHEMA_VERSION,
        "scenario": scenario_section,
        "boot_config": dict(boot_config or {}),
        "started_at": _iso(started),
        "ended_at": _iso(ended),
        "wall_clock_ms": int(getattr(result, "wall_clock_ms", 0) or 0),
        "model": str(getattr(result, "model", "") or ""),
        "final_text": str(getattr(result, "final_text", "") or ""),
        "stop_reason": str(getattr(result, "stop_reason", "") or ""),
        "messages": messages,
        "tool_calls": tool_calls,
        "stats": stats,
    }


def write_transcript(transcript: Mapping[str, Any], path: str | Path) -> None:
    """Write ``transcript`` as JSON to ``path`` atomically."""
    p = Path(os.path.expanduser(str(path)))
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_suffix(p.suffix + ".tmp")
    with tmp.open("w", encoding="utf-8") as f:
        json.dump(transcript, f, ensure_ascii=False, indent=2)
    tmp.replace(p)


# ---------------------------------------------------------------------------
#  Internals
# ---------------------------------------------------------------------------


def _iso(t: float) -> str:
    gm = time.gmtime(t)
    return (
        f"{gm.tm_year:04d}-{gm.tm_mon:02d}-{gm.tm_mday:02d}"
        f"T{gm.tm_hour:02d}:{gm.tm_min:02d}:{gm.tm_sec:02d}Z"
    )


def _as_dict(obj: Any) -> Any:
    if obj is None:
        return None
    if hasattr(obj, "as_dict") and callable(obj.as_dict):
        try:
            return obj.as_dict()
        except Exception:  # pragma: no cover
            pass
    if isinstance(obj, Mapping):
        return dict(obj)
    # Best effort: let JSON default() surface any weirdness.
    return obj


def _project_messages(
    messages: list[dict[str, Any]],
    *,
    redact_user: bool,
) -> list[dict[str, Any]]:
    """Reduce the raw transcript to a JSON-safe list.

    Anthropic assistant content contains SDK objects (text/tool_use blocks
    with attributes). We normalise them to ``{type, ...}`` dicts so the
    whole thing serialises with ``json.dumps``.
    """
    out: list[dict[str, Any]] = []
    for msg in messages:
        role = msg.get("role") or "user"
        content = msg.get("content")
        if isinstance(content, str):
            text = (
                _redacted_marker(content)
                if redact_user and role == "user"
                else content
            )
            out.append({"role": role, "content": text})
            continue
        if not isinstance(content, list):
            out.append({"role": role, "content": str(content)})
            continue
        blocks: list[dict[str, Any]] = []
        for b in content:
            blocks.append(_project_block(b, redact=redact_user and role == "user"))
        out.append({"role": role, "content": blocks})
    return out


def _project_block(block: Any, *, redact: bool) -> dict[str, Any]:
    btype = _get(block, "type") or "unknown"
    if btype == "text":
        text = _get(block, "text") or ""
        if redact:
            text = _redacted_marker(text)
        return {"type": "text", "text": text}
    if btype == "tool_use":
        return {
            "type": "tool_use",
            "id": _get(block, "id"),
            "name": _get(block, "name"),
            "input": _get(block, "input") or {},
        }
    if btype == "tool_result":
        return {
            "type": "tool_result",
            "tool_use_id": _get(block, "tool_use_id"),
            "is_error": bool(_get(block, "is_error")),
            "content": _get(block, "content"),
        }
    # Unknown block — best effort.
    return {"type": btype, "raw": repr(block)}


def _get(obj: Any, name: str) -> Any:
    if isinstance(obj, Mapping):
        return obj.get(name)
    return getattr(obj, name, None)


def _redacted_marker(text: str) -> str:
    import hashlib
    n = len(text)
    if n == 0:
        return "<<redacted: empty>>"
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()[:12]
    return f"<<redacted: len={n} hash={digest}>>"


__all__ = [
    "TRANSCRIPT_SCHEMA_VERSION",
    "build_transcript",
    "write_transcript",
]
