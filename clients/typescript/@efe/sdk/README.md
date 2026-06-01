# `@efe/sdk` — official TypeScript SDK for EFE

> **Sprint D24 (Φ-B)** — isomorphic TypeScript SDK for the EFE (Essaim
> Fluide Éphémère) inference platform.
>
> **Status** : Beta (`v0.3.0` private during Φ-B). GA target `v1.0.0`
> post-D40.
> **Runtimes** : Node ≥ 18 (full surface) + modern browsers (subset
> idempotent only).
> **License** : Apache-2.0
> **Wire contract** : `efe.v1` proto family — same `.proto` consumed
> by Python (M8) + Go (M9) SDKs.

---

## Install

### Private (during Φ-B)

```bash
# Configure private scoped registry (one-time per machine).
echo "@efe:registry=https://npm.pkg.github.com" >> ~/.npmrc
echo "//npm.pkg.github.com/:_authToken=$NPM_TOKEN" >> ~/.npmrc

pnpm add @efe/sdk
# OR npm/yarn equivalents.
```

### Public (post-GA)

```bash
pnpm add @efe/sdk
# (no registry tweak — public npm.org)
```

---

## Quickstart — Node

```ts
import { EfeClient } from '@efe/sdk/node';

const client = new EfeClient({
  endpoint: process.env.EFE_ENDPOINT!,
  auth: { kind: 'jwt', token: process.env.EFE_JWT_TOKEN! },
  deadline: 10_000,
});

try {
  const verdict = await client.classify('I want a refund');
  console.log(`label=${verdict.label} confidence=${verdict.confidence}`);
} finally {
  await client.close();
}
```

## Quickstart — Browser

```ts
import { EfeClient } from '@efe/sdk/web';

const client = new EfeClient({
  endpoint: 'https://api.efe.ai',          // points at Envoy gRPC-Web :50052
  auth: { kind: 'jwt', token: localStorage.getItem('efe_jwt')! },
});

const verdict = await client.classify('I want a refund');
// `verdict` is fully typed — Verdict interface, 10 fields cross-language pinned.
```

---

## Public API surface

### Classes

| Class | Runtime | Purpose |
|---|---|---|
| `EfeClient` | Node | full surface (Classify + Healthcheck + ClassifyStream + admin) |
| `EfeWebClient` (re-exported as `EfeClient` in browser context) | Browser | subset (Classify + Healthcheck only) |
| `MockEfeClient` | universal (test fixture) | drop-in replacement, no gRPC |
| `ExponentialBackoff` | universal | retry backoff impl |

### Types

| Type | Description |
|---|---|
| `Verdict` | classification verdict (10 fields, immutable) |
| `StreamItem` | one item in `classifyStream` |
| `ClientOptions` | constructor config |
| `AuthConfig` | discriminated union (jwt / mtls / none) |
| `RetryConfig` | retry policy |
| `RetryBackoff` | backoff interface |

### Errors (10 sentinels)

| Class | gRPC code | Retryable |
|---|---|:-:|
| `ErrUnauthenticated` | UNAUTHENTICATED (16) | no |
| `ErrPermissionDenied` | PERMISSION_DENIED (7) | no |
| `ErrCatalogNotFound` | NOT_FOUND (5) | no |
| `ErrCatalogInvalid` | FAILED_PRECONDITION (9) | no |
| `ErrTimeout` | DEADLINE_EXCEEDED (4) | yes |
| `ErrUnavailable` | UNAVAILABLE (14) | yes |
| `ErrRateLimited` | RESOURCE_EXHAUSTED (8) | yes |
| `ErrInvalidArgument` | INVALID_ARGUMENT (3) | no |
| `ErrInternal` | INTERNAL (13) / UNKNOWN (2) | no |
| `ErrClosed` | (local) | no |

Use `errors.is`-style :

```ts
import { ErrTimeout, ErrRateLimited, isRetryable, errorCode } from '@efe/sdk';

try {
  await client.classify(text);
} catch (err) {
  if (err instanceof ErrTimeout) {
    // 504
  } else if (err instanceof ErrRateLimited) {
    // 429 — backoff
  }
  if (isRetryable(err)) {
    // generic retryable handler
  }
  console.log(errorCode(err)); // "timeout" | "rate_limited" | ...
}
```

### Helpers

| Function | Description |
|---|---|
| `isRetryable(err)` | true iff err ∈ {ErrUnavailable, ErrTimeout, ErrRateLimited} |
| `errorCode(err)` | sentinel → short string label (for metrics) |

### Constants

| Const | Value |
|---|---|
| `Version` | `'0.3.0'` (D24 ship) |

---

## Three framework examples

Live under `examples/` — each is a **separate Go sub-module** so the
SDK itself does NOT pull React/Vue/Express as transitive deps :

```
clients/typescript/@efe/sdk/examples/
├── react/                   # React 18 + Vite + TS
├── vue/                     # Vue 3 + Vite + TS
└── express/                 # Express + tsx
```

Run any of them :

```bash
cd examples/react
EFE_ENDPOINT=https://staging.efe.ai \
EFE_JWT_TOKEN=<token> \
pnpm dev
```

---

## Mock for unit tests

```ts
import { MockEfeClient, newVerdict } from '@efe/sdk/mock';

const mock = new MockEfeClient();
mock.setVerdict(newVerdict({ label: 'fraud', confidence: 0.92 }));

const verdict = await mock.classify('test input');
expect(verdict.label).toBe('fraud');
expect(mock.getCalls()).toEqual(['test input']);
```

For tests that exercise the FULL gRPC stack (interceptors, retry,
OTel), use `createMockServer` :

```ts
import { createMockServer } from '@efe/sdk/mock';
import { EfeClient } from '@efe/sdk/node';

const server = await createMockServer();
const client = new EfeClient({ endpoint: server.endpoint });

server.onClassify(async (req) => ({
  output: { text: 'ok' },
  confidence: 0.91,
  // ...
}));

const verdict = await client.classify('hello');

await client.close();
await server.close();
```

---

## Browser limitations

The browser bundle (`@efe/sdk/web`) intentionally exposes a **subset
of the API surface** :

| Method | Node | Browser |
|---|:-:|:-:|
| `classify(text)` | ✓ | ✓ |
| `healthcheck()` | ✓ | ✓ |
| `classifyStream(texts)` | ✓ | ✗ (deferred D26+) |
| `registerCatalog(...)` | ✓ | ✗ (admin → use BFF) |
| `evolveCatalog(...)` | ✓ | ✗ |
| `deleteCatalog(...)` | ✓ | ✗ |

**Why** : browsers can't speak HTTP/2 client-streaming or bidi gRPC ;
non-idempotent RPCs (writes, admin) should never originate from a
browser anyway (CSRF + scope-elevation risk). Use a Node BFF layer
that consumes `@efe/sdk/node` for those.

The browser also requires the EFE server to expose port `:50052`
(gRPC-Web translator, e.g. via Envoy `envoy.filters.http.grpc_web`)
with appropriate CORS headers.

---

## Development

```bash
# 1. Install workspace deps.
pnpm install

# 2. Generate proto stubs (one-time + on proto changes).
pnpm gen:proto

# 3. Type-check.
pnpm typecheck

# 4. Test (unit + integration + coverage).
pnpm test

# 5. E2E browser tests (requires Envoy).
pnpm test:e2e

# 6. Bundle size budget check.
pnpm bundlesize

# 7. Build.
pnpm build

# 8. Lint.
pnpm lint
```

---

## SLOs

| ID | Target |
|:--:|---|
| `SLO-TS-1` | `new EfeClient(...)` cold start < 30 ms |
| `SLO-TS-2` | `classify` p99 ≤ server p99 + 30 ms overhead |
| `SLO-TS-3` | Bundle size browser ≤ 50 KB gz |
| `SLO-TS-4` | Bundle size Node ≤ 250 KB gz |
| `SLO-TS-5` | Tree-shake : Verdict-only import < 5 KB gz |
| `SLO-TS-6` | TypeScript ≥ 5.4 supported |
| `SLO-TS-7` | typedoc clean |
| `SLO-TS-8` | Coverage ≥ 80% lines |

---

## Cross-language consistency

The TypeScript SDK is the third in a family ; all three SDKs share
the same `efe.v1` proto family + the same 10 sentinel errors + the
same Verdict 10 fields :

| Aspect | Python (M8) | Go (M9) | TypeScript (M10) |
|---|---|---|---|
| Module path | `clients/python/efe_tool/` | `clients/go/efe-sdk/` | `clients/typescript/@efe/sdk/` |
| Public type | `Verdict @dataclass(frozen=True)` | `type Verdict struct {...}` | `interface Verdict {...}` |
| Sentinel base | `EfeError(Exception)` | `var Err* = errors.New(...)` | `class EfeError extends Error` |
| 10 sentinels | ✓ | ✓ | ✓ |
| 5 Prom metrics | ✓ | ✓ | ✓ Node only |
| OTel auto | ✓ | ✓ | ✓ universal |
| Retry policy | ✓ | ✓ | ✓ Node only |
| mTLS support | ✓ | ✓ | ✓ Node only |

Pin test `test_d24_typescript_sdk_verdict_field_set_matches_m8_m9`
anchors that 10-field Verdict match across the 3 SDKs.

---

## References

* [`docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md`](../../../../docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md)
  — sprint D24 architecture (~3 345 lignes).
* [`protos/efe/v1/`](../../../../protos/efe/v1/) — wire contract.
* [`docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md`](../../../../docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md)
  — sister SDK (Python, M8).
* [`docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md`](../../../../docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md)
  — sister SDK (Go, M9).
