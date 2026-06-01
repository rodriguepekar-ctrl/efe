// Sprint D24 — unit tests : 10 sentinel errors + isRetryable + errorCode.
import { describe, expect, it } from 'vitest';

import {
  ALL_SENTINELS,
  EfeError,
  ErrCatalogInvalid,
  ErrCatalogNotFound,
  ErrClosed,
  ErrInternal,
  ErrInvalidArgument,
  ErrPermissionDenied,
  ErrRateLimited,
  ErrTimeout,
  ErrUnauthenticated,
  ErrUnavailable,
  errorCode,
  isRetryable,
} from '../../src/index.js';

describe('Sentinels', () => {
  it('exposes exactly 10 sentinel classes', () => {
    expect(ALL_SENTINELS).toHaveLength(10);
  });

  it('every sentinel extends EfeError', () => {
    for (const SentinelClass of ALL_SENTINELS) {
      const instance = new SentinelClass();
      expect(instance).toBeInstanceOf(EfeError);
      expect(instance).toBeInstanceOf(Error);
    }
  });

  it('every sentinel has a distinct `code` field', () => {
    const codes = ALL_SENTINELS.map((cls) => new cls().code);
    expect(new Set(codes).size).toBe(codes.length);
  });

  it('every sentinel has a stable `name` field', () => {
    const names = ALL_SENTINELS.map((cls) => new cls().name);
    expect(names).toEqual([
      'EfeUnauthenticated',
      'EfePermissionDenied',
      'EfeCatalogNotFound',
      'EfeCatalogInvalid',
      'EfeTimeout',
      'EfeUnavailable',
      'EfeRateLimited',
      'EfeInvalidArgument',
      'EfeInternal',
      'EfeClosed',
    ]);
  });

  it('preserves the cause chain', () => {
    const root = new Error('root');
    const wrapped = new ErrTimeout('wrapped', { cause: root });
    expect(wrapped.cause).toBe(root);
  });
});

describe('isRetryable', () => {
  it('returns true for the 3 retryable sentinels', () => {
    expect(isRetryable(new ErrUnavailable())).toBe(true);
    expect(isRetryable(new ErrTimeout())).toBe(true);
    expect(isRetryable(new ErrRateLimited())).toBe(true);
  });

  it('returns false for non-retryable sentinels', () => {
    expect(isRetryable(new ErrUnauthenticated())).toBe(false);
    expect(isRetryable(new ErrPermissionDenied())).toBe(false);
    expect(isRetryable(new ErrCatalogNotFound())).toBe(false);
    expect(isRetryable(new ErrCatalogInvalid())).toBe(false);
    expect(isRetryable(new ErrInvalidArgument())).toBe(false);
    expect(isRetryable(new ErrInternal())).toBe(false);
    expect(isRetryable(new ErrClosed())).toBe(false);
  });

  it('returns false for non-EfeError values', () => {
    expect(isRetryable(new Error('random'))).toBe(false);
    expect(isRetryable(null)).toBe(false);
    expect(isRetryable(undefined)).toBe(false);
    expect(isRetryable('string')).toBe(false);
  });
});

describe('errorCode', () => {
  it('returns the canonical short label for each sentinel', () => {
    expect(errorCode(new ErrUnauthenticated())).toBe('unauthenticated');
    expect(errorCode(new ErrPermissionDenied())).toBe('permission_denied');
    expect(errorCode(new ErrCatalogNotFound())).toBe('catalog_not_found');
    expect(errorCode(new ErrCatalogInvalid())).toBe('catalog_invalid');
    expect(errorCode(new ErrTimeout())).toBe('timeout');
    expect(errorCode(new ErrUnavailable())).toBe('unavailable');
    expect(errorCode(new ErrRateLimited())).toBe('rate_limited');
    expect(errorCode(new ErrInvalidArgument())).toBe('invalid_argument');
    expect(errorCode(new ErrInternal())).toBe('internal');
    expect(errorCode(new ErrClosed())).toBe('closed');
  });

  it('returns "unknown" for non-EfeError values', () => {
    expect(errorCode(new Error('random'))).toBe('unknown');
  });

  it('returns empty string for nullish', () => {
    expect(errorCode(null)).toBe('');
    expect(errorCode(undefined)).toBe('');
  });
});
