// Types for the `ufo` polyfill (`--polyfill ufo`) - functional URL utilities
// from unjs/ufo, complementing the class-based `URL`/`URLSearchParams` the
// `url` polyfill provides. Exposed as a `ufo` namespace object rather than
// individual globals, to avoid crowding globalThis with 50+ small functions.
//
// Not a full re-declaration of every export - covers the commonly-used
// surface. See https://github.com/unjs/ufo for the complete API.

type QueryValue = string | number | undefined | null | boolean | QueryValue[] | Record<string, unknown>;
type QueryObject = Record<string, QueryValue | QueryValue[]>;
type ParsedQuery = Record<string, string | string[]>;

interface ParsedURL {
  protocol?: string;
  host?: string;
  auth?: string;
  href?: string;
  pathname: string;
  hash: string;
  search: string;
}
type ParsedPath = Pick<ParsedURL, "pathname" | "hash" | "search">;
interface ParsedAuth {
  username: string;
  password: string;
}
interface ParsedHost {
  hostname: string;
  port: string;
}

declare const ufo: {
  // Query utils
  parseQuery<T extends ParsedQuery = ParsedQuery>(parametersString?: string): T;
  stringifyQuery(query: QueryObject): string;
  encodeQueryItem(key: string, value: QueryValue | QueryValue[]): string;
  getQuery<T extends ParsedQuery = ParsedQuery>(input: string): T;
  withQuery(input: string, query: QueryObject): string;
  filterQuery(input: string, predicate: (key: string, value: string | string[]) => boolean): string;

  // Encoding utils
  encode(text: string | number): string;
  decode(text?: string | number): string;
  encodePath(text: string | number): string;
  decodePath(text: string): string;
  encodeHash(text: string): string;

  // Parsing
  parseURL(input?: string, defaultProto?: string): ParsedURL;
  parsePath(input?: string): ParsedPath;
  parseAuth(input?: string): ParsedAuth;
  parseHost(input?: string): ParsedHost;
  stringifyParsedURL(parsed: Partial<ParsedURL>): string;

  // Path/slash utils
  hasTrailingSlash(input?: string, respectQueryAndFragment?: boolean): boolean;
  withoutTrailingSlash(input?: string, respectQueryAndFragment?: boolean): string;
  withTrailingSlash(input?: string, respectQueryAndFragment?: boolean): string;
  hasLeadingSlash(input?: string): boolean;
  withoutLeadingSlash(input?: string): string;
  withLeadingSlash(input?: string): string;
  cleanDoubleSlashes(input?: string): string;

  // Joining/resolving
  joinURL(base: string, ...input: string[]): string;
  resolveURL(base?: string, ...inputs: string[]): string;
  withBase(input: string, base: string): string;
  withoutBase(input: string, base: string): string;
  normalizeURL(input: string): string;

  // Protocol utils
  hasProtocol(inputString: string, acceptRelative?: boolean): boolean;
  isRelative(inputString: string): boolean;
  withHttp(input: string): string;
  withHttps(input: string): string;
  withoutProtocol(input: string): string;
  withProtocol(input: string, protocol: string): string;

  // Fragment utils
  withFragment(input: string, hash: string): string;
  withoutFragment(input: string): string;
  withoutHost(input: string): string;

  // Comparison
  isSamePath(p1: string, p2: string): boolean;
  isEqual(a: string, b: string, options?: { leadingSlash?: boolean; trailingSlash?: boolean; encoding?: boolean }): boolean;
  isEmptyURL(url: string): boolean;
  isNonEmptyURL(url: string): boolean | "";
};
