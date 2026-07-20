//! Narrows the borrow-forwarding trap (`tests/borrow_passthrough.rs`,
//! `tests/own_then_borrow.rs`) further: does it require crossing the
//! export/import boundary at all, or a *freestanding* function taking a
//! named `borrow<T>` parameter specifically - as opposed to a resource
//! **method** call, where `self: borrow<T>` is an implicit parameter (the
//! exact shape wit-dylib's own upstream test program `resources_caller.rs`
//! exercises: `[constructor]a` then `[method]a.frob` on the same handle,
//! which is presumably a passing reference test upstream). This calls
//! `create-thing()` for an owned resource, then calls `.ping()` - a
//! **method** - on it, instead of a freestanding function taking a borrow
//! parameter.

mod common;

use wasmtime::Store;
use wasmtime::component::{Component, Linker, ResourceType, Val};
use wasmtime_wasi::WasiCtxBuilder;

use common::{WasiCtxState, async_engine};
use dwarf_core::{ComponentizeOpts, Runtime};

struct ProbeThing;

#[tokio::test]
async fn test_calling_a_method_on_a_previously_owned_resource_traps() {
    let wit_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/internal-borrow");

    let opts = ComponentizeOpts {
        wit_path: &wit_path,
        js_source: r#"
            import { createThing } from "test:internal-borrow/probe";

            export async function run() {
                const thing = createThing();
                return thing.ping();
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
        .expect("should componentize the internal-borrow component");

    let engine = async_engine();
    let component = Component::new(engine, &wasm).expect("should parse component");

    let wasi = WasiCtxBuilder::new().build();
    let table = wasmtime::component::ResourceTable::new();
    let mut store = Store::new(engine, WasiCtxState { wasi, table });

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("wasi p2");
    wasmtime_wasi::p3::add_to_linker(&mut linker).expect("wasi p3");

    let mut probe = linker
        .instance("test:internal-borrow/probe")
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
        .func_new("create-thing", |mut store, _ty, _params, results| {
            let resource = wasmtime::component::Resource::<ProbeThing>::new_own(7);
            let resource_any =
                wasmtime::component::ResourceAny::try_from_resource(resource, &mut store)?;
            results[0] = Val::Resource(resource_any);
            Ok(())
        })
        .expect("register create-thing");

    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("should instantiate");

    let run_func = instance.get_func(&mut store, "run").expect("run export");

    let mut results = [Val::String(String::new())];
    let result = run_func.call_async(&mut store, &[], &mut results).await;

    assert!(
        result.is_err(),
        "expected calling a method (an implicit borrow<T> self parameter) \
         on a previously-`own`-returned resource to trap, matching the \
         freestanding-function cases in borrow_passthrough.rs/\
         own_then_borrow.rs - if this ever starts returning Ok, that's a \
         real fix worth revisiting docs/design-notes/host-hijack-feasibility.md \
         and the other borrow-forwarding tests over"
    );
    let message = format!("{:#}", result.unwrap_err());
    assert!(
        message.contains("borrow handles still remain at the end of the call"),
        "expected the specific known trap, got: {message}"
    );
}
