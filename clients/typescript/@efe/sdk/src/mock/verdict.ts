// Sprint D24 (Φ-B) — `newVerdict()` factory + `VerdictOverrides`.
//
// Canonical pre-baked Verdict for tests that don't want to spin up
// a mock server.

import { type Verdict, deriveZone } from '../core/verdict.js';

/** Optional fields to override the canonical defaults. */
export interface VerdictOverrides {
  readonly label?: string;
  readonly confidence?: number;
  readonly reject?: boolean;
  readonly rounds?: number;
  readonly alignmentDelta?: number;
  readonly catalog?: string;
  readonly zone?: string;
  readonly swarmId?: string;
  readonly requestId?: string;
  readonly elapsedMs?: number;
}

/** Frozen canonical defaults. */
const DEFAULTS: Verdict = Object.freeze({
  label: 'alpha',
  confidence: 0.85,
  reject: false,
  rounds: 4,
  alignmentDelta: 0.1,
  catalog: 'test/cat@v1',
  zone: 'trusted',
  swarmId: 'mock-swarm',
  requestId: '',
  elapsedMs: 42,
});

/**
 * Build a canonical `Verdict` for tests.
 *
 * Default :
 *   { label: "alpha", confidence: 0.85, rounds: 4, catalog: "test/cat@v1",
 *     zone: "trusted", swarmId: "mock-swarm", elapsedMs: 42 }
 *
 * Override any field via the `overrides` arg. The Zone auto-derives
 * from Confidence ONLY IF `zone` is NOT explicitly provided in
 * overrides AND `reject` is false (≥ 0.80 → trusted, ≥ 0.50 → warn,
 * else reject). Matches Go SDK + Python SDK `new_verdict()`.
 *
 * @example
 *   newVerdict({ label: 'fraud', confidence: 0.92 })
 *   // → zone === 'trusted' (auto-derived)
 *
 *   newVerdict({ label: 'unknown', confidence: 0.30 })
 *   // → zone === 'reject' (auto-derived)
 *
 *   newVerdict({ label: 'fraud', confidence: 0.92, zone: 'warn' })
 *   // → zone === 'warn' (explicit)
 */
export function newVerdict(overrides: VerdictOverrides = {}): Verdict {
  const merged: Verdict = { ...DEFAULTS, ...overrides };
  // Auto-derive zone if not explicitly set + not rejected.
  if (overrides.zone === undefined && !merged.reject) {
    return Object.freeze({ ...merged, zone: deriveZone(merged.confidence) });
  }
  return Object.freeze(merged);
}
