//! Decisive check on `tests/internal_borrow.rs`'s finding: is the
//! "borrow handles still remain at the end of the call" trap specific to
//! wasmtime's *dynamic* host-resource API (`Linker::instance(...).resource
//! (...)` + `Val::Resource`/`ResourceAny`, used by all the borrow-forwarding
//! tests so far), or does it also happen with a *typed*, `bindgen!`-macro-
//! generated host implementation - the way real WASI implementations
//! (`wasmtime-wasi`, which dwarf's own `tcp-socket` support already proves
//! works fine for this exact "own from constructor, then borrow via a
//! separate method call" shape - see `sock.bind()`/`.getLocalAddress()` in
//! `tests/task_lifetime.rs`) and presumably `periapisis:host/hijack`'s own
//! real host implementation would actually be built?
//!
//! If this test succeeds where the dynamic-API ones trap, the
//! borrow-forwarding bug is an artifact of the dynamic Linker API
//! specifically, not a general dwarf-runtime problem - which would mean
//! host-hijack is NOT actually blocked by it, provided trail's real
//! `claim()`/`hijacked-connection` implementation uses typed `bindgen!`
//! bindings rather than the dynamic API (the normal, idiomatic choice).

mod common;

use wasmtime::Store;
use wasmtime::component::{Component, Linker, Resource, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use common::async_engine;
use dwarf_core::{ComponentizeOpts, Runtime};

wasmtime::component::bindgen!({
    path: "tests/wit/internal-borrow",
    world: "internal-borrow-test",
    imports: { default: async },
    exports: { default: async },
});

use test::internal_borrow::probe::ProbeThing;

struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for Ctx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl test::internal_borrow::probe::HostProbeThing for Ctx {
    async fn ping(&mut self, _self_: Resource<ProbeThing>) -> String {
        "pong".to_string()
    }

    async fn drop(&mut self, _rep: Resource<ProbeThing>) -> wasmtime::Result<()> {
        Ok(())
    }
}

impl test::internal_borrow::probe::Host for Ctx {
    async fn create_thing(&mut self) -> Resource<ProbeThing> {
        Resource::new_own(42)
    }
}

#[tokio::test]
async fn test_typed_bindgen_own_then_method_borrow_succeeds() {
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
    let table = ResourceTable::new();
    let mut store = Store::new(engine, Ctx { wasi, table });

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("wasi p2");
    wasmtime_wasi::p3::add_to_linker(&mut linker).expect("wasi p3");
    InternalBorrowTest::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |c| c)
        .expect("add probe world to linker");

    let instance = InternalBorrowTest::instantiate_async(&mut store, &component, &linker)
        .await
        .expect("should instantiate");

    let result = store
        .run_concurrent(async |accessor| instance.call_run(accessor).await)
        .await
        .expect("run_concurrent scope should not error");

    assert!(
        result.is_ok(),
        "expected a typed bindgen!-based host implementation to handle 'own \
         from a constructor, then borrow via a separate method call' \
         WITHOUT trapping - if this ALSO traps, the borrow-forwarding bug \
         is general (not specific to the dynamic Linker API), which would \
         reopen the question of whether it affects real WASI-shaped host \
         implementations too. Result: {result:?}"
    );
}
