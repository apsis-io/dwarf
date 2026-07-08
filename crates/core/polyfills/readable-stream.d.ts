// Types for the `readable-stream` polyfill (`--polyfill readable-stream`) -
// hand-written minimal ReadableStream, not the full spec (no BYOB readers,
// tee(), pipeTo/pipeThrough).

interface ReadableStreamDefaultController<T = unknown> {
  enqueue(chunk: T): void;
  close(): void;
  error(reason?: unknown): void;
  readonly desiredSize: number | null;
}

interface ReadableStreamDefaultReader<T = unknown> {
  read(): Promise<{ value: T; done: false } | { value: undefined; done: true }>;
  cancel(reason?: unknown): Promise<void>;
  releaseLock(): void;
}

interface UnderlyingSource<T = unknown> {
  start?(controller: ReadableStreamDefaultController<T>): void | Promise<void>;
  pull?(controller: ReadableStreamDefaultController<T>): void | Promise<void>;
  cancel?(reason?: unknown): void | Promise<void>;
}

declare class ReadableStream<T = unknown> {
  constructor(source?: UnderlyingSource<T>);
  getReader(): ReadableStreamDefaultReader<T>;
  cancel(reason?: unknown): Promise<void>;
}

declare namespace wit {
  /**
   * Wraps a `wit.Stream()` readable end as a real ReadableStream. Single-read
   * only (see README): reads once with the given chunk size (default 65536)
   * then closes - a real drain loop isn't safe (reading a WASI stream again
   * after the writable end drops and all data is consumed is a hard
   * host-level trap, not a catchable error).
   */
  function readableStreamFromStream(readable: unknown, chunkSize?: number): ReadableStream<Uint8Array>;
}
