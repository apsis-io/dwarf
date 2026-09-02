//! Tests for the global `fetch()` polyfill (wired to `wasi:http/client@0.3.x`
//! when the world imports it) - see crates/core/src/polyfills.rs's
//! `generate_fetch`.
//!
//! A real network round-trip isn't exercised here: dwarf's own test harness
//! (tests/common/mod.rs) only links `wasmtime_wasi::p2`/`p3` (core WASI),
//! not `wasmtime-wasi-http` - a component importing `wasi:http/client` can be
//! componentized and validated as well-formed here, but not instantiated.
//! The request/response construction was manually verified end-to-end via
//! `wasmtime run -S http` against both a real local HTTP server (successful
//! round trip) and an unreachable port (a clean, catchable "connection
//! refused" error, not a crash) before landing - see the commit message for
//! specifics.
#![cfg(feature = "component-model-async")]

mod common;

use wasmtime::component::{Component, Val};

use common::{TestCase, engine, wasi_wit_dir};

#[test]
fn test_fetch_component_builds_with_wasi_http_client() {
    let opts = dwarf_core::ComponentizeOpts {
        wit_path: &wasi_wit_dir(),
        js_source: r#"
            export async function run() {
                const resp = await fetch("http://example.invalid/");
                return await resp.text();
            }
        "#,
        js_path: None,
        minify: false,
        module_root: None,
        world_name: Some("fetch-test"),
        stub_wasi: false,
        auto_vendor: false,
        polyfills: &["fetch-classes"],
        disable_gc: false,
        runtime: dwarf_core::Runtime::Default,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("should build tokio runtime");
    let wasm = rt
        .block_on(dwarf_core::componentize(&opts))
        .expect("should componentize a world importing wasi:http/client with fetch-classes");

    Component::new(engine(), &wasm).expect("should produce a well-formed component");
}

#[tokio::test]
async fn test_fetch_requires_wasi_http_client_import() {
    let mut instance = TestCase::new()
        .wit_dir(wasi_wit_dir())
        .world("fetch-no-client")
        .polyfills(&["fetch-classes"])
        .script(
            r#"
            export async function run() {
                try {
                    await fetch("http://example.invalid/");
                    return "FAIL: fetch should have thrown";
                } catch (e) {
                    if (!e.message.includes("wasi:http/client")) {
                        return "FAIL: wrong message: " + e.message;
                    }
                    return "OK";
                }
            }
            "#,
        )
        .build_async()
        .await
        .expect("should build a fetch component without a wasi:http/client import");

    let result = instance.call1_async("run", &[]).await.unwrap();
    assert_eq!(result, Val::String("OK".into()));
}
