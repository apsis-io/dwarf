//! Integration tests for the setTimeout/setInterval polyfill (always-on,
//! wired to wasi:clocks/monotonic-clock@0.3.x) - see
//! crates/core/src/polyfills.rs's `generate_timers`.
#![cfg(feature = "component-model-async")]

mod common;

use common::{TestCase, wasi_wit_dir};
use wasmtime::component::Val;

#[tokio::test]
async fn test_set_timeout_fires_with_args_and_clear_timeout_cancels() {
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("timers-test")
        .script(
            r#"
            export async function run() {
                const sum = await new Promise((resolve) => {
                    setTimeout((a, b) => resolve(a + b), 10, 3, 4);
                });
                if (sum !== 7) return "FAIL: setTimeout args wrong: " + sum;

                let fired = false;
                const handle = setTimeout(() => { fired = true; }, 10);
                clearTimeout(handle);
                // Wait longer than the cancelled timeout would have taken.
                await new Promise((resolve) => setTimeout(resolve, 30));
                if (fired) return "FAIL: clearTimeout did not cancel";

                return "OK";
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build timers component");

    let result = instance.call1_async("run", &[]).await.unwrap();
    assert_eq!(result, Val::String("OK".into()));
}

#[tokio::test]
async fn test_set_interval_fires_repeatedly_and_clear_interval_stops_it() {
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("timers-test")
        .script(
            r#"
            export async function run() {
                let count = 0;
                const total = await new Promise((resolve) => {
                    const iv = setInterval(() => {
                        count++;
                        if (count >= 3) {
                            clearInterval(iv);
                            resolve(count);
                        }
                    }, 5);
                });
                if (total !== 3) return "FAIL: setInterval count wrong: " + total;

                // Confirm it actually stopped: wait a while longer and check
                // count didn't keep climbing past 3.
                await new Promise((resolve) => setTimeout(resolve, 30));
                if (count !== 3) return "FAIL: clearInterval did not stop it, count=" + count;

                return "OK";
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build timers component");

    let result = instance.call1_async("run", &[]).await.unwrap();
    assert_eq!(result, Val::String("OK".into()));
}

#[tokio::test]
async fn test_set_timeout_requires_monotonic_clock_import() {
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("timers-no-clock")
        .script(
            r#"
            export async function run() {
                try {
                    setTimeout(() => {}, 10);
                    return "FAIL: setTimeout should have thrown";
                } catch (e) {
                    if (!e.message.includes("wasi:clocks/monotonic-clock")) {
                        return "FAIL: wrong setTimeout message: " + e.message;
                    }
                }

                try {
                    setInterval(() => {}, 10);
                    return "FAIL: setInterval should have thrown";
                } catch (e) {
                    if (!e.message.includes("wasi:clocks/monotonic-clock")) {
                        return "FAIL: wrong setInterval message: " + e.message;
                    }
                }

                // clearTimeout/clearInterval must be safe no-ops regardless.
                clearTimeout(123);
                clearInterval(456);

                return "OK";
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build a timers component without a clock import");

    let result = instance.call1_async("run", &[]).await.unwrap();
    assert_eq!(result, Val::String("OK".into()));
}
