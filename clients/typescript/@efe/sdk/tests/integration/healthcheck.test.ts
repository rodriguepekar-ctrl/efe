// Sprint D24 — integration tests : EfeClient.healthcheck.
//
// **Audit fix M10-A4** — gated by `EFE_TEST_REAL_PROTO=1`. See
// `classify.test.ts` for rationale.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { EfeClient } from '../../src/node.js';
import { ErrClosed } from '../../src/index.js';
import { type MockServer, createMockServer } from '../../src/mock.js';

const REAL_PROTO = process.env['EFE_TEST_REAL_PROTO'] === '1';
const describeIfRealProto = REAL_PROTO ? describe : describe.skip;

let server: MockServer;
let client: EfeClient;

describeIfRealProto('EfeClient.healthcheck', () => {
  beforeEach(async () => {
    server = await createMockServer();
    client = new EfeClient({ endpoint: server.endpoint, deadline: 1_000 });
  });

  afterEach(async () => {
    await client.close();
    await server.close();
  });

  it('throws ErrClosed after close()', async () => {
    await client.close();
    await expect(client.healthcheck()).rejects.toBeInstanceOf(ErrClosed);
  });

  it('errors gracefully when server has no Health service registered', async () => {
    // The mock server doesn't register the Health service, so this RPC
    // returns Unimplemented (or similar) → SDK throws.
    await expect(client.healthcheck()).rejects.toThrow();
  });
});
