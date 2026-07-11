// Types for the `fetch-classes` polyfill (`--polyfill fetch-classes`) -
// Headers/Request/Response/DOMException from whatwg-fetch, trimmed, plus a
// real global `fetch()` (see crates/core/src/polyfills.rs's generate_fetch)
// wired directly to wasi:http/client@0.3.x when the world imports it (throws
// a clear error otherwise) - declared here rather than builtins.d.ts since
// its signature references Request/Response, which only exist when this
// polyfill is also requested.
//
// Body.blob()/formData() throw at runtime (no Blob/FormData in dwarf) -
// omitted here so using them is a type error rather than a silent runtime
// surprise. Use text()/json()/arrayBuffer() instead - all three work
// regardless of whether the body originated as a string, JSON, binary data,
// or was omitted entirely.

// AbortController/AbortSignal are a real, always-on global (see
// builtins.d.ts) - nothing in dwarf's own fetch() implementation observes
// `signal` yet, but a caller polling `signal.aborted` in its own loop works
// today.

type HeadersInit = Headers | Record<string, string> | Iterable<[string, string]>;

declare class Headers {
  constructor(init?: HeadersInit);
  append(name: string, value: string): void;
  delete(name: string): void;
  get(name: string): string | null;
  has(name: string): boolean;
  set(name: string, value: string): void;
  forEach(callback: (value: string, name: string, parent: Headers) => void, thisArg?: unknown): void;
  entries(): IterableIterator<[string, string]>;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  [Symbol.iterator](): IterableIterator<[string, string]>;
}

type BodyInit = string | Uint8Array | ArrayBuffer | URLSearchParams;

interface RequestInit {
  method?: string;
  headers?: HeadersInit;
  body?: BodyInit | null;
  credentials?: "omit" | "same-origin" | "include";
  mode?: "cors" | "no-cors" | "same-origin" | "navigate";
  signal?: AbortSignal | null;
}

declare class Request {
  constructor(input: string | Request, init?: RequestInit);
  readonly url: string;
  readonly method: string;
  readonly headers: Headers;
  readonly credentials: string;
  readonly mode: string | null;
  readonly signal: AbortSignal | null;
  readonly bodyUsed: boolean;
  text(): Promise<string>;
  json(): Promise<unknown>;
  arrayBuffer(): Promise<ArrayBuffer>;
  clone(): Request;
}

interface ResponseInit {
  status?: number;
  statusText?: string;
  headers?: HeadersInit;
}

declare class Response {
  constructor(body?: BodyInit | null, init?: ResponseInit);
  readonly type: string;
  readonly status: number;
  readonly ok: boolean;
  readonly statusText: string;
  readonly headers: Headers;
  readonly bodyUsed: boolean;
  text(): Promise<string>;
  json(): Promise<unknown>;
  arrayBuffer(): Promise<ArrayBuffer>;
  clone(): Response;
  static error(): Response;
  static redirect(url: string, status?: number): Response;
}

declare class DOMException extends Error {
  constructor(message?: string, name?: string);
}

// Requires importing wasi:http/client@0.3.x - throws a clear error
// otherwise. Single-read response body (bodies larger than 64KiB are
// truncated - see generate_fetch's doc comment for why).
declare function fetch(input: string | Request, init?: RequestInit): Promise<Response>;
