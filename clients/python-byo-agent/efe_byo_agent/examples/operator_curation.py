"""Example : operator curation flow — confusion analysis + sentence generation."""
from __future__ import annotations

import asyncio
import os

from efe_byo_agent import Agent
from efe_byo_agent.llm import OpenAIProvider


async def main() -> None:
    agent = Agent(
        llm_provider=OpenAIProvider(model="gpt-5-mini"),
        mcp_endpoint=os.environ.get("EFE_MCP_ENDPOINT"),
        oidc_token=os.environ.get("EFE_OIDC_TOKEN"),
    )
    try:
        # 1. Analyse the confusion matrix
        analysis = await agent.helper(
            "analyze_confusion_matrix",
            catalog="support-routing",
            confusion_matrix={
                "billing":      {"billing": 95, "refund": 5},
                "refund":       {"billing": 12, "refund": 88},
                "tech_support": {"tech_support": 82, "billing": 3, "refund": 15},
            },
            min_confusion_rate=0.05,
            max_pairs=5,
        )
        print("Confusion analysis :")
        print(analysis["summary"])
        for action in analysis["proposed_actions"]:
            print(f"  - {action['action']} {action['classes']} : {action['rationale']}")

        # 2. Generate test sentences for under-covered class
        sentences = await agent.helper(
            "generate_test_sentences",
            catalog="support-routing",
            target_class="refund",
            count=20,
            difficulty="mixed",
            language="en",
        )
        print("\nGenerated sentences for class 'refund' :")
        for s in sentences["sentences"][:5]:
            print(f"  - {s}")
        print(f"... ({sentences['metadata']['count_returned']} total)")
    finally:
        await agent.aclose()


if __name__ == "__main__":
    asyncio.run(main())
