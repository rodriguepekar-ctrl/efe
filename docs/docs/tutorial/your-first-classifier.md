# Your first classifier

> Audience: you finished [Quickstart](quickstart.md). Now you want
> to classify *your own* labels — not the bundled `support_triage`.
> Time budget: 15 minutes.

---

## The 4-step workflow

1. **Sketch your intents** — list the 3-10 distinct labels you
   want, with 2-4 example prompts each.
2. **Author a catalog YAML**.
3. **Load + classify** — `efe_tool` does the embedding + swarm RPC.
4. **Iterate** — measure + tweak anchor prompts when a query goes
   to the wrong class.

The reference implementation is
`examples/02_custom_catalog.py`. Read it once, then come back here
for the design tips.

---

## Step 1 — sketch the intents

Pick labels that are:

* **Mutually exclusive** — a query should belong to exactly one.
* **Linguistically diverse** — write 2-3 prompts per language you
  expect; the embedder handles 50+.
* **Concrete, not abstract** — "track_order" beats "logistics".

A typical e-commerce catalog:

```text
track_order:
  - Where is my order?
  - Quand vais-je recevoir mon colis ?

request_refund:
  - I'd like to return this and get a refund.
  - Je veux annuler ma commande.

product_question:
  - Is this compatible with my model X?
  - Quelle est la garantie ?

complaint:
  - This is unacceptable, I want to speak to a manager.
  - Votre service client est une catastrophe.
```

---

## Step 2 — author the YAML

The minimal schema:

```yaml
apiVersion: efe.mode1/v1
kind: AnchorCatalog
metadata:
  name: ecommerce_postsale
  version: "0.1.0"

embedder:
  family: sentence-transformers
  model: paraphrase-multilingual-MiniLM-L12-v2
  dim: 384
  normalize: l2

cone_policy:
  max_projection_distance: 0.5

thresholds:
  confidence_high: 0.60
  confidence_medium: 0.50

classes:
  - name: track_order
    description: "User asks about delivery / shipping."
    canonical_sentences:    # NOT "anchors" — see reference/catalog-schema.md
      - "Where is my order?"
      - "Quand vais-je recevoir mon colis ?"
  - name: request_refund
    ...
```

Full schema in [reference/catalog-schema](../reference/catalog-schema.md).

The complete `examples/02_custom_catalog.py` materialises this
YAML programmatically (good for one-shot demos) — for production,
**commit the YAML to git**.

---

## Step 3 — classify

```python
from pathlib import Path
from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder

embedder = LocalSTEmbedder.from_env()
catalog = AnchorCatalog.load(
    Path("catalogs/ecommerce_postsale.yaml"),
    embedder=embedder,
)

with EfeClient(endpoint="127.0.0.1:50051", embedder=embedder, insecure=True) as client:
    r = client.classify("My package is 5 days late.", catalog)
    print(r.label, r.confidence, r.reject)
```

The full self-contained example with multilingual queries is in
`examples/02_custom_catalog.py`.

---

## Step 4 — iterate

Once you've classified ~100 real customer queries, do a confusion
analysis:

```bash
# (Hand-tally for now ; D11 will ship a /confusion endpoint.)
$ python examples/02_custom_catalog.py | tee /tmp/labels.tsv
$ # ...annotate /tmp/labels.tsv with the *expected* label
$ awk -F'\t' '$1!=$2' /tmp/labels.tsv | wc -l   # count errors
```

For each misclassification, ask yourself:

* **Is the query genuinely ambiguous?** Real-world data is messy
  — accept some errors.
* **Is the anchor for the correct class missing a key phrase?**
  Add a new anchor prompt that closely matches the failing query.
* **Are two classes too close?** Merge them, or refine the
  descriptions to make the boundary clearer.

The iteration loop is fast: edit YAML, re-`AnchorCatalog.load(...)`,
re-classify. No model retraining. Most catalogs reach > 0.90
macro-F1 on real data within 2-3 iteration rounds.

---

## Tips

* **Never go below 2 anchors per class.** A single anchor is
  fragile — a typo or weird wording in the user query can flip
  the cosine sign.
* **Anchors should be ~5–20 words** each. Single words ("refund")
  embed as a less specific point than full phrases
  ("I want to return this and get a refund.").
* **Mix languages** in the same catalog if your traffic is
  multilingual — one class can have FR + EN + DE prompts, and the
  embedder maps them close together.
* **Start with high-coverage classes**, add `other` / `general`
  fallback if needed (with `reject_band` tuned high so it only
  triggers when nothing fits).

---

## Next

* **[How-to: custom catalog](../howto/custom-catalog.md)** — the
  YAML schema in full.
* **[Explanation: why anchors](../explanation/why-anchors.md)** —
  design rationale (zero-shot vs fine-tuning).
* **[Reference: API](../reference/api.md)** — full SDK API.
