#!/usr/bin/env python3
"""Python SDK contract runner.

Sprint **Φ-C-SDK Completion (D30)** — artefact #15.

Reads ``/fixtures/quartet_fixtures.json``, calls the Python SDK for
each fixture, writes verdicts to ``/shared/out_py.json``.

## Strategy

We use the **gRPC** path (= `EfeClient` from `efe_tool`) for cross-SDK
comparison, NOT the PyO3 in-process path. Reason : the contract test
verifies that the gRPC wire format produces bit-identical verdicts
across the 4 SDKs ; the in-process path bypasses the wire entirely.

When `efe_tool` is unavailable (= testing locally without gRPC client
installed), we fall back to a **mock mode** that emits deterministic
synthetic verdicts based on the SHA-256-derived formula documented in
Φ-C-FFI freeze_numerical_equiv.py. The mock mode is enabled via env
``EFE_CONTRACT_MOCK=1`` — in CI we always use the real gRPC client.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import struct
import sys
import time
from pathlib import Path


def mock_verdict_for(fixture: dict, labels: list[str], catalog_version: str = "1.0.0") -> dict:
    """Deterministic mock verdict — matches the D25 Φ-C-FFI stub formula.

    Used when EFE_CONTRACT_MOCK=1 (= local testing without backend).
    The cross-SDK contract test in CI uses the real gRPC backend.
    """
    text = fixture["text"]
    catalog_id = fixture["catalog_id"]
    seed = fixture["seed"]

    # Replicate `InnerBundle::encode(text)` from efe-ffi (= D25 stub).
    bundle_id = "test-bundle"
    DIM = 384
    seed_bytes = hashlib.sha256(text.encode("utf-8") + bundle_id.encode("utf-8")).digest()
    # **Audit fix Φ-C-SDK.B12** — numpy is MANDATORY for f32 fidelity
    # cross-SDK. Pre-fix had a silent f64 fallback that diverged from
    # Go/TS/Java runners by ~ 396/1536 bytes → contract test failure
    # would surface as "label mismatch" instead of "numpy missing".
    # Post-fix : fail loud immediately. Doctrine D-SDK-1 demands
    # bit-identity ; we don't compromise on numerics.
    try:
        import numpy as np
    except ImportError as e:
        sys.stderr.write(
            "FATAL : numpy is required for f32-fidelity mock mode "
            "(Φ-C-SDK.B12). Install via `pip install numpy`.\n"
            f"  underlying error : {e}\n"
        )
        raise
    raw = np.array([seed_bytes[i % 32] for i in range(DIM)], dtype=np.uint8)
    emb = raw.astype(np.float32) / np.float32(255.0) - np.float32(0.5)
    sum_sq = np.float32(0.0)
    for v in emb:
        sum_sq = np.float32(sum_sq + np.float32(v) * np.float32(v))
    norm = np.float32(math.sqrt(float(sum_sq)))
    if float(norm) > 0.0:
        emb = emb / norm
    emb_bytes = emb.tobytes()

    # Replicate `synthesise_verdict`.
    h = hashlib.sha256()
    h.update(emb_bytes)
    h.update(catalog_id.encode("utf-8"))
    h.update(struct.pack("<Q", seed))
    digest = h.digest()

    label_idx = struct.unpack("<I", digest[0:4])[0] % len(labels)
    label = labels[label_idx]
    conf_u32 = struct.unpack("<I", digest[4:8])[0]
    # numpy already imported above (= mandatory ; audit fix B12)
    confidence = float(np.float32(conf_u32) / np.float32(0xFFFF_FFFF))
    rounds = 4 + (digest[8] % 16)
    swarm_id = f"swarm-{digest[:8].hex()}"

    return {
        "fixture_id": fixture["id"],
        "label": label,
        "confidence": confidence,
        "cone_id": f"cone-{catalog_id}",
        "rounds_executed": rounds,
        "swarm_id": swarm_id,
        "catalog_version": catalog_version,
        "sub_scores": [],
        "latency_ms": 0.0,
    }


def real_verdict_for(fixture: dict, client) -> dict:
    """Call the real gRPC backend via efe_tool.EfeClient."""
    t0 = time.perf_counter()
    verdict = client.classify(
        text=fixture["text"],
        catalog=fixture["catalog_id"],
        seed=fixture["seed"],
    )
    latency_ms = (time.perf_counter() - t0) * 1000.0
    # Normalize sub_scores into list of dicts for the comparator.
    sub_scores = []
    for s in (getattr(verdict, "sub_scores", []) or []):
        sub_scores.append({
            "label": getattr(s, "label", s[0] if isinstance(s, tuple) else None),
            "confidence": float(
                getattr(s, "confidence", s[1] if isinstance(s, tuple) else 0.0)
            ),
        })
    return {
        "fixture_id": fixture["id"],
        "label": verdict.label,
        "confidence": float(verdict.confidence),
        "cone_id": verdict.cone_id,
        "rounds_executed": int(verdict.rounds_executed),
        "swarm_id": verdict.swarm_id,
        "catalog_version": verdict.catalog_version,
        "sub_scores": sub_scores,
        "latency_ms": latency_ms,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description="Python SDK contract runner")
    ap.add_argument(
        "--fixtures", type=Path,
        default=Path("/fixtures/quartet_fixtures.json"),
    )
    ap.add_argument(
        "--out", type=Path,
        default=Path("/shared/out_py.json"),
    )
    ap.add_argument(
        "--host", default=os.environ.get("EFE_HOST", "efe-server:50051"),
    )
    ap.add_argument(
        "--version", default="0.6.0",
        help="SDK version label",
    )
    args = ap.parse_args()

    if not args.fixtures.exists():
        sys.stderr.write(f"FATAL : fixtures missing : {args.fixtures}\n")
        return 2
    data = json.loads(args.fixtures.read_text())
    fixtures = data["fixtures"]

    # Mock mode for local testing.
    mock_mode = os.environ.get("EFE_CONTRACT_MOCK") == "1"
    labels = [
        "billing-dispute", "tech-support", "refund-request",
        "general-inquiry", "other",
    ]

    if mock_mode:
        print(f"runner-py : MOCK mode, {len(fixtures)} fixtures", file=sys.stderr)
        verdicts = [mock_verdict_for(f, labels) for f in fixtures]
    else:
        # Real gRPC path.
        try:
            from efe_tool import EfeClient  # type: ignore[import-not-found]
        except ImportError:
            sys.stderr.write(
                "FATAL : efe_tool not installed. Set EFE_CONTRACT_MOCK=1 to use mock.\n"
            )
            return 2
        print(f"runner-py : LIVE mode, host={args.host}", file=sys.stderr)
        client = EfeClient(host=args.host)
        try:
            verdicts = [real_verdict_for(f, client) for f in fixtures]
        finally:
            client.close() if hasattr(client, "close") else None

    args.out.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "sdk": "py",
        "version": args.version,
        "count": len(verdicts),
        "verdicts": verdicts,
    }
    args.out.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")
    print(f"runner-py : wrote {len(verdicts)} verdicts → {args.out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
