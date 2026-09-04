//! CLI integration tests for dwarf
mod common;

use std::fs;
use std::path::PathBuf;

use predicates::prelude::*;
use tempfile::TempDir;
use wasmtime::Store;
use wasmtime::component::{Component, Linker, ResourceTable, Val};
use wasmtime_wasi::WasiCtxBuilder;

use common::{ComponentInstance, WasiCtxState, dwarf_cmd, engine, run_cli_build};

#[test]
fn test_cli_help() {
    dwarf_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: dwarf"))
        .stdout(predicate::str::contains("--opt-size"))
        .stdout(predicate::str::contains("--sync"))
        .stdout(predicate::str::contains("--module-root <PATH>"))
        .stdout(predicate::str::contains("--runtime <PATH>"));
}

#[test]
fn test_cli_version() {
    // Asserted against the crate version rather than a literal, so a
    // release bump cannot leave this test passing while --version lies.
    let expected = format!("dwarf {}", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-V"] {
        dwarf_cmd()
            .arg(flag)
            .assert()
            .success()
            .stdout(predicate::str::contains(expected.as_str()));
    }
}

#[test]
fn test_cli_errors() {
    dwarf_cmd()
        .arg("--wit")
        .arg("nonexistent.wit")
        .arg("--file")
        .arg("test.js")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    fs::write(&wit_path, "package test:test; world test {}").unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg("nonexistent.js")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));

    let js_path = dir.path().join("test.js");
    fs::write(&js_path, "export {};").unwrap();
    let runtime_path = dir.path().join("runtime.wasm");
    fs::write(&runtime_path, dwarf_core::default_runtime_wasm()).unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--opt-size")
        .arg("--runtime")
        .arg(&runtime_path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_cli_output() {
    let (output, _dir) = run_cli_build(
        "package test:hello;\nworld hello { export add: func(a: u32, b: u32) -> u32; }",
        "export function add(a, b) { return a + b; }",
        &[],
    );

    let wasm = fs::read(&output).unwrap();
    let mut inst =
        ComponentInstance::from_wasm(wasm, vec![], vec![]).expect("should instantiate component");

    assert_eq!(inst.call1("add", &[Val::U32(3), Val::U32(4)]), Val::U32(7));
}

#[test]
fn test_cli_defers_host_imports_until_runtime() {
    let (output, _dir) = run_cli_build(
        r#"
            package local:test;

            interface math {
                add: func(a: s32, b: s32) -> s32;
                multiply: func(a: s32, b: s32) -> s32;
            }

            world imports {
                import math;
                export double-add: func(a: s32, b: s32) -> s32;
            }
        "#,
        r#"
            import math from "local:test/math";

            export function doubleAdd(a, b) {
                const sum = math.add(a, b);
                return math.multiply(sum, 2);
            }
        "#,
        &[],
    );

    let engine = engine();
    let component = Component::new(engine, fs::read(&output).unwrap()).unwrap();
    let mut store = Store::new(
        engine,
        WasiCtxState {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        },
    );

    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).unwrap();
    let mut math = linker.instance("local:test/math").unwrap();
    math.func_wrap("add", |_, (a, b): (i32, i32)| Ok((a + b,)))
        .unwrap();
    math.func_wrap("multiply", |_, (a, b): (i32, i32)| Ok((a * b,)))
        .unwrap();

    let instance = linker.instantiate(&mut store, &component).unwrap();
    let func = instance.get_func(&mut store, "double-add").unwrap();
    let mut results = [Val::S32(0)];
    func.call(&mut store, &[Val::S32(4), Val::S32(5)], &mut results)
        .unwrap();

    assert_eq!(results[0], Val::S32(18));
}

#[test]
fn test_cli_resolves_relative_import() {
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    let js_path = dir.path().join("main.js");
    let dep_path = dir.path().join("dep.js");
    let output = dir.path().join("output.wasm");

    fs::write(
        &wit_path,
        "package test:modules;\nworld modules { export answer: func() -> u32; }",
    )
    .unwrap();
    fs::write(
        &js_path,
        r#"import { value } from "./dep.js"; export function answer() { return value + 1; }"#,
    )
    .unwrap();
    fs::write(&dep_path, "export const value = 41;").unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--output")
        .arg(&output)
        .assert()
        .success();

    let wasm = fs::read(&output).unwrap();
    let mut inst =
        ComponentInstance::from_wasm(wasm, vec![], vec![]).expect("should instantiate component");

    assert_eq!(inst.call1("answer", &[]), Val::U32(42));
}

#[test]
fn test_cli_resolves_package_import_from_module_root() {
    let dir = TempDir::new().unwrap();
    let src_dir = dir.path().join("src");
    let pkg_dir = dir.path().join("node_modules").join("pkg");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&pkg_dir).unwrap();

    let wit_path = dir.path().join("test.wit");
    let js_path = src_dir.join("main.js");
    let output = dir.path().join("output.wasm");

    fs::write(
        &wit_path,
        "package test:modules;\nworld modules { export answer: func() -> u32; }",
    )
    .unwrap();
    fs::write(
        &js_path,
        r#"import { value } from "pkg"; export function answer() { return value + 1; }"#,
    )
    .unwrap();
    fs::write(pkg_dir.join("package.json"), r#"{"main":"index.js"}"#).unwrap();
    fs::write(pkg_dir.join("index.js"), "export const value = 41;").unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--module-root")
        .arg(dir.path())
        .arg("--output")
        .arg(&output)
        .assert()
        .success();

    let wasm = fs::read(&output).unwrap();
    let mut inst =
        ComponentInstance::from_wasm(wasm, vec![], vec![]).expect("should instantiate component");

    assert_eq!(inst.call1("answer", &[]), Val::U32(42));
}

#[test]
fn test_cli_resolves_nested_imports_and_caches_modules() {
    let dir = TempDir::new().unwrap();
    let nested_dir = dir.path().join("nested");
    let index_dir = dir.path().join("dir");
    fs::create_dir_all(&nested_dir).unwrap();
    fs::create_dir_all(&index_dir).unwrap();

    let wit_path = dir.path().join("test.wit");
    let js_path = dir.path().join("main.js");
    let output = dir.path().join("output.wasm");

    fs::write(
        &wit_path,
        "package test:modules;\nworld modules { export answer: func() -> u32; }",
    )
    .unwrap();
    fs::write(
        &js_path,
        r#"
            import { nested } from "./nested/entry";
            import { fromIndex } from "./dir";
            import { count as countA } from "./a.js";
            import { count as countB } from "./b.js";

            export function answer() {
                return nested + fromIndex + countA + countB + globalThis.__counter;
            }
        "#,
    )
    .unwrap();
    fs::write(
        nested_dir.join("entry.js"),
        r#"import { base } from "./base"; export const nested = base * 2;"#,
    )
    .unwrap();
    fs::write(nested_dir.join("base.js"), "export const base = 10;").unwrap();
    fs::write(index_dir.join("index.js"), "export const fromIndex = 5;").unwrap();
    fs::write(
        dir.path().join("counter.js"),
        r#"
            globalThis.__counter = (globalThis.__counter ?? 0) + 1;
            export const count = globalThis.__counter;
        "#,
    )
    .unwrap();
    fs::write(
        dir.path().join("a.js"),
        r#"import { count } from "./counter.js"; export { count };"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("b.js"),
        r#"import { count } from "./counter.js"; export { count };"#,
    )
    .unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--output")
        .arg(&output)
        .assert()
        .success();

    let wasm = fs::read(&output).unwrap();
    let mut inst =
        ComponentInstance::from_wasm(wasm, vec![], vec![]).expect("should instantiate component");

    assert_eq!(inst.call1("answer", &[]), Val::U32(28));
}

#[test]
fn test_cli_reports_missing_import() {
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    let js_path = dir.path().join("main.js");
    let output = dir.path().join("output.wasm");

    fs::write(
        &wit_path,
        "package test:modules;\nworld modules { export answer: func() -> u32; }",
    )
    .unwrap();
    fs::write(
        &js_path,
        r#"import { value } from "./missing.js"; export function answer() { return value; }"#,
    )
    .unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--output")
        .arg(&output)
        .assert()
        .failure()
        .stderr(predicate::str::contains("filesystem module not found"));
}

/// wit-parser reports a missing top-level terminator (a `;` after `package`,
/// or a `}` closing a preceding `interface`/`record`/etc.) as an error at the
/// position of the *next* item it manages to parse, not at the missing
/// token - `annotate_syntax_hint` (crates/core/src/wit.rs) appends a hint
/// pointing at the real, more common cause for exactly this "cascading"
/// shape. Covers every variant reproduced while building it, plus a
/// negative case (a genuine typo, unrelated to a missing terminator,
/// already produces a clear, correctly-located error and should NOT get
/// this hint - it would be actively misleading there).
#[test]
fn test_cli_hints_at_missing_terminator_on_cascading_parse_errors() {
    fn build_and_capture_stderr(wit: &str) -> String {
        let dir = TempDir::new().unwrap();
        let wit_path = dir.path().join("test.wit");
        let js_path = dir.path().join("test.js");
        fs::write(&wit_path, wit).unwrap();
        fs::write(&js_path, "export function f() {}").unwrap();

        let output = dwarf_cmd()
            .arg("--wit")
            .arg(&wit_path)
            .arg("--file")
            .arg(&js_path)
            .arg("--output")
            .arg(dir.path().join("out.wasm"))
            .assert()
            .failure()
            .get_output()
            .stderr
            .clone();
        String::from_utf8(output).unwrap()
    }

    // Missing ';' after `package` - cascades into "expected '{', found
    // keyword `world`".
    let stderr =
        build_and_capture_stderr("package simple:test\n\nworld hello { export greet: func(); }");
    assert!(
        stderr.contains("hint:"),
        "missing ';' after package should get a hint:\n{stderr}"
    );

    // Missing '}' closing an `interface`, cascading into the next `world`.
    let stderr = build_and_capture_stderr(
        "package simple:test;\n\ninterface types {\n  record point {\n    x: u32,\n  }\n\nworld hello { export greet: func(); }",
    );
    assert!(
        stderr.contains("hint:"),
        "missing '}}' on an interface should get a hint:\n{stderr}"
    );

    // Missing '}' closing a `world`, cascading all the way to EOF.
    let stderr =
        build_and_capture_stderr("package simple:test;\n\nworld hello { export greet: func();");
    assert!(
        stderr.contains("hint:"),
        "missing '}}' at EOF should get a hint:\n{stderr}"
    );

    // A genuine typo (misspelled `world`), unrelated to a missing
    // terminator - wit-parser already reports this clearly and correctly
    // located, so it should NOT get the "missing ';'/'}'" hint.
    let stderr =
        build_and_capture_stderr("package simple:test;\n\nwrold hello { export greet: func(); }");
    assert!(
        !stderr.contains("hint:"),
        "a genuine typo should not get the missing-terminator hint:\n{stderr}"
    );
}

#[test]
fn test_cli_creates_the_output_directory() {
    // `-o dist/app.wasm` into a tree that has no `dist/` yet is an ordinary
    // ask. It used to fail at the very end, after the whole build had been
    // done, with an I/O error that named neither the directory nor the fix.
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    let js_path = dir.path().join("app.js");
    fs::write(
        &wit_path,
        "package test:mk;\n\nworld mk { export ping: func() -> string; }\n",
    )
    .unwrap();
    fs::write(&js_path, "export function ping() { return 'pong'; }\n").unwrap();

    let out = dir.path().join("dist").join("nested").join("app.wasm");
    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--output")
        .arg(&out)
        .assert()
        .success();

    assert!(out.exists(), "the component should be at {}", out.display());
}

#[test]
fn test_cli_still_accepts_the_old_js_flag() {
    // `--js`/`-j` were renamed to `--file`/`-f`, since the flag names the
    // build's input and a TypeScript project should not have to say "js" on
    // every invocation. Both spellings stay accepted: scripts, CI jobs and
    // copied-out README lines predating the rename must keep working.
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    let js_path = dir.path().join("app.js");
    fs::write(
        &wit_path,
        "package test:alias;\n\nworld app { export ping: func() -> string; }\n",
    )
    .unwrap();
    fs::write(&js_path, "export function ping() { return 'pong'; }\n").unwrap();

    for flag in ["--file", "--js", "-f", "-j"] {
        dwarf_cmd()
            .arg("--wit")
            .arg(&wit_path)
            .arg(flag)
            .arg(&js_path)
            .arg("--output")
            .arg(dir.path().join(format!("out{}.wasm", flag.trim_matches('-'))))
            .assert()
            .success();
    }
}

#[test]
fn test_cli_prints_build_time_console_log() {
    // A module's top level runs once, at build time, under Wizer - so a
    // `console.log` there is the developer's only window into what their
    // module did while it was being snapshotted. It used to produce nothing
    // at all: the generated console lowers to `wasi:cli/stdout`'s
    // write-via-stream, which needs an active async task, and `init` is a
    // synchronous export, so the throw became an unobserved rejected
    // promise. It now takes `module-loader`'s synchronous `build-log`.
    let dir = TempDir::new().unwrap();
    let js_path = dir.path().join("app.js");
    fs::write(
        &js_path,
        "console.log('BUILD TIME MARKER', 42);\nexport const run = { async run() {} };\n",
    )
    .unwrap();

    let wit_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/wasi-stdio");
    let output = dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--world")
        .arg("wasi-stdio")
        .arg("--file")
        .arg(&js_path)
        .arg("--output")
        .arg(dir.path().join("out.wasm"))
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(output).unwrap();

    assert!(
        stderr.contains("[js stdout] BUILD TIME MARKER 42"),
        "a top-level console.log should reach the build's stderr:\n{stderr}"
    );
}

#[test]
fn test_cli_stub_wasi() {
    let (output, _dir) = run_cli_build(
        "package test:hello;\nworld hello { export add: func(a: u32, b: u32) -> u32; }",
        "export function add(a, b) { return a + b; }",
        &["--stub-wasi"],
    );

    let wasm = fs::read(&output).unwrap();
    let mut inst = ComponentInstance::from_wasm(wasm, vec![], vec![])
        .expect("should instantiate stubbed component");

    assert_eq!(inst.call1("add", &[Val::U32(3), Val::U32(4)]), Val::U32(7));
}

#[test]
fn test_cli_opt_size_runtime() {
    let (output, _dir) = run_cli_build(
        "package test:runtime;\nworld runtime { export add: func(a: u32, b: u32) -> u32; }",
        "export function add(a, b) { return a + b; }",
        &["--opt-size"],
    );

    let wasm = fs::read(&output).unwrap();
    let mut inst = ComponentInstance::from_wasm(wasm, vec![], vec![])
        .expect("should instantiate opt-size runtime component");

    assert_eq!(inst.call1("add", &[Val::U32(3), Val::U32(4)]), Val::U32(7));
}

#[test]
fn test_cli_sync_runtime() {
    let (output, _dir) = run_cli_build(
        "package test:runtime;\nworld runtime { export add: func(a: u32, b: u32) -> u32; }",
        "export function add(a, b) { return a + b; }",
        &["--sync"],
    );

    let wasm = fs::read(&output).unwrap();
    let mut inst = ComponentInstance::from_wasm(wasm, vec![], vec![])
        .expect("should instantiate non-async runtime component");

    assert_eq!(inst.call1("add", &[Val::U32(3), Val::U32(4)]), Val::U32(7));
}

#[test]
fn test_cli_sync_opt_size_runtime() {
    let (output, _dir) = run_cli_build(
        "package test:runtime;\nworld runtime { export add: func(a: u32, b: u32) -> u32; }",
        "export function add(a, b) { return a + b; }",
        &["--sync", "--opt-size"],
    );

    let wasm = fs::read(&output).unwrap();
    let mut inst = ComponentInstance::from_wasm(wasm, vec![], vec![])
        .expect("should instantiate non-async opt-size runtime component");

    assert_eq!(inst.call1("add", &[Val::U32(3), Val::U32(4)]), Val::U32(7));
}

#[test]
fn test_cli_custom_runtime_file() {
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    let js_path = dir.path().join("test.js");
    let output = dir.path().join("output.wasm");
    let runtime_path = dir.path().join("runtime.wasm");

    fs::write(
        &wit_path,
        "package test:runtime;\nworld runtime { export add: func(a: u32, b: u32) -> u32; }",
    )
    .unwrap();
    fs::write(&js_path, "export function add(a, b) { return a + b; }").unwrap();
    fs::write(&runtime_path, dwarf_core::default_runtime_wasm()).unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--output")
        .arg(&output)
        .arg("--runtime")
        .arg(&runtime_path)
        .assert()
        .success();

    let wasm = fs::read(&output).unwrap();
    let mut inst = ComponentInstance::from_wasm(wasm, vec![], vec![])
        .expect("should instantiate custom runtime component");

    assert_eq!(inst.call1("add", &[Val::U32(3), Val::U32(4)]), Val::U32(7));
}

#[test]
fn test_cli_minify() {
    let wit = r#"
        package test:minify;
        world minify-test {
            export add: func(a: u32, b: u32) -> u32;
            export greet: func(name: string) -> string;
        }
    "#;
    let js = r#"
        // This comment and whitespace should be stripped by minification
        // but the logic should remain identical

        /**
         * Foo bar baz.
         */
        export function add(a, b) {
            const result = a + b;
            return result;
        }

        /**
         * Foo bar baz.
         */
        export function greet(name) {
            const greeting = "Hello, " + name + "!";
            return greeting;
        }
    "#;

    let (output, _dir) = run_cli_build(wit, js, &["--minify"]);

    let wasm = fs::read(&output).unwrap();
    let mut inst = ComponentInstance::from_wasm(wasm, vec![], vec![])
        .expect("should instantiate minified component");

    assert_eq!(inst.call1("add", &[Val::U32(3), Val::U32(4)]), Val::U32(7));
    assert_eq!(
        inst.call1("greet", &[Val::String("World".into())]),
        Val::String("Hello, World!".into()),
    );
}

/// Builds with `--emit-types <dir>`, using an ABSOLUTE path for the types
/// output directory - `--emit-types .` would resolve "." against the test
/// binary's own process cwd (the workspace root), not the temp dir, since
/// `assert_cmd::Command` doesn't sandbox `current_dir` - confirmed the hard
/// way: that mistake let a real test run overwrite dwarf's own committed
/// `crates/core/polyfills/*.d.ts` source files with `jco types`' mangled
/// round-trip output (WIT has no optional-parameter syntax, so `init?: Foo`
/// came back as `init: Foo | null`).
fn run_cli_build_with_emit_types(wit: &str, js: &str, extra_args: &[&str]) -> (PathBuf, TempDir) {
    let dir = TempDir::new().unwrap();
    let types_dir = dir.path().join("types");
    fs::create_dir(&types_dir).unwrap();

    let mut args: Vec<&str> = extra_args.to_vec();
    args.push("--emit-types");
    let types_dir_str = types_dir.to_str().unwrap();
    args.push(types_dir_str);

    let wit_path = dir.path().join("test.wit");
    fs::write(&wit_path, wit).unwrap();
    let js_path = dir.path().join("test.js");
    fs::write(&js_path, js).unwrap();
    let output = dir.path().join("output.wasm");

    let mut cmd = dwarf_cmd();
    cmd.arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--output")
        .arg(&output);
    for arg in &args {
        cmd.arg(arg);
    }

    cmd.assert().success();
    (types_dir, dir)
}

#[test]
fn test_cli_emit_types_includes_always_on_globals_and_p3_aliases() {
    let (types_dir, _dir) = run_cli_build_with_emit_types(
        "package test:hello;\nworld hello { export add: func(a: u32, b: u32) -> u32; }",
        "export function add(a, b) { return a + b; }",
        &[],
    );

    let globals = fs::read_to_string(types_dir.join("globals.d.ts"))
        .expect("--emit-types should write globals.d.ts");

    for name in [
        "declare const console: Console;",
        "declare const consoleP3: Console;",
        "declare const process: Process;",
        "declare const processP3: Process;",
        "declare class WebSocketServer {",
        "declare const WebSocketServerP3: typeof WebSocketServer;",
        "declare function setTimeoutP3(",
        "declare function setIntervalP3(",
        "declare function clearTimeoutP3(",
        "declare function clearIntervalP3(",
        "getRandomValuesP3<T extends ArrayBufferView>(typedArray: T): T;",
    ] {
        assert!(
            globals.contains(name),
            "globals.d.ts missing `{name}`:\n{globals}"
        );
    }
}

#[test]
fn test_cli_emit_types_fetch_classes_includes_fetch_p3() {
    let (types_dir, _dir) = run_cli_build_with_emit_types(
        "package test:hello;\nworld hello { export add: func(a: u32, b: u32) -> u32; }",
        "export function add(a, b) { return a + b; }",
        &["--polyfill", "fetch-classes"],
    );

    let globals = fs::read_to_string(types_dir.join("globals.d.ts"))
        .expect("--emit-types should write globals.d.ts");

    assert!(globals.contains(
        "declare function fetch(input: string | Request, init?: RequestInit): Promise<Response>;"
    ));
    assert!(globals.contains(
        "declare function fetchP3(input: string | Request, init?: RequestInit): Promise<Response>;"
    ));
}

#[test]
fn test_cli_minify_reports_parse_errors() {
    // Adapted from componentize-qjs #70, which found the same thing dwarf
    // did: the minifier used to accept whatever partial AST oxc returned
    // after a parse error, and emit a program missing its exports.
    let dir = TempDir::new().unwrap();
    let wit_path = dir.path().join("test.wit");
    let js_path = dir.path().join("test.js");

    fs::write(
        &wit_path,
        "package test:minify-error;\nworld minify-error { export run: func(); }",
    )
    .unwrap();
    fs::write(&js_path, "export function run( {").unwrap();

    dwarf_cmd()
        .arg("--wit")
        .arg(&wit_path)
        .arg("--file")
        .arg(&js_path)
        .arg("--minify")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--minify"));
}

#[test]
fn test_cli_explains_a_wit_specifier_that_is_not_an_interface() {
    // The failure that prompted this: a world-level function import
    // (`import wedge: func();`) is not a module, and importing the WORLD's
    // name reached the filesystem resolver, which reported "filesystem
    // module not found" - true, and useless, because nobody was looking
    // for a file. Neither did it mention that dwarf puts world-level
    // imports on globalThis, which is the actual fix.
    let dir = TempDir::new().unwrap();
    let wit_dir = dir.path().join("wit");
    fs::create_dir(&wit_dir).unwrap();
    fs::write(
        wit_dir.join("world.wit"),
        "package spike:reentry;\n\nworld guest {\n  import wedge: func();\n  export poke: func() -> u32;\n}\n",
    )
    .unwrap();
    let js_path = dir.path().join("main.js");
    fs::write(
        &js_path,
        "import { wedge } from 'spike:reentry/guest';\nexport function poke() { return 42; }\n",
    )
    .unwrap();

    let stderr = dwarf_cmd()
        .arg("--wit")
        .arg(&wit_dir)
        .arg("--file")
        .arg(&js_path)
        .arg("--world")
        .arg("guest")
        .arg("--output")
        .arg(dir.path().join("out.wasm"))
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(stderr).unwrap();

    assert!(
        !stderr.contains("filesystem module not found"),
        "a WIT specifier should not be reported as a missing file:\n{stderr}"
    );
    assert!(
        stderr.contains("is not a WIT interface this world imports"),
        "should name the actual problem:\n{stderr}"
    );
    assert!(
        stderr.contains("globalThis") && stderr.contains("wedge"),
        "should say world-level imports are globals, and name this one:\n{stderr}"
    );
    // The thrown exception is truncated by QuickJS at 256 bytes, so the
    // one-line cause has to carry the fix on its own.
    assert!(
        stderr.contains("call wedge() with no import"),
        "the exception itself should carry the remedy:\n{stderr}"
    );
}
