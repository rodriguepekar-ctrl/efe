# agent-core

EFE M25 agent runtime — Python core that runs inside a Firecracker microVM.

Boots from `/proc/cmdline` parameters, opens a vsock channel to the
controller pod, and runs the LLM-driven planner loop until completion,
approval-needed, cost-cap, or step-cap.

See `docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md` for the full spec.
