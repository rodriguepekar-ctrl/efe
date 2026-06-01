# Reference — Catalog schema

The full schema for an `AnchorCatalog` YAML.

## Top-level fields

```yaml
apiVersion: efe.mode1/v1   # required, exact value
kind: AnchorCatalog        # required, exact value
metadata: {...}            # required
embedder: {...}            # required
cone_policy: {...}         # required
thresholds: {...}          # required
classes: [...]             # required, len ≥ 2
```

## `metadata`

```yaml
metadata:
  name: my_catalog          # required, [a-z][a-z0-9_]*
  version: "0.1.0"          # required, SemVer
  created_at: 2026-04-27T12:00:00Z  # optional
  author: you@example.com   # optional
  description: ""           # optional
```

## `embedder`

```yaml
embedder:
  family: sentence-transformers           # required (only value supported in v0.1.x)
  model: paraphrase-multilingual-MiniLM-L12-v2   # required
  dim: 384                                # required, must match the model's output dim
  normalize: l2                           # required, must be `l2`
```

## `cone_policy`

```yaml
cone_policy:
  max_projection_distance: 0.5    # required, in [0.0, 2.0]
  registered_in_m9: false          # optional, defaults false
```

## `thresholds`

The MVP schema is **flat** (no per-class overrides yet — those
are tracked for sprint D12). Two confidence cutoffs partition the
output of the swarm fusion into ``trusted`` / ``warn`` / ``reject``
zones.

```yaml
thresholds:
  confidence_high: 0.60      # ≥ → ClassifyResult.reject = False, label trusted
  confidence_medium: 0.50    # [medium, high) → label with warning
                             # < confidence_medium → ClassifyResult.reject = True
```

Defaults if absent: ``confidence_high=0.85``, ``confidence_medium=0.65``.

## `classes` (list)

```yaml
classes:
  - name: fraud                      # required, [a-z][a-z0-9_]*
    description: "..."               # optional
    canonical_sentences:             # required, len ≥ 1
      - "Someone charged my card."
      - "Quelqu'un a piraté mon compte."
    color: red                       # optional, UI hint
    deprecated: false                # optional, default false
```

> ⚠ The field name is ``canonical_sentences``, NOT ``anchors``.
> The bundled ``catalogs/support_triage.yaml`` is the canonical
> reference.

## Validation

`AnchorCatalog.load(path, embedder=...)` performs:

1. YAML schema validation (raises `CatalogError` on violation).
2. Embedder dim check (`len(embed("test")) == catalog.embedder.dim`).
3. Inter-class similarity: each pair of class centroids must be at
   cosine distance > `0.2` ; otherwise the classifier cannot
   distinguish them reliably.
4. Anchor cache lookup (`<dir>/.cache/<sha256>.npz`) ; computes if absent.

## Versioning

Bump `metadata.version` whenever the anchor list changes — it's
part of the gRPC idempotency key, so cached results are invalidated
correctly.
