pub mod codegen;
pub mod polyfills;
mod resolver;
pub mod stubwasi;
pub mod types;
mod wit;

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
    /// Automatically fetch missing WIT dependencies with `wkg wit fetch` when
    /// `wit_path` is a directory and resolution fails on an unresolved package
    pub auto_vendor: bool,
    /// Names of static (non-WASI) polyfills to include, e.g. `["buffer"]` -
    /// see `polyfills::POLYFILLS` for the available set
    pub polyfills: &'a [&'a str],
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
    let (resolve, pkg_id) = wit::resolve_wit(opts.wit_path, opts.auto_vendor)?;
    let world_id = resolve.select_world(&[pkg_id], opts.world_name)?;

    let mut shim = codegen::generate_shim(&resolve, world_id);
    shim.push_str(&polyfills::resolve_shim_suffix(opts.polyfills)?);
    let resolver = module_resolution(opts)?;
    let mut wit_dylib = wit_dylib::create(&resolve, world_id, None);

    // wit_dylib is freshly generated per WIT world (unlike the vendored QuickJS
    // runtime, which is a fixed blob wasm-opt barely touches - see
    // periapsis's wasm-opt experiment notes), so it's the one piece of a dwarf
    // component actually worth running through wasm-opt: ~30% smaller in
    // practice. Must run before embed_component_metadata below, since wasm-opt
    // doesn't preserve the "component-type" custom section that call adds.
    wit_dylib = optimize_wasm(&wit_dylib).context("failed to wasm-opt the wit-dylib module")?;

    wit_component::embed_component_metadata(
        &mut wit_dylib,
        &resolve,
        world_id,
        wit_component::StringEncoding::UTF8,
    )?;

    let pre_wizer_component = wit_component::Linker::default()
        .validate(true)
        .library("dwarf_runtime.wasm", runtime_wasm(opts.runtime), false)?
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

    let mut producers = wasm_metadata::Producers::empty();
    producers.add("processed-by", "dwarf", env!("CARGO_PKG_VERSION"));
    component = producers
        .add_to_wasm(&component)
        .context("failed to tag component with dwarf producers metadata")?;

    Ok(component)
}

/// Run binaryen's wasm-opt (-O3, all wasm features enabled) over a core wasm
/// module's bytes. `wasm-opt`'s only public API is file-to-file, so this
/// round-trips through a temp dir.
fn optimize_wasm(bytes: &[u8]) -> Result<Vec<u8>> {
    let dir = tempfile::tempdir()?;
    let in_path = dir.path().join("in.wasm");
    let out_path = dir.path().join("out.wasm");
    std::fs::write(&in_path, bytes)?;

    let mut opts = wasm_opt::OptimizationOptions::new_opt_level_3();
    opts.features.baseline = wasm_opt::FeatureBaseline::All;
    opts.run(&in_path, &out_path)
        .map_err(|e| anyhow!("wasm-opt failed: {e}"))?;

    Ok(std::fs::read(&out_path)?)
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
    // Wraps both failure paths - a trap during the call itself (e.g. a panic
    // in the guest, which surfaces as a raw wasmtime error from `.await?`)
    // and a graceful `result<_, string>` error from the WIT function - in
    // the same `.with_context`, so a crash during the guest's own top-level
    // module evaluation still shows whatever it printed to stdout/stderr
    // first (a Rust panic message, a console.log, etc.) instead of only a
    // bare wasm backtrace.
    async {
        init.call_init(
            &mut store,
            shim,
            js,
            resolver.as_ref().map(Resolver::entry_path),
            disable_gc,
        )
        .await?
        .map_err(|e| anyhow!("{e}"))
    }
    .await
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

    // WASI 0.2 interfaces (http, filesystem, sockets, cli) re-export
    // `wasi:io/streams`'s `input-stream`/`output-stream`, `wasi:io/poll`'s
    // `pollable`, and `wasi:io/error`'s `error` via `use` (sometimes under a
    // renamed local alias, e.g. `wasi:http/types`'s `error as io-error`).
    // wasmtime's component type system resolves these to the *same*
    // resource identity as their `wasi:io/*` origin (confirmed empirically:
    // `ComponentItem::Resource`'s inner `ResourceType` compares equal across
    // sites), but `wasmtime_wasi::p2::add_to_linker_async` only registers a
    // real implementation under the literal `wasi:io/streams`/`wasi:io/poll`/
    // `wasi:io/error` instance paths — any *other* path needing the same
    // resource (e.g. `wasi:http/types`) is left on our stub's generic
    // `ResourceType::host::<()>()`, which then mismatches the real type
    // registered elsewhere for that same resource, and instantiation fails
    // with "mismatched resource types" (wasmtime's own
    // `Linker::define_unknown_imports_as_traps` has the identical gap — it
    // only skips a name already defined at the *same* linker path, not
    // aliases elsewhere). Fixed by pre-scanning for the canonical
    // `wasi:io/*` sighting of each such resource and, everywhere else that
    // resource's identity recurs, stubbing it with wasmtime-wasi-io's own
    // public resource type (the exact one its bindgen `with:` map uses) so
    // it's TypeId-identical to whatever `add_to_linker_async` later
    // registers at the canonical path.
    let mut known_resources = Vec::new();
    for (name, ext) in &imports {
        collect_known_wasi_io_resources(engine, name, &ext.ty, None, &mut known_resources);
    }

    let mut root = linker.root();
    for (name, ext) in imports {
        stub_item(&mut root, engine, name, &ext.ty, None, &known_resources)?;
    }
    Ok(())
}

fn known_wasi_io_resource(
    parent_instance: &str,
    item_name: &str,
) -> Option<wasmtime::component::ResourceType> {
    use wasmtime::component::ResourceType;

    if !parent_instance.starts_with("wasi:io/") {
        return None;
    }
    match item_name {
        "input-stream" if parent_instance.starts_with("wasi:io/streams@") => {
            Some(ResourceType::host::<
                wasmtime_wasi_io::streams::DynInputStream,
            >())
        }
        "output-stream" if parent_instance.starts_with("wasi:io/streams@") => {
            Some(ResourceType::host::<
                wasmtime_wasi_io::streams::DynOutputStream,
            >())
        }
        "pollable" if parent_instance.starts_with("wasi:io/poll@") => {
            Some(ResourceType::host::<wasmtime_wasi_io::poll::DynPollable>())
        }
        "error" if parent_instance.starts_with("wasi:io/error@") => {
            Some(ResourceType::host::<wasmtime_wasi_io::streams::Error>())
        }
        _ => None,
    }
}

fn collect_known_wasi_io_resources(
    engine: &Engine,
    item_name: &str,
    item_ty: &wasmtime::component::types::ComponentItem,
    parent_instance: Option<&str>,
    out: &mut Vec<(
        wasmtime::component::ResourceType,
        wasmtime::component::ResourceType,
    )>,
) {
    use wasmtime::component::types::ComponentItem;

    match item_ty {
        ComponentItem::ComponentInstance(inst) => {
            for (export_name, export) in inst.exports(engine) {
                collect_known_wasi_io_resources(
                    engine,
                    export_name,
                    &export.ty,
                    Some(item_name),
                    out,
                );
            }
        }
        ComponentItem::Resource(rt) => {
            if let Some(parent) = parent_instance
                && let Some(host_ty) = known_wasi_io_resource(parent, item_name)
            {
                out.push((*rt, host_ty));
            }
        }
        _ => {}
    }
}

fn stub_item<T: Send + 'static>(
    linker: &mut wasmtime::component::LinkerInstance<'_, T>,
    engine: &Engine,
    item_name: &str,
    item_ty: &wasmtime::component::types::ComponentItem,
    parent_instance: Option<&str>,
    known_resources: &[(
        wasmtime::component::ResourceType,
        wasmtime::component::ResourceType,
    )],
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
                stub_item(
                    &mut sub,
                    engine,
                    export_name,
                    &export.ty,
                    Some(item_name),
                    known_resources,
                )?;
            }
        }
        ComponentItem::Resource(rt) => {
            let host_ty = known_resources
                .iter()
                .find(|(component_ty, _)| component_ty == rt)
                .map(|(_, host_ty)| *host_ty)
                .unwrap_or_else(wasmtime::component::ResourceType::host::<()>);
            linker.resource(item_name, host_ty, |_, _| Ok(()))?;
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
