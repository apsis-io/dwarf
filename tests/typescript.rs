//! TypeScript entry points and TypeScript imports.
//!
//! dwarf strips types in-process (oxc) rather than shelling out to a
//! compiler or asking the user to bundle first, so `--file app.ts` is a
//! single command. These tests are about the seam between that transform
//! and the rest of the build: the parts of TypeScript that EMIT code, the
//! import conventions a real project uses, and the boundary where dwarf
//! stops (types are erased, never checked).
mod common;

use std::fs;
use std::path::Path;

use common::{TestCase, dwarf_cmd};
use tempfile::TempDir;
use wasmtime::component::Val;

const WIT: &str = r#"
    package test:ts;
    world ts {
        export value: func() -> string;
    }
"#;

fn build_and_call(dir: &TempDir, entry: &str) -> String {
    let wit_path = dir.path().join("test.wit");
    fs::write(&wit_path, WIT).unwrap();
    let out = dir.path().join("out.wasm");

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(dir.path().join(entry))
        .arg("--output")
        .arg(&out)
        .assert()
        .success();

    out.display().to_string()
}

#[test]
fn a_typescript_entry_builds_and_runs() {
    let mut case = TestCase::new()
        .wit(WIT)
        .script_named(
            "app.ts",
            r#"
            interface Shape { readonly label: string }
            type Rendered = string;

            export function value(): Rendered {
                const s: Shape = { label: "typed" };
                return `${s.label} at runtime` as Rendered;
            }
            "#,
        )
        .expect_call("value", vec![], Val::String("typed at runtime".into()))
        .build()
        .unwrap();

    case.run();
}

#[test]
fn typescript_that_emits_code_still_emits_it() {
    // Enums and parameter properties are not annotations - erasing them
    // would silently change behaviour rather than fail. The enum case is
    // load-bearing: the transformer needs const-evaluated members and
    // panics without them, so this is the test that keeps that wiring.
    let mut case = TestCase::new()
        .wit(WIT)
        .script_named(
            "app.ts",
            r#"
            enum Level { Low = 1, High = Low * 10 }
            class Tagged { constructor(public readonly tag: string) {} }

            export function value(): string {
                return `${new Tagged("t").tag}${Level.High}`;
            }
            "#,
        )
        .expect_call("value", vec![], Val::String("t10".into()))
        .build()
        .unwrap();

    case.run();
}

#[test]
fn a_relative_import_written_the_typescript_way_resolves() {
    // TypeScript source says `./helper.js` and means `./helper.ts`, since
    // the specifier describes the EMITTED module. A project written this
    // way (the documented convention under NodeNext) must build.
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("helper.ts"),
        "export const part: string = \"from-ts-helper\";\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("app.ts"),
        "import { part } from \"./helper.js\";\nexport function value(): string { return part; }\n",
    )
    .unwrap();

    let component = build_and_call(&dir, "app.ts");
    assert!(Path::new(&component).exists());
}

#[test]
fn a_javascript_entry_may_import_a_typescript_module() {
    // Mixed trees are the realistic migration state, and the extensionless
    // specifier has to find the `.ts` file.
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("typed.ts"),
        "export function part(): string { return \"mixed\"; }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("app.js"),
        "import { part } from \"./typed\";\nexport function value() { return part(); }\n",
    )
    .unwrap();

    let component = build_and_call(&dir, "app.js");
    assert!(Path::new(&component).exists());
}

#[test]
fn a_type_error_is_not_dwarfs_to_report() {
    // Types are stripped, never checked - the same contract as Node's own
    // type stripping and esbuild. Code that `tsc` would reject still
    // builds and still runs; `tsc --noEmit` is where that check lives.
    let mut case = TestCase::new()
        .wit(WIT)
        .script_named(
            "app.ts",
            r#"
            const wrong: number = "actually a string" as unknown as number;
            export function value(): string { return String(wrong); }
            "#,
        )
        .expect_call("value", vec![], Val::String("actually a string".into()))
        .build()
        .unwrap();

    case.run();
}

#[test]
fn a_typescript_syntax_error_names_the_file_and_the_distinction() {
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    let entry = dir.path().join("broken.ts");
    fs::write(&wit_path, WIT).unwrap();
    fs::write(&entry, "export function value(: string {\n").unwrap();

    let stderr = dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&entry)
        .arg("--output")
        .arg(dir.path().join("out.wasm"))
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(stderr).unwrap();

    assert!(
        stderr.contains("broken.ts"),
        "should name the file: {stderr}"
    );
    assert!(
        stderr.contains("not a type error"),
        "should distinguish a syntax error from the type errors dwarf never \
         reports: {stderr}"
    );
}
