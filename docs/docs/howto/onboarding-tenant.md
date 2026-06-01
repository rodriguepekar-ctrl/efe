# How to onboard a tenant (Sprint D15)

This guide walks the **operator** through registering, activating,
and (eventually) suspending a tenant in an EFE deployment that uses
the OIDC validation path. It assumes :

- Postgres backend is wired (`config.catalog.backend: postgres`) ;
- the `tenants` migration `0004_tenants.sql` has been applied ;
- you have access to the `efe-onboard` CLI (built from
  `crates/efe-server/src/bin/efe_onboard.rs`).

If your deployment still uses the legacy HS256 path, see the
[Auth howto](auth.md) instead — tenants then live in JWT claims and
require no row in the registry.

---

## Mental model

```
┌─ pending ─┐
▼           ▼
active ───► suspended
▲           │
└───────────┘   (re-activation allowed)
```

A tenant moves through three states. Tokens for non-`active`
tenants are rejected with `permission_denied` — **but only when the
EFE deployment uses `EFE_CATALOG_BACKEND=postgres`**, which is what
wires the `TenantStatusGate` into the OIDC validator (audit fix
D15-C1). On Memory backends (test / single-pod dev), the registry
table doesn't exist and tokens for any tenant resolved by an issuer
will validate, regardless of registry status — the runtime emits a
`tracing::warn!` at boot to make this explicit.

> **D15-bis backlog — audit trail.** State transitions land
> timestamps in the `tenants` table (`created_at`, `activated_at`,
> `suspended_at`) but are **not** hash-chained in the HMAC audit
> log (no SQL trigger, no Rust-side emission). Treat the registry as
> append-mostly governance metadata, not as cryptographic audit
> evidence — pending the D15-bis follow-up that extends the audit
> emitter schema with a tenant-lifecycle action class.

---

## Step 1 — Register the tenant

Pick a canonical id matching `^[a-z0-9_-]{1,63}$` (same regex as the
`catalogs.tenant` column from D14). Avoid uppercase or unicode —
they're rejected at INSERT time.

```sh
./efe-onboard register \
    --tenant-id acme \
    --display-name "Acme Corp" \
    --idp-issuer "https://acme.auth0.com/" \
    --idp-strategy claim \
    --pg-url "$EFE_PG_URL"
```

The row is inserted with `status='pending'`. Tokens for `acme` are
still rejected at this point — this is intentional, so a typo'd
issuer URL doesn't bypass the controlling activation step.

**`--idp-strategy claim`** : the tenant id will be read from the JWT
claim configured in `oidc-issuers.json` (default `tenant_id`). Use
this for multi-tenant Auth0 tenants, Okta orgs, or any IdP where one
issuer hosts multiple customer tenants.

**`--idp-strategy issuer`** : the issuer URL maps directly to one
fixed tenant id. Use this for Auth0 organizations or Keycloak realms
where each customer has a dedicated issuer URL. In that case, also
pass `--idp-meta` with vendor-specific keys :

```sh
./efe-onboard register \
    --tenant-id acme \
    --display-name "Acme Corp" \
    --idp-issuer "https://login.acme.example/realms/acme" \
    --idp-strategy issuer \
    --idp-meta '{"keycloak_realm":"acme","keycloak_client_id":"efe-resource-server"}'
```

`idp_meta` is an opaque JSONB column — the runtime never reads it.
Stash whatever the operator team needs for vendor-specific tooling
(audit context, Terraform mapping, etc.).

---

## Step 2 — Verify with `list`

```sh
./efe-onboard list
```

Output :

```
TENANT_ID            STATUS     ISSUER                         STRATEGY   DISPLAY_NAME
--------------------------------------------------------------------------------------------------------
acme                 pending    https://acme.auth0.com/        claim      Acme Corp
```

The `--json` flag emits machine-readable JSON for scripts.

---

## Step 3 — Add the issuer to `oidc-issuers.json`

The CLI manages the **registry table**. The runtime also needs to
know how to validate that issuer's tokens — that comes from the
Secret-mounted `oidc-issuers.json`. Append :

```json
{
  "issuer": "https://acme.auth0.com/",
  "audience": "efe.v1",
  "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
  "alg_whitelist": ["RS256"],
  "tenant_strategy": "claim",
  "tenant_claim": "tenant_id",
  "max_token_age_s": 3600,
  "leeway_s": 30
}
```

Then re-roll the Secret + restart the deployment. **The runtime
loads `oidc-issuers.json` at boot time only** — there is no live
reload (yet ; see ADR-D15-1 for the rationale).

```sh
kubectl -n efe create secret generic efe-oidc-issuers \
    --from-file=oidc-issuers.json=./oidc-issuers.json \
    --dry-run=client -o yaml | kubectl apply -f -
kubectl -n efe rollout restart deploy/efe-server
```

---

## Step 4 — Run the preflight check

Before flipping the tenant to `active`, sanity-check both paths :

```sh
./efe-onboard preflight \
    --issuers-file ./oidc-issuers.json \
    --pg-url "$EFE_PG_URL"
```

Output looks like :

```
✓ issuers_file OK : 1 issuers
    - https://acme.auth0.com/  audience=efe.v1  jwks=https://...  alg=["RS256"]  strategy=Claim
✓ postgres reachable : 1 pending / 0 active / 0 suspended
```

If preflight returns exit code `1`, the issuers file is malformed —
fix it before activation. Exit code `2` means the database is
unreachable (network / auth / migrations).

---

## Step 5 — Activate

```sh
./efe-onboard activate acme
```

The row's `status` flips to `active`, `activated_at` is set to
`now()`, and the trigger emits a row in `audit_events` (signed via
D14-bis HMAC chain). Tokens for `acme` are now accepted.

The Prometheus gauge `efe_tenants_total{status}` updates within
~30 s (the periodic refresh job picks up the new state).

---

## Step 6 — Test from the client side

Use a real OIDC token from your IdP (see the IdP-specific guides
[Auth0](idp-integration-auth0.md) and [Keycloak](idp-integration-keycloak.md))
or a service-to-service token if you've configured a machine
identity. Hit any authenticated RPC :

```sh
TOKEN=$(curl -s ... | jq -r .access_token)
grpcurl -H "authorization: Bearer $TOKEN" \
    -d '{"text":"hello"}' \
    efe.example.com:50051 \
    efe.v1.SwarmService/ExecuteOnce
```

If the response is `{"text": ...}`, you're live.

---

## Step 7 — Suspending a tenant

When a contract ends or you need to cut access for a security event :

```sh
./efe-onboard suspend acme
```

`status` flips to `suspended`, `suspended_at` is set, and tokens are
rejected within the JWKS cache TTL (default 5 min — older tokens
already validated may continue serving until they expire ; this is
expected RFC 6749 behaviour).

To accelerate the kill switch, also rotate the IdP signing key —
that immediately invalidates every outstanding token at the
cryptographic level, and the JWKS cache picks up the new keys at
the next refresh tick.

---

## Troubleshooting

### "tenant resolution failed"

The token validates cryptographically but the tenant claim is
missing or doesn't match the `^[a-z0-9_-]{1,63}$` regex. Check :

- which `tenant_strategy` was configured (`claim` vs `issuer`) ;
- the JWT actually carries the claim listed in `tenant_claim` ;
- the value matches the regex (lowercase, no spaces, no dots).

### "unknown issuer"

The JWT's `iss` claim doesn't match any entry in
`oidc-issuers.json`. The match is **byte-for-byte** — common gotcha
is the trailing slash (`https://acme.auth0.com/` ≠
`https://acme.auth0.com`).

### "JWKS fetch failed" loop in logs

The IdP is unreachable. The runtime serves stale-but-within-grace
keys for `grace_stale_s` (default 1 h) ; past that, every token is
rejected. Check IdP status page first, then network policies.

### `efe_tenants_total{status="pending"}` not decreasing

The activate command silently no-ops if Postgres rejected the UPDATE.
Run :

```sh
./efe-onboard get acme --json
```

If `status` is still `pending`, check the postgres logs for the
trigger output — a likely cause is a constraint violation on
`tenants.idp_issuer` (non-empty CHECK).

---

## Related references

- [`docs/bench/d15_onboarding_idp_architecture.md`](../../bench/d15_onboarding_idp_architecture.md)
  — full design rationale, threat model, alert catalogue.
- [`crates/efe-server/migrations/0004_tenants.sql`](../../../crates/efe-server/migrations/0004_tenants.sql)
  — schema source of truth.
- [`deploy/config/oidc-issuers.example.json`](../../../deploy/config/oidc-issuers.example.json)
  — vendor templates (Auth0, Keycloak, Okta).
