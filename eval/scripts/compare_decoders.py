"""Sprint D8 — 3-way decoder comparison report.

Runs three classifiers against the same (catalog, golden) pair :

  1. **swarm-only** — `EfeClient.classify` direct (post-D7 default)
  2. **hybrid**     — `HybridClassifier` (Plan B, D7.6 armed, D8.0 activated)
  3. **raw-cosine** — argmax of input embedding cosine to anchors,
                      no swarm RPC at all (pure embedder baseline)

Emits :
  * JSON report `target/decoder_comparison.json` — full per-case data
  * Markdown summary `target/decoder_comparison.md` — human-readable

This script is the **chiffrage objectif** of the Plan B value-add :
* Where does hybrid beat swarm-only ? (specific cases identified)
* Where does swarm-only beat raw-cosine ? (cases where the swarm
  consensus actually adds signal beyond the embedder)
* Where do all three converge ? (the easy cases — most of them)
* Where do they all disagree ? (the truly hard cases — H3c-α
  inherent ambiguity)

Usage :
    python eval/scripts/compare_decoders.py \
        --catalog catalogs/support_triage.yaml \
        --golden eval/golden/support_triage.jsonl \
        --endpoint 127.0.0.1:50051 \
        --output-dir target/decoder_comparison
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from collections import Counter
from pathlib import Path
from typing import Any

import numpy as np

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_CLIENT = _ROOT / "clients" / "python"
if (_CLIENT / "efe_tool").is_dir():
    sys.path.insert(0, str(_CLIENT))

sys.path.insert(0, str(_THIS.parent))
from run_eval import compute_metrics, load_golden  # noqa: E402

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder  # noqa: E402
from efe_tool.integrations.hybrid_decoder import (  # noqa: E402
    HybridClassifier,
    HybridResult,
    raw_cosine_predict,
)


def _classify_raw_cosine(
    text: str, catalog: AnchorCatalog, embedder: LocalSTEmbedder
) -> tuple[str, float, float]:
    """Classify via pure raw-cosine to catalog anchors. No RPC."""
    vec = embedder.encode(text)
    label, top, runner = raw_cosine_predict(vec, catalog)
    return label, top, runner


def compare_one_catalog(
    *,
    catalog_path: str,
    golden_path: str,
    endpoint: str,
    verbose: bool = False,
) -> dict[str, Any]:
    """Run all 3 classifiers on the same (catalog, golden) and emit
    a structured comparison."""
    embedder = LocalSTEmbedder()
    catalog = AnchorCatalog.load(catalog_path, embedder=embedder)
    items = load_golden(golden_path)
    labels = list(catalog.class_names())

    print(f"\n=== {catalog.name}@{catalog.version} ↔ "
          f"{Path(golden_path).name} (n={len(items)}) ===")

    # --- 1. Raw cosine (no RPC, fast) ---
    raw_preds: list[tuple[str, str, bool, float]] = []
    raw_lat: list[float] = []
    print("[1/3] Raw cosine ...")
    for it in items:
        t0 = time.perf_counter()
        label, top, _ = _classify_raw_cosine(it["text"], catalog, embedder)
        raw_lat.append((time.perf_counter() - t0) * 1000.0)
        raw_preds.append((it["expected"], label, label == "uncertain", float(top)))

    # --- 2. Swarm-only (RPC) ---
    print("[2/3] Swarm-only ...")
    os.environ.setdefault("EFE_ENDPOINT", endpoint)
    client = EfeClient.from_env()
    try:
        swarm_preds: list[tuple[str, str, bool, float]] = []
        swarm_lat: list[float] = []
        for it in items:
            t0 = time.perf_counter()
            try:
                r = client.classify(it["text"], catalog)
                lat = (time.perf_counter() - t0) * 1000.0
                lbl = r.label if not r.reject else "uncertain"
                swarm_preds.append((it["expected"], lbl, r.reject, float(r.confidence)))
            except Exception as e:
                print(f"  !! {it['id']} swarm error: {e}")
                lat = 0.0
                swarm_preds.append((it["expected"], "error", True, 0.0))
            swarm_lat.append(lat)

        # --- 3. Hybrid ---
        print("[3/3] Hybrid ...")
        hybrid = HybridClassifier(client=client, catalog=catalog)
        hybrid_preds: list[tuple[str, str, bool, float]] = []
        hybrid_lat: list[float] = []
        hybrid_branches: Counter[str] = Counter()
        for it in items:
            t0 = time.perf_counter()
            try:
                hr: HybridResult = hybrid.classify(it["text"])
                lat = (time.perf_counter() - t0) * 1000.0
                lbl = hr.label if hr.label != "uncertain" else "uncertain"
                hybrid_preds.append(
                    (it["expected"], lbl, hr.label == "uncertain", float(hr.confidence))
                )
                hybrid_branches[hr.path] += 1
            except Exception as e:
                print(f"  !! {it['id']} hybrid error: {e}")
                lat = 0.0
                hybrid_preds.append((it["expected"], "error", True, 0.0))
                hybrid_branches["error"] += 1
            hybrid_lat.append(lat)
    finally:
        client.close()

    # --- Aggregate metrics per decoder ---
    raw_metrics = compute_metrics(raw_preds, labels=labels)
    swarm_metrics = compute_metrics(swarm_preds, labels=labels)
    hybrid_metrics = compute_metrics(hybrid_preds, labels=labels)

    # --- Per-case agreement matrix ---
    per_case = []
    for i, it in enumerate(items):
        per_case.append({
            "id": it["id"],
            "text": it["text"][:80],
            "expected": it["expected"],
            "raw": {"pred": raw_preds[i][1], "score": raw_preds[i][3]},
            "swarm": {"pred": swarm_preds[i][1], "conf": swarm_preds[i][3]},
            "hybrid": {"pred": hybrid_preds[i][1], "conf": hybrid_preds[i][3]},
            "all_agree": (
                raw_preds[i][1] == swarm_preds[i][1] == hybrid_preds[i][1]
                == it["expected"]
            ),
        })

    # --- Disagreement classes ---
    only_raw_correct = sum(
        1 for c in per_case
        if c["raw"]["pred"] == c["expected"]
        and c["swarm"]["pred"] != c["expected"]
    )
    only_swarm_correct = sum(
        1 for c in per_case
        if c["swarm"]["pred"] == c["expected"]
        and c["raw"]["pred"] != c["expected"]
    )
    hybrid_better_than_swarm = sum(
        1 for c in per_case
        if c["hybrid"]["pred"] == c["expected"]
        and c["swarm"]["pred"] != c["expected"]
    )
    hybrid_worse_than_swarm = sum(
        1 for c in per_case
        if c["hybrid"]["pred"] != c["expected"]
        and c["swarm"]["pred"] == c["expected"]
    )
    all_three_wrong = sum(
        1 for c in per_case
        if c["raw"]["pred"] != c["expected"]
        and c["swarm"]["pred"] != c["expected"]
        and c["hybrid"]["pred"] != c["expected"]
    )

    return {
        "catalog": catalog.name,
        "catalog_version": catalog.version,
        "golden_path": str(golden_path),
        "n": len(items),
        "raw_cosine": {
            "metrics": raw_metrics,
            "latency_ms": {
                "mean": round(sum(raw_lat) / max(1, len(raw_lat)), 2),
                "max": round(max(raw_lat or [0.0]), 2),
            },
        },
        "swarm_only": {
            "metrics": swarm_metrics,
            "latency_ms": {
                "mean": round(sum(swarm_lat) / max(1, len(swarm_lat)), 2),
                "max": round(max(swarm_lat or [0.0]), 2),
            },
        },
        "hybrid": {
            "metrics": hybrid_metrics,
            "latency_ms": {
                "mean": round(sum(hybrid_lat) / max(1, len(hybrid_lat)), 2),
                "max": round(max(hybrid_lat or [0.0]), 2),
            },
            "branches": dict(hybrid_branches),
        },
        "delta": {
            "only_raw_correct": only_raw_correct,
            "only_swarm_correct": only_swarm_correct,
            "hybrid_better_than_swarm": hybrid_better_than_swarm,
            "hybrid_worse_than_swarm": hybrid_worse_than_swarm,
            "all_three_wrong": all_three_wrong,
        },
        "per_case": per_case if verbose else per_case[:10],  # cap if not verbose
    }


# ---------------------------------------------------------------------------
# Markdown rendering
# ---------------------------------------------------------------------------


def render_markdown(report_per_catalog: list[dict[str, Any]]) -> str:
    out: list[str] = []
    out.append("# Sprint D8 — Decoder comparison report\n\n")
    out.append("Comparison of 3 decoders on the same (catalog, golden) pairs :\n\n")
    out.append("| Decoder | Description |\n")
    out.append("|---------|-------------|\n")
    out.append("| `raw_cosine` | argmax cosine to catalog anchors, no RPC, embedder-only |\n")
    out.append("| `swarm_only` | EfeClient.classify direct (post-D7 default) |\n")
    out.append("| `hybrid`     | HybridClassifier (raw + swarm consensus, Plan B) |\n\n")

    for r in report_per_catalog:
        out.append(f"## {r['catalog']}@{r['catalog_version']} ↔ "
                   f"`{Path(r['golden_path']).name}` (n={r['n']})\n\n")
        out.append("| Decoder | Acc | F1 | Mean conf correct | Latency mean | Latency max |\n")
        out.append("|---------|----:|---:|------------------:|-------------:|------------:|\n")
        for key in ("raw_cosine", "swarm_only", "hybrid"):
            d = r[key]
            m = d["metrics"]["summary"]
            lat = d["latency_ms"]
            out.append(
                f"| `{key}` | {m['accuracy_top1']:.4f} | {m['macro_f1']:.4f} | "
                f"{m['mean_conf_correct']:.4f} | {lat['mean']:>5.0f} ms | {lat['max']:>6.0f} ms |\n"
            )
        out.append("\n### Hybrid branch distribution\n\n")
        out.append("| Branch | Count | Pct |\n|--------|------:|----:|\n")
        b = r["hybrid"]["branches"]
        n = r["n"]
        for path in ("agree", "raw_overrides_swarm", "swarm_overrides_raw", "reject"):
            cnt = b.get(path, 0)
            out.append(f"| `{path}` | {cnt} | {100*cnt/max(1,n):.1f} % |\n")
        out.append("\n### Delta analysis\n\n")
        d = r["delta"]
        out.append(f"- Only raw correct (swarm wrong) : **{d['only_raw_correct']}**\n")
        out.append(f"- Only swarm correct (raw wrong) : **{d['only_swarm_correct']}**\n")
        out.append(f"- Hybrid > swarm-only : **{d['hybrid_better_than_swarm']}**\n")
        out.append(f"- Hybrid < swarm-only : **{d['hybrid_worse_than_swarm']}**\n")
        out.append(f"- All three wrong (H3c-α inherent) : **{d['all_three_wrong']}**\n\n")

    return "".join(out)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _main() -> int:
    p = argparse.ArgumentParser(
        description="Sprint D8 — 3-way decoder comparison (raw / swarm / hybrid).")
    p.add_argument("--catalog", action="append", required=True)
    p.add_argument("--golden", action="append", required=True)
    p.add_argument("--endpoint", default="127.0.0.1:50051")
    p.add_argument("--output-dir", default="target/decoder_comparison")
    p.add_argument("-v", "--verbose", action="store_true")
    args = p.parse_args()
    if len(args.catalog) != len(args.golden):
        print("FATAL : --catalog and --golden counts must match.")
        return 2

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    reports = []
    for cat_path, gold_path in zip(args.catalog, args.golden):
        r = compare_one_catalog(
            catalog_path=cat_path,
            golden_path=gold_path,
            endpoint=args.endpoint,
            verbose=args.verbose,
        )
        reports.append(r)

    (output_dir / "report.json").write_text(
        json.dumps(reports, indent=2, ensure_ascii=False)
    )
    (output_dir / "report.md").write_text(render_markdown(reports))
    print(f"\nWrote :\n  {output_dir / 'report.json'}")
    print(f"  {output_dir / 'report.md'}")
    return 0


if __name__ == "__main__":
    sys.exit(_main())
