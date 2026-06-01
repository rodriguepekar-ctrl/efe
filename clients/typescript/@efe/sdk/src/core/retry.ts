// Sprint D24 (Φ-B) — Retry backoff (universal — used by Node interceptor).
//
// Browser NO retry by default (caller's responsibility — TanStack Query
// / SWR / etc. have richer policies). The class is universal because
// `MockEfeClient` may want it for tests + a future browser opt-in.
//
// Pattern : full jitter (Marc Brooker / AWS canonical) :
//
//   delay = random(0, min(maxIntervalMs, initialIntervalMs * multiplier^(attempt-1)))
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §12.5.

/**
 * RetryBackoff computes the delay (in ms) before the (attempt+1)-th try.
 *
 * Implementations MUST be deterministic given the same attempt number —
 * except for jitter, which intentionally introduces randomness to
 * avoid thundering herd.
 */
export interface RetryBackoff {
  /**
   * Returns the delay in ms before retry attempt `attempt` (1-indexed,
   * so attempt=1 = first retry, AFTER the initial try).
   */
  next(attempt: number): number;
}

/**
 * Exponential backoff with full jitter.
 *
 * Defaults if any field is omitted :
 *   - initialIntervalMs : 100 ms
 *   - maxIntervalMs     : 30_000 ms (30 s)
 *   - multiplier        : 2.0
 *   - random            : `Math.random` (replaceable in tests for
 *                          deterministic delays)
 */
export class ExponentialBackoff implements RetryBackoff {
  readonly initialIntervalMs: number;
  readonly maxIntervalMs: number;
  readonly multiplier: number;
  private readonly random: () => number;

  constructor(opts?: {
    initialIntervalMs?: number;
    maxIntervalMs?: number;
    multiplier?: number;
    /** Test-only : deterministic random source. */
    random?: () => number;
  }) {
    this.initialIntervalMs = opts?.initialIntervalMs ?? 100;
    this.maxIntervalMs = opts?.maxIntervalMs ?? 30_000;
    this.multiplier = opts?.multiplier ?? 2.0;
    this.random = opts?.random ?? Math.random;

    if (this.initialIntervalMs <= 0) throw new Error('initialIntervalMs must be > 0');
    if (this.maxIntervalMs <= 0) throw new Error('maxIntervalMs must be > 0');
    if (this.multiplier <= 1) throw new Error('multiplier must be > 1');
  }

  next(attempt: number): number {
    if (attempt < 1) throw new Error('attempt must be ≥ 1');
    const base = Math.min(
      this.initialIntervalMs * this.multiplier ** (attempt - 1),
      this.maxIntervalMs,
    );
    return this.random() * base;
  }
}

/** Sleep for `ms` milliseconds. Used by the retry interceptor. */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
