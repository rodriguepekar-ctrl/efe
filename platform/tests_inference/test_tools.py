"""ToolAdapter + ToolValidator + JsonRepair."""

from __future__ import annotations

import pytest

from efe_inference.errors import ToolArgumentsInvalid, ToolNotFound
from efe_inference.messages.request import ToolDef
from efe_inference.tools.adapter import ToolAdapter
from efe_inference.tools.validator import JsonRepair, ToolValidator


def _weather_tool() -> ToolDef:
    return ToolDef(
        name="get_weather",
        description="Get weather by city.",
        parameters_json_schema={
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
        },
    )


def test_openai_roundtrip_identity():
    tool = _weather_tool()
    out = ToolAdapter.openai_to_canonical_def(
        ToolAdapter.canonical_to_openai_def(tool),
    )
    assert out == tool


def test_anthropic_roundtrip_identity():
    tool = _weather_tool()
    out = ToolAdapter.anthropic_to_canonical_def(
        ToolAdapter.canonical_to_anthropic_def(tool),
    )
    assert out == tool


def test_cross_dialect_roundtrip_openai_anthropic():
    tool = _weather_tool()
    out = ToolAdapter.round_trip_openai_anthropic(tool)
    assert out == tool


def test_canonical_call_to_openai_and_back():
    from efe_inference.messages.response import ToolCall

    call = ToolCall(id="x1", name="get_weather", arguments={"city": "Paris"})
    wire = ToolAdapter.canonical_to_openai_call(call)
    back = ToolAdapter.openai_to_canonical_call(wire)
    assert back == call


def test_canonical_call_to_anthropic_and_back():
    from efe_inference.messages.response import ToolCall

    call = ToolCall(id="x1", name="get_weather", arguments={"city": "Paris"})
    wire = ToolAdapter.canonical_to_anthropic_call(call)
    back = ToolAdapter.anthropic_to_canonical_call(wire)
    assert back == call


def test_json_repair_trailing_comma():
    out, repaired = JsonRepair.try_parse('{"a": 1,}')
    assert out == {"a": 1}
    assert repaired


def test_json_repair_fence():
    out, repaired = JsonRepair.try_parse("```json\n{\"x\": 1}\n```")
    assert out == {"x": 1}
    assert repaired


def test_json_repair_gives_up_on_garbage():
    with pytest.raises(ToolArgumentsInvalid):
        JsonRepair.try_parse("{this is not json!!")


def test_tool_validator_happy_path():
    validator = ToolValidator((_weather_tool(),))
    call, result = validator.parse_and_validate(
        "get_weather", '{"city": "Paris"}',
    )
    assert call.name == "get_weather"
    assert call.arguments == {"city": "Paris"}
    assert result.ok


def test_tool_validator_unknown_tool():
    validator = ToolValidator(())
    with pytest.raises(ToolNotFound):
        validator.parse_and_validate("ghost", "{}")


def test_tool_validator_missing_required():
    validator = ToolValidator((_weather_tool(),))
    with pytest.raises(ToolArgumentsInvalid):
        validator.parse_and_validate("get_weather", "{}")
