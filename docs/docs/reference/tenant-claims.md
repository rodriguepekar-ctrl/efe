# Reference — JWT tenant claims

> Sprint D11 contract reference. The IDP issuing JWTs to EFE
> partners MUST respect this contract.

## Required claims

| Claim | Type | Description |
|-------|------|-------------|
| `iss` | string | Issuer URL of the IDP |
| `aud` | string | MUST equal `efe.v1` |
| `sub` | string | User or service-account identifier |
| `exp` | integer | Unix epoch seconds, must be in the future |
| `iat` | integer | Unix epoch seconds, issued-at |
| `tenant_id` | string | Tenant identifier (custom claim) |
| `scopes` | array<string> | Allowed scopes (custom claim) |

## `tenant_id` constraints

* **Format**: `[a-z][a-z0-9_-]{0,62}` (kubectl-compatible label).
* **Stability**: stable across token rotations for the same tenant
  (don't generate a new tenant_id on every token refresh).
* **Uniqueness**: each tenant has exactly one `tenant_id`.
* **No PII**: never use customer name / email as `tenant_id` —
  use an opaque identifier from your tenant directory.

## `scopes` values

```json
{
  "scopes": ["classify:read", "admin:cone:read"]
}
```

| Scope | RPCs allowed |
|-------|--------------|
| `classify:read` | `Execute`, `ExecuteStreaming`, `DescribeSpec` |
| `classify:write` | (reserved — equivalent to `read` in v0.1.x) |
| `admin:cone:read` | `GetCone`, `ListVersions`, `ListCatalogs` |
| `admin:cone:write` | `RegisterCone`, `EvolveCone`, `DeleteCone`, `BreachHistory` |

A token with no `admin:*` scope cannot invoke any `AdminService`
RPC.

## Example token (decoded payload)

```json
{
  "iss": "https://idp.your-corp.com/",
  "aud": "efe.v1",
  "sub": "service-account-acme-classifier",
  "exp": 1714137600,
  "iat": 1714134000,
  "tenant_id": "acme",
  "scopes": ["classify:read", "admin:cone:read"]
}
```

Signed with HS256 using the active tenant signing key.

## Server-side validation

The server (`crates/efe-server/src/auth/jwt.rs`) validates :

1. Signature against multi-key store.
2. `aud` exact match.
3. `exp` > now (with 60 s clock skew).
4. `tenant_id` non-empty.
5. (Per-RPC) required scope present in `scopes`.
6. (D11 NEW) For RPCs with an explicit `tenant` field in the
   request body, `tenant == claims.tenant_id` else
   `PERMISSION_DENIED`.

## Token rotation

* Multi-key signing supported. Add a new key, ship tokens with
  it ; remove the old key after `exp_max` of all in-flight
  tokens.
* See [Operations / Key rotation](../../operations/key_rotation.md).

## Configuration on the EFE server

In the Helm chart (`config.auth.*`) :

| Key | Required | Description |
|-----|:--------:|-------------|
| `jwtAudience` | ✓ | Expected `aud` claim, must match the IDP |
| `jwtIssuer` | optional | If non-empty, validates `iss` claim |
| `secretsFile` | ✓ | Filesystem path to the multi-key file |
| `enabled` | ✓ | Tier 2 forces `true` |

The `secretsFile` is a JSON / line-delimited list of HS256 keys.
The chart's Secret resource carries it ; the SDK never sees the
keys.

## See also

* [How-to / Multi-tenant deployment](../howto/multi-tenant.md)
* [Explanation / Security model](../explanation/security-model.md)
* [Operations / Key rotation](../../operations/key_rotation.md)
