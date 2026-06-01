// Sprint D24 (Φ-B) — `classifyStream` impl (Node-only).
//
// Single-dispatcher pattern : guarantees that StreamItem.index reflects
// the input position regardless of concurrent worker scheduling.
// Mirrors the M9 audit fix M9-A3 (Go SDK).
//
// **Audit fix M10-A5 + M10-A6** — the previous wakeup-promise pattern
// had two races :
//
//   - M10-A5 : consumer drains `out`, sets `wakeupResolve` ; in the
//     window between drain and `await`, a worker pushes + signals.
//     The signal becomes a no-op because `wakeupResolve` is null at
//     the exact moment of the push. Then the consumer awaits forever.
//
//   - M10-A6 : multiple workers race to set `producerDone = true` when
//     `workersAlive === 0`. Could double-decrement or signal twice.
//
// Fix : replace the ad-hoc wakeup pattern with a `AsyncQueue<T>`
// abstraction. Workers push() (sync) ; consumer shift() (async). The
// queue is the single source of truth for "data available" + "no more
// data" signals — no manual race tracking. The orchestrator closes the
// queue once Promise.all of (dispatcher + workers) settles.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §10.1.

import type { ClassifyStreamOptions, IndexedText } from '../core/_types.js';
import type { StreamItem } from '../core/verdict.js';
import type { EfeClient } from './client.js';

const DEFAULT_CONCURRENCY = 8;

/**
 * Race-safe async queue : single-producer / single-consumer / N workers.
 *
 *   - push(value)    : sync. If a consumer is already awaiting (via
 *                      shift), resolves it immediately ; else buffers.
 *   - shift()        : async. Resolves with `{value, done:false}` on
 *                      next push, or `{value:undefined, done:true}` if
 *                      the queue is closed AND empty.
 *   - close()        : drain all pending consumers with `{done:true}`.
 *
 * Race-safe by construction : every push is delivered exactly once
 * (either to a waiting consumer OR to the buffer ; close drains pending
 * consumers atomically).
 */
class AsyncQueue<T> {
  private readonly items: T[] = [];
  private readonly pending: ((value: { value: T; done: false } | { value: undefined; done: true }) => void)[] = [];
  private closed = false;

  push(value: T): void {
    if (this.closed) {
      // Refuse silently — late-arriving items after close are dropped.
      // Caller's responsibility to push() before close().
      return;
    }
    const waiter = this.pending.shift();
    if (waiter !== undefined) {
      waiter({ value, done: false });
    } else {
      this.items.push(value);
    }
  }

  /** Mark queue closed. Pending consumers receive `{done: true}`. */
  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const waiter of this.pending.splice(0, this.pending.length)) {
      waiter({ value: undefined, done: true });
    }
  }

  /**
   * Shift one item. If queue is empty AND closed, resolves with
   * `{done:true}`. If empty + open, suspends until next push or close.
   */
  shift(): Promise<{ value: T; done: false } | { value: undefined; done: true }> {
    const item = this.items.shift();
    if (item !== undefined) {
      return Promise.resolve({ value: item, done: false });
    }
    if (this.closed) {
      return Promise.resolve({ value: undefined, done: true });
    }
    return new Promise((resolve) => {
      this.pending.push(resolve);
    });
  }
}

/**
 * Worker-pool-backed batch classifier with strict input-order indices.
 *
 * Architecture :
 *   1. DISPATCHER  : single goroutine reads `texts`, assigns monotonic
 *      indices, pushes (index, text) to `fanout` queue.
 *   2. WORKERS     : N goroutines (default 8) consume from `fanout`,
 *      call `client.classify`, push results to `output` queue.
 *   3. ORCHESTRATOR: closes `output` queue once Promise.all of
 *      (dispatcher + workers) settles. ELIMINATES race M10-A6.
 *   4. CONSUMER    : `for await ... of` pulls from `output`.
 *
 * Cancellation : `opts.signal` aborts dispatcher + workers ; in-flight
 * RPCs are cancelled via the same signal.
 */
export function classifyStreamImpl(
  client: EfeClient,
  texts: AsyncIterable<string> | Iterable<string>,
  opts: ClassifyStreamOptions,
): AsyncIterable<StreamItem> {
  const concurrency = opts.concurrency ?? DEFAULT_CONCURRENCY;
  if (concurrency < 1) {
    throw new Error('concurrency must be ≥ 1');
  }

  const fanout = new AsyncQueue<IndexedText>();
  const output = new AsyncQueue<StreamItem>();

  // ── Dispatcher ──
  const dispatcherPromise = (async () => {
    let idx = 0;
    try {
      for await (const text of asAsyncIterable(texts)) {
        if (opts.signal?.aborted) break;
        fanout.push({ index: idx, text });
        idx += 1;
      }
    } finally {
      fanout.close();
    }
  })();

  // ── Worker pool ──
  const workerPromises: Promise<void>[] = [];
  for (let w = 0; w < concurrency; w++) {
    workerPromises.push(
      (async () => {
        while (true) {
          if (opts.signal?.aborted) return;
          const next = await fanout.shift();
          if (next.done) return;
          const item = next.value;
          try {
            const verdict = await client.classify(item.text, {
              ...(opts.signal !== undefined ? { signal: opts.signal } : {}),
              ...(opts.timeout !== undefined ? { timeout: opts.timeout } : {}),
            });
            output.push({ index: item.index, verdict });
          } catch (err) {
            output.push({ index: item.index, error: err as Error });
          }
        }
      })(),
    );
  }

  // ── Orchestrator : close output once all workers + dispatcher settle ──
  // **Audit fix M10-A6** : single Promise.all replaces worker-individual
  // decrement-and-set-done. No race possible — Promise.all resolves
  // exactly once when ALL inputs settle.
  void Promise.all([dispatcherPromise, ...workerPromises])
    .catch((err) => {
      // Dispatcher / worker fatal error — surface as sentinel item.
      output.push({ index: -1, error: err as Error });
    })
    .finally(() => {
      output.close();
    });

  // ── Output AsyncIterable ──
  return {
    async *[Symbol.asyncIterator](): AsyncIterator<StreamItem> {
      while (true) {
        const next = await output.shift();
        if (next.done) return;
        yield next.value;
      }
    },
  };
}

/** Coerce a sync Iterable to AsyncIterable. */
function asAsyncIterable<T>(src: AsyncIterable<T> | Iterable<T>): AsyncIterable<T> {
  if (Symbol.asyncIterator in Object(src)) {
    return src as AsyncIterable<T>;
  }
  return {
    async *[Symbol.asyncIterator](): AsyncIterator<T> {
      for (const item of src as Iterable<T>) {
        yield item;
      }
    },
  };
}
