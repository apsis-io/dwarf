//! Empirical proof that dwarf's guest-side async dispatch supports genuine
//! CONCURRENT overlap of multiple pending async import calls within a
//! single instance — not just one pending operation at a time (every other
//! async test in this suite, and every real WASI 0.3 example built this
//! session, only ever exercises one in-flight async op).
//!
//! `test:concurrent/api`'s `delay-echo(id, delay-ms)` has a REAL host-side
//! implementation that `tokio::time::sleep`s for `delay-ms` before
//! resolving. The guest calls it twice via `Promise.all` without awaiting
//! either individually first. If dwarf's dispatch serializes the two calls
//! (e.g. only issues the second once the first's `Pending::ImportCall`
//! resolves), total wall-clock time would be ~2x a single call's delay; if
//! both are genuinely in flight to the host at once, it's ~1x. Also checks
//! correctness: the two results must come back matched to their own `id`,
//! not scrambled.

mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use wasmtime::component::Val;

const DELAY_MS: u64 = 150;

#[tokio::test]
async fn test_concurrent_async_import_calls_overlap() {
    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/concurrent");

    let opts = dwarf_core::ComponentizeOpts {
        wit_path: &wit_path,
        js_source: include_str!("wit/concurrent/probe.js"),
        js_path: None,
        module_root: None,
        world_name: Some("concurrent-probe"),
        stub_wasi: false,
        disable_gc: false,
        runtime: dwarf_core::Runtime::Default,
    };
    let wasm = dwarf_core::componentize(&opts)
        .await
        .expect("should build the concurrent-probe component");

    let engine = common::async_engine();
    let component =
        wasmtime::component::Component::new(engine, &wasm).expect("component should validate");

    let wasi = wasmtime_wasi::WasiCtxBuilder::new().build();
    let table = wasmtime::component::ResourceTable::new();
    let mut store = wasmtime::Store::new(engine, common::WasiCtxState { wasi, table });

    let mut linker = wasmtime::component::Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).unwrap();
    wasmtime_wasi::p3::add_to_linker(&mut linker).unwrap();

    // Real host-side implementation: genuinely sleeps, so two overlapping
    // calls only finish quickly if the host is actually driving them both
    // at once, not one-after-another.
    let mut api = linker.instance("test:concurrent/api").unwrap();
    api.func_new_concurrent("delay-echo", |_accessor, _ty, args, results| {
        let Val::U32(id) = args[0] else {
            panic!("expected u32 id, got {:?}", args[0]);
        };
        let Val::U32(delay_ms) = args[1] else {
            panic!("expected u32 delay-ms, got {:?}", args[1]);
        };
        let result_slot = &mut results[0];
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms as u64)).await;
            *result_slot = Val::U32(id);
            Ok(())
        })
    })
    .unwrap();

    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("should instantiate");

    let func = instance
        .get_func(&mut store, "probe")
        .expect("probe export not found");
    let mut results = [Val::Bool(false)];

    let start = Instant::now();
    func.call_async(&mut store, &[], &mut results)
        .await
        .expect("calling probe should not trap");
    let elapsed = start.elapsed();

    let Val::List(items) = &results[0] else {
        panic!("expected a list result, got {:?}", results[0]);
    };
    let ids: Vec<u32> = items
        .iter()
        .map(|v| match v {
            Val::U32(n) => *n,
            other => panic!("expected u32 list element, got {other:?}"),
        })
        .collect();
    assert_eq!(
        ids,
        vec![1, 2],
        "each delay-echo call's result must come back matched to its own id, not scrambled"
    );

    // Two DELAY_MS-ms calls: serialized would take ~2x, genuinely concurrent
    // ~1x. Threshold at 1.5x leaves headroom for scheduling jitter while
    // still clearly distinguishing the two cases (2x would need >270ms of
    // headroom to false-negative past a 225ms threshold).
    let threshold = Duration::from_millis(DELAY_MS * 3 / 2);
    assert!(
        elapsed < threshold,
        "two {DELAY_MS}ms delay-echo calls took {elapsed:?} (>= {threshold:?}) - \
         looks serialized, not concurrent"
    );
}
