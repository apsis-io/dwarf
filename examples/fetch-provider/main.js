// A dwarf-built WASI 0.3 component providing a simple, flattened `fetch`
// function backed by the real `wasi:http/client`, for other components to
// import and compose in via `wac plug` - see README.md in this directory.
//
// Why a separate component rather than a polyfill injected into the caller's
// own world: constructing a wasi:http/client request needs `wit.Future`/
// `wit.Stream` type indices, which are auto-assigned per-component based on
// every other stream/future type that component's own WIT world happens to
// use - not something glue code injected into an arbitrary caller's world
// could reliably reference. Isolating this in its own fixed, minimal world
// (only ever importing wasi:http/client) makes those indices fixed and known,
// and the exported `client.fetch` interface below uses only plain types
// (strings, lists, records) - no resources, streams, or futures cross the
// component boundary, so the caller's own world never needs to deal with any
// of this.

import { Fields, Request, Response } from "wasi:http/types@0.3.0";
import { send } from "wasi:http/client@0.3.0";

function parseUrl(url) {
  const m = /^(https?):\/\/([^/:?#]+)(?::(\d+))?([^?#]*)(\?[^#]*)?/.exec(url);
  if (!m) throw new Error(`invalid URL: ${url}`);
  const [, scheme, host, port, path, query] = m;
  return {
    scheme: scheme.toLowerCase(),
    authority: port ? `${host}:${port}` : host,
    pathWithQuery: (path || "/") + (query || ""),
  };
}

const METHODS = new Set(["get", "head", "post", "put", "delete", "connect", "options", "trace", "patch"]);
function toMethodTag(method) {
  const m = (method || "GET").toLowerCase();
  return METHODS.has(m) ? { tag: m } : { tag: "other", val: method };
}

// wasi:http's request/response constructors take a trailers future the
// caller must construct and resolve itself, even when there are no trailers
// to send. wit.Future's type constants below are specific to this
// component's own (fixed, minimal) world - see the README before copying
// this into a different world.
function resolvedTrailersFuture() {
  const { readable, writable } = wit.Future(wit.Future.RESULT_OPTION_OTHER_ERROR_CODE);
  writable.write({ tag: "ok", val: null });
  return readable;
}

function resolvedVoidFuture() {
  const { readable, writable } = wit.Future(wit.Future.RESULT_VOID_ERROR_CODE);
  writable.write({ tag: "ok", val: undefined });
  return readable;
}

export const client = {
  async fetch(request) {
    const { scheme, authority, pathWithQuery } = parseUrl(request.url);

    const headerEntries = (request.headers || []).map((h) => [h.name, Array.from(new TextEncoder().encode(h.value))]);
    const headers = Fields.fromList(headerEntries);

    const hasBody = request.body && request.body.length > 0;
    let bodyStream = null;
    let bodyWritable = null;
    if (hasBody) {
      const pair = wit.Stream(wit.Stream.U8);
      bodyStream = pair.readable;
      bodyWritable = pair.writable;
    }

    const [wireRequest, transmitFuture] = Request.new(headers, bodyStream, resolvedTrailersFuture(), null);
    wireRequest.setMethod(toMethodTag(request.method));
    wireRequest.setScheme({ tag: scheme === "https" ? "HTTPS" : "HTTP" });
    wireRequest.setAuthority(authority);
    wireRequest.setPathWithQuery(pathWithQuery);

    let response;
    try {
      // send() must be started before the body is written - the writable
      // end has no reader until the request is actually in flight, so
      // awaiting the write first would hang waiting for a reader that
      // never shows up.
      const sendPromise = send(wireRequest);
      if (hasBody) {
        await bodyWritable.writeAll(Array.from(request.body));
        bodyWritable.drop();
      }
      response = await sendPromise;
    } catch (e) {
      throw new Error(`fetch failed: ${e && e.message ? e.message : JSON.stringify(e)}`);
    }

    const status = response.getStatusCode();
    const responseHeaders = response.getHeaders().copyAll().map(([name, value]) => ({
      name,
      value: new TextDecoder().decode(Uint8Array.from(value)),
    }));

    // A single read, not a drain loop: known limitation, bodies larger than
    // this are truncated. A real drain loop needs a proven end-of-stream
    // signal for wasi:io's stream.read, which hasn't been verified yet.
    const [respBodyStream] = Response.consumeBody(response, resolvedVoidFuture());
    const bytes = await respBodyStream.read(65536);
    respBodyStream.drop();
    await transmitFuture.read();
    transmitFuture.drop();

    return { status, headers: responseHeaders, body: Array.from(bytes) };
  },
};
