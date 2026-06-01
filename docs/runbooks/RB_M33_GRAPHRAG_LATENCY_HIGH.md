# RB_M33_GRAPHRAG_LATENCY_HIGH — GraphRAG p99 > SLO

**Severity** : P2.
**Trigger** : `M33GraphRagP99HighLatency`.

## 1. Diagnose
```bash
# Check which step is slow
curl -X POST $API/v1/graphrag/search ... | jq '.hits[].score'
```

## 2. Common causes

### 2.1 Dense-search slow
- Underlying M32 collection too large or HNSW efSearch high.

### 2.2 Graph fan-out high
- Hot tenant graph has a super-node (1M outgoing edges).
- Mitigation : reduce `max_hops` in the query.

### 2.3 Fusion CPU-bound
- Top_k > 1000 sorting all hits at scale.

## 3. Mitigation
1. Reduce `top_k` or `max_hops` in client SDK defaults.
2. Identify super-node + add filter predicate to skip it.
