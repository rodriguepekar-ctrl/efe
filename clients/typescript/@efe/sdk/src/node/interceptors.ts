// Sprint D24 (Φ-B) — Retry interceptor (Node).
//
// The retry interceptor inspects the error via `isRetryable()` AFTER
// translating it through `fromConnectError()`. Non-retryable errors
// propagate immediately — saves budget AND avoids double-side-effects
// on writes (defense-in-depth ; the CatalogService non-idempotent
// methods aren't even exposed to the browser subset).
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §12.6.

import type { Interceptor } from '@connectrpc/connect';

import { EfeError, isRetryable } from '../core/errors.js';
import { fromConnectError } from '../core/from-rpc.js';
import type { RetryConfig } from '../core/options.js';
import { sleep } from '../core/retry.js';

/**
 * Returns a Connect-RPC interceptor that retries retry-eligible
 * errors up to `cfg.maxAttempts` times.
 *
 * Order in the interceptor chain : LAST (outermost). Retries are
 * counted as separate observations + each retry runs through auth +
 * otel independently (fresh span per retry), matching Go SDK.
 */
export function retryInterceptor(cfg: RetryConfig): Interceptor {
  return (next) => async (req) => {
    let lastErr: unknown;
    for (let attempt = 1; attempt <= cfg.maxAttempts; attempt++) {
      try {
        return await next(req);
      } catch (err) {
        const translated = fromConnectError(err);
        if (translated instanceof EfeError && !isRetryable(translated)) {
          throw translated;
        }
        if (!(translated instanceof EfeError)) {
          // Non-RPC error (e.g. signal abort) — don't retry.
          throw err;
        }
        lastErr = translated;
        if (attempt === cfg.maxAttempts) {
          throw translated;
        }
        const delay = cfg.backoff.next(attempt);
        await sleep(delay);
      }
    }
    // Defensive fallthrough — TS narrows lastErr to unknown.
    throw lastErr ?? new Error('retry: exhausted attempts');
  };
}
