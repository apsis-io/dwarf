// Types for dwarf's always-on `WebSocketServer` (no --polyfill flag - wired
// automatically when the world imports wasi:sockets/types@0.3.0; see
// README.md "WebSockets" section for the full API, requirements (also needs
// --polyfill webcrypto for the handshake), and scope cuts).
//
// `WebSocketServerP3` is the same class as `WebSocketServer` - see
// console.d.ts's note on the `...P3` naming convention.

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
  constructor(opts?: { maxPayload?: number });
  /** The actual bound port, set once `listen()` has bound the socket - useful when `port` was `0` (OS-assigned). */
  readonly port: number | null;
  on(event: "connection", cb: (conn: WebSocketConnection) => void): void;
  on(event: "error", cb: (err: unknown) => void): void;
  /** Binds, listens, and accept-loops forever - await this from a long-lived entrypoint. */
  listen(port?: number, host?: string): Promise<void>;
  /** Stops the accept loop. */
  close(): void;
}

declare const WebSocketServerP3: typeof WebSocketServer;
