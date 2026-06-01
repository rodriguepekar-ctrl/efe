// Sprint D24 (Φ-B) — Prometheus metrics (Node-only).
//
// SOFT dependency : `prom-client` is a peerDependency. If the app
// installs it AND passes `prometheus: { kind: 'registry', registry }`,
// the SDK registers 5 collectors with canonical names matching Go SDK.
//
// Names are STABLE — bump major to rename or remove. Pin tested by
// `test_d24_typescript_sdk_metric_names_match_go_sdk`.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §14.

import { createRequire } from 'node:module';

import type { Counter, Gauge, Histogram, Registry } from 'prom-client';

import type { Interceptor } from '@connectrpc/connect';

import { errorCode } from '../core/errors.js';
import { fromConnectError } from '../core/from-rpc.js';
import type { PrometheusConfig } from '../core/options.js';

// **Audit fix M10-A3** — `require('prom-client')` doesn't exist in ESM
// context (`type: "module"` in package.json). `createRequire(import.meta.url)`
// produces a `require`-compatible function that works in BOTH ESM and CJS
// builds (tsup emits both via dist/node/index.js + index.cjs).
//
// Pin tested by `test_d24_typescript_sdk_audit_fix_a3_uses_create_require`.
const requireCJS = createRequire(import.meta.url);

/** Stable metric names — match Go SDK + Python SDK. */
export const METRIC_NAMES = {
  classifyTotal: 'efe_sdk_classify_total',
  classifyLatency: 'efe_sdk_classify_latency_seconds',
  classifyErrorsTotal: 'efe_sdk_classify_errors_total',
  inFlight: 'efe_sdk_in_flight',
  retryAttemptsTotal: 'efe_sdk_retry_attempts_total',
} as const;

/** Bag of 5 collectors. Lifecycle-bound to one EfeClient instance. */
export class SdkMetrics {
  readonly classifyTotal: Counter<'outcome'>;
  readonly classifyLatency: Histogram<'outcome'>;
  readonly classifyErrors: Counter<'sentinel'>;
  readonly inFlight: Gauge;
  readonly retryAttempts: Counter<'attempt'>;

  constructor(registry: Registry, promClient: typeof import('prom-client')) {
    this.classifyTotal = new promClient.Counter({
      name: METRIC_NAMES.classifyTotal,
      help: 'Total efe_sdk Classify calls, partitioned by outcome.',
      labelNames: ['outcome'] as const,
      registers: [registry],
    });
    this.classifyLatency = new promClient.Histogram({
      name: METRIC_NAMES.classifyLatency,
      help: 'Classify wall-clock latency in seconds, partitioned by outcome.',
      labelNames: ['outcome'] as const,
      buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
      registers: [registry],
    });
    this.classifyErrors = new promClient.Counter({
      name: METRIC_NAMES.classifyErrorsTotal,
      help: 'Total efe_sdk Classify errors, partitioned by sentinel code.',
      labelNames: ['sentinel'] as const,
      registers: [registry],
    });
    this.inFlight = new promClient.Gauge({
      name: METRIC_NAMES.inFlight,
      help: 'Currently in-flight Classify calls.',
      registers: [registry],
    });
    this.retryAttempts = new promClient.Counter({
      name: METRIC_NAMES.retryAttemptsTotal,
      help: 'Total retry attempts, partitioned by attempt number.',
      labelNames: ['attempt'] as const,
      registers: [registry],
    });
  }
}

/**
 * Build a Prometheus interceptor that observes start/end of every RPC.
 *
 *   - inFlight  : ++ on start, -- on end.
 *   - classifyTotal{outcome}   : success | error.
 *   - classifyLatency{outcome} : Histogram observation in seconds.
 *   - classifyErrors{sentinel} : code label on error.
 *
 * If `prom-client` isn't installed OR cfg.kind === 'noop', the
 * interceptor is a no-op (zero overhead).
 */
export function promInterceptor(cfg: PrometheusConfig): Interceptor {
  if (cfg.kind === 'noop') {
    return (next) => async (req) => next(req);
  }
  // **Audit fix M10-A3** — use `createRequire(import.meta.url)` defined at
  // module top instead of bare `require()` (which is unavailable in ESM).
  // Lazy require so non-Node bundles don't pull `prom-client`.
  const promClient = requireCJS('prom-client') as typeof import('prom-client');
  const m = new SdkMetrics(cfg.registry, promClient);
  return (next) => async (req) => {
    const start = Date.now();
    m.inFlight.inc();
    try {
      const resp = await next(req);
      const elapsedSec = (Date.now() - start) / 1000;
      m.classifyTotal.labels('success').inc();
      m.classifyLatency.labels('success').observe(elapsedSec);
      return resp;
    } catch (err) {
      const elapsedSec = (Date.now() - start) / 1000;
      m.classifyTotal.labels('error').inc();
      m.classifyLatency.labels('error').observe(elapsedSec);
      const translated = fromConnectError(err);
      m.classifyErrors.labels(errorCode(translated)).inc();
      throw err;
    } finally {
      m.inFlight.dec();
    }
  };
}
