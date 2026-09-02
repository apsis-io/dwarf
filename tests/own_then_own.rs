//! Second differential test against `tests/borrow_passthrough.rs`: same
//! shape, but the import ALSO takes `own<probe-thing>` (not `borrow`) -
//! isolating whether the trap is specific to lowering a *borrow* argument
//! for an import call, as opposed to a general problem with forwarding any
//! externally-received resource into an import call.

mod common;

use wasmtime::Store;
use wasmtime::component::{Component, Linker, Resource, ResourceAny, ResourceType, Val};
use wasmtime_wasi::WasiCtxBuilder;

use common::{WasiCtxState, async_engine};
use dwarf_core::{ComponentizeOpts, Runtime};

struct ProbeThing;

#[tokio::test]
async fn test_export_received_owned_resource_passes_through_to_import_as_own() {
    let wit_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/own-then-own");

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
        .expect("should componentize the own-then-own component");

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
    receive_func
        .call_async(&mut store, &[Val::Resource(resource_any)], &mut results)
        .await
        .expect("receive() should not trap while forwarding its OWNED parameter to an import call as own");

    assert_eq!(
        results[0],
        Val::String("use-it saw the resource".to_string()),
    );
}
