"""Agent core entry point.

Boot sequence inside the microVM :
  1. Read /proc/cmdline → extract `efe.run_id`, `efe.tenant_id`,
     `efe.vsock_cid`.
  2. Open the vsock control channel to the controller (CID 2 / port 9999).
  3. Pull initial prompt + tool whitelist via vsock-relay.
  4. Run the planner loop until : COMPLETED / TERMINATED / step cap /
     cost cap / approval timeout.
  5. Emit final_output via vsock-relay.

This file is the *only* entry point — agent-core never spawns
subprocesses, never opens a TCP socket directly, never reads files
outside its read-only rootfs.
"""
from __future__ import annotations

import asyncio
import os
import sys
from typing import Any

import structlog

from .audit_client import AuditClient
from .checkpoint import Checkpoint
from .cost_tracker import CostTracker
from .llm_client import LlmClient
from .mcp_client import McpClient, ToolCallDenied
from .pii_redact import redact_pii

log = structlog.get_logger()


class AgentCoreConfig:
    """Boot config sourced from the kernel cmdline + env."""

    def __init__(self) -> None:
        self.run_id = _required_env("EFE_RUN_ID")
        self.tenant_id = _required_env("EFE_TENANT_ID")
        self.controller_cid = int(os.environ.get("EFE_CONTROLLER_CID", "2"))
        self.controller_port = int(os.environ.get("EFE_CONTROLLER_PORT", "9999"))
        self.tool_whitelist = (os.environ.get("EFE_TOOL_WHITELIST", "")).split(",")
        self.tool_whitelist = [t for t in self.tool_whitelist if t]
        self.max_steps = int(os.environ.get("EFE_MAX_STEPS", "25"))
        self.cost_cap_cents = int(os.environ.get("EFE_COST_CAP_CENTS", "50"))
        self.provider = os.environ.get("EFE_PROVIDER", "openai")
        self.model = os.environ.get("EFE_MODEL", "gpt-4o-mini")
        self.system_prompt = os.environ.get("EFE_SYSTEM_PROMPT", "You are a helpful agent.")
        self.require_approval_for_writes = (
            os.environ.get("EFE_REQUIRE_APPROVAL_FOR_WRITES", "true").lower() == "true"
        )


def _required_env(key: str) -> str:
    v = os.environ.get(key)
    if not v:
        log.error("missing required env", key=key)
        sys.exit(2)
    return v


async def run() -> int:
    cfg = AgentCoreConfig()
    log.info(
        "agent-core booted",
        run_id=cfg.run_id,
        tenant=cfg.tenant_id,
        whitelist=cfg.tool_whitelist,
        max_steps=cfg.max_steps,
    )

    cost = CostTracker(cap_cents=cfg.cost_cap_cents)
    audit = AuditClient(cfg)
    llm = LlmClient(cfg, cost=cost, audit=audit)
    mcp = McpClient(cfg, audit=audit)
    checkpoint = Checkpoint(cfg.run_id)

    # Greet the controller (audited as run_started on its side).
    await audit.heartbeat({"version": "2.0.0"})

    # Read initial prompt (set by controller via env-var injected into
    # the cloud-init drop). For simple research tasks this is a single
    # user message ; for templates it can be more elaborate.
    initial_user_prompt = os.environ.get("EFE_INITIAL_USER_PROMPT", "Hello.")
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": cfg.system_prompt},
        {"role": "user", "content": redact_pii(initial_user_prompt)},
    ]

    final_output: dict[str, Any] | None = None
    for step in range(cfg.max_steps):
        if cost.exceeded():
            log.warning("cost cap reached", spent=cost.spent_cents)
            await audit.event("cost_cap_exceeded", {"spent_cents": cost.spent_cents})
            return 7  # exit code 7 = cost-capped

        # 1. Ask the LLM for the next move.
        llm_resp = await llm.complete(messages=messages)

        # 2. If the LLM proposed a tool call, dispatch via mcp-server.
        if llm_resp.tool_call:
            tname = llm_resp.tool_call["name"]
            targs = llm_resp.tool_call["arguments"]

            # **Layer 2 enforcement** : even if the LLM hallucinates a
            # tool name, agent-core refuses to dispatch it.
            if tname not in cfg.tool_whitelist:
                log.warning("LLM proposed non-whitelisted tool", tool=tname)
                await audit.event(
                    "tool_call_rejected_whitelist",
                    {"tool": tname, "step": step},
                )
                # Tell the LLM and continue.
                messages.append(
                    {"role": "tool", "name": tname, "content": "DENIED: not in whitelist"}
                )
                continue

            try:
                tool_result = await mcp.call_tool(tname, targs, step=step)
            except ToolCallDenied as e:
                # The mcp-server denied — usually because approval is
                # required and we don't yet have a token. Park the run
                # in AWAITING_APPROVAL by returning ; the controller
                # will resume us once granted.
                log.info("tool needs approval", tool=tname, reason=str(e))
                await checkpoint.save(messages, step)
                return 3  # exit code 3 = awaiting_approval

            messages.append(
                {"role": "tool", "name": tname, "content": tool_result}
            )
        else:
            # No tool call → the LLM emitted a final answer.
            final_output = {"answer": llm_resp.text, "steps_used": step + 1}
            break

    if final_output is None:
        # Hit max_steps without final output.
        log.warning("max steps reached without final output")
        await audit.event("run_failed", {"reason": "max_steps_reached"})
        return 6  # exit code 6 = max_steps

    await audit.event("final_output", {"answer_len": len(final_output.get("answer", ""))})
    await mcp.report_final(final_output)
    return 0


def main() -> None:
    structlog.configure(
        processors=[
            structlog.processors.add_log_level,
            structlog.processors.TimeStamper(fmt="iso"),
            structlog.processors.JSONRenderer(),
        ]
    )
    code = asyncio.run(run())
    sys.exit(code)


if __name__ == "__main__":
    main()
