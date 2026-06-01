# Changelog — @efe/sdk

All notable changes to this package will be documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Mapping** — `@efe/sdk@vX.Y.Z` ↔ EFE platform sprint Dx (cf. repo
> `CHANGELOG.md`). Until `v1.0.0` GA, this package ships under the
> private scope `@efe` (npm `--access=restricted`) ; from `v1.0.0`
> onwards, it goes public on https://www.npmjs.com/package/@efe/sdk.

## [0.3.0] — 2026-04-30 — Sprint D24 (Φ-B) — Initial release

### Added

* `EfeClient` (Node) — full surface : `classify`, `healthcheck`,
  `classifyStream`, `close`. Built on `@connectrpc/connect-node` +
  `@grpc/grpc-js` (HTTP/2).
* `EfeWebClient` (Browser) — subset surface : `classify`,
  `healthcheck`, `close`. Built on `@connectrpc/connect-web`
  (gRPC-Web HTTP/1.1). RPCs idempotents only.
* `Verdict` interface — 10 fields cross-language pinned (matches
  Python M8 + Go M9).
* 10 sentinel error classes (`EfeError` base + `ErrUnauthenticated`,
  `ErrPermissionDenied`, `ErrCatalogNotFound`, `ErrCatalogInvalid`,
  `ErrTimeout`, `ErrUnavailable`, `ErrRateLimited`,
  `ErrInvalidArgument`, `ErrInternal`, `ErrClosed`).
* `isRetryable(err)` + `errorCode(err)` helpers.
* `ExponentialBackoff` with full jitter (universal).
* `MockEfeClient` + `newVerdict()` + `createMockServer()` test
  fixtures (`@efe/sdk/mock`).
* 5 Prometheus collectors (Node only) — names match Go SDK :
  `efe_sdk_classify_total`, `efe_sdk_classify_latency_seconds`,
  `efe_sdk_classify_errors_total`, `efe_sdk_in_flight`,
  `efe_sdk_retry_attempts_total`.
* OTel auto-instrumentation (universal soft dep).
* JWT Bearer auth + mTLS (Node-only).
* 3 framework examples : React 18 + Vue 3 + Express.
* ESM + CJS dual exports + browser-conditional + `@efe/sdk/node` /
  `/web` / `/mock` sub-paths.
* Strict TS mode : `noUncheckedIndexedAccess`,
  `exactOptionalPropertyTypes`, etc.

### Limitations

* Browser : no streaming RPCs (deferred D26+).
* Browser : no mTLS (browser API limitation).
* Browser : no Prometheus metrics scrape (no stable endpoint).
* Edge runtimes (Cloudflare Workers, Vercel Edge) : browser bundle
  works ; Node bundle does NOT (no TCP socket).
