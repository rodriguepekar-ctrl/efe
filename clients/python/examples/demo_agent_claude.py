"""Example: Claude agent using EFE as a classification tool.

Requires:
    pip install anthropic
    export ANTHROPIC_API_KEY=sk-...
    export EFE_ENDPOINT=127.0.0.1:50051

Usage:
    python examples/demo_agent_claude.py
"""

from __future__ import annotations

import os
import sys
from pathlib import Path


SYSTEM_PROMPT = """\
You are a customer-support router. For every user message you receive,
use the `efe_classify` tool with catalog='support_triage' to decide
the intent, then ACK to the user with a short, warm message that
matches the label.

Interpretation of the tool result:
- If `reject` is true OR `confidence` < 0.62 : ask a clarifying
  question instead of picking a route.
- If 0.62 ≤ `confidence` < 0.82 : mention the uncertainty and offer
  to escalate to a human.
- Otherwise : proceed.
"""


def main() -> int:
    try:
        from anthropic import Anthropic
    except ImportError:
        print("install anthropic: pip install anthropic", file=sys.stderr)
        return 2

    from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder
    from efe_tool.bridges.claude import run_conversation_with_tool

    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        print("set ANTHROPIC_API_KEY", file=sys.stderr)
        return 2

    catalog_path = Path(__file__).resolve().parents[3] / "catalogs" / "support_triage.yaml"
    catalog = AnchorCatalog.load(catalog_path, embedder=LocalSTEmbedder())
    os.environ.setdefault("EFE_ENDPOINT", "127.0.0.1:50051")
    client = EfeClient.from_env()
    anthropic = Anthropic(api_key=api_key)

    user_msg = (
        "My credit card was charged 149€ at 3am by a merchant I never heard of. "
        "Can you help me block this?"
    )
    transcript = run_conversation_with_tool(
        anthropic,
        efe_client=client,
        catalogs={catalog.name: catalog},
        model="claude-sonnet-4-6",
        system=SYSTEM_PROMPT,
        user=user_msg,
    )

    # Print the conversation. The Anthropic SDK returns content blocks as
    # *objects* (not dicts) for assistant turns we received from the API,
    # but the tool_result blocks we inject ourselves are dicts. We handle
    # both forms uniformly.
    def _get(block: object, field: str) -> object:
        if isinstance(block, dict):
            return block.get(field)
        return getattr(block, field, None)

    for msg in transcript:
        role = msg["role"]
        content = msg["content"]
        if isinstance(content, str):
            print(f"\n[{role}] {content}")
            continue
        for block in content:
            kind = _get(block, "type") or "text"
            if kind == "text":
                print(f"\n[{role}] {_get(block, 'text') or ''}")
            elif kind == "tool_use":
                print(
                    f"\n[{role}] tool_use {_get(block, 'name')} → "
                    f"{_get(block, 'input')}"
                )
            elif kind == "tool_result":
                print(f"\n[tool] {_get(block, 'content')}")
            else:
                # Future-proof: unknown block types are surfaced rather
                # than silently dropped.
                print(f"\n[{role}] <{kind}> {block!r}")
    client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
