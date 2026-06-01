// Sprint D24 — unit tests : auth header builder.
import { describe, expect, it } from 'vitest';

import { buildBearerHeader, ErrInvalidArgument } from '../../src/index.js';

describe('buildBearerHeader', () => {
  it('builds canonical Bearer header', () => {
    expect(buildBearerHeader('eyJhbGc...')).toBe('Bearer eyJhbGc...');
  });

  it('throws on empty token', () => {
    expect(() => buildBearerHeader('')).toThrow(ErrInvalidArgument);
  });

  it('preserves leading/trailing whitespace (caller responsibility)', () => {
    // The SDK doesn't munge the token — caller MUST trim if needed.
    expect(buildBearerHeader(' xyz ')).toBe('Bearer  xyz ');
  });
});
