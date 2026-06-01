// Sprint D24 (Φ-B) — Browser entrypoint.
//
// `import { EfeClient } from '@efe/sdk/web'` resolves here.
// `import { EfeClient } from '@efe/sdk'` ALSO resolves here when
// running in Browser context (via package.json `exports.browser`).
//
// The browser surface is a SUBSET — only idempotent RPCs (Classify,
// Healthcheck) are exposed. The class is RE-EXPORTED as `EfeClient`
// (not `EfeWebClient`) so consumer code is portable across runtimes
// without import-path tweaks.

// Re-export universal core.
export * from './index.js';

// Browser-specific class — re-exported as EfeClient for consumer
// convenience (matches the universal `import { EfeClient }` pattern).
export { EfeWebClient as EfeClient } from './web/client.js';
export { EfeWebClient } from './web/client.js';

// Browser-specific auth helper.
export { withFetchAuth } from './web/auth.js';
