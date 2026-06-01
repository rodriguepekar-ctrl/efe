// Sprint D24 — Express BFF example.
//
// Wraps the @efe/sdk/node EfeClient behind a small REST API :
//
//   POST /classify  → forwards body.text to client.classify(...)
//   GET  /healthz   → probes client.healthcheck()
//
// Sentinel error → HTTP status mapping mirrors the Go SDK examples.

import express, { type Request, type Response } from 'express';

import {
  EfeClient,
  ErrCatalogInvalid,
  ErrCatalogNotFound,
  ErrInvalidArgument,
  ErrPermissionDenied,
  ErrRateLimited,
  ErrTimeout,
  ErrUnauthenticated,
  ErrUnavailable,
  Version,
  errorCode,
} from '@efe/sdk/node';

function mustEnv(key: string): string {
  const v = process.env[key];
  if (!v) throw new Error(`missing required env var ${key}`);
  return v;
}

const endpoint = mustEnv('EFE_ENDPOINT');
const token = mustEnv('EFE_JWT_TOKEN');

const client = new EfeClient({
  endpoint,
  auth: { kind: 'jwt', token },
  deadline: 10_000,
  userAgent: `@efe/sdk-express-example/${Version}`,
});

const app = express();
app.use(express.json());

app.post('/classify', async (req: Request, res: Response) => {
  const text = (req.body as { text?: unknown }).text;
  if (typeof text !== 'string' || text.trim().length === 0) {
    res.status(400).json({ error: 'text required (string)', code: 'bad_request' });
    return;
  }
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 10_000);
  try {
    const verdict = await client.classify(text, { signal: ctl.signal });
    res.json(verdict);
  } catch (err) {
    res.status(efeErrorStatus(err)).json({
      error: err instanceof Error ? err.message : String(err),
      code: errorCode(err),
    });
  } finally {
    clearTimeout(timer);
  }
});

app.get('/healthz', async (_req, res) => {
  try {
    await client.healthcheck();
    res.json({ status: 'serving' });
  } catch (err) {
    res.status(503).json({
      status: 'not_serving',
      error: err instanceof Error ? err.message : String(err),
    });
  }
});

const port = Number.parseInt(process.env['PORT'] ?? '3000', 10);
app.listen(port, () => {
  // eslint-disable-next-line no-console
  console.log(`efe-sdk-express example listening on :${port}`);
});

/**
 * Map sentinel errors → HTTP status codes (mirrors Go SDK gin/fiber/chi
 * examples).
 *
 *   ErrUnauthenticated  → 401
 *   ErrPermissionDenied → 403
 *   ErrInvalidArgument  → 400
 *   ErrCatalogNotFound  → 404
 *   ErrCatalogInvalid   → 422
 *   ErrTimeout          → 504
 *   ErrUnavailable      → 503
 *   ErrRateLimited      → 429
 *   default             → 500
 */
function efeErrorStatus(err: unknown): number {
  if (err instanceof ErrUnauthenticated) return 401;
  if (err instanceof ErrPermissionDenied) return 403;
  if (err instanceof ErrInvalidArgument) return 400;
  if (err instanceof ErrCatalogNotFound) return 404;
  if (err instanceof ErrCatalogInvalid) return 422;
  if (err instanceof ErrTimeout) return 504;
  if (err instanceof ErrUnavailable) return 503;
  if (err instanceof ErrRateLimited) return 429;
  return 500;
}
