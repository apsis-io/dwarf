# dwarf CLI Cheat Sheet

Quick reference for the `dwarf` command-line tool. See [README.md](../README.md)
for full explanations; this is the fast-lookup version.

## Synopsis

```
dwarf [OPTIONS] --wit <WIT> --js <JS>
```

## Minimal build

```bash
dwarf --wit hello.wit --js hello.js -o hello.wasm
wasmtime run --wasm component-model-async=y --invoke 'greet("World")' hello.wasm
```

## Flags

| Flag | Short | Value | Default | Description |
|---|---|---|---|---|
| `--wit` | `-w` | path | *(required)* | WIT file or directory |
| `--js` | `-j` | path | *(required)* | JS entry module |
| `--output` | `-o` | path | `output.wasm` | Output component path |
| `--world` | `-n` | name | (auto-detect) | World name, if the WIT defines more than one |
| `--module-root` | | path | entry's dir | Root exposed read-only during Wizer for resolving JS `import`s (relative/bare specifiers, `node_modules`) |
| `--no-vendor` | | | off | Disable auto-fetching missing WIT deps via `wkg wit fetch` (dir `--wit` only) |
| `--stub-wasi` | | | off | Replace all WASI imports with trap stubs (no host capabilities) |
| `--polyfill` | | name | *(repeatable)* | Include a static polyfill — see [Polyfills](#polyfills) |
| `--emit-types` | | dir | | Also emit `.d.ts` for the WIT world + polyfills via `jco types` |
| `--minify` | `-m` | | off | Minify JS via oxc before embedding |
| `--disable-gc` | | | off | Disable QuickJS auto-GC |
| `--opt-size` | | | off | Embed the size-optimized built-in runtime |
| `--sync` | | | off | Embed the non-async built-in runtime (no component-model-async) |
| `--runtime` | | path | | Custom QuickJS runtime `.wasm` (overrides `--opt-size`/`--sync`) |

`--opt-size`/`--sync` are mutually exclusive with `--runtime`, combinable with each other.

## Common recipes

```bash
# Auto-detect vendoring, world, everything default
dwarf --wit wit/ --js src/main.js -o out.wasm

# Multiple worlds in one WIT dir — must disambiguate
dwarf --wit wit/ --js main.js --world my-world -o out.wasm

# Static polyfills, repeatable
dwarf --wit wit/ --js main.js --polyfill buffer --polyfill url --polyfill fetch-classes -o out.wasm

# TypeScript types alongside the component (covers WIT world + requested polyfills)
dwarf --wit wit/ --js main.js --polyfill buffer --emit-types types/ -o out.wasm

# Smallest possible component (size-optimized runtime + minified JS)
dwarf --wit wit/ --js main.js --opt-size --minify -o out.wasm

# No component-model-async (older/plain wasmtime hosts)
dwarf --wit wit/ --js main.js --sync -o out.wasm

# Sandbox: no real host capabilities at all
dwarf --wit wit/ --js main.js --stub-wasi -o out.wasm

# Standalone single WIT file (no deps/ dir, vendoring doesn't apply)
dwarf --wit hello.wit --js hello.js -o out.wasm

# Edit a polyfill's .js/.d.ts on disk with zero rebuilds (dev only)
DWARF_POLYFILLS_DIR=/path/to/dwarf/crates/core/polyfills \
  dwarf --wit wit/ --js main.js --polyfill buffer -o out.wasm
```

## Polyfills (`--polyfill <name>`)

| Name | Provides |
|---|---|
| `buffer` | `Buffer` (feross/buffer) |
| `url` | `URL`, `URLSearchParams` (whatwg-url, IDNA-compliant) |
| `fetch-classes` | `Headers`, `Request`, `Response`, `DOMException`, plus a real `fetch()` wired to `wasi:http/client@0.3.x` (always-on, throws a clear error if that import is missing) |
| `path` | `path` module (join/dirname/basename/etc., matches Node's shape) |
| `readable-stream` | `ReadableStream`, `wit.readableStreamFromStream(readable)` |
| `webcrypto` | `crypto.subtle` (digest, HMAC, ECDSA/ECDH P-256/P-384, HKDF, AES-GCM — @noble/hashes+curves+ciphers). A subset, not full spec parity — see webcrypto.d.ts. `crypto.getRandomValues` is always-on (below), independent of this flag |
| `ufo` | `ufo.*` namespace — functional URL utilities (joinURL, withQuery, parseURL, etc.), complements `url`'s class-based API |
| `scule` | `scule.*` namespace — string case conversion (camelCase, kebabCase, snakeCase, pascalCase, etc.) |
| `klona` | `klona(value)` — fast deep clone. dwarf has no `structuredClone` at all |
| `ohash` | `ohash.*` namespace — `hash()`/`serialize()`/`isEqual()`, non-cryptographic (see `webcrypto` for real hashing) |
| `knitwork` | `knitwork.*` namespace — JS/TS code-string generation (genImport, genObjectFromRaw, etc.), no parsing |
| `unstorage` | `unstorage.*` namespace — universal KV storage API (`createStorage()`), zero-config in-memory only — no fs/redis/kv drivers bundled |

Unknown name → build error listing valid names. Full details, caveats, and
attributions: README's [Polyfills](../README.md#polyfills) section and
[NOTICES](../NOTICES).

## Always-on globals (no flag needed)

| Global | Backed by |
|---|---|
| `TextEncoder` / `TextDecoder` | Hand-written, always present |
| `console.log/info/debug/warn/error` | `wasi:cli/stdout`/`stderr@0.3.x` (`write-via-stream`, Promise-returning) |
| `console.print/println/eprint/eprintln` | Same interfaces, always async (Promise-returning) |
| `process.env/argv/cwd()/exit()` | `wasi:cli/environment`/`exit` (same shape in 0.2 and 0.3) |
| `crypto.getRandomValues` | `wasi:random/random#get-random-bytes` |
| `setTimeout`/`setInterval` | `wasi:clocks/monotonic-clock@0.3.x#wait-for` (0.2 has no non-blocking wait, so only 0.3 works) |
| `clearTimeout`/`clearInterval` | Always safe no-ops, even without the clock import |
| `AbortController`/`AbortSignal` | Hand-written, no WIT dependency — real `abort()`/`aborted`/`reason`/listeners |
| `fetch()` | `wasi:http/client@0.3.x` — requires `--polyfill fetch-classes` too (for `Request`/`Response`/`Headers`) |
| `WebSocketServer` | `wasi:sockets/types@0.3.0`'s `tcp-socket` — requires `--polyfill webcrypto` too (for the `Sec-WebSocket-Accept` handshake header) |

`console`/`process`/`crypto.getRandomValues`/`setTimeout`/`setInterval`/`fetch()`/`WebSocketServer`
throw a clear error naming the missing import if the world doesn't provide it —
see README's [Console](../README.md#console) and [Process](../README.md#process)
sections for the full fallback rules and the async-logging completion-ordering
caveat. `setTimeout`/`setInterval` have an unavoidable caveat under
component-model-async: an unawaited timer's callback is cancelled if the
async export that (transitively) created it settles first — reliable only
when awaited or called from a still-running export. `WebSocketServer.listen()`
has the same "must be awaited from a still-running export" shape, since it
accept-loops forever — see README's [WebSockets](../README.md#websockets)
section for the full API and scope cuts (IPv4 only, no permessage-deflate).

## WIT → JS type mapping (condensed)

| WIT | JS |
|---|---|
| `bool` | `boolean` |
| `u8`..`u64`, `s8`..`s64`, `f32`/`f64` | `number` (u64/s64 capped at 2⁵³) |
| `char`, `string` | `string` |
| `list<T>` | `Array` (`list<u8>` → `Uint8Array`) |
| `tuple<...>` | `Array` |
| `option<T>` | `T \| null` (nested: `{tag:"some"\|"none", val}`) |
| `result<T,E>` (top-level fn return) | return `T` / throw `E` |
| `result<T,E>` (nested) | `{tag:"ok"\|"err", val}` |
| `record` | object, camelCase keys |
| `variant` | `{tag, val?}` |
| `enum` | string (case name) |
| `flags` | object of camelCase booleans |
| `own<R>`/`borrow<R>` | resource class instance |

Full details incl. imported resources, async exports, streams/futures:
README's [WIT Type Mappings](../README.md#wit-type-mappings) and
[docs/runtime-intrinsics.md](runtime-intrinsics.md).

## Environment variables

| Var | Effect |
|---|---|
| `DWARF_POLYFILLS_DIR` | Read polyfill `.js`/`.d.ts` fresh from disk instead of the compiled-in copy — live edits, no rebuild. Dev only; unset = self-contained binary. |
| `WASM_OPT` | Path to a `wasm-opt` binary, checked before PATH/auto-download (build-time, not runtime) |

## Cargo features (building dwarf itself)

| Feature | Effect |
|---|---|
| `component-model-async` *(default)* | Embed the async runtime as default; disable for a smaller binary with only the non-async runtime |
| `opt-size` | Selects the bundled opt-size runtime as default when no runtime flag is given |

```bash
cargo build --release --features opt-size
```

## Gotchas

- **Vendoring** only applies when `--wit` is a *directory* (needs a `deps/` to
  populate). A single WIT file with missing deps is always an error.
- **Dynamic `import()`**: only works if reached during Wizer's build-time
  module evaluation (top-level code). One reached later, at real runtime
  (e.g. lazily inside a request handler), throws a catchable error naming the
  module — not a crash, but also not going to work; configure your bundler to
  inline dynamic imports instead (e.g. Rollup/Vite's
  `output.inlineDynamicImports: true`).
- **`fetch()` itself** isn't a polyfill — WASI 0.3 resource/stream/future type
  indices are per-component, so it has to be a separate composed component.
  See [`examples/fetch-provider`](../examples/fetch-provider).
