"""Example : draft a runbook from an SLO breach event."""
from __future__ import annotations

import asyncio
from pathlib import Path

from efe_byo_agent import Agent
from efe_byo_agent.llm import AnthropicProvider


async def main() -> None:
    agent = Agent(
        llm_provider=AnthropicProvider(model="claude-sonnet-4-5"),
    )
    try:
        result = await agent.helper(
            "draft_runbook",
            incident_event={
                "slo_id": "M19-OBS-3",
                "trigger_metric": "log_volume_kbytes_per_minute",
                "current_value": 247,
                "threshold": 100,
                "since": "2026-08-15T14:22:11Z",
            },
            related_logs=[
                "ERROR backend bundle pull retried 3 times",
                "WARN oci registry latency p99 = 8.2s",
                "WARN structlog buffer flush rate down 70%",
            ],
            context_metadata={
                "cluster_id": "prod-us-east-1",
                "region": "us-east-1",
                "env": "production",
            },
        )

        out_path = Path("runbook_draft.md")
        out_path.write_text(result["runbook_markdown"], encoding="utf-8")
        print(f"Wrote runbook draft → {out_path.absolute()}")
        print(f"Risk level : {result['risk_level']}")
        print(
            "Estimated resolution : "
            f"{result['estimated_resolution_time_min']} min"
        )
        print("Primary diagnostics :")
        for diag in result["primary_diagnostics"]:
            print(f"  - {diag}")
    finally:
        await agent.aclose()


if __name__ == "__main__":
    asyncio.run(main())
