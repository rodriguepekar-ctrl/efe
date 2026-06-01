// Sprint D24 — unit tests : Verdict + helpers.
import { describe, expect, it } from 'vitest';

import { deriveZone, isReject, isTrusted, isWarn, type Verdict } from '../../src/index.js';

const baseVerdict = (zone: string, reject = false): Verdict => ({
  label: 'foo',
  confidence: 0.9,
  reject,
  rounds: 4,
  alignmentDelta: 0.1,
  catalog: 'cat',
  zone,
  swarmId: 's',
  requestId: '',
  elapsedMs: 1,
});

describe('Verdict zone predicates', () => {
  it('isTrusted is true only for non-rejected trusted', () => {
    expect(isTrusted(baseVerdict('trusted'))).toBe(true);
    expect(isTrusted(baseVerdict('warn'))).toBe(false);
    expect(isTrusted(baseVerdict('trusted', true))).toBe(false);
  });

  it('isWarn is true only for non-rejected warn', () => {
    expect(isWarn(baseVerdict('warn'))).toBe(true);
    expect(isWarn(baseVerdict('trusted'))).toBe(false);
    expect(isWarn(baseVerdict('warn', true))).toBe(false);
  });

  it('isReject is true for explicit reject OR zone=reject', () => {
    expect(isReject(baseVerdict('reject'))).toBe(true);
    expect(isReject(baseVerdict('warn', true))).toBe(true);
    expect(isReject(baseVerdict('trusted'))).toBe(false);
  });
});

describe('deriveZone', () => {
  it('returns trusted ≥ 0.80', () => {
    expect(deriveZone(0.8)).toBe('trusted');
    expect(deriveZone(0.99)).toBe('trusted');
  });

  it('returns warn ≥ 0.50', () => {
    expect(deriveZone(0.5)).toBe('warn');
    expect(deriveZone(0.79)).toBe('warn');
  });

  it('returns reject < 0.50', () => {
    expect(deriveZone(0.0)).toBe('reject');
    expect(deriveZone(0.49)).toBe('reject');
  });
});
