"""Deep diagnostic — print centroid cosine similarity to each anchor.

Helps identify why predictions cluster on a single class.
"""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder

EMBEDDER_MODEL = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"

INPUTS = [
    ("EN-fraud", "Someone charged my card and I never bought that."),
    ("EN-bug", "The app crashes when I open the settings menu."),
    ("EN-question", "How do I export my data to a CSV?"),
    ("EN-billing", "I want to cancel my subscription starting next month."),
    ("FR-fraud", "J'ai été débité de 89 € sans autorisation, merci de bloquer."),
]


def cosine(a: np.ndarray, b: np.ndarray) -> float:
    na = float(np.linalg.norm(a))
    nb = float(np.linalg.norm(b))
    if na < 1e-8 or nb < 1e-8:
        return 0.0
    return float(np.dot(a, b) / (na * nb))


def main() -> int:
    catalog_path = Path(__file__).resolve().parents[3] / "catalogs" / "support_triage.yaml"
    emb = LocalSTEmbedder(EMBEDDER_MODEL)
    catalog = AnchorCatalog.load(catalog_path, embedder=emb)
    client = EfeClient.from_env()

    print("=== ANCHOR COSINE SIMILARITY ===")
    print("(query embedding vs each anchor)")
    print()
    print(f"{'tag':<13}  ", end="")
    for c in catalog.classes:
        print(f"{c.name:>10}  ", end="")
    print(" predicted (server)")
    print("-" * 80)

    try:
        for tag, text in INPUTS:
            # Embed locally to compare to anchors
            qvec = emb.encode(text)
            cos_to_anchor = []
            for ac in catalog.anchors:
                cos_to_anchor.append(cosine(qvec, np.asarray(ac, dtype=np.float32)))

            # Server-side classification
            r = client.classify(text, catalog)
            print(f"{tag:<13}  ", end="")
            best_idx = int(np.argmax(cos_to_anchor))
            for i, c in enumerate(catalog.classes):
                marker = "*" if i == best_idx else " "
                print(f"{cos_to_anchor[i]:9.4f}{marker} ", end="")
            print(f"  → {r.label} (conf={r.confidence:.3f})")

        print()
        print("Legend: * = nearest anchor by raw cosine (no swarm)")
        print()
        print("=== CENTROID FROM SERVER ===")
        # Re-run one query and inspect the centroid the server returned (via raw RPC)
        text = INPUTS[0][1]
        ev = client.classify(text, catalog)
        print(f"Query: {text!r}")
        print(f"Server label: {ev.label}, confidence: {ev.confidence:.3f}, reject: {ev.reject}")
        if hasattr(ev, "centroid_norm"):
            print(f"Centroid norm: {ev.centroid_norm}")
        if hasattr(ev, "alignment_delta"):
            print(f"Alignment delta: {ev.alignment_delta}")

    finally:
        client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
