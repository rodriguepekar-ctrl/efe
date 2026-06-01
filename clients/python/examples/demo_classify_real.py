"""Real practical EFE Mode-1 test — multilingual support triage classification.

Runs the same 9 examples as `demo_classify.py` but uses the multilingual
embedder explicitly, matching the catalog's compiled identity. This is
the canonical practical test for EFE.
"""
from __future__ import annotations

import sys
from pathlib import Path

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder

# Embedder identity locked to the catalog's compiled model.
# (catalogs/support_triage.yaml declares
#  embedder.model: paraphrase-multilingual-MiniLM-L12-v2.)
EMBEDDER_MODEL = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"

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

    print(f"loading embedder: {EMBEDDER_MODEL}")
    emb = LocalSTEmbedder(EMBEDDER_MODEL)
    catalog = AnchorCatalog.load(catalog_path, embedder=emb)
    print(f"loaded catalog '{catalog.name}@{catalog.version}' — {len(catalog.classes)} classes")
    print(f"classes: {', '.join(c.name for c in catalog.classes)}")
    print()

    client = EfeClient.from_env()
    correct = 0
    rejected_correctly = 0
    total = len(EXAMPLES)
    try:
        rows = []
        for text, expected in EXAMPLES:
            result = client.classify(text, catalog)
            ok = expected is None or result.label == expected
            # Adversarial case (expected=None) — pass if rejected
            adv_pass = expected is None and result.reject
            mark = "✓" if (ok or adv_pass) else "✗"
            if ok and expected is not None:
                correct += 1
            if adv_pass:
                rejected_correctly += 1
            rows.append((mark, result.label, result.confidence, result.reject, expected, text))

        # Print summary table
        print(f"{'':2}  {'label':<10}  {'conf':>6}  {'rej':<3}  {'exp':<10}  text")
        print("-" * 100)
        for mark, label, conf, reject, expected, text in rows:
            t = text[:50] + ("…" if len(text) > 50 else "")
            print(
                f"{mark:2}  {label:<10}  {conf:6.3f}  {'yes' if reject else ' no':<3}  "
                f"{(expected or '-'):<10}  {t}"
            )
        print()
        non_adv = total - sum(1 for _, e in EXAMPLES if e is None)
        adv = total - non_adv
        print(f"Accuracy on labeled cases : {correct}/{non_adv} = {100 * correct / non_adv:.1f}%")
        print(f"Adversarial rejected     : {rejected_correctly}/{adv}")

        stats = client.cache_stats()
        print(f"Cache: hits={stats['hits']} misses={stats['misses']} size={stats['size']}")
        return 0 if (correct == non_adv and rejected_correctly == adv) else 1
    finally:
        client.close()


if __name__ == "__main__":
    sys.exit(main())
