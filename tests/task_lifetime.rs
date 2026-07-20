//! Empirical verification for the host-hijack feasibility note
//! (docs/design-notes/host-hijack-feasibility.md): does a resource received
//! by an exported async function, held and used via an *unawaited*
//! background continuation, keep working after the exporting call has
//! already returned (settling its own task)?
//!
//! This is the shape `periapisis:host/hijack`'s intended usage needs: a
//! guest's `wasi:http/incoming-handler#handle()` calls `claim(request)` to
//! get a `hijacked-connection`, then reads/writes on it in a loop meant to
//! outlive `handle()`'s own return - trail's own notes say this requires
//! `--persistent` trail mode specifically because it does NOT work for free.
//!
//! Rather than needing the real (unavailable-here) `periapisis:host/hijack`
//! interface, this reproduces the general mechanism with an already-real,
//! already-supported resource (`wasi:sockets`' `tcp-socket`) and a plain
//! unawaited async IIFE standing in for "the background read/write loop" -
//! the concern being tested doesn't depend on which specific resource type
//! is involved.
//!
//! **Finding: no new blocker, no trap - it's the already-documented
//! setTimeout/fire-and-forget caveat, generalized.** All three cases below
//! behave identically: the exporting call itself always completes cleanly
//! (never traps), and a background continuation with a still-pending async
//! operation at settle time (a socket `connect()`, or merely a `setTimeout`)
//! is silently abandoned rather than resumed later - matching the existing
//! documented behavior for `setTimeout`/console fire-and-forget writes (see
//! `generate_timers`'s doc comment), just not previously confirmed for
//! resource-backed async import calls specifically. There's nothing
//! resource-specific here: holding a resource reference across the boundary
//! is not itself unsafe, it just doesn't extend that continuation's
//! lifetime any more than a plain timer would. (An earlier version of this
//! file reported a "trap" for the resource cases - that was a bug in the
//! test script itself, referencing `TcpSocket` as a bare global instead of
//! importing it from `"wasi:sockets/types@0.3.0"` as the WIT bindings
//! actually require, which threw an uncaught `ReferenceError` that a
//! `-> string`-only export (no error channel at all) can never represent,
//! panicking on lowering. Caught only once the guest's own stderr was
//! captured and inspected directly - inferring the cause from wasm-backtrace
//! symbol names alone was the mistake.)
//!
//! What this means for host-hijack: dwarf's generic codegen has no
//! resource/task-lifetime blocker for the interface shape itself. The real
//! open question is exactly the one trail already flagged - whether
//! `--persistent` trail mode keeps the whole task (not just the JS
//! continuation) alive across `handle()`'s own return - which is a host-side
//! question this synthetic repro can't answer, not a new dwarf-side risk.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use wasmtime::component::Val;

use common::TestCase;

/// A bare unawaited continuation with nothing resource-like in it: the
/// exporting call completes cleanly, and the continuation is simply never
/// resumed - the already-documented setTimeout/console fire-and-forget
/// caveat.
#[tokio::test]
async fn test_timer_only_continuation_is_silently_abandoned() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/task-lifetime");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("task-lifetime-test")
        .script(
            r#"
            let sideEffect = "not-run";

            export async function run() {
                (async () => {
                    await new Promise((resolve) => setTimeout(resolve, 20));
                    sideEffect = "completed";
                })();

                return "run-returned";
            }

            export async function check() {
                return sideEffect;
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build the task-lifetime component");

    let (instance, store) = inst.parts();
    let run_func = instance.get_func(&mut *store, "run").expect("run export");
    let check_func = instance
        .get_func(&mut *store, "check")
        .expect("check export");

    let mut run_results = [Val::String(String::new())];
    run_func
        .call_async(&mut *store, &[], &mut run_results)
        .await
        .expect("a plain timer continuation with no resource should not trap");
    assert_eq!(run_results[0], Val::String("run-returned".into()));

    // Give the guest's own event loop every chance to advance the
    // background continuation, if the task model allowed it to.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut check_results = [Val::String(String::new())];
    check_func
        .call_async(&mut *store, &[], &mut check_results)
        .await
        .expect("check should complete without trapping");
    assert_eq!(
        check_results[0],
        Val::String("not-run".into()),
        "a bare unawaited continuation is expected to be abandoned when its \
         exporting task settles - if this ever starts failing (the \
         continuation completes), that's a real change in dwarf's task \
         lifetime semantics worth its own investigation, not a bug in this \
         test"
    );
}

/// The shape host-hijack actually needs: a resource with a still-pending
/// async operation on it (`sock.connect()`) when the exporting call settles.
/// Behaves identically to the timer-only case above - no trap, but the
/// continuation is abandoned rather than resumed.
#[tokio::test]
async fn test_holding_a_resource_with_a_pending_async_operation_is_also_silently_abandoned() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/task-lifetime");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("task-lifetime-test")
        .script(
            r#"
            import { TcpSocket } from "wasi:sockets/types@0.3.0";

            let sideEffect = "not-run";

            export async function run() {
                const sock = TcpSocket.create("ipv4");

                // Fire-and-forget: deliberately NOT awaited by run() itself -
                // stands in for a background read/write loop over a resource
                // (e.g. host-hijack's hijacked-connection) meant to keep
                // running after the exporting call (handle()) has returned.
                (async () => {
                    try {
                        // Nothing listens on port 1 - connect() is expected
                        // to reject. What's under test is whether run()
                        // survives this being in flight when it settles, not
                        // the specific outcome of connect() itself.
                        await sock.connect({ tag: "ipv4", val: { port: 1, address: [127, 0, 0, 1] } });
                    } catch (e) {
                        // Expected.
                    }
                    sideEffect = "completed";
                })();

                return "run-returned";
            }

            export async function check() {
                return sideEffect;
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build the task-lifetime component");

    let (instance, store) = inst.parts();
    let run_func = instance.get_func(&mut *store, "run").expect("run export");
    let check_func = instance
        .get_func(&mut *store, "check")
        .expect("check export");

    let mut run_results = [Val::String(String::new())];
    run_func
        .call_async(&mut *store, &[], &mut run_results)
        .await
        .expect(
            "run() should not trap while a background continuation holds a resource with a \
             pending async op on it",
        );
    assert_eq!(run_results[0], Val::String("run-returned".into()));

    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut check_results = [Val::String(String::new())];
    check_func
        .call_async(&mut *store, &[], &mut check_results)
        .await
        .expect("check should complete without trapping");
    assert_eq!(
        check_results[0],
        Val::String("not-run".into()),
        "a resource-holding continuation with a pending async op is expected \
         to be abandoned exactly like the plain-timer case above, not \
         resumed later and not trapped - if this ever starts failing, \
         that's a real change in dwarf's task lifetime semantics worth its \
         own investigation, not a bug in this test"
    );
}

/// Narrows the finding above down further: does abandonment require an
/// in-flight *async* resource operation specifically, or does merely
/// *holding a reference* to a resource across the task-settlement boundary
/// behave the same way even with a plain synchronous method call and no
/// async resource operation ever in flight? Here the background
/// continuation only touches a resource's synchronous method after a timer
/// elapses.
#[tokio::test]
async fn test_holding_a_resource_across_task_settlement_does_not_trap_either() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/task-lifetime");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("task-lifetime-test")
        .script(
            r#"
            import { TcpSocket } from "wasi:sockets/types@0.3.0";

            let sideEffect = "not-run";

            export async function run() {
                const sock = TcpSocket.create("ipv4");
                sock.bind({ tag: "ipv4", val: { port: 0, address: [127, 0, 0, 1] } });

                (async () => {
                    await new Promise((resolve) => setTimeout(resolve, 20));
                    // A synchronous call on the resource, well after run()
                    // would have settled - no async resource operation is
                    // ever in flight across that boundary.
                    const addr = sock.getLocalAddress();
                    sideEffect = "completed:" + addr.val.port;
                })();

                return "run-returned";
            }

            export async function check() {
                return sideEffect;
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build the task-lifetime component");

    let (instance, store) = inst.parts();
    let run_func = instance.get_func(&mut *store, "run").expect("run export");
    let check_func = instance
        .get_func(&mut *store, "check")
        .expect("check export");

    let mut run_results = [Val::String(String::new())];
    run_func
        .call_async(&mut *store, &[], &mut run_results)
        .await
        .expect("run() should not trap merely for holding a resource across task settlement");
    assert_eq!(run_results[0], Val::String("run-returned".into()));

    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut check_results = [Val::String(String::new())];
    check_func
        .call_async(&mut *store, &[], &mut check_results)
        .await
        .expect("check should complete without trapping");
    assert_eq!(
        check_results[0],
        Val::String("not-run".into()),
        "holding a resource with no pending async op on it is expected to be \
         abandoned exactly like the other two cases - not resumed, and no \
         trap - if this ever starts failing, that's a real change in \
         dwarf's task lifetime semantics worth its own investigation, not a \
         bug in this test"
    );
}
