# Hono on WASI 0.3

A [Hono](https://hono.dev) app served as a WebAssembly component through
`wasi:http/handler`.

**This is not the proof that a real framework works — that already exists.**
Periapsis runs a full Nuxt 4 SSR app as a dwarf component
(`examples/wasm/js-dwarf-nuxt`), plus h3, WebSocket and dual-entry variants.
The request/response bridge below is theirs; this example follows it rather
than inventing another one, and exists to show that bridge at a size you can
read in one sitting, with a second framework, plus the `_initialize`
lifecycle hook.

Almost none of it is glue by nature: Hono's contract is
`app.fetch(Request) -> Promise<Response>` and `wasi:http/handler`'s is
`handle: async func(request) -> result<response, error-code>`. Those are the
same shape, so `app.ts` is a Hono app plus a translation between two
spellings of request and response.

## Build

```bash
npm install                 # or: nub add hono / pnpm i / bun i
dwarf --wit wit --file app.ts --polyfill fetch-classes -o hono.wasm
tsc -p tsconfig.json        # optional: type-check (dwarf never does)
```

The entry is TypeScript and needs no compile step of its own — dwarf strips
the types. Hono ships its own declarations; the wasi:http side is declared
in `wit-http.d.ts`, and `--emit-types` generates that from the world for a
real project.

`--polyfill fetch-classes` supplies `Request`/`Response`/`Headers`, which
Hono needs and dwarf does not provide by default. It is the ONLY polyfill
required: `--polyfill url` costs about 5.5 MB (whatwg-url's IDNA tables) and
Hono does not need it — it parses URLs itself.

## Serve

```bash
trail --p3 --serve --component hono.wasm --listen 127.0.0.1:8080
```

```console
$ curl localhost:8080/
hello from hono, on wasi:http
$ curl localhost:8080/json
{"ok":true,"runtime":"dwarf","framework":"hono"}
$ curl localhost:8080/user/42
{"id":"42"}
$ curl -X POST --data 'ping' localhost:8080/echo
ping
$ curl -i localhost:8080/nope | head -1
HTTP/1.1 404 Not Found
```

`wasmtime serve` runs it too — the component exports the standard interface,
not anything trail-specific.

## Instance lifetime, visibly

`/count` reports a module-level counter and the time `_initialize` ran, which
is the difference between trail's two serve modes:

```console
$ trail --p3 --serve ...                    # fresh instance per request
{"served":1,...}  {"served":1,...}  {"served":1,...}

$ trail --p3 --serve --persistent ...       # one instance for the process
{"served":1,"startedAt":1787584798675}
{"served":2,"startedAt":1787584798675}
{"served":3,"startedAt":1787584798675}
```

`startedAt` staying put under `--persistent` is `_initialize` running once
per INSTANCE. The module's top level runs earlier still — once, at build
time, under Wizer — so anything that cannot be snapshotted (a clock read, a
handle, config from an imported interface) belongs in `_initialize` rather
than at module scope. See "Reactors" in the repository README.

## The two things that are easy to get wrong

Both cost real time here before `js-dwarf-nuxt`'s bridge settled them.

Reading a request body needs `Request.consumeBody(request, transmitted)`,
whose `transmitted` argument is a `future<result<_, error-code>>` — the
VOID-payload future, `wit.Future.RESULT_VOID_WASI_HTTP_TYPES_0_3_0_ERROR_CODE`.
Passing the trailers future (`result<option<trailers>, error-code>`) instead
traps inside the bindings, below any JS `try`/`catch`. The generated constants
for a world are discoverable at run time via `Object.keys(wit.Future.types)`.

A stream ENDS by the writer dropping its end, which surfaces as a rejected
`read()`. The `catch` around the read loop is the normal termination path,
not an error path.

## Size

| | |
|---|---|
| bare handler, no polyfills | 1.6 MB |
| this example (`fetch-classes`) | 1.9 MB |
| ...adding `--polyfill url` | 7.4 MB |

Hono itself costs **219 KB** — the difference between a bare handler and this
app at the same polyfill set (1,775,782 vs 2,000,408 bytes). The framework is
not what makes a component big; `--polyfill url` alone is 25x that, being
whatwg-url's IDNA tables.

Measure differences with ONE variable changed. An earlier pass here compared
builds that differed in flags as well as source and produced "514 bytes",
which is off by three orders of magnitude — the polyfill dominated both
sides and hid the framework entirely.
