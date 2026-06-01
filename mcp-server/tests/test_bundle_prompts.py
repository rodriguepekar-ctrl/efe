"""M18 D48 prompt template pin tests."""
from __future__ import annotations

import pytest

from mcp_server.prompts import PROMPT_DESCRIPTORS, render_prompt


def test_prompts_list_includes_3_d48_templates():
    names = {p.name for p in PROMPT_DESCRIPTORS}
    assert "verify_bundle_supply_chain" in names
    assert "audit_bundle_provenance" in names
    assert "weekly_bundle_health_report" in names


def test_prompts_list_total_count_at_least_8(agent_native_dispatcher):
    """M17 shipped 5 prompts ; D48 added 3 → ≥ 8.

    D49 added 2 more (10 total). Test asserts the M17+D48 baseline so
    future milestones can extend without breaking this.
    """
    prompts = agent_native_dispatcher.list_prompts()
    assert len(prompts) >= 8


# ─────────────────────────────────────────────────────────────────────
# verify_bundle_supply_chain
# ─────────────────────────────────────────────────────────────────────


def test_verify_bundle_supply_chain_substitutes_slug_version():
    text = render_prompt(
        "verify_bundle_supply_chain",
        {"slug": "support-routing", "version": "v3.2.0"},
    )
    assert "support-routing:v3.2.0" in text or (
        "support-routing" in text and "v3.2.0" in text
    )
    assert "efe_inspect_bundle" in text
    assert "efe_verify_bundle_signature" in text


def test_verify_bundle_supply_chain_missing_required_arg_raises():
    with pytest.raises(ValueError):
        render_prompt(
            "verify_bundle_supply_chain", {"slug": "x"},  # missing version
        )


def test_verify_bundle_supply_chain_references_slsa():
    text = render_prompt(
        "verify_bundle_supply_chain",
        {"slug": "x", "version": "v1.0.0"},
    )
    assert "slsa_provenance" in text.lower() or "slsa" in text.lower()


# ─────────────────────────────────────────────────────────────────────
# audit_bundle_provenance
# ─────────────────────────────────────────────────────────────────────


def test_audit_bundle_provenance_no_args_uses_default_scope():
    text = render_prompt("audit_bundle_provenance", {})
    # Default scope text is in the prompt
    assert "every bundle currently registered" in text


def test_audit_bundle_provenance_with_slug_only():
    text = render_prompt("audit_bundle_provenance", {"slug": "support-routing"})
    assert "support-routing" in text


def test_audit_bundle_provenance_with_slug_and_version():
    text = render_prompt(
        "audit_bundle_provenance",
        {"slug": "x", "version": "v1.0.0"},
    )
    assert "x" in text
    assert "v1.0.0" in text


def test_audit_bundle_provenance_args_optional():
    """Both `slug` and `version` are non-required."""
    descr = next(p for p in PROMPT_DESCRIPTORS if p.name == "audit_bundle_provenance")
    for arg in descr.arguments:
        assert arg.required is False


# ─────────────────────────────────────────────────────────────────────
# weekly_bundle_health_report
# ─────────────────────────────────────────────────────────────────────


def test_weekly_bundle_health_report_no_args():
    text = render_prompt("weekly_bundle_health_report", {})
    assert "weekly bundle health report" in text.lower()
    assert "efe_inspect_bundle" in text


def test_weekly_bundle_health_report_references_severity_levels():
    text = render_prompt("weekly_bundle_health_report", {})
    for sev in ("HIGH", "CRITICAL"):
        assert sev in text


def test_weekly_bundle_health_report_no_required_args():
    descr = next(
        p for p in PROMPT_DESCRIPTORS
        if p.name == "weekly_bundle_health_report"
    )
    assert descr.arguments == ()


# ─────────────────────────────────────────────────────────────────────
# Anti-injection — TM-M17-E-02 carry-over for D48 templates
# ─────────────────────────────────────────────────────────────────────


def test_d48_prompts_escape_brace_injection_in_args():
    """Variable values containing `{evil}` get stripped (no
    template re-evaluation)."""
    text = render_prompt(
        "verify_bundle_supply_chain",
        {"slug": "{injected}", "version": "v1.0.0"},
    )
    # Braces in the user value are stripped → literal `injected` may
    # appear but no `{injected}` template directive.
    assert "{injected}" not in text


def test_dispatcher_get_prompt_for_d48_templates(agent_native_dispatcher):
    res = agent_native_dispatcher.get_prompt(
        "verify_bundle_supply_chain",
        {"slug": "x", "version": "v1.0.0"},
    )
    assert "messages" in res
    assert res["messages"][0]["role"] == "user"
