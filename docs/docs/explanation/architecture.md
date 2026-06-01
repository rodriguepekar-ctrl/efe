# Architecture

> High-level overview of how EFE works under the hood.
> Audience: SRE / architect evaluating EFE for production use.

---

## The component stack

```
┌─────────────────────────────────────────────────────────────┐
│   YOUR APPLICATION                                           │
│   ┌──────────────────────────────┐                           │
│   │ efe_tool.EfeClient (Python)  │                           │
│   └────┬────────────────┬────────┘                           │
│        │ embed locally  │ gRPC ExecuteRequest                 │
│        ▼                ▼                                     │
│   ┌─────────────────┐  ┌────────────────────────────┐        │
│   │ Local embedder  │  │ efe-server (Rust)          │        │
│   │ (sentence-      │  │ ┌───────────────────────┐  │        │
│   │  transformers)  │  │ │ Mode-1 fusion engine  │  │        │
│   └─────────────────┘  │ │ (4-7 round swarm)     │  │        │
│                         │ └───────────────────────┘  │        │
│                         │ ┌───────────────────────┐  │        │
│                         │ │ Cone projection       │  │        │
│                         │ └───────────────────────┘  │        │
│                         │ ┌───────────────────────┐  │        │
│                         │ │ Anchor argmax → label │  │        │
│                         │ └───────────────────────┘  │        │
│                         └────────────────────────────┘        │
└─────────────────────────────────────────────────────────────┘
```

## Per-request flow

1. **Client side**: SDK loads the catalog (cached embeddings),
   embeds the user query via the local sentence-transformers
   model.
2. **gRPC**: SDK sends `ExecuteRequest{spec, embedding}` to
   `efe.v1.SwarmService/Execute`.
3. **Server side — Mode-1**:
   - Initial centroid = the embedded query.
   - For 4-7 rounds: fuse the centroid with the alignment cone's
     anchors using the calibrated β/α coefficients.
   - At each round, compute convergence (variance, gradient).
4. **Project** the final centroid onto the cone (constrained
   region defined by anchors).
5. **Argmax** cosine similarity vs each anchor → label.
6. **Return** `ExecuteResponse{label, confidence, rounds, ...}`.

The swarm fusion is what makes EFE robust to wording variation
that simple cosine can't handle. The cone projection enforces a
"semantic prior" that prevents drift on out-of-distribution
inputs (the `reject=True` path).

## Why a separate server?

* **Stateless deploy** — `efe-server` is a stateless gRPC service.
  Scales horizontally via Helm chart's HPA.
* **Latency-critical isolation** — the Rust binary keeps Mode-1
  in tight inner loops; Python clients get a clean RPC surface.
* **Multi-language clients** — D10 ships Python; D15+ will add
  Go and TypeScript clients against the same gRPC contract.

## Calibration

The fusion engine has 4 hyperparameters (β₀, λ_β, α₀, τ) that
trade off between "input dominance" (β) and "anchor dominance"
(α) over the rounds. D7 ran a sweep and gelled the production
defaults; partners can override via `EFE_FUSION_*` env vars on
the server.

## The hybrid path (Plan B)

When swarm-only macro-F1 is < 0.80, the SDK can fall back to
`HybridClassifier` which combines raw cosine + swarm with a
4-branch decision tree. See [Hybrid](hybrid.md).

## What about training?

EFE has **zero training**. The embedder is frozen
(`paraphrase-multilingual-MiniLM-L12-v2`); the fusion engine has
no learned parameters. Only the catalog YAML — anchor prompts —
is partner-authored.

This is by design: it makes deployment a YAML-edit instead of a
model-fine-tune cycle. The cost is that EFE has a hard ceiling
on accuracy compared to a fine-tuned classifier with abundant
labelled data.

## See also

* [Why anchors](why-anchors.md) — design rationale.
* [Trade-offs](tradeoffs.md) — what EFE is and isn't.
* [Hybrid](hybrid.md) — when to use Plan B.
* `MODE1_ARCHITECTURE.md` (in repo root) — the full design doc.
