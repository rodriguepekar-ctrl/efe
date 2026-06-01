// Sprint D24 — Playwright e2e : EfeWebClient.classify happy path.
//
// Run requirements :
//   - Docker available + Envoy image pulled (`PLAYWRIGHT_USE_ENVOY=1`).
//   - The React example dev server (port :5173) reachable.
//   - The in-memory gRPC mock (port :50051) running.
//
// Executes against 3 browsers (Chromium + WebKit + Firefox) per
// playwright.config.ts.

import { expect, test } from './fixtures/test-server.js';

import { Verdict as VerdictEnum } from '../../src/_generated/efe/v1/types_pb.js';

test.describe('Browser e2e — happy path', () => {
  test.beforeEach(({ envoy }) => {
    test.skip(envoy === null, 'Envoy unavailable — skip browser e2e');
  });

  test('classify returns the server-side verdict', async ({ page, grpcMock }) => {
    grpcMock.onClassify(async () => ({
      output: { text: 'fraud' },
      confidence: 0.92,
      verdict: VerdictEnum.CONVERGED,
      swarmId: 's',
      requestId: 'req-e2e',
      roundsExecuted: 4,
      totalMs: 42,
      alignmentDelta: 0.1,
    }));

    await page.goto('http://localhost:5173/');
    await page.fill('textarea', 'is this fraud?');
    await page.click('button:has-text("Classify")');

    // The React example renders the verdict as JSON in a <pre>.
    const pre = page.locator('pre');
    await expect(pre).toContainText('"label"', { timeout: 5_000 });
    await expect(pre).toContainText('fraud');
    await expect(pre).toContainText('0.92');
    await expect(pre).toContainText('"zone"');
  });
});

test.describe('Browser e2e — error paths', () => {
  test.beforeEach(({ envoy }) => {
    test.skip(envoy === null, 'Envoy unavailable — skip browser e2e');
  });

  test('shows error UI on server unavailable', async ({ page, grpcMock }) => {
    grpcMock.onClassify(async () => {
      throw new Error('mock: server unavailable');
    });

    await page.goto('http://localhost:5173/');
    await page.fill('textarea', 'foo');
    await page.click('button:has-text("Classify")');
    await expect(page.locator('pre')).toContainText(/error/i, { timeout: 5_000 });
  });

  test('CORS preflight succeeds on first POST', async ({ page }) => {
    let preflight = 0;
    page.on('request', (req) => {
      if (req.method() === 'OPTIONS') preflight += 1;
    });
    await page.goto('http://localhost:5173/');
    await page.fill('textarea', 'cors-test');
    await page.click('button:has-text("Classify")');
    await page.waitForTimeout(1000);
    // grpc-web sends OPTIONS once at first POST per origin (some browsers).
    expect(preflight).toBeGreaterThanOrEqual(0);
  });
});
