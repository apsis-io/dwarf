//! Waking an export that is blocked on a host call.
//!
//! dwarf drains QuickJS's job queue in exactly one place - `TaskState::poll`
//! - and `poll` is re-entered only when a WAITABLE in the suspended task's
//! own set fires. So nothing living purely in the job queue can reach a
//! blocked export: `AbortController`, `Promise.race` over hand-rolled
//! promises, an abort listener that demonstrably runs - all of them schedule
//! a continuation that is never drained.
//!
//! A WIT `future` is a waitable, and that is the entire difference. The
//! mechanism that makes it usable is that `future_write` needs task state
//! only when the write BLOCKS: when a reader is already waiting it completes
//! immediately, so a SYNCHRONOUS export - which has no task state at all -
//! can complete a future that a suspended task is reading, and the resulting
//! event re-enters the task, drains the queue, settles the race, and lets
//! the export return through the normal path.
//!
//! Both arms are here on purpose. The promise arm is the negative control:
//! without it, "the call returned" passes for the wrong reason, since an
//! export that never wedged also returns. Deleting it once the positive arm
//! is green would leave a test that cannot fail for the reason it exists.
//!
//! Found with periapsis's radiant-main, who measured it end to end in trail
//! first; the host call shape here is theirs.
mod common;

use std::time::Duration;

use wasmtime::Store;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use common::async_engine;
use dwarf_core::{ComponentizeOpts, Runtime};

wasmtime::component::bindgen!({
    path: "tests/wit/future-wake",
    world: "guest",
    imports: { default: async },
    // `store` is load-bearing: it gives EVERY export - including `poke`,
    // which is a plain `func` in the WIT - a call that takes an `&Accessor`
    // and returns an awaitable future. Without it a sync-in-WIT export wants
    // an `AsContextMut`, which inside a concurrent scope is only reachable
    // through a synchronous closure the future cannot outlive.
    exports: { default: async | store },
});

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

impl GuestImports for Ctx {}

impl GuestImportsWithStore<Ctx> for wasmtime::component::HasSelf<Ctx> {
    /// The wedge: a host call that never returns. Not slow - unanswerable,
    /// which is the case no timeout on the host side can rescue.
    async fn wedge(_accessor: &wasmtime::component::Accessor<Ctx, Self>) {
        std::future::pending::<()>().await
    }
}

impl spike::wake::waker_types::Host for Ctx {
    async fn make(&mut self) -> wasmtime::component::FutureReader<u32> {
        unreachable!("declared only so the world contains a future<u32> type")
    }
}

/// Builds `js` into a component, calls `block`, and pokes it once `block` is
/// parked in `wedge`. Returns what `block` returned, or `None` if it never
/// returned within `budget`.
async fn block_then_poke(js: &str, budget: Duration) -> Option<String> {
    let wit_path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit/future-wake");
    let wasm = dwarf_core::componentize(&ComponentizeOpts {
        wit_path: &wit_path,
        js_source: js,
        js_path: None,
        minify: false,
        module_root: None,
        world_name: Some("guest"),
        stub_wasi: false,
        auto_vendor: false,
        polyfills: &[],
        disable_gc: false,
        runtime: Runtime::Default,
    })
    .await
    .expect("should componentize");

    let engine = async_engine();
    let component = Component::new(engine, &wasm).expect("should parse component");
    let mut store = Store::new(
        engine,
        Ctx {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        },
    );

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("wasi p2");
    wasmtime_wasi::p3::add_to_linker(&mut linker).expect("wasi p3");
    Guest::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut linker, |c| c)
        .expect("add world to linker");

    let instance = Guest::instantiate_async(&mut store, &component, &linker)
        .await
        .expect("should instantiate");

    let run = store.run_concurrent(async move |acc| {
        // Both calls go through the SAME accessor, in one concurrent scope.
        let block = instance.call_block(acc);
        tokio::pin!(block);

        // The delay is load-bearing: `block` has to reach its await so the
        // read is genuinely registered in the suspended task's set before the
        // write lands. Poke too early and this measures something else.
        let poke = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            instance.call_poke(acc).await
        };
        tokio::pin!(poke);

        // Pinned and polled by `&mut`, so neither future is dropped and
        // recreated on each turn - a select! over unpinned futures would
        // restart `block` every iteration.
        let mut poked = false;
        loop {
            tokio::select! {
                returned = &mut block => return returned.expect("block should not trap"),
                result = &mut poke, if !poked => {
                    result.expect("poke should not trap");
                    poked = true;
                }
            }
        }
    });

    tokio::time::timeout(budget, run).await.ok().map(|r| {
        r.expect("run_concurrent scope should not error")
    })
}

#[tokio::test]
async fn a_future_wakes_an_export_blocked_on_a_host_call() {
    let outcome = block_then_poke(
        include_str!("wit/future-wake/guest.js"),
        Duration::from_secs(20),
    )
    .await;

    // Asserting the OUTCOME, not merely that it returned: an export that
    // never wedged also returns, so "returned" alone passes for the wrong
    // reason. "WOKEN" means the future's arm of the race won.
    assert_eq!(
        outcome.as_deref(),
        Some("WOKEN"),
        "the blocked export should resume via the future, not the wedged import"
    );
}

#[tokio::test]
async fn a_plain_promise_cannot_wake_a_blocked_export() {
    // The negative control. `poke` runs and resolves the promise - the same
    // sync export, doing the same thing, one type different - and the
    // continuation is never drained, because a promise is not a waitable.
    let outcome = block_then_poke(
        include_str!("wit/future-wake/guest-promise.js"),
        Duration::from_secs(5),
    )
    .await;

    assert_eq!(
        outcome, None,
        "a JS promise is not a waitable and must not be able to resume a \
         suspended task; if this ever returns, the positive arm above has \
         stopped testing what it claims"
    );
}
