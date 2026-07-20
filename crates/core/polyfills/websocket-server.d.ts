// Types for dwarf's always-on `WebSocketServer` (no --polyfill flag - wired
// automatically when the world imports wasi:sockets/types@0.3.0; see
// README.md "WebSockets" section for the full API, requirements (also needs
// --polyfill webcrypto for the handshake), and scope cuts).
//
// `WebSocketServerP3` is the same class as `WebSocketServer` - see
// console.d.ts's note on the `...P3` naming convention.
//
// The `on("request", ...)` general-HTTP-router overload (see README) is
// declared in fetch-classes.d.ts instead of here, via TS declaration
// merging - its signature references Request/Response, which only exist
// when that polyfill is also requested, same reason `fetch`'s own types
// live there instead of in this always-on file.

interface WebSocketConnection {
  readonly path: string;
  readonly headers: Record<string, string>;
  /** `data` is a string for a text frame, a Uint8Array for a binary frame. */
  send(data: string | Uint8Array): Promise<void>;
  ping(data?: Uint8Array): Promise<void>;
  close(code?: number, reason?: string): Promise<void>;
  on(event: "message", cb: (data: string | Uint8Array) => void): void;
  on(event: "ping" | "pong", cb: (data: Uint8Array) => void): void;
  on(event: "close", cb: (code: number, reason: string) => void): void;
  on(event: "error", cb: (err: unknown) => void): void;
}

declare class WebSocketServer {
  /**
   * `maxPayload` bounds a single WS frame's payload (default 100 MiB).
   * `maxBodyBytes` bounds an HTTP request body via its `Content-Length`
   * (default 10 MiB) - a request declaring more than this is rejected with
   * `413` before any of the body is read, not after buffering it.
   * `idleTimeoutMs` (default 30000) bounds how long the HTTP router will
   * wait for the next chunk of a request (headers or body) before giving
   * up on the connection - protects against a client that opens a
   * connection and then sends nothing, or trickles bytes forever
   * (slowloris-shaped). Only enforced when the world imports
   * `wasi:clocks/monotonic-clock@0.3.x`; without it, reads have no timeout
   * (graceful degradation, matching `setTimeout`'s own fallback - see
   * generate_timers).
   */
  constructor(opts?: { maxPayload?: number; maxBodyBytes?: number; idleTimeoutMs?: number });
  /** The actual bound port, set once `listen()` has bound the socket - useful when `port` was `0` (OS-assigned). */
  readonly port: number | null;
  on(event: "connection", cb: (conn: WebSocketConnection) => void): void;
  on(event: "error", cb: (err: unknown) => void): void;
  /**
   * Binds, listens, and accept-loops forever - await this from a long-lived
   * entrypoint. Each accepted connection runs an HTTP/1.1 keep-alive loop:
   * a WS upgrade request hands the connection to `"connection"`'s handler
   * for the rest of its life; any other request goes to `"request"`'s
   * handler if registered (--polyfill fetch-classes), or is otherwise
   * dropped, unchanged from before this option existed.
   */
  listen(port?: number, host?: string): Promise<void>;
  /** Stops the accept loop. */
  close(): void;
}

declare const WebSocketServerP3: typeof WebSocketServer;
