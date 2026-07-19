// Types for dwarf's always-on `console` (no --polyfill flag - wired
// automatically when the world imports wasi:cli/stdout/stderr@0.3.x; see
// README.md "Console" section for exactly what's backed vs. what throws).
//
// `consoleP3` is the same object as `console` - a stable, version-pinned
// name for code that wants "the 0.3 implementation, specifically" and
// doesn't want to be affected if a future WASI version ever changes what
// the plain `console` points to. Prefer the plain name day to day; use
// `import { consoleP3 as console } from "..."` (TS import aliasing) at your
// own call site if you want the short name back after depending on the
// pinned one.

interface Console {
  /** Always returns a Promise (WASI 0.3 has no synchronous write primitive) - await it if you need the write to have completed. */
  log(...args: unknown[]): Promise<void>;
  info(...args: unknown[]): Promise<void>;
  debug(...args: unknown[]): Promise<void>;
  warn(...args: unknown[]): Promise<void>;
  error(...args: unknown[]): Promise<void>;
  /** Always exists and always returns a Promise (rejects rather than throwing synchronously) - see README's "Async logging" section for the safety constraint on the WASI 0.3 path. */
  print(...args: unknown[]): Promise<void>;
  println(...args: unknown[]): Promise<void>;
  eprint(...args: unknown[]): Promise<void>;
  eprintln(...args: unknown[]): Promise<void>;
}

declare const console: Console;
declare const consoleP3: Console;
