// Sprint D24 (Φ-B) — Node entrypoint.
//
// `import { EfeClient } from '@efe/sdk/node'` resolves here.
// `import { EfeClient } from '@efe/sdk'` ALSO resolves here when
// running in Node context (via package.json `exports.node`).

// Re-export universal core.
export * from './index.js';

// Node-specific class.
export { EfeClient } from './node/client.js';

// Node-specific TLS helper.
export { withMTLS } from './node/tls.js';
export type { NodeTlsCredentials } from './node/tls.js';

// Node-specific metrics names (for advanced consumers who want to
// build their own dashboards programmatically).
export { METRIC_NAMES } from './node/metrics.js';
