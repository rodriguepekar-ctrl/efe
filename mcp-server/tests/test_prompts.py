"""MCP Prompts pin tests (M17 D44)."""
from __future__ import annotations

import pytest

from mcp_server.prompts import PROMPT_DESCRIPTORS, render_prompt


def test_prompts_list_includes_m17_baseline(advanced_dispatcher):
    """M17 baseline must remain ; D48 added 3 more (8 total now).

    This test no longer hard-codes a specific count so milestones
    can extend cleanly. The 5 M17 templates must always be present.
    """
    prompts = advanced_dispatcher.list_prompts()
    names = {p["name"] for p in prompts}
    for required in (
        "classify_with_reasoning",
        "bootstrap_catalog_from_csv",
        "audit_compliance_check",
        "drift_investigation",
        "threshold_calibration_session",
    ):
        assert required in names, f"M17 prompt {required!r} missing"
    assert len(prompts) >= 5


def test_prompts_list_disabled_in_core(dispatcher):
    assert dispatcher.list_prompts() == []


def test_classify_with_reasoning_substitutes_variables():
    text = render_prompt(
        "classify_with_reasoning",
        {"catalog": "support", "text": "hi there"},
    )
    assert "support" in text
    assert "hi there" in text


def test_bootstrap_optional_var_skipped_when_missing():
    text = render_prompt(
        "bootstrap_catalog_from_csv",
        {"csv_path": "/tmp/data.csv"},
    )
    assert "/tmp/data.csv" in text
    assert "I'd like roughly" not in text


def test_bootstrap_optional_var_included_when_provided():
    text = render_prompt(
        "bootstrap_catalog_from_csv",
        {"csv_path": "/tmp/data.csv", "target_classes_hint": "5"},
    )
    assert "I'd like roughly 5 classes" in text


def test_classify_with_reasoning_missing_required_raises():
    with pytest.raises(ValueError):
        render_prompt("classify_with_reasoning", {"catalog": "x"})  # missing text


def test_unknown_prompt_raises():
    with pytest.raises(KeyError):
        render_prompt("nonexistent", {})


def test_threshold_calibration_default_optional():
    text = render_prompt(
        "threshold_calibration_session",
        {"catalog": "x"},
    )
    assert "0.9" in text


def test_dispatcher_get_prompt_returns_messages(advanced_dispatcher):
    res = advanced_dispatcher.get_prompt(
        "classify_with_reasoning",
        {"catalog": "support", "text": "hi"},
    )
    assert "messages" in res
    assert res["messages"][0]["role"] == "user"


def test_dispatcher_get_prompt_unknown_returns_error(advanced_dispatcher):
    res = advanced_dispatcher.get_prompt("nonexistent")
    assert "error" in res
    assert res["code"] == "unknown_prompt"


def test_prompt_variable_with_braces_escaped():
    """TM-M17-E-02 mitigation : injected braces stripped."""
    text = render_prompt(
        "classify_with_reasoning",
        {"catalog": "{evil}", "text": "hi"},
    )
    # The injected braces should be stripped, so no '{evil}' literal
    assert "{evil}" not in text
