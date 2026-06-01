"""Pin tests : prompt template loader + parity vs mcp-server version."""
from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

from efe_byo_agent.prompts import (
    PromptTemplateError,
    clear_cache,
    list_templates,
    load,
    render,
)


@pytest.fixture(autouse=True)
def _clear_cache():
    clear_cache()
    yield
    clear_cache()


def test_list_5_templates():
    names = set(list_templates())
    assert names == {
        "summarize_classification_v1",
        "compare_bundles_quality_v1",
        "generate_test_sentences_v1",
        "analyze_confusion_matrix_v1",
        "draft_runbook_v1",
    }


def test_load_yields_eval_pass_true():
    for name in list_templates():
        t = load(name)
        assert t.eval_pass is True


def test_load_unknown_raises():
    with pytest.raises(PromptTemplateError):
        load("nonexistent")


def test_render_substitutes_variables():
    sys, usr = render(
        "summarize_classification_v1",
        catalog="x", sentence="hi",
        classification_result_json="{}",
        class_definitions_json="{}",
        language="English",
    )
    assert "x" in sys or "x" in usr
    assert "hi" in usr


def test_all_provider_compat_includes_anthropic():
    for name in list_templates():
        t = load(name)
        assert "anthropic" in t.provider_compat


def test_parity_with_mcp_server_templates():
    """Ensure efe-byo-agent prompt files are byte-identical to the
    mcp-server side. Drift between the two would cause
    helper-via-direct-mode and helper-via-mcp-sampling to behave
    differently — a major hazard."""
    mcp_dir = (
        Path(__file__).parents[3]
        / "mcp-server" / "mcp_server" / "tools" / "m20_prompts"
    )
    if not mcp_dir.exists():
        pytest.skip(f"mcp-server templates not at {mcp_dir}")
    agent_dir = (
        Path(__file__).parents[1] / "efe_byo_agent" / "prompts"
    )
    for tpl in mcp_dir.glob("*.md"):
        other = agent_dir / tpl.name
        assert other.exists(), f"Missing in efe-byo-agent : {tpl.name}"
        h1 = hashlib.sha256(tpl.read_bytes()).hexdigest()
        h2 = hashlib.sha256(other.read_bytes()).hexdigest()
        assert h1 == h2, (
            f"Prompt drift on {tpl.name} : "
            f"mcp={h1[:8]} != agent={h2[:8]}"
        )
