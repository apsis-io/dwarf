// Types for the `ohash` polyfill (`--polyfill ohash`) - hashing utilities
// from unjs/ohash, bundled with its pure-JS (non-Node) SHA-256-based digest.
// Not for security use ("best efforts... not designed for security
// purposes" per ohash's own docs) - use `--polyfill webcrypto`'s
// `crypto.subtle.digest` for anything security-sensitive.

declare const ohash: {
  /** Serializes any value into a stable string, then hashes it. */
  hash(input: unknown): string;
  /** Serializes any value into a stable string (used internally by hash()). */
  serialize(input: unknown): string;
  /** Deep-equality check via serialize() comparison. */
  isEqual(a: unknown, b: unknown): boolean;
};
