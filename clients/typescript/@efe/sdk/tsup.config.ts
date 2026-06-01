// Sprint D24 (Φ-B) — tsup bundling config for @efe/sdk.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §20.4.
//
// Three output bundles :
//   1. dist/node/  — Node ESM + CJS dual (target node18, ~250 KB gz limit).
//   2. dist/web/   — Browser ESM only   (target es2022, ~50 KB gz limit).
//   3. dist/mock/  — Mock ESM + CJS dual (Node-only test fixtures).
//
// All bundles are tree-shakeable + sourcemap'd + .d.ts emitted.

import { defineConfig } from 'tsup';

export default defineConfig([
  {
    entry: { index: 'src/node.ts' },
    outDir: 'dist/node',
    format: ['esm', 'cjs'],
    target: 'node18',
    dts: true,
    sourcemap: true,
    clean: true,
    splitting: false,
    treeshake: true,
    platform: 'node',
    external: [
      '@grpc/grpc-js',
      '@opentelemetry/api',
      'prom-client',
    ],
  },
  {
    entry: { index: 'src/web.ts' },
    outDir: 'dist/web',
    format: ['esm'],
    target: 'es2022',
    dts: true,
    sourcemap: true,
    clean: false,
    splitting: false,
    treeshake: true,
    platform: 'browser',
    external: [
      // **Audit fix M10-A13** — pre-fix only listed `@opentelemetry/api`.
      // Adding the Node-only peerDeps so they NEVER end up in the browser
      // bundle (defense-in-depth ; tree-shaking should already drop them).
      '@opentelemetry/api',
      '@grpc/grpc-js',
      'prom-client',
      'node:fs',
      'node:tls',
      'node:module',
    ],
  },
  {
    entry: { index: 'src/mock.ts' },
    outDir: 'dist/mock',
    format: ['esm', 'cjs'],
    target: 'node18',
    dts: true,
    sourcemap: true,
    clean: false,
    splitting: false,
    treeshake: true,
    platform: 'node',
    external: ['@grpc/grpc-js'],
  },
]);
