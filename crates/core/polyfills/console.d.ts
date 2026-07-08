// Types for dwarf's always-on `console` (no --polyfill flag - wired
// automatically when the world imports wasi:cli/stdout/stderr; see README.md
// "Console" section for exactly what's backed vs. what throws).

interface Console {
  /** `void` when your world imports wasi:cli/stdout@0.2.x (sync, guaranteed complete on return); `Promise<void>` when it only has wasi:cli/stdout@0.3.x (no sync write primitive exists in 0.3) - await the return value if you need the write to have completed and aren't sure which applies. */
  log(...args: unknown[]): void | Promise<void>;
  info(...args: unknown[]): void | Promise<void>;
  debug(...args: unknown[]): void | Promise<void>;
  warn(...args: unknown[]): void | Promise<void>;
  error(...args: unknown[]): void | Promise<void>;
  /** Always exists and always returns a Promise (rejects rather than throwing synchronously) - see README's "Async logging" section for the safety constraint on the WASI 0.3 path. */
  print(...args: unknown[]): Promise<void>;
  println(...args: unknown[]): Promise<void>;
  eprint(...args: unknown[]): Promise<void>;
  eprintln(...args: unknown[]): Promise<void>;
}

declare const console: Console;
