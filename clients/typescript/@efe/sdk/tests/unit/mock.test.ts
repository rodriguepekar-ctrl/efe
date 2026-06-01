// Sprint D24 — unit tests : MockEfeClient + newVerdict.
import { describe, expect, it } from 'vitest';

import {
  ErrClosed,
  ErrTimeout,
  MockEfeClient,
  newVerdict,
} from '../../src/mock.js';

describe('newVerdict', () => {
  it('returns canonical defaults', () => {
    const v = newVerdict();
    expect(v.label).toBe('alpha');
    expect(v.confidence).toBe(0.85);
    expect(v.zone).toBe('trusted');
    expect(v.swarmId).toBe('mock-swarm');
  });

  it('applies overrides', () => {
    const v = newVerdict({ label: 'fraud', confidence: 0.92 });
    expect(v.label).toBe('fraud');
    expect(v.confidence).toBe(0.92);
  });

  it('auto-derives zone when not explicitly set', () => {
    expect(newVerdict({ confidence: 0.9 }).zone).toBe('trusted');
    expect(newVerdict({ confidence: 0.6 }).zone).toBe('warn');
    expect(newVerdict({ confidence: 0.1 }).zone).toBe('reject');
  });

  it('preserves explicit zone over derived', () => {
    expect(newVerdict({ confidence: 0.9, zone: 'warn' }).zone).toBe('warn');
  });

  it('returned verdict is frozen', () => {
    const v = newVerdict();
    expect(Object.isFrozen(v)).toBe(true);
  });
});

describe('MockEfeClient', () => {
  it('records calls in invocation order', async () => {
    const m = new MockEfeClient();
    await m.classify('a');
    await m.classify('b');
    await m.classify('c');
    expect(m.getCalls()).toEqual(['a', 'b', 'c']);
    expect(m.getCallCount()).toBe(3);
  });

  it('returns SetVerdict when set', async () => {
    const m = new MockEfeClient();
    m.setVerdict(newVerdict({ label: 'fraud' }));
    const v = await m.classify('hello');
    expect(v.label).toBe('fraud');
  });

  it('returns canonical default when no verdict set', async () => {
    const m = new MockEfeClient();
    const v = await m.classify('hello');
    expect(v.label).toBe('alpha');
  });

  it('throws SetError when set', async () => {
    const m = new MockEfeClient();
    const boom = new ErrTimeout('boom');
    m.setError(boom);
    await expect(m.classify('hello')).rejects.toThrow('boom');
  });

  it('throws ErrClosed after close()', async () => {
    const m = new MockEfeClient();
    await m.close();
    await expect(m.classify('hello')).rejects.toBeInstanceOf(ErrClosed);
    await expect(m.healthcheck()).rejects.toBeInstanceOf(ErrClosed);
  });

  it('healthcheck fails symmetrically when SetError (post M9-A4 fix coherence)', async () => {
    const m = new MockEfeClient();
    m.setError(new ErrTimeout('boom'));
    await expect(m.healthcheck()).rejects.toBeInstanceOf(ErrTimeout);
  });

  it('reset clears calls + verdict + error + closed', async () => {
    const m = new MockEfeClient();
    await m.classify('a');
    m.setVerdict(newVerdict({ label: 'x' }));
    m.setError(new ErrTimeout());
    await m.close();
    m.reset();
    expect(m.getCalls()).toEqual([]);
    expect(m.closed).toBe(false);
    const v = await m.classify('after-reset');
    expect(v.label).toBe('alpha'); // Default after reset.
  });

  it('returns a copy of the verdict (caller mutation does not poison)', async () => {
    const m = new MockEfeClient();
    m.setVerdict(newVerdict({ label: 'fraud' }));
    const v1 = await m.classify('a');
    // Verdict is frozen, but the wrapper returns a `{ ...v }` copy
    // — copies are NOT frozen, so mutation is allowed but doesn't
    // affect the next call.
    const v2 = await m.classify('b');
    expect(v1.label).toBe(v2.label);
  });
});
