// Types for dwarf's always-on `process` (no --polyfill flag - wired
// automatically when the world imports wasi:cli/environment/exit; see
// README.md "Process" section for divergences from Node and exactly what's
// backed vs. what throws).
//
// `processP3` is the same object as `process` - see console.d.ts's note on
// the `...P3` naming convention for why it exists alongside the plain name.

interface Process {
  /** Always re-fetched from wasi:cli/environment on access, never cached. */
  readonly env: Record<string, string>;
  /** Exactly wasi:cli/environment's get-arguments() - no synthetic node/script-path entries prepended. */
  readonly argv: string[];
  /** `null` (not a fabricated path) when wasi:cli/environment's initial-cwd() is none. */
  cwd(): string | null;
  /** Maps onto wasi:cli/exit's exit-with-code(status-code: u8) - code is coerced into a single byte. */
  exit(code?: number): never;
}

declare const process: Process;
declare const processP3: Process;
