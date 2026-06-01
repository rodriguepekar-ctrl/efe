"""End-to-end routing example with audit log + metrics.

This example demonstrates the full production-like path:

1. Start a :class:`SwarmClient` + :class:`ConeRegistryClient` against
   in-memory transports.
2. Register a cone.
3. Build a :class:`Bot` with an attached :class:`AuditLog` (tenant
   ``acme``, redact-prompts enabled).
4. Drive three classify rounds via the bot.
5. Print metrics + replay the audit file to stdout.

Run::

    python examples/audited_routing.py
"""

from __future__ import annotations

import asyncio
import json
import tempfile
from pathlib import Path

from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.bot.providers.mock import MockProvider
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.cone import (
    AlignmentCone,
    ConeRegistryClient,
    InMemoryAdminTransport,
)
from efe_platform.swarm.protocol import AnchorRef
from efe_platform.telemetry.audit import AuditLog
from efe_platform.telemetry.metrics import MetricsRegistry, RequestMetrics

PROMPTS: list[str] = [
    (
        "plan: swarm.classify tenant=acme task=routing cone_version=1 "
        "problem_embedding=[0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,"
        "0.9,1.0,0.1,0.2,0.3,0.4,0.5,0.6] dim_latent=16"
    ),
    (
        "plan: swarm.classify tenant=acme task=routing cone_version=1 "
        "problem_embedding=[0.01,0.02,0.03,0.04,0.05,0.06,0.07,0.08,"
        "0.09,0.10,0.11,0.12,0.13,0.14,0.15,0.16] dim_latent=16"
    ),
    "Bonjour, comment ça va ?",  # conversational
]


async def main() -> None:
    audit_path = Path(tempfile.gettempdir()) / "efe_platform_demo_audit.jsonl"
    audit_path.unlink(missing_ok=True)

    swarm = SwarmClient(
        SwarmEndpoint("in-memory"),
        transport=InMemorySwarmTransport(),
        max_retries=0,
    )
    cones = ConeRegistryClient(InMemoryAdminTransport())

    # Register a trivial cone so the tool "swarm.list_cones" has data.
    await cones.register(
        tenant="acme",
        task="routing",
        version=1,
        cone=AlignmentCone(
            anchors=(AnchorRef(tuple([float(i) / 16 for i in range(16)])),),
            max_projection_distance=2.0,
        ),
        created_by="demo",
        description="demo cone",
    )

    bot = Bot(BotConfig(), swarm=swarm, cones=cones, provider=MockProvider())
    audit = AuditLog(audit_path, redact_prompts=True, fsync_each=False)
    metrics = MetricsRegistry()

    for i, prompt in enumerate(PROMPTS, start=1):
        reply = await bot.answer(prompt, tenant="acme")
        audit.record_bot_reply(
            request_id=reply.request_id,
            tenant=reply.tenant,
            reply_payload=reply.to_dict(),
        )
        metrics.record_request(
            RequestMetrics(
                tenant=reply.tenant,
                outcome="ok" if reply.ok else "error",
                duration_s=reply.duration_ms / 1000.0,
                tool_calls=int(reply.ledger["tool_calls_made"]),
                llm_tokens=int(reply.ledger["tokens_consumed"]),
                llm_usd=float(reply.ledger["usd_consumed"]),
            )
        )
        print(
            f"[{i}] ok={reply.ok} verdict={reply.recap.get('verdict'):<14} "
            f"dur={reply.duration_ms:.1f}ms "
            f"calls={reply.ledger['tool_calls_made']}"
        )

    audit.close()
    print()
    print("=== /metrics (Prometheus text) ===")
    print(metrics.render())
    print(f"=== audit ({audit_path}) ===")
    for entry in AuditLog(audit_path, fsync_each=False).replay():
        print(json.dumps(entry, indent=2, ensure_ascii=False))

    await swarm.close()


if __name__ == "__main__":
    asyncio.run(main())
