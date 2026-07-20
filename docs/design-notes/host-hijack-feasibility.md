# Feasibility: dwarf-side support for `periapisis:host/hijack`

Status: **scoping + verification + root cause done - the open question is
answered, and it's good news.** Written to de-risk a possible post-demo-7
migration away from `WebSocketServer`'s raw-socket HTTP+WS router
(`crates/core/polyfills/websocket-server.js`, WS-1/WS-1b) toward a
host-hijack-based single-port answer, per the architecture fork
`trail-main-cli` flagged during WS-1b (`periapisis:host/hijack`, ADR-0046,
live in the periapsis repo). The empirical verification step this note
originally recommended has been run (`tests/task_lifetime.rs`): holding a
resource across the exporting call's own task settlement does **not** trap
the guest, it just behaves like the already-documented `setTimeout`
fire-and-forget caveat (silently abandoned, not resumed). A follow-on root
cause investigation (`tests/task_persistence_rootcause.rs`) then answered
the one remaining real question - **is the abandonment a fundamental
wasmtime/component-model constraint, or something a host can avoid?** It's
the latter: keeping wasmtime's own concurrent event loop open across calls
(via `Func::call_concurrent` + a persistent `run_concurrent` scope, instead
of a fresh `Func::call_async` per call) lets the background continuation
survive and complete, confirmed for both a plain timer and a real
resource-backed async op. See "Root cause: it's a host-side API choice, not
a wasmtime limit" below.

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
| A `borrow<T>` parameter on an *imported* function | Untested in isolation, but see below - the underlying mechanism (`pop_borrow`) is shared, type-agnostic machinery, not something built specifically for any one interface |

The one combination that doesn't have dedicated test coverage yet - and is
the actual crux of this feasibility question - is:

> A resource received as a parameter of an **exported** function (`request`,
> from `handle(request)`) gets passed as a **borrowed argument to an
> imported** function (`claim(request)`).

### Why this should already work

Confirmed by reading `crates/runtime/src/call.rs`'s `pop_borrow`/`pop_own`:

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

This is the lowering path used for *every* borrowed-resource argument to an
import call, regardless of which WIT interface declared the resource type or
where the JS-side value being lowered originally came from. `request` (a
type from `wasi:http/types`, an *imported* interface from the guest's
perspective even though the value arrives via an export call) would resolve
through the `imported_resource_to_handle` branch - the same code path
already exercised by, e.g., passing a `TcpSocket` instance around. Lifting
`request` on the way *into* `handle()` and lowering it back out to `claim()`
are both existing, type-agnostic, already-tested mechanisms; there is no
`hijack`-specific (or even `wasi:http`-specific) code that would need to be
written for this to type-check and round-trip correctly at the ABI level.

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
fresh `call_async` per request. If that is indeed what `--persistent` mode
does (trail's own docs already imply as much), **host-hijack's background
read/write loop pattern is fully viable with dwarf's current runtime as-is -
no dwarf-runtime change is needed.**

**Recommendation, updated:** still don't build the host-hijack integration
until it's actually prioritized post-demo - this remains scoping, not a
build decision. But the blocking uncertainty is gone: when the work is
picked up, the next step is simply confirming (or, if needed, arranging)
that trail's host embedding calls into the guest via `call_concurrent` +
a persistent `run_concurrent` scope for any component using this pattern -
not a dwarf-side investigation or fix. No "fix sketch" is included here
because there is nothing in dwarf's own runtime crate to fix; the lever is
entirely in the host's own wasmtime embedding code.

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
per lead's read (which matches the reasoning above), WS-1/WS-1b ships and
stays valuable as a guest-side fallback for hosts without the hijack
primitive; not every host will have `periapisis:host/hijack` available.
Verification and root-cause investigation together found no dwarf-side
blocker and no fundamental wasmtime limit - the task-lifetime risk this note
opened with is resolved to a known, well-understood host-embedding
requirement (`call_concurrent` + a persistent `run_concurrent` scope), not
an open unknown. That still doesn't make this a green light to build - it
remains scoping, prioritized post-demo, same as when this note started.
