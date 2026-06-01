// Sprint D24 (Φ-B) — Mock package entrypoint.
//
// `import { MockEfeClient } from '@efe/sdk/mock'` resolves here.
//
// This entrypoint exposes :
//   - MockEfeClient   : pure-JS test fixture (no gRPC)
//   - newVerdict      : factory for canonical Verdict instances
//   - createMockServer: in-memory gRPC server (Node only)
//
// The `createMockServer` is Node-only because it requires `@grpc/grpc-js`
// to bind a real port. Browser tests should use MSW (Mock Service Worker)
// or similar fetch-mocking libs.

// Re-export universal types so mock consumers don't need a separate import.
export type { Verdict } from './core/verdict.js';

// Mock client (no gRPC).
export { MockEfeClient } from './mock/client.js';

// Verdict factory.
export { newVerdict } from './mock/verdict.js';
export type { VerdictOverrides } from './mock/verdict.js';

// In-memory mock server (Node only).
export { createMockServer } from './mock/server.js';
export type { MockServer, ClassifyHandler } from './mock/server.js';
