"""efe-agent CLI — minimal command-line interface (M20).

Usage :
    efe-agent helper <name> --json-input <path> [--provider anthropic] [--model ...]
    efe-agent helpers list
    efe-agent providers list

The CLI does NOT install any provider SDK — it relies on whichever
extras are installed in the current Python environment.
"""
from __future__ import annotations

import asyncio
import json
import os
import sys
from pathlib import Path

import click

from .__version__ import __version__
from .agent import Agent
from .exceptions import EfeAgentError
from .helpers import HELPERS


@click.group()
@click.version_option(__version__)
def main() -> None:
    """efe-agent — BYO-LLM SDK for EFE M20 helpers."""


@main.group()
def helper() -> None:
    """Run a single M20 helper from the command line."""


@helper.command(name="list")
def helper_list() -> None:
    """List available helpers."""
    for name in sorted(HELPERS):
        click.echo(name)


@helper.command(name="run")
@click.argument("helper_name")
@click.option(
    "--json-input",
    type=click.Path(exists=True, dir_okay=False, path_type=Path),
    required=True,
    help="Path to JSON file containing helper kwargs.",
)
@click.option(
    "--provider",
    type=click.Choice([
        "anthropic", "openai", "bedrock", "azure-openai",
        "vertex", "vllm", "llama-cpp", "mlx",
    ]),
    default="anthropic",
)
@click.option("--model", default=None, help="Model id ; provider-specific.")
@click.option(
    "--mcp-endpoint",
    default=lambda: os.environ.get("EFE_MCP_ENDPOINT"),
    help="MCP server URL (env : EFE_MCP_ENDPOINT).",
)
@click.option(
    "--oidc-token",
    default=lambda: os.environ.get("EFE_OIDC_TOKEN"),
    help="OIDC bearer token (env : EFE_OIDC_TOKEN).",
)
def helper_run(
    helper_name: str,
    json_input: Path,
    provider: str,
    model: str | None,
    mcp_endpoint: str | None,
    oidc_token: str | None,
) -> None:
    """Run a helper end-to-end and print the JSON result to stdout."""
    if helper_name not in HELPERS:
        raise click.UsageError(
            f"unknown helper {helper_name!r}. "
            f"Known : {sorted(HELPERS)}"
        )

    args = json.loads(json_input.read_text(encoding="utf-8"))
    if not isinstance(args, dict):
        raise click.UsageError("--json-input must contain a JSON object")

    try:
        result = asyncio.run(
            _run(
                helper_name=helper_name,
                args=args,
                provider=provider,
                model=model,
                mcp_endpoint=mcp_endpoint,
                oidc_token=oidc_token,
            )
        )
    except EfeAgentError as e:
        click.echo(f"error : {e}", err=True)
        sys.exit(1)

    click.echo(json.dumps(result, indent=2, ensure_ascii=False))


async def _run(
    *,
    helper_name: str,
    args: dict,
    provider: str,
    model: str | None,
    mcp_endpoint: str | None,
    oidc_token: str | None,
) -> dict:
    llm = _build_provider(provider, model)
    agent = Agent(
        llm_provider=llm,
        mcp_endpoint=mcp_endpoint,
        oidc_token=oidc_token,
    )
    try:
        return await agent.helper(helper_name, **args)
    finally:
        await agent.aclose()


def _build_provider(name: str, model: str | None):
    if name == "anthropic":
        from .llm import AnthropicProvider
        return AnthropicProvider(model=model or "claude-sonnet-4-5")
    if name == "openai":
        from .llm import OpenAIProvider
        return OpenAIProvider(model=model or "gpt-5-mini")
    if name == "bedrock":
        from .llm import BedrockProvider
        return BedrockProvider(
            model_id=model or "anthropic.claude-sonnet-4-5-v1:0",
        )
    if name == "azure-openai":
        from .llm import AzureOpenAIProvider
        endpoint = os.environ.get("AZURE_OPENAI_ENDPOINT")
        if not endpoint:
            raise EfeAgentError(
                "azure-openai requires AZURE_OPENAI_ENDPOINT env var"
            )
        return AzureOpenAIProvider(
            deployment_name=model or "gpt-5",
            endpoint=endpoint,
        )
    if name == "vertex":
        from .llm import VertexProvider
        return VertexProvider(model=model or "gemini-2.5-flash")
    if name == "vllm":
        from .llm import VLLMProvider
        url = os.environ.get("EFE_VLLM_URL", "http://localhost:8000/v1")
        return VLLMProvider(server_url=url, model=model or "local")
    if name == "llama-cpp":
        from .llm import LlamaCppProvider
        url = os.environ.get("EFE_LLAMA_CPP_URL", "http://localhost:8080/v1")
        return LlamaCppProvider(server_url=url, model=model or "local")
    if name == "mlx":
        from .llm import MLXProvider
        if not model:
            raise EfeAgentError("--model is required for mlx provider")
        return MLXProvider(model_path=model)
    raise EfeAgentError(f"unknown provider {name!r}")


@main.group()
def providers() -> None:
    """Provider utilities."""


@providers.command(name="list")
def providers_list() -> None:
    """List available providers (extras-aware)."""
    items = [
        ("anthropic",     "efe-agent[anthropic]"),
        ("openai",        "efe-agent[openai]"),
        ("bedrock",       "efe-agent[bedrock]"),
        ("azure-openai",  "efe-agent[azure-openai]"),
        ("vertex",        "efe-agent[vertex]"),
        ("vllm",          "efe-agent[vllm]      (delegates to openai SDK)"),
        ("llama-cpp",     "efe-agent[llama-cpp] (delegates to openai SDK)"),
        ("mlx",           "efe-agent[mlx]       (Mac M-series only)"),
    ]
    for name, install in items:
        click.echo(f"{name:14}  → {install}")


if __name__ == "__main__":
    main()
