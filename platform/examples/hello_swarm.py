"""Hello-world — run a mock end-to-end bot answer.

No external dependencies required — runs entirely in-process with the
:class:`InMemorySwarmTransport` and the mock LLM provider.

Run::

    python examples/hello_swarm.py
"""

from __future__ import annotations

import asyncio
import json

from efe_platform.bot.agent import Bot, BotConfig
from efe_platform.swarm.client import InMemorySwarmTransport, SwarmClient, SwarmEndpoint
from efe_platform.swarm.cone import ConeRegistryClient, InMemoryAdminTransport


async def main() -> None:
    swarm = SwarmClient(
        SwarmEndpoint("in-memory"),
        transport=InMemorySwarmTransport(),
        max_retries=0,
    )
    cones = ConeRegistryClient(InMemoryAdminTransport())
    bot = Bot(BotConfig(), swarm=swarm, cones=cones)

    print("=== 1. Conversational greeting ===")
    r1 = await bot.answer("Bonjour, qui es-tu ?")
    print(r1.final_text)
    print(f"[trace] {len(r1.trace)} event(s)")

    print()
    print("=== 2. Swarm-backed classification (mock plan) ===")
    prompt = (
        "plan: swarm.classify "
        "tenant=acme task=routing cone_version=1 "
        "problem_embedding=[0.01,0.02,0.03,0.04,0.05,0.06,0.07,0.08,"
        "0.09,0.10,0.11,0.12,0.13,0.14,0.15,0.16] "
        "dim_latent=16"
    )
    r2 = await bot.answer(prompt, tenant="acme")
    print(r2.final_text)
    print("[recap]", json.dumps(r2.recap, ensure_ascii=False, indent=2))
    print("[ledger]", json.dumps(r2.ledger, indent=2))

    await swarm.close()


if __name__ == "__main__":
    asyncio.run(main())
