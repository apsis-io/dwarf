pub mod codegen;
mod resolver;
pub mod stubwasi;

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use resolver::Resolver;
use stubwasi::{stub_internal_imports, stub_wasi_imports};
use wasi_preview1_component_adapter_provider::WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER;
use wasmtime::component::{Component as WasmtimeComponent, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wizer::{WasmtimeWizerComponent, Wizer};
use wit_parser::Resolve;

include!(concat!(env!("OUT_DIR"), "/output.rs"));

wasmtime::component::bindgen!({
    path: "wit/init.wit",
    world: "init",
    exports: { default: async },
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

/// Options for componentizing a JavaScript source file.
pub struct ComponentizeOpts<'a> {
    /// Path to the WIT file or directory
    pub wit_path: &'a Path,
    /// JavaScript source code
    pub js_source: &'a str,
    /// Path to the JavaScript entry file, used as the base for resolving imports
    pub js_path: Option<&'a Path>,
    /// Host directory exposed read-only during Wizer for resolving imported modules
    pub module_root: Option<&'a Path>,
    /// World name to use from the WIT (None = default world)
    pub world_name: Option<&'a str>,
    /// Stub all WASI imports with traps
    pub stub_wasi: bool,
    /// Disable automatic garbage collection in the QuickJS runtime
    pub disable_gc: bool,
    /// Runtime to embed before Wizer initialization
    pub runtime: Runtime<'a>,
}

/// QuickJS runtime variant to embed in the generated component.
#[derive(Clone, Copy, Debug)]
pub enum Runtime<'a> {
    /// Standard runtime optimized for speed.
    ///
    /// Built with component-model async support when the `component-model-async`
    /// feature is enabled (the default); otherwise this is the non-async runtime.
    Default,
    /// Runtime optimized for smaller generated components.
    ///
    /// Built with component-model async support when the `component-model-async`
    /// feature is enabled (the default); otherwise this is the non-async runtime.
    OptSize,
    /// Non-async runtime optimized for speed.
    ///
    /// Produces components that do not use the component-model async ABI, so they
    /// run on hosts without async support. Always available regardless of Cargo
    /// features.
    DefaultSync,
    /// Non-async runtime optimized for smaller generated components.
    ///
    /// Produces components that do not use the component-model async ABI, so they
    /// run on hosts without async support. Always available regardless of Cargo
    /// features.
    OptSizeSync,
    /// Caller-provided runtime Wasm bytes.
    Custom(&'a [u8]),
}

impl Default for Runtime<'_> {
    fn default() -> Self {
        default_builtin_runtime()
    }
}

/// Return the built-in runtime selected by Cargo features.
pub fn default_builtin_runtime() -> Runtime<'static> {
    if cfg!(feature = "opt-size") {
        Runtime::OptSize
    } else {
        Runtime::Default
    }
}

/// Convert JavaScript source code into a WebAssembly component.
pub async fn componentize(opts: &ComponentizeOpts<'_>) -> Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let (pkg_id, _) = resolve.push_path(opts.wit_path)?;
    let world_id = resolve.select_world(&[pkg_id], opts.world_name)?;

    let shim = codegen::generate_shim(&resolve, world_id);
    let resolver = module_resolution(opts)?;
    let mut wit_dylib = wit_dylib::create(&resolve, world_id, None);

    wit_component::embed_component_metadata(
        &mut wit_dylib,
        &resolve,
        world_id,
        wit_component::StringEncoding::UTF8,
    )?;

    let pre_wizer_component = wit_component::Linker::default()
        .validate(true)
        .library(
            "dwarf_runtime.wasm",
            runtime_wasm(opts.runtime),
            false,
        )?
        .library("wit-dylib.wasm", &wit_dylib, false)?
        .adapter(
            "wasi_snapshot_preview1",
            WASI_SNAPSHOT_PREVIEW1_REACTOR_ADAPTER,
        )?
        .encode()
        .context("failed to link and encode component")?;

    let mut component = wizer_init(
        &pre_wizer_component,
        &shim,
        opts.js_source,
        resolver,
        opts.disable_gc,
    )
    .await?;

    component = stub_internal_imports(&component)
        .context("failed to stub internal module-loader import")?;

    if opts.stub_wasi {
        component = stub_wasi_imports(&component).context("failed to stub WASI imports")?;
    }

    Ok(component)
}

fn module_resolution(opts: &ComponentizeOpts<'_>) -> Result<Option<Resolver>> {
    let Some(js_path) = opts.js_path else {
        if opts.module_root.is_some() {
            return Err(anyhow!("module_root requires js_path"));
        }
        return Ok(None);
    };

    Resolver::new(js_path, opts.module_root).map(Some)
}

/// Return the built-in default runtime Wasm bytes.
pub fn default_runtime_wasm() -> &'static [u8] {
    DEFAULT_RUNTIME_WASM
}

/// Return the built-in opt-size runtime Wasm bytes.
pub fn opt_size_runtime_wasm() -> &'static [u8] {
    OPT_SIZE_RUNTIME_WASM
}

/// Return the built-in non-async runtime Wasm bytes.
pub fn default_sync_runtime_wasm() -> &'static [u8] {
    DEFAULT_SYNC_RUNTIME_WASM
}

/// Return the built-in non-async opt-size runtime Wasm bytes.
pub fn opt_size_sync_runtime_wasm() -> &'static [u8] {
    OPT_SIZE_SYNC_RUNTIME_WASM
}

fn runtime_wasm(runtime: Runtime<'_>) -> &[u8] {
    match runtime {
        Runtime::Default => DEFAULT_RUNTIME_WASM,
        Runtime::OptSize => OPT_SIZE_RUNTIME_WASM,
        Runtime::DefaultSync => DEFAULT_SYNC_RUNTIME_WASM,
        Runtime::OptSizeSync => OPT_SIZE_SYNC_RUNTIME_WASM,
        Runtime::Custom(wasm) => wasm,
    }
}

async fn wizer_init(
    component: &[u8],
    shim: &str,
    js: &str,
    resolver: Option<Resolver>,
    disable_gc: bool,
) -> Result<Vec<u8>> {
    let stdout = MemoryOutputPipe::new(10000);
    let stderr = MemoryOutputPipe::new(10000);

    let wasi = WasiCtxBuilder::new()
        .stdin(MemoryInputPipe::new(Bytes::new()))
        .stdout(stdout.clone())
        .stderr(stderr.clone())
        .build();

    let table = ResourceTable::new();
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.wasm_component_model_async(true);

    let engine = Engine::new(&config)?;
    let mut store = Store::new(&engine, Ctx { wasi, table });

    let wizer = Wizer::new();
    let (cx, instrumented) = wizer.instrument_component(component)?;
    let comp = WasmtimeComponent::new(&engine, &instrumented)?;

    let mut linker = Linker::new(&engine);
    linker.allow_shadowing(true);
    define_unknown_imports_as_traps_async(&mut linker, &engine, &comp)?;
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi::p3::add_to_linker(&mut linker)?;

    register_module_loader(&mut linker, resolver.clone())?;

    let instance = linker.instantiate_async(&mut store, &comp).await?;
    let init = Init::new(&mut store, &instance)?;
    init.call_init(
        &mut store,
        shim,
        js,
        resolver.as_ref().map(Resolver::entry_path),
        disable_gc,
    )
    .await?
    .map_err(|e| anyhow!("{e}"))
    .with_context(move || {
        format!(
            "{}{}",
            String::from_utf8_lossy(&stdout.contents()),
            String::from_utf8_lossy(&stderr.contents())
        )
    })?;

    let component = wizer
        .snapshot_component(
            cx,
            &mut WasmtimeWizerComponent {
                store: &mut store,
                instance,
            },
        )
        .await?;

    Ok(component)
}

/// Like `Linker::define_unknown_imports_as_traps`, but async-aware.
///
/// wasmtime's own `define_unknown_imports_as_traps` stubs every unsatisfied
/// import with a *synchronous* trapping function (`LinkerInstance::func_new`)
/// regardless of whether the component's WIT declares that import `async`.
/// For a component whose world imports a user-defined `async func` with no
/// real host implementation (i.e. it isn't wasi:* and isn't one of dwarf's
/// own host intrinsics — both linked *after* this stub pass, overwriting it),
/// the stub's sync-ness never matches the real (async) type wasmtime records
/// for that import, so instantiation fails with "type mismatch with async"
/// before Wizer ever gets to run the component's JS init. Confirmed via a
/// minimal repro with a non-resource `u32` parameter that the failure is
/// about the stub's async-ness, not anything resource-specific.
///
/// `allow_shadowing(true)` is set on `linker` by the caller, so it's safe to
/// stub unconditionally here (including things wasi/dwarf's own host
/// intrinsics will define for real immediately afterward) rather than
/// replicating wasmtime's internal "skip if already defined" check, which
/// isn't reachable through the public API anyway.
fn define_unknown_imports_as_traps_async(
    linker: &mut Linker<Ctx>,
    engine: &Engine,
    comp: &WasmtimeComponent,
) -> Result<()> {
    let ty = comp.component_type();
    let imports: Vec<_> = ty.imports(engine).collect();
    let mut root = linker.root();
    for (name, ext) in imports {
        stub_item(&mut root, engine, name, &ext.ty, None)?;
    }
    Ok(())
}

fn stub_item<T: Send + 'static>(
    linker: &mut wasmtime::component::LinkerInstance<'_, T>,
    engine: &Engine,
    item_name: &str,
    item_ty: &wasmtime::component::types::ComponentItem,
    parent_instance: Option<&str>,
) -> Result<()> {
    use wasmtime::component::types::ComponentItem;

    match item_ty {
        ComponentItem::ComponentFunc(f) => {
            let full_name = match parent_instance {
                Some(parent) => format!("{parent}#{item_name}"),
                None => item_name.to_string(),
            };
            if f.async_() {
                // NOT `func_new_async`: its dynamic typecheck hardcodes
                // `DynamicHostFn<_, false>` regardless of the function's actual
                // async-ness (confirmed by reading wasmtime 46.0.1's
                // `HostFunc::func_new_async` source — both it and `func_new`
                // construct `DynamicHostFn::<_, false>`, only `Asyncness`, a
                // separate field unrelated to typechecking, differs). Since
                // `DynamicHostFn::<F, ASYNC>::typecheck` bails unless
                // `ASYNC == ty.async_`, a `func_new_async` stub can never
                // satisfy a component function whose WIT type is `async func`
                // — exactly the "type mismatch with async" this replaces.
                // `func_new_concurrent` constructs `DynamicHostFn::<_, true>`
                // instead, which does typecheck correctly against an async
                // import. Requires `Config::concurrency_support` (defaults to
                // `true`, not changed here).
                linker.func_new_concurrent(item_name, move |_, _, _, _| {
                    let full_name = full_name.clone();
                    Box::pin(async move {
                        wasmtime::bail!("unknown import: `{full_name}` has not been defined")
                    })
                })?;
            } else {
                linker.func_new(item_name, move |_, _, _, _| {
                    wasmtime::bail!("unknown import: `{full_name}` has not been defined")
                })?;
            }
        }
        ComponentItem::ComponentInstance(inst) => {
            let mut sub = linker.instance(item_name)?;
            for (export_name, export) in inst.exports(engine) {
                stub_item(&mut sub, engine, export_name, &export.ty, Some(item_name))?;
            }
        }
        ComponentItem::Resource(_) => {
            linker.resource(
                item_name,
                wasmtime::component::ResourceType::host::<()>(),
                |_, _| Ok(()),
            )?;
        }
        ComponentItem::Component(_) | ComponentItem::Module(_) => {
            anyhow::bail!("unable to define {item_name} imports as traps")
        }
        _ => {}
    }
    Ok(())
}

fn register_module_loader(linker: &mut Linker<Ctx>, resolver: Option<Resolver>) -> Result<()> {
    let resolve = resolver.clone();
    let load = resolver;

    let mut instance = linker.instance("local:init/module-loader")?;
    instance.func_wrap(
        "resolve",
        move |_, (referrer, specifier): (String, String)| -> wasmtime::Result<_> {
            let result = resolve.as_ref().map_or_else(
                || {
                    Err(
                        "filesystem module not found: module resolution requires js_path"
                            .to_string(),
                    )
                },
                |resolver| {
                    resolver
                        .resolve(&referrer, &specifier)
                        .map_err(|err| err.to_string())
                },
            );
            Ok((result,))
        },
    )?;
    instance.func_wrap(
        "load",
        move |_, (path,): (String,)| -> wasmtime::Result<_> {
            let result = load.as_ref().map_or_else(
                || Err("filesystem module not found: module loading requires js_path".to_string()),
                |resolver| resolver.load(&path).map_err(|err| err.to_string()),
            );
            Ok((result,))
        },
    )?;

    Ok(())
}
