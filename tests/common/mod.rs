//! Shared test harness for dwarf integration tests.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use tempfile::TempDir;
use wasmtime::component::{Component, Instance, Linker, ResourceTable, Val};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use dwarf_core::{ComponentizeOpts, Runtime, ScriptcConfig};

pub struct WasiCtxState {
    pub wasi: WasiCtx,
    pub table: ResourceTable,
}

impl WasiView for WasiCtxState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        #[cfg(feature = "component-model-async")]
        config.wasm_component_model_async(true);
        config.wasm_component_model(true);
        Engine::new(&config).expect("Failed to create engine")
    })
}

pub fn async_engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.wasm_component_model_async(true);
        config.wasm_component_model_async_stackful(true);
        Engine::new(&config).expect("Failed to create async engine")
    })
}

pub struct Expectation {
    pub func_name: String,
    pub params: Vec<Val>,
    pub expected: Val,
}

/// Builder for constructing and running component tests.
pub struct TestCase {
    wit: Option<String>,
    wit_dir: Option<PathBuf>,
    world_name: Option<String>,
    script: Option<String>,
    stub_wasi: bool,
    env_vars: Vec<(String, String)>,
    stdin: Option<String>,
    expectations: Vec<Expectation>,
    polyfills: Vec<String>,
    scriptc_profiles: Vec<PathBuf>,
}

/// The scriptc executable for --scriptc tests. scriptc is not published, so
/// tests that need it name it through DWARF_TEST_SCRIPTC and skip when it is
/// absent (see `scriptc_available`).
pub fn scriptc_bin() -> Option<PathBuf> {
    std::env::var_os("DWARF_TEST_SCRIPTC").map(PathBuf::from)
}

/// Whether the statically-compiled-module tests can run here.
pub fn scriptc_available() -> bool {
    scriptc_bin().is_some_and(|p| p.exists())
}

impl TestCase {
    pub fn new() -> Self {
        Self {
            wit: None,
            wit_dir: None,
            world_name: None,
            script: None,
            stub_wasi: false,
            scriptc_profiles: Vec::new(),
            env_vars: Vec::new(),
            stdin: None,
            expectations: Vec::new(),
            polyfills: Vec::new(),
        }
    }

    /// Set inline WIT source (written to a temp file).
    pub fn wit(mut self, wit: &str) -> Self {
        self.wit = Some(wit.to_string());
        self
    }

    /// Set path to a WIT directory (with deps/).
    pub fn wit_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.wit_dir = Some(path.into());
        self
    }

    /// Select a specific world from the WIT package.
    pub fn world(mut self, name: &str) -> Self {
        self.world_name = Some(name.to_string());
        self
    }

    pub fn script(mut self, js: &str) -> Self {
        self.script = Some(js.to_string());
        self
    }

    pub fn stub_wasi(mut self) -> Self {
        self.stub_wasi = true;
        self
    }

    /// Include static (non-WASI) polyfills, e.g. `["webcrypto"]`.
    pub fn polyfills(mut self, names: &[&str]) -> Self {
        self.polyfills = names.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Compile a module statically with scriptc and plug it in.
    pub fn scriptc(mut self, profile: PathBuf) -> Self {
        self.scriptc_profiles.push(profile);
        self
    }

    /// Add an environment variable visible to the WASI context.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env_vars.push((key.to_string(), value.to_string()));
        self
    }

    /// Set bytes visible through WASI stdin.
    pub fn stdin(mut self, input: &str) -> Self {
        self.stdin = Some(input.to_string());
        self
    }

    /// Register an expected function call: name, params, and expected return value.
    pub fn expect_call(mut self, name: &str, params: Vec<Val>, expected: Val) -> Self {
        self.expectations.push(Expectation {
            func_name: name.to_string(),
            params,
            expected,
        });
        self
    }

    /// Build the component and return a live instance ready for calls.
    pub fn build(self) -> anyhow::Result<ComponentInstance> {
        let dir = TempDir::new()?;

        let wit_path = if let Some(ref wit_dir) = self.wit_dir {
            wit_dir.clone()
        } else {
            let p = dir.path().join("test.wit");
            fs::write(&p, self.wit.as_deref().unwrap())?;
            p
        };

        let polyfill_refs: Vec<&str> = self.polyfills.iter().map(String::as_str).collect();
        let opts = ComponentizeOpts {
            wit_path: &wit_path,
            js_source: self.script.as_deref().unwrap(),
            js_path: None,
            module_root: None,
            world_name: self.world_name.as_deref(),
            stub_wasi: self.stub_wasi,
            auto_vendor: false,
            polyfills: &polyfill_refs,
            disable_gc: false,
            runtime: Runtime::Default,
        };

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let scriptc_path = scriptc_bin();
        let scriptc = ScriptcConfig {
            profiles: &self.scriptc_profiles,
            bin: scriptc_path.as_deref(),
        };
        let wasm = rt.block_on(dwarf_core::componentize_with(&opts, &scriptc))?;
        ComponentInstance::from_wasm_with_stdin(wasm, self.env_vars, self.stdin, self.expectations)
    }

    /// Build the component and return an async-capable instance.
    pub async fn build_async(self) -> anyhow::Result<AsyncComponentInstance> {
        let dir = TempDir::new()?;

        let wit_path = if let Some(ref wit_dir) = self.wit_dir {
            wit_dir.clone()
        } else {
            let p = dir.path().join("test.wit");
            fs::write(&p, self.wit.as_deref().unwrap())?;
            p
        };

        let polyfill_refs: Vec<&str> = self.polyfills.iter().map(String::as_str).collect();
        let opts = ComponentizeOpts {
            wit_path: &wit_path,
            js_source: self.script.as_deref().unwrap(),
            js_path: None,
            module_root: None,
            world_name: self.world_name.as_deref(),
            stub_wasi: self.stub_wasi,
            auto_vendor: false,
            polyfills: &polyfill_refs,
            disable_gc: false,
            runtime: Runtime::Default,
        };

        let wasm = dwarf_core::componentize(&opts).await?;

        AsyncComponentInstance::from_wasm_with_stdin(wasm, self.env_vars, self.stdin).await
    }
}

pub struct ComponentInstance {
    /// The component's own bytes, for tests that inspect the artifact.
    pub wasm: Vec<u8>,
    store: Store<WasiCtxState>,
    inner: Instance,
    stdout: MemoryOutputPipe,
    expectations: Vec<Expectation>,
}

impl ComponentInstance {
    /// Instantiate a component from pre-built wasm bytes.
    pub fn from_wasm(
        wasm: Vec<u8>,
        env_vars: Vec<(String, String)>,
        expectations: Vec<Expectation>,
    ) -> anyhow::Result<Self> {
        Self::from_wasm_with_stdin(wasm, env_vars, None, expectations)
    }

    pub fn from_wasm_with_stdin(
        wasm: Vec<u8>,
        env_vars: Vec<(String, String)>,
        stdin: Option<String>,
        expectations: Vec<Expectation>,
    ) -> anyhow::Result<Self> {
        let engine = engine();
        let component = Component::new(engine, &wasm)?;

        let mut wasi_builder = WasiCtxBuilder::new();
        if !env_vars.is_empty() {
            wasi_builder.inherit_env();
            for (k, v) in &env_vars {
                wasi_builder.env(k, v);
            }
        }
        let stdout = MemoryOutputPipe::new(10000);
        wasi_builder
            .stdin(MemoryInputPipe::new(stdin.unwrap_or_default()))
            .stdout(stdout.clone());
        let wasi = wasi_builder.build();
        let table = ResourceTable::new();
        let mut store = Store::new(engine, WasiCtxState { wasi, table });

        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

        let instance = linker.instantiate(&mut store, &component)?;

        Ok(ComponentInstance {
            wasm,
            store,
            inner: instance,
            stdout,
            expectations,
        })
    }

    /// Call an exported function with the given params and return results.
    pub fn call(&mut self, name: &str, params: &[Val], result_count: usize) -> Vec<Val> {
        let func = self
            .inner
            .get_func(&mut self.store, name)
            .unwrap_or_else(|| panic!("export `{name}` not found"));

        let mut results = vec![Val::Bool(false); result_count];
        func.call(&mut self.store, params, &mut results)
            .unwrap_or_else(|e| panic!("calling `{name}` failed: {e}"));

        results
    }

    /// Call an exported function expecting a single return value.
    pub fn call1(&mut self, name: &str, params: &[Val]) -> Val {
        self.call(name, params, 1).into_iter().next().unwrap()
    }

    /// Run all registered expectations, asserting each call matches.
    pub fn run(&mut self) {
        let expectations = std::mem::take(&mut self.expectations);

        for exp in expectations {
            let result = self.call1(&exp.func_name, &exp.params);
            assert_eq!(
                result, exp.expected,
                "calling `{}`: expected {:?}, got {:?}",
                exp.func_name, exp.expected, result
            );
        }
    }

    pub fn stdout_bytes(&self) -> Vec<u8> {
        self.stdout.contents().to_vec()
    }

    /// Get the wasmtime instance and store for typed/interface function access.
    pub fn parts(&mut self) -> (&Instance, &mut Store<WasiCtxState>) {
        (&self.inner, &mut self.store)
    }
}

pub fn dwarf_cmd() -> assert_cmd::Command {
    assert_cmd::cargo::cargo_bin_cmd!("dwarf")
}

pub struct AsyncComponentInstance {
    store: Store<WasiCtxState>,
    inner: Instance,
    stdout: MemoryOutputPipe,
    stderr: MemoryOutputPipe,
}

impl AsyncComponentInstance {
    pub async fn from_wasm(wasm: Vec<u8>, env_vars: Vec<(String, String)>) -> anyhow::Result<Self> {
        Self::from_wasm_with_stdin(wasm, env_vars, None).await
    }

    pub async fn from_wasm_with_stdin(
        wasm: Vec<u8>,
        env_vars: Vec<(String, String)>,
        stdin: Option<String>,
    ) -> anyhow::Result<Self> {
        let engine = async_engine();
        let component = Component::new(engine, &wasm)?;

        let mut wasi_builder = WasiCtxBuilder::new();
        if !env_vars.is_empty() {
            wasi_builder.inherit_env();
            for (k, v) in &env_vars {
                wasi_builder.env(k, v);
            }
        }
        let stdout = MemoryOutputPipe::new(10000);
        let stderr = MemoryOutputPipe::new(10000);
        wasi_builder
            .stdin(MemoryInputPipe::new(stdin.unwrap_or_default()))
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            // Needed for tests that bind/listen/connect real sockets (e.g.
            // websocket.rs, sockprobe in async_resource.rs) - without this,
            // wasmtime-wasi's wasi:sockets host implementation denies every
            // socket operation by default.
            .inherit_network();
        let wasi = wasi_builder.build();
        let table = ResourceTable::new();
        let mut store = Store::new(engine, WasiCtxState { wasi, table });

        let mut linker = Linker::new(engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        wasmtime_wasi::p3::add_to_linker(&mut linker)?;

        let instance = linker.instantiate_async(&mut store, &component).await?;

        Ok(AsyncComponentInstance {
            store,
            inner: instance,
            stdout,
            stderr,
        })
    }

    /// Call an exported async function with the given params and return results.
    pub async fn call_async(
        &mut self,
        name: &str,
        params: &[Val],
        result_count: usize,
    ) -> anyhow::Result<Vec<Val>> {
        let func = self
            .inner
            .get_func(&mut self.store, name)
            .unwrap_or_else(|| panic!("export `{name}` not found"));

        let mut results = vec![Val::Bool(false); result_count];
        func.call_async(&mut self.store, params, &mut results)
            .await?;

        Ok(results)
    }

    /// Call an exported async function expecting a single return value.
    pub async fn call1_async(&mut self, name: &str, params: &[Val]) -> anyhow::Result<Val> {
        Ok(self
            .call_async(name, params, 1)
            .await?
            .into_iter()
            .next()
            .unwrap())
    }

    /// Get the wasmtime instance and store for typed function access.
    pub fn parts(&mut self) -> (&Instance, &mut Store<WasiCtxState>) {
        (&self.inner, &mut self.store)
    }

    pub fn stdout_bytes(&self) -> Vec<u8> {
        self.stdout.contents().to_vec()
    }

    pub fn stderr_bytes(&self) -> Vec<u8> {
        self.stderr.contents().to_vec()
    }
}

/// Write WIT + JS to a temp dir, run the CLI, return the output wasm path and temp dir.
pub fn run_cli_build(wit: &str, js: &str, extra_args: &[&str]) -> (PathBuf, TempDir) {
    let dir = TempDir::new().unwrap();

    let wit_path = dir.path().join("test.wit");
    fs::write(&wit_path, wit).unwrap();

    let js_path = dir.path().join("test.js");
    fs::write(&js_path, js).unwrap();

    let output = dir.path().join("output.wasm");

    let mut cmd = dwarf_cmd();
    cmd.arg("--wit")
        .arg(&wit_path)
        .arg("--js")
        .arg(&js_path)
        .arg("--output")
        .arg(&output);

    for arg in extra_args {
        cmd.arg(arg);
    }

    cmd.assert().success();
    assert!(output.exists(), "Output wasm file should exist");
    (output, dir)
}

pub fn wasi_wit_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wit")
}
