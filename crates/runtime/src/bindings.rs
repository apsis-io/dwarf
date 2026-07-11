//! WIT to/from JS binding registration.
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use rquickjs::Persistent;
use rquickjs::function;
use rquickjs::function::{Constructor, Rest, This};
use rquickjs::{CaughtError, Ctx, Function, Object, Value};
use smallvec::SmallVec;
use wit_dylib_ffi::{ExportFunction, Resource, Wit};

use crate::CtxExt;
use crate::futures::{make_future, register_future_classes};
use crate::result::ResultBoundary;
use crate::streams::{make_stream, register_stream_classes};
use crate::task::Pending;
use crate::trivia::iface_lookup;
use crate::wit_imports::{FuncKind, WitInterface, classify, find_resource, root_bindings};
use crate::{DetHashSet, DetIndexMap, QjsCallContext, coerce_fn};

/// Register all wit bindings on the js global scope.
pub(crate) fn register(ctx: &rquickjs::Ctx<'_>, wit_def: Wit) -> rquickjs::Result<()> {
    register_stream_classes(ctx)?;
    register_future_classes(ctx)?;
    register_resource_classes(ctx, wit_def)?;
    register_root_imports(ctx, wit_def)?;
    register_cqjs_namespace(ctx, wit_def)?;
    Ok(())
}

/// Build a JS "class" (constructor + prototype) for every imported resource.
fn register_resource_classes<'js>(ctx: &Ctx<'js>, wit: Wit) -> rquickjs::Result<()> {
    struct Group {
        resource: Resource,
        ctor: Option<usize>,
        methods: Vec<(&'static str, usize)>,
        statics: Vec<(&'static str, usize)>,
    }

    let mut groups: DetIndexMap<usize, Group> = DetIndexMap::default();

    for func in wit.iter_import_funcs() {
        let kind = classify(func.name());
        let resource_name = match kind {
            FuncKind::Freestanding => continue,
            FuncKind::Constructor { resource }
            | FuncKind::Method { resource, .. }
            | FuncKind::Static { resource, .. } => resource,
        };

        let Some(resource) = find_resource(wit, func.interface(), resource_name) else {
            continue;
        };

        // Only imported resources get host-backed classes; exported (JS-backed)
        // resources have a `rep` and are handled on the export side.
        if resource.rep().is_some() {
            continue;
        }

        let group = groups.entry(resource.index()).or_insert_with(|| Group {
            resource,
            ctor: None,
            methods: Vec::new(),
            statics: Vec::new(),
        });

        match kind {
            FuncKind::Constructor { .. } => group.ctor = Some(func.index()),
            FuncKind::Method { method, .. } => group.methods.push((method, func.index())),
            FuncKind::Static { method, .. } => group.statics.push((method, func.index())),
            FuncKind::Freestanding => unreachable!(),
        }
    }

    let mut built: Vec<(
        usize,
        Persistent<Value<'static>>,
        Persistent<Value<'static>>,
    )> = Vec::new();

    for (index, group) in groups {
        let prototype = Object::new(ctx.clone())?;
        for (method, func_index) in group.methods {
            let js_func = Function::new(
                ctx.clone(),
                move |this: This<Value<'js>>, ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                    let mut call_args: SmallVec<[Value<'js>; 8]> =
                        SmallVec::with_capacity(args.0.len() + 1);
                    call_args.push(this.0);
                    call_args.extend(args.0);
                    call_import(ctx, func_index, call_args)
                },
            )?;
            prototype.set(method.to_lower_camel_case(), js_func)?;
        }

        // `push_own` (call.rs) hands the guest an *owned* handle it must be
        // able to release early (e.g. a WASI 0.2 `output-stream` returned
        // from `outgoing-body.write()`) rather than only on GC — dwarf's own
        // Stream/Future wrapper types already expose `drop`/`[Symbol.dispose]`
        // (streams.rs) for the same reason; generic imported resources need
        // the same escape hatch or the host-side resource table entry is
        // held until the whole store is torn down. `__cqjs_owned` (set only
        // by `push_own`'s host-resource branch) distinguishes an owned
        // instance from a `push_borrow`'d one sharing this same prototype —
        // dropping a borrow here would double-free it once `QjsCallContext`
        // auto-drops it at the end of the call that lent it out, so borrows
        // silently no-op instead.
        let drop_fn = group.resource.drop();
        let js_drop = Function::new(
            ctx.clone(),
            move |this: This<Value<'js>>| -> rquickjs::Result<()> {
                let Some(obj) = this.0.as_object() else {
                    return Ok(());
                };
                let owned: bool = obj.get("__cqjs_owned").unwrap_or(false);
                if !owned {
                    return Ok(());
                }
                if let Ok(handle) = obj.get::<_, u32>("__cqjs_handle") {
                    let _ = obj.remove("__cqjs_handle");
                    let _ = obj.remove("__cqjs_owned");
                    unsafe { drop_fn(handle) };
                }
                Ok(())
            },
        )?;
        prototype.set("drop", js_drop.clone())?;
        prototype.set(crate::trivia::symbol_dispose(ctx)?, js_drop)?;

        let class: Constructor = match group.ctor {
            Some(func_index) => Constructor::new_prototype(
                ctx,
                prototype.clone(),
                move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                    call_import(ctx, func_index, SmallVec::from_vec(args.0))
                },
            )?,
            None => {
                let resource_name = group.resource.name();
                Constructor::new_prototype(
                    ctx,
                    prototype.clone(),
                    move |ctx: Ctx<'js>, _args: Rest<Value<'js>>| -> rquickjs::Result<Value<'js>> {
                        Err(rquickjs::Exception::throw_type(
                            &ctx,
                            &format!("{resource_name} has no constructor"),
                        ))
                    },
                )?
            }
        };

        for (method, func_index) in group.statics {
            let js_func =
                Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
                    call_import(ctx, func_index, SmallVec::from_vec(args.0))
                })?;
            class.set(method.to_lower_camel_case(), js_func)?;
        }

        built.push((
            index,
            Persistent::save(ctx, class.into_value()),
            Persistent::save(ctx, prototype.into_value()),
        ));
    }

    let registry = ctx.resource_classes();
    for (index, class, prototype) in built {
        registry.insert(index, class, prototype);
    }

    Ok(())
}

/// Create a js object containing all functions, flags, enums, and variants
/// for a single wit interface.
pub(crate) fn interface_to_js<'js>(
    ctx: &rquickjs::Ctx<'js>,
    iface: &WitInterface,
) -> rquickjs::Result<rquickjs::Object<'js>> {
    let obj = rquickjs::Object::new(ctx.clone())?;

    let mut seen_resources: DetHashSet<usize> = DetHashSet::default();
    for func in &iface.funcs {
        match classify(func.name()) {
            FuncKind::Freestanding => {
                let func_name = func.name().to_lower_camel_case();
                let func_index = func.index();
                let js_func = rquickjs::Function::new(
                    ctx.clone(),
                    move |ctx: rquickjs::Ctx<'js>, args: Rest<Value<'js>>| {
                        call_import(ctx, func_index, SmallVec::from_vec(args.0))
                    },
                )?;
                obj.set(func_name, js_func)?;
            }
            FuncKind::Constructor { resource }
            | FuncKind::Method { resource, .. }
            | FuncKind::Static { resource, .. } => {
                let Some(res) = find_resource(ctx.wit(), func.interface(), resource) else {
                    continue;
                };
                if !seen_resources.insert(res.index()) {
                    continue;
                }
                if let Some(class) = ctx.resource_classes().class(res.index()) {
                    obj.set(resource.to_upper_camel_case(), class.restore(ctx)?)?;
                }
            }
        }
    }

    Ok(obj)
}

fn register_root_imports(ctx: &rquickjs::Ctx<'_>, wit_def: Wit) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let obj = interface_to_js(ctx, &root_bindings(wit_def))?;

    for key in obj.keys::<String>() {
        let key = key?;
        let val: Value = obj.get(&key)?;
        globals.set(key, val)?;
    }

    Ok(())
}

fn call_import<'js>(
    ctx: rquickjs::Ctx<'js>,
    func_index: usize,
    args: SmallVec<[Value<'js>; 8]>,
) -> rquickjs::Result<Value<'js>> {
    let wit_def = ctx.wit();
    let func = wit_def.import_func(func_index);

    let boundary = ResultBoundary::new(func.result());
    let mut call = QjsCallContext::default();
    for arg in args.into_iter().rev() {
        call.push_value(&ctx, arg);
    }

    if func.is_async() {
        let (promise, resolve, reject) = ctx.promise()?;

        if let Some(pending) = unsafe { func.call_import_async(&mut call) } {
            let handle = pending.subtask;
            let buffer = pending.buffer;

            let resolve = Persistent::save(&ctx, resolve.into_value());
            let reject = Persistent::save(&ctx, reject.into_value());
            let pending = Pending::ImportCall {
                func_index,
                call,
                buffer,
                resolve,
                reject,
            };
            ctx.task().register(handle, pending);
        } else {
            boundary
                .lift(&ctx, call.maybe_pop_value(&ctx)?)?
                .settle(&resolve, &reject)
                .expect("Failed to settle async import");
        }

        Ok(promise.into_value())
    } else {
        func.call_import_sync(&mut call);
        boundary
            .lift(&ctx, call.maybe_pop_value(&ctx)?)?
            .into_result(&ctx)
    }
}

/// Describes why an async export's settled promise couldn't be lowered into
/// its declared WIT result type - the one case `ResultBoundary::lower_value`/
/// `lower_throw` can genuinely never handle: a non-`string` `err` type (e.g. a
/// `variant`/`enum`/`record`, common for things like `wasi:http`'s
/// `error-code`) has no generic mapping from an arbitrary thrown/returned JS
/// value, so there's no way to synthesize a matching payload. Names the
/// export (interface + function) so this reads as "your export did X" rather
/// than an unattributed internal crash, and uses `e`'s `Display` (richer for
/// a real thrown `Error`, including its stack, once `lower_throw`/
/// `result_error_payload` classify it as `CaughtError::Exception` rather than
/// the generic `Value` fallback - see `result.rs`).
fn describe_lower_failure(func: &ExportFunction, phase: &str, e: CaughtError<'_>) -> String {
    let qualified = match func.interface() {
        Some(iface) => format!("{iface}.{}", func.name()),
        None => func.name().to_string(),
    };
    format!(
        "async export '{qualified}' {phase} a value that can't be represented as its \
         declared WIT result type - its `err` type isn't `string` and the value has no \
         `.payload` property matching the expected shape. Either await/catch this in your \
         JS and throw a plain string or an Error with a `.payload` property shaped like the \
         WIT error, or avoid throwing an unrelated error from this export. Original error: {e}"
    )
}

/// EXPERIMENTAL: before signaling `task_return`, drains any outstanding
/// fire-and-forget writes registered via the console polyfill's
/// `__dwarfTrackWrite` (see `crates/core/src/polyfills.rs`) - so a library
/// that calls `console.log(...)` without awaiting it (as virtually all
/// real-world JS libraries do, since real `console.log` is synchronous)
/// still gets a flushed write by the time the whole export call completes,
/// rather than having it silently cancelled along with the task the moment
/// the export's own result is ready. Falls back to calling `task_return`
/// immediately if `__dwarfDrainPendingWrites` isn't defined (no console
/// polyfill emitted for this world at all).
fn finish_export_after_drain<'js>(
    ctx: &Ctx<'js>,
    func_index: usize,
    call: QjsCallContext,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    let Ok(drain_fn) = globals.get::<_, Function>("__dwarfDrainPendingWrites") else {
        let func = ctx.wit().export_func(func_index);
        let mut call = call;
        func.call_task_return(&mut call);
        return Ok(());
    };

    let drain_promise: Value = drain_fn.call(())?;
    let promise_obj = drain_promise
        .as_object()
        .ok_or_else(|| rquickjs::Error::new_from_js("value", "promise"))?;
    let then_fn: Function = promise_obj.get("then")?;

    let call_cell = std::cell::Cell::new(Some(call));
    let finish_cb = Function::new(
        ctx.clone(),
        coerce_fn(move |ctx: Ctx<'_>, _args: Rest<Value<'_>>| {
            let mut call = call_cell
                .take()
                .expect("drain completion callback invoked more than once");
            let func = ctx.wit().export_func(func_index);
            func.call_task_return(&mut call);
            Ok(Value::new_undefined(ctx))
        }),
    )?;

    // Registered as BOTH the resolve and reject handler: the drain is a
    // best-effort flush of already-fire-and-forgotten writes, and the
    // export's own result was already correctly lowered into `call` before
    // this ran - so even if draining itself somehow rejects (it shouldn't in
    // practice, since __dwarfDrainPendingWrites is built on
    // Promise.allSettled, which never rejects - but a future change to that
    // polyfill, or a user-overridden version of it, could), `task_return`
    // must still be called or the whole task hangs forever with no panic,
    // no trap, nothing - a real, independently-confirmed gap (only an
    // onFulfilled handler was ever registered here, unlike build_async_exports's
    // own then_cb/catch_cb pair on the export's result itself).
    let mut call_args = function::Args::new(ctx.clone(), 2);
    call_args.this(drain_promise.clone())?;
    call_args.push_arg(finish_cb.clone())?;
    call_args.push_arg(finish_cb)?;
    then_fn.call_arg::<Value>(call_args)?;
    Ok(())
}

/// Looks up `fn_name` on `obj` and confirms it's actually a callable
/// function, throwing a clear JS `Error` naming exactly which WIT export is
/// missing if not - rather than the FFI's own generic property-lookup
/// failure (`.get::<Function>()` on a missing/non-function property
/// surfaces as an opaque "Exception generated by QuickJS" once it crosses
/// back into a panicking `unwrap_or_else`, giving no hint at all about what
/// actually went wrong). The most common way this happens: a `wasi:cli/
/// command@0.3.0` world's implicit `run` export requires `export function
/// run() { ... }` in the entry JS file, easy to forget since nothing about
/// componentizing successfully signals that it's missing until `run` is
/// actually invoked.
fn require_export<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    fn_name: &str,
    wit_name: &str,
) -> rquickjs::Result<Function<'js>> {
    let value: Value = obj.get(fn_name)?;
    if value.is_function() {
        return obj.get(fn_name);
    }
    let message = format!(
        "your JS module doesn't export a function for WIT export '{wit_name}' (looked for `{fn_name}`) - add `export function {fn_name}(...) {{ ... }}` to your JS entry file"
    );
    let ctor: Constructor = ctx.globals().get("Error")?;
    let error: Value = ctor.construct((message,))?;
    Err(ctx.throw(error))
}

/// Same as `require_export`, but for a WIT `interface` export, which JS
/// represents as a plain object of functions rather than a function itself.
fn require_export_object<'js>(
    ctx: &Ctx<'js>,
    obj: &Object<'js>,
    prop_name: &str,
    wit_name: &str,
) -> rquickjs::Result<Object<'js>> {
    let value: Value = obj.get(prop_name)?;
    if let Some(o) = value.as_object() {
        return Ok(o.clone());
    }
    let message = format!(
        "your JS module doesn't export an object for WIT interface export '{wit_name}' (looked for `{prop_name}`) - add `export const {prop_name} = {{ ... }}` (with each of the interface's functions as properties) to your JS entry file"
    );
    let ctor: Constructor = ctx.globals().get("Error")?;
    let error: Value = ctor.construct((message,))?;
    Err(ctx.throw(error))
}

/// Build the `asyncExports` object for the `__cqjs` namespace.
///
/// Each wrapper calls the user's export function, then chains `.then()` to
/// signal `task_return` back to the host.
fn build_async_exports<'js>(
    ctx: &rquickjs::Ctx<'js>,
    wit_def: Wit,
) -> rquickjs::Result<rquickjs::Object<'js>> {
    let exports = rquickjs::Object::new(ctx.clone())?;
    // Insertion-ordered so the resulting object's property order is deterministic
    // (and follows WIT declaration order) for a reproducible Wizer snapshot.
    let mut iface_objs: DetIndexMap<String, rquickjs::Object<'_>> = DetIndexMap::default();

    for (func_index, func) in wit_def.iter_export_funcs().enumerate() {
        let func_name = func.name().to_lower_camel_case();
        let iface_name = func
            .interface()
            .map(|interface| iface_lookup(ctx, interface).to_string());

        let fn_name = func_name.clone();
        let iface = iface_name.clone();

        let wrapper = Function::new(
            ctx.clone(),
            coerce_fn(move |ctx: Ctx<'_>, args: Rest<Value<'_>>| {
                let exports = ctx.user_module().exports(&ctx)?;

                let user_fn: Function = if let Some(ref iface) = iface {
                    let iface_obj =
                        require_export_object(&ctx, &exports, iface.as_str(), iface.as_str())?;
                    require_export(
                        &ctx,
                        &iface_obj,
                        fn_name.as_str(),
                        &format!("{iface}.{fn_name}"),
                    )?
                } else {
                    require_export(&ctx, &exports, fn_name.as_str(), fn_name.as_str())?
                };

                let mut js_args = function::Args::new(ctx.clone(), args.0.len());
                for arg in args.0 {
                    js_args.push_arg(arg)?;
                }
                let result = user_fn.call_arg::<Value>(js_args)?;

                let promise_obj = result
                    .as_object()
                    .ok_or_else(|| rquickjs::Error::new_from_js("value", "promise"))?;

                let then_fn: Function = promise_obj.get("then")?;

                let then_cb = Function::new(
                    ctx.clone(),
                    coerce_fn(move |ctx: Ctx<'_>, args: Rest<Value<'_>>| {
                        let value = args
                            .0
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));

                        let func = ctx.wit().export_func(func_index);
                        let boundary = ResultBoundary::new(func.result());
                        let mut call = QjsCallContext::default();

                        let value = boundary.lower_value(&ctx, value).unwrap_or_else(|e| {
                            panic!("{}", describe_lower_failure(&func, "resolved with", e))
                        });

                        if let Some(value) = value {
                            call.push_value(&ctx, value);
                        }
                        finish_export_after_drain(&ctx, func_index, call)?;
                        Ok(Value::new_undefined(ctx))
                    }),
                )?;

                let catch_cb = Function::new(
                    ctx.clone(),
                    coerce_fn(move |ctx: Ctx<'_>, args: Rest<Value<'_>>| {
                        let reason = args
                            .0
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| Value::new_undefined(ctx.clone()));
                        let func = ctx.wit().export_func(func_index);
                        let boundary = ResultBoundary::new(func.result());
                        let mut call = QjsCallContext::default();
                        let value = boundary.lower_throw(&ctx, reason).unwrap_or_else(|e| {
                            panic!("{}", describe_lower_failure(&func, "rejected with", e))
                        });

                        if let Some(value) = value {
                            call.push_value(&ctx, value);
                        }

                        finish_export_after_drain(&ctx, func_index, call)?;
                        Ok(Value::new_undefined(ctx))
                    }),
                )?;

                let mut call_args = function::Args::new(ctx.clone(), 2);
                call_args.this(result)?;
                call_args.push_arg(then_cb)?;
                call_args.push_arg(catch_cb)?;
                then_fn.call_arg(call_args)
            }),
        )?;

        let target = match &iface_name {
            Some(iface) => iface_objs
                .entry(iface.clone())
                .or_insert_with(|| rquickjs::Object::new(ctx.clone()).unwrap()),
            None => &exports,
        };
        target.set(func_name.as_str(), wrapper)?;
    }

    for (name, obj) in iface_objs {
        exports.set(name.as_str(), obj)?;
    }

    Ok(exports)
}

/// Register the `__cqjs` namespace object on globalThis.
///
/// Consolidates all internal bridge globals into a single frozen object:
/// - `makeStream(typeIndex)` — create a stream pair
/// - `makeFuture(typeIndex)` — create a future pair
/// - `hasActiveTask()` — whether component-model stream/future operations
///   are currently safe to use (see `TaskState::is_active`)
/// - `getMemoryUsage()` — return QuickJS memory statistics
/// - `runGc()` — trigger QuickJS garbage collection
/// - `asyncExports` — object containing async export wrappers
fn register_cqjs_namespace(ctx: &rquickjs::Ctx<'_>, wit_def: Wit) -> rquickjs::Result<()> {
    let ns = rquickjs::Object::new(ctx.clone())?;

    // Stream/future factories
    ns.set(
        "makeStream",
        Function::new(
            ctx.clone(),
            coerce_fn(move |ctx: Ctx<'_>, args: Rest<Value<'_>>| make_stream(ctx, args)),
        )?,
    )?;

    ns.set(
        "makeFuture",
        Function::new(
            ctx.clone(),
            coerce_fn(move |ctx: Ctx<'_>, args: Rest<Value<'_>>| make_future(ctx, args)),
        )?,
    )?;

    // Memory introspection
    ns.set(
        "getMemoryUsage",
        Function::new(
            ctx.clone(),
            coerce_fn(
                move |ctx: Ctx<'_>, _args: Rest<Value<'_>>| -> rquickjs::Result<Value<'_>> {
                    let usage = unsafe {
                        let rt = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                        let mut usage = std::mem::MaybeUninit::uninit();
                        rquickjs::qjs::JS_ComputeMemoryUsage(rt, usage.as_mut_ptr());
                        usage.assume_init()
                    };
                    let obj = rquickjs::Object::new(ctx.clone())?;
                    obj.set("mallocSize", usage.malloc_size)?;
                    obj.set("mallocCount", usage.malloc_count)?;
                    obj.set("memoryUsedSize", usage.memory_used_size)?;
                    obj.set("objCount", usage.obj_count)?;
                    obj.set("strCount", usage.str_count)?;
                    obj.set("atomCount", usage.atom_count)?;
                    obj.set("atomSize", usage.atom_size)?;
                    obj.set("propCount", usage.prop_count)?;
                    obj.set("shapeCount", usage.shape_count)?;
                    obj.set("arrayCount", usage.array_count)?;
                    Ok(obj.into_value())
                },
            ),
        )?,
    )?;

    ns.set(
        "hasActiveTask",
        Function::new(
            ctx.clone(),
            coerce_fn(
                move |ctx: Ctx<'_>, _args: Rest<Value<'_>>| -> rquickjs::Result<Value<'_>> {
                    let active = ctx.task().is_active();
                    Ok(Value::new_bool(ctx, active))
                },
            ),
        )?,
    )?;

    ns.set(
        "runGc",
        Function::new(
            ctx.clone(),
            coerce_fn(
                move |ctx: Ctx<'_>, _args: Rest<Value<'_>>| -> rquickjs::Result<Value<'_>> {
                    unsafe {
                        let rt = rquickjs::qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
                        rquickjs::qjs::JS_RunGC(rt);
                    }
                    Ok(Value::new_undefined(ctx))
                },
            ),
        )?,
    )?;

    // Async export wrappers
    let async_exports = build_async_exports(ctx, wit_def)?;
    ns.set("asyncExports", async_exports)?;

    // Freeze and install on globalThis
    let object_ctor: rquickjs::Object = ctx.globals().get("Object")?;
    let freeze_fn: Function = object_ctor.get("freeze")?;
    freeze_fn.call::<_, Value>((ns.clone(),))?;

    ctx.globals().set("__cqjs", ns)?;
    Ok(())
}
