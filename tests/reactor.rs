//! Reactor lifecycle: instantiate once, call many times, keep state.
//!
//! "Reactor" is a preview-1 CORE MODULE distinction (`_initialize` rather
//! than `_start`), and dwarf still links the p1 reactor adapter because its
//! QuickJS module is built against p1. At the COMPONENT level the label
//! carries no meaning: a component exports what its world declares, and
//! "command" is simply the world that exports `wasi:cli/run`. Everything
//! else — every ordinary `dwarf --wit x.wit --js x.js` — is what people
//! call a reactor.
//!
//! What actually distinguishes one is the LIFECYCLE, and that is what these
//! tests pin: module state established before the first call survives
//! across calls on one instance, and a fresh instance starts from the
//! snapshot rather than from its predecessor's state.
mod common;

use common::TestCase;
use wasmtime::component::Val;

const WIT: &str = r#"
    package test:reactor;
    world reactor {
        export bump: func() -> u32;
        export total: func() -> u32;
        export greet: func(name: string) -> string;
    }
"#;

/// Module-level state, mutated per call. Wizer snapshots this file's
/// top level at BUILD time, so `count` starts at 0 in every instance;
/// what these tests check is what happens after that.
const JS: &str = r#"
    let count = 0;
    const prefix = "hello, ";

    export function bump() {
      count += 1;
      return count;
    }

    export function total() {
      return count;
    }

    export function greet(name) {
      return prefix + name + " #" + count;
    }
"#;

#[test]
fn state_survives_across_calls_on_one_instance() {
    TestCase::new()
        .wit(WIT)
        .script(JS)
        // The same instance answers all of these in order: a reactor that
        // reset between calls would return 1, 1, 0.
        .expect_call("bump", vec![], Val::U32(1))
        .expect_call("bump", vec![], Val::U32(2))
        .expect_call("bump", vec![], Val::U32(3))
        .expect_call("total", vec![], Val::U32(3))
        // ...and the snapshotted const is still there, so a call reads both
        // build-time and runtime state.
        .expect_call(
            "greet",
            vec![Val::String("engi".into())],
            Val::String("hello, engi #3".into()),
        )
        .build()
        .unwrap()
        .run();
}

#[test]
fn a_fresh_instance_starts_from_the_snapshot() {
    // Not the previous test's end state: each instantiation begins at the
    // Wizer snapshot, which is what makes a reactor reusable rather than
    // accumulating across unrelated runs.
    TestCase::new()
        .wit(WIT)
        .script(JS)
        .expect_call("total", vec![], Val::U32(0))
        .expect_call("bump", vec![], Val::U32(1))
        .build()
        .unwrap()
        .run();
}

/// The per-instance init hook: `_initialize`.
///
/// The JS top level runs at WIZER time and is snapshotted, so it cannot do
/// per-instance work. `_initialize` can: it runs once per instance, before
/// the first exported call. The name is collision-proof — WIT identifiers
/// are lowercase kebab-case and reach JS as camelCase, so no WIT export can
/// ever be called `_initialize`.
const INIT_WIT: &str = r#"
    package test:reactorinit;
    world reactorinit {
        export ready: func() -> string;
        export bump: func() -> u32;
    }
"#;

const INIT_JS: &str = r#"
    // Runs at BUILD time, snapshotted into every instance.
    let phase = "snapshot";
    let inits = 0;
    let count = 0;

    export function _initialize() {
      phase = "initialized";
      inits += 1;
    }

    export function ready() {
      return `${phase} inits=${inits}`;
    }

    export function bump() {
      count += 1;
      return count;
    }
"#;

#[test]
fn initialize_runs_once_per_instance_before_the_first_call() {
    TestCase::new()
        .wit(INIT_WIT)
        .script(INIT_JS)
        // The FIRST call already sees the hook's effect, so it ran before
        // this call rather than after it...
        .expect_call("ready", vec![], Val::String("initialized inits=1".into()))
        // ...and it is not re-run per call: still exactly one.
        .expect_call("bump", vec![], Val::U32(1))
        .expect_call("ready", vec![], Val::String("initialized inits=1".into()))
        .build()
        .unwrap()
        .run();
}

#[test]
fn a_module_without_the_hook_is_unaffected() {
    // The common case: no `_initialize`, nothing runs, no error.
    TestCase::new()
        .wit(WIT)
        .script(JS)
        .expect_call("bump", vec![], Val::U32(1))
        .expect_call("total", vec![], Val::U32(1))
        .build()
        .unwrap()
        .run();
}
