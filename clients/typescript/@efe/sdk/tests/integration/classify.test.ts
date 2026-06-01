// Sprint D24 — integration tests : EfeClient.classify vs in-memory mock server.
//
// These tests exercise the FULL gRPC stack (interceptors, retry, error
// mapping, OTel context propagation) without ever touching the network.
//
// **Audit fix M10-A4** : gated by `EFE_TEST_REAL_PROTO=1` env var.
// During the D24 placeholder phase (hand-coded proto stubs), the SDK
// Connect-RPC client uses BINARY proto wire format while the mock
// server uses JSON serialization (no real proto codegen yet) — wire
// mismatch makes the round-trip fail. The tests are STRUCTURAL (we
// validate the test code compiles + the mock + client APIs match) ;
// they're activated end-to-end ONLY after `pnpm gen:proto` produces
// real `@bufbuild/protobuf` types AND the mock server is upgraded
// to speak the same wire format.
//
// Pin tested by `test_d24_typescript_sdk_audit_fix_a4_integration_gated`.

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { EfeClient } from '../../src/node.js';
import {
  ErrClosed,
  ErrInvalidArgument,
  ExponentialBackoff,
} from '../../src/index.js';
import { type MockServer, createMockServer } from '../../src/mock.js';
import { Verdict as VerdictEnum } from '../../src/_generated/efe/v1/types_pb.js';

const REAL_PROTO = process.env['EFE_TEST_REAL_PROTO'] === '1';
const describeIfRealProto = REAL_PROTO ? describe : describe.skip;

/**
 * Helper : produce a canonical ExecuteResponse from a `newVerdict()`-shaped
 * object. Mirrors the Go SDK `OkResponse` helper.
 */
function okResponse(label: string, confidence = 0.9): {
  output: { text: string };
  confidence: number;
  verdict: VerdictEnum;
  swarmId: string;
  requestId: string;
  roundsExecuted: number;
  totalMs: number;
  alignmentDelta: number;
} {
  return {
    output: { text: label },
    confidence,
    verdict: VerdictEnum.CONVERGED,
    swarmId: 'mock-swarm',
    requestId: 'req-test',
    roundsExecuted: 4,
    totalMs: 42,
    alignmentDelta: 0.1,
  };
}

let server: MockServer;
let client: EfeClient;

describeIfRealProto('EfeClient.classify (integration vs in-memory mock)', () => {
  beforeEach(async () => {
    server = await createMockServer();
    client = new EfeClient({ endpoint: server.endpoint });
  });

  afterEach(async () => {
    await client.close();
    await server.close();
  });

  it('happy path : returns server-side verdict with derived zone', async () => {
    server.onClassify(async () => okResponse('billing', 0.91));
    const verdict = await client.classify('I want a refund');
    expect(verdict.label).toBe('billing');
    expect(verdict.confidence).toBeCloseTo(0.91, 2);
    expect(verdict.zone).toBe('trusted');
    expect(verdict.reject).toBe(false);
    expect(verdict.elapsedMs).toBeGreaterThanOrEqual(0);
  });

  it('throws ErrInvalidArgument on empty text (local validation)', async () => {
    await expect(client.classify('  ')).rejects.toBeInstanceOf(ErrInvalidArgument);
  });

  it('throws ErrClosed after close()', async () => {
    await client.close();
    await expect(client.classify('foo')).rejects.toBeInstanceOf(ErrClosed);
  });

  it('preserves the requestId field from server', async () => {
    server.onClassify(async () => ({ ...okResponse('x'), requestId: 'req-XYZ-123' }));
    const v = await client.classify('foo');
    expect(v.requestId).toBe('req-XYZ-123');
  });

  it('marks reject=true for VERDICT_DIVERGENCE', async () => {
    server.onClassify(async () => ({
      ...okResponse('x', 0.9),
      verdict: VerdictEnum.DIVERGENCE,
    }));
    const v = await client.classify('hostile');
    expect(v.reject).toBe(true);
    expect(v.zone).toBe('reject');
  });

  it('marks reject=true for VERDICT_MODE_COLLAPSE', async () => {
    server.onClassify(async () => ({
      ...okResponse('x', 0.9),
      verdict: VerdictEnum.MODE_COLLAPSE,
    }));
    const v = await client.classify('looped');
    expect(v.reject).toBe(true);
  });

  it('handles VERDICT_CONTINUE as warn', async () => {
    server.onClassify(async () => ({
      ...okResponse('x', 0.6),
      verdict: VerdictEnum.CONTINUE,
    }));
    const v = await client.classify('inconclusive');
    expect(v.zone).toBe('warn');
  });
});

describeIfRealProto('EfeClient.classify retry policy', () => {
  beforeEach(async () => {
    server = await createMockServer();
    client = new EfeClient({
      endpoint: server.endpoint,
      retry: {
        maxAttempts: 3,
        backoff: new ExponentialBackoff({ initialIntervalMs: 1, maxIntervalMs: 10 }),
      },
    });
  });

  afterEach(async () => {
    await client.close();
    await server.close();
  });

  it('fails fast on non-retryable error (InvalidArgument)', async () => {
    server.onClassify(async () => {
      // Simulate INVALID_ARGUMENT from server side.
      throw new Error('mock: simulated invalid');
    });
    // Will be wrapped into ErrInternal (UNKNOWN code from server) — not retryable.
    await expect(client.classify('hello')).rejects.toThrow();
  });
});

describeIfRealProto('EfeClient timeout & abort', () => {
  beforeEach(async () => {
    server = await createMockServer();
    client = new EfeClient({ endpoint: server.endpoint, deadline: 60_000 });
  });

  afterEach(async () => {
    await client.close();
    await server.close();
  });

  it('respects the AbortSignal', async () => {
    // Server hangs forever ; we abort after 50ms.
    server.onClassify(async () => {
      return new Promise(() => {
        /* hang */
      });
    });
    const ctl = new AbortController();
    setTimeout(() => ctl.abort(), 50);
    await expect(client.classify('hello', { signal: ctl.signal })).rejects.toThrow();
  });
});
