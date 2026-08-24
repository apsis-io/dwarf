// A Hono app served as a WASI 0.3 HTTP component.
//
// Hono's own contract is `app.fetch(Request) -> Promise<Response>`, and
// `wasi:http/handler`'s is `handle: async func(request) -> result<response>`.
// They are the same shape, so this file is almost entirely the ADAPTER
// between the two type worlds: wasi:http resources on one side, the
// fetch-classes polyfill's Request/Response on the other.
import { Hono } from "hono";
import {
  Fields,
  Request as WasiRequest,
  Response as WasiResponse,
} from "wasi:http/types";

const app = new Hono();

app.get("/", (c) => c.text("hello from hono, on wasi:http\n"));
app.get("/json", (c) => c.json({ ok: true, runtime: "dwarf", framework: "hono" }));
app.get("/user/:id", (c) => c.json({ id: c.req.param("id") }));
app.post("/echo", async (c) => c.text(await c.req.text()));

// Module-level state: with `trail --serve --persistent` this survives across
// requests (one Store for the process); without it every request gets a
// fresh instance from the Wizer snapshot and the count restarts at 1.
let served = 0;
let startedAt = null;

app.get("/count", (c) => c.json({ served: ++served, startedAt }));

/// Per-instance setup. The module top level above runs ONCE, at build time,
/// under Wizer — its results are snapshotted into every instance. This runs
/// once per INSTANCE instead, before the first request, which is where
/// anything unsnapshottable belongs: a clock read, a handle, configuration
/// pulled through an imported interface.
///
/// With `--persistent` there is one instance for the process, so `startedAt`
/// is fixed and `served` climbs. Without it every request gets a fresh
/// instance and both reset.
export function _initialize() {
  startedAt = Date.now();
}

const decoder = new TextDecoder();
const encoder = new TextEncoder();

/** wasi:http scheme variant -> the string a URL needs. */
function schemeOf(request) {
  const scheme = request.getScheme();
  if (!scheme) return "http";
  if (scheme.tag === "HTTP") return "http";
  if (scheme.tag === "HTTPS") return "https";
  return String(scheme.val ?? "http").toLowerCase();
}

/** Drain a body stream to a Uint8Array.
 *
 * A zero-length read is end-of-stream; the handles are dropped afterwards
 * and the "request transmitted" future is awaited, which is the lifecycle
 * periapsis's js-dwarf-nuxt bridge established. Reading in 64 KiB bites
 * keeps a large upload from costing a host call per byte.
 */
async function drainBody(stream) {
  const chunks = [];
  let total = 0;
  for (;;) {
    const chunk = await stream.read(65536);
    if (chunk.length === 0) break;
    chunks.push(chunk);
    total += chunk.length;
  }
  const out = new Uint8Array(total);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}

/** The trailers future a response needs: `result<option<trailers>, error-code>`. */
function resolvedTrailersFuture() {
  const { readable, writable } = wit.Future(wit.Future.RESULT_OPTION_OTHER_ERROR_CODE);
  writable.write({ tag: "ok", val: null });
  return readable;
}

/** The "request transmitted" future `consumeBody` takes: `result<_, error-code>`.
 *
 * The VOID-payload one. Passing the trailers future instead traps inside the
 * bindings, below any JS try/catch — the constants for a world are listable
 * at run time with `Object.keys(wit.Future.types)`.
 */
function resolvedVoidFuture() {
  const { readable, writable } = wit.Future(
    wit.Future.RESULT_VOID_WASI_HTTP_TYPES_0_3_0_ERROR_CODE,
  );
  writable.write({ tag: "ok", val: undefined });
  return readable;
}

export const handler = {
  async handle(request) {
    const method = (request.getMethod().tag ?? "get").toUpperCase();
    const pathWithQuery = request.getPathWithQuery() ?? "/";
    const authority = request.getAuthority() ?? "localhost";
    const url = `${schemeOf(request)}://${authority}${pathWithQuery}`;

    const headers = new Headers();
    for (const [name, value] of request.getHeaders().copyAll()) {
      headers.append(name, decoder.decode(Uint8Array.from(value)));
    }

    let body;
    if (method !== "GET" && method !== "HEAD") {
      const [bodyStream, bodyDone] = WasiRequest.consumeBody(
        request,
        resolvedVoidFuture(),
      );
      body = await drainBody(bodyStream);
      bodyStream.drop();
      await bodyDone.read();
      bodyDone.drop();
    }

    const res = await app.fetch(
      new Request(url, { method, headers, ...(body?.length ? { body } : {}) }),
    );

    const outBytes = new Uint8Array(await res.arrayBuffer());
    const outHeaders = new Fields();
    res.headers.forEach((value, name) => {
      outHeaders.append(name, Array.from(encoder.encode(value)));
    });

    const { readable, writable } = wit.Stream(wit.Stream.U8);
    const [response] = WasiResponse.new(
      outHeaders,
      readable,
      resolvedTrailersFuture(),
    );
    response.setStatusCode(res.status);

    // Write after the response exists: the readable end has no consumer
    // until the host takes the response, so writing first would block on a
    // reader that is not there yet. Deliberately not awaited.
    writable.writeAll(Array.from(outBytes)).then(() => writable.drop());

    return response;
  },
};
