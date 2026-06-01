"""Output validation for efe-byo-agent helpers (M20).

Mirrors mcp-server/mcp_server/tools/m20_validation.py — same logic,
duplicated here so the lib has zero mcp-server dependency.

A CI test (`test_validation_parity`) ensures both implementations
produce identical results on a shared fixture set.
"""
from __future__ import annotations

import json
import re
from typing import Any, Awaitable, Callable, Literal

from .exceptions import ValidationFailedError


_FENCE_RE = re.compile(r"^```(?:json)?\s*(.*?)\s*```\s*$", re.DOTALL)


def _strip_fences(raw: str) -> str:
    s = raw.strip()
    m = _FENCE_RE.match(s)
    return m.group(1).strip() if m else s


def _ensure_str(v: Any, *, field: str, min_len: int, max_len: int) -> str:
    if not isinstance(v, str):
        raise ValueError(f"{field} must be a string")
    if not (min_len <= len(v) <= max_len):
        raise ValueError(
            f"{field} length must be in [{min_len}, {max_len}], got {len(v)}"
        )
    return v


def _ensure_list(v: Any, *, field: str, min_len: int, max_len: int, item_kind: type) -> list:
    if not isinstance(v, list):
        raise ValueError(f"{field} must be a list")
    if not (min_len <= len(v) <= max_len):
        raise ValueError(
            f"{field} length must be in [{min_len}, {max_len}], got {len(v)}"
        )
    for i, x in enumerate(v):
        if not isinstance(x, item_kind):
            raise ValueError(f"{field}[{i}] must be {item_kind.__name__}")
    return v


def _ensure_enum(v: Any, *, field: str, options: tuple[str, ...]) -> str:
    if not isinstance(v, str) or v not in options:
        raise ValueError(f"{field} must be one of {options}, got {v!r}")
    return v


def _validate_summarize_classification(p: Any) -> dict:
    if not isinstance(p, dict):
        raise ValueError("output must be a JSON object")
    return {
        "explanation": _ensure_str(p.get("explanation"), field="explanation", min_len=20, max_len=1000),
        "key_factors": _ensure_list(p.get("key_factors"), field="key_factors", min_len=1, max_len=5, item_kind=str),
        "confidence_assessment": _ensure_enum(
            p.get("confidence_assessment"), field="confidence_assessment",
            options=("high", "medium", "low"),
        ),
        "suggested_alternatives": _ensure_list(
            p.get("suggested_alternatives", []),
            field="suggested_alternatives",
            min_len=0, max_len=3, item_kind=str,
        ),
    }


def _validate_compare_bundles_quality(p: Any) -> dict:
    if not isinstance(p, dict):
        raise ValueError("output must be a JSON object")
    delta = p.get("accuracy_delta_pp")
    if not isinstance(delta, (int, float)):
        raise ValueError("accuracy_delta_pp must be a number")
    if not (-100.0 <= float(delta) <= 100.0):
        raise ValueError("accuracy_delta_pp out of [-100, 100]")
    return {
        "accuracy_delta_pp": float(delta),
        "divergence_examples": _ensure_list(
            p.get("divergence_examples", []),
            field="divergence_examples",
            min_len=0, max_len=20, item_kind=dict,
        ),
        "summary": _ensure_str(p.get("summary"), field="summary", min_len=20, max_len=500),
        "recommendation": _ensure_enum(
            p.get("recommendation"), field="recommendation",
            options=("promote", "investigate", "do_not_promote"),
        ),
    }


def _validate_generate_test_sentences(p: Any, *, expected_count: int) -> dict:
    if not isinstance(p, dict):
        raise ValueError("output must be a JSON object")
    sentences = _ensure_list(
        p.get("sentences"), field="sentences",
        min_len=1, max_len=64, item_kind=str,
    )
    if len(sentences) != expected_count:
        raise ValueError(f"expected {expected_count} sentences, got {len(sentences)}")
    for i, s in enumerate(sentences):
        if not (1 <= len(s) <= 256):
            raise ValueError(f"sentences[{i}] length out of [1, 256]")
    diff = p.get("difficulty_self_assessment", {})
    if not isinstance(diff, dict):
        raise ValueError("difficulty_self_assessment must be a dict")
    valid = {"easy", "medium", "hard", "edge"}
    for k, v in diff.items():
        if v not in valid:
            raise ValueError(f"difficulty_self_assessment[{k!r}] = {v!r} invalid")
    return {
        "sentences": sentences,
        "difficulty_self_assessment": diff,
        "rationale": _ensure_str(
            p.get("rationale"), field="rationale", min_len=20, max_len=500,
        ),
    }


def _validate_analyze_confusion_matrix(p: Any) -> dict:
    if not isinstance(p, dict):
        raise ValueError("output must be a JSON object")
    valid_actions = {"split", "merge", "reword", "investigate"}
    actions = _ensure_list(
        p.get("proposed_actions"), field="proposed_actions",
        min_len=0, max_len=10, item_kind=dict,
    )
    for i, a in enumerate(actions):
        if a.get("action") not in valid_actions:
            raise ValueError(
                f"proposed_actions[{i}].action must be in {valid_actions}"
            )
    return {
        "top_confusion_pairs": _ensure_list(
            p.get("top_confusion_pairs"), field="top_confusion_pairs",
            min_len=0, max_len=10, item_kind=dict,
        ),
        "proposed_actions": actions,
        "summary": _ensure_str(p.get("summary"), field="summary", min_len=20, max_len=1000),
    }


def _validate_draft_runbook(p: Any) -> dict:
    if not isinstance(p, dict):
        raise ValueError("output must be a JSON object")
    eta = p.get("estimated_resolution_time_min")
    if not isinstance(eta, int) or not (1 <= eta <= 240):
        raise ValueError("estimated_resolution_time_min must be int [1, 240]")
    return {
        "runbook_markdown": _ensure_str(
            p.get("runbook_markdown"), field="runbook_markdown",
            min_len=200, max_len=20000,
        ),
        "estimated_resolution_time_min": eta,
        "primary_diagnostics": _ensure_list(
            p.get("primary_diagnostics"), field="primary_diagnostics",
            min_len=1, max_len=10, item_kind=str,
        ),
        "risk_level": _ensure_enum(
            p.get("risk_level"), field="risk_level",
            options=("low", "medium", "high"),
        ),
    }


HelperName = Literal[
    "summarize_classification",
    "compare_bundles_quality",
    "generate_test_sentences",
    "analyze_confusion_matrix",
    "draft_runbook",
]

_VALIDATORS: dict[str, Callable[..., dict]] = {
    "summarize_classification": _validate_summarize_classification,
    "compare_bundles_quality": _validate_compare_bundles_quality,
    "generate_test_sentences": _validate_generate_test_sentences,
    "analyze_confusion_matrix": _validate_analyze_confusion_matrix,
    "draft_runbook": _validate_draft_runbook,
}


async def validate_with_retry(
    *,
    helper: HelperName,
    raw_output: str,
    retry_callback: Callable[[list[str]], Awaitable[str]] | None = None,
    max_retries: int = 2,
    **validator_kwargs: Any,
) -> dict:
    if helper not in _VALIDATORS:
        raise ValueError(f"unknown helper {helper!r}")
    validator = _VALIDATORS[helper]
    errors: list[str] = []
    output = raw_output
    for attempt in range(max_retries + 1):
        try:
            stripped = _strip_fences(output)
            parsed = json.loads(stripped)
            return validator(parsed, **validator_kwargs)
        except json.JSONDecodeError as e:
            errors.append(f"invalid JSON ({e.msg} line {e.lineno})")
        except (ValueError, KeyError, TypeError) as e:
            errors.append(f"schema violation : {e}")
        if attempt < max_retries and retry_callback is not None:
            try:
                output = await retry_callback(errors)
            except Exception as cb:  # noqa: BLE001
                errors.append(f"retry_callback error : {cb}")
                break
        else:
            break
    raise ValidationFailedError(
        f"validation failed after {len(errors)} attempt(s)",
        helper=helper,
        attempts=len(errors),
        errors=errors,
    )


__all__ = ["validate_with_retry"]
