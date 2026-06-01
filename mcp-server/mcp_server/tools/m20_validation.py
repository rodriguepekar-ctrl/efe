"""Output validation for M20 LLM helpers (M20 D55).

LLM outputs are inherently non-deterministic. M20 tools enforce a
strict per-helper schema via JSON Schema 2020-12 + Pydantic-like
field constraints. On validation failure, a retry loop is triggered
with the validation errors echoed back to the LLM.

Pattern :
  1. Strip Markdown fences (`json ... `).
  2. Try `json.loads`.
  3. Dispatch to per-helper validator.
  4. On failure, retry up to `max_retries` (default 2) by re-issuing
     the sampling request with the validation errors in the system
     prompt.
  5. Final failure → ToolError(code="validation_failed").

Carry-over from M17 sampling validation pattern + M18 schema-level
enforcement.

Pin tests : tests/test_m20_validation.py + per-tool integration tests.
"""
from __future__ import annotations

import json
import re
from typing import Any, Awaitable, Callable, Literal

from .types import ToolError


# ─────────────────────────────────────────────────────────────────────
# Strict per-helper validators
# ─────────────────────────────────────────────────────────────────────


_FENCE_RE = re.compile(r"^```(?:json)?\s*(.*?)\s*```\s*$", re.DOTALL)


def _strip_fences(raw: str) -> str:
    """Remove ```json ... ``` if present, otherwise return stripped."""
    s = raw.strip()
    m = _FENCE_RE.match(s)
    return m.group(1).strip() if m else s


def _ensure_str(value: Any, *, field: str, min_len: int, max_len: int) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{field!r} must be a string, got {type(value).__name__}")
    if not (min_len <= len(value) <= max_len):
        raise ValueError(
            f"{field!r} length must be in [{min_len}, {max_len}], got {len(value)}"
        )
    return value


def _ensure_list(
    value: Any,
    *,
    field: str,
    min_len: int = 0,
    max_len: int,
    item_kind: type = str,
) -> list:
    if not isinstance(value, list):
        raise ValueError(f"{field!r} must be a list, got {type(value).__name__}")
    if not (min_len <= len(value) <= max_len):
        raise ValueError(
            f"{field!r} length must be in [{min_len}, {max_len}], got {len(value)}"
        )
    for i, item in enumerate(value):
        if not isinstance(item, item_kind):
            raise ValueError(
                f"{field}[{i}] must be {item_kind.__name__}, "
                f"got {type(item).__name__}"
            )
    return value


def _ensure_enum(value: Any, *, field: str, options: tuple[str, ...]) -> str:
    if not isinstance(value, str) or value not in options:
        raise ValueError(
            f"{field!r} must be one of {options}, got {value!r}"
        )
    return value


# ─────────────────────────────────────────────────────────────────────
# Helper-specific validators
# ─────────────────────────────────────────────────────────────────────


def validate_summarize_classification(parsed: Any) -> dict:
    """Validate efe_summarize_classification output."""
    if not isinstance(parsed, dict):
        raise ValueError("output must be a JSON object")
    return {
        "explanation": _ensure_str(
            parsed.get("explanation"), field="explanation",
            min_len=20, max_len=1000,
        ),
        "key_factors": _ensure_list(
            parsed.get("key_factors"), field="key_factors",
            min_len=1, max_len=5, item_kind=str,
        ),
        "confidence_assessment": _ensure_enum(
            parsed.get("confidence_assessment"),
            field="confidence_assessment",
            options=("high", "medium", "low"),
        ),
        "suggested_alternatives": _ensure_list(
            parsed.get("suggested_alternatives", []),
            field="suggested_alternatives",
            min_len=0, max_len=3, item_kind=str,
        ),
    }


def validate_compare_bundles_quality(parsed: Any) -> dict:
    """Validate efe_compare_bundles_quality output."""
    if not isinstance(parsed, dict):
        raise ValueError("output must be a JSON object")
    delta = parsed.get("accuracy_delta_pp")
    if not isinstance(delta, (int, float)):
        raise ValueError("accuracy_delta_pp must be a number")
    if not (-100.0 <= float(delta) <= 100.0):
        raise ValueError("accuracy_delta_pp out of range [-100, 100]")
    return {
        "accuracy_delta_pp": float(delta),
        "divergence_examples": _ensure_list(
            parsed.get("divergence_examples", []),
            field="divergence_examples",
            min_len=0, max_len=20, item_kind=dict,
        ),
        "summary": _ensure_str(
            parsed.get("summary"), field="summary",
            min_len=20, max_len=500,
        ),
        "recommendation": _ensure_enum(
            parsed.get("recommendation"),
            field="recommendation",
            options=("promote", "investigate", "do_not_promote"),
        ),
    }


def validate_generate_test_sentences(parsed: Any, *, expected_count: int) -> dict:
    """Validate efe_generate_test_sentences output."""
    if not isinstance(parsed, dict):
        raise ValueError("output must be a JSON object")
    sentences = _ensure_list(
        parsed.get("sentences"), field="sentences",
        min_len=1, max_len=64, item_kind=str,
    )
    if len(sentences) != expected_count:
        raise ValueError(
            f"sentences length must equal expected_count={expected_count}, "
            f"got {len(sentences)}"
        )
    for i, s in enumerate(sentences):
        if not (1 <= len(s) <= 256):
            raise ValueError(
                f"sentences[{i}] length out of [1, 256], got {len(s)}"
            )
    diff = parsed.get("difficulty_self_assessment", {})
    if not isinstance(diff, dict):
        raise ValueError("difficulty_self_assessment must be an object")
    valid_levels = {"easy", "medium", "hard", "edge"}
    for k, v in diff.items():
        if v not in valid_levels:
            raise ValueError(
                f"difficulty_self_assessment[{k!r}] must be in {valid_levels}, "
                f"got {v!r}"
            )
    return {
        "sentences": sentences,
        "difficulty_self_assessment": diff,
        "rationale": _ensure_str(
            parsed.get("rationale"), field="rationale",
            min_len=20, max_len=500,
        ),
    }


def validate_analyze_confusion_matrix(parsed: Any) -> dict:
    """Validate efe_analyze_confusion_matrix output."""
    if not isinstance(parsed, dict):
        raise ValueError("output must be a JSON object")
    pairs = _ensure_list(
        parsed.get("top_confusion_pairs"), field="top_confusion_pairs",
        min_len=0, max_len=10, item_kind=dict,
    )
    actions = _ensure_list(
        parsed.get("proposed_actions"), field="proposed_actions",
        min_len=0, max_len=10, item_kind=dict,
    )
    valid_action_types = {"split", "merge", "reword", "investigate"}
    for i, action in enumerate(actions):
        action_type = action.get("action")
        if action_type not in valid_action_types:
            raise ValueError(
                f"proposed_actions[{i}].action must be in {valid_action_types}, "
                f"got {action_type!r}"
            )
    return {
        "top_confusion_pairs": pairs,
        "proposed_actions": actions,
        "summary": _ensure_str(
            parsed.get("summary"), field="summary",
            min_len=20, max_len=1000,
        ),
    }


def validate_draft_runbook(parsed: Any) -> dict:
    """Validate efe_draft_runbook output."""
    if not isinstance(parsed, dict):
        raise ValueError("output must be a JSON object")
    eta = parsed.get("estimated_resolution_time_min")
    if not isinstance(eta, int) or not (1 <= eta <= 240):
        raise ValueError("estimated_resolution_time_min must be int in [1, 240]")
    return {
        "runbook_markdown": _ensure_str(
            parsed.get("runbook_markdown"), field="runbook_markdown",
            min_len=200, max_len=20000,
        ),
        "estimated_resolution_time_min": eta,
        "primary_diagnostics": _ensure_list(
            parsed.get("primary_diagnostics"), field="primary_diagnostics",
            min_len=1, max_len=10, item_kind=str,
        ),
        "risk_level": _ensure_enum(
            parsed.get("risk_level"), field="risk_level",
            options=("low", "medium", "high"),
        ),
    }


# ─────────────────────────────────────────────────────────────────────
# Top-level retry loop
# ─────────────────────────────────────────────────────────────────────


HelperName = Literal[
    "summarize_classification",
    "compare_bundles_quality",
    "generate_test_sentences",
    "analyze_confusion_matrix",
    "draft_runbook",
]

_VALIDATORS: dict[str, Callable[..., dict]] = {
    "summarize_classification": validate_summarize_classification,
    "compare_bundles_quality": validate_compare_bundles_quality,
    "generate_test_sentences": validate_generate_test_sentences,
    "analyze_confusion_matrix": validate_analyze_confusion_matrix,
    "draft_runbook": validate_draft_runbook,
}


async def validate_with_retry(
    *,
    helper: HelperName,
    raw_output: str,
    retry_callback: Callable[[list[str]], Awaitable[str]] | None = None,
    max_retries: int = 2,
    **validator_kwargs: Any,
) -> dict:
    """Validate `raw_output` ; on failure, retry up to `max_retries` via
    `retry_callback(errors) -> new_raw_output`.

    Args :
      helper           — which validator to apply.
      raw_output       — the LLM-produced string (potentially wrapped
                         in markdown fences).
      retry_callback   — async function called with validation errors
                         to obtain a regenerated output. If None, no
                         retry is attempted.
      max_retries      — defensive cap on retry loops (default 2).
      validator_kwargs — extra args forwarded to the helper validator
                         (e.g. expected_count for generate_test_sentences).

    Returns the validated dict ready for inclusion in the tool response.

    Raises ToolError(code="validation_failed") if all attempts fail.
    """
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
            err = f"invalid JSON ({e.msg} at line {e.lineno} col {e.colno})"
        except (ValueError, KeyError, TypeError) as e:
            err = f"schema violation : {e}"
        errors.append(err)
        if attempt < max_retries and retry_callback is not None:
            try:
                output = await retry_callback(errors)
            except Exception as cb_err:  # pragma: no cover - defensive
                errors.append(f"retry_callback error : {cb_err}")
                break
        else:
            break

    raise ToolError(
        f"LLM output failed validation after {len(errors)} attempt(s) : "
        f"{'; '.join(errors)}",
        code="validation_failed",
    )


__all__ = [
    "validate_with_retry",
    "validate_summarize_classification",
    "validate_compare_bundles_quality",
    "validate_generate_test_sentences",
    "validate_analyze_confusion_matrix",
    "validate_draft_runbook",
]
