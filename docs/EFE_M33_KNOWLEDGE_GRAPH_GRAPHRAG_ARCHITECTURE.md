# EFE M33 — Knowledge Graph + GraphRAG

**Status** : Architecture · `EXPERIMENTAL` · sprint scope ≈ 6 weeks
**Hard dependencies** : M28 (NATS for entity events), M29 (PII redactor
+ erasure cascade), M30 (Triton for NER inference), M31 (metering
adds `KgEntitiesExtracted` + `GraphHopsTraversed`), M32 (vector
search is fused with graph traversal).
**Activation** : OFF by default. Opt-in via `efe-kg.enabled=true` +
SQL migration 0027 + per-tenant `tenant_feature.kg_enabled`.

---

## 1. Why M33 now ?

M32 gave EFE **dense + sparse retrieval**. The next bottleneck is
**reasoning over relationships**. Customers want :

1. *"Find tickets similar to this one, **and** show the related
   accounts + their owners."*
2. *"Build a knowledge graph from these 10 k Q&A pairs and let an
   agent traverse it."*
3. *"Trace why this customer churned — across calls, emails, and
   billing events."*

Vanilla vector search returns nearest neighbors but cannot follow
edges. M33 adds :

- **Entity extraction** : NER over text (and OCR'd images via M30)
  to identify mentions of `PERSON`, `ORG`, `LOCATION`, `PRODUCT`,
  `EVENT`, `MISC`.
- **Relation extraction** : a classifier head over the entity pair
  context (e.g. `(person, "works_at", org)`).
- **Graph storage** : in-process adjacency-list graph with Postgres
  mirror.
- **Graph traversal** : BFS / DFS / k-hop / shortest path with
  filter predicates.
- **GraphRAG** : multi-hop expand starting from M32 vector hits,
  re-ranked by graph-distance + dense similarity.

---

## 2. Scope summary

### In scope (M33)

| Capability | Description |
|---|---|
| Entity extraction | NER over text via Triton-served model (English + French) ; 6 entity types. |
| Relation extraction | Classifier head over `(subject, predicate, object)` triples ; 18 base relations + custom per tenant. |
| Graph storage | One per-tenant graph backed by in-process `Graph` + PG mirror (`kg_entity`, `kg_relation`, `kg_claim`). |
| Insert / upsert | `POST /v1/kg/extract` extracts entities + relations from text and merges into the graph. |
| Direct CRUD | `POST /v1/kg/entities`, `POST /v1/kg/relations` for explicit construction. |
| Traversal | BFS / DFS / k-hop / shortest-path with predicate + label filters. |
| Multi-hop planner | Translates a natural-language query into a sequence of (vector_search → graph_expand → vector_search) steps. |
| GraphRAG fusion | Combines vector hits with graph proximity. |
| Erasure cascade | M29 erasure drops the tenant's kg_graph + PG rows + S3 snapshot. |
| Metering | Two new meters : `KgEntitiesExtracted`, `GraphHopsTraversed`. |

### Out of scope (deferred)

| Capability | Target |
|---|---|
| Open-domain relation extraction (zero-shot) | M33.1 |
| Cypher-like query language | M33.1 |
| Cross-tenant graph federation | M34 |
| Real-time NER (streaming) | M34 |
| Triplet reasoning (TransE / ComplEx) | M34 |
| Graph neural networks for link prediction | M35 |

---

## 3. Domain model

### 3.1 Vocabulary

- **Entity** : a node in the graph. Has `id` (ULID), `kind` (one
  of 6 NER labels), `canonical_name`, `mentions` (list of doc
  references), `embedding` (vector for fusion with M32).
- **Relation** : a directed edge. Has `subject_id`, `predicate`,
  `object_id`, `confidence` (0..1), `provenance` (list of
  doc-spans).
- **Claim** : a (subject, predicate, object) triple with
  provenance. Multiple claims can support one relation (high
  confidence ⇒ promoted to relation).
- **Graph** : the per-tenant collection of entities + relations.
- **Query plan** : sequence of (`vector_search`, `graph_expand`,
  `score_merge`) steps produced by the multi-hop planner.

### 3.2 Predicates (M33 default catalog)

| Predicate | Domain | Range | Notes |
|---|---|---|---|
| `works_at` | PERSON | ORG | Default M33. |
| `located_in` | ORG / EVENT | LOCATION | |
| `subsidiary_of` | ORG | ORG | |
| `member_of` | PERSON | ORG | |
| `attended` | PERSON | EVENT | |
| `organized` | ORG | EVENT | |
| `acquired` | ORG | ORG | |
| `released` | ORG | PRODUCT | |
| `competes_with` | ORG | ORG | |
| `parent_of` | PERSON | PERSON | |
| `spouse_of` | PERSON | PERSON | Symmetric. |
| `founded` | PERSON | ORG | |
| `ceo_of` | PERSON | ORG | |
| `same_as` | * | * | Entity resolution. |
| `mentions` | DOC | * | Indirect link. |
| `relates_to` | * | * | Generic catch-all. |
| `purchased` | PERSON / ORG | PRODUCT | |
| `tagged_with` | * | TAG | Custom labels. |

Tenants may add **custom predicates** via the
`POST /v1/kg/predicates` endpoint (stored in
`kg_predicate_catalog`).

### 3.3 ER summary

```
tenant_feature                              ← M22+
  └── kg_graph (1:1)                        ← M33 (per-tenant root)
        ├── kg_entity (0:N)
        │     └── kg_entity_mention (0:N)  (which doc span produced it)
        ├── kg_relation (0:N)
        ├── kg_claim (0:N)                  (raw triples → promotable to relations)
        ├── kg_predicate_catalog (1:N)     (custom predicates per tenant)
        └── kg_query_log (0:N)              (analytics + audit)
```

---

## 4. Crate boundaries

### 4.1 `efe-kg` (≈ 4 500 LOC)

Pure-Rust knowledge-graph plane. No HTTP / SQL dependencies.

| Module | Responsibility |
|---|---|
| `lib.rs` | Public re-exports + invariants. |
| `entity.rs` | `Entity`, `EntityKind`, canonical-name normalization. |
| `relation.rs` | `Relation`, `Predicate`, base catalog. |
| `claim.rs` | `Claim` (raw triple) + `promote_claims_to_relations`. |
| `graph.rs` | Adjacency-list graph + insert / lookup. |
| `traversal.rs` | BFS, DFS, k-hop, shortest path with filters. |
| `store.rs` | `KgStore` trait + `InMemoryStore`. |
| `extractor.rs` | `NerExtractor` + `RelationExtractor` traits ; HTTP impl via Triton. |
| `dedup.rs` | Entity resolution via cosine similarity + Levenshtein name distance. |
| `error.rs` | Typed errors. |
| `metrics.rs` | Prometheus metrics. |

### 4.2 `efe-graphrag` (≈ 2 000 LOC)

Fuses M32 vector search with the M33 graph.

| Module | Responsibility |
|---|---|
| `lib.rs` | Re-exports. |
| `planner.rs` | Multi-hop planner — converts a user query into a sequence of steps. |
| `multi_hop.rs` | Executes the plan ; orchestrates dense → graph_expand → dense. |
| `fusion.rs` | Combines vector scores + graph-distance into a final ranking. |
| `scoring.rs` | Score normalization + graph-distance decay function. |
| `error.rs` | Typed errors. |

### 4.3 `efe-server::v1::kg`

| Endpoint | Method | Description |
|---|---|---|
| `/v1/kg/graph/status` | GET | Inspect tenant's graph (entity count, relation count). |
| `/v1/kg/extract` | POST | Run NER + RE over a text snippet ; merge into the graph. |
| `/v1/kg/entities` | GET, POST | List + insert entities. |
| `/v1/kg/entities/{id}` | GET, DELETE | Inspect + drop one entity. |
| `/v1/kg/relations` | GET, POST | List + insert relations. |
| `/v1/kg/traverse` | POST | k-hop traversal (BFS) from a seed entity. |
| `/v1/kg/predicates` | GET, POST | List + add custom predicates. |
| `/v1/graphrag/search` | POST | Multi-hop GraphRAG retrieval (fuses M32 + M33). |

### 4.4 PG schema (cf. migration 0027)

5 new tables : `kg_graph`, `kg_entity`, `kg_entity_mention`,
`kg_relation`, `kg_claim`, `kg_predicate_catalog`, `kg_query_log`.
1 view : `v_kg_graph_summary`.

### 4.5 Helm `efe-kg`

NER extractor sidecar (a Python container hosting a fine-tuned
spaCy + transformers model — French + English). Optional :
tenants can choose to extract via M30 Triton instead.

### 4.6 Terraform

S3 bucket for graph snapshots (6h cadence, same lifecycle policy
as M32).

---

## 5. Algorithms

### 5.1 NER pipeline

1. Tokenize the input text (whitespace + sentence splitter).
2. Call Triton or the sidecar NER model — returns
   `[(start, end, kind, text)]` spans.
3. Normalize canonical names (lowercase + strip punctuation +
   diacritics).
4. Compute the per-entity embedding via M30 classifier head.
5. Run entity resolution (cosine ≥ 0.85 + Levenshtein ≤ 3 ⇒
   merge with existing entity).

### 5.2 Relation extraction

Per sentence with ≥ 2 entities :

1. Build candidate triples `(e_i, ?, e_j)` for each entity pair.
2. Feed the sentence + entity spans to the RE classifier.
3. Top-1 predicate ⇒ candidate `Claim`.
4. `Claim` becomes a `Relation` when supported by ≥ 2 claims OR
   confidence ≥ 0.9.

### 5.3 Graph traversal

Standard graph algorithms. Cap : `MAX_HOPS = 6`,
`MAX_FANOUT_PER_HOP = 256` (cf. ADR-M33-006).

### 5.4 Multi-hop GraphRAG

Plan :

```
1. dense_hits   = vector_search(query_text, k=top_k_dense)
2. seeds        = entities_mentioned_in(dense_hits)
3. expanded     = bfs(seeds, max_hops=2, predicates=[...])
4. graph_score  = (1 - hop_distance / max_hops)
5. final_score  = α × dense_score + (1-α) × graph_score
6. rank         = top_k(by final_score)
```

Default `α = 0.7` (favours dense ; graph is a relevance booster
for related entities).

---

## 6. Failure modes

| Failure | Mitigation |
|---|---|
| NER sidecar OOM | Fallback to per-sentence retries with smaller batch. |
| Graph cycle | Traversal bounded by `MAX_HOPS` ; cycles are flagged but not error. |
| Entity-resolution false merge | Tenant can `POST /v1/kg/entities/{id}/split` to revert. |
| RE confidence low | Demote to `Claim` table (not surfaced in traversal). |
| Triton timeout | Same per-request cap as M30 ; fail-open NER returns empty extraction. |
| Adversarial entity name (> 200 chars) | Reject at ingress ; runbook flags. |

---

## 7. Security & compliance

- **PII** : entities of kind `PERSON` route through M29 redactor
  before persistence. We store `canonical_name = sha256_prefix(...)`
  for strict-policy tenants.
- **GDPR Art. 17** : erasure drops the tenant's kg_graph + S3
  snapshot + NATS subjects.
- **Tenant isolation** : all kg tables keyed on `tenant_id` ; no
  cross-tenant traversal.
- **Audit** : create / drop / split / merge entities each write
  HMAC audit rows.

---

## 8. Performance budget

| Operation | p99 target | Notes |
|---|---:|---|
| Entity insert | < 1 ms | In-RAM graph + async PG mirror. |
| Relation insert | < 1 ms | Same. |
| 2-hop traversal | < 10 ms | At fan-out 256, ~ 65 k visited at hop 2. |
| GraphRAG search (dense=20, hops=2) | < 50 ms | Sum of vector + graph + fusion. |
| NER extraction (1 KB text) | < 100 ms | Triton round-trip. |
| Snapshot (1 M entities, 5 M relations) | < 30 s | bincode → S3. |

---

## 9. Observability

| Metric | Type | Labels |
|---|---|---|
| `efe_kg_entity_insert_total` | counter | kind |
| `efe_kg_relation_insert_total` | counter | predicate |
| `efe_kg_traversal_duration_ms` | histogram | algo |
| `efe_kg_extractor_duration_ms` | histogram | outcome |
| `efe_kg_graph_size` | gauge | side (entities, relations) |
| `efe_graphrag_search_duration_ms` | histogram | mode |
| `efe_graphrag_hops_traversed` | histogram | (none) |
| `efe_kg_resolution_merges_total` | counter | (none) |

Cardinality budget : ≤ 24 series / histogram.

---

## 10. SLOs

| SLO | Target |
|---|---|
| KG insert availability | 99.95 % |
| GraphRAG p99 (top-10) | < 100 ms |
| Snapshot success rate | 99.9 % |
| Entity-resolution false-merge rate | ≤ 0.5 % |

---

## 11. Threat model

| Threat | Mitigation |
|---|---|
| Adversarial NER feed (huge text) | Cap input at 100 kB ; reject. |
| Graph explosion via cycles | `MAX_HOPS` + `MAX_FANOUT_PER_HOP` enforced. |
| Cross-tenant data leak | Every traversal scoped by `tenant_id` (compile-time enforced). |
| Replay of NER from another tenant's text | NER service is stateless. |
| Custom predicate injection | Slug-validated `[a-z0-9_]{1,32}`. |

---

## 12. Rollout

| Phase | Duration | Description |
|---|---|---|
| α (internal) | 1 week | Internal tenant. NER on 100 docs ; manually inspect graph. |
| β (3 pilots) | 3 weeks | Real-world Q&A datasets. Measure recall@10 of GraphRAG vs vector-only. |
| γ (EXPERIMENTAL GA) | 6 weeks | Open opt-in. Watch SLOs ; collect feedback. |
| GA stable | M33.1 | After 3 months of stable operation. |

---

## 13. ADRs (M33-001..008)

- **M33-001** Adjacency-list graph over relational join model.
- **M33-002** Claim-then-promote over direct relation insert.
- **M33-003** In-process graph + PG mirror over Neo4j / Memgraph.
- **M33-004** Predicate catalog over open-domain relations.
- **M33-005** Prometheus cardinality budget.
- **M33-006** `MAX_HOPS = 6`, `MAX_FANOUT_PER_HOP = 256`.
- **M33-007** GraphRAG α default = 0.7 (favours dense).
- **M33-008** Entity-resolution threshold (cosine 0.85 + Lev 3).

---

## 14. Runbooks (RB_M33_*)

- **RB_M33_NER_DOWN** — extractor sidecar unhealthy.
- **RB_M33_GRAPH_OOM** — in-process graph exceeded pod memory.
- **RB_M33_RESOLUTION_DRIFT** — false-merge rate climbing.
- **RB_M33_GRAPHRAG_LATENCY_HIGH** — multi-hop SLO breach.
- **RB_M33_SNAPSHOT_FAILED** — graph snapshot upload failed.

---

## 15. Migration plan

1. SQL migration 0027 (tables empty).
2. Deploy `efe-kg.enabled=false`.
3. Enable for `efe-internal` ; extract over a 1 k-doc sample.
4. Validate erasure cascade.
5. Pilot beta with 3 friendly tenants.
6. EXPERIMENTAL GA opt-in.

---

## 16. Annex A — files shipped in this milestone

```
crates/efe-kg/                       [NEW]
  Cargo.toml
  src/
    lib.rs / entity.rs / relation.rs / claim.rs / graph.rs
    traversal.rs / store.rs / extractor.rs / dedup.rs
    error.rs / metrics.rs

crates/efe-graphrag/                 [NEW]
  Cargo.toml
  src/
    lib.rs / planner.rs / multi_hop.rs / fusion.rs
    scoring.rs / error.rs

crates/efe-server/src/v1/kg/         [NEW]
  mod.rs / handlers.rs / errors.rs / state.rs
crates/efe-server/src/v1/graphrag/   [NEW]
  mod.rs / handlers.rs / errors.rs / state.rs

crates/efe-server/migrations/
  0027_m33_knowledge_graph.sql
  0027_m33_knowledge_graph_rollback.sql

charts/efe-stack/charts/efe-kg/      [NEW]
  Chart.yaml / values.yaml / templates/_helpers.tpl
  templates/ner-extractor.yaml / templates/snapshot-cron.yaml

terraform/m33_kg/                    [NEW]
  s3_snapshot.tf / variables.tf / outputs.tf / README.md

deploy/observability/
  prometheus/rules.m33/alerts.yaml
  grafana/dashboards/m33-01-kg-overview.json

scripts/
  chaos/m33/ / load/m33/ / m33/
  m33-assembly-verify.sh
bench/m33_kg_pentest/

docs/
  EFE_M33_KNOWLEDGE_GRAPH_GRAPHRAG_ARCHITECTURE.md   [this]
docs/docs/adr/  ADR-M33-001..008-*.md
docs/runbooks/  RB_M33_*.md                          (5 files)
CHANGELOG.md (2.8.0 entry)
```

Approximate net new : **~ 70 files**.
