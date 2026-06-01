# How to integrate Auth0 with EFE (Sprint D15)

Step-by-step guide for connecting an Auth0 tenant (or one or more
Auth0 *organizations* under a single tenant) to EFE's OIDC
validator.

For Keycloak, see [the Keycloak guide](idp-integration-keycloak.md).

---

## Prerequisites

- An Auth0 tenant (free tier is sufficient for evaluation).
- Auth0 admin access — you'll create one *API* and one or more
  *Applications*.
- A Postgres instance with `0004_tenants.sql` applied.

Two integration shapes :

| Shape | When |
|-------|------|
| **One Auth0 tenant = one EFE tenant** | Single-tenant SaaS, internal tools |
| **Auth0 organizations = EFE tenants** | Multi-tenant B2B SaaS |

The `oidc-issuers.json` differs slightly between the two ; both are
covered below.

---

## Step 1 — Create the API

In the Auth0 dashboard :

1. **Applications → APIs → Create API**.
2. *Name* : `EFE`.
3. *Identifier* (also called audience) : `https://api.efe.example/v1`
   or simply `efe.v1` — pick a stable identifier ; this becomes the
   `aud` claim.
4. *Signing algorithm* : **RS256** (default — never select HS256).
5. **Settings tab → Token settings → Access Token Profile** :
   - *Profile* : `Auth0`
   - Token Lifetime : 1 h or less (matches EFE's
     `max_token_age_s` default).
6. **Permissions tab** : add the EFE scopes you'll grant
   (`swarm:execute`, `admin:catalog:read`, …).

---

## Step 2 — Create one Application per consumer

Per consumer (frontend, backend service, CLI) :

1. **Applications → Applications → Create Application**.
2. *Type* : `Machine to Machine` (M2M) for service-to-service ;
   `Single Page` for browser SPAs ; etc.
3. **APIs tab** : authorise the Application against the EFE API and
   grant the relevant scopes.

---

## Step 3 — Choose a tenant strategy

### Shape A — One Auth0 tenant = one EFE tenant

Use `tenant_strategy = "issuer"` and pin the EFE tenant id in
`oidc-issuers.json`. The Auth0 tenant URL maps directly :

```json
{
  "issuer": "https://acme.auth0.com/",
  "audience": "efe.v1",
  "jwks_url": "https://acme.auth0.com/.well-known/jwks.json",
  "alg_whitelist": ["RS256"],
  "tenant_strategy": "issuer",
  "tenant_id_for_issuer": "acme",
  "max_token_age_s": 3600,
  "leeway_s": 30
}
```

`tenant_id_for_issuer` MUST match the regex `^[a-z0-9_-]{1,63}$`.

### Shape B — Auth0 organizations as EFE tenants

Auth0 *organizations* let one Auth0 tenant host many customers.
Each user authenticates against an organization, and the access
token carries `org_id`.

```json
{
  "issuer": "https://efe-shared.auth0.com/",
  "audience": "efe.v1",
  "jwks_url": "https://efe-shared.auth0.com/.well-known/jwks.json",
  "alg_whitelist": ["RS256"],
  "tenant_strategy": "claim",
  "tenant_claim": "org_id",
  "max_token_age_s": 3600,
  "leeway_s": 30
}
```

**Caveat** : Auth0's `org_id` looks like `org_AbCdEf12345` — the
first three characters and the underscore violate EFE's
`^[a-z0-9_-]{1,63}$` regex (the prefix `org_` is fine — only
uppercase fails). You have two options :

1. **Custom Action — recommended.** Add an Auth0 Action (Login flow)
   that reads `event.organization.name` (lowercase by Auth0 rule)
   and copies it into a custom claim under your namespace :

   ```js
   exports.onExecutePostLogin = async (event, api) => {
     if (event.organization) {
       api.accessToken.setCustomClaim(
         "https://efe.example/tenant",
         event.organization.name
       );
     }
   };
   ```

   Then in `oidc-issuers.json` :

   ```json
   "tenant_claim": "https://efe.example/tenant"
   ```

2. **Pre-onboard mapping** in `efe-onboard register` — set
   `--idp-meta '{"auth0_org_id":"org_AbCdEf12345"}'` for traceability,
   but use the lowercase organization name as the canonical EFE
   tenant id. The runtime resolves by name (the claim) ; the
   mapping table lives in the registry for the audit trail.

The first option is cleaner ; the second is faster if you can't
deploy an Action.

---

## Step 4 — Sanity check the token

Use the M2M Application's client credentials flow (or the SPA flow
with a real user) and decode at `https://jwt.io` :

```sh
TOKEN=$(curl -s -X POST \
    https://acme.auth0.com/oauth/token \
    -H "content-type: application/json" \
    -d "{
        \"client_id\":\"$CID\",
        \"client_secret\":\"$CSEC\",
        \"audience\":\"efe.v1\",
        \"grant_type\":\"client_credentials\"
    }" | jq -r .access_token)

# Decode payload (no verify) :
echo "$TOKEN" | cut -d. -f2 | base64 -d | jq
```

Expected fields :

```json
{
  "iss": "https://acme.auth0.com/",
  "sub": "abc@clients",
  "aud": "efe.v1",
  "iat": 1745000000,
  "exp": 1745003600,
  "scope": "swarm:execute admin:catalog:read",
  "https://efe.example/tenant": "acme"
}
```

The `iss` ends with a trailing slash on Auth0 — preserve it
verbatim in `oidc-issuers.json`.

---

## Step 5 — Push to EFE

```sh
kubectl -n efe create secret generic efe-oidc-issuers \
    --from-file=oidc-issuers.json=./oidc-issuers.json \
    --dry-run=client -o yaml | kubectl apply -f -
kubectl -n efe rollout restart deploy/efe-server
```

Then register + activate :

```sh
./efe-onboard register \
    --tenant-id acme \
    --display-name "Acme Corp" \
    --idp-issuer "https://acme.auth0.com/" \
    --idp-strategy claim \
    --idp-meta '{"auth0_application":"my-spa","auth0_api":"efe-v1"}'

./efe-onboard activate acme
```

---

## Step 6 — End-to-end test

```sh
grpcurl -H "authorization: Bearer $TOKEN" \
    -d '{"text":"hello"}' \
    efe.example.com:50051 \
    efe.v1.SwarmService/ExecuteOnce
```

---

## Operational notes

### Rate limits

Auth0's free tier rate-limits the JWKS endpoint to ~1 RPS sustained.
EFE's default 5-min cache TTL keeps you well under that. If you
serve > 200 issuers from the same Auth0 tenant (you shouldn't —
budget says ≤ 10 issuers per cluster), you may hit the limit at
boot. Stagger deploys or use the SWR window
(`grace_stale_s = 3600`) to ride out the transient.

### Custom domains

If you use an Auth0 custom domain (`https://login.example.com/`),
the `iss` in tokens minted via the custom domain is the custom
domain, not the `*.auth0.com` URL. Mirror exactly. Auth0 publishes
keys at both endpoints (the custom domain proxies to the canonical),
so any `jwks_url` works — pick one and stick to it.

### Refresh tokens

Refresh tokens are not validated by EFE — only access tokens.
Auth0's refresh token rotation policy is independent.

### M2M token caching at the client

Each grpcurl call obtains a new token, which triggers an Auth0
billing event. Your real client should cache tokens until ~30 s
before `exp`. The Auth0 SDKs do this automatically.

---

## Troubleshooting

### `outcome="invalid_signature"` after a JWKS rotation

Auth0 rotates RS256 keys ~yearly. The new public key is added to
the JWKS endpoint hours before the old one is deactivated. EFE's
cache picks up the new keys at the next refresh tick — usually no
operator action needed. If you see a hard wall of failures, force
a refresh by restarting the pod.

### `outcome="bad_audience"` with custom claim namespace

Auth0 namespaces non-standard claims to avoid collision (e.g.
`https://efe.example/tenant`). EFE accepts that ; just make sure
the namespace string in `oidc-issuers.json::tenant_claim` matches
exactly.

### `outcome="tenant_resolution_failed"` for `org_id`

The `org_id` claim violates the canonical regex (uppercase). Either :

- Use the Action approach to copy `organization.name` (always
  lowercase) ;
- Or pre-process at the IdP boundary with a different mapper.

---

## Related references

- [`docs/bench/d15_onboarding_idp_architecture.md`](../../bench/d15_onboarding_idp_architecture.md)
- [`onboarding-tenant.md`](onboarding-tenant.md)
- [Keycloak integration](idp-integration-keycloak.md)
- [Auth0 docs : organizations](https://auth0.com/docs/manage-users/organizations)
