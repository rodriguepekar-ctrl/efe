// Sprint D24 — unit tests : ExponentialBackoff + sleep.
import { describe, expect, it } from 'vitest';

import { ExponentialBackoff, sleep } from '../../src/index.js';

describe('ExponentialBackoff', () => {
  it('applies sensible defaults from zero-config', () => {
    const b = new ExponentialBackoff();
    expect(b.initialIntervalMs).toBe(100);
    expect(b.maxIntervalMs).toBe(30_000);
    expect(b.multiplier).toBe(2);
  });

  it('first attempt jitter < initial * multiplier^0 = initial', () => {
    // With deterministic random=1.0, the result equals base.
    const b = new ExponentialBackoff({ random: () => 1.0 });
    expect(b.next(1)).toBe(100);
  });

  it('grows multiplicatively per attempt (deterministic random=1)', () => {
    const b = new ExponentialBackoff({
      initialIntervalMs: 100,
      maxIntervalMs: 10_000,
      multiplier: 2,
      random: () => 1.0,
    });
    expect(b.next(1)).toBe(100);
    expect(b.next(2)).toBe(200);
    expect(b.next(3)).toBe(400);
    expect(b.next(4)).toBe(800);
  });

  it('caps at maxIntervalMs', () => {
    const b = new ExponentialBackoff({
      initialIntervalMs: 1000,
      maxIntervalMs: 2000,
      multiplier: 10,
      random: () => 1.0,
    });
    for (let attempt = 1; attempt <= 10; attempt++) {
      expect(b.next(attempt)).toBeLessThanOrEqual(2000);
    }
  });

  it('never returns negative', () => {
    const b = new ExponentialBackoff();
    for (let attempt = 1; attempt <= 100; attempt++) {
      expect(b.next(attempt)).toBeGreaterThanOrEqual(0);
    }
  });

  it('throws on attempt < 1', () => {
    const b = new ExponentialBackoff();
    expect(() => b.next(0)).toThrow();
  });
});

describe('sleep', () => {
  it('resolves after the specified duration', async () => {
    const start = Date.now();
    await sleep(50);
    const elapsed = Date.now() - start;
    expect(elapsed).toBeGreaterThanOrEqual(45);
    expect(elapsed).toBeLessThan(200);
  });
});
