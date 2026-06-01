# Why anchors? (vs fine-tuning)

## The classification problem at the partner pilot stage

A typical partner has:

* 5–30 distinct intents to route messages to.
* 0 to a-few-thousand labelled examples — usually inconsistent
  across labelling rounds.
* A multi-language inbound stream.
* Need to ship in days, not months.

The classical ML answer is "fine-tune a transformer on labelled
examples". That requires:

* Coherent labelling (impossible at zero examples).
* Per-language training data or a multilingual pre-trained model.
* Compute infrastructure for fine-tuning.
* Re-deploy + re-validate when labels change.

EFE's anchor-based approach trades model expressiveness for
**zero-training-time iteration speed**. Instead of fine-tuning on
labelled examples, you write 2-4 *anchor prompts* per class — a
short example of what each class looks like. The classifier:

1. Embeds the anchors with a frozen multilingual model.
2. Embeds the user query.
3. Routes via cosine similarity, refined by the swarm fusion
   (which improves robustness vs raw cosine on adjacent classes).

## Why this beats raw nearest-neighbour

Raw cosine on a single anchor per class is fragile. EFE adds two
robustness layers:

* **Multiple anchors per class** form a small cluster — a query
  is matched to whichever cluster's centroid it's closest to
  (in projected latent space).
* **Swarm fusion** runs 4-7 rounds of consensus on the embedded
  query, producing a centroid that's MORE robust to wording
  variations than a single forward pass.

In our internal benchmarks (D7 calibration sweep), the swarm path
adds 5-12 percentage points to macro-F1 over raw cosine on
adjacent-class pairs (`question` vs `billing`, `track_order` vs
`product_question`).

## Why this beats fine-tuning at the pilot stage

* **Iteration speed**: edit YAML → reload → classify. Seconds, not
  hours.
* **Zero compute footprint**: no GPU for training. Inference on
  CPU is also acceptable for < 50 RPS.
* **Multilingual for free**: one catalog, one embedder, 50+
  languages.

## When fine-tuning beats EFE

Once a partner reaches:

* > 200 distinct labels (anchor authoring becomes the bottleneck).
* > 10 000 high-quality labelled examples (fine-tuning extracts
  more signal than 4 anchors / class).
* Need for sub-50ms latency at high RPS (specialised classifier
  beats embedder + swarm).

…then a fine-tuned classifier wins. EFE is a pilot-stage and
mid-scale tool, not a replacement for ML training pipelines at
scale.

## Trade-offs vs LLM zero-shot

A modern LLM ("classify this into [fraud, billing, question, bug]")
also works zero-shot. EFE's advantages over LLM-as-classifier:

* **Cost per call** — sentence-transformers embedding + Mode-1
  fusion costs ~$10⁻⁶ vs LLM ~$10⁻³.
* **Latency** — 50-200ms vs 500-2000ms for an LLM API call.
* **Calibrated confidence** — EFE returns [0,1] confidence ; LLMs
  give text-based probabilities that are notoriously poorly
  calibrated.
* **No data leaves your perimeter** — embedder + EFE server run on
  your infrastructure ; LLM calls hit a third-party API.

LLM-as-classifier wins when:

* You need free-text reasoning (not just label classification).
* The task is too rare/niche for any embedder to capture
  (specialised legal terms, code).

## See also

* [Explanation — Hybrid (Plan B)](hybrid.md): how raw cosine and
  swarm get combined.
* [Explanation — Trade-offs](tradeoffs.md): what EFE explicitly is NOT.
