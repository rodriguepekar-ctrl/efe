# How-to — catalog evolution

> Sprint **D12** (v0.1.12). The 5 `EvolveCatalog` ops are the only
> way to mutate a server-registered catalog. Catalogs are
> immutable post-register : every change produces a new version
> with a fresh integer (incremented +1 from the source) and a
> `parent_key` pointing back to the source.

For a quick start, see [catalog admin](./catalog-admin.md).

---

## Why "evolution" rather than "edit"

Two design choices drive the shape of this API :

1. **Forensic-grade audit** : every change records a `parent_key`
   so you can replay the chain. Editing in place destroys that
   history.
2. **Concurrent classifiers** : a classifier in production may be
   pinned to `acme/billing@v3`. If `v3` were mutable, that pin
   would be meaningless — the classifier could see different
   anchors on consecutive calls. Versioning by construction means
   a pin is a content guarantee.

This is the same model as Kubernetes ConfigMaps with
`immutable: true`, except evolution comes for free.

---

## The 5 ops

| Op                            | Cost (server-side compile)         | Atomicity                  |
|-------------------------------|------------------------------------|----------------------------|
| `APPEND_CLASS`                | embed the new class only           | atomic                     |
| `REMOVE_CLASS`                | no recompile (just drop)           | atomic                     |
| `UPDATE_THRESHOLDS`           | no recompile                       | atomic                     |
| `REPLACE_CLASS_ANCHORS`       | embed only the named class         | atomic                     |
| `RECOMPILE`                   | embed every class with new model   | atomic, may run for ~5 min |

`RECOMPILE` is the only op that can be slow. The server returns
quickly (the new version is "pending" until the embed finishes) ;
classifications against the new key block until ready (and time
out at the per-call deadline). For predictable latency, keep
`RECOMPILE` in scheduled maintenance windows.

---

## APPEND_CLASS — add one class

Adds a single class to the existing set. The new class must :

* Have a `name` not already present in the source.
* Have a non-empty `canonical_sentences` list.
* Carry the same embedder fingerprint (no implicit recompile —
  use `RECOMPILE` if you want a different embedder).

Python :

```python
from efe_tool import EfeAdminClient, EvolveOp

with EfeAdminClient(endpoint="...", jwt="...") as admin:
    v2 = admin.evolve_catalog(
        from_key="acme/support_triage@v1",
        op=EvolveOp.APPEND_CLASS,
        op_payload={
            "cls": {
                "name": "churn_signal",
                "description": "User signalling intent to leave.",
                "canonical_sentences": [
                    "I am thinking about cancelling",
                    "switching to a competitor next month",
                ],
                "color": "#f59e0b",
                "deprecated": False,
            },
        },
        description="Add churn_signal class for outreach automation.",
    )
    print(f"new version: {v2.key} (parent: {v2.metadata.parent_key})")
```

CLI — payload from a YAML file :

```yaml
# ops/append_churn.yaml
cls:
  name: churn_signal
  description: User signalling intent to leave.
  canonical_sentences:
    - I am thinking about cancelling
    - switching to a competitor next month
  color: "#f59e0b"
  deprecated: false
```

```bash
efe-admin catalog evolve \
    --from-key acme/support_triage@v1 \
    --op append-class \
    --payload ops/append_churn.yaml \
    --description "Add churn_signal class."
```

---

## REMOVE_CLASS — drop one class

Removes the named class. The remaining classes must still be ≥ 2
(so a 2-class catalog can't be reduced to 1).

```python
v3 = admin.evolve_catalog(
    from_key="acme/support_triage@v2",
    op=EvolveOp.REMOVE_CLASS,
    op_payload={"class_name": "feature_request"},
    description="Drop feature_request — moved to product board.",
)
```

```yaml
# ops/remove_feature_request.yaml
class_name: feature_request
```

```bash
efe-admin catalog evolve \
    --from-key acme/support_triage@v2 \
    --op remove-class \
    --payload ops/remove_feature_request.yaml
```

> **Tip** : prefer setting `deprecated: true` (via
> `REPLACE_CLASS_ANCHORS` with the same sentences) over removal,
> if you want classifiers to still see the class but treat it as
> low-priority. Hard removal breaks any pin to a previous version
> that referenced it (because the chain `v_n.parent_key = v_n-1`
> still points to a version with the class — but new
> classifications will never see it).

---

## UPDATE_THRESHOLDS — re-tune confidence cutoffs

Cheap and safe : no recompile, no anchor change. Useful for
post-deploy tuning when you've measured the error distribution
in shadow mode.

```python
v4 = admin.evolve_catalog(
    from_key="acme/support_triage@v3",
    op=EvolveOp.UPDATE_THRESHOLDS,
    op_payload={
        "thresholds": {
            "confidence_high": 0.85,
            "confidence_medium": 0.55,
        },
    },
    description="Tighten thresholds after measuring drift in v3.",
)
```

```yaml
# ops/raise_thresholds.yaml
thresholds:
  confidence_high: 0.85
  confidence_medium: 0.55
```

The server enforces `confidence_high ≥ confidence_medium > 0`. The
SDK pre-validates the same invariant ; an inverted pair raises
`EfeInvalidArgumentError` before the wire call.

---

## REPLACE_CLASS_ANCHORS — rewrite one class's anchors

Replaces the `canonical_sentences` for one named class. The class
itself, its color, and the deprecated flag are preserved. Every
other class is left untouched.

Use when you've discovered new representative phrases for one
class but don't want to recompile the others.

```python
v5 = admin.evolve_catalog(
    from_key="acme/support_triage@v4",
    op=EvolveOp.REPLACE_CLASS_ANCHORS,
    op_payload={
        "class_name": "billing",
        "canonical_sentences": [
            "I want a refund",
            "my card was charged twice",
            "please cancel my subscription",
            "where is my invoice",
            "this charge looks wrong",     # NEW
            "I was double-billed",         # NEW
        ],
    },
    description="Add 2 anchors to billing after Q1 incident analysis.",
)
```

The server re-embeds **only the named class** and copies the
existing compiled anchors for every other class — fast, in the
single-second range for a 5-class catalog.

---

## RECOMPILE — change the embedder model

The heavy hammer. Re-embeds **every** class against a new
`EmbedderSpec`. Use when :

* The model has been retrained.
* You're upgrading from a 384-dim to a 768-dim variant.
* You suspect drift between the production model and the catalog.

```python
v_next = admin.evolve_catalog(
    from_key="acme/support_triage@v5",
    op=EvolveOp.RECOMPILE,
    op_payload={
        "new_embedder": {
            "family": "sentence-transformers",
            "model": "paraphrase-multilingual-mpnet-base-v2",
            "dim": 768,
            "normalize": "l2",
        },
    },
    description="Upgrade to mpnet-base-v2 for higher accuracy.",
)
```

Operational notes :

* The new version becomes available only when the embed finishes
  (server-side, on the embedding pool). Until then, querying the
  new key yields `FAILED_PRECONDITION`.
* Storage doubles temporarily — the server keeps the old compiled
  anchors live until you delete the previous version.
* Cone fingerprints are tied to the embedder ; changing the
  embedder also forces classifiers to re-warm their server-side
  cone cache. This is automatic but adds ~50 ms to the first call
  per cone.

---

## Common patterns

### Branch-then-validate-then-promote

```python
# Branch off prod into a candidate version with stricter thresholds
candidate = admin.evolve_catalog(
    from_key="acme/billing@v7",
    op=EvolveOp.UPDATE_THRESHOLDS,
    op_payload={"thresholds": {"confidence_high": 0.9, "confidence_medium": 0.65}},
    description="Candidate: tighter thresholds for shadow eval.",
)

# (run shadow evaluation against `candidate.key` for a week)

# If accuracy holds, the candidate IS the new prod — no extra step.
# Just point the prod classifier to `candidate.key`. The "old"
# `acme/billing@v7` stays live until you `delete` it.
```

### Roll back to an earlier version

There's no built-in "rollback" — instead, you create a new
version that's a copy of an older one :

```python
# Snapshot v5 content
v5 = admin.get_catalog("acme/billing@v5")

# Re-register as v_next (server assigns the integer)
new = admin.register_catalog(
    tenant="acme", name="billing", version="v6-rollback",
    content=v5.content,
    description="Rollback to v5 content (post-incident #1234).",
    labels={"rollback_target": "v5", "incident": "1234"},
)
```

This preserves the audit trail (the new version's `parent_key`
points to the *current* latest, not the rollback target — the
label captures the intent).

---

## Failure modes

| Op                         | Server error                            | Client SDK exception          |
|----------------------------|-----------------------------------------|-------------------------------|
| any                        | `from_key` not found                    | `EfeNotFoundError`            |
| any                        | JWT lacks `admin:catalog:write`         | `EfeAuthError`                |
| `APPEND_CLASS`             | name collision with existing class      | `EfeInvalidArgumentError`     |
| `APPEND_CLASS`             | embedder fingerprint mismatch           | `EfeInvalidArgumentError`     |
| `REMOVE_CLASS`             | resulting catalog has < 2 classes       | `EfeInvalidArgumentError`     |
| `REMOVE_CLASS`             | class name doesn't exist                | `EfeInvalidArgumentError`     |
| `UPDATE_THRESHOLDS`        | `confidence_high < confidence_medium`   | `EfeInvalidArgumentError`     |
| `REPLACE_CLASS_ANCHORS`    | empty `canonical_sentences`             | `EfeInvalidArgumentError`     |
| `RECOMPILE`                | embedder family unknown to server       | `EfeInvalidArgumentError`     |
| `RECOMPILE`                | embed pool overloaded                   | `EfeRateLimitError` (retryable) |

The SDK pre-validates the cheap invariants (≥ 2 classes,
non-empty sentences, threshold ordering) so you get a fast local
error before the round-trip. The server re-validates everything —
the SDK is best-effort, the server is authoritative.

---

## Next

* [Catalog admin overview](./catalog-admin.md)
* [Catalog RPC reference](../reference/catalog-rpc.md)
