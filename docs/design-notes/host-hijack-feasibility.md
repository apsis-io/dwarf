# Feasibility: dwarf-side support for `periapisis:host/hijack`

Status: **task-lifetime question resolved (good news); borrow-forwarding
question reopened (real, confirmed blocker).** Written to de-risk a possible
post-demo-7 migration away from `WebSocketServer`'s raw-socket HTTP+WS router
(`crates/core/polyfills/websocket-server.js`, WS-1/WS-1b) toward a
host-hijack-based single-port answer, per the architecture fork
`trail-main-cli` flagged during WS-1b (`periapisis:host/hijack`, ADR-0046,
live in the periapsis repo).

Two separate empirical questions have now been run down:

1. **Task/resource lifetime past `handle()`'s own return**
   (`tests/task_lifetime.rs`, `tests/task_persistence_rootcause.rs`): holding
   a resource across the exporting call's own task settlement does **not**
   trap the guest - it's the already-documented `setTimeout` fire-and-forget
   caveat, and it's entirely a host-embedding API choice (`call_async` vs
   `call_concurrent` + a persistent `run_concurrent` scope), not a
   wasmtime or dwarf-runtime limitation. See "Root cause: it's a host-side
   API choice, not a wasmtime limit" below - **this part is resolved, no
   dwarf-side blocker.**
2. **Passing an export-received resource as a borrow into an import call**
   (`tests/borrow_passthrough.rs`, `tests/own_then_borrow.rs`,
   `tests/own_then_own.rs`): this is the *actual* `claim(request:
   borrow<request>)` shape, and it **traps** - "borrow handles still remain
   at the end of the call." This reverses this note's original
   read-by-inspection conclusion that `pop_borrow`'s generic, type-agnostic
   code "should already work." See "Borrow-forwarding finding: a real,
   confirmed, general gap" below - **this is a genuine blocker, not
   host-hijack-specific, and not yet root-caused.**

## The interface being scoped

```wit
package periapisis:host@0.1.0;

interface hijack {
  use wasi:http/types@0.3.0.{request};

  enum hijack-error { not-allowed, already-claimed, connection-gone, not-applicable }

  resource hijacked-connection {
    read: async func(max: u64) -> result<list<u8>, hijack-error>;
    write: async func(data: list<u8>) -> result<_, hijack-error>;
  }

  claim: func(request: borrow<request>) -> result<hijacked-connection, hijack-error>;
}
```

The intended usage shape (per trail-main-cli): a guest exports
`wasi:http/incoming-handler` as usual; on a request it decides to upgrade,
instead of returning a `response` it imports `periapisis:host/hijack` and
calls `claim(request)` on the *same* `request` value `handle()` received,
getting back a `hijacked-connection` it reads/writes raw bytes on (the
WebSocket handshake response + RFC 6455 frames) for the rest of the
connection's life.

## Bottom line

**Dwarf's existing generic codegen almost certainly needs zero new code for
the interface shape itself.** Every individual piece here is a pattern dwarf
already generates and has working, tested coverage for:

| Piece | Already proven by |
|---|---|
| A custom (non-`wasi:`) package/interface, no special-casing needed | Every example (`math-provider.wit`, `imports.wit`) - `wit_dylib`'s codegen is fully generic per-WIT-world, not hardcoded per interface (confirmed repeatedly this session, e.g. `generate_fetch`'s own doc comment on why hardcoding `wit.Future`/`wit.Stream` constant *names* - not indices - is safe for any world) |
| A `resource` with `async func` methods | `wasi:sockets/types`' `tcp-socket` (`TcpSocket.create(...)`, `await sock.connect(...)`) - `WebSocketServer` itself is built entirely on this pattern |
| `list<u8>` parameters/returns | Pervasive - WS frame bytes, `wasi:io` stream reads, `getRandomBytes`, etc. |
| A plain top-level `func` (not `async`) returning `result<T, error-code-like-enum>` | `TcpSocket.create`/`.bind()`/`.getLocalAddress()` - "throws on err, returns `T` directly on ok" convention, confirmed against `tests/wit/sockprobe/probe.js` |
| A `borrow<T>` parameter on an *imported* function | **Tested, and it fails** - see "Borrow-forwarding finding" below. This row was originally "untested but should work by inspection"; that read turned out to be wrong. |

The one combination that didn't have dedicated test coverage - and is the
actual crux of this feasibility question - is:

> A resource received as a parameter of an **exported** function (`request`,
> from `handle(request)`) gets passed as a **borrowed argument to an
> imported** function (`claim(request)`).

### Why this looked like it should already work (turned out wrong)

The original reasoning, from reading `crates/runtime/src/call.rs`'s
`pop_borrow`/`pop_own`:

```rust
fn pop_borrow(&mut self, ty: Resource) -> u32 {
    let persistent = self.pop_persistent();
    with_ctx(|ctx| {
        let val = persistent.restore(ctx).unwrap();
        if ty.new().is_some() {
            exported_resource_to_handle(ctx, ty, &val)
        } else {
            imported_resource_to_handle(&val)
        }
    })
}
```

This *is* the lowering path used for every borrowed-resource argument to an
import call, regardless of which WIT interface declared the resource type -
generic, type-agnostic code, not written for any one interface. What this
reading missed: genericity at the dwarf-runtime level doesn't guarantee
correctness at the canonical-ABI level. `imported_resource_to_handle` simply
returns the resource's existing raw handle number; it does not itself create
or reclaim any component-model "borrow lend" scoped to *this specific* import
call. Whatever `wit-dylib`'s generated canon-lower glue does around that
raw handle (the actual code embedded in the component at build time -
external to dwarf's own crates, from the `wit-dylib`/`wasm-tools` git
dependency) is what ultimately failed the check - see the finding below.
This is exactly the class of thing "reasoning about the code" can't catch:
the bug isn't in the function signature or the type-agnostic dispatch, it's
in what a *different* codebase's generated glue does with the value.

### The one real open question: resource/task lifetime

This is the actual risk, and it's **not** something reasoning about the code
can settle - it needs an empirical test. trail-main-cli's own notes on
`periapisis:host/hijack` mention it "needs `--persistent` trail mode (a
spawned background read/write loop only keeps running past `handle()`
returning under persistent Stores)". That's exactly the shape of question
this session already had to answer empirically once before, for `--sync`
runtime error semantics and for `setTimeout`'s task-lifetime caveat
(documented in `generate_timers`): component-model-async ties a lot of
in-flight state to the *task* that created it, and "does the borrowed
`request` (and the `hijacked-connection` obtained from it) stay valid for a
read/write loop that outlives the `handle()` call that received it" is
precisely the kind of question that has bitten this codebase before in
subtle, only-reproducible-empirically ways (see: the `cancelRead()` trap
found while hardening WS-1b, or the WASI-0.2-insecure-seed discovery when
scoping the p2-removal work). Reasoning from the WIT alone isn't enough
confidence to call this "done"; it needs a real, hand-tested repro.

## Recommended verification step (small, not full implementation)

A synthetic repro matching the existing `tests/wit/sockprobe` pattern (a
hand-rolled minimal resource + import, not needing the real `periapisis:host`
package): define a tiny WIT world with an exported function taking a
`borrow<some-resource>` parameter and an imported function taking the same
resource type as `borrow<...>`, confirm the guest can pass the received
value straight through, and - the actual open question - confirm it still
works if the guest holds onto it (e.g. via a stored reference used in a
`.then()` continuation) past the point where the exporting function's own
`Promise` has resolved, approximating the "outlives `handle()` under a
persistent Store" shape without needing a real trail/perigeos deployment to
test it. This is a half-day-scale task, not a new feature.

## Verification result: no trap, just the existing fire-and-forget caveat

Ran as `tests/task_lifetime.rs` on `feature/host-hijack-lifetime-verification`,
using `wasi:sockets`' `tcp-socket` as a stand-in resource (no dependency on the
real, unavailable-here `periapisis:host/hijack` package - the concern being
tested doesn't depend on which resource type is involved). Three cases, all
behaving identically:

1. **A bare unawaited continuation with no resource at all** (just an
   unawaited `setTimeout`): the exporting call (`run()`) completes cleanly,
   and the continuation is simply never resumed - `check()` afterward shows
   its side effect never ran. This is the already-documented
   `setTimeout`/console fire-and-forget caveat (`generate_timers`'s doc
   comment).
2. **A resource with a still-pending async operation on it**
   (`await sock.connect(...)`, never awaited by `run()` itself) when the
   exporting call settles: behaves identically to case 1 - `run()` completes
   cleanly, and the continuation (including its `connect()`/`catch`) is
   simply never resumed. No trap.
3. **A resource merely held across the boundary, with no pending async
   operation on it at all** - the background continuation only calls a
   *synchronous* method (`sock.getLocalAddress()`) on the resource, after its
   own internal timer: also behaves identically - no trap, continuation
   abandoned.

**Correction to an earlier version of this section:** this note previously
reported that cases 2 and 3 *trapped* the guest. That was wrong, and the
error was in the test script, not dwarf: it referenced `TcpSocket` as a bare
global (`const sock = TcpSocket.create(...)`) instead of importing it from
the WIT-generated module, per the same pattern already used by
`tests/wit/sockprobe/probe.js` (`import { TcpSocket } from
"wasi:sockets/types@0.3.0"`). Without the import, `TcpSocket` is undefined,
so `run()` threw a plain, uncaught `ReferenceError` - and since this test
world's `run()` is declared `-> string` (no error type in its WIT signature
at all), *any* exception it throws has no way to be represented, so lowering
it panics (`describe_lower_failure`, `crates/runtime/src/bindings.rs`) -
correct, expected behavior for a mis-shaped export, not a resource/task
lifetime bug. This was only caught by actually capturing and reading the
guest's own stderr (`AsyncComponentInstance::stderr_bytes()`) - the original
conclusion was inferred from wasm-backtrace *symbol names* alone (both cases
happened to panic from the same `then_cb`/`catch_cb` code path), which look
identical whether the export's own code threw or whether something else did.
Lesson for next time doing this kind of empirical verification: capture and
print the guest's stderr before drawing conclusions from a trap's backtrace
shape - the backtrace tells you *where* the panic macro fired, not *why*.

**What this means for host-hijack:** no *new* risk found from this step
alone. The remaining open question - does `--persistent` trail mode keep the
whole task alive across `handle()`'s own return? - is answered definitively
below.

## Root cause: it's a host-side API choice, not a wasmtime limit

The one real open question left after the corrected verification above was
*why* the background continuation gets abandoned at all, and whether that's
fixable. `tests/task_persistence_rootcause.rs` answers it directly by
reading wasmtime's own source and testing its lower-level API.

**wasmtime's own documentation already says the abandonment isn't
mandatory.** `Func::call_concurrent`'s doc comment
(`wasmtime-46.0.1/src/runtime/component/concurrent/func.rs`) states: "If the
future created by this function is dropped it does not cancel the
in-progress execution of the wasm task... the task will still progress and
invoke callbacks and such until completion" - provided the store's
`run_concurrent` event loop keeps running. `Func::call_async` - what dwarf's
own test harness uses, and what a "one call in, one call out" embedding
naturally reaches for - is sugar for `run_concurrent_trap_on_idle`
(`concurrent.rs`), which opens a **fresh** `run_concurrent` scope for each
individual call and returns as soon as *that specific call's* own future
resolves (`task.return`), independent of whether other tasks still have
futures outstanding in the shared `ConcurrentState::futures` queue. Nothing
then keeps polling those leftover futures until some *later* call happens to
reopen a scope over the same store - and even then (see the "not-run"
results in `task_lifetime.rs`), a scope that closes before the timer/subtask
resolves never revives it.

**Confirmed by direct experiment:** `tests/task_persistence_rootcause.rs`
calls `run()` and `check()` via `Func::call_concurrent` inside a single,
continuous `store.run_concurrent(async |accessor| { ... })` scope, with the
same `tokio::time::sleep(300ms)` between them used in the earlier
(`call_async`-based) tests - except this time the sleep happens *inside* the
scope instead of between two separate `call_async` invocations. Result: the
background continuation completes correctly in both cases -
`test_continuation_survives_within_one_continuous_run_concurrent_scope`
(bare timer) and
`test_resource_holding_continuation_survives_within_one_continuous_scope`
(a real `TcpSocket.connect()`, matching host-hijack's actual shape). Same
guest code, same dwarf-built component, same dwarf-runtime crate - the only
difference is which wasmtime API the *host* uses to drive the call.

**Verdict: not a fundamental constraint, and not a dwarf-runtime bug or
design flaw either.** dwarf's guest-side code (`crates/runtime`, including
the single global `TaskState` slot in `task.rs` this investigation
originally suspected) has no part in this - the callback for the old task's
pending subtask is correctly restored and delivered via the existing
`context_set`/`context_get` handle mechanism regardless of which later call
happens to be in flight, *as long as the host keeps polling the store's
event loop long enough to deliver it*. The constraint lives entirely on the
**embedding host's** side, in the choice between:

- `Func::call_async` per call (a fresh, narrow `run_concurrent` scope each
  time) - background continuations that outlive one call are abandoned the
  moment that call's own scope closes, and
- `Func::call_concurrent` inside one long-lived `run_concurrent` scope
  (spanning the whole server/component lifetime, not one request) -
  background continuations survive exactly as wasmtime's own docs promise.

This gives trail's own documented "`--persistent` trail mode" requirement
concrete technical grounding instead of being an opaque necessity: it is, in
all likelihood, precisely trail keeping a persistent `run_concurrent`/
`Accessor`-driven event loop open across the component's whole serving
lifetime (using `call_concurrent` per incoming request) rather than doing a
fresh `call_async` per request. **This specific question (task/resource
lifetime past `handle()`'s return) is resolved: no dwarf-runtime change is
needed for it.** It is not, on its own, enough to call host-hijack viable -
see the next section for why.

## Borrow-forwarding finding: a real, confirmed, general gap

The task-lifetime work above answers *one* of the two things this note
flagged as open. The other - the `pop_borrow`/`imported_resource_to_handle`
reasoning in "Why this looked like it should already work" - was tested
directly and **fails**. Four differential tests isolate it precisely:

| Test | How the resource was obtained | How it's used | Result |
|---|---|---|---|
| `tests/borrow_passthrough.rs` | Export param, `borrow<probe-thing>` | Passed to an import as `borrow<probe-thing>` | **Traps**: "borrow handles still remain at the end of the call" |
| `tests/own_then_borrow.rs` | Export param, `own<probe-thing>` | Passed to an import as `borrow<probe-thing>` | **Traps**, identically |
| `tests/own_then_own.rs` | Export param, `own<probe-thing>` | Passed to an import as `own<probe-thing>` | Succeeds |
| `tests/internal_borrow.rs` | A *different* import call (`create-thing() -> probe-thing`), no export involved at all | A **method call** on it (`thing.ping()` - `self: borrow<probe-thing>` implicit) | **Traps**, identically |

All four use a minimal host-defined resource (`probe-thing`, registered via
`Linker::instance(...).resource(...)`) instead of `wasi:http/types`' actual
`request` - the concern doesn't depend on which interface declares the
resource type, and this avoids needing the real (unavailable-here)
`periapisis:host/hijack`/`wasi:http` machinery.

**What this isolates:** the trap has nothing to do with the export/import
boundary at all - `tests/internal_borrow.rs` reproduces it with no export
parameter of any resource type in the picture whatsoever, and it doesn't
matter whether the borrow argument is a named parameter on a freestanding
function or a method's implicit `self`. The one true variable is **whether
the resource being lowered as a `borrow<T>` was just constructed in *this
exact* call** (`own_then_own.rs`, succeeds) **or was obtained earlier and is
now being reused** (all three other tests, traps) - regardless of whether
"earlier" means an export parameter or a prior, separate import call. This
is not host-hijack-specific: grepping dwarf's entire existing test suite
turns up **zero** tests that call any import function (or method) with a
`borrow<T>` parameter for a host-defined resource obtained earlier
(`wasi:sockets`, `wasi:filesystem`, `wasi:io`, and `wasi:http`'s own
vendored WIT all declare several such functions - `is-same-object`,
`network-error-code`, `splice`, etc. - but none of dwarf's tests exercise
any of them). This is a previously-undiscovered, general gap that any of
dwarf's *existing* polyfills could in principle hit too, if they ever needed
this shape - it just happens that none currently do.

**Root cause: narrowed further, but not conclusively pinned down; only
partially in dwarf's own code.** `crates/runtime/src/call.rs`'s `pop_borrow`
(dwarf's own code) returns the resource's existing raw handle number for the
imported-resource branch; it does not itself create or reclaim any scoped
"borrow lend" for this specific call - and `wit-dylib-ffi`'s `Interpreter`
trait exposes no separate hook for that either (only `pop_borrow`/`pop_own`/
`push_borrow`/`push_own`), so if such a step is needed, it isn't something
`pop_borrow`'s implementation could add on its own regardless. The actual
canonical-ABI lowering around that raw handle is generated by `wit-dylib`'s
bindgen (`crates/wit-dylib/src/bindgen.rs` in the `wit-dylib`/`wasm-tools`
git dependency - see `Cargo.lock`'s `wit-dylib-ffi` source, external to
dwarf's own crates) and embedded into the component at build time; the
trap's backtrace bottoms out in
`wit_dylib_ffi::types::ImportFunction::call_import_sync`, in that generated
glue, not in dwarf-runtime's own code.

Notably, `wit-dylib`'s own upstream test suite
(`crates/wit-dylib/test-programs/src/bin/resources_caller.rs`) contains a
test of the *exact same shape* - call an import constructor for `own`
resource, then call a method (`[method]a.frob`) on it, passing
`Val::Borrow(handle.borrow())` - which is presumably a passing reference
test upstream, suggesting the underlying mechanism is *intended* to support
exactly this. Confirming whether it actually passes (which would point the
bug squarely at something dwarf/wit-dylib-ffi-integration-specific rather
than a genuine upstream gap) needs running that test suite directly, which
needs a `wasi-sdk` toolchain layout this environment's ad-hoc install didn't
match (`bin/wasm32-wasip3-clang` vs. the plain `bin/clang` their build
script expects) - blocked on toolchain setup, not investigated further here.
Pinning the exact mechanism needs either resolving that toolchain mismatch
to run wit-dylib's own tests, or tracing the generated wasm bytecode
directly - both out of scope for this verification pass, which established
*whether* this works and narrowed *where* the difference must lie, not
conclusively *why* it differs.

**What this means for host-hijack:** `claim(request: borrow<request>)` is
*exactly* this shape - `request` arrives via `handle(request)` (as either
`own` or `borrow` depending on `wasi:http`'s actual signature, per the tests
above it doesn't matter which) and gets forwarded into `claim()` as
`borrow<request>`. **This traps as things stand today.** Unlike the
task-lifetime question, this is not resolved by a host-embedding choice -
it's either a dwarf-runtime bug, a `wit-dylib` upstream bug, or a
genuine canonical-ABI subtlety this shape runs into; some of that needs
fixing (in dwarf's own `call.rs`, or upstream in `wit-dylib`/`wasm-tools`,
or both) before `claim()` can be called this way at all.

**Recommendation, updated again:** don't start real host-hijack bindgen work
yet - the task-lifetime gate is clear, but this borrow-forwarding gate is
not. Before any implementation: either (a) root-cause and fix this in
whichever codebase actually owns it (dwarf's `call.rs`, or an upstream
`wit-dylib`/`wasm-tools` issue/PR), or (b) confirm whether `claim` could be
redesigned to take `own<request>` instead of `borrow<request>` (sidestepping
the broken path entirely, since `own→own` was confirmed to work) - a
protocol-level question for whoever owns the `periapisis:host/hijack`
interface, not a dwarf question. Either path is real work, not scoping.

## Rough integration shape

Not a rewrite - **most of WS-1/WS-1b's existing code is directly reusable**:

- `FrameParser`, `buildFrame`/`buildCloseFrame`, `isValidUTF8`,
  `isValidStatusCode`, `WebSocketConnection` (crates/core/polyfills/
  websocket-server.js) are all transport-agnostic already - they operate on
  plain `Uint8Array`s in and out, never touching `wasi:sockets` directly.
  These would carry over essentially unchanged.
- What *would* change: the transport layer (`HijackedConnection.read(max)`/
  `.write(data)` instead of `wasi:sockets`' stream read/`writeAll`) and the
  handshake (reading `Sec-WebSocket-Key`/`Upgrade` etc. off the
  already-parsed `wasi:http` `request`'s own headers accessor, instead of
  this module's hand-rolled `readHttpHeaders`/`parseHttpHeaders` raw-byte
  parser - which also means the entire hostile-input-hardening pass from
  WS-1b, header/body size limits included, becomes wasi:http's problem, not
  dwarf's, for anything going through hijack).
- No raw-socket accept loop at all - the host's own `wasi:http` request
  dispatch replaces `WebSocketServer.listen()`'s `TcpSocket`/accept-stream
  loop entirely.

Effort estimate, **conditional on trail's `--persistent` mode using
`call_concurrent` + a persistent `run_concurrent` scope, as "Root cause"
above implies it must** (worth a quick confirmation on trail's side, but no
longer a blocking unknown):
small-to-medium. The RFC 6455 protocol logic (the hard, security-sensitive
part, per WS-1b) is done and reusable as-is; the new work is a thinner
transport/handshake adapter plus whatever ergonomic wrapper (if any) is
wanted for parity with today's `WebSocketServer` class shape.

## What this note is *not* saying

This isn't a recommendation to build it now, or to deprecate WS-1/WS-1b -
WS-1/WS-1b ships and stays valuable as a guest-side fallback for hosts
without the hijack primitive; not every host will have
`periapisis:host/hijack` available. Nor is it a claim that host-hijack is
dead - the borrow-forwarding trap is real but narrow (isolated to one
specific ABI shape) and has at least one plausible way around it (redesign
`claim` to take `own<request>`) even before any dwarf/wit-dylib fix lands.
But it is a real, confirmed blocker as things stand today - this is not yet
a green light to build, and is a step *less* ready than "no known blocker"
would have been. The task-lifetime half of the original risk is resolved
cleanly; the borrow-forwarding half is a genuine open problem that needs
real engineering (in dwarf, upstream, or both) before implementation starts.
