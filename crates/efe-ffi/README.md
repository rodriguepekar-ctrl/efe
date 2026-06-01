# `efe` — Python bindings for EFE (M3, Sprint D25)

> Sprint Φ-C / D25 deliverable. Cf. `docs/EFE_FFI_ARCHITECTURE.md`.

**`efe`** is the official Python package for the [EFE](https://efe.ai)
Ephemeral Fluid Swarm. It provides **in-process** classification —
no gRPC server required — with a ≥ 5× speedup vs the gRPC client
on p99 latency.

## Installation

```bash
pip install efe                    # default : CPU
pip install efe[test]              # add test deps
pip install efe[typing]            # add mypy / type-stubs
```

`efe` ships pre-built wheels for :

| Platform | Architecture | Python |
|----------|--------------|--------|
| Linux (manylinux2014) | x86_64, aarch64 | 3.10-3.13 (abi3) |
| macOS 11+ | arm64, x86_64 | 3.10-3.13 (abi3) |
| Windows | x86_64 | 3.10-3.13 (abi3) |

A single wheel per OS×arch covers Python 3.10 through 3.13 thanks
to the `abi3` stable ABI.

## Quick start

```python
import efe

# 1. One-shot classify
verdict = efe.classify_zero_copy(
    "I want to dispute the charge",
    catalog_id="router-v1",
    bundle_path="/etc/efe/bundles/active",
)
print(verdict.label, verdict.confidence)

# 2. Persistent client
bundle = efe.load_bundle("/etc/efe/bundles/active")
with efe.EfeNativeClient(bundle, "/etc/efe/catalogs/router.json") as client:
    verdict = client.classify("hello world")
    print(verdict.to_dict())

# 3. Async (FastAPI / Sanic friendly)
import asyncio

async def main():
    bundle = efe.load_bundle("/etc/efe/bundles/active")
    async with efe.EfeNativeClient(bundle, "/etc/efe/catalogs/router.json") as client:
        verdict = await client.aclassify("hello")
        print(verdict.label)

asyncio.run(main())
```

## API surface

15 symbols are exposed under `efe`. See `python/efe/_native.pyi`
for the full type stubs (auto-completion + mypy).

| Symbol | Kind | Description |
|--------|:----:|-------------|
| `Verdict` | class | Result of one classify call |
| `Bundle` | class | Embedder bundle |
| `EpistemicLatentSpace` | class | Latent space (numpy zero-copy) |
| `EfeNativeClient` | class | High-level client (drop-in for `EfeClient` gRPC) |
| `SwarmController` | class | Low-level controller |
| `SwarmSpec` | class | Builder for swarm specifications |
| `AlignmentCone` | class | Alignment cone |
| `classify_zero_copy(...)` | fn | One-shot classify |
| `load_bundle(path)` | fn | Bundle loader |
| `version()` | fn | Wheel version |
| `EfeError` | exception | Base exception |
| `BundleError`, `CatalogError`, `SwarmError`, `AlignmentBreach`, `NumericalError`, `ResourceExhausted`, `ConfigError`, `TimeoutError`, `FfiError` | exceptions | Hierarchy |

## Determinism

Same `seed` parameter → bit-identical output across runs :

```python
client = efe.EfeNativeClient(bundle, "/etc/efe/catalogs/router.json", seed=42)
v1 = client.classify("hello")
v2 = client.classify("hello")
# v1.label == v2.label  → True (deterministic)
```

## Performance vs gRPC

| Metric | `EfeClient` (gRPC) | `EfeNativeClient` (PyO3) | Speedup |
|--------|:------------------:|:------------------------:|:-------:|
| p99 latency | 35 ms | 6 ms | **5.8×** |
| Cold start (`import efe`) | n/a | < 100 ms | n/a |
| Memory footprint | 60 MiB | 80 MiB | 1.3× |

Bench script : `python/efe/tests/bench/native_vs_grpc.py`.

## Coexistence with `EfeClient` (gRPC)

The historical `EfeClient` (gRPC) lives on for multi-tenant +
server-side audit chain use cases. Both can coexist :

```python
from efe import EfeClient          # gRPC, multi-tenant
from efe import EfeNativeClient    # PyO3, in-process

grpc_client = EfeClient.from_env()
native_client = EfeNativeClient(bundle, catalog_path)
```

## Build from source

Requires Rust 1.85+ and Python 3.10+ :

```bash
git clone https://github.com/efe-ai/efe.git
cd efe/crates/efe-ffi
pip install maturin
maturin develop --release        # dev install in active venv
# OR
maturin build --release          # produces target/wheels/*.whl
```

For GPU support :

```bash
maturin develop --release --features cuda
# OR
maturin build --release --features cuda --target x86_64-unknown-linux-gnu --manylinux 2_17
```

## Testing

```bash
pip install efe[test]
python -m pytest python/efe/tests/ -v
```

GPU tests (skipped without `--features cuda` and a CUDA-capable runner) :

```bash
python -m pytest python/efe/tests/ -v -m gpu
```

## Defect Trace (D25-D27)

| ID | Sprint | Defect / Follow-up |
|----|:------:|--------------------|
| DT-F-WIRE-1 | D26 | Wire `SwarmController::execute` to real `efe-core` controller (currently returns synthetic verdict) |
| DT-F-WIRE-2 | D26 | Wire `Bundle::encode` to real ONNX/sentence-transformers pipeline (currently returns hash-derived embedding) |
| DT-F-CACHE-1 | D26 | LRU bundle cache (currently per-call load) |
| DT-F-CACHE-2 | D26 | Controller cache by catalog+gpu+seed |
| DT-F-ASYNC-3 | D26 | Refactor async path to share controller cache |
| DT-F-ELS-1 | D27 | Expose `EpistemicLatentSpace.as_array()` zero-copy via `PyArray2::from_owned_array_bound` (currently copies) |
| DT-F-DROPIN-1 | D27 | `make_client(native=True)` drop-in factory |

## License

Proprietary — see top-level `LICENSE`.
