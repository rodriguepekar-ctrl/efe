"""Sprint D10 — example 2: custom anchor catalog.

Goal: define your own intents (e-commerce post-sale support) and
classify against them. Demonstrates the "partner adapts EFE to
their domain" path:

  1. Author intent labels + 2-3 example prompts each (per language).
  2. Materialise as a YAML catalog that the SDK can load.
  3. Use the resulting catalog with EfeClient.classify().

The YAML is written to a tmp dir on each run — adapt the
``DOMAIN_INTENTS`` dict and the path below to ship a permanent
catalog with your application.

Pre-requisites:
    * efe-server reachable on 127.0.0.1:50051
    * pip install efe-tool==0.1.10

Run:
    python examples/02_custom_catalog.py
"""

from __future__ import annotations

import tempfile
from pathlib import Path

import yaml

from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder


# 1. Define domain intents — each class needs ≥ 2 example prompts.
#    Mix languages freely : paraphrase-multilingual-MiniLM-L12-v2
#    embeds across 50+ languages in the same vector space.
DOMAIN_INTENTS: dict[str, list[str]] = {
    "track_order": [
        "Where is my order?",
        "What's the shipping status of order ABC123?",
        "Quand vais-je recevoir mon colis ?",
        "Mein Paket ist noch nicht angekommen.",
    ],
    "request_refund": [
        "I'd like to return this and get a refund.",
        "How do I get my money back for the broken product?",
        "Je veux annuler ma commande et être remboursé.",
        "Quiero devolver el artículo, está roto.",
    ],
    "product_question": [
        "What are the dimensions of this item?",
        "Is this product compatible with my model X?",
        "Quelle est la garantie sur cet article ?",
        "Welches Material ist das?",
    ],
    "complaint": [
        "This is unacceptable, I want to speak to a manager.",
        "The product arrived damaged and you're not responding.",
        "Votre service client est une catastrophe.",
        "Atención al cliente terrible, llevo 3 días esperando.",
    ],
}

# Real customer queries to classify (mix of EN/FR/DE/ES).
CASES: list[str] = [
    "My package hasn't arrived after 2 weeks.",
    "Can I return this if I changed my mind?",
    "Does this come with batteries included?",
    "I've been waiting 3 days for a reply, this is ridiculous.",
    "Le produit est cassé, comment je le renvoie ?",
    "¿Cómo rastreo mi envío?",
]


def materialise_catalog(
    intents: dict[str, list[str]],
    out_path: Path,
    embedder_model: str,
    embedder_dim: int,
) -> None:
    """Write a minimal AnchorCatalog YAML from a flat ``intents`` dict.

    Real partners will commit the YAML to git ; here we tmpfile it
    so the example is self-contained.

    Schema notes (audit fix E4):
        * ``classes[].canonical_sentences`` is the field name (not
          ``anchors``) — see the bundled
          ``catalogs/support_triage.yaml`` for the canonical schema.
        * ``thresholds`` is FLAT with two keys ``confidence_high``
          and ``confidence_medium`` (not nested under ``default``).
        * The embedder model + dim MUST match the embedder the
          caller will use at classify time — passed in here so the
          example respects whatever the env-driven default produces
          (``EFE_EMBEDDER_MODEL`` honoured by ``LocalSTEmbedder.from_env``).
    """
    classes = [
        {
            "name": label,
            "description": f"User intent: {label}",
            "canonical_sentences": prompts,
        }
        for label, prompts in intents.items()
    ]
    doc = {
        "apiVersion": "efe.mode1/v1",
        "kind": "AnchorCatalog",
        "metadata": {
            "name": "ecommerce_postsale",
            "version": "0.1.0",
            "description": "Custom e-commerce post-sale support classifier",
        },
        "embedder": {
            "family": "sentence-transformers",
            "model": embedder_model,
            "dim": embedder_dim,
            "normalize": "l2",
        },
        "cone_policy": {
            "max_projection_distance": 0.5,
        },
        "thresholds": {
            "confidence_high": 0.60,    # ≥ → trusted label
            "confidence_medium": 0.50,  # [medium, high) → label with warning
            # < confidence_medium → reject (LLM asks a clarifying question)
        },
        "classes": classes,
    }
    out_path.write_text(yaml.safe_dump(doc, allow_unicode=True, sort_keys=False))


def main() -> None:
    # Build the embedder first so the catalog's declared model
    # matches what we'll classify against at runtime (audit fix E4).
    # `LocalSTEmbedder.from_env()` honours ``EFE_EMBEDDER_MODEL`` ;
    # the catalog's YAML mirrors whatever that resolves to so the
    # fingerprint check at classify time succeeds.
    embedder = LocalSTEmbedder.from_env()
    fp = embedder.fingerprint()
    # fp is "sentence-transformers/<model>@<dim>" — split it out.
    model_short = fp.split("/", 1)[1].split("@", 1)[0]
    dim = embedder.dim()

    print("[1/3] Materialising custom catalog YAML…")
    with tempfile.TemporaryDirectory() as tmpdir:
        catalog_path = Path(tmpdir) / "ecommerce_postsale.yaml"
        materialise_catalog(
            DOMAIN_INTENTS, catalog_path,
            embedder_model=model_short,
            embedder_dim=dim,
        )
        print(f"  → {catalog_path}")
        print(f"  → embedder: {fp}")

        print("[2/3] Compiling catalog (embedding the anchor prompts)…")
        catalog = AnchorCatalog.load(catalog_path, embedder=embedder)
        # `canonical_sentences` is the YAML field, but the loader
        # exposes it as `ClassDef.canonical_sentences` on the in-memory
        # object. Count for diagnostics.
        n_anchors = sum(len(c.canonical_sentences) for c in catalog.classes)
        print(f"  → {len(catalog.classes)} classes, {n_anchors} anchor prompts")

        print("[3/3] Classifying customer queries…")
        with EfeClient(
            endpoint="127.0.0.1:50051", embedder=embedder, insecure=True,
        ) as client:
            for text in CASES:
                r = client.classify(text, catalog)
                label = r.label if not r.reject else "UNKNOWN"
                print(f"  {text[:60]:60s} → {label:18s} ({r.confidence:.2f})")


if __name__ == "__main__":
    main()
