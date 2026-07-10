# dwarf

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

Convert JavaScript source code into
[WebAssembly components](https://component-model.bytecodealliance.org/) —
including genuine **WASI 0.3 (Preview 3)** components, async exports and all —
using [QuickJS](https://github.com/quickjs-ng/quickjs). QuickJS-sized (not a
~13 MB SpiderMonkey embedding) with full typed WIT bindings for arbitrary
worlds (not just standard WASI).

`dwarf` is a fork of
[componentize-qjs](https://github.com/andreiltd/componentize-qjs)
(Apache-2.0), rebranded and maintained here for the
[Periapsis](https://github.com/apsis-io/periapsis) project family. See
[NOTICES](NOTICES) for full upstream attribution and `git log` for
commit-level history predating the fork. Local fork only for now — no
published crate/npm package yet.

## Overview

`dwarf` takes a JavaScript source file and a
[WIT](https://component-model.bytecodealliance.org/design/wit.html) definition,
and produces a standalone WebAssembly component that can run on any
component-model runtime (e.g. [Wasmtime](https://wasmtime.dev/)).

Under the hood it:

1. Embeds the [QuickJS](https://github.com/quickjs-ng/quickjs) engine (via
   [rquickjs](https://github.com/DelSkayn/rquickjs)) as the JavaScript
   runtime.
2. Uses [wit-dylib](https://crates.io/crates/wit-dylib) to generate WIT
   bindings that bridge the component model and the JS engine.
3. Snapshots the initialized JS state with
   [Wizer](https://github.com/bytecodealliance/wizer) so startup cost is paid at
   build time, not at runtime.

## Prerequisites

Rust **1.94** or later is required (the `wasm32-wasip2` target needs a recent
toolchain for PIC support in wasi-libc).

Two optional external tools enable extra features, checked for on `PATH` at
runtime — neither is required to build or run `dwarf` itself:

```bash
# wkg - enables automatic fetching of missing WIT dependencies (e.g. wasi:*
# imports) into deps/ instead of vendoring them by hand
cargo install wkg

# jco - enables `dwarf --emit-types <dir>`, generating TypeScript declarations
# for a WIT world. Note: the unscoped `jco` package on npm is an unrelated
# placeholder - the real package is scoped.
npm install -g @bytecodealliance/jco
```

## Installation

Not published to crates.io or npm — this is a local-only fork. Build from
source:

### Rust CLI (from source)

```bash
cargo install --path . --locked
```

This installs the `dwarf` command.

### npm package (from source)

```bash
cd npm && npm install && npm run build
```

## Quick Start

**1. Define a WIT interface** (`hello.wit`):

```wit
package test:hello;

world hello {
    export greet: func(name: string) -> string;
}
```

**2. Implement it in JavaScript** (`hello.js`):

JavaScript sources are ES modules. Export WIT functions and interfaces directly
from the module.

```js
export function greet(name) {
    return `Hello, ${name}!`;
}
```

**3. Build the component:**

```bash
dwarf --wit hello.wit --js hello.js -o hello.wasm
```

**4. Run it:**

```bash
wasmtime run --wasm component-model-async=y --invoke 'greet("World")' hello.wasm
# "Hello, World!"
```

The built-in runtime published with dwarf includes component-model
async support. Pass `--sync` to embed the built-in non-async runtime instead,
producing components that run on hosts without component-model async support. A
custom runtime can also be supplied with `--runtime`.

## CLI Reference

See also: [docs/cli-cheatsheet.md](docs/cli-cheatsheet.md) for a dense,
single-page quick reference (flags, recipes, polyfills, type mappings) without
the surrounding prose.

```
dwarf [OPTIONS] --wit <WIT> --js <JS>
```

| Flag | Short | Description |
|---|---|---|
| `--wit <PATH>` | `-w` | Path to the WIT file or directory |
| `--js <PATH>` | `-j` | Path to the JavaScript source file |
| `--output <PATH>` | `-o` | Output path (default: `output.wasm`) |
| `--module-root <PATH>` | | Root directory exposed read-only during Wizer for resolving JavaScript imports |
| `--world <NAME>` | `-n` | World name when the WIT defines multiple worlds |
| `--stub-wasi` | | Replace all WASI imports with trap stubs |
| `--no-vendor` | | Disable automatically fetching missing WIT dependencies via `wkg wit fetch` |
| `--emit-types <DIR>` | | Also generate TypeScript type declarations for the WIT world via `jco types` |
| `--polyfill <NAME>` | | Include a static polyfill (repeatable), e.g. `--polyfill buffer` — see [Polyfills](#polyfills) |
| `--minify` | `-m` | Minify JS source before embedding |
| `--disable-gc` | | Disable automatic garbage collection in the QuickJS runtime |
| `--opt-size` | | Use the built-in QuickJS runtime optimized for size |
| `--sync` | | Use the built-in non-async runtime (combine with `--opt-size` for the non-async opt-size runtime) |
| `--runtime <PATH>` | | Custom QuickJS runtime Wasm module to embed |

### Cargo features

| Feature | Effect |
|---|---|
| `component-model-async` | (default) Embed the component-model async runtime as the default built-in. The non-async runtime is always embedded and selectable via `--sync`. Disable to build a smaller binary with only the non-async runtime |
| `opt-size` | Selects the bundled opt-size runtime when no runtime option is provided by the CLI or npm API |

Build with features:

```bash
cargo build --release --features opt-size
```

### TypeScript Types

`--emit-types <DIR>` generates `.d.ts` declarations for the WIT world via
[`jco types`](https://github.com/bytecodealliance/jco) (must be on `PATH`,
see [Prerequisites](#prerequisites)). jco's output assumes its own
(componentize-js-oriented) JS binding conventions, which diverge from dwarf's
actual runtime in two confirmed ways — `u64`/`s64` typed `bigint` instead of
`number`, and `option<T>` typed `T | undefined`/an omittable `field?: T`
instead of dwarf's actual `T | null` (dwarf always includes the
property/value, using `null` for "none," never `undefined` and never omitting
it). dwarf patches jco's generated files to match its own conventions before
writing them out. This is a best-effort textual patch, not a from-scratch
type generator — deeply nested option shapes (e.g. `option<option<T>>`,
which dwarf represents with a tagged `{ tag, val }` form rather than plain
`null`) aren't specifically handled and may still read as jco's own
convention.

## Using Imports

WIT imports are available as ES module imports using their fully-qualified WIT
interface name:

```wit
// imports.wit
package local:test;

interface math {
    add: func(a: s32, b: s32) -> s32;
    multiply: func(a: s32, b: s32) -> s32;
}

world imports {
    import math;
    export double-add: func(a: s32, b: s32) -> s32;
}
```

```js
// imports.js
import math from "local:test/math";

export function doubleAdd(a, b) {
    const sum = math.add(a, b);
    return math.multiply(sum, 2);
}
```

JavaScript modules imported by the entry file are resolved during Wizer
initialization. Relative imports are resolved from the entry file path passed to
`--js`; bare package imports are resolved under the read-only module root. By
default the CLI uses the current directory when the entry file is under it, or
the entry file's parent directory otherwise. Use `--module-root <PATH>` to expose
a project root that contains shared files or `node_modules`.

**Static `import` only — dynamic `import()` doesn't work at runtime.**
Resolving a module needs real filesystem access, which only exists during
Wizer's build-time pre-init (where module resolution runs); the actual
runtime a component executes in later has no such capability. Every
`import`/`import()` your code can reach must be resolvable at build time —
which in practice means only ever calling `import()` from top-level module
code (so it runs during Wizer's own pre-init, not deferred until a real
request/export call). A bundled third-party library that does its *own*
dynamic `import()` lazily — e.g. only the first time a particular code path
actually runs, rather than at module load — throws a normal, catchable
`Error` the moment that path is reached at real runtime, naming the module
it tried to load. If a bundled dependency does this, configure your
bundler to inline dynamic imports rather than emit runtime `import()`
calls — e.g. Rollup/Vite's
`build.rollupOptions.output.inlineDynamicImports: true`.

### Composable Capability Components

Some WASI capabilities (`wasi:http/client` in particular) need constructing
resources — headers, request bodies as `stream<u8>`, trailers as `future<T>`
— whose `wit.Stream`/`wit.Future` type indices are auto-assigned per
component based on everything else in that component's own WIT world. That
makes them a poor fit for glue code meant to drop into an arbitrary caller's
world. The better pattern: build the capability as its own small, focused
component (a fixed world, so its indices are fixed and known) that exports a
plain-types-only interface — strings, lists, records, no resources/streams/
futures crossing the boundary — and compose it into your own component with
[`wac plug`](https://github.com/bytecodealliance/wac). See
[`examples/fetch-provider`](examples/fetch-provider) for a complete,
verified-against-real-HTTP-servers example of this pattern.

### Vendoring WIT Dependencies

When `--wit` points at a directory, referenced packages (e.g. `wasi:cli`)
are normally vendored under a `deps/` subdirectory next to your WIT files. If
a package is missing, dwarf automatically runs `wkg wit fetch` to fetch it
(requires [`wkg`](#prerequisites) on `PATH`) and retries — no manual `deps/`
setup needed for a quick start. Disable this with `--no-vendor`; auto-vendoring
only applies to directory `--wit` paths, since a single standalone WIT file has
no `deps/` directory to populate.

## Console

`console.log`/`info`/`debug` and `console.warn`/`error` are available when the
world imports either version of `wasi:cli/stdout`/`stderr`, in priority order:

1. **WASI 0.2** (matched by `get-stdout`/`get-stderr`) — genuinely synchronous,
   guaranteed complete by the time the call returns, exactly like real
   `console.log`. Prefer this when your world can have it.
2. **WASI 0.3** (matched by `write-via-stream`) otherwise — WASI 0.3 has no
   synchronous write primitive, so this path makes `log`/`info`/`debug`/
   `warn`/`error` return a `Promise<void>` instead of `undefined`. This
   matters: a p3 world that `include`s `wasi:cli/command@0.3.0` **cannot**
   also import `wasi:cli/stdout@0.2.x`/`stderr@0.2.x` (see the note below),
   so any p3-only world lands on this path.

```wit
world hello {
    import wasi:cli/stdout@0.2.12;
    import wasi:cli/stderr@0.2.12;
    export greet: func(name: string) -> string;
}
```

```js
export function greet(name) {
    console.log("greeting", name);
    return `Hello, ${name}!`;
}
```

`console` always exists — if neither import is declared, calling the
relevant method throws a clear error naming both options, rather than
leaving `console` undefined or silently doing nothing.

**In a 0.3-only world, unawaited calls can silently lose output** if nothing
else in the same async export subsequently awaits or yields — confirmed
empirically: two unawaited `console.log` calls followed by an `await` on a
third all flushed correctly, but an unawaited `console.error` as the last
statement before an export returns produced no output at all. Await
`console.log`/`error`/etc. (they always return a promise-or-undefined; awaiting
`undefined` is a no-op) whenever you can't otherwise guarantee the surrounding
async export keeps running long enough to flush the write. This is *not* a
concern in a WASI-0.2-backed world, where these calls are always synchronous
and complete before returning.

> **WIT-level constraint:** `wasi:cli/command@0.3.0` and `wasi:cli/stdout@0.2.x`/
> `stderr@0.2.x` cannot be vendored together in the same world — `wkg wit fetch`
> fails to resolve `wasi:cli@0.3.0`'s own transitive deps once a 0.2.x import of
> the same package is also present. This isn't a dwarf limitation to work
> around; it's why the 0.3 fallback above exists at all.

Non-string arguments are formatted with `JSON.stringify` — a compact
single-line dump (`{"foo":"bar"}`, `[1,2,3]`), not Node's multi-line
`util.inspect`-style output. Two quirks worth knowing: `console.log(undefined)`
prints a blank line (`JSON.stringify(undefined)` is the JS value `undefined`,
not a string, and `Array.join` coerces it to `''`), and a circular-reference
object throws synchronously at the call site rather than being silently
swallowed.

### Async logging: `print`/`println`/`eprint`/`eprintln`

`console.print`/`println` (stdout) and `console.eprint`/`eprintln` (stderr)
always exist and always return a `Promise<void>` that rejects rather than
throwing synchronously — `print`/`eprint` write with no trailing newline,
`println`/`eprintln` append one. They use, in priority order:

1. **WASI 0.3** `wasi:cli/stdout`/`stderr` (matched by `write-via-stream`) if
   imported — genuinely async, via a real `stream<u8>` handoff.
2. **WASI 0.2** `wasi:cli/stdout`/`stderr` (matched by `get-stdout`/`get-stderr`)
   otherwise — the sync write wrapped in an `async` function, for a uniform
   Promise-returning API regardless of which WASI version is available.
3. Neither imported — the returned promise rejects, naming both import options.

```js
export async function greet(name) {
    await console.print("no newline, ");
    await console.println("then a line.");
    return `Hello, ${name}!`;
}
```

**Only safe to call from an async export.** The WASI 0.3 path uses
component-model stream/future machinery that has no task state outside an
active async export call — calling it (even indirectly via `console.print`,
or `console.log`/`error` in a 0.3-only world) from a plain sync export
crashes outright ("no active task state"), not a graceful error.
`log`/`info`/`debug`/`warn`/`error` have no such restriction *when backed by
WASI 0.2* — always safe to call fire-and-forget from anywhere in that case —
but inherit the same async-export-only restriction as `print`/`println` when
falling back to WASI 0.3 (see the Console section above).

## Process

`process.env`/`process.argv`/`process.cwd()` are wired to
`wasi:cli/environment`, and `process.exit(code)` to `wasi:cli/exit`'s
`exit-with-code`, when the world imports them (this interface's shape is
unchanged between WASI 0.2 and 0.3, so unlike `console` there's no version
branching). Like `console`, `process` always exists — accessing an
unbacked property throws a clear error naming what to add.

```wit
world hello {
    import wasi:cli/environment@0.2.12;
    import wasi:cli/exit@0.2.12;
    export greet: func(name: string) -> string;
}
```

```js
export function greet(name) {
    const key = process.env.API_KEY;
    if (!key) process.exit(1);
    return `Hello, ${name}!`;
}
```

Divergences from Node worth knowing: `argv` is exactly `get-arguments()`
with no synthetic `node`/script-path entries prepended (WASI has no such
convention); `cwd()` returns `null` (not a fabricated path) when
`initial-cwd()` is `option::none`; `exit(code)` maps onto
`exit-with-code(status-code: u8)`, so `code` is coerced into a single byte
the same way Node itself truncates exit codes outside 0-255. `env`/`argv`
are always re-fetched on access rather than cached, matching the same
Wizer-snapshot-timing concern as `console`'s stdout handle.

## Polyfills

`TextEncoder`/`TextDecoder` (UTF-8 only) are always available — dwarf's
QuickJS runtime doesn't provide them natively, but they're foundational
enough (no WIT/host dependency, needed internally by other polyfills like
`url`) to include unconditionally rather than gate behind a flag.
`crypto.getRandomValues` is likewise always available, wired to
`wasi:random/random#get-random-bytes` when the world imports it (throws a
clear error otherwise, matching `console`/`process` below) — independent of
`--polyfill webcrypto`, which only adds `crypto.subtle`.

Beyond that, `--polyfill <name>` (repeatable) includes vendored third-party
libraries with no WIT/host dependency at all — opt-in, since (unlike
`console`/`process`) there's nothing in a WIT world to auto-detect "this is
wanted" from:

| Name | Provides | Notes |
|---|---|---|
| `buffer` | `Buffer` | [feross/buffer](https://github.com/feross/buffer), Node's `Buffer` reimplemented on `Uint8Array` |
| `url` | `URL`, `URLSearchParams` | [whatwg-url](https://github.com/jsdom/whatwg-url), spec-compliant including internationalized domain names — ~750KB before Wizer snapshotting (mostly Unicode/IDNA tables), only paid by components that request it |
| `fetch-classes` | `Headers`, `Request`, `Response`, `DOMException` | [whatwg-fetch](https://github.com/JakeChampion/fetch), trimmed to just the classes (its `fetch()` itself, which uses `XMLHttpRequest`, is excluded — pair with your own `fetch()` wired to a real WASI HTTP import) |
| `path` | `path` (`.join`, `.dirname`, `.basename`, etc. — matches Node's `path` module shape) | [unjs/pathe](https://github.com/unjs/pathe), fast and dependency-free |
| `readable-stream` | `ReadableStream`, `wit.readableStreamFromStream(readable)` | Hand-written (not vendored), verified against real `ReadableStream` — minimal pull-based-controller subset (no BYOB readers, `tee()`, or `pipeTo`/`pipeThrough`), matching what libraries like h3 actually use for streaming response bodies. The bridge helper wraps a `wit.Stream()` readable end for handing a WASI-backed body to code expecting the standard interface — single-read only, same reason as `fetch-provider`'s documented limitation (see below) |
| `webcrypto` | `crypto.subtle` (`digest`, HMAC, ECDSA/ECDH on P-256/P-384, HKDF, AES-GCM) | [@noble/hashes](https://github.com/paulmillr/noble-hashes) + [@noble/curves](https://github.com/paulmillr/noble-curves) + [@noble/ciphers](https://github.com/paulmillr/noble-ciphers), wrapped in a hand-written `crypto.subtle` covering a deliberate subset of the real Web Crypto API — no RSA, AES-CBC/CTR, PBKDF2, spki/pkcs8 DER (only `"raw"`/`"jwk"`), or Ed25519/X25519. `generateKey` needs `crypto.getRandomValues` (above), which needs `wasi:random/random` imported |

An unknown `--polyfill` name is a build error listing the available names.
See [NOTICES](NOTICES) for full attribution of vendored polyfill code.

Every polyfill also has a `.d.ts` (in `crates/core/polyfills/`), automatically
included in `--emit-types`' output as `globals.d.ts` alongside the WIT-derived
types — covering the always-on globals (`console`, `process`,
`TextEncoder`/`TextDecoder`) plus whichever `--polyfill` names were requested,
so `--polyfill` and `--emit-types` give full type coverage together.

**Development note:** setting `DWARF_POLYFILLS_DIR=/path/to/crates/core/polyfills`
(pointed at a local dwarf checkout) makes dwarf read polyfill `.js`/`.d.ts`
content fresh from disk instead of what's compiled into the binary — editing
a polyfill's source takes effect immediately, no `cargo build` needed. Unset
(the default), dwarf stays a single self-contained binary with no runtime
directory dependency.

## WIT Type Mappings

### Primitive Types

| WIT Type | JS Type | Notes |
|----------|---------|-------|
| `bool` | `boolean` | |
| `u8`, `u16`, `u32` | `number` | |
| `s8`, `s16`, `s32` | `number` | |
| `u64`, `s64` | `number` | Precision limited to 2⁵³ (Number.MAX_SAFE_INTEGER) |
| `f32`, `f64` | `number` | |
| `char` | `string` | Must be exactly one Unicode scalar value |
| `string` | `string` | |

### Compound Types

| WIT Type | JS Type | Example |
|----------|---------|---------|
| `list<T>` | `Array` | `[1, 2, 3]` |
| `list<u8>` | `Uint8Array` or `Array` | `new Uint8Array([1, 2, 3])` |
| `tuple<T, U, ...>` | `Array` | `[42, "hello"]` |
| `option<T>` | `T \| null` (nested: `{ tag: "some"\|"none", val }`) | `null` for none; `option<option<T>>` is wrapped |
| `result<T, E>` | top-level function result: return `T` or throw `E`; nested result: `{ tag: "ok"\|"err", val?: T\|E }` | `return 42` / `throw "error"` |
| `record { ... }` | `object` (camelCase keys) | `{ myField: 1 }` |
| `variant` | `{ tag: string, val?: T }` | `{ tag: "circle", val: 2.5 }` |
| `enum` | `string` (case name) | `"red"` |
| `flags` | `object` (camelCase booleans) | `{ read: true, write: false }` |
| `own<R>`, `borrow<R>` | resource object (methods on its prototype) | `input.blockingRead(n)` |

### Imported Resources

Imported resources are exposed as JavaScript classes. Resource methods are
called on the handle:

```js
import stdin from "wasi:cli/stdin@0.2.12";
import stdout from "wasi:cli/stdout@0.2.12";

const input = stdin.getStdin();     // an InputStream
const output = stdout.getStdout();  // an OutputStream

// Methods whose WIT return type is result<...> return the ok payload or throw.
const chunk = input.blockingRead(4096);   // method on the resource (len is a number)
output.blockingWriteAndFlush(chunk);
```

`[static]` methods are exposed on the resource class and `[constructor]` makes
the class callable with `new`.

### Async Exports

Async exports are declared with the `async` keyword in WIT and implemented
as JavaScript `async` functions:

```wit
package example:greeting;

world greeter {
    export greet: async func(name: string) -> string;
}
```

```js
export async function greet(name) {
    // You can use await here
    await Promise.resolve();
    return `Hello, ${name}!`;
}
```

### Streams

Streams transfer a sequence of values between components.
The `wit` global provides `Stream` and `Future` constructors for creating
stream/future pairs. The type is automatically determined from the WIT definition:

```wit
package example:streaming;

world streaming {
    export produce: async func() -> stream<u8>;
}
```

```js
async function produce() {
    // When only one stream type exists in the WIT, no argument needed
    const { readable, writable } = wit.Stream();

    writable.write(new Uint8Array([1, 2, 3]));
    writable.drop();

    return readable;
}
```

When the WIT defines multiple stream types, use the type constant:

```js
// wit.Stream.U8, wit.Stream.STRING, wit.Stream.U32, etc.
const { readable, writable } = wit.Stream(wit.Stream.U8);
const { readable, writable } = wit.Stream(wit.Stream.STRING);
```

Available type constants (populated from WIT metadata):

| WIT type | Constant |
|----------|----------|
| `stream<u8>` / `future<u8>` | `wit.Stream.U8` / `wit.Future.U8` |
| `stream<u32>` / `future<u32>` | `wit.Stream.U32` / `wit.Future.U32` |
| `stream<string>` / `future<string>` | `wit.Stream.STRING` / `wit.Future.STRING` |
| `stream<f64>` / `future<f64>` | `wit.Stream.F64` / `wit.Future.F64` |

All constructors return `{ readable, writable }`.

**Complex element types** are also supported. The type constant is generated
recursively from the WIT type structure:

```js
// stream<result<string, u32>>
wit.Stream(wit.Stream.RESULT_STRING_U32);

// stream<option<u32>>
wit.Stream(wit.Stream.OPTION_U32);

// stream<tuple<u32, string>>
wit.Stream(wit.Stream.TUPLE_U32_STRING);

// Named record types use their WIT name:
// record point { x: f64, y: f64 }
// stream<point>
wit.Stream(wit.Stream.POINT);
```

Use `wit.Stream.types` or `wit.Future.types` to discover all available type
constants at runtime.

**StreamReadable methods:**

| Method | Returns | Description |
|--------|---------|-------------|
| `read(count?)` | `Promise<T[]>` (or `Uint8Array` for `u8`) | Read up to `count` values |
| `cancelRead()` | result or `undefined` | Cancel an in-progress read |
| `drop()` | `void` | Release the stream handle |

**Calling `read()` again after the writable end has dropped and all data is
already consumed is not a catchable JS error** — confirmed empirically, it's
a hard host-level trap ("cannot read after being notified that the writable
end dropped"). A drain-until-empty loop is only safe if you have some other
way to know more data is coming before calling `read()` again; otherwise,
read once with a generously-sized `count` and treat that as the whole
payload (the pattern `examples/fetch-provider` and the `readable-stream`
polyfill's WASI bridge both use).

**StreamWritable methods:**

| Method | Returns | Description |
|--------|---------|-------------|
| `write(data)` | `Promise<number>` | Write values, returns count written |
| `writeAll(data)` | `Promise<number>` | Write all values, retrying as needed |
| `cancelWrite()` | result or `undefined` | Cancel an in-progress write |
| `drop()` | `void` | Release the stream handle |

### Futures

Futures transfer a single value. They work like streams but carry exactly one
value:

```wit
package example:async-value;

world async-value {
    export compute: async func() -> future<string>;
}
```

```js
async function compute() {
    const { readable, writable } = wit.Future();

    // Write the value (fire-and-forget; completes when reader reads)
    writable.write("computed result");

    return readable;
}
```

**Future type constants** follow the same pattern: `wit.Future.U32`,
`wit.Future.STRING`, etc.

**FutureReadable methods:**

| Method | Returns | Description |
|--------|---------|-------------|
| `read()` | `Promise<T>` | Read the single value |
| `cancelRead()` | result or `undefined` | Cancel an in-progress read |
| `drop()` | `void` | Release the future handle |

**FutureWritable methods:**

| Method | Returns | Description |
|--------|---------|-------------|
| `write(value)` | `Promise<boolean>` | Write the value, returns success |
| `cancelWrite()` | result or `undefined` | Cancel an in-progress write |
| `drop()` | `void` | Release the future handle |

### Resource Cleanup

Stream and future handles support
[Explicit Resource Management](https://github.com/tc39/proposal-explicit-resource-management)
via `Symbol.dispose`. In environments that support `using`:

```js
{
    using stream = wit.Stream();
    // stream.writable and stream.readable are auto-dropped when leaving scope
}
```

Otherwise, call `.drop()` explicitly to release handles.

## Node.js API

The npm package exposes both a CLI and a programmatic API.

### CLI

```bash
npx dwarf --wit hello.wit --js hello.js -o hello.wasm
```

(Or, if you installed the package globally, just `dwarf ...`.)

### Usage

```js
import { componentize } from "dwarf";

const { component } = await componentize({
    witPath: "hello.wit",
    jsSource: "export function greet(name) { return `Hello, ${name}!`; }",
    optSize: true,
});
// component is a Buffer containing the WebAssembly component bytes
```

Runtime selection is available through `optSize`, `sync`, `runtime`, or
`runtimeBytes`. `optSize` and `sync` may be combined to select the non-async
opt-size runtime, but neither can be combined with a custom `runtime`/`runtimeBytes`.
The `runtime` option is a path to a custom QuickJS runtime Wasm module.

## Acknowledgments

`dwarf` is a fork of [componentize-qjs](https://github.com/andreiltd/componentize-qjs)
by Andrei (andreiltd) — nearly all functional code originates there; see
[NOTICES](NOTICES). componentize-qjs itself builds on ideas and code from:

- [ComponentizeJS](https://github.com/dicej/componentize-js) by Joel Dice
- [lua-component-demo](https://github.com/alexcrichton/lua-component-demo) by Alex Crichton

## License

Licensed under [Apache-2.0](LICENSE).
