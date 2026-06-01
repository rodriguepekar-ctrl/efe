# RB_M32_RECALL_REGRESSION — Recall @ 10 dropped below SLO

**Severity** : P3 (quality, not availability). P2 if < 0.85.
**Trigger** : `M32RecallRegression` (`efe_vector_recall_at_10 < 900`).

## 1. Diagnose

Recall is measured by `bench/m32_recall.py` which runs nightly
against a fixed evaluation set + the live index. The gauge is set
by the bench harness.

```bash
# Confirm the regression on the latest bench run
python3 bench/m32_recall.py --model siglip-base-768
```

## 2. Common causes

### 2.1 HNSW parameter drift
- Operator lowered `efSearch` / `M` to save memory ; recall drops.
- Mitigation : restore the previous values or recompute via the
  recall sweeper.

### 2.2 Embedding model swap
- A tenant migrated from `efe-classifier-768` to `external-1536`
  without re-indexing → recall meaningless because the test set
  uses old embeddings.
- Mitigation : re-index ; the recall gauge ignores collections
  without a valid model.

### 2.3 Adversarial inserts
- Tenant inserted vectors with extreme magnitude that destabilize
  the HNSW graph.
- Mitigation : enforce L2 normalization at ingress.

## 3. Customer impact
- Search results quality degrades subtly. Customers may complain
  about "missed obvious matches".

## 4. Mitigation
1. Compare last 7 days of recall ; identify regression date.
2. Roll back any config change made on that date.
3. If the cause is data : trigger a full rebuild (snapshot →
   replay).

## 5. Post-incident
- File P3 ticket.
- If the cause was a config knob : add a guard in
  `vector_collection` CHECK constraints (e.g. min efSearch=64).
