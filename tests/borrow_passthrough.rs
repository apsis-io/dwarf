//! Verifies the one combination the host-hijack feasibility note
//! (docs/design-notes/host-hijack-feasibility.md) flagged as reasoned-through
//! but not empirically tested: a resource received as a parameter of an
//! **exported** function, passed straight through as a **borrowed** argument
//! to an **imported** function - the exact shape `claim(request: borrow
//! <request>)` needs, where `request` arrives via `handle(request)`.
//!
//! Rather than needing `wasi:http/types`, this defines a minimal host-owned
//! resource (`probe-thing`) via `Linker::instance(...).resource(...)` and a
//! matching import (`use-it`) that the guest calls with the value it
//! received as `receive`'s own borrowed parameter - the same
//! export-receives-then-import-forwards shape, independent of which
//! interface declares the resource type.
//!
//! **Finding: this traps.** Contrary to the feasibility note's optimistic
//! read of `pop_borrow`/`imported_resource_to_handle` (generic,
//! type-agnostic code that "should already work"), lowering a borrowed
//! import-call argument from a resource the guest received externally (not
//! one it just constructed) fails with "borrow handles still remain at the
//! end of the call" - see `tests/own_then_borrow.rs` (same trap even when
//! the export's own parameter is `own`, not `borrow`) and
//! `tests/own_then_own.rs` (succeeds when the import ALSO takes `own`, not
//! `borrow`) for the isolating differential tests. This is not
//! host-hijack-specific: no existing dwarf test exercises *any* import call
//! with a `borrow<T>` parameter for a host-defined resource, so this is a
//! previously-undiscovered gap - see the design note's "Borrow-forwarding
//! finding" section for the full writeup and what remains unconfirmed (the
//! exact mechanism inside `wit-dylib`'s generated canon-lower glue, which
//! lives in an external dependency, not dwarf's own crates).

mod common;

use wasmtime::Store;
use wasmtime::component::{Component, Linker, Resource, ResourceAny, ResourceType, Val};
use wasmtime_wasi::WasiCtxBuilder;

use common::{WasiCtxState, async_engine};
use dwarf_core::{ComponentizeOpts, Runtime};

/// Marker type used only to give `Resource<T>`/`ResourceType::host::<T>()` a
/// distinct Rust type - this test never stores any real host-side state for
/// it, since the point under test is purely the guest's ABI-level handling
/// of the borrowed handle, not what it points to.
struct ProbeThing;

#[tokio::test]
async fn test_export_received_resource_passes_through_to_import_as_borrow() {
    let wit_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/borrow-passthrough");

    let opts = ComponentizeOpts {
        wit_path: &wit_path,
        js_source: r#"
            import { useIt } from "test:borrow-passthrough/probe";

            export async function receive(thing) {
                return useIt(thing);
            }
        "#,
        js_path: None,
        module_root: None,
        world_name: None,
        stub_wasi: false,
        auto_vendor: false,
        polyfills: &[],
        disable_gc: false,
        runtime: Runtime::Default,
    };

    let wasm = dwarf_core::componentize(&opts)
        .await
        .expect("should componentize the borrow-passthrough component");

    let engine = async_engine();
    let component = Component::new(engine, &wasm).expect("should parse component");

    let wasi = WasiCtxBuilder::new().build();
    let table = wasmtime::component::ResourceTable::new();
    let mut store = Store::new(engine, WasiCtxState { wasi, table });

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("wasi p2");
    wasmtime_wasi::p3::add_to_linker(&mut linker).expect("wasi p3");

    let mut probe = linker
        .instance("test:borrow-passthrough/probe")
        .expect("probe instance");
    probe
        .resource(
            "probe-thing",
            ResourceType::host::<ProbeThing>(),
            |_store, _rep| Ok(()),
        )
        .expect("register probe-thing resource type");
    probe
        .func_new(
            "[method]probe-thing.ping",
            |_store, _ty, _params, results| {
                results[0] = Val::String("pong".to_string());
                Ok(())
            },
        )
        .expect("register probe-thing.ping");
    probe
        .func_new("use-it", |_store, _ty, params, results| {
            let Val::Resource(_) = &params[0] else {
                return Err(wasmtime::Error::msg("expected a resource argument"));
            };
            // The actual assertion: the guest successfully lowered the
            // borrowed handle it received as `receive`'s own parameter back
            // into this import call - if the ABI-level plumbing were broken,
            // this host function would either never be reached (a trap
            // before the call), or `params[0]` would be some other Val.
            results[0] = Val::String("use-it saw the borrowed resource".to_string());
            Ok(())
        })
        .expect("register use-it");

    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("should instantiate");

    let receive_func = instance
        .get_func(&mut store, "receive")
        .expect("receive export not found");

    // Construct a host-owned resource instance to pass in as `receive`'s
    // own parameter - standing in for `wasi:http`'s `request`, which the
    // guest similarly never constructs itself, only ever receives.
    let host_resource = Resource::<ProbeThing>::new_own(42);
    let resource_any = ResourceAny::try_from_resource(host_resource, &mut store)
        .expect("should convert to ResourceAny");

    let mut results = [Val::String(String::new())];
    let result = receive_func
        .call_async(&mut store, &[Val::Resource(resource_any)], &mut results)
        .await;

    assert!(
        result.is_err(),
        "expected receive() to trap when forwarding its own borrowed \
         parameter to an import call as a borrow - if this ever starts \
         returning Ok, that's a real (and very welcome) fix to dwarf's \
         borrow-forwarding handling, worth revisiting \
         docs/design-notes/host-hijack-feasibility.md and this test over"
    );
    let message = format!("{:#}", result.unwrap_err());
    assert!(
        message.contains("borrow handles still remain at the end of the call"),
        "expected the specific known trap, got: {message}"
    );
}
