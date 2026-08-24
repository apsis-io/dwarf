# dwarf CLI Cheat Sheet

Quick reference for the `dwarf` command-line tool. See [README.md](../README.md)
for full explanations; this is the fast-lookup version.

## Synopsis

```
dwarf [OPTIONS] --wit <WIT> --file <PATH>
```

## Minimal build

```bash
dwarf --wit hello.wit --file hello.ts -o hello.wasm   # .js works the same
wasmtime run --wasm component-model-async=y --invoke 'greet("World")' hello.wasm
```

## Flags

| Flag | Short | Value | Default | Description |
|---|---|---|---|---|
| `--wit` | `-w` | path | *(required)* | WIT file or directory |
| `--file` | `-f` | path | *(required)* | Entry module, `.ts` or `.js` (`--js`/`-j` still accepted) |
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
| `--optimize` | | path | *(repeatable)* | Compile a TypeScript module statically with scriptc and plug it in; the boundary is DERIVED from its exported signatures. JS imports it as `scriptc:<name>/ops` |
| `--scriptc` | | path | *(repeatable)* | Same, but from a scriptc profile that declares the boundary explicitly instead of deriving it |
| `--scriptc-bin` | | path | `scriptc` on PATH | The scriptc executable `--optimize`/`--scriptc` invoke |

`--opt-size`/`--sync` are mutually exclusive with `--runtime`, combinable with each other.

## Common recipes

```bash
# Auto-detect vendoring, world, everything default
dwarf --wit wit/ --file src/main.ts -o out.wasm

# Multiple worlds in one WIT dir — must disambiguate
dwarf --wit wit/ --file main.ts --world my-world -o out.wasm

# Static polyfills, repeatable
dwarf --wit wit/ --file main.ts --polyfill buffer --polyfill url --polyfill fetch-classes -o out.wasm

# TypeScript types alongside the component (covers WIT world + requested polyfills)
dwarf --wit wit/ --file main.ts --polyfill buffer --emit-types types/ -o out.wasm

# Smallest possible component (size-optimized runtime + minified JS)
dwarf --wit wit/ --file main.ts --opt-size --minify -o out.wasm

# No component-model-async (older/plain wasmtime hosts)
dwarf --wit wit/ --file main.ts --sync -o out.wasm

# Compile a hot TypeScript module with scriptc and plug it in (no engine in
# it; JS imports it as `scriptc:hot/ops`). The seam does not survive into
# the output component.
dwarf --wit wit/ --file main.ts --optimize src/hot.ts -o out.wasm

# ...from a profile that declares the boundary explicitly, and with a
# scriptc that is not the one on PATH
dwarf --wit wit/ --file main.ts --scriptc src/hot.profile.json \
  --scriptc-bin ./node_modules/.bin/scriptc -o out.wasm

# Sandbox: no real host capabilities at all
dwarf --wit wit/ --file main.ts --stub-wasi -o out.wasm

# Standalone single WIT file (no deps/ dir, vendoring doesn't apply)
dwarf --wit hello.wit --file hello.ts -o out.wasm

# Edit a polyfill's .js/.d.ts on disk with zero rebuilds (dev only)
DWARF_POLYFILLS_DIR=/path/to/dwarf/crates/core/polyfills \
  dwarf --wit wit/ --file main.ts --polyfill buffer -o out.wasm
```

## TypeScript

`--file app.ts` just works — types are stripped in-process (oxc), no `tsc`
or `esbuild` needed, and imported `.ts` modules are stripped too.

| | |
|---|---|
| Entry | `.ts`, `.mts`, `.cts`, or any `.js` flavour |
| Imports | `./x.js` resolves `x.ts` (TypeScript's NodeNext convention), `./x` works too, and `.js`↔`.ts` may be mixed |
| Emitting syntax | `enum`, parameter properties and decorators are compiled, not erased |
| Type checking | **Never** — same contract as Node/esbuild. Run `tsc --noEmit` yourself |
| Syntax errors | Fail the build, naming the file |
| `.d.ts` input | Rejected: it declares types and emits nothing |
| World types | `--emit-types <dir>` generates declarations from the WIT |

```bash
dwarf --wit wit/ --file src/app.ts --emit-types types/ -o app.wasm
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
| `console.log/info/debug/warn/error` | `wasi:cli/stdout`/`stderr@0.3.x` (`write-via-stream`, Promise-returning); at build time, the build's own stderr |
| `console.print/println/eprint/eprintln` | Same interfaces, always async (Promise-returning) |
| `process.env/argv/cwd()/exit()` | `wasi:cli/environment`/`exit` (same shape in 0.2 and 0.3) |
| `crypto.getRandomValues` | `wasi:random/random#get-random-bytes` |
| `setTimeout`/`setInterval` | `wasi:clocks/monotonic-clock@0.3.x#wait-for` (0.2 has no non-blocking wait, so only 0.3 works) |
| `clearTimeout`/`clearInterval` | Always safe no-ops, even without the clock import |
| `AbortController`/`AbortSignal` | Hand-written, no WIT dependency — real `abort()`/`aborted`/`reason`/listeners |
| `fetch()` | `wasi:http/client@0.3.x` — requires `--polyfill fetch-classes` too (for `Request`/`Response`/`Headers`) |
| `WebSocketServer` | `wasi:sockets/types@0.3.0`'s `tcp-socket` — requires `--polyfill webcrypto` too (for the `Sec-WebSocket-Accept` handshake header). `on("request", ...)` also doubles it as a single-port HTTP+WS router (additionally requires `--polyfill fetch-classes`) |

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

Every global in the table above also has a `...P3` name (`consoleP3`,
`processP3`, `crypto.getRandomValuesP3`, `setTimeoutP3`/etc., `fetchP3`,
`WebSocketServerP3`) - the same object/function, additive, not a
replacement. See README's [Version-pinned `...P3`
names](../README.md#version-pinned-p3-names) section for why.

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
| `SOURCE_DATE_EPOCH` | The wall clock the guest sees while being snapshotted (seconds since the Unix epoch; default 0). The one deliberate input to an otherwise byte-reproducible build — see [Reproducible builds](#reproducible-builds) |

## Cargo features (building dwarf itself)

| Feature | Effect |
|---|---|
| `component-model-async` *(default)* | Embed the async runtime as default; disable for a smaller binary with only the non-async runtime |
| `opt-size` | Selects the bundled opt-size runtime as default when no runtime flag is given |

```bash
cargo build --release --features opt-size
```

## Reproducible builds

Same inputs, same bytes. Wizer freezes the initialized guest heap into the
artifact, so dwarf pins what the guest can observe while being snapshotted:
the wall clock (QuickJS seeds `Math.random`'s state from it), `wasi:random`,
and the module root. Only the snapshot sees any of this — the component you
ship reads the host's real clock and randomness.

```bash
dwarf --wit hello.wit --file hello.ts -o a.wasm
dwarf --wit hello.wit --file hello.ts -o b.wasm
cmp a.wasm b.wasm                      # identical, from any directory

SOURCE_DATE_EPOCH=1700000000 dwarf ... # the one knob; same value, same bytes
```

## Reactor lifecycle

Every build is a reactor: instantiate once, call exports repeatedly. (A
*command* is the other shape — a component exporting `wasi:cli/run`, whose
world needs `export function run() { ... }`.)

| | |
|---|---|
| JS module top level | runs at **build** time under Wizer, snapshotted |
| `console.log` in it | printed by the *build*, prefixed `  [js stdout]` on dwarf's stderr |
| a fresh instance | starts from that snapshot, never from another instance |
| state between calls | persists for the life of one instance |
| `export function _initialize()` | runs **once per instance**, before the first exported call — the place for setup that cannot be snapshotted |
| teardown | none; the component model gives a guest no destructor. Declare `export shutdown: func();` in your world and have the host call it |

## Gotchas

- **`--module-root` defaults to the ENTRY'S OWN DIRECTORY**, not the current
  working directory — otherwise the build would depend on where it was run
  from (see [Reproducible builds](#reproducible-builds)). An entry that
  imports from *above* its own directory must name the wider root with the
  flag; without it those imports fail as "outside the module root".
- **Vendoring** only applies when `--wit` is a *directory* (needs a `deps/` to
  populate). A single WIT file with missing deps is always an error.
- **`console.log` needs an async task at *runtime***, since WASI 0.3 has no
  synchronous write: called from a plain sync export it throws a catchable
  error rather than writing. At *build* time (module top level) there is no
  task and none is possible, so it goes out through the build host's stderr
  instead — that path is build-only and is stubbed out of the component.
- **Dynamic `import()`**: only works if reached during Wizer's build-time
  module evaluation (top-level code). One reached later, at real runtime
  (e.g. lazily inside a request handler), throws a catchable error naming the
  module — not a crash, but also not going to work; configure your bundler to
  inline dynamic imports instead (e.g. Rollup/Vite's
  `output.inlineDynamicImports: true`).
- **`fetch()` itself** isn't a polyfill — WASI 0.3 resource/stream/future type
  indices are per-component, so it has to be a separate composed component.
  See [`examples/fetch-provider`](../examples/fetch-provider).
