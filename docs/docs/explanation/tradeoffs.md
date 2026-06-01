# Trade-offs — what EFE is NOT

EFE optimises for **fast partner pilot iteration on multilingual
text classification**. That's a specific niche. Be honest about
where it fails:

## EFE is NOT

* **A summariser** — it produces labels, not text.
* **A generator** — no free-form output.
* **A translator** — uses translation as a side-effect of the
  multilingual embedder, but doesn't expose translation as an
  API.
* **A search engine** — vector similarity at classification
  granularity, not document retrieval.
* **A semantic agent** — Mode-1 is fixed-shape; complex multi-step
  reasoning needs an LLM upstream.
* **A drop-in replacement for fine-tuned classifiers at scale** —
  see [why-anchors](why-anchors.md).

## Specific EFE limitations

| Limitation | Workaround |
|-----------|-----------|
| Hard ceiling on macro-F1 (~0.92 typical) | Add more anchors per class; if still insufficient → fine-tune |
| Per-call latency 50–300 ms | Use HPA + headless Service; for sub-50ms latency look elsewhere |
| Anchor authoring effort grows linearly with classes | OK up to ~50 classes; beyond → fine-tune |
| Hybrid Plan B threshold tuning is manual | D11+ sprint will ship auto-tuning |
| No streaming `Execute` (Mode-1 is stateful per call) | Letter B follow-up |
| One catalog per RPC (no multi-catalog routing) | D12 admin endpoints |

## When to choose something else

* **Fine-tuned BERT/DeBERTa** — you have ≥10k labelled examples
  and need accuracy maximised.
* **GPT-4 / Claude as classifier** — task requires reasoning,
  not just labelling. Costs ~10³× more per call.
* **Lucene/Elastic** — task is retrieval, not classification.
* **Regex / rule engine** — task is fully deterministic
  (e.g. ID format validation).

## When EFE is the right tool

* Pilot stage — < 1k labelled examples, but 5-30 distinct
  intents.
* Multilingual inbound stream.
* Need to iterate on labels weekly without retraining.
* Privacy constraint — no third-party LLM call.

## See also

* [Why anchors](why-anchors.md)
* [Architecture](architecture.md)
