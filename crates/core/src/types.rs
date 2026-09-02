//! Generates TypeScript type declarations for a WIT world via `jco types`
//! (<https://github.com/bytecodealliance/jco>), so editors/tsc can type-check
//! JS/TS source against the WIT-backed imports dwarf's runtime resolves.

use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, Result, anyhow};
use regex::Regex;

use crate::polyfills;

const INSTALL_HINT: &str = "install it with `npm install -g @bytecodealliance/jco` and ensure \
     it's on PATH, see https://github.com/bytecodealliance/jco. Note: the unscoped `jco` \
     package on npm is an unrelated placeholder - the real package is `@bytecodealliance/jco`";

/// Runs `jco types <wit_path> -o <out_dir>` using a `jco` binary already on
/// `PATH`, then patches the generated `.d.ts` files to match dwarf's actual
/// JS runtime conventions rather than jco's own (componentize-js-oriented)
/// ones - see `patch_dwarf_conventions`. Does not install or invoke `jco` via
/// `npx` - only a `jco` the user explicitly installed themselves is used.
///
/// Also writes `globals.d.ts` covering every always-on global
/// (`console`/`process`/`TextEncoder`/`TextDecoder`) plus each requested
/// static `polyfill` - so `--emit-types` gives full type coverage together
/// with `--polyfill`, rather than only ever covering WIT interfaces and
/// silently saying nothing about polyfill globals.
pub fn emit_ts_types(
    wit_path: &Path,
    world_name: Option<&str>,
    out_dir: &Path,
    polyfills: &[impl AsRef<str>],
) -> Result<()> {
    std::fs::create_dir_all(out_dir).with_context(|| {
        format!(
            "failed to create types output directory {}",
            out_dir.display()
        )
    })?;

    let mut cmd = Command::new("jco");
    cmd.arg("types").arg(wit_path).arg("-o").arg(out_dir);
    if let Some(world) = world_name {
        cmd.arg("-n").arg(world);
    }

    let output = cmd.output();
    let output = match output {
        Ok(output) => output,
        Err(io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!("`jco` is not installed; {INSTALL_HINT}"));
        }
        Err(io_err) => return Err(anyhow!("failed to run `jco types`: {io_err}")),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "`jco types` exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    patch_dwarf_conventions(out_dir)
        .context("failed to patch jco-generated types to match dwarf's JS runtime conventions")?;

    let globals_dts = polyfills::dts_for(polyfills)?;
    std::fs::write(out_dir.join("globals.d.ts"), globals_dts)
        .context("failed to write globals.d.ts")?;

    Ok(())
}

/// jco's generated `.d.ts` assumes componentize-js's own JS bindings, which
/// diverge from dwarf's actual runtime in several ways (confirmed
/// empirically, not just from reading jco's source):
///
/// - `u64`/`s64` are typed `bigint`, but dwarf's runtime always hands back a
///   plain JS `number`.
/// - `option<T>` is typed `T | undefined` (function params/returns) or an
///   omittable `field?: T` (record fields), but dwarf's runtime always
///   includes the property/value and uses `null` for "none", never
///   `undefined` and never omits the property.
/// - A `stream<T>` value is typed `AsyncIterable<T>` (older jco:
///   `ReadableStream<T>`), but dwarf's runtime hands back its own
///   `StreamReadable<T>` wrapper (`read(count?)`, `cancelRead()`, `drop()`).
///   Since componentize-qjs #69 that wrapper IS also async-iterable, so the
///   emitted interface extends `AsyncIterable` rather than replacing it -
///   `for await` and `read(n)` are both real, and a declaration naming only
///   one of them is only half the object. In parameter position a plain
///   async iterable is accepted as well, since dwarf lowers a generator
///   straight to a stream.
/// - A `future<T>` value is typed `PromiseLike<T>`, but dwarf's runtime hands
///   back its own `FutureReadable<T>` wrapper (`read()`, `cancelRead()`,
///   `drop()`) - not a thenable. `await`ing one directly typechecks and never
///   waits on the future at all.
///
/// Both spellings are handled in EVERY position, including a function's own
/// top-level return. That used to be excluded on the grounds that a
/// `Promise<T>` return is ambiguous between a genuinely `async` WIT function
/// (a real native Promise) and a `future<T>` returned from a non-async one.
/// Current jco distinguishes them: `PromiseLike<T>` for the future value,
/// `Promise<T>` for the async function. So `PromiseLike` is rewritten
/// unconditionally, and a bare `Promise` return is still left alone, which is
/// correct rather than merely cautious.
///
/// Two things made this worth chasing rather than documenting. The patches
/// targeted jco's OLD spellings, so against current jco they silently matched
/// nothing and the output was pure canonical-ABI - and a generated
/// declaration is trusted exactly because it was generated, so the failure
/// arrives as code that typechecks and does not run. Reported by greenfield
/// after hitting it on `wasi:sockets`.
///
/// Patches every `.d.ts` under `dir` in place to match. This is a best-effort
/// textual fix, not a from-scratch type generator - deeply nested option
/// shapes (e.g. `option<option<T>>`, which dwarf represents with a tagged
/// `{ tag, val }` form rather than plain `null`) and generic types nested
/// more than two levels deep (e.g. `stream<list<result<T, E>>>`) are not
/// specifically handled and may still read as jco's own convention.
fn patch_dwarf_conventions(dir: &Path) -> Result<()> {
    for entry in walk_dts(dir)? {
        let content = std::fs::read_to_string(&entry)
            .with_context(|| format!("failed to read {}", entry.display()))?;
        let patched = patch_ts_source(&content);
        if patched != content {
            std::fs::write(&entry, &patched)
                .with_context(|| format!("failed to write {}", entry.display()))?;
        }
    }
    Ok(())
}

fn walk_dts(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "ts") {
                out.push(path);
            }
        }
    }
    Ok(out)
}

static BIGINT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bbigint\b").unwrap());
static OPTIONAL_FIELD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\w+)\?:\s*([^,;\n]+)").unwrap());

/// Matches a generic type application up to two levels of `<...>` nesting -
/// covers the common case (e.g. `Result<number, string>`) without needing a
/// recursive grammar (which the `regex` crate can't express).
const BALANCED_GENERIC_BODY: &str = r"(?:[^<>]|<[^<>]*>)*";

static READABLE_STREAM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"ReadableStream<({BALANCED_GENERIC_BODY})>")).unwrap());
static PROMISE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"Promise<({BALANCED_GENERIC_BODY})>")).unwrap());
/// Current jco's spelling for `stream<T>`, in every position.
static ASYNC_ITERABLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"AsyncIterable<({BALANCED_GENERIC_BODY})>")).unwrap());
/// Current jco's spelling for a `future<T>` VALUE, as distinct from the
/// `Promise<T>` it emits for a genuinely `async func`.
/// Matches an already-rewritten stream, for widening it in parameters.
static STREAM_READABLE_PARAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"StreamReadable<({BALANCED_GENERIC_BODY})>")).unwrap()
});
static PROMISE_LIKE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"PromiseLike<({BALANCED_GENERIC_BODY})>")).unwrap());
static TUPLE_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\[\]]*)\]").unwrap());
static PARAM_LIST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^()]*)\)").unwrap());

/// Extends `AsyncIterable`, because a lifted stream genuinely is one:
/// `for await (const chunk of stream)` works alongside `read(n)`. That is
/// what jco's `AsyncIterable<T>` was reaching for and, before dwarf took
/// componentize-qjs #69, got wrong - the object had no iterator at all.
/// `stream<u8>` yields bounded `Uint8Array` chunks rather than one promise
/// per byte, which is why the element type is conditional.
const STREAM_READABLE_IFACE: &str = "interface StreamReadable<T> extends AsyncIterable<T extends number ? Uint8Array : T> {\n  read(count?: number): Promise<T extends number ? Uint8Array : T[]>;\n  cancelRead(): void | { progress: number; result: number };\n  drop(): void;\n}\n";
const FUTURE_READABLE_IFACE: &str = "interface FutureReadable<T> {\n  read(): Promise<T>;\n  cancelRead(): void | number;\n  drop(): void;\n}\n";

fn wrap_futures_in(text: &str) -> String {
    PROMISE_RE
        .replace_all(text, "FutureReadable<$1>")
        .into_owned()
}

/// Rewrites `future<T>` values in parameter and tuple-return-element
/// position (both unambiguous - see `patch_dwarf_conventions`'s doc comment)
/// from jco's bare `Promise<T>` to dwarf's actual `FutureReadable<T>`
/// wrapper shape.
fn patch_future_positions(src: &str) -> String {
    let src = TUPLE_TYPE_RE.replace_all(src, |caps: &regex::Captures| {
        format!("[{}]", wrap_futures_in(&caps[1]))
    });
    PARAM_LIST_RE
        .replace_all(&src, |caps: &regex::Captures| {
            format!("({})", wrap_futures_in(&caps[1]))
        })
        .into_owned()
}

fn patch_ts_source(src: &str) -> String {
    let src = BIGINT_RE.replace_all(src, "number");
    let src = src.replace(" | undefined", " | null");
    let src = OPTIONAL_FIELD_RE
        .replace_all(&src, "$1: $2 | null")
        .into_owned();
    let src = READABLE_STREAM_RE
        .replace_all(&src, "StreamReadable<$1>")
        .into_owned();
    // Current jco's stream spelling. A `stream<T>` is dwarf's wrapper in
    // EVERY position - handing one to an import passes the readable end
    // just as receiving one does - so this needs no position analysis.
    let src = ASYNC_ITERABLE_RE
        .replace_all(&src, "StreamReadable<$1>")
        .into_owned();
    // In PARAMETER position an ordinary async iterable is accepted too -
    // dwarf lowers a generator straight to a stream - so requiring the
    // wrapper there would reject code that works.
    let src = PARAM_LIST_RE
        .replace_all(&src, |caps: &regex::Captures| {
            format!(
                "({})",
                STREAM_READABLE_PARAM_RE.replace_all(&caps[1], "StreamReadable<$1> | AsyncIterable<$1>")
            )
        })
        .into_owned();
    // Current jco's future spelling, and the reason a top-level return no
    // longer has to be left alone: `PromiseLike<T>` is a `future<T>` value,
    // `Promise<T>` is a genuinely `async func`. Must run before the
    // `Promise<...>` pass below, which is only for jco versions that spelled
    // both the same.
    let src = PROMISE_LIKE_RE
        .replace_all(&src, "FutureReadable<$1>")
        .into_owned();
    let src = patch_future_positions(&src);

    let mut prelude = String::new();
    if src.contains("StreamReadable<") {
        prelude.push_str(STREAM_READABLE_IFACE);
    }
    if src.contains("FutureReadable<") {
        prelude.push_str(FUTURE_READABLE_IFACE);
    }
    prelude + &src
}

#[cfg(test)]
mod tests {
    use super::patch_ts_source;

    /// Verbatim from `jco types` (0.x, 2026-08) for a world exporting a
    /// stream, a non-async func returning `future<result<_, string>>`, a
    /// genuinely `async func`, and a tuple mixing the first two.
    const JCO_OUTPUT: &str = r#"
export function makeStream(): AsyncIterable<number>;
export function takeStream(s: AsyncIterable<number>): void;
export function send(data: Uint8Array): PromiseLike<Result<void, string>>;
export function fetchIt(url: string): Promise<string>;
export function split(): [AsyncIterable<number>, PromiseLike<Result<void, string>>];
export function maybe(n: bigint | undefined): bigint | undefined;
"#;

    #[test]
    fn streams_become_dwarfs_wrapper_in_every_position() {
        let patched = patch_ts_source(JCO_OUTPUT);

        assert!(
            patched.contains("makeStream(): StreamReadable<number>"),
            "a returned stream is the wrapper: {patched}"
        );
        // Not merely the wrapper: it is async-iterable too, so a
        // declaration must admit both. `for await` really works.
        assert!(
            patched.contains("interface StreamReadable<T> extends AsyncIterable<"),
            "the wrapper must extend AsyncIterable: {patched}"
        );
        // A parameter accepts a bare generator, which lowering turns into a
        // stream, so it must not be narrowed to the wrapper alone.
        assert!(
            patched.contains("takeStream(s: StreamReadable<number> | AsyncIterable<number>)"),
            "a stream parameter should accept any async iterable: {patched}"
        );
    }

    #[test]
    fn a_future_return_is_patched_but_an_async_func_return_is_not() {
        // The distinction the whole fix rests on: `PromiseLike<T>` is a
        // future VALUE and must become the wrapper; `Promise<T>` is a
        // genuinely async function and must stay a real Promise. Getting
        // this backwards would be worse than not patching at all.
        let patched = patch_ts_source(JCO_OUTPUT);

        assert!(
            patched.contains("send(data: Uint8Array): FutureReadable<Result<void, string>>"),
            "a future return should become the wrapper: {patched}"
        );
        assert!(
            patched.contains("fetchIt(url: string): Promise<string>"),
            "an async func must keep its real Promise: {patched}"
        );
        assert!(patched.contains("interface FutureReadable<T>"));
    }

    #[test]
    fn a_tuple_mixing_both_is_patched_elementwise() {
        let patched = patch_ts_source(JCO_OUTPUT);
        assert!(
            patched.contains("split(): [StreamReadable<number>, FutureReadable<Result<void, string>>]"),
            "{patched}"
        );
    }

    #[test]
    fn the_older_jco_spellings_are_still_handled() {
        // dwarf targeted these before jco renamed them; a pinned older jco
        // should not regress just because the new names are handled too.
        let old = "export function f(s: ReadableStream<number>, done: Promise<void>): void;\n";
        let patched = patch_ts_source(old);
        assert!(patched.contains("s: StreamReadable<number>"), "{patched}");
        assert!(patched.contains("done: FutureReadable<void>"), "{patched}");
    }

    #[test]
    fn dwarfs_own_conventions_still_apply() {
        let patched = patch_ts_source(JCO_OUTPUT);
        assert!(
            patched.contains("maybe(n: number | null): number | null"),
            "u64 is a number and none is null: {patched}"
        );
    }
}
