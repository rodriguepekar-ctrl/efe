"""OpenAI / function-calling integration.

Example:

    from openai import OpenAI
    from efe_tool import EfeClient, AnchorCatalog
    from efe_tool.bridges.openai import (
        build_function_spec, run_conversation_with_function,
    )

    client  = EfeClient.from_env()
    catalog = AnchorCatalog.load("catalogs/support_triage.yaml",
                                 embedder=client.embedder)
    openai  = OpenAI()
    catalogs = {catalog.name: catalog}

    transcript = run_conversation_with_function(
        openai,
        efe_client=client,
        catalogs=catalogs,
        model="gpt-4o",
        system="You are a support router.",
        user="My card was charged twice for the same order.",
    )
"""

from __future__ import annotations

import json
from typing import Any, Mapping

from ..catalog import AnchorCatalog
from ..client import EfeClient

FUNCTION_NAME = "efe_classify"


def build_function_spec(catalog_names: list[str]) -> dict[str, Any]:
    """Build the OpenAI tool spec (function-calling format)."""
    if not catalog_names:
        raise ValueError("build_function_spec: at least one catalog name is required")
    return {
        "type": "function",
        "function": {
            "name": FUNCTION_NAME,
            "description": (
                "Classify a user utterance into one of the pre-defined categories "
                "using an ephemeral fluid swarm. Returns { label, confidence, rounds, "
                "alignment_delta, reject }. Treat reject=true or label='uncertain' as "
                "a signal to ask the user a clarifying question."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "text": {"type": "string", "description": "User utterance."},
                    "catalog": {
                        "type": "string",
                        "enum": list(catalog_names),
                    },
                },
                "required": ["text", "catalog"],
                "additionalProperties": False,
            },
        },
    }


def dispatch(
    arguments_json: str,
    *,
    efe_client: EfeClient,
    catalogs: Mapping[str, AnchorCatalog],
) -> str:
    """Parse the function-call arguments and return a JSON string."""
    try:
        args = json.loads(arguments_json)
    except json.JSONDecodeError:
        return json.dumps({"error": "invalid JSON arguments"})
    text = args.get("text")
    cat_name = args.get("catalog")
    if not isinstance(text, str) or not text.strip():
        return json.dumps({"error": "text must be a non-empty string"})
    if cat_name not in catalogs:
        return json.dumps({"error": f"unknown catalog '{cat_name}'"})
    try:
        result = efe_client.classify(text, catalogs[cat_name])
    except Exception as e:
        return json.dumps({"error": f"{type(e).__name__}: {e}"})
    return json.dumps(result.as_tool_result())


def run_conversation_with_function(
    openai_client: Any,
    *,
    efe_client: EfeClient,
    catalogs: Mapping[str, AnchorCatalog],
    model: str,
    system: str,
    user: str,
    max_iter: int = 5,
) -> list[dict[str, Any]]:
    """Run a conversation until the LLM stops requesting tool calls."""
    spec = build_function_spec(list(catalogs.keys()))
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system},
        {"role": "user", "content": user},
    ]

    for _ in range(max_iter):
        resp = openai_client.chat.completions.create(
            model=model,
            messages=messages,
            tools=[spec],
            tool_choice="auto",
        )
        choice = resp.choices[0]
        msg = choice.message
        messages.append(msg.model_dump() if hasattr(msg, "model_dump") else dict(msg))

        if not msg.tool_calls:
            break

        for tc in msg.tool_calls:
            if tc.function.name != FUNCTION_NAME:
                content = json.dumps({"error": f"unknown function '{tc.function.name}'"})
            else:
                content = dispatch(
                    tc.function.arguments,
                    efe_client=efe_client,
                    catalogs=catalogs,
                )
            messages.append({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": content,
            })
    return messages
