// Types for the `buffer` polyfill (`--polyfill buffer`). Covers the common
// feross/buffer surface, not exhaustive.
type BufferEncoding = "utf8" | "utf-8" | "hex" | "base64" | "ascii" | "latin1" | "binary" | "ucs2" | "ucs-2" | "utf16le" | "utf-16le";

// `slice`/`fill` deliberately NOT overridden here (they'd otherwise return
// `Buffer`, since that's what feross/buffer's real implementation returns) -
// overriding either fights newer TypeScript's generic
// Uint8Array<TArrayBuffer> variance checking (confirmed while testing;
// different TS versions disagree about it) for no real benefit over just
// inheriting Uint8Array's own signature. They still work at runtime and
// still type-check, just as returning `Uint8Array` rather than `Buffer` -
// re-wrap with `Buffer.from(buf.slice(...))` if you need the narrower type
// back for chaining.
interface Buffer extends Uint8Array {
  toString(encoding?: BufferEncoding, start?: number, end?: number): string;
  equals(other: Uint8Array): boolean;
  includes(value: string | number | Uint8Array, byteOffset?: number, encoding?: BufferEncoding): boolean;
  indexOf(value: string | number | Uint8Array, byteOffset?: number, encoding?: BufferEncoding): number;
  write(string: string, offset?: number, length?: number, encoding?: BufferEncoding): number;
  copy(target: Uint8Array, targetStart?: number, start?: number, end?: number): number;
}

interface BufferConstructor {
  from(value: string | ArrayBuffer | ArrayLike<number>, encodingOrOffset?: BufferEncoding | number, length?: number): Buffer;
  alloc(size: number, fill?: string | number | Uint8Array, encoding?: BufferEncoding): Buffer;
  allocUnsafe(size: number): Buffer;
  allocUnsafeSlow(size: number): Buffer;
  isBuffer(obj: unknown): obj is Buffer;
  isEncoding(encoding: string): encoding is BufferEncoding;
  compare(a: Uint8Array, b: Uint8Array): -1 | 0 | 1;
  concat(list: Uint8Array[], totalLength?: number): Buffer;
  byteLength(string: string | Uint8Array, encoding?: BufferEncoding): number;
}

declare const Buffer: BufferConstructor;
