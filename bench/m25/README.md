# M25 Bench Scenarios

Benchmark + chaos scenarios for the M25 agent runtime.

| File | Purpose |
|---|---|
| `e2e_agent_run.py` | End-to-end : create → provision → run → final |
| `sandbox_escape.py` | Adversarial : verify escape attempts increment metric + audited |
| `tool_whitelist_bypass.py` | Adversarial : LLM hallucinates non-whitelisted tool |
| `cost_cap_enforcement.py` | Verify SIGTERM at cap (no grace period) |
| `approval_token_replay.py` | Verify single-use enforcement |
| `gdpr_art17_cascade.py` | Verify cascade delete on offboard |

Run all :
```bash
pytest bench/m25/ -v
```

Each scenario is **self-contained** — no fixtures from `bench/load/`,
no shared state. They are designed to run against a freshly-bootstrapped
test cluster (k3d + helm install).
