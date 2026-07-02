//! `wit.Checkpoint.snapshot()`: dump this component's own WASM linear memory
//! (the QuickJS heap, engine bookkeeping, every JS global) as bytes.
//!
//! There is deliberately no `restore()` here. An earlier version of this test
//! tried to prove a restore round-trip and instead proved the opposite: it
//! reliably crashes (even restoring into the SAME still-running instance,
//! not just a fresh one — ruling out a cross-instance globals mismatch and
//! pointing at the return path itself), and a trap-based workaround runs
//! straight into the Component Model's `may_enter` reentrance lock (wasmtime
//! permanently refuses further calls into an instance after any trapped
//! call — confirmed at the canonical-ABI call-entry check in
//! `wasmtime-46.0.1/src/runtime/component/func.rs:445`, a spec-level
//! invariant, not a wasmtime gap). See README's "Checkpoint / Restore"
//! section and `crates/runtime/src/bindings.rs`'s `dump_linear_memory` doc
//! comment for the full account.

mod common;

use std::fs;

use tempfile::TempDir;
use wasmtime::component::Val;

use common::ComponentInstance;
use dwarf_core::{ComponentizeOpts, Runtime};

const WIT: &str = r#"
package test:checkpoint;

world checkpoint {
    export bump: func() -> u32;
    export get: func() -> u32;
    export snapshot: func() -> list<u8>;
}
"#;

const JS: &str = r#"
let count = 0;

export function bump() {
    count += 1;
    return count;
}

export function get() {
    return count;
}

export function snapshot() {
    return wit.Checkpoint.snapshot();
}
"#;

fn build_wasm() -> Vec<u8> {
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    fs::write(&wit_path, WIT).unwrap();

    let opts = ComponentizeOpts {
        wit_path: &wit_path,
        js_source: JS,
        js_path: None,
        module_root: None,
        world_name: None,
        stub_wasi: false,
        disable_gc: false,
        runtime: Runtime::Default,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(dwarf_core::componentize(&opts)).unwrap()
}

fn as_bytes(v: Val) -> Vec<u8> {
    match v {
        Val::List(items) => items
            .into_iter()
            .map(|item| match item {
                Val::U8(b) => b,
                other => panic!("expected list<u8> element, got {other:?}"),
            })
            .collect(),
        other => panic!("expected list<u8>, got {other:?}"),
    }
}

#[test]
fn test_checkpoint_snapshot_returns_nonempty_bytes() {
    let wasm = build_wasm();
    let mut inst = ComponentInstance::from_wasm(wasm, Vec::new(), Vec::new()).unwrap();

    assert_eq!(inst.call1("bump", &[]), Val::U32(1));
    assert_eq!(inst.call1("bump", &[]), Val::U32(2));
    assert_eq!(inst.call1("get", &[]), Val::U32(2));

    let bytes = as_bytes(inst.call1("snapshot", &[]));
    assert!(!bytes.is_empty(), "snapshot must not be empty");

    // snapshot() is a pure read: it must not disturb the running instance.
    assert_eq!(inst.call1("get", &[]), Val::U32(2));
    assert_eq!(inst.call1("bump", &[]), Val::U32(3));
}

#[test]
fn test_checkpoint_snapshot_is_repeatable() {
    let wasm = build_wasm();
    let mut inst = ComponentInstance::from_wasm(wasm, Vec::new(), Vec::new()).unwrap();

    // A snapshot is the whole linear memory image, easily tens of MB even for
    // a trivial component (QuickJS's own heap baseline). wasmtime's default
    // 128 MiB "hostcall fuel" budget (a DoS guard on guest-to-host data
    // volume, covering data returned from an export call too) is meant for
    // ordinary-sized WIT payloads, not repeated whole-memory dumps — any host
    // that wants to call `snapshot()` more than once or twice per instance
    // needs to raise this the same way. See README's "Checkpoint / Restore"
    // section.
    let (_, store) = inst.parts();
    store.set_hostcall_fuel(usize::MAX);

    let first = as_bytes(inst.call1("snapshot", &[]));
    let second = as_bytes(inst.call1("snapshot", &[]));

    // NOT asserted equal: snapshot() itself allocates (the returned Vec<u8>,
    // marshaled back through the canonical ABI's list machinery), which grows
    // linear memory as a side effect — so back-to-back snapshots legitimately
    // differ in size. Memory only ever grows, never shrinks, so the second
    // must be at least as large as the first.
    assert!(
        second.len() >= first.len(),
        "linear memory only grows, so a later snapshot can't be smaller \
         (first={}, second={})",
        first.len(),
        second.len()
    );
}
