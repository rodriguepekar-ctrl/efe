# The hybrid (Plan B) decoder

## Background — D7 calibration plateau

Sprint D7's β/α calibration sweep converged at the empirical
winner `(β₀=0.45, λ_β=0.08, α₀=0.25)` with aggregate macro-F1 =
0.815 across three synthetic catalogs. (The analytical pre-sweep
starting point was `(0.45, 0.05, 0.30)`; the sweep improved the
tuple by ~0.0254 aggregate_macro_f1.) The catalog `billing_escalation`
was the bottleneck at macro-F1 = 0.6548 — an irreducible residual
on the structurally adjacent `question` vs `billing` pair, since
both classes embed near each other on financial-language inputs.

Mathematical β/α tuning could not close the gap; we needed an
algorithmic change. Plan B was that change.

## The idea

Run TWO independent classifiers on the same input:

1. **Raw cosine** — direct cosine similarity between the input
   embedding and each class's anchor centroid. ~10 ms (one
   embedder forward pass + N cosines). High recall on
   *easy* cases, fragile on adjacent-class cases.

2. **Swarm centroid** — the standard EFE Mode-1 fusion result.
   ~150-300 ms. Robust to wording variation, sometimes drifts on
   adjacent classes (the D7 residual).

Combine them with a 4-branch decision tree:

```
  Both classifiers agree on label
    AND swarm confidence ≥ agree_threshold (0.45)
  └── return that label, confidence = swarm's

  Disagree
    AND swarm confidence ≥ override_threshold (0.85)
  └── return swarm's label, confidence = swarm's
      (the swarm is very sure of a different label;
       trust the multi-round consensus)

  Disagree
    AND swarm confidence < override_threshold
  └── return raw cosine's label, confidence = cosine's
      (fall back to the simpler signal that
       isn't dominated by adjacent-class drift)

  Both agree, low swarm confidence
  └── reject (= "I don't know")
```

## Empirical impact (D8.1 measurement)

On the 3 synthetic catalogs (100 cases total):

| Catalog | Raw cosine F1 | Swarm-only F1 | Hybrid F1 | Δ vs swarm |
|---------|---------------:|---------------:|-----------:|-----------:|
| `support_triage` | 0.81 | 0.876 | 0.91 | +0.034 |
| `fraud_alert` | 0.84 | 0.882 | 0.90 | +0.018 |
| `billing_escalation` | 0.62 | 0.6548 | **0.733** | **+0.078** |
| Aggregate | 0.76 | 0.815 | **0.857** | **+0.042** |

The hybrid doesn't outright dominate — it picks the right tool per
input. The big win is on `billing_escalation` where the swarm's
adjacent-class drift was the bottleneck.

## When to enable

Plan B is available via `efe_tool.integrations.HybridClassifier`,
opt-in. Use it when:

* You measure swarm-only macro-F1 < 0.80 on your catalog.
* The errors are concentrated on a small number of class pairs.
* Latency budget allows ~150 ms total (raw cosine + swarm).

Don't bother when swarm-only is already > 0.90 — the marginal
hybrid gain is < 1 pp at that level.

## Why it doesn't just dominate everywhere

The hybrid is sensitive to its two thresholds (`agree_threshold`,
`override_threshold`). On catalogs with very low swarm confidence
across the board, the override threshold of 0.85 is rarely met,
making the hybrid behave like raw cosine — losing the swarm's
robustness gains. Tune the thresholds per-catalog if you ship
hybrid in production.

## See also

* [D8 architecture →
  d8_hybrid_realdata_architecture.md](https://github.com/efe-ai/efe/blob/main/docs/bench/d8_hybrid_realdata_architecture.md)
* `efe_tool.integrations.hybrid_decoder` source code
* `examples/01_quickstart_classify.py` (sync, swarm-only)
