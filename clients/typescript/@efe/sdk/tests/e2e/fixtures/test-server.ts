// Sprint D24 — Playwright fixtures : spawn the full e2e stack.
//
// Spawns :
//   1. In-memory mock gRPC server on :50051 (provided by createMockServer).
//   2. Envoy gRPC-Web translator on :50052 (Docker — only if available).
//   3. Vite dev server for the React example on :5173.
//
// The full chain : Browser → :5173 (React app) → :50052 (Envoy) → :50051 (mock).
//
// CI may run on a stripped-down image without Docker — in that case
// the Envoy step is skipped + the test suite degrades gracefully via
// `test.skip()` markers.

import { spawn, type ChildProcess } from 'node:child_process';
import { test as base } from '@playwright/test';

import { type MockServer, createMockServer } from '../../../src/mock.js';

interface Fixtures {
  /** In-memory gRPC mock listening on :50051. */
  grpcMock: MockServer;
  /** Envoy translator on :50052 (or null if Docker unavailable). */
  envoy: ChildProcess | null;
  /** Vite dev server for the React example. */
  viteServer: ChildProcess;
}

export const test = base.extend<Fixtures>({
  grpcMock: async ({}, use) => {
    const server = await createMockServer({ port: 50051 });
    await use(server);
    await server.close();
  },

  envoy: async ({}, use) => {
    if (!process.env.PLAYWRIGHT_USE_ENVOY) {
      // CI without Docker — skip envoy ; tests degrade via test.skip.
      // eslint-disable-next-line no-await-in-loop
      await use(null);
      return;
    }
    const proc = spawn(
      'docker',
      [
        'run',
        '--rm',
        '--network',
        'host',
        '-v',
        `${process.cwd()}/tests/e2e/fixtures:/cfg`,
        'envoyproxy/envoy:v1.29-latest',
        '-c',
        '/cfg/envoy.yaml',
        '--log-level',
        'warn',
      ],
      { stdio: 'pipe' },
    );
    await waitForPort(50052, 10_000);
    await use(proc);
    proc.kill('SIGTERM');
  },

  viteServer: async ({}, use) => {
    const proc = spawn('pnpm', ['dev'], {
      cwd: 'examples/react',
      stdio: 'pipe',
      detached: false,
    });
    await waitForPort(5173, 15_000);
    await use(proc);
    proc.kill('SIGTERM');
  },
});

async function waitForPort(port: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}/`).catch(() => null);
      if (r) return;
    } catch {
      // Ignore.
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`port ${port} did not open within ${timeoutMs}ms`);
}

export { expect } from '@playwright/test';
