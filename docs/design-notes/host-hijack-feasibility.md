# Feasibility: dwarf-side support for `periapisis:host/hijack`

Status: **scoping only** - no implementation, no commitment. Written to de-risk
a possible post-demo-7 migration away from `WebSocketServer`'s raw-socket
HTTP+WS router (`crates/core/polyfills/websocket-server.js`, WS-1/WS-1b) toward
a host-hijack-based single-port answer, per the architecture fork
`trail-main-cli` flagged during WS-1b (`periapisis:host/hijack`, ADR-0046, live
in the periapsis repo).

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

## If verification succeeds: rough integration shape

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

Effort estimate, **conditional on the verification step above succeeding**:
small-to-medium. The RFC 6455 protocol logic (the hard, security-sensitive
part, per WS-1b) is done and reusable as-is; the new work is a thinner
transport/handshake adapter plus whatever ergonomic wrapper (if any) is
wanted for parity with today's `WebSocketServer` class shape.

## What this note is *not* saying

This isn't a recommendation to build it now, or to deprecate WS-1/WS-1b -
per lead's read (which matches the reasoning above), WS-1/WS-1b ships and
stays valuable as a guest-side fallback for hosts without the hijack
primitive; not every host will have `periapisis:host/hijack` available. If
`trail-main-cli`/lead decide to standardize on host-hijack later, this note
is the starting point for scoping that work for real - not a decision to do
so.
