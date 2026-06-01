# How-to — Authentication

EFE uses **JWT HS256** signed by your IDP. Pass the token via:

* `EFE_JWT` environment variable (preferred)
* `EfeClient(..., jwt=token)` constructor argument
* gRPC metadata `authorization: Bearer <token>` (manual)

## Setup

```bash
export EFE_JWT=$(cat /tmp/jwt)
export EFE_TENANT=production-pilot   # optional
python examples/01_quickstart_classify.py
```

## Programmatic

```python
from efe_tool import EfeClient, LocalSTEmbedder

client = EfeClient(
    endpoint="efe.your-corp.io:443",
    embedder=LocalSTEmbedder.from_env(),
    jwt=token,
    insecure=False,        # production: TLS via Service mesh
)
```

## Disable auth for local dev

The Helm chart `config.auth.enabled: false` lets you skip the JWT
during local testing. Don't ever do this in prod.

```bash
helm upgrade efe oci://ghcr.io/efe-project/charts/efe \
  --version 0.1.10 -n efe --reuse-values \
  --set config.auth.enabled=false
```

## Rotation

EFE supports zero-downtime multi-key rotation. See
[Operations / Key rotation](../../operations/key_rotation.md) for
the full procedure.

## Errors

| Symptom | gRPC code | Exception | Action |
|---------|----------|-----------|--------|
| Token absent | UNAUTHENTICATED | `EfeAuthError` | Set `EFE_JWT` |
| Token expired | UNAUTHENTICATED | `EfeAuthError` | Refresh from IDP |
| Wrong audience | UNAUTHENTICATED | `EfeAuthError` | Check `EFE_FUSION_AUTH_AUDIENCE` server-side |
| Wrong tenant | PERMISSION_DENIED | `EfeAuthError` | Check `EFE_TENANT` |

The `EfeAuthError.retryable = False` — never retry on auth failure
without first refreshing the token.
