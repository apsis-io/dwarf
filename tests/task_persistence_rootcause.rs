//! Root-cause investigation (host-hijack follow-on): is the "unawaited
//! background continuation gets abandoned once its exporting call settles"
//! finding ([[task_lifetime.rs]] / docs/design-notes/host-hijack-feasibility.md)
//! a fundamental wasmtime/component-model constraint, or an artifact of
//! dwarf's test harness (and the simple `Func::call_async` API) not keeping
//! wasmtime's own concurrent event loop alive long enough for the
//! still-pending subtask to be driven forward?
//!
//! wasmtime's own docs on `Func::call_concurrent` state plainly: "If the
//! future created by this function is dropped it does not cancel the
//! in-progress execution of the wasm task... the task will still progress
//! and invoke callbacks and such until completion" - PROVIDED the store's
//! `run_concurrent` scope keeps running. `Func::call_async` (what dwarf's
//! own test harness and presumably most simple embeddings use) is sugar for
//! `run_concurrent_trap_on_idle`, which opens a *fresh* `run_concurrent`
//! scope per call and returns as soon as *that specific* call's own future
//! resolves - independent of whether other tasks still have futures
//! outstanding in the shared `ConcurrentState::futures` queue.
//!
//! This test calls `run()` and `check()` within a SINGLE continuous
//! `run_concurrent` scope (via `Func::call_concurrent`, not `call_async`),
//! with an explicit sleep *inside* that same scope between the two calls -
//! to see whether the background continuation (which the earlier
//! `call_async`-based tests found gets silently abandoned) actually
//! completes when the store's event loop is kept alive continuously,
//! instead of being torn down and reopened between calls.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use wasmtime::component::Val;

use common::TestCase;

#[tokio::test]
async fn test_continuation_survives_within_one_continuous_run_concurrent_scope() {
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

    let (run_result, check_result) = store
        .run_concurrent(async |accessor| {
            let mut run_results = vec![Val::String(String::new())];
            run_func
                .call_concurrent(accessor, &[], &mut run_results)
                .await?;

            // Give the background continuation's 20ms timer every chance to
            // fire, WITHOUT ever leaving this run_concurrent scope (unlike
            // the earlier tests, which called `call_async` twice - each
            // call opening and closing its own separate scope).
            tokio::time::sleep(Duration::from_millis(300)).await;

            let mut check_results = vec![Val::String(String::new())];
            check_func
                .call_concurrent(accessor, &[], &mut check_results)
                .await?;

            anyhow::Ok((run_results[0].clone(), check_results[0].clone()))
        })
        .await
        .expect("run_concurrent scope should not error")
        .expect("both calls should succeed");

    assert_eq!(run_result, Val::String("run-returned".into()));

    eprintln!("check() result after a single continuous run_concurrent scope: {check_result:?}");

    // This is the actual question under test: does keeping ONE continuous
    // run_concurrent scope alive (rather than dwarf's call_async pattern,
    // which opens/closes a fresh scope per call) let the background
    // continuation actually complete? If check_result is "completed", the
    // earlier "abandoned" finding was an artifact of call_async's scope
    // lifetime, not a fundamental constraint - a real, fixable dwarf-runtime
    // opportunity (see docs/design-notes/host-hijack-feasibility.md). If
    // it's still "not-run", the constraint survives even a persistent
    // scope, and lies deeper (e.g. in wasmtime's guest-task/callback
    // scheduling once task.return has fired for that task specifically).
    assert_eq!(
        check_result,
        Val::String("completed".into()),
        "if this fails with 'not-run', the background continuation is STILL \
         abandoned even inside one continuous run_concurrent scope - meaning \
         the constraint is deeper than dwarf's call_async usage alone"
    );
}

/// Same question, but with a real resource (`wasi:sockets`' `tcp-socket`)
/// instead of a plain timer - matching host-hijack's actual shape (a
/// `hijacked-connection` resource read/write loop, not a bare `setTimeout`).
#[tokio::test]
async fn test_resource_holding_continuation_survives_within_one_continuous_scope() {
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

                (async () => {
                    try {
                        await sock.connect({ tag: "ipv4", val: { port: 1, address: [127, 0, 0, 1] } });
                    } catch (e) {
                        // Expected - nothing listens on port 1.
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

    let (run_result, check_result) = store
        .run_concurrent(async |accessor| {
            let mut run_results = vec![Val::String(String::new())];
            run_func
                .call_concurrent(accessor, &[], &mut run_results)
                .await?;

            tokio::time::sleep(Duration::from_millis(300)).await;

            let mut check_results = vec![Val::String(String::new())];
            check_func
                .call_concurrent(accessor, &[], &mut check_results)
                .await?;

            anyhow::Ok((run_results[0].clone(), check_results[0].clone()))
        })
        .await
        .expect("run_concurrent scope should not error")
        .expect("both calls should succeed, no trap");

    assert_eq!(run_result, Val::String("run-returned".into()));
    assert_eq!(
        check_result,
        Val::String("completed".into()),
        "a resource-holding background continuation should survive within a \
         single continuous run_concurrent scope exactly like the plain-timer \
         case - if this fails, resources specifically behave differently \
         from timers under a persistent scope, which would be a real, \
         narrower finding worth its own investigation"
    );
}
