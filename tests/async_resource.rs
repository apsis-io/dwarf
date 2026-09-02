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

use wasmtime::AsContextMut;
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

/// Proves an ASYNC TOP-LEVEL import function that takes an OWNED RESOURCE as
/// a PARAMETER (not a resource method, not just returning a resource — the
/// argument itself is `own<R>`) works end to end: JS constructs the resource,
/// calls the async import with it, and gets back a real result computed by a
/// genuine host-side implementation (not a stub).
///
/// Was a known gap: dwarf's own build-time linker stubbing
/// (`define_unknown_imports_as_traps_async` in `crates/core/src/lib.rs`)
/// used `Linker::func_new_async` for unsatisfied async imports, which can
/// never typecheck against a WIT `async func` — `func_new_async`/
/// `func_wrap_async` implement *sync*-typed WIT functions via blocking async
/// Rust (see wasmtime's own "Blocking / Async Behavior" docs on
/// `func_wrap_async`), not genuinely async-typed ones. Fixed by switching to
/// `func_new_concurrent`, which does typecheck correctly against `async
/// func`. Filed upstream against wasmtime for the confusing error message /
/// undocumented gap: https://github.com/bytecodealliance/wasmtime/issues/13813
///
/// Found while wiring `wasi:http/client`'s `send: async func(request:
/// request) -> result<response, error-code>` into a Periapsis example —
/// `request` is passed BY VALUE. Isolated here with a minimal synthetic
/// world, independent of wasi:http entirely, to confirm it's a general dwarf
/// fix and not something specific to that interface.
///
/// Bypasses `TestCase::build_async` (the shared test harness, which only
/// links wasi p2/p3) since this needs a REAL host-side implementation of
/// `test:resarg/api` — a hand-rolled resource + async function — to prove
/// dwarf's guest-side binding round-trips correctly, not just that the
/// component instantiates.
#[tokio::test]
async fn test_async_import_taking_owned_resource_param() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/resarg");

    let opts = dwarf_core::ComponentizeOpts {
        wit_path: &wit_path,
        js_source: include_str!("wit/resarg/probe.js"),
        js_path: None,
        minify: false,
        module_root: None,
        world_name: Some("resarg"),
        stub_wasi: false,
        auto_vendor: false,
        polyfills: &[],
        disable_gc: false,
        runtime: dwarf_core::Runtime::Default,
    };
    let wasm = dwarf_core::componentize(&opts)
        .await
        .expect("should build the resarg component");

    let engine = common::async_engine();
    let component =
        wasmtime::component::Component::new(engine, &wasm).expect("component should validate");

    let wasi = wasmtime_wasi::WasiCtxBuilder::new().build();
    let table = wasmtime::component::ResourceTable::new();
    let mut store = wasmtime::Store::new(engine, common::WasiCtxState { wasi, table });

    let mut linker = wasmtime::component::Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    wasmtime_wasi::p3::add_to_linker(&mut linker).unwrap();

    // Real host-side implementation of `test:resarg/api` — a `thing`
    // resource (host representation: the u32 it was constructed with) and
    // `consume`, a genuine `async func` returning a string derived from the
    // resource's value. Neither is a stub: a wrong dwarf-side ABI (bad
    // argument encoding, wrong resource handle lowering) would surface as a
    // wrong string, a trap, or a hang here — not just a build failure.
    let mut api = linker.instance("test:resarg/api").unwrap();
    api.resource(
        "thing",
        wasmtime::component::ResourceType::host::<u32>(),
        |_, _| Ok(()),
    )
    .unwrap();
    api.func_wrap(
        "[constructor]thing",
        |mut store: wasmtime::StoreContextMut<'_, common::WasiCtxState>, (): ()| {
            let resource = store.data_mut().table.push(42u32)?;
            Ok((resource,))
        },
    )
    .unwrap();
    api.func_new_concurrent("consume", |accessor, _ty, args, results| {
        // Resolve the resource and read its value synchronously via the
        // accessor (no real async work needed for this test's host side);
        // only the resulting String needs to cross into the returned future.
        let value: wasmtime::Result<u32> = accessor.with(|mut access| {
            let wasmtime::component::Val::Resource(resource) = args[0].clone() else {
                wasmtime::bail!("expected a resource argument, got {:?}", args[0]);
            };
            let resource = resource.try_into_resource::<u32>(&mut access)?;
            let value = *access.as_context_mut().data_mut().table.get(&resource)?;
            Ok(value)
        });
        let result_slot = &mut results[0];
        Box::pin(async move {
            let value = value?;
            *result_slot = wasmtime::component::Val::String(format!("consumed:{value}"));
            Ok(())
        })
    })
    .unwrap();

    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("should instantiate: the async-owned-resource-param gap is fixed");

    let func = instance
        .get_func(&mut store, "probe")
        .expect("probe export not found");
    let mut results = [wasmtime::component::Val::Bool(false)];
    func.call_async(&mut store, &[], &mut results)
        .await
        .expect("calling probe should not trap");

    let wasmtime::component::Val::String(result) = &results[0] else {
        panic!("expected a string result, got {:?}", results[0]);
    };
    assert_eq!(
        result, "consumed:42",
        "the JS-constructed resource's value should have round-tripped through \
         the real host consume() implementation"
    );
}
