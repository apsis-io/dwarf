mod abi;
mod bindings;
mod buffer;
mod call;
mod futures;
mod interpreter;
mod module;
mod resources;
mod result;
mod streams;
mod tagged;
mod task;
mod trivia;
mod wit_imports;

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::hash_map::DefaultHasher;

use rquickjs::runtime::UserDataGuard;
use rquickjs::{Context, JsLifetime, Persistent, Runtime, Value, function};
use smallvec::SmallVec;
use task::TaskState;
use wit_dylib_ffi::Wit;

use crate::interpreter::WitData;
use crate::resources::BorrowedResource;
use crate::resources::ResourceClasses;
use crate::resources::ResourceTable;
use crate::trivia::*;

/// Deterministic, fixed-seed hash map/set used everywhere in the runtime so the
/// wizer snapshot is reproducible.
pub(crate) type DetHasher = core::hash::BuildHasherDefault<DefaultHasher>;
pub(crate) type DetHashMap<K, V> = std::collections::HashMap<K, V, DetHasher>;
pub(crate) type DetHashSet<T> = std::collections::HashSet<T, DetHasher>;
pub(crate) type DetIndexMap<K, V> = indexmap::IndexMap<K, V, DetHasher>;

// Generate bindings for the init interface for wizer
#[allow(clippy::too_many_arguments)]
mod init {
    wit_bindgen::generate!({
        world: "init",
        path: "wit/init.wit",
        generate_all,
        disable_run_ctors_once_workaround: true,
    });

    use super::InitImpl;
    export!(InitImpl);
}

// Global JS runtime and context.
static JS_STATE: GlobalJsState = GlobalJsState(OnceCell::new());

/// Global storage for the single-threaded Wasm runtime.
struct GlobalJsState(OnceCell<JsState>);

// SAFETY: WASM execution is single-threaded for now.
unsafe impl Sync for GlobalJsState {}

/// Global state for the quickjs runtime and context.
struct JsState {
    context: Context,
    /// Ensures the JavaScript source is only evaluated once during initialization.
    evaluated: Cell<bool>,
    /// Whether `local:init/module-loader` (real filesystem-backed module
    /// resolution) is actually linked and safe to call. True throughout
    /// Wizer's own build-time module evaluation (`init_js`, where it's
    /// genuinely available); flipped to false right after that finishes -
    /// which, since Wizer snapshots state at exactly that point, means
    /// every real-runtime instantiation of the built component starts with
    /// this already false. `stub_internal_imports` (crates/core/src/
    /// stubwasi.rs) replaces the import with an unconditionally-trapping
    /// stub for the final shipped component, so a dynamic `import()`
    /// reached at real runtime (a resolver/loader call made after this
    /// flips) must never actually attempt that call - see
    /// `module::host::{HostModuleResolver, HostModuleLoader}`, which check
    /// this first and throw a normal catchable JS error instead.
    module_loader_available: Cell<bool>,
    /// Cached active context pointer for re-entrant `with_ctx` calls.
    ctx_ptr: Cell<Option<*const ()>>,
}

struct ActiveContextGuard<'a> {
    slot: &'a Cell<Option<*const ()>>,
    previous: Option<*const ()>,
}

impl<'a> ActiveContextGuard<'a> {
    fn enter(slot: &'a Cell<Option<*const ()>>, ctx: &rquickjs::Ctx<'_>) -> Self {
        let previous = slot.replace(Some(std::ptr::from_ref(ctx).cast()));
        Self { slot, previous }
    }
}

impl Drop for ActiveContextGuard<'_> {
    fn drop(&mut self) {
        self.slot.set(self.previous);
    }
}

/// Extension trait for `rquickjs::Ctx` providing convenient access to
/// runtime userdata.
pub(crate) trait CtxExt<'js> {
    /// Retrieve the WIT definition stored during initialization.
    fn wit(&self) -> Wit;

    /// Retrieve the async task state.
    fn task(&self) -> UserDataGuard<'_, TaskState>;

    /// Retrieve the exported resource table.
    fn resources(&self) -> UserDataGuard<'_, ResourceTable>;

    /// Retrieve the imported resource class/prototype registry.
    fn resource_classes(&self) -> UserDataGuard<'_, ResourceClasses>;

    /// Retrieve the function/interface name cache.
    fn fns(&self) -> UserDataGuard<'_, FnNameCache>;

    /// Retrieve the evaluated user ES module state.
    fn user_module(&self) -> UserDataGuard<'_, module::UserModule>;

    /// Retrieve transient WIT import module declaration state.
    fn wit_import_declarations(&self) -> UserDataGuard<'_, module::WitImportDeclarations>;

    /// Retrieve precomputed WIT import metadata.
    fn wit_import_registry(&self) -> UserDataGuard<'_, wit_imports::WitImportRegistry>;
}

impl<'js> CtxExt<'js> for rquickjs::Ctx<'js> {
    fn wit(&self) -> Wit {
        self.userdata::<WitData>().expect("WIT not initialized").0
    }

    fn task(&self) -> UserDataGuard<'_, TaskState> {
        self.userdata().expect("TaskState not initialized")
    }

    fn resources(&self) -> UserDataGuard<'_, ResourceTable> {
        self.userdata().expect("ResourceTable not initialized")
    }

    fn resource_classes(&self) -> UserDataGuard<'_, ResourceClasses> {
        self.userdata().expect("ResourceClasses not initialized")
    }

    fn fns(&self) -> UserDataGuard<'_, FnNameCache> {
        self.userdata().expect("FnNameCache not stored")
    }

    fn user_module(&self) -> UserDataGuard<'_, module::UserModule> {
        self.userdata().expect("UserModule not stored")
    }

    fn wit_import_declarations(&self) -> UserDataGuard<'_, module::WitImportDeclarations> {
        self.userdata().expect("WitImportDeclarations not stored")
    }

    fn wit_import_registry(&self) -> UserDataGuard<'_, wit_imports::WitImportRegistry> {
        self.userdata().expect("WitImportRegistry not stored")
    }
}

impl JsState {
    fn get_or_init() -> &'static Self {
        JS_STATE.0.get_or_init(|| {
            let runtime = Runtime::new().expect("Failed to create quikcjs runtime");
            module::install_loader(&runtime);
            let context = Context::full(&runtime).expect("Failed to create quickjs context");

            context.with(|ctx| {
                ctx.store_userdata(FnNameCache::default())
                    .expect("Failed to store function name cache");
                module::init_state(&ctx);
            });

            JsState {
                context,
                evaluated: Default::default(),
                module_loader_available: Cell::new(true),
                ctx_ptr: Default::default(),
            }
        })
    }

    /// Re-uses the active context if already inside `Context::with()` to avoid deadlock.
    ///
    /// This is needed for re-entrant flows such as export → host import callback → JS conversions.
    fn with_ctx<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&rquickjs::Ctx<'_>) -> R,
        R: 'static,
    {
        if let Some(ptr) = self.ctx_ptr.get() {
            // SAFETY: ptr is only set while a `Context::with()` frame is active.
            let ctx = unsafe { &*(ptr as *const rquickjs::Ctx<'_>) };
            return f(ctx);
        }

        self.context.with(|ctx| {
            let _guard = ActiveContextGuard::enter(&self.ctx_ptr, &ctx);
            f(&ctx)
        })
    }
}

// Implements the init interface for wit-bindgen
struct InitImpl;

impl init::Guest for InitImpl {
    fn init(
        shim: String,
        js: String,
        entry_path: Option<String>,
        disable_gc: bool,
    ) -> Result<(), String> {
        init_js(&shim, &js, entry_path.as_deref(), disable_gc)
    }
}

/// Call context for export/import invocations.
#[derive(Default)]
pub struct QjsCallContext {
    /// Value stack for WIT to JS: arguments in, result out
    stack: Vec<Persistent<Value<'static>>>,
    /// Tracks current index per nested list iteration
    iter_stack: SmallVec<[usize; 4]>,
    /// Keeps borrowed `&str` returns alive across FFI boundaries
    temp_strings: SmallVec<[String; 4]>,
    /// Raw allocations to free when this context is dropped
    deferred_deallocs: SmallVec<[(*mut u8, std::alloc::Layout); 4]>,
    /// Imported resource borrows to drop when this context is dropped
    borrows: SmallVec<[BorrowedResource; 4]>,
}

impl QjsCallContext {
    pub(crate) fn push_value<'js>(&mut self, ctx: &rquickjs::Ctx<'js>, val: Value<'js>) {
        self.stack.push(Persistent::save(ctx, val));
    }

    pub(crate) fn pop_value<'js>(&mut self, ctx: &rquickjs::Ctx<'js>) -> Value<'js> {
        self.pop_persistent().restore(ctx).expect("stack underflow")
    }

    /// Takes the FIRST value off the stack, not the last.
    ///
    /// A resource method's receiver is lowered as its first argument, so
    /// `pop_value` reached for it and got the last one instead - fine for a
    /// method taking no arguments, and silently the wrong object for every
    /// method that takes one. Ported from componentize-qjs #76.
    pub(crate) fn shift_value<'js>(&mut self, ctx: &rquickjs::Ctx<'js>) -> Value<'js> {
        self.stack.remove(0).restore(ctx).expect("stack underflow")
    }

    pub(crate) fn pop_persistent(&mut self) -> Persistent<Value<'static>> {
        self.stack.pop().expect("stack underflow")
    }

    pub(crate) fn maybe_pop_persistent(&mut self) -> Option<Persistent<Value<'static>>> {
        self.stack.pop()
    }

    pub(crate) fn maybe_pop_value<'js>(
        &mut self,
        ctx: &rquickjs::Ctx<'js>,
    ) -> rquickjs::Result<Option<Value<'js>>> {
        self.maybe_pop_persistent()
            .map(|persistent| persistent.restore(ctx))
            .transpose()
    }

    pub(crate) fn stack_into_args<'js>(&mut self, ctx: &rquickjs::Ctx<'js>) -> function::Args<'js> {
        let mut args = function::Args::new(ctx.clone(), self.stack.len());
        for p in self.stack.drain(..) {
            p.restore(ctx)
                .and_then(|val| args.push_arg(val))
                .expect("Failed to restore arg");
        }
        args
    }
}

impl Drop for QjsCallContext {
    fn drop(&mut self) {
        for (ptr, layout) in self.deferred_deallocs.drain(..) {
            unsafe {
                std::alloc::dealloc(ptr, layout);
            }
        }
        for borrow in self.borrows.drain(..) {
            unsafe {
                (borrow.drop_fn)(borrow.handle);
            }
        }
    }
}

/// Cache for converting WIT function/interface names to camelCase, stored as
/// rquickjs userdata so it is tied to the JS runtime lifetime.
#[derive(Default, JsLifetime)]
pub(crate) struct FnNameCache(RefCell<DetHashMap<&'static str, &'static str>>);

/// Initialize the quickjs runtime with JavaScript source code.
/// This is called by Wizer during pre-initialization.
fn init_js(
    shim: &str,
    js_source: &str,
    entry_path: Option<&str>,
    disable_gc: bool,
) -> Result<(), String> {
    let state = JsState::get_or_init();

    if state.evaluated.replace(true) {
        return Err("JavaScript already evaluated".to_string());
    }

    if disable_gc {
        state.with_ctx(|ctx| unsafe {
            let rt = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
            rquickjs::qjs::JS_SetGCThreshold(rt, usize::MAX as _);
        });
    }

    state.with_ctx(|ctx| {
        register_build_log(ctx)?;
        module::evaluate_shim(ctx, shim)?;
        module::evaluate_user(ctx, js_source, entry_path)
    })?;

    unsafe {
        abi::reset_adapter_state();
        abi::__wasilibc_reset_preopens();
    }

    // From here on `local:init/module-loader` is no longer safe to call -
    // see `JsState::module_loader_available`'s doc comment. Wizer snapshots
    // state at exactly this point, so every real-runtime instantiation of
    // the built component starts with this already false.
    state.module_loader_available.set(false);

    Ok(())
}

/// Installs `__dwarfBuildLog(target, bytes) -> bool`, the escape hatch the
/// generated `console` uses when it finds no active async task.
///
/// It RETURNS whether it wrote, which is what lets one generated `console`
/// serve both phases without the snapshot carrying a stale answer: during
/// Wizer it prints and returns true, and after the snapshot the same global
/// sees `module_loader_available()` false, writes nothing and returns
/// false, leaving the caller to take the ordinary async path. Deleting the
/// global at the end of init would work too, but this keeps the build/run
/// distinction in the one place that already owns it rather than spreading
/// it across a second mechanism.
fn register_build_log(ctx: &rquickjs::Ctx<'_>) -> Result<(), String> {
    let build_log = rquickjs::Function::new(ctx.clone(), |target: String, bytes: Vec<u8>| {
        if !module_loader_available() {
            return false;
        }
        init::local::init::module_loader::build_log(&target, &bytes);
        true
    })
    .map_err(|e| format!("failed to create __dwarfBuildLog: {e}"))?;

    ctx.globals()
        .set("__dwarfBuildLog", build_log)
        .map_err(|e| format!("failed to install __dwarfBuildLog: {e}"))
}

/// Whether `local:init/module-loader` is still safe to call (see
/// `JsState::module_loader_available`).
pub(crate) fn module_loader_available() -> bool {
    JsState::get_or_init().module_loader_available.get()
}

/// Delegates to `JsState::with_ctx`.
pub(crate) fn with_ctx<F, R>(f: F) -> R
where
    F: FnOnce(&rquickjs::Ctx<'_>) -> R,
    R: 'static,
{
    JsState::get_or_init().with_ctx(f)
}
