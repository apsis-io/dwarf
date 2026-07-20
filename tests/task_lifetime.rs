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
//! the concern being tested (does a resource-holding continuation survive
//! past its originating export's return) doesn't depend on which specific
//! resource type is involved.
//!
//! **Finding: it's worse than "silently cancelled".** A bare, unawaited
//! continuation with no resource involved (`test_timer_only_continuation_is_silently_cancelled`)
//! is simply never resumed once its exporting task settles - the same
//! already-documented behavior as `setTimeout`/console fire-and-forget writes
//! (see `generate_timers`'s doc comment). But the moment a continuation
//! *holds a resource* across that same boundary, the exporting call itself
//! traps the guest instead - and this happens even when the resource is
//! never used with an in-flight async operation at settle time
//! (`test_holding_a_resource_across_task_settlement_traps_even_without_pending_async_use`).
//! The full `wasm backtrace` for both trapping cases bottoms out in
//! `dwarf_runtime::bindings::build_async_exports`'s `then_cb`/`catch_cb` -
//! the exact machinery that lowers the *exporting* async function's own
//! settled promise into its declared WIT result - even though that
//! function's literal return value (a plain string) should lower trivially.
//! That strongly suggests the panic isn't about the exporting call's own
//! declared return at all, but some form of re-entrancy into that same
//! lowering path triggered by tearing down a still-referenced resource when
//! the task settles. Pinning the exact trigger (e.g. an async resource
//! destructor's completion looping back through the export-result boundary)
//! would need tracing dwarf-runtime's resource-table/task teardown wiring -
//! out of scope for this synthetic repro, which only needed to establish
//! *whether* this is safe, not exactly *why* it isn't.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use wasmtime::component::Val;

use common::TestCase;

/// A bare unawaited continuation with nothing resource-like in it: cancelled
/// silently (no trap) the moment its exporting task settles, matching the
/// already-documented setTimeout/console fire-and-forget caveat.
#[tokio::test]
async fn test_timer_only_continuation_is_silently_cancelled() {
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
        "a bare unawaited continuation is expected to be cancelled when its \
         exporting task settles - if this ever starts failing (the \
         continuation completes), that's a real change in dwarf's task \
         lifetime semantics worth its own investigation, not a bug in this \
         test"
    );
}

/// The shape host-hijack actually needs: a resource with a still-pending
/// async operation on it (`sock.connect()`) when the exporting call settles.
/// Traps the guest rather than being silently cancelled.
#[tokio::test]
async fn test_holding_a_resource_with_a_pending_async_operation_traps() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/task-lifetime");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("task-lifetime-test")
        .script(
            r#"
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

    let mut run_results = [Val::String(String::new())];
    let run_result = run_func
        .call_async(&mut *store, &[], &mut run_results)
        .await;

    assert!(
        run_result.is_err(),
        "expected run() to trap the guest when it settles while a background \
         continuation still has an in-flight async operation on a resource \
         it holds - if this ever starts returning Ok, that's a real (and \
         very welcome) change in dwarf's resource/task lifetime semantics, \
         worth revisiting docs/design-notes/host-hijack-feasibility.md over"
    );
    let message = format!("{:#}", run_result.unwrap_err());
    assert!(
        message.contains("unreachable"),
        "expected a wasm `unreachable` trap specifically, got: {message}"
    );
}

/// Narrows the finding above down further: does the trap require an
/// in-flight *async* resource operation specifically, or does merely
/// *holding a reference* to a resource across the task-settlement boundary
/// already trigger it, even with a plain synchronous method call and no
/// async resource operation ever in flight? Here the background
/// continuation only touches a resource's synchronous method after a timer
/// elapses.
#[tokio::test]
async fn test_holding_a_resource_across_task_settlement_traps_even_without_pending_async_use() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/task-lifetime");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("task-lifetime-test")
        .script(
            r#"
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

    let mut run_results = [Val::String(String::new())];
    let run_result = run_func
        .call_async(&mut *store, &[], &mut run_results)
        .await;

    assert!(
        run_result.is_err(),
        "expected run() to trap even though the resource has no pending \
         async operation at settle time - this is the key finding: it's not \
         about abandoning an in-flight async op, merely holding a resource \
         reference across the task-settlement boundary is already unsafe. \
         If this ever starts returning Ok, that's a real (and very welcome) \
         change in dwarf's resource/task lifetime semantics, worth revisiting \
         docs/design-notes/host-hijack-feasibility.md over"
    );
    let message = format!("{:#}", run_result.unwrap_err());
    assert!(
        message.contains("unreachable"),
        "expected a wasm `unreachable` trap specifically, got: {message}"
    );
}
