# EFE — Examples

> **Sprint D10 deliverable.** Four worked examples covering the
> beginner→advanced spectrum. Each example is `≤ 120 LOC`,
> standalone, and tested in CI against a stub gRPC server.

---

| # | File | Audience | LOC | What it shows |
|--:|------|----------|----:|---------------|
| 1 | [`01_quickstart_classify.py`](01_quickstart_classify.py) | beginner — first 10 min | ~30 | bundled `support_triage` catalog → 5 cases classified |
| 2 | [`02_custom_catalog.py`](02_custom_catalog.py) | partner adapting to domain | ~110 | author intents in a dict → materialise YAML → load + classify |
| 3 | [`03_multilang_streaming.py`](03_multilang_streaming.py) | concurrency / multilingual | ~95 | `AsyncEfeClient` + asyncio.Semaphore back-pressure + FR/EN/DE/ES |
| 4 | [`04_production_client.py`](04_production_client.py) | SRE / prod | ~200 | JWT + retries (full jitter) + OTel + structured JSON logs + graceful shutdown |

---

## Pre-requisites (all examples)

```bash
# 1. Server reachable on 127.0.0.1:50051
helm install efe oci://ghcr.io/efe-project/charts/efe --version 0.1.10 \
  -n efe --create-namespace \
  --set service.headless=true --set replicaCount=2
kubectl wait deploy/efe -n efe --for=condition=available --timeout=120s
kubectl port-forward -n efe svc/efe 50051:50051 &

# 2. SDK installed
pip install efe-tool==0.1.10

# 3. Optional for example 3+4: set EFE_IDEMPOTENCY_TTL_MS=0 on
#    server side so concurrent / production runs measure real
#    compute (not the LRU cache). See
#    docs/bench/d9_deep_analysis_supplement.md §A2.
```

## Running each example

```bash
python examples/01_quickstart_classify.py
python examples/02_custom_catalog.py
python examples/03_multilang_streaming.py
EFE_TOKEN=$(cat /tmp/jwt) python examples/04_production_client.py
```

## CI smoke tests

```bash
pytest examples/tests/ -v
```

The smoke runs each example against a gRPC stub server
(`tests/stub_server.py`), asserts:

* exit code 0 (or 2 for example 04 without `EFE_TOKEN`)
* expected label substrings present in stdout
* aggregate wallclock < 30 s
* no `efe_tool._internal.*` imports (private surface leak)

## Adapt to your domain

Start from `02_custom_catalog.py` — replace `DOMAIN_INTENTS` with
your own labels and example prompts (≥ 2 per class, mix languages
freely). The bundled `paraphrase-multilingual-MiniLM-L12-v2`
embedder covers 50+ languages in the same vector space.

For production wiring patterns (auth rotation, retries,
observability), see `04_production_client.py`.

---

## See also

* Tutorial: [`docs.efe.ai/tutorial/quickstart`](https://docs.efe.ai/tutorial/quickstart/)
* How-to: [`custom-catalog`](https://docs.efe.ai/howto/custom-catalog/)
* Reference: [`API`](https://docs.efe.ai/reference/api/)
