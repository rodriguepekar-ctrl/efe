# Concepts

> Audience: you've run [Quickstart](quickstart.md), now you want to
> understand what just happened.

---

## The 30-second mental model

EFE is a **zero-shot text classifier** that doesn't require model
fine-tuning. You define labels by giving 2–4 example prompts each;
the classifier embeds those into a fixed vector space (the "anchor
cone") and routes new inputs to whichever anchor cluster they
project closest to.

The "ephemeral fluid swarm" part: each classification spins up a
small population of compute agents that converge on a centroid via
4–7 rounds of fusion. The result is **more robust to wording
variation** than a single nearest-neighbour search, especially on
short / noisy / multilingual inputs.

Key properties:

* **Zero training time** — drop a new catalog YAML, reload, classify.
* **Multilingual** — the bundled embedder
  (`paraphrase-multilingual-MiniLM-L12-v2`) covers 50+ languages
  in the same vector space.
* **Per-request cost: ~50–200 ms** depending on text length and
  embedder warm-up state.
* **Calibrated confidence** in [0, 1].

---

## Anchors and the alignment cone

A **catalog** is a flat YAML listing of `(label, anchor prompts)`
tuples. Each anchor prompt is a short, typical example of that
intent. Multi-language is encouraged: the embedder maps EN/FR/DE/ES
prompts close together if their meaning is the same.

```yaml
# catalogs/your_domain.yaml — minimal example
classes:
  - name: track_order
    canonical_sentences:
      - "Where is my order?"
      - "Quand vais-je recevoir mon colis ?"
  - name: request_refund
    canonical_sentences:
      - "I'd like a refund"
      - "Je veux être remboursé"
```

At catalog load-time, the SDK calls the embedder once per anchor.
The resulting vectors form an **alignment cone** — a constrained
region in latent space that the swarm's convergence centroid must
project onto.

A user query gets:

1. Embedded by the same embedder.
2. Sent to `efe-server` as part of an `ExecuteRequest`.
3. Fused over multiple swarm rounds.
4. Projected onto the cone.
5. Compared (cosine) against each anchor → argmax → label.

---

## Confidence and rejection

The result includes:

* `label` — the chosen class name.
* `confidence` — swarm convergence confidence in [0, 1].
* `reject` — boolean ; `True` when either:
  - confidence is below the catalog's threshold, OR
  - the projection distance from cone is too high (the input is
    semantically outside the catalog's coverage).

Production code should treat `reject=True` as "ask a human" or
"fall back to default", not as a label.

---

## The hybrid (Plan B) decoder

The default path runs the full Mode-1 swarm fusion. Some classes
are sufficiently easy that the **raw cosine** (embedder
similarity, no swarm) is already accurate. EFE ships a
`HybridClassifier` that combines both signals:

* **Agree branch**: raw cosine and swarm pick the same label →
  return that with the swarm's (typically higher) confidence.
* **Disagree branch**: depending on each branch's confidence,
  pick the more confident one.
* **Reject branch**: both disagree and both have low confidence
  → reject.

In our internal Sprint D8 measurement on three synthetic
catalogs, the hybrid path lifted aggregate macro-F1 from 0.815 to
0.857 (+0.042), with the largest gain on a notoriously difficult
catalog (`billing_escalation`: +0.078).

[Read more →](../explanation/hybrid.md)

---

## When NOT to use EFE

EFE is strong at:

* Routing inbound messages to a small set of intents (≤ 50 classes).
* Multi-lingual classification with no training data.
* Short / medium-length inputs (a few sentences).

EFE is not the right tool for:

* Document classification > 5 KB.
* Generative tasks (write a response, summarise, translate).
* Fine-grained taxonomies with > 200 labels (per-class anchor
  authoring becomes the bottleneck).
* Tasks where you have abundant labelled data — fine-tune a
  classifier instead, you'll get higher accuracy.

[Read more →](../explanation/tradeoffs.md)

---

## Next

* **[Your first classifier](your-first-classifier.md)** — guided
  walkthrough authoring a 4-class catalog.
* **[Explanation: architecture](../explanation/architecture.md)**
  — the deeper design.
