// Sprint **Φ-C-SDK Completion (D30)** — TypeScript SDK contract runner.
//
// Cf. docs/EFE_POLYGLOT_SDKS_COMPLETION_ARCHITECTURE.md § 5 + artefact #13.
//
// Reads /fixtures/quartet_fixtures.json, calls the TS SDK gRPC client
// for each fixture, writes verdicts to /shared/out_ts.json.
//
// Mock mode (EFE_CONTRACT_MOCK=1) replicates the D25 Φ-C-FFI formula
// bit-identical to the Python + Go runners.
//
// Build : tsx main.ts (= no compile step needed for the runner).
// Or : npm install --no-save typescript tsx ; tsx runners/ts/main.ts

import { createHash } from "node:crypto";
import { writeFileSync, readFileSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";

interface Fixture {
  id: string;
  text: string;
  catalog_id: string;
  seed: number;
}

interface FixturePayload {
  version: string;
  count: number;
  catalog_id: string;
  fixtures: Fixture[];
}

interface SubScore {
  label: string;
  confidence: number;
}

interface Verdict {
  fixture_id: string;
  label: string;
  confidence: number;
  cone_id: string;
  rounds_executed: number;
  swarm_id: string;
  catalog_version: string;
  sub_scores: SubScore[];
  latency_ms: number;
}

interface OutputPayload {
  sdk: string;
  version: string;
  count: number;
  verdicts: Verdict[];
}

const DEFAULT_LABELS = [
  "billing-dispute",
  "tech-support",
  "refund-request",
  "general-inquiry",
  "other",
];
const DEFAULT_CATALOG_VERSION = "1.0.0";
const DEFAULT_BUNDLE_ID = "test-bundle";
const DIM = 384;

/**
 * Replicate D25 stub : (byte / 255 - 0.5) → L2 normalize → f32 LE bytes.
 *
 * f32 fidelity : JavaScript Number is IEEE 754 double (f64). We use
 * `Math.fround(x)` to round each intermediate to f32 (= matches Rust
 * f32 arithmetic). The final emb is packed as little-endian f32.
 *
 * **Critical** : `Math.fround` is the canonical way to emulate f32 in
 * JS. Without it, accumulation errors in the norm computation would
 * diverge from Rust → SHA-256 mismatch → 100/100 fixtures FAIL.
 */
function encodeText(text: string, bundleId: string = DEFAULT_BUNDLE_ID): Buffer {
  const seedHasher = createHash("sha256");
  seedHasher.update(text);
  seedHasher.update(bundleId);
  const seed = seedHasher.digest(); // 32 bytes

  // f32 vector via Float32Array (= guarantees f32 precision on assignment).
  const emb = new Float32Array(DIM);
  for (let i = 0; i < DIM; i++) {
    emb[i] = Math.fround(seed[i % 32] / 255.0 - 0.5);
  }
  // Linear sum-of-squares in f32 (= match Rust .sum::<f32>()).
  let sumSq = Math.fround(0.0);
  for (let i = 0; i < DIM; i++) {
    sumSq = Math.fround(sumSq + Math.fround(emb[i] * emb[i]));
  }
  const norm = Math.fround(Math.sqrt(sumSq));
  if (norm > 0.0) {
    for (let i = 0; i < DIM; i++) {
      emb[i] = Math.fround(emb[i] / norm);
    }
  }
  // Pack as little-endian f32 bytes (= match Rust to_le_bytes()).
  // Float32Array on x86/ARM is LE by default ; the Buffer view shares
  // the memory directly.
  return Buffer.from(emb.buffer, emb.byteOffset, emb.byteLength);
}

function mockVerdict(f: Fixture): Verdict {
  const embBytes = encodeText(f.text);

  const h = createHash("sha256");
  h.update(embBytes);
  h.update(f.catalog_id);
  // seed as u64 little-endian.
  const seedBuf = Buffer.alloc(8);
  seedBuf.writeBigUInt64LE(BigInt(f.seed), 0);
  h.update(seedBuf);
  const digest = h.digest();

  const labelIdx = digest.readUInt32LE(0) % DEFAULT_LABELS.length;
  const label = DEFAULT_LABELS[labelIdx];
  const confU32 = digest.readUInt32LE(4);
  // **Audit fix Φ-C-SDK.B1** — pre-fix used `Math.fround(confU32 / 0xffff_ffff)`
  // which divides f64/f64 then rounds to f32. Rust does
  // `(confU32 as f32) / (u32::MAX as f32)` (= cast both to f32 first,
  // then divide in f32). Empirical : 91/10000 random confU32 values
  // diverged by 1 ulp f32. Post-fix : explicit two-cast pattern matches
  // Rust bit-identical.
  //
  // Rationale on `Math.fround(0xffff_ffff)` :
  //   u32::MAX = 4294967295 is NOT exactly representable in f32 ; the
  //   nearest f32 is 4294967296 (= 2^32). Both Rust `(u32::MAX as f32)`
  //   and JS `Math.fround(0xffff_ffff)` produce 4294967296.0_f32.
  const confidence = Math.fround(
    Math.fround(confU32) / Math.fround(0xffff_ffff),
  );
  const rounds = 4 + (digest[8] % 16);
  const swarmId = "swarm-" + digest.subarray(0, 8).toString("hex");

  return {
    fixture_id: f.id,
    label,
    confidence,
    cone_id: "cone-" + f.catalog_id,
    rounds_executed: rounds,
    swarm_id: swarmId,
    catalog_version: DEFAULT_CATALOG_VERSION,
    sub_scores: [],
    latency_ms: 0.0,
  };
}

function main(): number {
  const fixturesPath = process.env.FIXTURES_PATH || "/fixtures/quartet_fixtures.json";
  const outPath = process.env.OUT_PATH || "/shared/out_ts.json";
  const versionLabel = process.env.SDK_VERSION || "0.6.0";

  let raw: string;
  try {
    raw = readFileSync(fixturesPath, "utf-8");
  } catch (e) {
    console.error(`FATAL: read fixtures: ${e}`);
    return 2;
  }
  const payload: FixturePayload = JSON.parse(raw);

  const mockMode = process.env.EFE_CONTRACT_MOCK === "1";
  if (!mockMode) {
    console.error(
      "FATAL : live gRPC path not yet wired ; set EFE_CONTRACT_MOCK=1 to use mock"
    );
    return 2;
  }

  const verdicts: Verdict[] = payload.fixtures.map(mockVerdict);
  const output: OutputPayload = {
    sdk: "ts",
    version: versionLabel,
    count: verdicts.length,
    verdicts,
  };

  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, JSON.stringify(output, null, 2) + "\n", "utf-8");
  console.error(`runner-ts : wrote ${verdicts.length} verdicts → ${outPath}`);
  return 0;
}

process.exit(main());
