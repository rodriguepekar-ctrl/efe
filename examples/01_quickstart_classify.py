"""Sprint D10 — example 1: quickstart classifier (≤ 30 LOC).

Goal: first classification in ~30 seconds. Uses the bundled
``support_triage`` catalog (4 intents: fraud / bug / question /
billing) and demonstrates the canonical ``EfeClient.classify``
happy path.

Pre-requisites:
    * efe-server reachable on 127.0.0.1:50051
        (``helm install efe …`` + ``kubectl port-forward svc/efe 50051:50051``)
    * ``pip install efe-tool==0.1.10``

Run:
    python examples/01_quickstart_classify.py

Expected output (3 cases out of N classified):
    Someone charged my card without my consent.            → fraud        (0.92)
    Where do I find a record of my past invoices?          → question     (0.88)
    L'application plante quand j'ouvre le menu paramètres. → bug          (0.91)

Next steps: see examples/02_custom_catalog.py to define your own
intents instead of the bundled support_triage.
"""

# --8<-- [start:imports]
from pathlib import Path

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder
# --8<-- [end:imports]

# Five mixed-language support tickets to classify.
CASES = [
    "Someone charged my card without my consent.",
    "Where do I find a record of my past invoices?",
    "L'application plante quand j'ouvre le menu paramètres.",
    "How do I reset my two-factor authentication?",
    "I was charged twice for the same order.",
]

# Catalog ships with the SDK ; partners override with their own YAML.
CATALOG_PATH = Path(__file__).resolve().parent.parent / "catalogs" / "support_triage.yaml"


# --8<-- [start:classify-loop]
def main() -> None:
    embedder = LocalSTEmbedder.from_env()
    catalog = AnchorCatalog.load(CATALOG_PATH, embedder=embedder)

    with EfeClient(endpoint="127.0.0.1:50051", embedder=embedder, insecure=True) as client:
        for text in CASES:
            result = client.classify(text, catalog)
            label = result.label if not result.reject else "uncertain"
            print(
                f"  {text[:55]:55s} → "
                f"{label:10s} ({result.confidence:.2f})"
            )
# --8<-- [end:classify-loop]


if __name__ == "__main__":
    main()
