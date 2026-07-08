// Types for the `path` polyfill (`--polyfill path`) - unjs/pathe, matches
// Node's `path` module shape (POSIX-only; pathe's win32/posix are aliases
// of the same implementation).

interface ParsedPath {
  root: string;
  dir: string;
  base: string;
  ext: string;
  name: string;
}

interface PathModule {
  join(...paths: string[]): string;
  dirname(path: string): string;
  basename(path: string, ext?: string): string;
  extname(path: string): string;
  resolve(...paths: string[]): string;
  relative(from: string, to: string): string;
  normalize(path: string): string;
  isAbsolute(path: string): boolean;
  parse(path: string): ParsedPath;
  format(parsed: Partial<ParsedPath>): string;
  matchesGlob(path: string, pattern: string): boolean;
  toNamespacedPath(path: string): string;
  readonly delimiter: string;
  readonly sep: string;
  readonly posix: PathModule;
  readonly win32: PathModule;
}

declare const path: PathModule;
