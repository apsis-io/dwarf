// Types for dwarf's always-on builtins (no --polyfill flag needed) - see
// README.md "Polyfills" section.

declare class TextEncoder {
  readonly encoding: "utf-8";
  encode(input?: string): Uint8Array;
}

declare class TextDecoder {
  readonly encoding: string;
  constructor(label?: string);
  decode(input?: Uint8Array | ArrayBuffer | ArrayLike<number>): string;
}

// crypto.getRandomValues is always generated (wired to wasi:random/random,
// throws a clear error if that's not imported) - independent of
// `--polyfill webcrypto`, which only adds `crypto.subtle` (see
// webcrypto.d.ts). Declared as `var` (not `const`) since the `webcrypto`
// polyfill extends the same `globalThis.crypto` object with `.subtle`.
// `getRandomValuesP3` is the same function as `getRandomValues` - see
// console.d.ts's note on the `...P3` naming convention.
declare var crypto: {
  getRandomValues<T extends ArrayBufferView>(typedArray: T): T;
  getRandomValuesP3<T extends ArrayBufferView>(typedArray: T): T;
};

interface AbortEvent {
  readonly type: "abort";
  readonly target: AbortSignal;
}

declare class AbortSignal {
  readonly aborted: boolean;
  readonly reason: unknown;
  onabort: ((event: AbortEvent) => void) | null;
  throwIfAborted(): void;
  addEventListener(type: "abort", listener: (event: AbortEvent) => void): void;
  removeEventListener(type: "abort", listener: (event: AbortEvent) => void): void;
  // No `AbortSignal.timeout()` static - it would need `setTimeout`, and this
  // stays dependency-free on purpose.
  static abort(reason?: unknown): AbortSignal;
}

declare class AbortController {
  readonly signal: AbortSignal;
  abort(reason?: unknown): void;
}

// setTimeout/setInterval require importing wasi:clocks/monotonic-clock@0.3.x
// (throw a clear error otherwise - wasi:clocks 0.2 has no non-blocking wait
// primitive). clearTimeout/clearInterval are always safe no-ops, even
// without that import. IMPORTANT: an unawaited timer's callback is cancelled
// if the async export that (transitively) created it settles first - a real
// component-model-async constraint, not a dwarf bug. See generate_timers in
// crates/core/src/polyfills.rs for the full explanation.
//
// The `...P3` counterparts are the same functions as the plain names - see
// console.d.ts's note on the naming convention.
declare function setTimeout(fn: (...args: unknown[]) => void, ms?: number, ...args: unknown[]): number;
declare function clearTimeout(handle: number | undefined): void;
declare function setInterval(fn: (...args: unknown[]) => void, ms?: number, ...args: unknown[]): number;
declare function clearInterval(handle: number | undefined): void;
declare function setTimeoutP3(fn: (...args: unknown[]) => void, ms?: number, ...args: unknown[]): number;
declare function clearTimeoutP3(handle: number | undefined): void;
declare function setIntervalP3(fn: (...args: unknown[]) => void, ms?: number, ...args: unknown[]): number;
declare function clearIntervalP3(handle: number | undefined): void;
