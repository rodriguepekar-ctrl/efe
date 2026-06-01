"""Tutorial 1 — Classify a single text via the Python SDK.

Sprint **Φ-C-SDK Completion (D32)** — artefact #25.

This is the simplest possible EFE use case : take a text, classify it
against a catalog, print the predicted label.

## Run

```bash
# With the in-process PyO3 path (5× faster) :
EFE_CLIENT_MODE=native \\
BUNDLE_PATH=/etc/efe/bundles/active \\
CATALOG_PATH=/etc/efe/catalogs/router-v1.json \\
    python main.py

# With the gRPC path (= multi-tenant) :
EFE_CLIENT_MODE=grpc \\
EFE_HOST=efe.example.com:443 \\
    python main.py
```
"""

from __future__ import annotations

import os
import sys

from efe._dropin import get_client


def main() -> int:
    bundle_path = os.environ.get("BUNDLE_PATH")
    catalog_path = os.environ.get("CATALOG_PATH", "/etc/efe/catalogs/router-v1.json")
    host = os.environ.get("EFE_HOST")

    client = get_client(
        bundle_path=bundle_path,
        catalog_path=catalog_path,
        host=host,
    )
    try:
        text = "I want to dispute the charge on my account"
        verdict = client.classify(text)
        print(f"text          : {text}")
        print(f"label         : {verdict.label}")
        print(f"confidence    : {verdict.confidence:.4f}")
        print(f"rounds        : {verdict.rounds_executed}")
        print(f"swarm_id      : {verdict.swarm_id}")
        print(f"catalog_ver   : {verdict.catalog_version}")
    finally:
        if hasattr(client, "close"):
            client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
