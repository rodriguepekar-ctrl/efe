// Sprint D24 — integration test : classifyStream order preservation.
//
// Pin test for the dispatcher pattern : even when the server returns
// out of order (faster classifications complete first), each
// StreamItem.index reflects the input position.
//
// Mirrors the M9-A3 Go SDK audit fix : single dispatcher + fan-out
// channel guarantees monotonic index assignment.
//
// **Audit fix M10-A4** — gated by `EFE_TEST_REAL_PROTO=1`. See
// `classify.test.ts` for rationale. The DISPATCHER PATTERN itself
// is unit-tested in a placeholder-friendly way via MockEfeClient
// in `tests/unit/stream-dispatcher.test.ts` (which doesn't need
// real proto codegen).

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { EfeClient } from '../../src/node.js';
import { type MockServer, createMockServer } from '../../src/mock.js';
import { Verdict as VerdictEnum } from '../../src/_generated/efe/v1/types_pb.js';

const REAL_PROTO = process.env['EFE_TEST_REAL_PROTO'] === '1';
const describeIfRealProto = REAL_PROTO ? describe : describe.skip;

let server: MockServer;
let client: EfeClient;

describeIfRealProto('EfeClient.classifyStream', () => {
  beforeEach(async () => {
    server = await createMockServer();
    client = new EfeClient({ endpoint: server.endpoint });
    // Echo the prompt back as the label so we can reconstruct (idx → text).
    server.onClassify(async (req) => ({
      output: { text: req.rawPrompt },
      confidence: 0.9,
      verdict: VerdictEnum.CONVERGED,
      swarmId: 's',
      requestId: '',
      roundsExecuted: 1,
      totalMs: 1,
      alignmentDelta: 0.1,
    }));
  });

  afterEach(async () => {
    await client.close();
    await server.close();
  });

  it('preserves input-order via StreamItem.index for 50 inputs', async () => {
    const N = 50;
    const inputs = Array.from({ length: N }, (_, i) => `text-${String(i).padStart(2, '0')}`);

    const got: (string | undefined)[] = new Array(N);
    const seen = new Array(N).fill(false);

    for await (const item of client.classifyStream(inputs)) {
      expect(item.index).toBeGreaterThanOrEqual(0);
      expect(item.index).toBeLessThan(N);
      expect(seen[item.index]).toBe(false); // No duplicates.
      seen[item.index] = true;
      got[item.index] = item.verdict?.label;
    }

    // Every index must have been observed (no holes).
    for (let i = 0; i < N; i++) {
      expect(seen[i]).toBe(true);
    }
    // And the label at each index must match the canonical input.
    for (let i = 0; i < N; i++) {
      expect(got[i]).toBe(inputs[i]);
    }
  }, 30_000);
});
