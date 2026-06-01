# How to integrate Keycloak with EFE (Sprint D15)

Step-by-step guide for wiring a self-hosted Keycloak realm to EFE's
OIDC validator. Tested against Keycloak 22.x and 24.x.

If you're using Auth0, see [the Auth0 guide](idp-integration-auth0.md)
instead — the high-level shape is the same but the URLs and the
"organizations" concept differ.

---

## Prerequisites

- Keycloak admin access on a realm dedicated to EFE (recommended
  isolation : one realm per EFE deployment).
- TLS-terminated Keycloak URL (`https://...`). EFE rejects `http://`
  unless `EFE_OIDC_ALLOW_HTTP=true` (Tier 1 dev only — never in
  production).
- A Postgres instance with `0004_tenants.sql` applied.

---

## Step 1 — Create the EFE client

In the Keycloak admin console :

1. **Clients → Create client**.
2. Set :
   - *Client ID* : `efe-resource-server`
   - *Client type* : `OpenID Connect`
   - *Client authentication* : ON (we'll use service accounts +
     authorization code flow for users).
3. **Capability config** :
   - Standard flow : ON
   - Direct access grants : OFF (production)
   - Service accounts roles : ON (for machine-to-machine tokens).
4. Save.

In the **Settings** tab :

- *Valid redirect URIs* : `https://your-app.example.com/*`
- *Web origins* : `https://your-app.example.com`

---

## Step 2 — Choose a tenant strategy

Two options, picked once per Keycloak realm :

### Option A — One realm = one tenant (`tenant_strategy = "issuer"`)

Use this when each customer gets a dedicated Keycloak realm.

The realm URL maps directly to a fixed tenant id ; there's no need
for a custom claim.

```json
{
  "issuer": "https://keycloak.example.com/realms/acme",
  "audience": "efe-resource-server",
  "jwks_url": "https://keycloak.example.com/realms/acme/protocol/openid-connect/certs",
  "alg_whitelist": ["RS256"],
  "tenant_strategy": "issuer",
  "tenant_id_for_issuer": "acme",
  "max_token_age_s": 3600,
  "leeway_s": 30
}
```

### Option B — One realm = many tenants (`tenant_strategy = "claim"`)

Use this when the same realm hosts multiple customer tenants. You
need a custom user attribute *and* a token mapper.

#### B.1 — User attribute

In the Keycloak admin console :

1. **Realm settings → User profile → Create attribute**.
2. *Name* : `tenant_id`. *Display name* : `EFE tenant id`.
3. *Required* : ON. *Permissions* : at least *view: user, admin* —
   the user shouldn't be able to edit their own tenant assignment.
4. Save.

For each user, set their `tenant_id` attribute (CLI or admin UI).

#### B.2 — Token mapper

1. **Clients → efe-resource-server → Client scopes → efe-resource-server-dedicated**.
2. **Mappers → Configure a new mapper → User attribute**.
3. Set :
   - *Name* : `tenant_id`
   - *User attribute* : `tenant_id`
   - *Token claim name* : `tenant_id`
   - *Claim JSON type* : `String`
   - *Add to ID token* : OFF
   - *Add to access token* : ON  ← critical
   - *Add to userinfo* : optional
4. Save.

Test by issuing a token (Direct grant on a dev user) and decoding
the access token at jwt.io — `tenant_id` should appear under the
custom claims.

#### B.3 — `oidc-issuers.json`

```json
{
  "issuer": "https://keycloak.example.com/realms/efe",
  "audience": "efe-resource-server",
  "jwks_url": "https://keycloak.example.com/realms/efe/protocol/openid-connect/certs",
  "alg_whitelist": ["RS256"],
  "tenant_strategy": "claim",
  "tenant_claim": "tenant_id",
  "max_token_age_s": 3600,
  "leeway_s": 30
}
```

---

## Step 3 — Match the audience

Keycloak's default access tokens use `aud = "account"`. EFE's
validator requires `aud = "efe-resource-server"` (or whatever you
named the client). Configure the **Audience mapper** :

1. **Clients → efe-resource-server → Client scopes → efe-resource-server-dedicated**.
2. **Mappers → Configure a new mapper → Audience**.
3. Set :
   - *Name* : `efe-resource-server-aud`
   - *Included client audience* : `efe-resource-server`
   - *Add to access token* : ON
4. Save.

Now the access token's `aud` is `["efe-resource-server"]`. EFE
accepts both single string and array shapes per RFC 7519.

---

## Step 4 — Push to EFE

Append to `oidc-issuers.json`, then redeploy :

```sh
kubectl -n efe create secret generic efe-oidc-issuers \
    --from-file=oidc-issuers.json=./oidc-issuers.json \
    --dry-run=client -o yaml | kubectl apply -f -
kubectl -n efe rollout restart deploy/efe-server
```

Run the preflight :

```sh
./efe-onboard preflight \
    --issuers-file ./oidc-issuers.json \
    --pg-url "$EFE_PG_URL"
```

Then register + activate the tenant (see
[onboarding-tenant.md](onboarding-tenant.md)).

---

## Step 5 — End-to-end test

Use the Direct grant flow (dev only — production should use auth
code or client credentials) :

```sh
TOKEN=$(curl -s -X POST \
    "https://keycloak.example.com/realms/efe/protocol/openid-connect/token" \
    -d grant_type=password \
    -d client_id=efe-resource-server \
    -d client_secret="$KC_SECRET" \
    -d username=alice -d password=... \
    -d scope="openid swarm:execute" \
    | jq -r .access_token)

grpcurl -H "authorization: Bearer $TOKEN" \
    -d '{"text":"hello"}' \
    efe.example.com:50051 \
    efe.v1.SwarmService/ExecuteOnce
```

A successful response confirms the chain :
JWKS fetch → JWT decode → tenant resolve → scope check → handler.

---

## Operational notes

### Key rotation

Keycloak rotates signing keys on a schedule (configurable per realm
under *Realm settings → Keys → Providers*). EFE's JWKS cache picks
up the new key at the next refresh (default 5 min) ; the old key
remains in the cache until it falls off the JWKS document, which
Keycloak controls. Tokens minted under the previous key continue to
validate until they expire — by design.

### Realm migration

If you move a realm to a new Keycloak instance, the issuer URL
changes. EFE matches `iss` byte-for-byte, so :

1. Add the new entry to `oidc-issuers.json` ;
2. Keep the old entry until every outstanding token has expired
   (typically 1 token lifetime later) ;
3. Remove the old entry on the next deploy.

### Service accounts

Keycloak's service-account tokens carry no `tenant_id` claim by
default. With `tenant_strategy = "issuer"` they work out of the box
(the realm = the tenant). With `tenant_strategy = "claim"` you must
set the `tenant_id` attribute on the *service account user* —
**Clients → efe-resource-server → Service Account Roles → User
attributes**.

### Role mapping → scopes

EFE's scope model is a flat list (`swarm:execute`, `admin:catalog:read`,
…). Keycloak's role mapping naturally maps : assign a role
`swarm-execute` and configure the scope mapper to translate roles
to the OIDC `scope` claim. The runtime accepts either `scope`
(string, space-delimited) or `scopes` (JSON array).

---

## Troubleshooting

### `outcome="bad_audience"`

Audience claim mismatch. Either Keycloak isn't injecting the
`efe-resource-server` audience, or your `oidc-issuers.json` lists
the wrong audience value. Decode the access token at jwt.io and
inspect `aud`.

### `outcome="tenant_resolution_failed"` with strategy=claim

The `tenant_id` claim is missing from the access token. Two common
causes :

1. The mapper is on the *ID token* but not the *access token* — the
   resource server only sees the access token.
2. The user has no `tenant_id` attribute set.

### `outcome="unknown_issuer"` after a Keycloak upgrade

Keycloak occasionally adjusts the trailing-slash convention on
`iss`. Check both : `https://keycloak.example.com/realms/efe` and
`https://keycloak.example.com/realms/efe/`. Mirror the exact value
in `oidc-issuers.json`.

---

## Related references

- [`docs/bench/d15_onboarding_idp_architecture.md`](../../bench/d15_onboarding_idp_architecture.md)
  — multi-issuer dispatch, threat model.
- [Auth0 integration](idp-integration-auth0.md) — equivalent guide
  for Auth0.
- [`onboarding-tenant.md`](onboarding-tenant.md) — the registry CRUD
  workflow that sits in front of every IdP integration.
