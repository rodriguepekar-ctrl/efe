# EFE M32 — Vector Search + Hybrid Retrieval + Embedding Index

**Status** : Architecture · `EXPERIMENTAL` · sprint scope ≈ 5 weeks
**Hard dependencies** : M27 (cache/quota), M28 (NATS event bus), M29
(audit + erasure cascade), M30 (multimodal embeddings), M31
(metering — adds a new meter `VectorSearchRequests`).
**Soft dependencies** : pgvector ≥ 0.7 (optional Postgres-native
store), Qdrant ≥ 1.10 (optional out-of-process store).
**Activation** : OFF by default. Opt-in via
`efe-vector.enabled=true` + SQL migration 0026 + per-tenant
`tenant_feature.vector_search_enabled`.

---

## 1. Why M32 now ?

Up to M31 EFE has been a **forward classifier** — text/image in,
class label out. The platform produces high-quality embeddings (SigLIP
vision, Whisper audio pooling, classifier-head text) but **discards
them** after the prediction is returned. Customers cannot :

1. Search past predictions ("find me similar tickets to the one this
   customer opened").
2. Build retrieval-augmented agents on top of EFE-produced embeddings
   (today they have to re-embed via OpenAI / Cohere — paying twice).
3. Implement deduplication, clustering, or recommendation flows.

M32 turns EFE into a **forward + retrieval** platform :

- A first-class **vector index** that stores embeddings produced by
  M30 multimodal + M21 classifier head, partitioned per tenant /
  collection.
- A **hybrid search** layer combining dense (vector) + sparse (BM25
  keyword) retrieval with Reciprocal Rank Fusion (RRF).
- An ergonomic **REST surface** under `/v1/vector/*` that competes
  with Pinecone / Weaviate / Qdrant.
- **Compliance-grade** : every collection respects the M29 GDPR
  Art. 17 erasure cascade and the M29 audit chain.

---

## 2. Scope summary

### In scope (M32)

| Capability | Description |
|---|---|
| Collections | One collection per (tenant, name) pair. Holds an HNSW index + an optional BM25 inverted index. |
| Insert | `POST /v1/vector/collections/{name}/documents` with text + optional vector + payload. |
| Search | `POST /v1/vector/collections/{name}/search` returns top-k with metadata. |
| Hybrid | Same endpoint accepts `mode: dense|sparse|hybrid_rrf|hybrid_weighted`. |
| Filters | Payload filter expressions (eq, lt, gt, in, exists, AND/OR). |
| Distance metrics | Cosine, dot product, L2. |
| HNSW index | In-RAM, M = 16 / efConstruction = 200 / efSearch = 64 tuned defaults. |
| Flat (brute-force) | Fallback for < 10k vectors per collection. |
| Quantization | Binary quantization (1 bit per dim) for ≥ 10× memory savings. |
| Persistence | Postgres mirror in `vector_doc` table (one row per vector). |
| Snapshots | Cron-driven snapshot to S3 every 6 h ; replay on pod restart. |
| Streaming index | Inserts/deletes go through NATS `vector.upsert` / `vector.delete` topics for cross-replica fan-out. |
| Erasure cascade | M29 erasure DROPs the collection + deletes the snapshot. |
| Metering | Each search call increments `VectorSearchRequests` meter (M31). |
| Audit | Every CRUD writes a HMAC audit chain row. |

### Out of scope (deferred)

| Capability | Target |
|---|---|
| Filtered HNSW (post-search filter only in M32) | M32.1 |
| Product quantization (PQ) — only binary in M32 | M32.1 |
| Cross-collection joins | M33 |
| Vector index replication across regions (passive PG mirror only in M32) | M33 |
| Cosine + dot-product on quantized vectors with re-ranking | M32.1 |
| Hybrid with reranker model (cross-encoder rerank) | M33 |
| Streaming search results (server-sent events) | M33 |

---

## 3. Domain model

### 3.1 Vocabulary

- **Collection** : a named namespace under a tenant. Holds one HNSW
  index, one optional BM25 index, a fixed `dim`, a fixed
  `distance metric`.
- **Document** : an immutable text + payload + (optional) external
  vector. The collection's HNSW stores its vector.
- **Vector ID** : `vec_…` (ULID, sortable, 26 chars).
- **Segment** : an HNSW shard. Currently 1 segment per collection ;
  M32.1 splits into multiple segments for parallel build.
- **Snapshot** : a serialized index uploaded to S3 every 6 h, plus
  on-demand at `POST /v1/vector/collections/{name}/snapshot`.
- **Query** : a dense vector, or a sparse query string, or both.
  Returns top-k `SearchHit` ordered by score (descending).
- **Score** : float. For cosine in [0, 1] (1 = identical). For dot
  no upper bound. For L2 negated (so higher = better, like cosine).

### 3.2 ER summary

```
tenant_feature                              ← M22+
  └── vector_collection (1:N)
        ├── vector_doc (0:N) — Postgres mirror of every doc
        ├── vector_index_segment (0:N) — HNSW shard descriptors
        └── retrieval_query_log (0:N) — audit + analytics

embedding_model — global table : (name, dim, normalization)
  └── vector_collection.embedding_model_name (FK)
```

### 3.3 Embedding-model catalog

| Model | Dim | Norm | Source |
|---|---:|---|---|
| `efe-classifier-768` | 768 | L2 | Internal classifier head (mode-1) |
| `siglip-base-768`    | 768 | L2 | M30 vision encoder |
| `whisper-tiny-384`   | 384 | L2 | M30 audio encoder |
| `bm25-sparse`        | — | (raw text) | Built-in tantivy BM25 |
| `external-1536`      | 1536 | L2 | Customer-provided (OpenAI / Cohere passthrough) |

The catalog rows are seeded by SQL migration 0026 and validated at
collection-create time so a tenant cannot create a collection with
an unknown model.

---

## 4. Crate boundaries

### 4.1 `efe-vector` (≈ 4 000 LOC)

Pure-Rust vector index + persistence + metrics. No HTTP / SQL
dependencies (those live in `efe-server`).

| Module | Responsibility |
|---|---|
| `lib.rs` | Public re-exports + crate invariants. |
| `distance.rs` | Cosine, dot, L2 ; SIMD-friendly inner loops. |
| `flat.rs` | Brute-force k-NN ; correct, slow, golden reference. |
| `hnsw.rs` | Skeleton HNSW (M, efConstruction, efSearch) — Rust port of the [hnsw_rs](https://crates.io/crates/hnsw_rs) algorithm tuned for our usage. |
| `quantize.rs` | Binary quantization + bit-packed scoring. |
| `store.rs` | `VectorStore` trait + `InMemoryStore` impl. |
| `segment.rs` | One HNSW shard ; build, insert, search, serialize. |
| `collection.rs` | High-level `Collection` API (insert / search / delete / snapshot). |
| `error.rs` | Typed errors. |
| `metrics.rs` | Prometheus metrics. |

### 4.2 `efe-retrieval` (≈ 1 500 LOC)

Hybrid search on top of `efe-vector` + a sparse BM25 inverted
index. Pure-Rust.

| Module | Responsibility |
|---|---|
| `lib.rs` | Public re-exports. |
| `bm25.rs` | Inverted-index BM25 scorer. |
| `sparse.rs` | Sparse-vector representation (token id → weight). |
| `rrf.rs` | Reciprocal Rank Fusion + weighted score merge. |
| `hybrid.rs` | Orchestrator combining dense + sparse + fusion. |
| `scoring.rs` | Score normalization (min-max, z-score). |
| `error.rs` | Typed errors. |

### 4.3 `efe-server::v1::vector`

| Endpoint | Method | Notes |
|---|---|---|
| `/v1/vector/collections` | GET / POST | List + create collection. |
| `/v1/vector/collections/{name}` | GET / DELETE | Inspect + drop. |
| `/v1/vector/collections/{name}/documents` | POST | Insert / upsert. |
| `/v1/vector/collections/{name}/documents/{id}` | DELETE | Drop one doc. |
| `/v1/vector/collections/{name}/search` | POST | Top-k. |
| `/v1/vector/collections/{name}/snapshot` | POST | Force snapshot. |
| `/v1/vector/collections/{name}/status` | GET | Health + cardinality + lag. |

All bearer-authed. The collection name is tenant-scoped (slug-
validated to `[a-z0-9_-]{1,32}`).

---

## 5. Distance metrics + algorithms

### 5.1 Distance metrics

| Metric | Formula | When to use |
|---|---|---|
| Cosine | `1 - (a·b / (|a| × |b|))` | Default. SigLIP & Whisper are L2-normalized so this reduces to dot. |
| Dot | `-a·b` (negate to make smaller = closer) | Pre-normalized vectors. |
| L2 | `√Σ(a-b)²` | Use when vectors are NOT L2-normalized. |

Internal representation : we always negate so the API consistently
returns `score` where **higher is better**. Cosine in [0, 1] ⇒ score
in [0, 1] ; dot in (-∞, ∞) ⇒ score in (-∞, ∞).

### 5.2 Flat (brute-force) k-NN

`O(N × dim)` per query. Used when the collection has
≤ `BRUTE_FORCE_MAX_VECTORS` (default 10 000) — HNSW build cost is
not amortized below that threshold.

### 5.3 HNSW (Hierarchical Navigable Small Worlds)

Graph-based approximate k-NN. Tunables :

| Param | Default | Meaning |
|---|---:|---|
| `M` | 16 | Max edges per node per layer. Higher = better recall, more memory. |
| `efConstruction` | 200 | Candidate list size during build. |
| `efSearch` | 64 | Candidate list size during search. |
| `maxLayers` | 16 | Max graph height. |

Recall @ 10 vs flat baseline expected ≥ 0.95 on `siglip-base-768`
embeddings at 1M vectors (validated by the bench in
`benches/hnsw_recall.rs`).

### 5.4 Binary quantization

1-bit-per-dim quantization with re-ranking. Memory : `dim / 8`
bytes instead of `dim × 4`, i.e. 32× savings. Recall @ 10 expected
≥ 0.85 with re-ranking step (load top-k×4 quantized, rerank with
fp32).

---

## 6. Hybrid retrieval

### 6.1 RRF (Reciprocal Rank Fusion)

For dense hit list `D = [d1, d2, …]` and sparse hit list
`S = [s1, s2, …]`, the RRF score of a document `x` is :

```
score_rrf(x) = Σ over runs r containing x of  1 / (k + rank_r(x))
```

`k` default = 60 (Cormack et al. 2009). Documents missing from a
run contribute 0. No score normalization needed — that's the
point of RRF.

### 6.2 Weighted fusion

For tenants who want explicit control :

```
score_weighted(x) = α × normalize(score_dense(x)) + (1-α) × normalize(score_sparse(x))
```

Default `α = 0.5`. Score normalization is min-max within the
candidate set.

### 6.3 Sparse (BM25)

Built-in inverted index over the document text. Stopword list
(English + French) ; Porter stemmer ; idf re-computed on every
insert/delete (incremental). Cardinality budget : 100 k unique
terms per collection ; above that we trim by document frequency.

---

## 7. Component diagram

```
                            ┌───────────────────┐
   POST /v1/vector/search   │   v1/vector/      │
   ─────────────────────────▶  handlers.rs      │
                            └─────────┬─────────┘
                                      ▼
                            ┌───────────────────┐
                            │  efe-retrieval    │
                            │  hybrid::search   │
                            └─────┬───────┬─────┘
                                  │       │
                            ┌─────▼──┐  ┌─▼──────┐
                            │  dense │  │ sparse │
                            │ (efe-  │  │ (BM25) │
                            │ vector)│  │        │
                            └────────┘  └────────┘
                                  │
                            ┌─────▼─────────────┐
                            │ Collection        │
                            │   ├── HNSW seg    │
                            │   ├── BM25 idx    │
                            │   └── PG mirror   │
                            └───────────────────┘
                                  ▲
              ┌───────────────────┴───────────────────┐
              │      NATS vector.upsert / .delete     │
              │   (cross-replica index fan-out)       │
              └───────────────────────────────────────┘
```

---

## 8. Failure modes

| Failure | Mitigation |
|---|---|
| Pod restart with non-persistent index | Bootstrap from latest S3 snapshot ; replay NATS subject since snapshot time. |
| S3 unavailable | New collections fail-loud with 503 ; existing collections keep running from RAM. |
| HNSW build OOM | Fallback to flat index ; alert P2. |
| Embedding-model dim mismatch on insert | 422 with clear error code. |
| Tenant erasure | Cascade drops collection + S3 snapshot prefix + NATS subject filter. |
| Hot collection saturation | Per-collection ratelimit (M27 plane). |
| Adversarial high-dim payload | Cap `max_dim = 4096` (M30 rule reused). |

---

## 9. Security & compliance

- **Tenant isolation** : collections keyed on `(tenant_id, name)` ;
  no cross-tenant search permitted.
- **GDPR Art. 17** : erasure drops the collection + S3 prefix.
- **GDPR Art. 20** : portability export includes vector_doc rows
  (M29 portability handler extends to include vectors).
- **PII** : payloads are subject to M29 PII redactor before
  insert if `tenant_feature.pii_policy != off`.
- **Audit** : every collection create/drop and every search query
  writes an audit chain row.

---

## 10. Performance budget

| Operation | Target p99 | Notes |
|---|---:|---|
| Insert (flat path) | < 500 µs | Pure memory + atomic counter. |
| Insert (HNSW path, M=16) | < 5 ms | Graph traversal. |
| Search (flat, 10 k docs, dim 768) | < 10 ms | SIMD inner loop. |
| Search (HNSW, 1 M docs, dim 768, efSearch=64) | < 8 ms | Sub-linear. |
| Search (hybrid RRF, dense + sparse, 1 M docs) | < 15 ms | Sum of dense + sparse + merge. |
| Snapshot (1 M docs, dim 768) | < 30 s | Bincode serialize + S3 PUT. |
| Replay from snapshot (1 M docs) | < 60 s | S3 GET + bincode deserialize. |

---

## 11. Observability

| Metric | Type | Labels |
|---|---|---|
| `efe_vector_insert_total` | counter | distance |
| `efe_vector_search_total` | counter | mode, distance |
| `efe_vector_search_duration_ms` | histogram | mode |
| `efe_vector_collection_size` | gauge | (none — gauge keyed via PG view) |
| `efe_vector_hnsw_build_duration_ms` | histogram | (none) |
| `efe_vector_snapshot_duration_ms` | histogram | outcome |
| `efe_vector_recall_at_10` | gauge | model |
| `efe_retrieval_rrf_duration_ms` | histogram | (none) |
| `efe_retrieval_bm25_duration_ms` | histogram | (none) |

Cardinality budget per ADR-M32-005 : ≤ 16 series per histogram.

---

## 12. SLOs

| SLO | Target |
|---|---|
| Vector search availability | 99.95 % |
| Vector search p99 latency (dense, < 1M docs) | 99% < 10 ms |
| Snapshot success rate | 99.9 % |
| Recall @ 10 vs flat baseline | ≥ 0.95 |
| Erasure cascade completion | within 4 h of M29 erasure trigger |

---

## 13. ADRs (M32-001..008)

- **M32-001** HNSW over IVF / DiskANN / brute-force-everywhere.
- **M32-002** RRF over weighted-mean as default hybrid strategy.
- **M32-003** In-process index + Postgres mirror over Qdrant for default deploy.
- **M32-004** Binary quantization over PQ for the M32 MVP.
- **M32-005** Prometheus cardinality budget for vector metrics.
- **M32-006** Tenant-scoped collection slugs (`[a-z0-9_-]{1,32}`).
- **M32-007** S3 snapshot cadence at 6 h with NATS replay tail.
- **M32-008** Hard cap on `max_dim = 4096` + `max_docs_per_collection = 10 M`.

---

## 14. Runbooks (RB_M32_*)

- **RB_M32_INDEX_OOM** — pod hit memory limit during build.
- **RB_M32_SNAPSHOT_FAILED** — S3 PUT errored ; index drift risk.
- **RB_M32_RECALL_REGRESSION** — recall@10 dropped below SLO.
- **RB_M32_REPLAY_LAG** — NATS replay tail not catching up.
- **RB_M32_TENANT_ISOLATION_AUDIT** — quarterly cross-tenant leak audit.

---

## 15. Migration plan

1. SQL migration 0026 (tables empty).
2. Deploy `efe-vector.enabled=false` everywhere.
3. Enable for `efe-internal` tenant, validate insert/search/erasure.
4. Open private beta : 3 friendly pilot tenants.
5. EXPERIMENTAL GA opt-in.
6. After 3 months of stable operation : GA stable.

---

## 16. Annex A — files shipped in this milestone

```
crates/efe-vector/                  [NEW]
  Cargo.toml
  src/
    lib.rs / distance.rs / flat.rs / hnsw.rs / quantize.rs
    store.rs / segment.rs / collection.rs / error.rs / metrics.rs
  benches/
    distance.rs / flat_search.rs / hnsw_recall.rs

crates/efe-retrieval/               [NEW]
  Cargo.toml
  src/
    lib.rs / bm25.rs / sparse.rs / rrf.rs / hybrid.rs
    scoring.rs / error.rs

crates/efe-server/src/v1/vector/    [NEW]
  mod.rs / handlers.rs / errors.rs / state.rs

crates/efe-server/migrations/
  0026_m32_vector_search.sql / 0026_m32_vector_search_rollback.sql

charts/efe-stack/charts/efe-vector/ [NEW]
  Chart.yaml / values.yaml / templates/_helpers.tpl
  templates/snapshot-cron.yaml / templates/network-policy.yaml

terraform/m32_vector/               [NEW]
  s3_snapshot.tf / variables.tf / outputs.tf / README.md

deploy/observability/
  prometheus/rules.m32/alerts.yaml
  grafana/dashboards/m32-01-vector-overview.json

scripts/
  chaos/m32/ (pod_kill_with_snapshot.sh)
  load/m32/ (search_steady_state.sh)
  m32/ (snapshot_runner.sh, replay_runner.sh)
  m32-assembly-verify.sh

bench/
  m32_vector_pentest/ (test_cross_tenant_leak.py)

docs/
  EFE_M32_VECTOR_SEARCH_ARCHITECTURE.md   [this doc]
  EFE_M32_KNOWN_GAPS.md
docs/docs/adr/  ADR-M32-001..008-*.md
docs/runbooks/  RB_M32_*.md                (5 files)
CHANGELOG.md (2.7.0 entry)
```

Approximate net new : **~ 75 files**.
