"""Minimal CLI-less demo of EFE Mode-1.

Usage:
    # 1. Start the EFE server
    EFE_LISTEN_ADDR=127.0.0.1:50051 \\
    EFE_SWARM_DIM_LATENT=384 \\
    cargo run -p efe-server --release

    # 2. Run this script
    EFE_ENDPOINT=127.0.0.1:50051 python examples/demo_classify.py

Prints per-text verdicts and a summary table.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder

EXAMPLES = [
    # (text, expected_label) — expected is informational (not enforced)
    ("Someone charged my card and I never bought that.", "fraud"),
    ("The app crashes when I open the settings menu.", "bug"),
    ("How do I export my data to a CSV?", "question"),
    ("I want to cancel my subscription starting next month.", "billing"),
    ("J'ai été débité de 89 € sans autorisation, merci de bloquer.", "fraud"),
    ("L'application affiche un écran blanc après le dernier update.", "bug"),
    ("Comment modifier mon adresse de livraison par défaut ?", "question"),
    ("Pouvez-vous m'envoyer une facture avec numéro TVA ?", "billing"),
    ("It's raining hard today, the weather is beautiful, no support question.", None),  # adversarial
]


def main() -> int:
    catalog_path = Path(__file__).resolve().parents[3] / "catalogs" / "support_triage.yaml"
    if not catalog_path.exists():
        print(f"catalog not found: {catalog_path}", file=sys.stderr)
        return 2

    os.environ.setdefault("EFE_ENDPOINT", "127.0.0.1:50051")
    emb = LocalSTEmbedder()
    catalog = AnchorCatalog.load(catalog_path, embedder=emb)
    print(f"loaded catalog '{catalog.name}@{catalog.version}' — {len(catalog.classes)} classes")

    client = EfeClient.from_env()
    try:
        rows: list[tuple[str, str, float, bool, str | None, str]] = []
        for text, expected in EXAMPLES:
            result = client.classify(text, catalog)
            ok = expected is None or result.label == expected
            mark = "✓" if ok else "✗"
            rows.append((
                mark,
                result.label,
                result.confidence,
                result.reject,
                expected,
                text[:60] + ("…" if len(text) > 60 else ""),
            ))
        # Print summary.
        w = max(len(r[5]) for r in rows)
        print()
        print(f"{'  ':2}  {'label':<10}  {'conf':>5}  {'rej':<3}  {'exp':<10}  text")
        print("-" * (32 + w))
        for mark, label, conf, reject, expected, text in rows:
            print(
                f"{mark:2}  {label:<10}  {conf:5.2f}  {'yes' if reject else ' no':<3}  "
                f"{(expected or '-'):<10}  {text}"
            )
        stats = client.cache_stats()
        print(f"\ncache: hits={stats['hits']} misses={stats['misses']} size={stats['size']}")
        return 0
    finally:
        client.close()


if __name__ == "__main__":
    sys.exit(main())
