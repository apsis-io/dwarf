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
declare var crypto: {
  getRandomValues<T extends ArrayBufferView>(typedArray: T): T;
};
