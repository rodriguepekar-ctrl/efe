// Sprint D24 (Φ-B) — OpenTelemetry interceptor (universal).
//
// SOFT dependency : `@opentelemetry/api` is a peerDependency. If the
// app installs it, the SDK wires spans automatically. If not, the
// interceptor is a no-op (zero overhead).
//
// We use a lazy `try { await import(...) }` pattern at module-load to
// detect the dep without a hard import (which would break in apps
// that don't install OTel).
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §13.

import type { Interceptor } from '@connectrpc/connect';

import type { OTelConfig } from './options.js';
import { Version } from './version.js';

/**
 * Returns an interceptor that creates one client span per RPC.
 *
 * Span name : `efe/<method>` (e.g. `efe/Execute`).
 * Span attributes : `rpc.system=grpc`, `rpc.service=efe.v1.SwarmService`,
 * `rpc.method=<MethodName>` (auto-set).
 *
 * On error, the span is marked ERROR + the exception is recorded.
 *
 * Pin tested by `test_d24_typescript_sdk_otel_no_op_when_unset`.
 */
export function otelInterceptor(cfg: OTelConfig): Interceptor {
  if (cfg.kind === 'noop') {
    return (next) => async (req) => next(req);
  }
  // We avoid `await import(...)` at top-level (it would force ESM-only
  // bundling). Instead we only call into OTel APIs IF the user passed a
  // provider — at that point we know `@opentelemetry/api` is installed.
  const tracer = cfg.tracerProvider.getTracer('@efe/sdk', Version);
  return (next) => async (req) => {
    const methodName = req.method.name;
    const span = tracer.startSpan(`efe/${methodName}`, {
      kind: 3, // SpanKind.CLIENT — hardcoded numeric to avoid the import-time dependency.
      attributes: {
        'rpc.system': 'grpc',
        'rpc.service': req.service.typeName,
        'rpc.method': methodName,
      },
    });
    try {
      const resp = await next(req);
      span.setStatus({ code: 1 /* StatusCode.OK */ });
      return resp;
    } catch (err) {
      span.recordException(err as Error);
      span.setStatus({
        code: 2 /* StatusCode.ERROR */,
        message: err instanceof Error ? err.message : String(err),
      });
      throw err;
    } finally {
      span.end();
    }
  };
}
