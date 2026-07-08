# fetch-provider

A dwarf-built WASI 0.3 component providing a simple `client.fetch` function
backed by the real `wasi:http/client`, meant to be **composed into other
components** rather than imported as source.

## Why a separate component, not a polyfill

Constructing a `wasi:http/client` request needs `wit.Future`/`wit.Stream`
type indices (for the request's trailers future and body stream). Those
indices are auto-assigned per-component, based on every other stream/future
type that specific component's WIT world happens to use — not something
generic glue code injected into an arbitrary caller's world could reliably
reference (the same shape of value could get a different index, or even a
different constant name, in a different world).

Isolating this in its own fixed, minimal world (only ever importing
`wasi:http/client`) makes those indices fixed and known. The exported
`client.fetch` interface uses only plain types — strings, lists, records —
no resources, streams, or futures cross the component boundary, so a
consuming component's own world never has to deal with any of this.

## Building

```bash
dwarf --wit . --js main.js -o fetch-provider.wasm --world fetch-provider
```

## Using it from another component

Add the interface as a dependency (`deps/dwarf-fetch/package.wit` — just the
`interface client { ... }` block from `package.wit` here, **without** the
`world fetch-provider { ... }` block, since that references `wasi:http/client`
and would otherwise pull that import requirement into your own world
unnecessarily) and import it:

```wit
world my-app {
    import dwarf:fetch/client;
    export run: async func() -> string;
}
```

```js
import { fetch } from "dwarf:fetch/client";

export async function run() {
    const res = await fetch({
        url: "https://example.com/api",
        method: "GET",
        headers: [{ name: "Accept", value: "application/json" }],
        body: null,
    });
    return new TextDecoder().decode(Uint8Array.from(res.body));
}
```

Build your component, then compose the two together:

```bash
dwarf --wit . --js main.js -o my-app.wasm
wac plug --plug fetch-provider.wasm my-app.wasm -o my-app.composed.wasm
wasmtime run -S http=y --wasm component-model-async=y --invoke 'run()' my-app.composed.wasm
```

`-S http=y` enables wasmtime's outbound WASI HTTP support; `fetch` is an
`async func`, so it must be called from an async export.

## DX layer: a standard `fetch(input, init)`

Calling `dwarf:fetch/client`'s `fetch` directly means building the flattened
request record and unpacking the flattened response yourself. `fetch.js` in
this directory wraps that into the standard signature instead — a URL string
or `Request` in, a real `Response` out:

```js
import { fetch } from "./fetch.js"; // copy fetch.js into your own component

export async function run() {
    const res = await fetch("https://example.com/api", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ hello: "world" }),
    });
    return await res.json();
}
```

Requires the `fetch-classes` polyfill for `Request`/`Response`/`Headers`
(`dwarf --wit . --js main.js -o my-app.wasm --polyfill fetch-classes`), on
top of composing in `fetch-provider.wasm` as above.

## Known limitations

- Response bodies are read with a single `read(65536)` call, not a drain
  loop — bodies larger than 64KB are truncated. A real drain loop needs a
  proven end-of-stream signal for `wasi:io`'s `stream.read`, which hasn't
  been verified yet.
- `fetch-request.url` is parsed with a small regex covering typical
  `http(s)://host[:port]/path?query` shapes, not full WHATWG URL parsing
  (no userinfo, no fragments — neither is meaningful for an outbound
  request line). Pre-validate with the `url` polyfill (`--polyfill url`) if
  you need to accept untrusted URLs.

## Verified

Built and run end-to-end against real local HTTP servers (not mocked): a
plain GET, and a POST with a body (checked with a raw TCP capture that the
request is correctly `Transfer-Encoding: chunked` and reassembled correctly
by a real server), composed via `wac plug`, both returning the expected
status/headers/body — both through the raw `dwarf:fetch/client` interface
and through `fetch.js`'s standard `fetch()`/`Response` wrapper (`.json()`,
`.status`, `.ok`, `.headers.get()` all confirmed working).
