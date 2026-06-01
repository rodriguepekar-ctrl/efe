"""Isolate which input causes the alignment breach.

Tries each input independently, catches errors, reports per-input outcome.
"""
from __future__ import annotations

import sys
from pathlib import Path

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder
from efe_tool.errors import BridgeError

EMBEDDER_MODEL = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"

EXAMPLES = [
    ("EN-fraud", "Someone charged my card and I never bought that.", "fraud"),
    ("EN-bug", "The app crashes when I open the settings menu.", "bug"),
    ("EN-question", "How do I export my data to a CSV?", "question"),
    ("EN-billing", "I want to cancel my subscription starting next month.", "billing"),
    ("FR-fraud", "J'ai été débité de 89 € sans autorisation, merci de bloquer.", "fraud"),
    ("FR-bug", "L'application affiche un écran blanc après le dernier update.", "bug"),
    ("FR-question", "Comment modifier mon adresse de livraison par défaut ?", "question"),
    ("FR-billing", "Pouvez-vous m'envoyer une facture avec numéro TVA ?", "billing"),
    ("ADV-weather", "It's raining hard today, the weather is beautiful, no support question.", None),
]


def main() -> int:
    catalog_path = Path(__file__).resolve().parents[3] / "catalogs" / "support_triage.yaml"
    emb = LocalSTEmbedder(EMBEDDER_MODEL)
    catalog = AnchorCatalog.load(catalog_path, embedder=emb)
    client = EfeClient.from_env()
    try:
        for tag, text, expected in EXAMPLES:
            try:
                r = client.classify(text, catalog)
                ok = expected is None or r.label == expected
                print(f"[{tag:12}] {'OK' if ok else 'WRONG':5} → label={r.label:9} conf={r.confidence:5.3f} reject={r.reject!s:5} expected={expected or '-'}")
            except BridgeError as e:
                print(f"[{tag:12}] ERROR → {e}")
            except Exception as e:
                print(f"[{tag:12}] PANIC → {type(e).__name__}: {e}")
    finally:
        client.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
