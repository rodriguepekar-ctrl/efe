# How-to — server-side catalog admin

> Sprint **D12** (v0.1.12). New surface : versioned anchor catalogs
> stored server-side, immutable post-register, evolved through the
> 5 oneof ops on `CatalogService.EvolveCatalog`.

This page covers the **register / list / get / delete** lifecycle.
For evolution, see [catalog evolution](./catalog-evolution.md). For
the underlying RPC contract, see
[catalog RPC reference](../reference/catalog-rpc.md).

---

## When to use server-side catalogs vs YAML files

| Use case                                       | Where the catalog lives        |
|------------------------------------------------|--------------------------------|
| Single team, single environment, hand-edited   | YAML in your repo, no RPC      |
| Multi-tenant SaaS, per-tenant catalogs         | Server-side via `CatalogService` |
| Need an audit trail of every change            | Server-side                    |
| Need to A/B-test a class definition            | Server-side (evolution)        |
| Need catalog versions ≥ 1000                   | Server-side                    |
| Air-gapped / no network                        | YAML                           |

The two modes are interoperable : a server-registered catalog can
be exported with `EfeAdminClient.get_catalog(...).content` and fed
into a local `AnchorCatalog` for offline classification.

---

## Prerequisites

* `efe-server` ≥ v0.1.12 with the new `CatalogService` enabled
  (the binary loads it unconditionally — see
  `crates/efe-server/src/catalog/`).
* A JWT carrying the right scopes :

  * `admin:catalog:read`  — for `list`, `versions`, `get`.
  * `admin:catalog:write` — for `register`, `evolve`, `delete`.

  The two scopes are **independent of** `admin:cone:*` (you can
  hand out catalog-write to a curator without giving them
  cone-write access). Configure scopes in your IdP per
  `docs/operations/jwt-scopes.md`.

---

## Register a v1 catalog (Python SDK)

```python
from efe_tool import EfeAdminClient, RegisteredCatalog

CONTENT = {
    "embedder": {
        "family": "sentence-transformers",
        "model": "paraphrase-multilingual-MiniLM-L12-v2",
        "dim": 384,
        "normalize": "l2",
    },
    "classes": [
        {"name": "billing",
         "description": "Charge / refund issues.",
         "canonical_sentences": [
             "I want a refund",
             "my card was charged twice",
             "please cancel my subscription",
         ],
         "color": "#ef4444"},
        {"name": "login",
         "description": "Authentication problems.",
         "canonical_sentences": [
             "I cannot log in",
             "password reset failed",
         ],
         "color": "#3b82f6"},
    ],
    "cone_policy": {"max_projection_distance": 0.7},
    "thresholds": {"confidence_high": 0.8, "confidence_medium": 0.5},
}

with EfeAdminClient(
    endpoint="efe.example.com:50051",
    jwt=os.environ["EFE_TOKEN"],
) as admin:
    reg: RegisteredCatalog = admin.register_catalog(
        tenant="acme",
        name="support_triage",
        version="v1",                     # informational
        content=CONTENT,
        description="Initial import (Q4 2026 prod cutover).",
        labels={"env": "prod", "owner": "support-eng"},
    )
    print(f"registered {reg.key} (v{reg.version})")
    print(f"sha256 = {reg.metadata.content_sha256}")
```

The server :

1. Validates the schema (≥ 2 classes, embedder fingerprint matches a
   known model, etc.).
2. Computes `content_sha256` over the canonicalised JSON form. This
   digest is the **immutability witness** — it pins the bytes the
   server agreed to commit.
3. Assigns the next integer version monotonically per
   `(tenant, name)`. The first call always returns `v1`.
4. Stamps `created_at` (UTC) and `created_by` (the JWT's `sub`).

The returned `RegisteredCatalog` is a [frozen dataclass][frozen] —
safe to share across threads.

[frozen]: https://docs.python.org/3/library/dataclasses.html#frozen-instances

---

## Register from the CLI

The CLI is a thin shell over the SDK :

```bash
efe-admin catalog register \
    --name support_triage \
    --version v1 \
    --content catalogs/support_triage.yaml \
    --description "Initial import (Q4 2026 prod cutover)." \
    --endpoint efe.example.com:50051 \
    --tls --jwt "$EFE_TOKEN" \
    --pretty
```

Output (JSON, pretty-printed) :

```json
{
  "schema_version": "1.0",
  "request_id": "5e2f...",
  "action": "register",
  "latency_ms": 142,
  "key": "acme/support_triage@v1",
  "tenant": "acme",
  "name": "support_triage",
  "version": 1,
  "metadata": {
    "created_at": "2026-04-26T14:08:11.412Z",
    "created_by": "alice@acme.com",
    "evolution_op": "register",
    "parent_key": "",
    "content_sha256": "deadbeef...",
    "labels": {"env": "prod", "owner": "support-eng"}
  },
  "content": { ... }
}
```

Pass `--labels labels.json` to attach key-value labels (a JSON
object with string values).

---

## Discover existing catalogs

### List one row per `(tenant, name)`

```bash
efe-admin catalog list --pretty
```

```json
{
  "n_catalogs": 3,
  "catalogs": [
    {"tenant": "acme", "name": "billing", "latest_version": 7,
     "num_classes": 5, "num_anchors": 27},
    {"tenant": "acme", "name": "support_triage", "latest_version": 3,
     "num_classes": 4, "num_anchors": 14},
    {"tenant": "acme", "name": "moderation_v2", "latest_version": 1,
     "num_classes": 8, "num_anchors": 42}
  ]
}
```

The server filters by the JWT's `tenant_id` claim — **you cannot see
catalogs from a foreign tenant**, ever. This is the same isolation
guarantee documented in [`d11_per_tenant_isolation_architecture.md`
§ 4](../../bench/d11_per_tenant_isolation_architecture.md).

### List all versions for one catalog

```bash
efe-admin catalog versions --name support_triage --pretty
```

```json
{
  "n_versions": 3,
  "keys": [
    "acme/support_triage@v1",
    "acme/support_triage@v2",
    "acme/support_triage@v3"
  ]
}
```

Versions are returned ascending by integer version — the latest is
always the last entry.

---

## Fetch a specific version

```bash
efe-admin catalog get acme/support_triage@v3 --pretty
```

The full canonical body (content + metadata) is on stdout.

Pass `--include-compiled` to also receive per-class compiled-anchor
metadata. The `anchors_blob` floats are NEVER included in this view
— even for the catalog owner — because the wire payload would
balloon for a 100-class catalog. To rehydrate the floats, talk to
the server's in-memory cache via the internal RPC documented in
[`d12_catalog_admin_versioning_architecture.md` § 9.2](../../bench/d12_catalog_admin_versioning_architecture.md).

---

## Use a server-registered catalog for client-side classify

You can take a `RegisteredCatalog.content` dict and feed it to a
local `AnchorCatalog` for offline classification :

```python
from efe_tool import AnchorCatalog, EfeAdminClient

with EfeAdminClient(endpoint="...", jwt="...") as admin:
    reg = admin.get_catalog("acme/support_triage@v3")

# RegisteredCatalog.content is the YAML-shaped dict.
catalog = AnchorCatalog.from_dict({
    "apiVersion": "efe.mode1/v1",
    "kind": "AnchorCatalog",
    "metadata": {"name": reg.name, "version": str(reg.version)},
    **reg.content,
})

# Now classify locally without the server:
result = catalog.classify("I cannot log in")
```

This is useful for batch jobs that don't want the gRPC round-trip.

---

## Delete a version

Delete a single version :

```bash
efe-admin catalog delete acme/support_triage@v1
```

The server refuses if any newer version still references this one
as `parent_key`. To cascade-delete the chain, pass `--force` :

```bash
efe-admin catalog delete acme/support_triage@v1 --force
```

> **Warning** : `--force` breaks any classifier still pinned to that
> version. Catalog deletion is irreversible — there's no soft-delete
> tier in v0.1.12.

---

## Operational guardrails

* **Quotas** : per-tenant, the server caps the number of live
  versions at 1000 (tunable via `EFE_CATALOG_MAX_VERSIONS_PER_NAME`)
  and the total number of `(tenant, name)` tuples at 100. Hitting
  the limit returns `RESOURCE_EXHAUSTED` → the SDK raises
  `EfeRateLimitError`.
* **Immutability witness** : `content_sha256` is computed
  server-side over a canonical JSON encoding of `CatalogContent`.
  If you ever see a registered catalog whose returned
  `content_sha256` doesn't match what you sent, that's a forensic
  signal — file a bug.
* **Audit log** : every `register` / `evolve` / `delete` emits a
  structured log line `{tenant, name, version, op, sub, ...}` to
  the server's audit sink (default : same logger as request log,
  but see `EFE_AUDIT_SINK` to redirect).

---

## Troubleshooting

| Symptom                                         | Likely cause                       | Fix                                                |
|-------------------------------------------------|------------------------------------|----------------------------------------------------|
| `EfeAuthError: PERMISSION_DENIED`               | JWT lacks `admin:catalog:write`    | Re-issue JWT with the right scope                  |
| `EfeInvalidArgumentError: ...`                  | YAML schema violation              | Run `efe-admin validate <yaml>`                    |
| `EfeInvalidArgumentError: tenant mismatch`      | `--tenant X` ≠ JWT `tenant_id`     | Drop `--tenant` (SDK extracts from JWT)            |
| `EfeNotFoundError`                              | Wrong canonical key                | `efe-admin catalog versions --name X`              |
| Quota exhaustion                                | ≥ 1000 versions for one catalog    | `delete --force` old branches                      |

---

## Next

* [Evolution ops in detail](./catalog-evolution.md)
* [RPC reference](../reference/catalog-rpc.md)
* `examples/05_catalog_admin.py` — full lifecycle script
