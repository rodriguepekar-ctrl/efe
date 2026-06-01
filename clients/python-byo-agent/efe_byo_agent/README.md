# efe-byo-agent — BYO-LLM SDK for EFE M20

**Default state** : NOT installed in the mcp-server image. Operators
do `pip install efe-agent` separately on their workstations / CI / batch
systems.

## Install

```bash
pip install efe-agent                 # base, no provider
pip install efe-agent[anthropic]      # + Anthropic SDK
pip install efe-agent[openai]         # + OpenAI SDK
pip install efe-agent[bedrock]        # + boto3
pip install efe-agent[azure-openai]   # + openai + azure-identity
pip install efe-agent[vertex]         # + google-cloud-aiplatform
pip install efe-agent[mlx]            # + mlx-lm (Mac M-series only)
pip install efe-agent[all]            # everything
```

## Quick start (direct provider mode)

```python
import asyncio
import os
from efe_byo_agent import Agent
from efe_byo_agent.llm import AnthropicProvider

async def main():
    agent = Agent(
        llm_provider=AnthropicProvider(model="claude-sonnet-4-5"),
    )
    try:
        result = await agent.helper(
            "summarize_classification",
            catalog="support-routing",
            sentence="My laptop won't boot after the update",
            classification_result={
                "label": "hardware_issue",
                "confidence": 0.87,
                "anchors_used": ["hardware", "boot", "device"],
            },
        )
        print(result["explanation"])
        print(f"Cost : ${agent.cost_tracker.total_cost_usd:.4f}")
    finally:
        await agent.aclose()

asyncio.run(main())
```

## Available helpers

| Helper | Use case |
|---|---|
| `summarize_classification` | Explain a Mode-1 classify result in natural language |
| `compare_bundles_quality` | Diff two bundles' classify outputs + recommend promote/investigate/do-not-promote |
| `generate_test_sentences` | Generate N synthetic test sentences for a class |
| `analyze_confusion_matrix` | Analyse confusion pairs + propose taxonomy actions |
| `draft_runbook` | Draft a runbook from an SLO breach event |

## Multi-provider with fallback

```python
from efe_byo_agent.llm import (
    AnthropicProvider, OpenAIProvider, FallbackChain,
)

llm = FallbackChain(
    primary=AnthropicProvider(model="claude-sonnet-4-5"),
    fallbacks=[
        OpenAIProvider(model="gpt-5-mini"),
    ],
)
agent = Agent(llm_provider=llm)
```

## CLI

```bash
efe-agent --version
efe-agent helper list
efe-agent providers list

# Run a helper from the command line
efe-agent helper run summarize_classification \
    --json-input ./input.json \
    --provider anthropic \
    --model claude-sonnet-4-5
```

## Documentation

See `docs/EFE_M20_BYO_LLM_AGENT_SDK_ARCHITECTURE.md` in the EFE
monorepo for the full architecture deep-dive.

## License

Apache-2.0.
