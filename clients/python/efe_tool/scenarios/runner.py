"""High-level ``run_scenario`` orchestrator for Letter-C.

Given a :class:`Scenario` (loaded via :mod:`loader`), builds the
Anthropic client, the EfeClient, loads the declared catalog(s), and
drives :func:`efe_tool.bridges.claude.run_conversation` to completion.

The runner is intentionally thin — any real policy lives in the
prompt. It returns a :class:`ScenarioRunResult` pairing the underlying
:class:`ConversationResult` with the scenario-level metadata needed to
build a transcript.
"""

from __future__ import annotations

import os
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Mapping, Optional, Sequence

from ..bridges.claude import (
    ConversationResult,
    DEFAULT_MAX_ROUNDS,
    DEFAULT_MAX_TOKENS,
    DEFAULT_TEMPERATURE,
    run_conversation,
)
from ..bridges.errors import LlmAuthError
from .loader import Scenario


# ---------------------------------------------------------------------------
#  Catalog resolver
# ---------------------------------------------------------------------------


#: The callable the runner uses to turn a catalog name into a loaded
#: :class:`AnchorCatalog`. Tests override this with a fake loader to
#: avoid depending on sentence-transformers at unit-test time.
CatalogResolver = Callable[[str], Any]


def default_catalog_resolver(
    *,
    search_paths: Sequence[Path | str] = (),
    embedder: Any = None,
) -> CatalogResolver:
    """Build a resolver that maps a catalog name to an AnchorCatalog.

    Search strategy:

    1. ``search_paths`` — each directory is scanned for
       ``<name>.yaml``.
    2. ``$EFE_CATALOG_DIR`` if set.
    3. ``./catalogs/<name>.yaml`` relative to cwd.

    The first match wins. Missing sentence-transformers is tolerated
    when the ``.compiled.json`` cache is fresh next to the YAML.
    """
    normalized = [Path(p) for p in search_paths]
    env_dir = os.environ.get("EFE_CATALOG_DIR")
    if env_dir:
        normalized.append(Path(env_dir))
    normalized.append(Path.cwd() / "catalogs")

    def _resolve(name: str) -> Any:
        from ..catalog import AnchorCatalog
        for root in normalized:
            candidate = root / f"{name}.yaml"
            if candidate.is_file():
                _embedder = embedder
                if _embedder is None:
                    # Only load the embedder if we need to recompile.
                    compiled = candidate.with_name(f"{candidate.stem}.compiled.json")
                    if not compiled.exists():
                        from ..embedder import LocalSTEmbedder
                        _embedder = LocalSTEmbedder.from_env()
                return AnchorCatalog.load(candidate, embedder=_embedder)
        raise FileNotFoundError(
            f"catalog {name!r} not found in "
            f"{[str(p) for p in normalized]}"
        )

    return _resolve


# ---------------------------------------------------------------------------
#  Result
# ---------------------------------------------------------------------------


@dataclass
class ScenarioRunResult:
    """A scenario run's output.

    Wraps the lower-level :class:`ConversationResult` with the
    metadata needed to build a transcript or an exec report.
    """

    scenario: Scenario
    conversation: ConversationResult
    started_at: float
    ended_at: float
    boot_config: dict[str, Any]

    @property
    def wall_clock_s(self) -> float:
        return round(self.ended_at - self.started_at, 3)


# ---------------------------------------------------------------------------
#  Entry point
# ---------------------------------------------------------------------------


def run_scenario(
    scenario: Scenario,
    *,
    user: str,
    anthropic_client: Any | None = None,
    efe_client: Any | None = None,
    catalog_resolver: CatalogResolver | None = None,
    catalogs: Optional[Mapping[str, Any]] = None,
    model: Optional[str] = None,
    max_rounds: int = DEFAULT_MAX_ROUNDS,
    max_tokens: int = DEFAULT_MAX_TOKENS,
    temperature: float = DEFAULT_TEMPERATURE,
    cost_ceiling_usd: Optional[float] = None,
    retry_on_rate_limit: bool = True,
    on_event: Optional[Callable[[dict[str, Any]], None]] = None,
) -> ScenarioRunResult:
    """Run ``scenario`` with the provided user message.

    Args:
        scenario: A :class:`Scenario` loaded via ``load_scenario``.
        user: The initial user message to classify.
        anthropic_client: Optional pre-built Anthropic client. If None,
            one is built from ``ANTHROPIC_API_KEY`` (raises
            :class:`LlmAuthError` if absent).
        efe_client: Optional pre-built :class:`EfeClient`. If None,
            ``EfeClient.from_env`` is used.
        catalog_resolver: Optional callable used to load each catalog
            declared by the scenario. Defaults to
            :func:`default_catalog_resolver`.
        catalogs: Pre-loaded catalog map ``{name: AnchorCatalog}``. If
            provided, ``catalog_resolver`` is ignored.
        model: Override for the prompt's declared model.
        max_rounds / max_tokens / temperature / cost_ceiling_usd /
        retry_on_rate_limit / on_event:
            Forwarded to :func:`run_conversation`.

    Returns:
        :class:`ScenarioRunResult`.

    Raises:
        LlmAuthError: ``ANTHROPIC_API_KEY`` missing.
        Any error from :func:`run_conversation` (LlmError family).
    """
    anthropic_instance = anthropic_client or _make_anthropic()
    ef_client = efe_client or _make_efe_client()

    if catalogs is None:
        resolver = catalog_resolver or default_catalog_resolver()
        catalogs = {
            name: resolver(name) for name in scenario.prompt.catalog_names
        }
    if not catalogs:
        raise LlmAuthError(
            f"scenario {scenario.name!r} declares no catalogs",
            hint="add a 'catalogs:' list to the prompt frontmatter",
        )

    effective_model = model or scenario.prompt.model
    boot_config = {
        "scenario": scenario.name,
        "scenario_version": scenario.prompt.version,
        "prompt_hash": scenario.prompt.hash,
        "catalogs": list(catalogs.keys()),
        "model": effective_model,
        "max_rounds": max_rounds,
        "max_tokens": max_tokens,
        "cost_ceiling_usd": cost_ceiling_usd,
    }

    started = time.time()
    conv = run_conversation(
        anthropic_instance,
        efe_client=ef_client,
        catalogs=dict(catalogs),
        model=effective_model,
        system=scenario.prompt.text,
        user=user,
        max_rounds=max_rounds,
        max_tokens=max_tokens,
        temperature=temperature,
        cost_ceiling_usd=cost_ceiling_usd,
        retry_on_rate_limit=retry_on_rate_limit,
        on_event=on_event,
        scenario=scenario.name,
    )
    ended = time.time()

    return ScenarioRunResult(
        scenario=scenario,
        conversation=conv,
        started_at=started,
        ended_at=ended,
        boot_config=boot_config,
    )


# ---------------------------------------------------------------------------
#  Internals
# ---------------------------------------------------------------------------


def _make_anthropic() -> Any:
    api_key = os.environ.get("ANTHROPIC_API_KEY")
    if not api_key:
        raise LlmAuthError(
            "ANTHROPIC_API_KEY is not set",
            hint="export ANTHROPIC_API_KEY=sk-… before running the scenario",
        )
    try:
        from anthropic import Anthropic  # type: ignore
    except ImportError as e:
        raise LlmAuthError(
            "anthropic SDK is not installed",
            hint="pip install 'efe-tool[claude]' (or add anthropic)",
            cause=e,
        ) from e
    return Anthropic(api_key=api_key)


def _make_efe_client() -> Any:
    from ..client import EfeClient
    os.environ.setdefault("EFE_ENDPOINT", "127.0.0.1:50051")
    return EfeClient.from_env()


__all__ = [
    "ScenarioRunResult",
    "CatalogResolver",
    "default_catalog_resolver",
    "run_scenario",
]
