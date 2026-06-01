"""Drop-in migration helper between gRPC ``EfeClient`` and PyO3 ``EfeNativeClient``.

Sprint **Φ-C-SDK Completion (D33)** — artefact #28.
Cf. ``docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md`` § 13.

## Mission

Allow Python users to switch between the **in-process PyO3 path**
(`efe._native.EfeNativeClient` — 5× faster, no network) and the
**gRPC path** (`efe_tool.EfeClient` — multi-tenant, networked)
with **2 lines of code changed**.

## Selection logic

1. If env ``EFE_CLIENT_MODE=native`` → force PyO3 path.
2. If env ``EFE_CLIENT_MODE=grpc`` → force gRPC path.
3. If env ``EFE_CLIENT_MODE=auto`` (default) :
   * If ``bundle_path`` argument is set → PyO3 path.
   * If ``host`` argument is set → gRPC path.
   * If both set → PyO3 path (= faster).
   * If neither set → raise ``ValueError``.

## Usage

```python
from efe._dropin import get_client

# Auto-pick (uses environment).
client = get_client(bundle_path="/etc/efe/bundles/active",
                    catalog_path="/etc/efe/catalogs/router-v1.json")

# Force gRPC mode.
client = get_client(host="efe.example.com:443",
                    catalog_path="/etc/efe/catalogs/router-v1.json")

# Force native mode via env.
os.environ["EFE_CLIENT_MODE"] = "native"
client = get_client(bundle_path="/etc/efe/bundles/active")

# Common interface — both clients expose `.classify(text)` returning a Verdict.
verdict = client.classify("Hello world")
print(verdict.label)
```

## Why this matters

Pre-Φ-C-FFI, the gRPC client (`efe_tool.EfeClient`) was the only path.
Φ-C-FFI added the in-process PyO3 path (`efe._native.EfeNativeClient`)
which is 5× faster but requires a local bundle. Customers who
want to migrate from gRPC to in-process used to need code changes
in every callsite ; this helper makes it a 1-line swap.

## Limitations

* The PyO3 path requires a local bundle (= disk space). Not suitable
  for serverless environments with cold starts.
* The gRPC path requires network connectivity. Not suitable for
  offline analytics.
* For maximum perf in mixed environments, use the explicit
  `EfeNativeClient` or `EfeClient` directly — the dropin helper has
  a small selection overhead (~ 5 µs).
"""

from __future__ import annotations

import os
from typing import Any, Protocol, runtime_checkable


@runtime_checkable
class ClassifyClient(Protocol):
    """Common protocol implemented by both ``EfeClient`` and ``EfeNativeClient``.

    Used purely for type-hinting ; runtime dispatch is dynamic.
    """

    def classify(self, text: str) -> Any:
        """Classify a text and return a Verdict."""
        ...

    def close(self) -> None:
        """Release any resources held by the client."""
        ...


def get_client(
    *,
    bundle_path: str | None = None,
    catalog_path: str | None = None,
    host: str | None = None,
    seed: int | None = None,
    **kwargs: Any,
) -> ClassifyClient:
    """Pick the right client based on env + args.

    Selection rules :
      1. ``EFE_CLIENT_MODE=native`` → PyO3 path (requires ``bundle_path``).
      2. ``EFE_CLIENT_MODE=grpc``   → gRPC path (requires ``host``).
      3. Default auto :
         * ``bundle_path`` set → PyO3.
         * ``host`` set        → gRPC.
         * both                → PyO3 (= faster).
         * neither             → ``ValueError``.

    Returns a client with at least ``.classify(text)`` and ``.close()``.
    """
    mode = os.environ.get("EFE_CLIENT_MODE", "auto").lower()

    if mode == "native":
        return _build_native(
            bundle_path=bundle_path,
            catalog_path=catalog_path,
            seed=seed,
            **kwargs,
        )
    if mode == "grpc":
        return _build_grpc(host=host, **kwargs)

    # Auto mode.
    if bundle_path:
        return _build_native(
            bundle_path=bundle_path,
            catalog_path=catalog_path,
            seed=seed,
            **kwargs,
        )
    if host:
        return _build_grpc(host=host, **kwargs)

    raise ValueError(
        "get_client() in auto mode requires either bundle_path "
        "(= PyO3 path) or host (= gRPC path). "
        "Set EFE_CLIENT_MODE=native or grpc to force a path."
    )


def _build_native(
    *,
    bundle_path: str | None,
    catalog_path: str | None,
    seed: int | None,
    **kwargs: Any,
) -> Any:
    """Construct the PyO3 in-process client."""
    if not bundle_path:
        raise ValueError(
            "native client requires bundle_path "
            "(e.g. /etc/efe/bundles/active)"
        )
    try:
        from efe._native import EfeNativeClient, load_bundle  # type: ignore[import-not-found]
    except ImportError as e:
        raise RuntimeError(
            "EFE_CLIENT_MODE=native requested but `efe._native` is not "
            f"importable ({e}). Install via `pip install efe`."
        ) from e
    bundle = load_bundle(bundle_path)
    return EfeNativeClient(
        bundle,
        catalog_path or "/etc/efe/catalogs/router-v1.json",
        seed=seed,
        **kwargs,
    )


def _build_grpc(*, host: str | None, **kwargs: Any) -> Any:
    """Construct the gRPC client."""
    if not host:
        raise ValueError(
            "gRPC client requires host (e.g. efe.example.com:443)"
        )
    try:
        from efe_tool import EfeClient  # type: ignore[import-not-found]
    except ImportError as e:
        raise RuntimeError(
            "EFE_CLIENT_MODE=grpc requested but `efe_tool` is not "
            f"importable ({e}). Install via `pip install efe-tool`."
        ) from e
    return EfeClient(host=host, **kwargs)


__all__ = ["ClassifyClient", "get_client"]
