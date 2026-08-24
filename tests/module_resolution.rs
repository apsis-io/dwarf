//! What the module root does and does not allow.
//!
//! The root is a statement about the module GRAPH — which files a build may
//! reach — so its containment check is lexical. That is what lets a package
//! manager symlink `node_modules/<pkg>` into a global content-addressed
//! store (pnpm, bun and nub all do) while still refusing a `..` that climbs
//! out of the root.
mod common;

use std::fs;
use std::path::Path;

use common::{TestCase, dwarf_cmd};
use wasmtime::component::Val;

const WIT: &str = r#"
    package test:res;
    world res {
        export value: func() -> string;
    }
"#;

/// A package laid out the way pnpm/bun/nub do it: the real files live in a
/// store OUTSIDE the module root, and `node_modules/<name>` is a symlink.
fn linked_package(root: &Path, store: &Path, name: &str, source: &str) {
    let pkg = store.join(name);
    fs::create_dir_all(&pkg).unwrap();
    fs::write(
        pkg.join("package.json"),
        format!(r#"{{"name":"{name}","version":"1.0.0","type":"module","main":"index.js"}}"#),
    )
    .unwrap();
    fs::write(pkg.join("index.js"), source).unwrap();

    let node_modules = root.join("node_modules");
    fs::create_dir_all(&node_modules).unwrap();
    std::os::unix::fs::symlink(&pkg, node_modules.join(name)).unwrap();
}

#[test]
fn a_symlinked_package_store_resolves() {
    // The failure this pins: dwarf canonicalized every resolved import and
    // then required the REAL path to sit under the module root, so a store
    // symlink was rejected with "is not under module root" and only a flat
    // npm node_modules worked.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("project");
    let store = dir.path().join("store"); // deliberately NOT under the root
    fs::create_dir_all(&root).unwrap();

    linked_package(&root, &store, "dep", "export const value = 'from the store';\n");
    fs::write(
        root.join("app.js"),
        "import { value } from 'dep';\nexport function value_() { return value; }\nexport { value_ as value };\n",
    )
    .unwrap();
    fs::write(root.join("app.wit"), WIT).unwrap();

    let out = dir.path().join("out.wasm");
    dwarf_cmd()
        .args(["--wit", root.join("app.wit").to_str().unwrap()])
        .args(["--js", root.join("app.js").to_str().unwrap()])
        .args(["--output", out.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn a_parent_escape_is_still_refused() {
    // Lexical containment still holds: `..` cannot leave the root, which is
    // what keeps the root meaningful at all.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(&root).unwrap();

    fs::write(dir.path().join("outside.js"), "export const value = 'escaped';\n").unwrap();
    fs::write(
        root.join("app.js"),
        "import { value } from '../outside.js';\nexport function value_() { return value; }\nexport { value_ as value };\n",
    )
    .unwrap();
    fs::write(root.join("app.wit"), WIT).unwrap();

    let out = dir.path().join("out.wasm");
    dwarf_cmd()
        .args(["--wit", root.join("app.wit").to_str().unwrap()])
        .args(["--js", root.join("app.js").to_str().unwrap()])
        .args(["--output", out.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicates::str::contains("module root"));
}

#[test]
fn a_promise_from_a_sync_export_says_so() {
    // The other half of `require_export`'s job: an async JS function under a
    // synchronous WIT `func` used to surface as a lift failure deep in the
    // ABI ("expected string: FromJs { from: \"promise\" }") naming neither
    // the export nor the fix.
    let mut case = TestCase::new()
        .wit(WIT)
        .script("export async function value() { return 'late'; }\n")
        .expect_call("value", vec![], Val::String("late".into()))
        .build()
        .unwrap();

    // The call traps; the guest's panic message is what the developer reads,
    // so that is what this asserts on rather than the host-side trap text.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| case.run()));
    let stderr = String::from_utf8_lossy(&case.stderr_bytes()).to_string();
    assert!(
        stderr.contains("returned a Promise") && stderr.contains("async func"),
        "the failure should name the cause and the fix, got: {stderr}"
    );
}
