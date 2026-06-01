# efe-nano-sdk

**Module** : M50 — Nano Agent Swarm SDK (public, MIT)
**Status** : Scaffold (Sprint 0)
**Companion** : `efe-nano` (private runtime, M50)

## What this crate is

Public SDK for authoring and invoking **EFE nano agents**.  Customers
add this single dependency to their Rust project to :

1. Define their own nano agents via the `#[nano_agent]` proc-macro (Sprint 8).
2. Invoke EFE-shipped or customer-published nano agents in parallel
   via the [`NanoClient`] (Sprint 7).
3. Use shared types (`NanoAgent`, `SwarmRequest`, etc.) for type-safe
   interop with the runtime.

## What this crate is NOT

This is **not** the runtime.  The runtime lives in the `efe-nano`
private crate (commercial license).  Customers don't import `efe-nano`
directly.

## Quick start (Sprint 7+ — not yet available in Sprint 0)

```rust
use efe_nano_sdk::client::NanoClient;
use efe_nano_sdk::types::{SwarmRequest, Operation};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = NanoClient::from_env()?;  // reads EFE_NANO_ENDPOINT + EFE_NANO_API_KEY

    let request = SwarmRequest::builder()
        .add_operation(Operation::new("pdf_extract_text", json!({"url": "..."})))
        .add_operation(Operation::new("ner_clinical", json!({"depends_on": "..."})))
        .build()?;

    let response = client.execute_swarm(request).await?;
    println!("Total cost : {} cents", response.total_cost_cents);
    Ok(())
}
```

## Status by sprint (M50 roadmap)

- **Sprint 0 (2026-06)** : Cargo.toml + module stubs (this file).
- **Sprint 1** : Shared types (`NanoAgent`, `ExecutionMode`, etc.) implemented.
- **Sprint 7** : HTTP client (`NanoClient`) + MCP bridge.
- **Sprint 8** : proc-macro `#[nano_agent]` + CLI scaffolding tool.
- **Sprint 13** : GA — published to crates.io as `efe-nano-sdk@1.0`.

## License

MIT — see workspace `LICENSE-MIT`.

## See also

- [M50 Architecture](../../docs/architecture/EFE_M50_AGENTIC_ARCHITECTURE.md)
- [M50 Roadmap](../../docs/roadmap/EFE_M50_ROADMAP.md)
- [Decision : 2-crate split](../../docs/decisions/D-M50-CRATES-001.md)
