# efe-tool — EFE Mode-1 Python client

Use an **Ephemeral Fluid Swarm** from any Python code or LLM agent.

Quick start:

```bash
# 1. Install in editable mode
cd clients/python
pip install -e ".[dev]"

# 2. Generate the protobuf stubs once
bash scripts/gen_protos.sh

# 3. Start the EFE server in another terminal
cd ../..
EFE_LISTEN_ADDR=127.0.0.1:50051 \
EFE_SWARM_DIM_LATENT=384 \
EFE_BACKEND=mvp \
cargo run --release -p efe-server

# 4. Compile the catalog (downloads the embedder the first time)
efe-admin compile-catalog ../../catalogs/support_triage.yaml

# 5. Classify a single message
efe-admin classify "j'ai été débité sans autorisation" \
    --catalog ../../catalogs/support_triage.yaml -v

# 6. Run the batch demo
EFE_ENDPOINT=127.0.0.1:50051 python examples/demo_classify.py

# 7. Golden-set evaluation
python ../../eval/scripts/run_eval.py \
    --catalog ../../catalogs/support_triage.yaml \
    --golden  ../../eval/golden/support_triage.jsonl \
    --endpoint 127.0.0.1:50051
```

## Architecture

This client implements the **Mode 1** contract described in
[`MODE1_ARCHITECTURE.md`](../../MODE1_ARCHITECTURE.md):

```
          ┌─ Python agent / CLI / LLM ─┐
          │                            │
          │   text  →  embedder        │
          │            (sbert 384d)    │
          │               │            │
          │               ▼            │
          │          SwarmSpec         │
          │               │            │
          │               ▼            │
          └─── gRPC Execute ───────────┘
                          │
                          ▼
              ┌─ EFE Server (Rust) ─┐
              │ genesis → fusion →  │
              │ consensus → diss.   │
              └─────────────────────┘
                          │
                          ▼
              projected centroid (384d)
                          │
                          ▼
          ┌─ Python client (this lib) ─┐
          │  nearest-anchor cosine     │
          │  threshold policy          │
          │  ClassifyResult            │
          └────────────────────────────┘
```

## API at a glance

```python
from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder

catalog = AnchorCatalog.load(
    "catalogs/support_triage.yaml",
    embedder=LocalSTEmbedder(),
)
client = EfeClient.from_env()
result = client.classify("my card was charged twice", catalog)
print(result.label, result.confidence, result.reject)
# billing 0.88 False
```

## LLM integration

- Anthropic Claude: [`efe_tool.bridges.claude`](efe_tool/bridges/claude.py)
- OpenAI: [`efe_tool.bridges.openai`](efe_tool/bridges/openai.py)

See the [`examples/`](examples/) folder for ready-to-run demos.
