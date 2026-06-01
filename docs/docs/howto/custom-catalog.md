# How-to — Custom catalog

How to define your own intent labels and ship them as a YAML
catalog. The full schema reference is at
[reference/catalog-schema](../reference/catalog-schema.md).

The reference example is `examples/02_custom_catalog.py`.

## Minimal valid catalog

```yaml
apiVersion: efe.mode1/v1
kind: AnchorCatalog

metadata:
  name: my_catalog
  version: "0.1.0"

embedder:
  family: sentence-transformers
  model: paraphrase-multilingual-MiniLM-L12-v2
  dim: 384
  normalize: l2

cone_policy:
  max_projection_distance: 0.5

thresholds:
  # Flat schema (NOT nested under `default`).
  confidence_high: 0.60     # ≥ → trusted label
  confidence_medium: 0.50   # [medium, high) → label with warning
                            # < confidence_medium → reject

classes:
  - name: greeting
    description: "User greets the bot."
    canonical_sentences:    # IMPORTANT: not "anchors" — see schema reference.
      - "Hello"
      - "Bonjour"
      - "Hi there"
  - name: goodbye
    description: "User says goodbye."
    canonical_sentences:
      - "Bye"
      - "À bientôt"
      - "See you"
```

## Authoring tips

* **2-4 anchors per class.** Single anchors are fragile; > 4 is
  diminishing returns.
* **Cover language variants** — same meaning, multiple languages.
* **Anchors should be short, complete phrases**: 5-20 words. Single
  words encode less context than full sentences.
* **Test on real data immediately.** The fastest feedback loop
  is: edit YAML → reload → classify 50 cases → review confusions.

## Loading

```python
from efe_tool import AnchorCatalog, LocalSTEmbedder

embedder = LocalSTEmbedder.from_env()
catalog = AnchorCatalog.load("path/to/your.yaml", embedder=embedder)
```

`AnchorCatalog.load(...)` will:

1. Parse + validate the YAML schema.
2. Embed every anchor prompt with the configured embedder
   (cached locally — second load is < 100 ms).
3. Compute the alignment cone from the anchors.

The cached embeddings live in `<catalog_dir>/.cache/<sha256>.npz`
by default; pass `cache_dir=Path("/some/dir")` to change.

## Versioning catalogs

Bump `metadata.version` whenever the anchors change. The SDK uses
the version as part of the gRPC idempotency key, so a same-text
classification will route to the same cached server response only
when the catalog version matches.

## Per-class threshold overrides (roadmap)

The current MVP threshold schema is **flat** (one ``confidence_high``
+ one ``confidence_medium`` for the whole catalog). Per-class
overrides are tracked for D12 — when shipped, the syntax will be
documented here. For now, tighten the global thresholds and rely
on the catalog's anchor distinctness to drive per-class behaviour.

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `CatalogError: dim mismatch` | Catalog declares `dim: 384` but embedder returns 768 | Either retrain to match, or change the YAML's `embedder.model` |
| `CatalogError: inter-class similarity above threshold` | Two classes' anchor centroids are too close | Edit anchors so the classes are more distinguishable |
| Many `reject=True` results | `confidence_medium` too high | Lower it, or add more anchor variants |
