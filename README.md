# dwarf

[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![WASI 0.3](https://img.shields.io/badge/WASI-0.3%20%C2%B7%20Preview%203-6f42c1.svg)](https://github.com/WebAssembly/WASI/blob/main/Proposals.md)

**JavaScript to WebAssembly components, written for WASI 0.3.**

`dwarf` takes a JavaScript file and a
[WIT](https://component-model.bytecodealliance.org/design/wit.html) world and
produces a standalone
[component](https://component-model.bytecodealliance.org/), using
[QuickJS](https://github.com/quickjs-ng/quickjs) — QuickJS-sized, not a ~13 MB
SpiderMonkey embedding, with full typed bindings for arbitrary worlds rather
than just the standard WASI ones.

The part worth leading with is **WASI 0.3 (Preview 3)**. An `async func` export
is an `async` JavaScript function; a `stream<u8>` is an object you `read()` and
`writeAll()`; a `future<T>` is one you `await`. Concurrency is the component
model's own, not a callback shape bolted on top, and 0.2 remains supported for
worlds that still need it.

Output runs on any component-model runtime — [Wasmtime](https://wasmtime.dev/)
directly, or **Trail**, the Periapsis workload runtime, which serves
`wasi:http/handler@0.3.0` components and is what the HTTP examples here are
verified against. See [Running the output](#running-the-output).

`dwarf` is a fork of
[componentize-qjs](https://github.com/andreiltd/componentize-qjs)
(Apache-2.0), rebranded and maintained here for the
Periapsis project family. See
[NOTICES](NOTICES) for full upstream attribution and `git log` for
commit-level history predating the fork. Not published to crates.io or npm —
build from source.

## Overview

### Why WASI 0.3

0.3 is the version where the component model's concurrency stops needing a
translation layer, and that shows up directly in the JavaScript you write.

```wit
world greeter {
    export greet: async func(name: string) -> string;
}
```

```js
export async function greet(name) {
    return `Hello, ${name}!`;   // an `async func` is just an async function
}
```

The same holds for the types around it. A `stream<u8>` is an object with
`read()` and `writeAll()`; a `future<T>` is one you `await`; a `result<T, E>`
returns `T` or throws `E`; an `option<T>` is `T` or `null`. Full detail in
[WIT Type Mappings](#wit-type-mappings),
[Async Exports](#async-exports), [Streams](#streams) and
[Futures](#futures).

Concretely, that means a component can *serve* — hold a connection open,
stream a body, await a timer, handle requests concurrently — rather than only
answer one synchronous call at a time. `wasi:http/handler@0.3.0`,
[WebSockets](#websockets) and [Timers](#timers) all fall out of the same
mechanism.

WASI 0.2 worlds still build and still work; `--sync` targets hosts without
component-model async at all. Where an 0.3 name might shift meaning later,
the version-pinned [`...P3` aliases](#version-pinned-p3-names) stay bound to
the 0.3 implementation.

### How it works

`dwarf` takes a JavaScript source file and a
[WIT](https://component-model.bytecodealliance.org/design/wit.html) definition,
and produces a standalone WebAssembly component that can run on any
component-model runtime (see [Running the output](#running-the-output)).

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

Not published to crates.io or npm yet. Build from source:

```bash
git clone https://github.com/apsis-io/dwarf.git
cd dwarf
```

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

## Running the output

The output is an ordinary WebAssembly component: any component-model host can
run it. Two that get used here.

**Wasmtime**, for one-shot calls and `wasi:cli/run` components. WASI 0.3 needs
the async feature turned on:

```bash
wasmtime run --wasm component-model-async=y --invoke 'greet("World")' hello.wasm
```

**Trail**, the Periapsis workload runtime, for components that serve HTTP. It
drives `wasi:http/handler@0.3.0` over a hyper HTTP/1.1 server:

```bash
trail --p3 --serve --component app.wasm --listen 127.0.0.1:8080
```

By default each request gets a fresh `Store` and instance — the snapshot
Wizer took is the starting point every time, so one request cannot leave
state behind for the next. `--persistent` holds a single instance for the
process lifetime instead, keeping module-level state across requests at the
cost of that isolation. Which one you want is the same question as
[Reactors: instantiate once, call many](#reactors-instantiate-once-call-many),
and `_initialize` is where per-instance setup belongs under either.

[`examples/hono`](examples/hono) is a full worked example — the Hono router on
`wasi:http/handler@0.3.0`, verified end to end under `trail --p3 --serve`.
Periapsis also runs a complete Nuxt 4 SSR app as a dwarf component
(`examples/wasm/js-dwarf-nuxt` there), which is where this repo's
request/response bridge comes from.

> Trail and Periapsis are not public repositories yet, so they are named here
> rather than linked. Nothing in dwarf depends on them: the components it
> builds are plain WASI 0.3 components, and Wasmtime runs them.

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
| `--optimize <MODULE>` | | Compile a TypeScript module statically with [scriptc](#statically-compiled-modules) and plug it in (repeatable) |
| `--scriptc <PROFILE>` | | Same, from a profile that declares the boundary explicitly instead of deriving it (repeatable) |
| `--scriptc-bin <PATH>` | | The scriptc executable for `--scriptc` (default: `scriptc` on PATH) |

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

## Reproducible builds

The same inputs produce a byte-identical component. Wizer freezes the
initialized guest heap into the artifact, so anything the guest observes
while being snapshotted is baked in — which makes reproducibility a property
of the *build environment*, not just of the code. dwarf pins that
environment:

| pinned | why |
| --- | --- |
| the wall clock | QuickJS seeds `ctx->random_state` from `js__gettimeofday_us()`, so `Math.random`'s state was the build time |
| `wasi:random` | something in the guest caches 16 bytes of it at init (std's `RandomState` shape); the runtime's own maps were already fixed-seed |
| the implicit module root | it was the process's **cwd**, and the root decides each module's guest-visible path — so the same entry built from two directories differed |

Only the snapshot sees any of this. The component you ship reads the host's
real clock and real randomness at run time; nothing about its behaviour
changes.

**`SOURCE_DATE_EPOCH`** is the one deliberate knob, following the
[reproducible-builds](https://reproducible-builds.org/docs/source-date-epoch/)
convention: set it and the snapshot's clock reads that instant, leave it and
the clock reads the Unix epoch. The same value always gives the same bytes.

```bash
dwarf --wit hello.wit --js hello.js -o a.wasm
dwarf --wit hello.wit --js hello.js -o b.wasm
cmp a.wasm b.wasm && echo identical
```

`tests/deterministic.rs` guards this at the level a consumer cares about —
the bytes — including that the working directory cannot change the output.

## Reactors: instantiate once, call many

Every ordinary `dwarf --wit x.wit --js x.js` build already produces what the
component-model community calls a **reactor** — a component that exports its
own world and is instantiated once, then called repeatedly. The only other
shape is a **command**: a component exporting `wasi:cli/run`, which a host
runs to completion once (the `wasi:cli/command` world, where the implicit
`run` export needs `export function run() { ... }` in your JS).

The reactor/command *label* is a preview-1 core-module distinction —
`_initialize` versus `_start` — and dwarf still links the p1 reactor adapter
because its QuickJS module is built against p1. At the component level the
label carries no meaning and WASI 0.3 has no `wasi:cli/reactor` world: a
component simply exports what its world declares. What survives into p3 is
the **lifecycle**, and that is worth knowing precisely:

| | |
| --- | --- |
| the JS module's top level | runs at **build** time, under Wizer, and is snapshotted |
| a fresh instance | starts from that snapshot, never from another instance's state |
| state between calls | persists for the life of one instance |
| `_initialize` | runs **once per instance**, before the first exported call |
| teardown | there is none — see below |

```js
let hits = 0;              // snapshotted at build time: every instance starts at 0

export function _initialize() {
  // Per-instance setup that CANNOT be snapshotted: a handle, a clock read,
  // configuration pulled through an imported interface.
}

export function handle(req) {
  hits += 1;               // persists across calls on this instance
  return `${hits}`;
}
```

`_initialize` cannot collide with a WIT export: WIT identifiers are
lowercase kebab-case and reach JS as camelCase, so no export is ever named
`_initialize`. If your module does not export it, nothing runs. If it
throws, the call traps rather than serving traffic from a half-built
instance.

**There is no teardown hook**, because the component model gives a guest no
destructor callback — an instance is simply dropped by the host. If you need
explicit cleanup, declare it in your world as an ordinary export
(`export shutdown: func();`) and have the host call it; that needs nothing
from dwarf.

A Periapsis *Comet* is exactly this shape: a reactor exporting its own
interface rather than `wasi:cli/run`, so it is written and built the way
this section describes.

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
makes them a poor fit for **prebuilt, reusable glue meant to be composed
into many different consumers without recompiling** (a real, standalone
component's own fixed indices can't reliably match an arbitrary caller's).
The pattern for that case: build the capability as its own small, focused
component (a fixed world, so its indices are fixed and known) that exports a
plain-types-only interface — strings, lists, records, no resources/streams/
futures crossing the boundary — and compose it into your own component with
[`wac plug`](https://github.com/bytecodealliance/wac). See
[`examples/fetch-provider`](examples/fetch-provider) for a complete,
verified-against-real-HTTP-servers example of this pattern.

This constraint does *not* apply to dwarf's own codegen, though, which
generates fresh JS for each consuming component's own specific WIT world at
build time and so always has the right indices for that world — which is
why the global `fetch()` polyfill (`--polyfill fetch-classes`, see
[Polyfills](#polyfills)) can wire directly to `wasi:http/client` without
needing a separate composed component at all, for the common case of one
component wanting its own `fetch()`.

### Statically Compiled Modules

Code that runs hot does not have to run in QuickJS.
[scriptc](https://github.com/vercel-labs/scriptc) compiles TypeScript ahead
of time, and `--optimize` builds a module with it and plugs the result into
the component being generated — QuickJS keeps everything dynamic, while leaf
modules doing real work over numbers, strings, and bytes become native Wasm.

Point it at a module and the boundary is derived from that module's
exported signatures:

```bash
dwarf --wit app.wit --js app.js --optimize hot.ts -o app.wasm
```

JavaScript imports it under `scriptc:<module name>/ops`, and your world
declares nothing — dwarf adds the interface itself from the WIT scriptc
generates:

```js
import ops from "scriptc:hot/ops";

export function digest(text) {
  return ops.checksum(new TextEncoder().encode(text));
}
```

The seam does not survive into the output: the interface is satisfied by
composition, so the finished component imports only WASI.

What crosses the boundary is limited to what the canonical ABI carries
cheaply — `number` (as `f64`), `boolean`, `string`, and `Uint8Array`. An
export taking a callback, a class instance, or a closure cannot cross, nor
can an `async` or generic one; each is named on stderr as it is left out,
so the interface is never a silent subset.

**Crossing is necessary, not sufficient.** Moving a call across costs
roughly 1.5µs per KB of string or list payload, so it pays only where the
call does substantially more work than that. Two functions of the same
shape, measured against a release build:

| | payload | result |
|---|---|---|
| `sha1(bytes) -> bytes` | 1 KiB in, 20 B out | **4.2× faster** |
| `layout(string, string) -> string` | ~20 B in, 2.4 KB out | **4.7× slower** |

`sha1` does ~1500µs of work per call and pays ~1µs to cross. `layout`
builds a template string in ~0.7µs and pays ~2.4µs. Use `scriptc boundary`
to find candidates, then measure — the win concentrates in leaf modules
whose work per call dwarfs their payload.

Measure with a **release** build: `build.rs` compiles the embedded QuickJS
runtime with profile-dependent flags (`-Clto=fat -Copt-level=3` for
release, none for debug), and the component-model marshalling runs inside
that runtime — so a debug host inflates both the wins and the losses.

### Declaring the boundary explicitly

`--optimize` writes the profile it derived beside the module, so you can
read it, check it in, and edit it once the defaults stop fitting — then
pass it with `--scriptc` instead. A profile also reaches what inference
will not guess: the sized integer classes (`u8`/`u32`/`i32`/`i64`/`u64`),
since `number` alone does not say which was meant.

A profile names the module and the functions to expose:

```json
{
  "profile_format": 1,
  "name": "hot",
  "entry": "hot.ts",
  "emission": "c",
  "abi": {
    "prefix": "hot_",
    "init_symbol": "hot_init",
    "sink_register_symbol": "hot_set_panic_sink",
    "collect_symbol": "hot_collect",
    "result_reset_symbol": null
  },
  "exports": [
    { "export": "checksum", "symbol": "hot_checksum", "params": ["bytes"], "returns": "f64" }
  ]
}
```

```bash
dwarf --wit app.wit --js app.js --scriptc hot/profile.json -o app.wasm
```

Either flag needs a scriptc install and the toolchain behind it (zig and
`wasm-tools`); the WASI adapter is dwarf's own, so there is nothing else to
configure.

### Vendoring WIT Dependencies

When `--wit` points at a directory, referenced packages (e.g. `wasi:cli`)
are normally vendored under a `deps/` subdirectory next to your WIT files. If
a package is missing, dwarf automatically runs `wkg wit fetch` to fetch it
(requires [`wkg`](#prerequisites) on `PATH`) and retries — no manual `deps/`
setup needed for a quick start. Disable this with `--no-vendor`; auto-vendoring
only applies to directory `--wit` paths, since a single standalone WIT file has
no `deps/` directory to populate.

## Version-pinned `...P3` names

Every WASI-backed global below (`console`, `process`, `crypto.getRandomValues`,
`setTimeout`/`setInterval`/`clearTimeout`/`clearInterval`, `fetch`,
`WebSocketServer`) is also available under an explicit `...P3` name
(`consoleP3`, `processP3`, `crypto.getRandomValuesP3`, `setTimeoutP3`, etc.) —
the exact same object/function as the plain name, just under a second,
stable name. This is purely additive: nothing that depends on the plain
name (your own code, vendored library code) is affected.

All of these are backed by WASI 0.3 today, so right now the alias changes
nothing. It exists for later: if a future WASI version ever changes what the
plain name points to, `xP3` stays pinned to "the 0.3 implementation,
specifically" — reach for it when you want that guarantee. Otherwise, just
use the plain name; if you want the short name back after depending on the
pinned one, TypeScript's own import aliasing covers that at your own call
site (`import { consoleP3 as console } from "..."`).

## Console

`console.log`/`info`/`debug` and `console.warn`/`error` are available when the
world imports `wasi:cli/stdout`/`stderr@0.3.x` (matched by `write-via-stream`).
WASI 0.3 has no synchronous write primitive, so these calls return a
`Promise<void>` instead of `undefined`.

```wit
world hello {
    import wasi:cli/stdout@0.3.0;
    import wasi:cli/stderr@0.3.0;
    export greet: async func(name: string) -> string;
}
```

```js
export async function greet(name) {
    await console.log("greeting", name);
    return `Hello, ${name}!`;
}
```

`console` always exists — if the import isn't declared, calling the relevant
method throws a clear error naming it, rather than leaving `console`
undefined or silently doing nothing.

**Unawaited calls can silently lose output** if nothing else in the same
async export subsequently awaits or yields — confirmed empirically: two
unawaited `console.log` calls followed by an `await` on a third all flushed
correctly, but an unawaited `console.error` as the last statement before an
export returns produced no output at all. Await `console.log`/`error`/etc.
whenever you can't otherwise guarantee the surrounding async export keeps
running long enough to flush the write.

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
`println`/`eprintln` append one. They use `wasi:cli/stdout`/`stderr@0.3.x`
(matched by `write-via-stream`) if imported — genuinely async, via a real
`stream<u8>` handoff — and otherwise the returned promise rejects, naming
the missing import.

```js
export async function greet(name) {
    await console.print("no newline, ");
    await console.println("then a line.");
    return `Hello, ${name}!`;
}
```

**Only safe to call from an async export.** WASI 0.3 uses component-model
stream/future machinery that has no task state outside an active async
export call — calling it (even indirectly via `console.print`, or
`console.log`/`error`) from a plain sync export crashes outright ("no
active task state"), not a graceful error.

## Process

`process.env`/`process.argv`/`process.cwd()` are wired to
`wasi:cli/environment`, and `process.exit(code)` to `wasi:cli/exit`'s
`exit-with-code`, when the world imports them (this interface's shape is
unchanged between WASI 0.2 and 0.3, so unlike `console` there's no version
branching). Like `console`, `process` always exists — accessing an
unbacked property throws a clear error naming what to add.

```wit
world hello {
    import wasi:cli/environment@0.3.0;
    import wasi:cli/exit@0.3.0;
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

## Timers

`setTimeout`/`setInterval` are wired to
`wasi:clocks/monotonic-clock@0.3.x`'s `wait-for` (a genuine `async func`,
directly awaitable) when the world imports it — throws a clear error
otherwise. WASI 0.2's `monotonic-clock` has no non-blocking wait primitive
at all (only `subscribe-duration` + `wasi:io/poll`'s `pollable`, which
*blocks* the whole component — the opposite of what `setTimeout` is for),
so only 0.3 is supported. `clearTimeout`/`clearInterval` are always defined
and never throw, even in a world where `setTimeout` itself would.

```wit
world poller {
    import wasi:clocks/monotonic-clock@0.3.0;
    export run: async func() -> string;
}
```

```js
function sleep(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

export async function run() {
    await sleep(100);
    return "done";
}
```

**Important caveat, unavoidable under component-model-async:** once the
async export that (transitively) called `setTimeout`/`setInterval` settles,
any still-pending timer callback that wasn't part of the explicitly-awaited
chain is cancelled along with the rest of that task — there is no way for
dwarf to keep a timer alive past its originating export call the way a real
JS host's event loop would. Reliable only when awaited (as in `sleep`
above) or when called from a still-running, long-lived export.

## WebSockets

A global `WebSocketServer` class is wired to `wasi:sockets/types@0.3.0`'s
`tcp-socket` resource when the world imports it — throws a clear error
otherwise. It also needs `--polyfill webcrypto` for computing the
`Sec-WebSocket-Accept` handshake header (`crypto.subtle.digest("SHA-1",
...)`); calling `listen()` without it throws a clear error naming the flag.

```wit
world ws-server {
    import wasi:sockets/types@0.3.0;
    export run: async func(port: u16);
}
```

```js
export async function run(port) {
    const server = new WebSocketServer();
    server.on("connection", (conn) => {
        conn.on("message", (data) => {
            // `data` is a string for text frames, a Uint8Array for binary.
            conn.send(typeof data === "string" ? "echo:" + data : data);
        });
        conn.on("close", (code, reason) => {});
    });
    await server.listen(port, "127.0.0.1"); // runs forever, accepting connections
}
```

`server.listen(port, host)` binds, starts listening, and accept-loops
forever — it's meant to be `await`ed from a long-lived entrypoint (e.g. a
`wasi:cli/run` `run()` kept alive as a background task via the host's own
persistent/also-run mechanism alongside normal request handling, the same
class of setup trail's `--persistent` flag provides). Each accepted
connection is handled concurrently with accepting further connections
(ordinary single-threaded JS promise concurrency, not real parallelism).
After binding, `server.port` holds the actual bound port (useful when
`port` is `0`, requesting an OS-assigned ephemeral port). `server.close()`
stops the accept loop.

A connection object passed to the `'connection'` handler has `.send(data)`
(string or `Uint8Array`), `.ping(data)`, `.close(code, reason)`, `.path`
and `.headers` (from the upgrade request), and `.on(event, cb)` for
`'message'`, `'ping'`, `'pong'`, `'close'`, and `'error'`.

The frame parser (validation rules, fragmentation/control-frame handling)
is adapted from [websockets/ws](https://github.com/websockets/ws)'s
`receiver.js` (see NOTICES), cross-tested against a real, independent
client. Scope cuts: IPv4 only, no permessage-deflate (never negotiated, so
compliant clients simply don't use it), no `Blob`/`ArrayBuffer`
`binaryType` switch (binary messages are always `Uint8Array` — there's no
`Blob` in dwarf).

### Single-port HTTP + WebSocket routing

`WebSocketServer` also doubles as a general router when a host can only
reach a component on one port (e.g. a TLS-passthrough-by-SNI edge with a
single backend, unable to split a normal HTTP response path and WebSocket
upgrades across two ports). Register an `on("request", ...)` handler —
additionally requires `--polyfill fetch-classes`, for real `Request`/
`Response` — and any non-upgrade HTTP request is routed there instead of
being dropped; WS upgrade requests still go through the handshake/frame
path unchanged, on the very same listening socket:

```js
export async function run(port) {
    const server = new WebSocketServer();

    server.on("request", async (request) => {
        // Wire your own SSR/HTTP handler here - e.g. Nitro/h3's
        // `toWebHandler(app)` produces exactly this (Request) => Response
        // shape, so `server.on("request", nitroHandler)` works directly.
        const url = new URL(request.url);
        if (url.pathname === "/api/health") return new Response("ok");
        return new Response("not found", { status: 404 });
    });

    server.on("connection", (conn) => {
        conn.on("message", (data) => conn.send(`echo:${data}`));
    });

    await server.listen(port, "127.0.0.1");
}
```

Without a `"request"` handler registered, behavior is unchanged from
before this existed: a non-upgrade request is dropped. With one, each
accepted connection runs an ordinary HTTP/1.1 keep-alive loop — multiple
requests over one connection are supported (real SSR pages need several
asset fetches), ending either when a request upgrades to a WebSocket (which
then owns the connection for the rest of its life, handed to
`"connection"`'s handler), `Connection: close`, or the peer disconnecting.
Request bodies are read via `Content-Length` only (no chunked
transfer-encoding, no `Expect: 100-continue`).

This parses untrusted network input inside the guest, so it's hardened
against hostile/malformed requests, failing closed and bounded rather than
hanging or over-allocating: request header blocks are capped at 16 KiB
(`431` if exceeded) with at most 100 headers, request lines and
`Content-Length` are strictly validated (`400` on anything malformed —
negative, non-numeric, or otherwise not a plain non-negative integer),
`Transfer-Encoding` is rejected outright (`501`) rather than silently
mishandled (treating a chunked body as length-zero would let its bytes be
misread as the next pipelined request), an oversized `Content-Length` is
rejected (`413`) before a single byte of the body is read or buffered, and
a connection that goes idle mid-request (opens, then sends nothing, or
trickles bytes forever — the slowloris shape) is given up on after
`idleTimeoutMs` (default 30s; needs `wasi:clocks/monotonic-clock@0.3.x`
imported to be enforced at all — see `WebSocketServer`'s constructor
options). Any parse failure closes the connection afterward rather than
keeping it alive for more pipelining, since the byte stream's framing can
no longer be trusted.

## Polyfills

`TextEncoder`/`TextDecoder` (UTF-8 only) are always available — dwarf's
QuickJS runtime doesn't provide them natively, but they're foundational
enough (no WIT/host dependency, needed internally by other polyfills like
`url`) to include unconditionally rather than gate behind a flag.
`AbortController`/`AbortSignal` are likewise always available and have real
semantics (`abort()` flips `aborted`, sets `reason`, fires
`onabort`/`'abort'` listeners) — nothing in dwarf's own `fetch()` observes
`signal` yet, but a caller polling `signal.aborted` in its own loop works
today. `crypto.getRandomValues` is likewise always available, wired to
`wasi:random/random#get-random-bytes` when the world imports it (throws a
clear error otherwise, matching `console`/`process` above) — independent of
`--polyfill webcrypto`, which only adds `crypto.subtle`.

Beyond that, `--polyfill <name>` (repeatable) includes vendored third-party
libraries with no WIT/host dependency at all — opt-in, since (unlike
`console`/`process`) there's nothing in a WIT world to auto-detect "this is
wanted" from:

| Name | Provides | Notes |
|---|---|---|
| `buffer` | `Buffer` | [feross/buffer](https://github.com/feross/buffer), Node's `Buffer` reimplemented on `Uint8Array` |
| `url` | `URL`, `URLSearchParams` | [whatwg-url](https://github.com/jsdom/whatwg-url), spec-compliant including internationalized domain names — ~750KB before Wizer snapshotting (mostly Unicode/IDNA tables), only paid by components that request it |
| `fetch-classes` | `Headers`, `Request`, `Response`, `DOMException` | [whatwg-fetch](https://github.com/JakeChampion/fetch), trimmed to just the classes. Requesting this polyfill also enables a real global `fetch()` (always-on, not gated by this flag itself — see below), wired directly to `wasi:http/client@0.3.x` when the world imports it |
| `path` | `path` (`.join`, `.dirname`, `.basename`, etc. — matches Node's `path` module shape) | [unjs/pathe](https://github.com/unjs/pathe), fast and dependency-free |
| `readable-stream` | `ReadableStream`, `wit.readableStreamFromStream(readable)` | Hand-written (not vendored), verified against real `ReadableStream` — minimal pull-based-controller subset (no BYOB readers, `tee()`, or `pipeTo`/`pipeThrough`), matching what libraries like h3 actually use for streaming response bodies. The bridge helper wraps a `wit.Stream()` readable end for handing a WASI-backed body to code expecting the standard interface — single-read only, same reason as `fetch-provider`'s documented limitation (see below) |
| `webcrypto` | `crypto.subtle` (`digest`, HMAC, ECDSA/ECDH on P-256/P-384, HKDF, AES-GCM) | [@noble/hashes](https://github.com/paulmillr/noble-hashes) + [@noble/curves](https://github.com/paulmillr/noble-curves) + [@noble/ciphers](https://github.com/paulmillr/noble-ciphers), wrapped in a hand-written `crypto.subtle` covering a deliberate subset of the real Web Crypto API — no RSA, AES-CBC/CTR, PBKDF2, spki/pkcs8 DER (only `"raw"`/`"jwk"`), or Ed25519/X25519. `generateKey` needs `crypto.getRandomValues` (above), which needs `wasi:random/random` imported |
| `ufo` | `ufo.*` — functional URL utilities (`joinURL`, `withQuery`, `parseURL`, `normalizeURL`, etc.) | [unjs/ufo](https://github.com/unjs/ufo). Complements `url`'s class-based `URL`/`URLSearchParams` with the more ergonomic functional helpers many h3/nitro-style codebases use |
| `scule` | `scule.*` — string case conversion (`camelCase`, `kebabCase`, `snakeCase`, `pascalCase`, `trainCase`, `titleCase`, etc.) | [unjs/scule](https://github.com/unjs/scule) |
| `klona` | `klona(value)` — deep clone | [lukeed/klona](https://github.com/lukeed/klona). dwarf's QuickJS-ng has no `structuredClone` at all (confirmed, not just "klona is faster") |
| `ohash` | `ohash.*` — `hash()`/`serialize()`/`isEqual()` | [unjs/ohash](https://github.com/unjs/ohash), bundled with its pure-JS (non-Node) SHA-256-based digest. Not for security use — see `webcrypto` for cryptographic hashing |
| `knitwork` | `knitwork.*` — JS/TS code-string generation (`genImport`, `genObjectFromRaw`, `genInterface`, etc., no parsing) | [unjs/knitwork](https://github.com/unjs/knitwork) |
| `unstorage` | `unstorage.*` — universal key-value storage (`createStorage()`, `.getItem`/`.setItem`/`.getKeys`/etc.) | [unjs/unstorage](https://github.com/unjs/unstorage). Only the core plus its zero-config default (in-memory) driver are bundled — every other driver (fs, redis, cloudflare-kv, etc.) is Node/host-specific and not available. In-process memory only, not persisted across restarts |

An unknown `--polyfill` name is a build error listing the available names.
See [NOTICES](NOTICES) for full attribution of vendored polyfill code.
Vendored (bundled-from-npm) static polyfills are bundled and registered with
[`scripts/bundle-polyfill`](scripts/bundle-polyfill), a dev-only tool that
generates the `install` line's global-exposure object from the bundle's own
actual export list (via [knitwork](https://github.com/unjs/knitwork)) rather
than a hand-transcribed one.

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

### Error shapes: imports vs. exports

`result<T, E>`'s `err` case is surfaced differently depending on which side
of the boundary you're on — this is intentional, not an inconsistency to
work around:

- **Calling an import** that comes back `err`: dwarf always wraps the raw
  err payload in a real `Error` — real `.message`/`.stack`,
  `instanceof Error`, catchable and loggable like any normal JS exception —
  with the raw tagged payload attached as `error.payload`. For a
  `variant`/`enum`/`record` err type, that means reading `error.payload.tag`/
  `.val`, not `error.tag`/`.val` directly.
- **An export** signaling its own `err` result can throw a plain object
  matching the WIT shape directly — `throw { tag: "not-found" }` is used
  as-is, no `.payload` needed — or a real `Error` with a `.payload` property
  shaped like the WIT error (unwrapped on the way out). A `string` err type
  also accepts a bare thrown string, or an `Error`'s `.message`.

The reasoning: an import's error is something JS *receives*, so wrapping it
in a real `Error` gives normal exception ergonomics for a value JS didn't
construct itself. An export's error is something JS *authors*, so accepting
the same `{ tag, val }` shape used everywhere else for variants/results
avoids forcing an `Error` wrapper around a value whose shape the author
already knows.

### Imported Resources

Imported resources are exposed as JavaScript classes. Resource methods are
called on the handle:

```js
import { TcpSocket } from "wasi:sockets/types@0.3.0";

// [static] methods are exposed directly on the class.
const sock = TcpSocket.create("ipv4");

// Methods whose WIT return type is result<...> return the ok payload or throw.
sock.bind({ tag: "ipv4", val: { port: 0, address: [0, 0, 0, 0] } });

// async func methods return a real Promise.
await sock.connect({ tag: "ipv4", val: { port: 80, address: [93, 184, 216, 34] } });
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
