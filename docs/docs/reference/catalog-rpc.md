# Reference — `efe.v1.catalog.CatalogService`

> Sprint **D12** (v0.1.12). Defined in
> `protos/efe/v1/catalog.proto`. The SDK Python wrapper is in
> [`efe_tool.admin.EfeAdminClient`](./api.md#efe_toolad_min) — this
> page mirrors the proto contract for clients written in any
> language.

For task-oriented guides, see [catalog admin](../howto/catalog-admin.md)
and [catalog evolution](../howto/catalog-evolution.md).

---

## Service summary

```protobuf
service CatalogService {
  rpc RegisterCatalog(RegisterCatalogRequest) returns (CatalogHandle);
  rpc EvolveCatalog(EvolveCatalogRequest) returns (CatalogHandle);
  rpc GetCatalog(GetCatalogRequest) returns (CanonicalCatalog);
  rpc ListCatalogs(ListCatalogsRequest) returns (ListCatalogsResponse);
  rpc ListCatalogVersions(ListCatalogVersionsRequest)
      returns (ListCatalogVersionsResponse);
  rpc DeleteCatalog(DeleteCatalogRequest) returns (google.protobuf.Empty);
}
```

All RPCs are **synchronous** and **idempotent in the read direction** ;
register/evolve are non-idempotent (each call produces a new
version).

---

## Authentication & RBAC

Every RPC requires a JWT in the `authorization: Bearer <jwt>`
metadata. The JWT must carry :

* `tenant_id` — the tenant scope. Cross-tenant access is
  **architecturally impossible** : the server rejects every
  request whose `tenant` field doesn't match the JWT claim.
* `sub` — the principal subject. Stamped on every audit record.
* one of these scopes :

| RPC                   | Required scope            |
|-----------------------|---------------------------|
| `RegisterCatalog`     | `admin:catalog:write`     |
| `EvolveCatalog`       | `admin:catalog:write`     |
| `GetCatalog`          | `admin:catalog:read`      |
| `ListCatalogs`        | `admin:catalog:read`      |
| `ListCatalogVersions` | `admin:catalog:read`      |
| `DeleteCatalog`       | `admin:catalog:write`     |

Scopes are independent of `admin:cone:*` (the M9-era cone admin
scope) — issuing a JWT with `admin:catalog:read` does not grant
cone access.

---

## Canonical key

Every catalog version is identified by a canonical key of the form

```
<tenant>/<name>@v<integer>
```

Example : `acme/support_triage@v3`. The integer is monotonic per
`(tenant, name)` ; the server assigns it on register/evolve. The
SemVer-shaped `version` string passed to `RegisterCatalog` is
**informational** — it shows up in the audit trail but does not
participate in lookup.

> The 'v' prefix is parsed leniently by the SDK :
> `acme/billing@3` and `acme/billing@v3` are equivalent.

---

## Messages

### CatalogContent

The full body of a registered catalog. Mirrors the YAML
`AnchorCatalog` schema (`apiVersion: efe.mode1/v1`).

```protobuf
message CatalogContent {
  EmbedderSpec embedder = 1;
  repeated CatalogClass classes = 2;
  ConePolicy cone_policy = 3;
  ThresholdPolicy thresholds = 4;
}

message EmbedderSpec {
  string family = 1;     // e.g. "sentence-transformers"
  string model = 2;
  uint32 dim = 3;
  string normalize = 4;  // "l2"
}

message CatalogClass {
  string name = 1;
  string description = 2;
  repeated string canonical_sentences = 3;
  string color = 4;
  bool deprecated = 5;
}

message ConePolicy {
  float max_projection_distance = 1;
}

message ThresholdPolicy {
  float confidence_high = 1;
  float confidence_medium = 2;
}
```

**Validation invariants** (enforced server-side ; SDK
pre-validates the cheap ones) :

* `classes` ≥ 2 entries.
* Every class has a non-empty `canonical_sentences` list.
* Every class name matches `[a-z][a-z0-9_]{0,62}`.
* `thresholds.confidence_high ≥ confidence_medium > 0`.
* `cone_policy.max_projection_distance ∈ (0, 1]`.
* `embedder.dim` matches the resolved model's output dimension.

### CanonicalCatalog

The returned form. Adds an audit envelope and (optionally)
compiled anchor refs.

```protobuf
message CanonicalCatalog {
  CatalogContent content = 1;
  CatalogMetadata metadata = 2;
  repeated CompiledClass compiled = 3;
}

message CatalogMetadata {
  google.protobuf.Timestamp created_at = 1;
  string created_by = 2;        // sub claim
  string description = 3;
  string parent_key = 4;        // empty for v1
  string evolution_op = 5;      // "register" | "append_class" | ...
  map<string, string> labels = 6;
  string content_sha256 = 7;    // immutability witness
}

message CompiledClass {
  string class_name = 1;
  bytes anchors_blob = 2;          // (N, dim) float32 LE
  uint32 num_anchors = 3;
  string embedder_fingerprint = 4;
}
```

The `compiled` field is populated **only** when `include_compiled=true`
on the `Get` request, and the caller has `admin:catalog:read`. The
`anchors_blob` floats are the actual server-side compiled anchors
in row-major float32 little-endian byte order — useful only for
the in-memory cache rehydration path.

---

## RegisterCatalog

```protobuf
rpc RegisterCatalog(RegisterCatalogRequest) returns (CatalogHandle);

message RegisterCatalogRequest {
  string tenant = 1;
  string name = 2;
  string version = 3;            // SemVer-shaped, informational
  CatalogContent content = 4;
  string created_by = 5;
  string description = 6;
  map<string, string> labels = 7;
}

message CatalogHandle {
  string key = 1;                // canonical "tenant/name@vN"
  CanonicalCatalog catalog = 2;
}
```

**Idempotency** : NOT idempotent. Each successful call creates a
new version with the next integer. To detect duplicate work,
clients can :

1. Compute the SHA-256 of the canonical JSON of `content` locally.
2. Compare against the latest version's `metadata.content_sha256`.
3. Skip if equal.

The server does NOT auto-skip duplicates because two callers
might want to record an "I tested this version" intent
independently — the audit trail is the value.

**gRPC errors** :

| code                 | meaning                                     |
|----------------------|---------------------------------------------|
| `INVALID_ARGUMENT`   | schema violation                            |
| `RESOURCE_EXHAUSTED` | quota hit (≥ 1000 versions per name, or ≥ 100 names per tenant) |
| `PERMISSION_DENIED`  | tenant mismatch / scope missing             |
| `INTERNAL`           | server-side compile failed (rare)           |

---

## EvolveCatalog

```protobuf
rpc EvolveCatalog(EvolveCatalogRequest) returns (CatalogHandle);

message EvolveCatalogRequest {
  string from_key = 1;            // source canonical key
  CatalogEvolutionOp op = 2;
  CatalogMetadata metadata_override = 3;
}

message CatalogEvolutionOp {
  oneof op {
    AppendClassOp append_class = 1;
    RemoveClassOp remove_class = 2;
    UpdateThresholdsOp update_thresholds = 3;
    ReplaceClassAnchorsOp replace_class_anchors = 4;
    RecompileOp recompile = 5;
  }
}

message AppendClassOp        { CatalogClass cls = 1; }
message RemoveClassOp        { string class_name = 1; }
message UpdateThresholdsOp   { ThresholdPolicy thresholds = 1; }
message ReplaceClassAnchorsOp {
  string class_name = 1;
  repeated string canonical_sentences = 2;
}
message RecompileOp          { EmbedderSpec new_embedder = 1; }
```

Returns the **new** version (with the integer incremented +1
relative to `from_key`'s version). The new
`metadata.parent_key` always points to `from_key`.

For per-op semantics, see
[catalog evolution how-to](../howto/catalog-evolution.md).

**gRPC errors** :

| code                  | meaning                                           |
|-----------------------|---------------------------------------------------|
| `NOT_FOUND`           | `from_key` is unknown                             |
| `INVALID_ARGUMENT`    | op-specific schema violation                      |
| `FAILED_PRECONDITION` | RECOMPILE: embed pool unavailable                 |
| `RESOURCE_EXHAUSTED`  | quota                                             |
| `PERMISSION_DENIED`   | tenant mismatch / scope missing                   |

---

## GetCatalog

```protobuf
rpc GetCatalog(GetCatalogRequest) returns (CanonicalCatalog);

message GetCatalogRequest {
  string key = 1;                 // canonical key
  bool include_compiled = 2;
}
```

**Idempotency** : safe.

**gRPC errors** :

| code                | meaning                          |
|---------------------|----------------------------------|
| `NOT_FOUND`         | key doesn't exist                |
| `INVALID_ARGUMENT`  | malformed key                    |
| `PERMISSION_DENIED` | tenant mismatch / scope missing  |

---

## ListCatalogs

```protobuf
rpc ListCatalogs(ListCatalogsRequest) returns (ListCatalogsResponse);

message ListCatalogsRequest {
  string tenant = 1;              // MUST match JWT tenant_id
  int32 page_size = 2;            // default 100, max 500
  string page_token = 3;          // empty for first page
}

message ListCatalogsResponse {
  repeated CatalogRefSummary catalogs = 1;
  string next_page_token = 2;
}

message CatalogRefSummary {
  string tenant = 1;
  string name = 2;
  uint32 latest_version = 3;
  uint32 num_classes = 4;
  uint32 num_anchors = 5;
}
```

Returns one row per `(tenant, name)` tuple — the *latest* version
of each — never one row per version. Use `ListCatalogVersions`
for version history.

**Idempotency** : safe (subject to tenant adding/removing
catalogs concurrently).

The server fills `tenant` to the caller's tenant on every entry
— foreign tenants are filtered server-side, never observed by
the client.

---

## ListCatalogVersions

```protobuf
rpc ListCatalogVersions(ListCatalogVersionsRequest)
    returns (ListCatalogVersionsResponse);

message ListCatalogVersionsRequest {
  string tenant = 1;
  string name = 2;
}

message ListCatalogVersionsResponse {
  repeated string keys = 1;       // each entry "tenant/name@vN"
}
```

Returns canonical keys sorted ascending by integer version. The
latest is always the last entry.

**Idempotency** : safe.

---

## DeleteCatalog

```protobuf
rpc DeleteCatalog(DeleteCatalogRequest) returns (google.protobuf.Empty);

message DeleteCatalogRequest {
  string key = 1;                 // canonical key
  bool force = 2;
}
```

Without `force=true`, the server refuses to delete a version
still referenced by another (e.g. as `parent_key`). With `force`,
it cascade-deletes the chain.

**gRPC errors** :

| code                  | meaning                                        |
|-----------------------|------------------------------------------------|
| `NOT_FOUND`           | key doesn't exist                              |
| `FAILED_PRECONDITION` | referenced and `force=false`                   |
| `PERMISSION_DENIED`   | tenant mismatch / scope missing                |

> **Note** : deletion is irreversible. There's no soft-delete
> tier in v0.1.12. To recover, register the previous content as
> a new version (see "Roll back" in
> [catalog evolution](../howto/catalog-evolution.md)).

---

## Concurrent register/evolve semantics

The server serializes register/evolve **per `(tenant, name)`** —
two concurrent register calls for the same name will be processed
in arrival order, and the second sees the integer assigned to it
post-serialization. There is no race in version assignment.

For different names within the same tenant, register/evolve calls
proceed in parallel.

---

## SLOs

The CatalogService publishes 7 SLOs (D12-CT-1 through D12-CT-7) :

| ID         | Metric                                               | Target  |
|------------|------------------------------------------------------|---------|
| D12-CT-1   | `RegisterCatalog` p99 latency, ≤ 5 classes           | ≤ 200 ms|
| D12-CT-2   | `RegisterCatalog` p99 latency, 100 classes           | ≤ 1500 ms|
| D12-CT-3   | `EvolveCatalog` p99 latency, APPEND_CLASS            | ≤ 250 ms|
| D12-CT-4   | `GetCatalog` p99 latency                             | ≤ 50 ms |
| D12-CT-5   | `ListCatalogs` p99 latency                           | ≤ 75 ms |
| D12-CT-6   | concurrent register/evolve serialization correctness | 100% pass |
| D12-CT-7   | `content_sha256` immutability witness round-trip     | 100% match |

Tracked in `bench/load/parse_ghz_report.py`. Regressions block
release at the same threshold as the D9-LOAD family (see
`docs/bench/d9_docker_helm_load_architecture.md`).

---

## Stub regeneration

When this proto changes, regenerate the Python stubs :

```bash
python3 -m grpc_tools.protoc \
    --proto_path=protos \
    --python_out=clients/python/efe_tool/_proto \
    --grpc_python_out=clients/python/efe_tool/_proto \
    --pyi_out=clients/python/efe_tool/_proto \
    protos/efe/v1/catalog.proto

# Patch the gRPC stub's import path (gencode emits 'efe.v1' but the
# package layout is 'efe_tool._proto.efe.v1') :
sed -i.bak \
    's|^from efe.v1 import catalog_pb2|from efe_tool._proto.efe.v1 import catalog_pb2|' \
    clients/python/efe_tool/_proto/efe/v1/catalog_pb2_grpc.py
```

Rust stubs are regenerated by `build.rs` — no manual step.
