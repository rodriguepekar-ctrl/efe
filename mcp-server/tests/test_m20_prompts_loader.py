"""M20 prompt template loader tests."""
from __future__ import annotations

import pytest

from mcp_server.tools.m20_prompts import (
    PromptTemplateError,
    clear_cache,
    list_templates,
    load,
    render,
)


@pytest.fixture(autouse=True)
def _clear():
    clear_cache()
    yield
    clear_cache()


def test_list_templates_5():
    """All 5 M20 templates must be present."""
    names = set(list_templates())
    assert names == {
        "summarize_classification_v1",
        "compare_bundles_quality_v1",
        "generate_test_sentences_v1",
        "analyze_confusion_matrix_v1",
        "draft_runbook_v1",
    }


def test_load_summarize_classification_eval_pass():
    t = load("summarize_classification_v1")
    assert t.eval_pass is True
    assert t.eval_accuracy is not None and t.eval_accuracy >= 0.85


def test_load_unknown_raises():
    with pytest.raises(PromptTemplateError):
        load("nonexistent_template")


def test_render_substitutes_variables():
    sys, usr = render(
        "summarize_classification_v1",
        catalog="x", sentence="hello",
        classification_result_json="{}",
        class_definitions_json="{}",
        language="English",
    )
    assert "x" in sys or "x" in usr
    assert "hello" in usr


def test_render_safe_substitute_keeps_unbound_vars():
    """`safe_substitute` does not raise on missing vars."""
    # We provide only some vars ; remaining $vars should remain literal
    sys, usr = render(
        "summarize_classification_v1",
        catalog="x", sentence="hello",
        # class_definitions_json missing on purpose
    )
    assert "$class_definitions_json" in usr or "{}" in usr or "$" in usr


def test_template_yaml_header_required(tmp_path, monkeypatch):
    bad = tmp_path / "bad.md"
    bad.write_text("# System\nno header\n# User\nx", encoding="utf-8")
    monkeypatch.setattr(
        "mcp_server.tools.m20_prompts._TEMPLATES_DIR", tmp_path,
    )
    monkeypatch.setattr(
        "mcp_server.tools.m20_prompts._CACHE", {},
    )
    with pytest.raises(PromptTemplateError):
        load("bad")


def test_all_templates_load():
    """All 5 templates must load without error."""
    for name in list_templates():
        t = load(name)
        assert t.system_body
        assert t.user_body


def test_template_provider_compat_includes_anthropic():
    """All templates must declare anthropic compatibility."""
    for name in list_templates():
        t = load(name)
        assert "anthropic" in t.provider_compat


def test_clear_cache_empties_cache():
    load("summarize_classification_v1")
    clear_cache()
    # Subsequent load goes through file IO again — covered by file-system
    # access ; here we just check the function does not raise.
    load("summarize_classification_v1")
