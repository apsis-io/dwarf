//! Differential test against `tests/borrow_passthrough.rs`: same
//! export-receives-then-import-forwards shape, except the export receives
//! an **owned** resource (`own<probe-thing>`) instead of a borrowed one,
//! while the import still takes a `borrow<probe-thing>` - isolating whether
//! the trap found in `borrow_passthrough.rs` is specific to the export's own
//! parameter also being a borrow, or a broader issue with forwarding any
//! received resource into an import call as a borrow.
//!
//! **Finding: traps identically**, regardless of whether the export's own
//! parameter was `own` or `borrow` - this isolates the problem to the
//! *import call's* borrow argument specifically, not anything about how the
//! export received the resource. See `tests/own_then_own.rs` for the
//! confirming counterpart (`own` on both sides succeeds).

mod common;

use wasmtime::Store;
use wasmtime::component::{Component, Linker, Resource, ResourceAny, ResourceType, Val};
use wasmtime_wasi::WasiCtxBuilder;

use common::{WasiCtxState, async_engine};
use dwarf_core::{ComponentizeOpts, Runtime};

struct ProbeThing;

#[tokio::test]
async fn test_export_received_owned_resource_passes_through_to_import_as_borrow() {
    let wit_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/own-then-borrow");

    let opts = ComponentizeOpts {
        wit_path: &wit_path,
        js_source: r#"
            import { useIt } from "test:own-passthrough/probe";

            export async function receive(thing) {
                return useIt(thing);
            }
        "#,
        js_path: None,
        minify: false,
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
        .expect("should componentize the own-then-borrow component");

    let engine = async_engine();
    let component = Component::new(engine, &wasm).expect("should parse component");

    let wasi = WasiCtxBuilder::new().build();
    let table = wasmtime::component::ResourceTable::new();
    let mut store = Store::new(engine, WasiCtxState { wasi, table });

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("wasi p2");
    wasmtime_wasi::p3::add_to_linker(&mut linker).expect("wasi p3");

    let mut probe = linker
        .instance("test:own-passthrough/probe")
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
            results[0] = Val::String("use-it saw the resource".to_string());
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

    let host_resource = Resource::<ProbeThing>::new_own(42);
    let resource_any = ResourceAny::try_from_resource(host_resource, &mut store)
        .expect("should convert to ResourceAny");

    let mut results = [Val::String(String::new())];
    let result = receive_func
        .call_async(&mut store, &[Val::Resource(resource_any)], &mut results)
        .await;

    assert!(
        result.is_err(),
        "expected receive() to trap even though it received an OWNED \
         parameter - the trap is about the import call's borrow argument, \
         not how the export received the resource"
    );
    let message = format!("{:#}", result.unwrap_err());
    assert!(
        message.contains("borrow handles still remain at the end of the call"),
        "expected the specific known trap, got: {message}"
    );
}
