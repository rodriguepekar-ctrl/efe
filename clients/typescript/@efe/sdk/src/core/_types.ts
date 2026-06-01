// Sprint D24 (Φ-B) — Internal type aliases (NOT exported publicly).
//
// Helpers used across `src/node/` + `src/web/` to keep type signatures
// terse without leaking through `@efe/sdk` package exports.

/** Per-RPC call options accepted by `EfeClient.classify` / `.healthcheck`. */
export interface CallOptions {
  /** Optional AbortSignal — call is cancelled when the signal fires. */
  readonly signal?: AbortSignal;
  /** Per-call deadline override (ms). Falls back to client-level `deadline`. */
  readonly timeout?: number;
}

/** Stream-specific call options (Node-only `classifyStream`). */
export interface ClassifyStreamOptions extends CallOptions {
  /** Worker pool size. Default 8. */
  readonly concurrency?: number;
}

/** Indexed text used by the dispatcher pattern in classifyStream. */
export interface IndexedText {
  readonly index: number;
  readonly text: string;
}
