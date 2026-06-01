// Sprint D24 (Φ-B) — Bundle size budget enforcer.
//
// Run AFTER `pnpm build`. Fails (exit 1) if any target exceeds budget.
//
// Targets :
//   - dist/web/index.js   ≤ 50 KB gz  (browser entry, hard limit)
//   - dist/node/index.js  ≤ 250 KB gz (Node ESM)
//   - dist/node/index.cjs ≤ 250 KB gz (Node CJS)
//   - dist/mock/index.js  ≤ 30 KB gz  (test fixtures)
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §20.

import { gzipSync } from 'node:zlib';
import { readFileSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';

/** @typedef {{ path: string, limitBytes: number }} BundleTarget */

/** @type {BundleTarget[]} */
const TARGETS = [
  { path: 'dist/web/index.js', limitBytes: 50 * 1024 },
  { path: 'dist/node/index.js', limitBytes: 250 * 1024 },
  { path: 'dist/node/index.cjs', limitBytes: 250 * 1024 },
  { path: 'dist/mock/index.js', limitBytes: 30 * 1024 },
];

const ROOT = resolve(new URL('.', import.meta.url).pathname, '..');

function fmt(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

let allOk = true;
let anyMissing = false;

console.log('━━━ @efe/sdk bundle size check ━━━');
for (const t of TARGETS) {
  const abs = resolve(ROOT, t.path);
  if (!existsSync(abs)) {
    console.log(`  ⚠️  ${t.path} : MISSING (run \`pnpm build\` first)`);
    anyMissing = true;
    continue;
  }
  const raw = readFileSync(abs);
  const gz = gzipSync(raw);
  const ok = gz.length <= t.limitBytes;
  const status = ok ? '✅' : '❌';
  const pct = ((gz.length / t.limitBytes) * 100).toFixed(0);
  console.log(
    `  ${status} ${t.path.padEnd(26)} ${fmt(gz.length).padStart(10)} gz / ${fmt(t.limitBytes)} (${pct}%)`,
  );
  if (!ok) allOk = false;
}

if (anyMissing) {
  console.error('\n⚠️  Some bundle artefacts are missing — run `pnpm build` first.');
  process.exit(2);
}
if (!allOk) {
  console.error('\n❌ Bundle size budget exceeded. Tree-shake or remove deps.');
  process.exit(1);
}
console.log('\n✅ All bundle targets within budget.');
