// Sprint D24 (Φ-B) — `MockEfeClient` : pure-JS test fixture, NO gRPC.
//
// Use this for unit tests that :
//   - inject canned verdicts ;
//   - assert the texts your handler classified ;
//   - simulate errors via `setError()`.
//
// For tests that need to exercise the FULL gRPC stack (interceptors,
// retry, OTel, transport), use `createMockServer()` instead.
//
// Cf. docs/EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md §17.1.

import { ErrClosed } from '../core/errors.js';
import type { Verdict } from '../core/verdict.js';
import type { CallOptions } from '../core/_types.js';

import { newVerdict } from './verdict.js';

/**
 * Drop-in replacement for `EfeClient` in unit tests.
 *
 * @example
 *   const mock = new MockEfeClient();
 *   mock.setVerdict(newVerdict({ label: 'fraud' }));
 *
 *   await myHandler(mock); // signature: (client) => Promise<...>
 *
 *   expect(mock.getCalls()).toEqual(['input text']);
 */
export class MockEfeClient {
  #verdict: Verdict | undefined;
  #error: Error | undefined;
  #calls: string[] = [];
  #closed = false;

  /** Set the verdict returned by subsequent `classify` calls. */
  setVerdict(v: Verdict): void {
    this.#verdict = v;
  }

  /** Cause subsequent `classify`/`healthcheck` calls to throw `err`. Pass `undefined` to clear. */
  setError(err: Error | undefined): void {
    this.#error = err;
  }

  /** Snapshot of the texts `classify` was called with, in invocation order. */
  getCalls(): readonly string[] {
    return [...this.#calls];
  }

  /** Number of recorded calls. */
  getCallCount(): number {
    return this.#calls.length;
  }

  /** Clear calls log + verdict + error + closed flag. */
  reset(): void {
    this.#verdict = undefined;
    this.#error = undefined;
    this.#calls = [];
    this.#closed = false;
  }

  /**
   * Mirror of `EfeClient.classify`.
   *
   * Behaviour :
   *   - Closed → throw ErrClosed.
   *   - setError(err) → throw err.
   *   - setVerdict(v) → return a copy of v.
   *   - else → return a fresh `newVerdict()`.
   */
  async classify(text: string, _opts: CallOptions = {}): Promise<Verdict> {
    if (this.#closed) {
      throw new ErrClosed('mock client is closed');
    }
    this.#calls.push(text);
    if (this.#error) {
      throw this.#error;
    }
    if (this.#verdict !== undefined) {
      // Return a deep copy so callers' mutations don't poison the next call.
      return { ...this.#verdict };
    }
    return newVerdict();
  }

  /**
   * Mirror of `EfeClient.healthcheck`. Symmetric : if SetError'd, fail.
   *
   * **NOTE** : matches the post-audit M9-A4 fix from Go SDK — no special
   * exception for ErrTimeout. Any non-undefined error fails healthcheck.
   */
  async healthcheck(_opts: CallOptions = {}): Promise<void> {
    if (this.#closed) {
      throw new ErrClosed('mock client is closed');
    }
    if (this.#error) {
      throw this.#error;
    }
  }

  /** Mark mock as closed. Subsequent calls throw ErrClosed. */
  async close(): Promise<void> {
    this.#closed = true;
  }

  get closed(): boolean {
    return this.#closed;
  }
}
