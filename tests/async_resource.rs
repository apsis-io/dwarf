//! Empirical proof that dwarf correctly dispatches an ASYNC METHOD on an
//! IMPORTED WIT resource — e.g. `wasi:sockets`'s `tcp-socket.connect()`, an
//! `async func` on a `resource`. Every existing test before this one only
//! exercised sync resource methods (`test_exported_resource` et al. in
//! wit_types.rs) or async TOP-LEVEL functions (async_types.rs); nothing
//! exercised the combination, which real WASI 0.3 interfaces
//! (wasi:sockets, wasi:filesystem, wasi:http) all depend on.
//!
//! Reuses wasmtime-wasi's own `wasi:sockets@0.3` host implementation (already
//! linked by `AsyncComponentInstance::from_wasm` via `wasmtime_wasi::p3::
//! add_to_linker`) rather than hand-rolling a synthetic host resource — this
//! is real, already-correct host tooling, so a failure here would point at
//! dwarf's guest-side binding, not at a test-only stand-in.

mod common;

use std::path::PathBuf;

use wasmtime::component::Val;

use common::TestCase;

#[tokio::test]
async fn test_async_resource_method_on_imported_socket() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/sockprobe");

    let mut inst = TestCase::new()
        .wit_dir(wit_path)
        .world("sockprobe")
        .script(include_str!("wit/sockprobe/probe.js"))
        .build_async()
        .await
        .expect("should build the sockprobe component");

    let (instance, store) = inst.parts();

    let func = instance
        .get_func(&mut *store, "probe")
        .expect("probe export not found");
    let mut results = [Val::Bool(false)];
    func.call_async(&mut *store, &[], &mut results)
        .await
        .expect("calling probe should not trap");

    let Val::String(result) = &results[0] else {
        panic!("expected a string result, got {:?}", results[0]);
    };

    // The specific outcome (denied by the test harness's default WASI
    // socket permissions, refused since nothing listens on 127.0.0.1:1, or
    // an actual connect) is incidental — what's actually being proven is
    // that the call completed with a clean, well-formed result at all: no
    // trap, no hang, no corruption. That's the async resource-method
    // round-trip (JS -> dwarf's import dispatch -> wasmtime-wasi's real
    // socket implementation -> back to a JS Promise) working end to end.
    assert!(
        result.starts_with("create:") || result.starts_with("connect:"),
        "expected a well-formed create:/connect: outcome, got: {result}"
    );
}

/// Known gap, tracked here rather than left silently undiscovered: an ASYNC
/// TOP-LEVEL import function that takes an OWNED RESOURCE as a PARAMETER
/// (not a resource method, not just returning a resource — the argument
/// itself is `own<R>`) fails at BUILD time, before the component ever runs.
/// Distinct from `test_async_resource_method_on_imported_socket` above: that
/// one proves async *resource methods* work; this shape doesn't.
///
/// Found while wiring `wasi:http/client`'s `send: async func(request:
/// request) -> result<response, error-code>` into a Periapsis example —
/// `request` is passed BY VALUE. Isolated here with a minimal synthetic
/// world, independent of wasi:http entirely, to confirm it's a general dwarf
/// gap and not something specific to that interface.
///
/// This test asserts the CURRENT (broken) behavior on purpose: if dwarf ever
/// silently starts handling this shape differently (fixed, or broken a new
/// way), this test fails loudly instead of the gap just sitting undiscovered
/// again. When this is actually fixed, replace the `unwrap_err`/message
/// assertion with a real round-trip proof, matching the pattern above.
#[tokio::test]
async fn test_known_gap_async_import_taking_owned_resource_param() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/resarg");

    let result = TestCase::new()
        .wit_dir(wit_path)
        .world("resarg")
        .script(include_str!("wit/resarg/probe.js"))
        .build_async()
        .await;

    // Not `.expect_err()`/`.unwrap_err()`: those require the Ok variant
    // (AsyncComponentInstance) to implement Debug, which it doesn't.
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!(
            "if this now succeeds, the async-import-taking-owned-resource gap is fixed — \
             replace this test with a real round-trip proof"
        ),
    };
    let message = format!("{err:#}");
    assert!(
        message.contains("type mismatch with async"),
        "expected the known 'type mismatch with async' build-time linking \
         failure, got a different error — investigate before assuming this \
         is the same known gap: {message}"
    );
}
