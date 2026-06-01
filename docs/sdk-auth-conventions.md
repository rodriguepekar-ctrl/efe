# SDK Authentication Conventions (Φ-C-SDK Polyglot)

> Sprint **Φ-C-SDK Completion (D31)** — artefact #20.
> Cf. `docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md` § 10.

## Mission

Standardiser les schémas d'authentification supportés par les 4 SDKs.
Chaque SDK doit accepter **au minimum** les 4 modes ci-dessous.

## Auth modes canoniques

| Mode | Use case | Mechanism |
|------|----------|-----------|
| **TLS** | Default for cloud | Server cert validé contre CA bundle |
| **mTLS** | Enterprise / zero-trust | Client cert + key |
| **Bearer token** | API key for SaaS | `authorization: Bearer <token>` metadata |
| **Insecure** | Dev only | No TLS, explicit opt-in |

## API per SDK

### Python

```python
# TLS (default).
client = efe.EfeClient(host="efe.example.com:443")

# mTLS.
client = efe.EfeClient(
    host="efe.example.com:443",
    cert="/etc/efe/client.pem",
    key="/etc/efe/client-key.pem",
    ca="/etc/efe/ca.pem",
)

# Bearer token.
client = efe.EfeClient(
    host="efe.example.com:443",
    token="sk-prod-abc123",
)

# Insecure (dev only).
client = efe.EfeClient(host="localhost:50051", insecure=True)
```

### Go

```go
// TLS (default).
client, _ := efe.NewClient("efe.example.com:443")

// mTLS.
client, _ := efe.NewClient("efe.example.com:443",
    efe.WithMTLS("/etc/efe/client.pem", "/etc/efe/client-key.pem", "/etc/efe/ca.pem"))

// Bearer token.
client, _ := efe.NewClient("efe.example.com:443",
    efe.WithBearerToken("sk-prod-abc123"))

// Insecure.
client, _ := efe.NewClient("localhost:50051", efe.WithInsecure())
```

### TypeScript

```typescript
// TLS (default).
const client = new EfeClient({ host: "efe.example.com:443" });

// mTLS.
const client = new EfeClient({
  host: "efe.example.com:443",
  mtls: {
    cert: "/etc/efe/client.pem",
    key: "/etc/efe/client-key.pem",
    ca: "/etc/efe/ca.pem",
  },
});

// Bearer token.
const client = new EfeClient({
  host: "efe.example.com:443",
  token: "sk-prod-abc123",
});

// Insecure.
const client = new EfeClient({
  host: "localhost:50051",
  insecure: true,
});
```

### Java

```java
// TLS (default).
EfeClient client = EfeClient.builder()
    .host("efe.example.com:443")
    .build();

// mTLS.
EfeClient client = EfeClient.builder()
    .host("efe.example.com:443")
    .mtls(new File("/etc/efe/client.pem"),
          new File("/etc/efe/client-key.pem"),
          new File("/etc/efe/ca.pem"))
    .build();

// Bearer token.
EfeClient client = EfeClient.builder()
    .host("efe.example.com:443")
    .token("sk-prod-abc123")
    .build();

// Insecure.
EfeClient client = EfeClient.builder()
    .host("localhost:50051")
    .insecure(true)
    .build();
```

## Token rotation

Les tokens Bearer doivent supporter la rotation **sans recréer le client** :

| SDK | Rotation API |
|-----|--------------|
| Python | `client.update_token("new-token")` |
| Go | `client.UpdateToken("new-token")` |
| TS | `client.updateToken("new-token")` |
| Java | `client.updateToken("new-token")` |

## Insecure mode safety guard

Le mode insecure DOIT logger un WARN au boot :

```
[efe-sdk] WARN : insecure mode enabled — DO NOT use in production
```

Si la variable env `EFE_ALLOW_INSECURE_PROD=false` est set, le mode
insecure DOIT throw une erreur explicite plutôt que de logger un warn.

## Cert validation

Par défaut, les SDKs valident :
* Le server cert chain contre le CA bundle système.
* Le hostname matches le SAN ou CN du cert.
* La cert n'est pas expirée.

Pour override (= self-signed dev cert) :

```python
# Python : override via SSLContext.
import ssl
ctx = ssl.create_default_context(cafile="/path/to/custom-ca.pem")
client = efe.EfeClient(host="...", ssl_context=ctx)
```

```go
// Go : override via tls.Config.
tlsCfg := &tls.Config{InsecureSkipVerify: true}  // dev only !
client, _ := efe.NewClient("...", efe.WithTLSConfig(tlsCfg))
```

## Pin test

`test_phi_c_sdk_convention_docs_exist` vérifie ce doc existe.
