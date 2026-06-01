// Sprint D24 (Φ-B) — Public `Verdict` interface + helpers.
//
// Cross-language pinned : 10 fields match Python M8 + Go M9 SDKs
// EXACTLY. Adding/renaming any field requires a coordinated bump
// across the 3 SDKs + the wire contract proto.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §10.4.

/**
 * Verdict is the public verdict type returned by `EfeClient.classify`.
 *
 * Conventionally immutable (`readonly` enforced via TS) — the SDK never
 * mutates a Verdict after returning it, and callers SHOULD NOT either.
 *
 * All fields are required to keep the JSON wire shape stable across
 * the 3 SDKs. Empty strings / 0 are used for "not provided" rather
 * than `undefined`.
 *
 * @example
 * ```ts
 * const v: Verdict = await client.classify('refund please');
 * if (v.zone === 'trusted') console.log(v.label, v.confidence);
 * ```
 */
export interface Verdict {
  /** Predicted class name (e.g. "fraud", "billing"). Empty when reject==true. */
  readonly label: string;
  /** Confidence in [0, 1]. Higher = more confident the label is right. */
  readonly confidence: number;
  /** True iff the swarm decided the input is OUT-of-cone (alignment violation). */
  readonly reject: boolean;
  /** Number of fusion rounds the swarm executed before converging. */
  readonly rounds: number;
  /** Distance from the alignment cone boundary, in [0, 1]. 0 = perfectly aligned. */
  readonly alignmentDelta: number;
  /** Canonical key of the catalog used (e.g. "acme/billing@v3"). */
  readonly catalog: string;
  /** "trusted" | "warn" | "reject" — derived from confidence + threshold policy. */
  readonly zone: string;
  /** Server-assigned id of the ephemeral swarm instance that produced this verdict. */
  readonly swarmId: string;
  /** gRPC request id (echoed by the server). Empty when not supplied. */
  readonly requestId: string;
  /** SDK-side wall-clock latency from RPC start to RPC return, in milliseconds. */
  readonly elapsedMs: number;
}

/**
 * StreamItem is one verdict + index pair from `EfeClient.classifyStream`
 * (Node-only API).
 *
 * `index` preserves the input order even when the server returns
 * out-of-order. `error` is non-undefined iff this particular item
 * failed ; the stream as a whole continues unless ctx is cancelled.
 */
export interface StreamItem {
  readonly index: number;
  readonly verdict?: Verdict;
  readonly error?: Error;
}

/** Returns true iff the verdict is in the "trusted" zone. */
export function isTrusted(v: Verdict): boolean {
  return !v.reject && v.zone === 'trusted';
}

/** Returns true iff the verdict is borderline (warn zone). */
export function isWarn(v: Verdict): boolean {
  return !v.reject && v.zone === 'warn';
}

/** Returns true iff the verdict is rejected. */
export function isReject(v: Verdict): boolean {
  return v.reject || v.zone === 'reject';
}

/**
 * Default zone derivation : trusted if confidence ≥ 0.80, warn if ≥ 0.50,
 * else reject. Matches Go SDK `deriveZone()`.
 */
export function deriveZone(confidence: number): string {
  if (confidence >= 0.8) return 'trusted';
  if (confidence >= 0.5) return 'warn';
  return 'reject';
}
