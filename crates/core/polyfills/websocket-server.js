// websocket-server.js - RFC 6455 server-side WebSocket support, plus a
// general single-port HTTP+WS router (WebSocketServer's raw-socket HTTP
// parser also serves plain HTTP requests when a 'request' handler is
// registered - lets one wasi:sockets listener carry both a normal HTTP
// (e.g. SSR) response path and WebSocket upgrades on the same port, which
// matters for hosts that front a component with a single-SNI/single-backend
// path that can't be split across two ports).
//
// The frame parser's validation rules (RSV bit checks, opcode/fragmentation
// legality, control-frame constraints, mask enforcement, UTF-8 and close-
// status-code validation) and its buffered byte-consumption state machine
// are adapted from websockets/ws's receiver.js (see NOTICES) - permessage-
// deflate, Blob support and client-mode (unmasked-outgoing) framing are all
// dropped since none of them apply to a server-only, dependency-free port,
// and Node's Writable/Buffer/EventEmitter are replaced with plain
// Uint8Array and a minimal callback list.

(function () {
const WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

function concatBytes(chunks, totalLength) {
  if (chunks.length === 1) return chunks[0];
  const target = new Uint8Array(totalLength);
  let offset = 0;
  for (const chunk of chunks) {
    target.set(chunk, offset);
    offset += chunk.length;
  }
  return target;
}

function maskBytes(buf, mask) {
  for (let i = 0; i < buf.length; i++) buf[i] ^= mask[i & 3];
}

function readUInt16BE(buf, offset) {
  return (buf[offset] << 8) | buf[offset + 1];
}

function readUInt32BE(buf, offset) {
  return (
    ((buf[offset] << 24) | (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3]) >>>
    0
  );
}

// Base64 encode - trivial enough to inline rather than require the `buffer`
// polyfill just for computing Sec-WebSocket-Accept.
const BASE64_CHARS =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
function base64Encode(bytes) {
  let out = "";
  let i = 0;
  for (; i + 2 < bytes.length; i += 3) {
    const n = (bytes[i] << 16) | (bytes[i + 1] << 8) | bytes[i + 2];
    out +=
      BASE64_CHARS[(n >> 18) & 0x3f] +
      BASE64_CHARS[(n >> 12) & 0x3f] +
      BASE64_CHARS[(n >> 6) & 0x3f] +
      BASE64_CHARS[n & 0x3f];
  }
  const remaining = bytes.length - i;
  if (remaining === 1) {
    const n = bytes[i] << 16;
    out += BASE64_CHARS[(n >> 18) & 0x3f] + BASE64_CHARS[(n >> 12) & 0x3f] + "==";
  } else if (remaining === 2) {
    const n = (bytes[i] << 16) | (bytes[i + 1] << 8);
    out +=
      BASE64_CHARS[(n >> 18) & 0x3f] +
      BASE64_CHARS[(n >> 12) & 0x3f] +
      BASE64_CHARS[(n >> 6) & 0x3f] +
      "=";
  }
  return out;
}

// Ported from ws's validation.js (Markus Kuhn's utf8_check.c algorithm).
function isValidUTF8(buf) {
  const len = buf.length;
  let i = 0;
  while (i < len) {
    if ((buf[i] & 0x80) === 0) {
      i++;
    } else if ((buf[i] & 0xe0) === 0xc0) {
      if (i + 1 === len || (buf[i + 1] & 0xc0) !== 0x80 || (buf[i] & 0xfe) === 0xc0) return false;
      i += 2;
    } else if ((buf[i] & 0xf0) === 0xe0) {
      if (
        i + 2 >= len ||
        (buf[i + 1] & 0xc0) !== 0x80 ||
        (buf[i + 2] & 0xc0) !== 0x80 ||
        (buf[i] === 0xe0 && (buf[i + 1] & 0xe0) === 0x80) ||
        (buf[i] === 0xed && (buf[i + 1] & 0xe0) === 0xa0)
      ) {
        return false;
      }
      i += 3;
    } else if ((buf[i] & 0xf8) === 0xf0) {
      if (
        i + 3 >= len ||
        (buf[i + 1] & 0xc0) !== 0x80 ||
        (buf[i + 2] & 0xc0) !== 0x80 ||
        (buf[i + 3] & 0xc0) !== 0x80 ||
        (buf[i] === 0xf0 && (buf[i + 1] & 0xf0) === 0x80) ||
        (buf[i] === 0xf4 && buf[i + 1] > 0x8f) ||
        buf[i] > 0xf4
      ) {
        return false;
      }
      i += 4;
    } else {
      return false;
    }
  }
  return true;
}

function isValidStatusCode(code) {
  return (
    (code >= 1000 && code <= 1014 && code !== 1004 && code !== 1005 && code !== 1006) ||
    (code >= 3000 && code <= 4999)
  );
}

// --- Frame parser -------------------------------------------------------
// State machine shape ported from ws's receiver.js: GET_INFO reads the
// opcode/FIN/RSV/mask-bit/length-hint byte pair, GET_PAYLOAD_LENGTH_16/64
// read the extended length when the 7-bit hint asks for it, GET_MASK reads
// the 4-byte masking key (always present - RFC 6455 5.1 requires every
// client-to-server frame to be masked), GET_DATA reads and unmasks the
// payload. Each state bails out (leaving `_state` unchanged) when not
// enough buffered bytes are available yet, resuming on the next `write()`.

const GET_INFO = 0;
const GET_PAYLOAD_LENGTH_16 = 1;
const GET_PAYLOAD_LENGTH_64 = 2;
const GET_MASK = 3;
const GET_DATA = 4;

const OPCODE_CONTINUATION = 0x0;
const OPCODE_TEXT = 0x1;
const OPCODE_BINARY = 0x2;
const OPCODE_CLOSE = 0x8;
const OPCODE_PING = 0x9;
const OPCODE_PONG = 0xa;

class WebSocketFrameError extends Error {
  constructor(message, closeCode) {
    super(message);
    this.closeCode = closeCode;
  }
}

class FrameParser {
  constructor(maxPayload) {
    this._maxPayload = maxPayload || 100 * 1024 * 1024;
    this._buffers = [];
    this._bufferedBytes = 0;
    this._state = GET_INFO;
    this._loop = false;
    this._concluded = false;
    this._fin = false;
    this._opcode = 0;
    this._masked = false;
    this._mask = null;
    this._payloadLength = 0;
    this._fragmented = 0;
    this._fragments = [];
    this._totalPayloadLength = 0;
    this._messageLength = 0;
    this.onMessage = null; // (data: string|Uint8Array) => void
    this.onPing = null; // (data: Uint8Array) => void
    this.onPong = null; // (data: Uint8Array) => void
    this.onClose = null; // (code: number, reason: string) => void
  }

  write(chunk) {
    if (this._concluded || chunk.length === 0) return;
    this._bufferedBytes += chunk.length;
    this._buffers.push(chunk);
    this._runLoop();
  }

  _consume(n) {
    this._bufferedBytes -= n;
    if (n === this._buffers[0].length) return this._buffers.shift();
    if (n < this._buffers[0].length) {
      const buf = this._buffers[0];
      this._buffers[0] = buf.subarray(n);
      return buf.subarray(0, n);
    }
    const dst = new Uint8Array(n);
    let filled = 0;
    let remaining = n;
    while (remaining > 0) {
      const buf = this._buffers[0];
      if (remaining >= buf.length) {
        dst.set(buf, filled);
        filled += buf.length;
        remaining -= buf.length;
        this._buffers.shift();
      } else {
        dst.set(buf.subarray(0, remaining), filled);
        this._buffers[0] = buf.subarray(remaining);
        filled += remaining;
        remaining = 0;
      }
    }
    return dst;
  }

  _runLoop() {
    this._loop = true;
    do {
      switch (this._state) {
        case GET_INFO:
          this._getInfo();
          break;
        case GET_PAYLOAD_LENGTH_16:
          this._getPayloadLength16();
          break;
        case GET_PAYLOAD_LENGTH_64:
          this._getPayloadLength64();
          break;
        case GET_MASK:
          this._getMask();
          break;
        case GET_DATA:
          this._getData();
          break;
      }
    } while (this._loop);
  }

  _fail(message, closeCode) {
    this._loop = false;
    this._concluded = true;
    throw new WebSocketFrameError(message, closeCode);
  }

  _getInfo() {
    if (this._bufferedBytes < 2) {
      this._loop = false;
      return;
    }
    const buf = this._consume(2);

    if ((buf[0] & 0x70) !== 0x00) this._fail("RSV1, RSV2 and RSV3 must be clear", 1002);

    this._fin = (buf[0] & 0x80) === 0x80;
    this._opcode = buf[0] & 0x0f;
    this._payloadLength = buf[1] & 0x7f;

    if (this._opcode === OPCODE_CONTINUATION) {
      if (!this._fragmented) this._fail("invalid opcode 0", 1002);
      this._opcode = this._fragmented;
    } else if (this._opcode === OPCODE_TEXT || this._opcode === OPCODE_BINARY) {
      if (this._fragmented) this._fail(`invalid opcode ${this._opcode}`, 1002);
    } else if (this._opcode > 0x07 && this._opcode < 0x0b) {
      if (!this._fin) this._fail("FIN must be set on a control frame", 1002);
      if (
        this._payloadLength > 0x7d ||
        (this._opcode === OPCODE_CLOSE && this._payloadLength === 1)
      ) {
        this._fail(`invalid control frame payload length ${this._payloadLength}`, 1002);
      }
    } else {
      this._fail(`invalid opcode ${this._opcode}`, 1002);
    }

    if (!this._fin && !this._fragmented) this._fragmented = this._opcode;
    this._masked = (buf[1] & 0x80) === 0x80;
    if (!this._masked) this._fail("MASK must be set", 1002);

    if (this._payloadLength === 126) this._state = GET_PAYLOAD_LENGTH_16;
    else if (this._payloadLength === 127) this._state = GET_PAYLOAD_LENGTH_64;
    else this._haveLength();
  }

  _getPayloadLength16() {
    if (this._bufferedBytes < 2) {
      this._loop = false;
      return;
    }
    this._payloadLength = readUInt16BE(this._consume(2), 0);
    this._haveLength();
  }

  _getPayloadLength64() {
    if (this._bufferedBytes < 8) {
      this._loop = false;
      return;
    }
    const buf = this._consume(8);
    const hi = readUInt32BE(buf, 0);
    // The maximum safe integer in JS is 2^53 - 1 = (2^21 - 1) * 2^32 + 0xffffffff.
    if (hi > 0x1fffff) this._fail("Unsupported WebSocket frame: payload length > 2^53 - 1", 1009);
    this._payloadLength = hi * 0x100000000 + readUInt32BE(buf, 4);
    this._haveLength();
  }

  _haveLength() {
    if (this._payloadLength && this._opcode < 0x08) {
      this._totalPayloadLength += this._payloadLength;
      if (this._totalPayloadLength > this._maxPayload) this._fail("Max payload size exceeded", 1009);
    }
    this._state = GET_MASK;
  }

  _getMask() {
    if (this._bufferedBytes < 4) {
      this._loop = false;
      return;
    }
    this._mask = this._consume(4);
    this._state = GET_DATA;
  }

  _getData() {
    let data = new Uint8Array(0);
    if (this._payloadLength) {
      if (this._bufferedBytes < this._payloadLength) {
        this._loop = false;
        return;
      }
      data = this._consume(this._payloadLength);
      if ((this._mask[0] | this._mask[1] | this._mask[2] | this._mask[3]) !== 0) {
        maskBytes(data, this._mask);
      }
    }

    if (this._opcode > 0x07) {
      this._controlMessage(data);
      return;
    }

    if (data.length) {
      this._messageLength = this._totalPayloadLength;
      this._fragments.push(data);
    }
    this._dataMessage();
  }

  _dataMessage() {
    if (!this._fin) {
      this._state = GET_INFO;
      return;
    }

    const messageLength = this._messageLength;
    const fragments = this._fragments;
    this._totalPayloadLength = 0;
    this._messageLength = 0;
    this._fragmented = 0;
    this._fragments = [];
    this._state = GET_INFO;

    const buf = concatBytes(fragments.length ? fragments : [new Uint8Array(0)], messageLength);
    if (this._opcode === OPCODE_BINARY) {
      if (this.onMessage) this.onMessage(buf);
    } else {
      if (!isValidUTF8(buf)) this._fail("invalid UTF-8 sequence", 1007);
      if (this.onMessage) this.onMessage(new TextDecoder().decode(buf));
    }
  }

  _controlMessage(data) {
    if (this._opcode === OPCODE_CLOSE) {
      this._loop = false;
      this._concluded = true;
      if (data.length === 0) {
        if (this.onClose) this.onClose(1005, "");
        return;
      }
      const code = readUInt16BE(data, 0);
      if (!isValidStatusCode(code)) this._fail(`invalid status code ${code}`, 1002);
      const reasonBytes = data.subarray(2);
      if (!isValidUTF8(reasonBytes)) this._fail("invalid UTF-8 sequence", 1007);
      if (this.onClose) this.onClose(code, new TextDecoder().decode(reasonBytes));
      return;
    }

    if (this._opcode === OPCODE_PING) {
      if (this.onPing) this.onPing(data);
    } else if (this.onPong) {
      this.onPong(data);
    }
    this._state = GET_INFO;
  }
}

// --- Frame builder -------------------------------------------------------
// Server-to-client frames are never masked (RFC 6455 5.1), which is what
// makes this side of the port dramatically simpler than ws's own sender.js
// (no masking-key generation, no permessage-deflate, no Blob handling).

function buildFrame(opcode, payload, fin) {
  const len = payload.length;
  let offset = 2;
  if (len >= 65536) offset += 8;
  else if (len > 125) offset += 2;

  const frame = new Uint8Array(offset + len);
  frame[0] = (fin === false ? 0 : 0x80) | (opcode & 0x0f);

  if (len >= 65536) {
    frame[1] = 127;
    const hi = Math.floor(len / 0x100000000);
    const lo = len >>> 0;
    frame[2] = (hi >>> 24) & 0xff;
    frame[3] = (hi >>> 16) & 0xff;
    frame[4] = (hi >>> 8) & 0xff;
    frame[5] = hi & 0xff;
    frame[6] = (lo >>> 24) & 0xff;
    frame[7] = (lo >>> 16) & 0xff;
    frame[8] = (lo >>> 8) & 0xff;
    frame[9] = lo & 0xff;
  } else if (len > 125) {
    frame[1] = 126;
    frame[2] = (len >> 8) & 0xff;
    frame[3] = len & 0xff;
  } else {
    frame[1] = len;
  }

  frame.set(payload, offset);
  return frame;
}

function buildCloseFrame(code, reason) {
  const reasonBytes = new TextEncoder().encode(reason || "");
  const payload = new Uint8Array(2 + reasonBytes.length);
  payload[0] = (code >> 8) & 0xff;
  payload[1] = code & 0xff;
  payload.set(reasonBytes, 2);
  return buildFrame(OPCODE_CLOSE, payload, true);
}

// --- HTTP message parsing -------------------------------------------------
// Shared by the WebSocket upgrade handshake and the plain-HTTP router below
// - both start the same way (read raw bytes off the socket until the blank
// line ending the header block), then diverge based on whether the request
// is a WS upgrade.

function findHeaderEnd(bytes) {
  for (let i = 0; i + 3 < bytes.length; i++) {
    if (bytes[i] === 13 && bytes[i + 1] === 10 && bytes[i + 2] === 13 && bytes[i + 3] === 10) {
      return i + 4;
    }
  }
  return -1;
}

function parseRequestLine(line) {
  const parts = line.split(" ");
  return { method: parts[0], path: parts[1] };
}

function parseHttpHeaders(headerText) {
  const lines = headerText.split("\r\n");
  const headers = {};
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line) continue;
    const idx = line.indexOf(":");
    if (idx === -1) continue;
    headers[line.slice(0, idx).trim().toLowerCase()] = line.slice(idx + 1).trim();
  }
  return headers;
}

async function computeAcceptKey(clientKey) {
  if (!globalThis.crypto || !globalThis.crypto.subtle) {
    throw new Error(
      "WebSocketServer requires the webcrypto polyfill (--polyfill webcrypto) to compute the Sec-WebSocket-Accept handshake header",
    );
  }
  const bytes = new TextEncoder().encode(clientKey + WS_GUID);
  const digest = await globalThis.crypto.subtle.digest("SHA-1", bytes);
  return base64Encode(new Uint8Array(digest));
}

function buildUpgradeResponse(acceptKey) {
  const text =
    "HTTP/1.1 101 Switching Protocols\r\n" +
    "Upgrade: websocket\r\n" +
    "Connection: Upgrade\r\n" +
    "Sec-WebSocket-Accept: " +
    acceptKey +
    "\r\n\r\n";
  return new TextEncoder().encode(text);
}

// Reads raw bytes off `recvReadable` (starting from any already-buffered
// `leftover` bytes, e.g. the tail of a previous HTTP/1.1 keep-alive request)
// until the blank line ending an HTTP header block is seen. Returns the
// header bytes and any bytes read past that point (the start of the body,
// or a pipelined next request/WS frame), or `null` if the connection closed
// before a complete header block arrived.
async function readHttpHeaders(recvReadable, leftover) {
  let combined = leftover || new Uint8Array(0);
  for (;;) {
    const end = findHeaderEnd(combined);
    if (end !== -1) {
      return { headerBytes: combined.subarray(0, end), rest: combined.subarray(end) };
    }
    const next = await recvReadable.read(4096);
    if (next.length === 0) return null;
    combined = combined.length ? concatBytes([combined, next], combined.length + next.length) : next;
  }
}

// Reads exactly `contentLength` body bytes, starting from whatever's already
// buffered in `leftover` (bytes read past the header block). Returns the
// body and any bytes left over past it (the start of the next pipelined
// request). A connection closing before the full body arrives yields
// whatever was actually read - callers treat a short body as if the peer
// closed normally, matching this module's existing best-effort error style.
async function readHttpBody(recvReadable, leftover, contentLength) {
  let combined = leftover;
  while (combined.length < contentLength) {
    const next = await recvReadable.read(4096);
    if (next.length === 0) break;
    combined = concatBytes([combined, next], combined.length + next.length);
  }
  const end = Math.min(contentLength, combined.length);
  return { body: combined.subarray(0, end), rest: combined.subarray(end) };
}

const STATUS_TEXTS = {
  200: "OK",
  201: "Created",
  204: "No Content",
  301: "Moved Permanently",
  302: "Found",
  304: "Not Modified",
  400: "Bad Request",
  401: "Unauthorized",
  403: "Forbidden",
  404: "Not Found",
  405: "Method Not Allowed",
  500: "Internal Server Error",
  502: "Bad Gateway",
  503: "Service Unavailable",
};

// Serializes a real `Response` (from the `fetch-classes` polyfill - not
// referenced until this function actually runs, matching the same lazy,
// requires-the-polyfill-or-throws-a-plain-ReferenceError convention already
// used by generate_fetch's `fetch()`) into raw HTTP/1.1 response bytes.
async function writeHttpResponse(sendWritable, response) {
  const bodyBytes = new Uint8Array(await response.arrayBuffer());
  const statusText = response.statusText || STATUS_TEXTS[response.status] || "";

  let head = `HTTP/1.1 ${response.status} ${statusText}\r\n`;
  let hasContentLength = false;
  for (const [name, value] of response.headers.entries()) {
    if (name.toLowerCase() === "content-length") hasContentLength = true;
    head += `${name}: ${value}\r\n`;
  }
  if (!hasContentLength) head += `Content-Length: ${bodyBytes.length}\r\n`;
  head += "\r\n";

  const headBytes = new TextEncoder().encode(head);
  const full = new Uint8Array(headBytes.length + bodyBytes.length);
  full.set(headBytes, 0);
  full.set(bodyBytes, headBytes.length);
  await sendWritable.writeAll(full);
}

// --- Public API -----------------------------------------------------------

class WebSocketConnection {
  constructor(sock, sendWritable, path, headers) {
    this._sock = sock;
    this._sendWritable = sendWritable;
    this._sendChain = Promise.resolve();
    this._closed = false;
    this._handlers = {};
    this.path = path;
    this.headers = headers;
  }

  on(event, cb) {
    (this._handlers[event] || (this._handlers[event] = [])).push(cb);
    return this;
  }

  _emit(event, ...args) {
    for (const cb of this._handlers[event] || []) cb(...args);
  }

  _write(bytes) {
    this._sendChain = this._sendChain.then(() => this._sendWritable.writeAll(bytes));
    return this._sendChain;
  }

  send(data) {
    if (this._closed) throw new Error("WebSocket connection is closed");
    const isBinary = data instanceof Uint8Array || data instanceof ArrayBuffer;
    const payload = isBinary
      ? data instanceof ArrayBuffer
        ? new Uint8Array(data)
        : data
      : new TextEncoder().encode(String(data));
    return this._write(buildFrame(isBinary ? OPCODE_BINARY : OPCODE_TEXT, payload, true));
  }

  ping(data) {
    return this._write(buildFrame(OPCODE_PING, data || new Uint8Array(0), true));
  }

  close(code, reason) {
    if (this._closed) return Promise.resolve();
    this._closed = true;
    return this._write(buildCloseFrame(code || 1000, reason));
  }
}

// Runs the WebSocket session to its conclusion once a connection has been
// upgraded - this consumes the rest of the underlying connection (RFC 6455
// gives the WS framing exclusive ownership of the byte stream from here
// on), so unlike plain HTTP requests there's no looping for a "next
// message" at this level - the FrameParser itself keeps consuming frames
// until closed.
async function runWebSocketSession(sendWritable, recvReadable, initialRest, path, headers, onConnection, maxPayload) {
  const conn = new WebSocketConnection(null, sendWritable, path, headers);
  const parser = new FrameParser(maxPayload);
  parser.onMessage = (data) => conn._emit("message", data);
  parser.onPing = (data) => {
    conn._write(buildFrame(OPCODE_PONG, data, true)).catch(() => {});
    conn._emit("ping", data);
  };
  parser.onPong = (data) => conn._emit("pong", data);
  parser.onClose = (code, reason) => {
    if (!conn._closed) {
      conn._closed = true;
      conn._write(buildCloseFrame(code === 1005 ? 1000 : code, "")).catch(() => {});
    }
    conn._emit("close", code, reason);
  };

  onConnection(conn);

  try {
    if (initialRest.length) parser.write(initialRest);
    while (!parser._concluded) {
      const chunk = await recvReadable.read(4096);
      if (chunk.length === 0) break;
      parser.write(chunk);
    }
  } catch (e) {
    conn._emit("error", e);
  }

  if (!conn._closed) {
    conn._closed = true;
    conn._emit("close", 1006, "");
  }

  try {
    await conn._sendChain;
  } catch (e) {
    // Best-effort: the peer may already be gone.
  }
}

// One accepted connection's whole lifetime: an HTTP/1.1 keep-alive loop
// (serving each non-upgrade request to `onRequest`, if registered) that
// exits either by upgrading to a WebSocket session (which then owns the
// connection until it closes) or by a normal HTTP close - `Connection:
// close`, the peer disconnecting, or (when no `onRequest` is registered,
// preserving this module's original behavior) the first non-upgrade
// request being dropped outright.
async function handleConnection(sock, onConnection, onRequest, maxPayload) {
  const { readable: sendReadable, writable: sendWritable } = wit.Stream(wit.Stream.U8);
  const sendFuture = sock.send(sendReadable);
  const [recvReadable, recvFuture] = sock.receive();

  let rest = new Uint8Array(0);

  try {
    for (;;) {
      let msg;
      try {
        msg = await readHttpHeaders(recvReadable, rest);
      } catch (e) {
        break;
      }
      if (!msg) break;

      const headerText = new TextDecoder().decode(msg.headerBytes);
      const requestLine = headerText.slice(0, headerText.indexOf("\r\n"));
      const { method, path } = parseRequestLine(requestLine);
      const headers = parseHttpHeaders(headerText);
      const clientKey = headers["sec-websocket-key"];
      const isUpgrade = !!clientKey && (headers["upgrade"] || "").toLowerCase() === "websocket";

      if (isUpgrade) {
        const acceptKey = await computeAcceptKey(clientKey);
        await sendWritable.writeAll(buildUpgradeResponse(acceptKey));
        await runWebSocketSession(
          sendWritable,
          recvReadable,
          msg.rest,
          path,
          headers,
          onConnection,
          maxPayload,
        );
        break;
      }

      if (!onRequest) break;

      const contentLength = parseInt(headers["content-length"] || "0", 10) || 0;
      const { body, rest: afterBody } = await readHttpBody(recvReadable, msg.rest, contentLength);
      rest = afterBody;

      const url = "http://" + (headers["host"] || "localhost") + path;
      const request = new Request(url, {
        method: method || "GET",
        headers,
        body: body.length ? body : undefined,
      });

      let response;
      try {
        response = await onRequest(request);
      } catch (e) {
        response = new Response(`Internal Server Error: ${e && e.message ? e.message : e}`, {
          status: 500,
        });
      }
      await writeHttpResponse(sendWritable, response);

      if ((headers["connection"] || "").toLowerCase() === "close") break;
    }
  } catch (e) {
    // Best-effort HTTP/WS server: an unexpected error tears the connection
    // down rather than propagating out of the accept loop.
  }

  sendWritable.drop();

  try {
    await sendFuture.read();
  } catch (e) {
    // ignore
  } finally {
    sendFuture.drop();
  }
  try {
    await recvFuture.read();
  } catch (e) {
    // ignore
  } finally {
    recvFuture.drop();
  }
  sock.drop();
}

function parseIpv4(host) {
  const parts = (host || "0.0.0.0").split(".").map(Number);
  return parts.length === 4 && parts.every((n) => Number.isInteger(n) && n >= 0 && n <= 255)
    ? parts
    : [0, 0, 0, 0];
}

class WebSocketServer {
  constructor(opts = {}) {
    this._maxPayload = opts.maxPayload;
    this._handlers = {};
    this._onRequest = null;
    this._sock = null;
    this._acceptStream = null;
    this.port = null;
  }

  // 'connection'/'error' behave like before (fire every registered
  // listener). 'request' is different on purpose: exactly one handler can
  // meaningfully produce the Response for a given request, so registering a
  // second one replaces the first rather than adding a second listener.
  on(event, cb) {
    if (event === "request") {
      this._onRequest = cb;
      return this;
    }
    (this._handlers[event] || (this._handlers[event] = [])).push(cb);
    return this;
  }

  _emit(event, ...args) {
    for (const cb of this._handlers[event] || []) cb(...args);
  }

  // Binds, listens, and accept-loops forever - meant to be `await`ed from a
  // long-lived entrypoint (e.g. a WASI 0.3 `wasi:cli/run` `run()` kept alive
  // as a background task, see the host's persistent/also-run mechanism for
  // running one alongside normal request handling). Each accepted connection
  // is handled concurrently (its promise isn't awaited before accepting the
  // next one) - ordinary single-threaded JS promise concurrency, no real
  // parallelism needed or assumed.
  async listen(port, host) {
    const sock = TcpSocket.create("ipv4");
    this._sock = sock;
    sock.bind({ tag: "ipv4", val: { port: port || 0, address: parseIpv4(host) } });

    const localAddress = sock.getLocalAddress();
    this.port = localAddress.val.port;

    const acceptStream = sock.listen();
    this._acceptStream = acceptStream;

    for (;;) {
      const accepted = await acceptStream.read(1);
      if (accepted.length === 0) break;
      for (const clientSock of accepted) {
        handleConnection(
          clientSock,
          (conn) => this._emit("connection", conn),
          this._onRequest,
          this._maxPayload,
        ).catch((err) => this._emit("error", err));
      }
    }
  }

  close() {
    if (this._acceptStream) {
      this._acceptStream.drop();
      this._acceptStream = null;
    }
  }
}

globalThis.WebSocketServerP3 = WebSocketServer;
})();
