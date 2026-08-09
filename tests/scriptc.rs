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
