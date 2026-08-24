//! WASI integration tests for dwarf
mod common;

use std::path::PathBuf;

use wasmtime::component::{Component, Val};

use common::{TestCase, engine, wasi_wit_dir};

#[test]
fn test_wasi_random() {
    let mut inst = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("wasi-random")
        .script(
            r#"
            import random from "wasi:random/random@0.2.12";

            export function getRandomU64() { return random.getRandomU64(); }
            export function getRandomBytes(len) { return random.getRandomBytes(len); }
        "#,
        )
        .build()
        .expect("should build wasi-random component");

    let val = inst.call1("get-random-u64", &[]);
    assert!(matches!(val, Val::U64(_)), "Expected u64, got: {:?}", val);

    let val2 = inst.call1("get-random-u64", &[]);
    assert_ne!(val, val2, "Two random u64 calls returned the same value");

    let bytes = inst.call1("get-random-bytes", &[Val::U32(16)]);
    match &bytes {
        Val::List(items) => assert_eq!(items.len(), 16, "Expected 16 random bytes"),
        other => panic!("Expected list, got: {:?}", other),
    }
}

#[test]
fn test_wasi_named_imports() {
    let mut inst = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("wasi-random")
        .script(
            r#"
            import {
                getRandomBytes as randomBytes,
                getRandomU64 as randomU64,
            } from "wasi:random/random@0.2.12";

            export function getRandomU64() { return randomU64(); }
            export function getRandomBytes(len) { return randomBytes(len); }
        "#,
        )
        .build()
        .expect("should build wasi-random component with named imports");

    let val = inst.call1("get-random-u64", &[]);
    assert!(matches!(val, Val::U64(_)), "Expected u64, got: {:?}", val);

    let bytes = inst.call1("get-random-bytes", &[Val::U32(8)]);
    match &bytes {
        Val::List(items) => assert_eq!(items.len(), 8, "Expected 8 random bytes"),
        other => panic!("Expected list, got: {:?}", other),
    }
}

#[test]
fn test_wasi_clocks() {
    let mut inst = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("wasi-clocks")
        .script(
            r#"
            import clock from "wasi:clocks/monotonic-clock@0.2.12";

            export function getTimeNs() { return clock.now(); }
            export function elapsedNs() {
                const start = clock.now();
                // Do some trivial work to burn a tiny bit of time
                let x = 0;
                for (let i = 0; i < 1000; i++) { x += i; }
                return clock.now() - start;
            }
        "#,
        )
        .build()
        .expect("should build wasi-clocks component");

    // Monotonic clock should return a positive timestamp
    let time = inst.call1("get-time-ns", &[]);
    match time {
        Val::U64(ns) => assert!(ns > 0, "Monotonic clock returned 0"),
        other => panic!("Expected u64, got: {:?}", other),
    }

    // Second call should be >= first
    let time2 = inst.call1("get-time-ns", &[]);
    match (&time, &time2) {
        (Val::U64(t1), Val::U64(t2)) => assert!(t2 >= t1, "Clock went backwards: {} -> {}", t1, t2),
        _ => unreachable!(),
    }

    // Elapsed should return a u64
    let elapsed = inst.call1("elapsed-ns", &[]);
    assert!(
        matches!(elapsed, Val::U64(_)),
        "Expected u64, got: {:?}",
        elapsed
    );
}

#[test]
fn test_wasi_environment() {
    let mut inst = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("wasi-environment")
        .env("TEST_KEY", "test_value")
        .script(
            r#"
            import env from "wasi:cli/environment@0.2.12";

            export function getEnvVars() { return env.getEnvironment(); }
        "#,
        )
        .build()
        .expect("should build wasi-environment component");

    let vars = inst.call1("get-env-vars", &[]);
    match &vars {
        Val::List(items) => {
            let found = items.iter().any(|item| {
                matches!(item, Val::Tuple(fields) if
                    fields.len() == 2
                    && fields[0] == Val::String("TEST_KEY".into())
                    && fields[1] == Val::String("test_value".into())
                )
            });
            assert!(
                found,
                "TEST_KEY=test_value not found in env vars: {:?}",
                items
            );
        }
        other => panic!("Expected list, got: {:?}", other),
    }
}

#[test]
fn test_wasi_stdio() {
    let mut inst = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("wasi-stdio")
        .stdin("hello from stdin")
        .script(
            r#"
            import stdin from "wasi:cli/stdin@0.2.12";
            import stdout from "wasi:cli/stdout@0.2.12";

            export function echoStdinToStdout() {
                const input = stdin.getStdin();
                const output = stdout.getStdout();

                while (true) {
                    let chunk;
                    try {
                        chunk = input.blockingRead(4096);
                    } catch (e) {
                        if (e && e.payload && e.payload.tag === "closed") {
                            break;
                        }
                        throw e;
                    }

                    output.blockingWriteAndFlush(chunk);
                }

                return;
            }
        "#,
        )
        .build()
        .expect("should build wasi-stdio component");

    let result = inst.call1("echo-stdin-to-stdout", &[]);
    assert_eq!(result, Val::Result(Ok(None)));
    assert_eq!(inst.stdout_bytes(), b"hello from stdin");
}

/// Regression test: an *owned* imported resource (e.g. `wasi:cli/stdin`'s
/// `get-stdin() -> own<input-stream>`) used to have no callable `drop()` at
/// all — `push_own` (crates/runtime/src/call.rs) built a plain object with no
/// drop registered anywhere, so the host-side resource table entry was held
/// until the whole store was torn down, and calling `.drop()` from JS threw
/// `TypeError: not a function`. Also checks that dropping twice is a safe
/// no-op rather than a double-free of the underlying handle.
#[test]
fn test_wasi_imported_resource_drop() {
    let mut inst = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("wasi-stdin-drop")
        .stdin("hello")
        .script(
            r#"
            import stdin from "wasi:cli/stdin@0.2.12";

            export function testDrop() {
                const input = stdin.getStdin();
                input.drop();
                input.drop();
                return true;
            }
        "#,
        )
        .build()
        .expect("should build wasi-stdin-drop component");

    let result = inst.call1("test-drop", &[]);
    assert_eq!(result, Val::Bool(true));
}

#[tokio::test]
async fn test_wasi_0_3_stdio_example() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/wasi-stdio");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("wasi-stdio")
        .stdin("hello from wasi 0.3")
        .script_named("echo.ts", include_str!("../examples/wasi-stdio/echo.ts"))
        .build_async()
        .await
        .expect("should build WASI 0.3 stdio example");

    {
        let (instance, store) = inst.parts();
        let iface_idx = instance
            .get_export_index(&mut *store, None, "wasi:cli/run@0.3.0")
            .expect("wasi:cli/run export not found");
        let run_idx = instance
            .get_export_index(&mut *store, Some(&iface_idx), "run")
            .expect("run export not found");
        let run = instance.get_func(&mut *store, run_idx).unwrap();

        let mut results = [Val::Bool(false)];
        run.call_async(&mut *store, &[], &mut results)
            .await
            .expect("WASI 0.3 stdio run should succeed");
        assert_eq!(results[0], Val::Result(Ok(None)));
    }

    assert_eq!(inst.stdout_bytes(), b"hello from wasi 0.3");
}

/// Regression test for a resource-identity bug: `wasi:http/types` re-exports
/// `wasi:io/streams`' `input-stream`/`output-stream`, `wasi:io/poll`'s
/// `pollable`, and `wasi:io/error`'s `error` (aliased `io-error`) via `use`.
/// componentize's WASI-import stub used to give each occurrence of these a
/// fresh, unrelated host resource type, so once `wasmtime_wasi::p2::
/// add_to_linker_async` registered the *real* type under `wasi:io/streams`
/// alone, `wasi:http/types`' still-stubbed alias mismatched it and
/// instantiation failed with "mismatched resource types" during Wizer init
/// — before any JS ever ran. This world (`include wasi:http/proxy@0.2.12`)
/// is exactly that shape; not calling into the built component here (dwarf
/// itself never links real wasi:http), just proving componentize succeeds.
#[test]
fn test_wasi_http_proxy_resource_aliasing() {
    let opts = dwarf_core::ComponentizeOpts {
        wit_path: &wasi_wit_dir(),
        js_source: r#"
            import { Fields, OutgoingResponse, ResponseOutparam, OutgoingBody } from "wasi:http/types@0.2.12";

            export const incomingHandler = {
                handle(request, responseOut) {
                    const response = new OutgoingResponse(new Fields());
                    response.setStatusCode(200);
                    const body = response.body();
                    ResponseOutparam.set(responseOut, { tag: "ok", val: response });
                    OutgoingBody.finish(body, undefined);
                }
            };
        "#,
        js_path: None,
        module_root: None,
        world_name: Some("wasi-http-proxy"),
        stub_wasi: false,
        auto_vendor: false,
        polyfills: &[],
        disable_gc: false,
        runtime: dwarf_core::Runtime::Default,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("should build tokio runtime");
    let wasm = rt
        .block_on(dwarf_core::componentize(&opts))
        .expect("should componentize a wasi:http/proxy world without a resource-type mismatch");

    Component::new(engine(), &wasm).expect("should produce a well-formed component");
}
