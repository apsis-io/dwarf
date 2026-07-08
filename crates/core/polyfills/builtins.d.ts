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
