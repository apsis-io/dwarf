//! Statically compiled modules, plugged into the component dwarf builds.
//!
//! These need a scriptc checkout (and the zig/wasm-tools toolchain behind
//! it), which is not something CI carries, so they name it through
//! DWARF_TEST_SCRIPTC and skip when it is absent. Run them with:
//!
//! ```sh
//! DWARF_TEST_SCRIPTC=/path/to/scriptc cargo test --test scriptc
//! ```
mod common;

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;
use wasmtime::component::Val;

use common::{TestCase, scriptc_available};

/// The TypeScript half, on its own. Its exported signatures are all the
/// boundary anyone needs (see `inferred_boundary_needs_no_profile`).
fn hot_source(dir: &TempDir) -> PathBuf {
    let module_dir = dir.path().join("hot");
    fs::create_dir_all(&module_dir).unwrap();
    let entry = module_dir.join("hot.ts");
    fs::write(&entry, HOT_TS).unwrap();
    entry
}

/// The same module with a hand-written profile beside it.
fn hot_module(dir: &TempDir) -> PathBuf {
    let module_dir = dir.path().join("hot");
    fs::create_dir_all(&module_dir).unwrap();
    fs::write(
        module_dir.join("hot.ts"),
        r#"
        export function shout(s: string): string {
          return s.toUpperCase() + "!";
        }
        export function twice(n: number): number {
          return n * 2;
        }
        export function checksum(data: Uint8Array): number {
          let h = 0;
          for (const b of data) h = (h * 31 + b) % 1000000007;
          return h;
        }
        "#,
    )
    .unwrap();
    let profile = module_dir.join("profile.json");
    fs::write(
        &profile,
        r#"{
          "profile_format": 1,
          "name": "hot",
          "entry": "hot.ts",
          "emission": "c",
          "abi": {
            "prefix": "hot_",
            "init_symbol": "hot_init",
            "sink_register_symbol": "hot_set_panic_sink",
            "collect_symbol": "hot_collect",
            "result_reset_symbol": null
          },
          "exports": [
            { "export": "shout", "symbol": "hot_shout", "params": ["string"], "returns": "string" },
            { "export": "twice", "symbol": "hot_twice", "params": ["f64"], "returns": "f64" },
            { "export": "checksum", "symbol": "hot_checksum", "params": ["bytes"], "returns": "f64" }
          ]
        }"#,
    )
    .unwrap();
    profile
}

/// The world declares no import: dwarf adds the interface itself from the
/// WIT scriptc generated, which is the whole point of --scriptc.
const WIT: &str = r#"
    package test:optimized;
    world optimized {
        export greet: func(name: string) -> string;
        export digest: func(text: string) -> f64;
    }
"#;

/// Two exports that cross and one that cannot, so inference has something
/// to leave out.
const HOT_TS: &str = r#"
    export function shout(s: string): string {
      return s.toUpperCase() + "!";
    }
    export function twice(n: number): number {
      return n * 2;
    }
    export function checksum(data: Uint8Array): number {
      let h = 0;
      for (const b of data) h = (h * 31 + b) % 1000000007;
      return h;
    }
    export async function unreachable(s: string): Promise<string> {
      return s;
    }
"#;

const JS: &str = r#"
    import ops from "scriptc:hot/ops";

    export function greet(name) {
      return `${ops.shout(name)} twice(21) = ${ops.twice(21)}`;
    }

    export function digest(text) {
      return ops.checksum(new TextEncoder().encode(text));
    }
"#;

#[test]
fn statically_compiled_module_is_callable_from_javascript() {
    if !scriptc_available() {
        eprintln!("skipping: set DWARF_TEST_SCRIPTC to a scriptc executable");
        return;
    }
    let dir = TempDir::new().unwrap();
    let profile = hot_module(&dir);

    TestCase::new()
        .wit(WIT)
        .script(JS)
        .scriptc(profile)
        // Strings cross both ways, and an f64 comes back.
        .expect_call(
            "greet",
            vec![Val::String("engi".into())],
            Val::String("ENGI! twice(21) = 42".into()),
        )
        // list<u8> in, f64 out — the same value Node computes for this
        // input, so the compiled loop is not merely plausible.
        .expect_call(
            "digest",
            vec![Val::String("hello world".into())],
            Val::Float64(204910434.0),
        )
        .build()
        .unwrap()
        .run();
}

#[test]
fn the_seam_does_not_survive_into_the_component() {
    if !scriptc_available() {
        eprintln!("skipping: set DWARF_TEST_SCRIPTC to a scriptc executable");
        return;
    }
    let dir = TempDir::new().unwrap();
    let profile = hot_module(&dir);

    let instance = TestCase::new()
        .wit(WIT)
        .script(JS)
        .scriptc(profile)
        .build()
        .unwrap();

    // Plugged, not merely declared: an unsatisfied import would leave
    // scriptc:hot/ops looking like something the host has to provide.
    // The name still occurs in the composition's own metadata, so this has
    // to ask the decoded world rather than search the bytes.
    let imports = dwarf_core::scriptc::import_names(&instance.wasm).unwrap();
    assert!(
        !imports.iter().any(|name| name.starts_with("scriptc:")),
        "the scriptc import survived into the final component: {imports:?}"
    );
    // ...while the WASI imports it genuinely needs are untouched.
    assert!(
        imports.iter().any(|name| name.starts_with("wasi:")),
        "expected WASI imports to remain, got {imports:?}"
    );
}

#[test]
fn inferred_boundary_needs_no_profile() {
    if !scriptc_available() {
        eprintln!("skipping: set DWARF_TEST_SCRIPTC to a scriptc executable");
        return;
    }
    let dir = TempDir::new().unwrap();
    // The module alone — the async export simply stays out of the
    // interface, and everything the JavaScript actually calls crosses.
    let entry = hot_source(&dir);

    TestCase::new()
        .wit(WIT)
        .script(JS)
        .scriptc(entry)
        .expect_call(
            "greet",
            vec![Val::String("engi".into())],
            Val::String("ENGI! twice(21) = 42".into()),
        )
        .expect_call(
            "digest",
            vec![Val::String("hello world".into())],
            Val::Float64(204910434.0),
        )
        .build()
        .unwrap()
        .run();
}

/// Records across the seam: scriptc derives a WIT `record` from the
/// TypeScript shape, dwarf plugs the component in, and JavaScript sees a
/// plain object on both sides. The three tests above cross only scalars,
/// strings and a list<u8>, so nothing here was covered by them — and the
/// interesting half is INBOUND, where the canonical ABI flattens the
/// record into core params and the entry has to rebuild it.
const RECORD_WIT: &str = r#"
    package test:records;
    world records {
        export locate: func(label: string) -> string;
        export span: func() -> f64;
    }
"#;

/// A record out, a record in, and a NESTED record — plus two record
/// parameters in one call, which is where a flattening that got its field
/// order from the wrong place would show up.
const RECORD_TS: &str = r#"
    export interface Point { x: number; y: number; label: string }
    export interface Box { origin: Point; w: number }

    export function makePoint(label: string): Point {
      return { x: 3, y: 4, label };
    }
    export function describe(p: Point): string {
      return `${p.label}@${p.x},${p.y}`;
    }
    export function makeBox(): Box {
      return { origin: { x: 1, y: 2, label: "o" }, w: 10 };
    }
    export function boxSpan(b: Box, p: Point): number {
      return b.w + b.origin.x + p.x;
    }
"#;

const RECORD_JS: &str = r#"
    import ops from "scriptc:records/ops";

    export function locate(label) {
      const p = ops.makePoint(label);
      return `${ops.describe(p)} nested=${ops.makeBox().origin.label}`;
    }

    export function span() {
      return ops.boxSpan(ops.makeBox(), ops.makePoint("p"));
    }
"#;

/// Records across the seam, both directions and nested.
///
/// This test found two scriptc bugs on the way in, which is the argument
/// for it existing: the shim dropped a record-returning export's own
/// arguments (clang refused the call), and the record-return path handed
/// the result arena a string BORROWED from the record and then released
/// the record — a use-after-free the next call's arena reset walked into.
/// Neither was reachable from scriptc's own component lane, which runs one
/// wasmtime --invoke per call and so never makes a SECOND call on a live
/// instance. Composition does, on every call.
#[test]
fn records_cross_the_seam_both_directions() {
    if !scriptc_available() {
        eprintln!("skipping: set DWARF_TEST_SCRIPTC to a scriptc executable");
        return;
    }
    let dir = TempDir::new().unwrap();
    let module_dir = dir.path().join("records");
    fs::create_dir_all(&module_dir).unwrap();
    let entry = module_dir.join("records.ts");
    fs::write(&entry, RECORD_TS).unwrap();

    TestCase::new()
        .wit(RECORD_WIT)
        .script(RECORD_JS)
        // No profile: the boundary is inferred, records and all.
        .scriptc(entry)
        // Out and back in: makePoint's record is handed straight to
        // describe, so a field that survived only one direction fails here.
        .expect_call(
            "locate",
            vec![Val::String("engi".into())],
            Val::String("engi@3,4 nested=o".into()),
        )
        // Two record params in one call, one of them nested: 10 + 1 + 3.
        .expect_call("span", vec![], Val::Float64(14.0))
        .build()
        .unwrap()
        .run();
}
