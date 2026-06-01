// Sprint D24 — unit tests : ClientOptions validation.
import { describe, expect, it } from 'vitest';

import {
  ErrInvalidArgument,
  ExponentialBackoff,
  validateOptions,
  Version,
} from '../../src/index.js';

describe('validateOptions', () => {
  it('throws on missing endpoint', () => {
    expect(() => validateOptions({ endpoint: '' })).toThrow(ErrInvalidArgument);
  });

  it('throws on negative deadline', () => {
    expect(() => validateOptions({ endpoint: 'localhost:50051', deadline: -1 })).toThrow(
      ErrInvalidArgument,
    );
  });

  it('accepts deadline=0 (no SDK-side deadline)', () => {
    const r = validateOptions({ endpoint: 'localhost:50051', deadline: 0 });
    expect(r.deadline).toBe(0);
  });

  it('throws on retry.maxAttempts < 1', () => {
    expect(() =>
      validateOptions({
        endpoint: 'localhost:50051',
        retry: { maxAttempts: 0, backoff: new ExponentialBackoff() },
      }),
    ).toThrow(ErrInvalidArgument);
  });

  it('throws on empty JWT token', () => {
    expect(() =>
      validateOptions({ endpoint: 'localhost:50051', auth: { kind: 'jwt', token: '' } }),
    ).toThrow(ErrInvalidArgument);
  });

  it('throws on partial mTLS paths', () => {
    expect(() =>
      validateOptions({
        endpoint: 'localhost:50051',
        auth: { kind: 'mtls', certPath: '/c', keyPath: '/k', caPath: '' },
      }),
    ).toThrow(ErrInvalidArgument);
  });

  it('fills sensible defaults', () => {
    const r = validateOptions({ endpoint: 'localhost:50051' });
    expect(r.deadline).toBe(10_000);
    expect(r.auth).toEqual({ kind: 'none' });
    expect(r.retry.maxAttempts).toBe(3);
    expect(r.userAgent).toBe(`@efe/sdk/${Version}`);
    expect(r.maxReceivedMessageSize).toBe(16 * 1024 * 1024);
  });

  it('throws on empty userAgent override', () => {
    expect(() =>
      validateOptions({ endpoint: 'localhost:50051', userAgent: '' }),
    ).toThrow(ErrInvalidArgument);
  });

  it('throws on maxReceivedMessageSize ≤ 0', () => {
    expect(() =>
      validateOptions({ endpoint: 'localhost:50051', maxReceivedMessageSize: 0 }),
    ).toThrow(ErrInvalidArgument);
  });
});
