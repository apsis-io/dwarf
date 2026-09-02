//! Top-level WIT `result` boundary semantics.
//!
//! Nested WIT `result` values use the normal tagged-object representation.
//! Function returns, however, follow the JCO/ComponentizeJS convention:
//! `ok` is returned/resolved and `err` is thrown/rejected.
//!
//! **The `err` shape is intentionally asymmetric between the two
//! directions** (confirmed a real, deliberate asymmetry, not a bug, when a
//! downstream consumer's JS wrapper had to account for it):
//! - Calling an *import* that errors: `lift`/`component_error_value` always
//!   wraps the raw err payload in a real `Error`, with the raw tagged value
//!   attached as `error.payload` - so a non-string err type reads back as
//!   `error.payload.tag`/`.val`, not `error.tag`/`.val`. JS is the receiver
//!   here and didn't construct the value, so wrapping it gives normal
//!   exception ergonomics (catchable, has `.stack`, `instanceof Error`).
//! - An *export* signaling its own err result (`result_error_payload`): a
//!   plain object matching the WIT shape is used as-is (`throw { tag: "x" }`
//!   needs no `.payload`), while an `Error` with a `.payload` property is
//!   unwrapped on the way out. JS is the author here and already knows the
//!   shape, so this avoids forcing an `Error` wrapper around a value that's
//!   already correctly shaped as the tagged `{ tag, val }` convention used
//!   everywhere else for variants/results.

use rquickjs::function::Args;
use rquickjs::object::Property;
use rquickjs::{
    CatchResultExt, CaughtError, CaughtResult, Constructor, Ctx, Exception, Function, Object,
    Persistent, Result, Value,
};
use wit_dylib_ffi::{Type, WitResult};

use crate::tagged::decode_tagged;
use crate::{reject_promise, resolve_promise};

#[derive(Clone, Copy)]
enum ReturnShape {
    None,
    Plain,
    Result(WitResult),
}

/// JavaScript control-flow produced by lifting a top-level WIT `result`.
pub(crate) enum JsCompletion<'js> {
    /// Return/resolve with this value.
    Return(Value<'js>),
    /// Throw/reject with this value.
    Throw(Value<'js>),
}

impl<'js> JsCompletion<'js> {
    /// Convert into a synchronous JS return or throw.
    pub(crate) fn into_result(self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        match self {
            Self::Return(value) => Ok(value),
            Self::Throw(reason) => Err(ctx.throw(reason)),
        }
    }

    /// Resolve or reject an in-context promise pair.
    pub(crate) fn settle(self, resolve: &Function<'js>, reject: &Function<'js>) -> Result<()> {
        match self {
            Self::Return(value) => {
                resolve.call::<_, Value>((value,))?;
            }
            Self::Throw(reason) => {
                reject.call::<_, Value>((reason,))?;
            }
        }
        Ok(())
    }

    /// Resolve or reject a promise pair saved across an async import boundary.
    pub(crate) fn settle_persistent(
        self,
        ctx: &Ctx<'js>,
        resolve: Persistent<Value<'static>>,
        reject: Persistent<Value<'static>>,
    ) {
        match self {
            JsCompletion::Return(result) => {
                resolve_promise(resolve, Some(Persistent::save(ctx, result)));
            }
            JsCompletion::Throw(reason) => {
                reject_promise(reject, Persistent::save(ctx, reason));
            }
        }
    }
}

/// Adapter between canonical tagged `result` values and JS return/throw.
#[derive(Clone, Copy)]
pub(crate) struct ResultBoundary {
    shape: ReturnShape,
}

impl ResultBoundary {
    /// Create a boundary for a function return type.
    pub(crate) fn new(result: Option<Type>) -> Self {
        let shape = match result.map(resolve_alias) {
            Some(Type::Result(result)) => ReturnShape::Result(result),
            Some(_) => ReturnShape::Plain,
            None => ReturnShape::None,
        };

        Self { shape }
    }

    /// Lift a canonical import return into JS return/throw control flow.
    pub(crate) fn lift<'js>(
        &self,
        ctx: &Ctx<'js>,
        value: Option<Value<'js>>,
    ) -> Result<JsCompletion<'js>> {
        let value = value.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let ReturnShape::Result(result_ty) = self.shape else {
            return Ok(JsCompletion::Return(value));
        };

        let (discriminant, payload) = decode_tagged(
            ctx,
            value,
            "result",
            [
                ("ok", result_ty.ok().is_some()),
                ("err", result_ty.err().is_some()),
            ],
        )?;
        let payload = payload.unwrap_or_else(|| Value::new_undefined(ctx.clone()));

        if discriminant == 1 {
            Ok(JsCompletion::Throw(component_error_value(ctx, payload)?))
        } else {
            Ok(JsCompletion::Return(payload))
        }
    }

    /// Lower a synchronous JS export call into a canonical return value.
    pub(crate) fn lower_call<'js>(
        &self,
        ctx: &Ctx<'js>,
        result: Result<Value<'js>>,
    ) -> CaughtResult<'js, Option<Value<'js>>> {
        self.lower_caught(ctx, result.catch(ctx))
    }

    /// Lower a rejected async JS export into a canonical return value.
    ///
    /// A promise rejection reason arrives as a plain `Value`, not something
    /// obtained via `ctx.catch()` - so unlike `CaughtError::from_error`, there's
    /// no automatic Exception-vs-Value classification. Do it here too: a real
    /// `Error`/subclass gets the richer `Exception` variant (message + stack,
    /// via its own `Display`) instead of the generic `Value` variant's bare
    /// `{:?}` fallback, which matters when this ends up in a diagnostic
    /// message (see `bindings.rs`'s async export dispatch).
    pub(crate) fn lower_throw<'js>(
        &self,
        ctx: &Ctx<'js>,
        reason: Value<'js>,
    ) -> CaughtResult<'js, Option<Value<'js>>> {
        self.lower_caught(ctx, Err(classify_caught(reason)))
    }

    /// Lower a fulfilled async JS export into a canonical return value.
    pub(crate) fn lower_value<'js>(
        &self,
        ctx: &Ctx<'js>,
        value: Value<'js>,
    ) -> CaughtResult<'js, Option<Value<'js>>> {
        match self.shape {
            ReturnShape::None => Ok(None),
            ReturnShape::Plain => Ok(Some(value)),
            ReturnShape::Result(result_ty) => Ok(Some(tagged_ok(ctx, result_ty, value)?)),
        }
    }

    fn lower_caught<'js>(
        &self,
        ctx: &Ctx<'js>,
        result: CaughtResult<'js, Value<'js>>,
    ) -> CaughtResult<'js, Option<Value<'js>>> {
        let ReturnShape::Result(result_ty) = self.shape else {
            return match (self.shape, result) {
                (ReturnShape::None, Ok(_)) => Ok(None),
                (ReturnShape::Plain, Ok(value)) => Ok(Some(value)),
                (ReturnShape::Result(_), _) => unreachable!(),
                (_, Err(err)) => Err(err),
            };
        };

        match result {
            Ok(value) => Ok(Some(tagged_ok(ctx, result_ty, value)?)),
            Err(err) => Ok(Some(tagged_err(ctx, result_ty, err)?)),
        }
    }
}

/// Classify a plain JS value as `CaughtError`'s richer `Exception` variant
/// when it's actually an `Error`/subclass instance, matching what
/// `CaughtError::from_error` would do for a value obtained via `ctx.catch()`.
fn classify_caught<'js>(value: Value<'js>) -> CaughtError<'js> {
    value
        .as_object()
        .and_then(|obj| Exception::from_object(obj.clone()))
        .map(CaughtError::Exception)
        .unwrap_or_else(|| CaughtError::Value(value))
}

fn resolve_alias(mut ty: Type) -> Type {
    while let Type::Alias(alias) = ty {
        ty = alias.ty();
    }

    ty
}

fn tagged_ok<'js>(
    ctx: &Ctx<'js>,
    ty: WitResult,
    payload: Value<'js>,
) -> CaughtResult<'js, Value<'js>> {
    let obj = Object::new(ctx.clone()).map_err(|err| CaughtError::from_error(ctx, err))?;
    obj.set("tag", "ok")
        .map_err(|err| CaughtError::from_error(ctx, err))?;

    if ty.ok().is_some() {
        obj.set("val", payload)
            .map_err(|err| CaughtError::from_error(ctx, err))?;
    }

    Ok(obj.into_value())
}

fn tagged_err<'js>(
    ctx: &Ctx<'js>,
    ty: WitResult,
    err: CaughtError<'js>,
) -> CaughtResult<'js, Value<'js>> {
    let payload = result_error_payload(ctx, ty.err(), err)?;
    let obj = Object::new(ctx.clone()).map_err(|err| CaughtError::from_error(ctx, err))?;

    obj.set("tag", "err")
        .map_err(|err| CaughtError::from_error(ctx, err))?;

    if ty.err().is_some() {
        obj.set("val", payload)
            .map_err(|err| CaughtError::from_error(ctx, err))?;
    } else {
        // The WIT result's err type has no payload (e.g. a bare `result` -
        // wasi:cli/run's own shape), so `payload` is about to be discarded
        // with no way to represent it in the returned value at all. Log it
        // to stderr first (a plain Rust eprintln! - reaches the real host
        // fd 2 under wasm32-wasip1/wasip2 regardless of whether the world
        // imports any WASI stderr interface, the same way an unhandled Rust
        // panic's message already does) so a real failure doesn't vanish
        // with zero diagnostic output anywhere - confirmed to happen in
        // practice: a rejected wasi:cli/run (bare `result`) exits cleanly
        // and silently, easy to mistake for a hang if something restarts
        // the process quickly afterward.
        log_discarded_err_payload(ctx, &payload);
    }

    Ok(obj.into_value())
}

/// Best-effort stderr log for an error payload that's about to be discarded
/// because the WIT result's err type has no payload to carry it. Skips
/// `undefined`/`null` - `throw undefined`/`throw null` against a payload-less
/// `result<T>` is how guest code signals "fail, deliberately with no
/// information" (see e.g. `test_async_result_no_error_payload`), not a real
/// error being lost, so logging there would just be noise. Otherwise uses
/// the payload directly if it's already a string; prefers `JSON.stringify`
/// for anything else (the payload here is typically a structured WIT
/// variant/record like a `header-error`, and `String()`'s default object
/// coercion just produces the useless "[object Object]"), falling back to
/// `String()` only if that fails (e.g. a value `JSON.stringify` can't
/// serialize). Never fails the export over a logging problem, so any error
/// here is itself just swallowed.
fn log_discarded_err_payload<'js>(ctx: &Ctx<'js>, payload: &Value<'js>) {
    if payload.is_undefined() || payload.is_null() {
        return;
    }
    let text = if let Some(s) = payload.as_string().and_then(|s| s.to_string().ok()) {
        s
    } else if let Ok(json) = ctx.globals().get::<_, Object>("JSON")
        && let Ok(stringify) = json.get::<_, Function>("stringify")
        && let Ok(s) = stringify.call::<_, String>((payload.clone(),))
    {
        s
    } else if let Ok(string_fn) = ctx.globals().get::<_, Function>("String")
        && let Ok(s) = string_fn.call::<_, String>((payload.clone(),))
    {
        s
    } else {
        return;
    };
    eprintln!(
        "dwarf: an async/sync export rejected with an error, but its declared WIT result type \
         has no err payload to carry it - the error is being discarded with no other way to \
         surface it. Original error: {text}"
    );
}

fn result_error_payload<'js>(
    ctx: &Ctx<'js>,
    err_ty: Option<Type>,
    err: CaughtError<'js>,
) -> CaughtResult<'js, Value<'js>> {
    let reason = match err {
        CaughtError::Exception(exception) => exception.into_value(),
        CaughtError::Value(reason) => reason,
        CaughtError::Error(err) => return Err(CaughtError::Error(err)),
    };

    let Some(obj) = reason.as_object() else {
        return Ok(reason);
    };

    if has_own_property(ctx, obj, "payload").map_err(|err| CaughtError::from_error(ctx, err))? {
        return obj
            .get("payload")
            .map_err(|err| CaughtError::from_error(ctx, err));
    }

    if reason.is_error()
        && matches!(err_ty.map(resolve_alias), Some(Type::String))
        && let Ok(message) = obj.get::<_, rquickjs::String>("message")
    {
        return Ok(message.into_value());
    }

    if reason.is_error() {
        return Err(classify_caught(reason));
    }

    Ok(reason)
}

fn has_own_property<'js>(ctx: &Ctx<'js>, obj: &Object<'js>, key: &str) -> Result<bool> {
    let ctor: Object = ctx.globals().get("Object")?;
    let proto: Object = ctor.get("prototype")?;
    let has_own_property: Function = proto.get("hasOwnProperty")?;

    let mut args = Args::new(ctx.clone(), 1);
    args.this(obj.clone())?;
    args.push_arg(key)?;

    has_own_property.call_arg(args)
}

fn component_error_value<'js>(ctx: &Ctx<'js>, payload: Value<'js>) -> Result<Value<'js>> {
    let is_string = payload.is_string();
    let message = if is_string {
        payload.get::<String>()?
    } else {
        let string_fn: Function = ctx.globals().get("String")?;
        let text: String = string_fn.call((payload.clone(),))?;
        format!("{text} (see error.payload)")
    };

    let ctor: Constructor = ctx.globals().get("Error")?;
    let error: Object = ctor.construct((message,))?;

    let payload_prop = if is_string {
        Property::from(payload)
    } else {
        Property::from(payload).enumerable()
    };

    error.prop("payload", payload_prop)?;
    Ok(error.into_value())
}
