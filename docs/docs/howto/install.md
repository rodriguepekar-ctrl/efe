# How-to — Install

> One-shot install reference. For the guided 10-min walk-through
> see [Tutorial / Quickstart](../tutorial/quickstart.md).

## Standard installation

```bash
pip install efe-tool==0.1.10
```

## Pre-release / development

```bash
pip install --pre efe-tool          # latest pre-release
pip install -e clients/python       # editable, from a clone
```

## Optional extras

| Extra      | Pulls | When to use |
|-----------|-------|-------------|
| `[otel]`   | `opentelemetry-api`, `opentelemetry-sdk`, `opentelemetry-exporter-otlp-proto-grpc` | OpenTelemetry trace + metric export |
| `[claude]` | `anthropic>=0.30` | Claude bridge integration |
| `[openai]` | `openai>=1.30`    | OpenAI bridge integration |
| `[dev]`    | pytest + mypy + ruff | local development |

```bash
pip install 'efe-tool[otel]==0.1.10'
```

## Behind a corporate proxy

If your network blocks `pypi.org`:

```bash
# 1. Download the wheel + dependencies from a machine with public access
pip download efe-tool==0.1.10 -d /tmp/wheels

# 2. Transfer /tmp/wheels into the corporate network

# 3. Install offline
pip install --no-index --find-links /tmp/wheels efe-tool==0.1.10
```

## Verify the install

```bash
python -c "import efe_tool; print(efe_tool.__version__)"
# 0.1.10
```

## Server-side install

The Python SDK is the **client**. To run an `efe-server` you also
need the Helm chart — see
[Operations / Deployment](../../operations/deployment.md).

A minimal local dev server:

```bash
docker run --rm -p 50051:50051 -p 9090:9090 \
  ghcr.io/efe-project/efe:0.1.10
```
