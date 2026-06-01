"""Pin tests for tool input schemas (M16).

Validates that the 6 tools expose well-formed JSON Schema 2020-12 with
`additionalProperties: false` (so unexpected fields are rejected).
"""
from __future__ import annotations

from jsonschema import Draft202012Validator

EXPECTED_TOOLS = {
    "efe_classify",
    "efe_list_catalogs",
    "efe_get_audit_log",
    "efe_get_admin_quotas",
    "efe_register_catalog",
    "efe_evolve_catalog",
}


def test_registry_contains_all_six_tools(registry):
    assert set(registry.keys()) == EXPECTED_TOOLS


def test_every_tool_schema_is_valid_json_schema_2020_12(registry):
    for name, tool in registry.items():
        # Will raise jsonschema.SchemaError on a bad meta-schema.
        Draft202012Validator.check_schema(tool.input_schema)


def test_every_tool_schema_disallows_additional_properties(registry):
    for name, tool in registry.items():
        assert tool.input_schema.get("additionalProperties") is False, (
            f"tool {name!r} must set additionalProperties:false for "
            "TM-M16-T-02 mitigation (defends against arg-injection attacks)"
        )


def test_write_tools_require_confirm_true(registry):
    """TM-M16-T-01 mitigation : write tools MUST require `confirm: true`."""
    for name in ("efe_register_catalog", "efe_evolve_catalog"):
        tool = registry[name]
        assert tool.is_write is True
        required = tool.input_schema.get("required", [])
        assert "confirm" in required, f"{name} must require confirm in schema"
        confirm_prop = tool.input_schema["properties"]["confirm"]
        assert confirm_prop["type"] == "boolean"


def test_classify_input_validates(registry):
    schema = registry["efe_classify"].input_schema
    validator = Draft202012Validator(schema)
    validator.validate({"catalog": "support", "text": "hi"})
    validator.validate({"catalog": "support", "text": "hi", "top_k": 5})


def test_classify_rejects_unknown_keys(registry):
    schema = registry["efe_classify"].input_schema
    validator = Draft202012Validator(schema)
    errors = list(validator.iter_errors({"catalog": "x", "text": "y", "secret": True}))
    assert errors, "additionalProperties:false should reject unknown 'secret' field"


# ─────────────────────────────────────────────────────────────────────
# Audit fix M17-A4 — extend coverage to all 20 tools (M16 + M17)
# ─────────────────────────────────────────────────────────────────────


def test_every_m17_tool_schema_is_valid_json_schema_2020_12(agent_native_registry):
    for name, tool in agent_native_registry.items():
        Draft202012Validator.check_schema(tool.input_schema)


def test_every_m17_tool_schema_disallows_additional_properties(agent_native_registry):
    """TM-M16-T-02 carry-over : every M17 tool MUST also set
    additionalProperties:false. Caught the gap during the M17 audit
    pass — previous test only iterated profile=core."""
    for name, tool in agent_native_registry.items():
        assert tool.input_schema.get("additionalProperties") is False, (
            f"tool {name!r} must set additionalProperties:false "
            "(TM-M16-T-02 mitigation extended to M17)"
        )


def test_every_sampling_tool_requires_confirm(agent_native_registry):
    """TM-M17-T-01 carry-over : sampling tools MUST require confirm:true
    so the user explicitly authorises the LLM-cost incurr."""
    for name, tool in agent_native_registry.items():
        if not tool.requires_sampling:
            continue
        required = tool.input_schema.get("required", [])
        assert "confirm" in required, (
            f"sampling tool {name!r} must require confirm in schema"
        )


def test_every_m17_tool_schema_has_title_and_description(agent_native_registry):
    for name, tool in agent_native_registry.items():
        schema = tool.input_schema
        assert schema.get("title"), f"{name!r} schema missing title"
        assert schema.get("description"), f"{name!r} schema missing description"
