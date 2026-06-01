"""Example : batch comparison of two bundles using `efe-agent` direct mode.

Run :
    pip install efe-agent[anthropic]
    export ANTHROPIC_API_KEY=...
    export EFE_MCP_ENDPOINT=http://efe-mcp.example.com:8800
    export EFE_OIDC_TOKEN=...
    python -m efe_byo_agent.examples.batch_classify
"""
from __future__ import annotations

import asyncio
import os
import sys

from efe_byo_agent import Agent
from efe_byo_agent.llm import AnthropicProvider


async def main() -> int:
    provider = AnthropicProvider(model="claude-sonnet-4-5")

    agent = Agent(
        llm_provider=provider,
        mcp_endpoint=os.environ["EFE_MCP_ENDPOINT"],
        oidc_token=os.environ["EFE_OIDC_TOKEN"],
    )

    try:
        result = await agent.helper(
            "compare_bundles_quality",
            catalog="support-routing",
            version_a="v1.2.0",
            version_b="v1.3.0",
            sample_size=200,
        )
    finally:
        await agent.aclose()

    print(f"Compared {result['n_sentences_compared']} sentences")
    print(f"Divergent : {result['n_divergent']}")
    print(f"Accuracy delta : {result['accuracy_delta_pp']:+.2f} pp")
    print(f"Recommendation : {result['recommendation']}")
    print()
    print(f"Summary : {result['summary']}")

    if result["recommendation"] != "promote":
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
