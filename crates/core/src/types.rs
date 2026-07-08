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
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("failed to create types output directory {}", out_dir.display()))?;

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
/// - A `stream<T>` value is typed `ReadableStream<T>`, but dwarf's runtime
///   hands back its own `StreamReadable<T>` wrapper (`read(count?)`,
///   `cancelRead()`, `drop()`) - not a real `ReadableStream` (no
///   `.getReader()`).
/// - A `future<T>` value *in parameter or tuple-return-element position* is
///   typed as a bare `Promise<T>`, but dwarf's runtime hands back its own
///   `FutureReadable<T>` wrapper (`read()`, `cancelRead()`, `drop()`) there
///   too - not a real `Promise` (no direct `.then()`, must call `.read()`
///   first). This patch deliberately does NOT touch a function's own
///   top-level return type, since `Promise<T>` there is genuinely ambiguous
///   between a real Promise (a genuinely `async` WIT function, which dwarf's
///   runtime represents as an actual native Promise) and a `future<T>` value
///   returned directly from a *non-async* function - jco's output doesn't
///   textually distinguish these two cases at that position, so patching it
///   blindly would risk turning a real Promise into a wrapper type or vice
///   versa. Hand-verify against the WIT signature for that specific case.
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

static READABLE_STREAM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"ReadableStream<({BALANCED_GENERIC_BODY})>")).unwrap()
});
static PROMISE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"Promise<({BALANCED_GENERIC_BODY})>")).unwrap());
static TUPLE_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\[\]]*)\]").unwrap());
static PARAM_LIST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(([^()]*)\)").unwrap());

const STREAM_READABLE_IFACE: &str = "interface StreamReadable<T> {\n  read(count?: number): Promise<T extends number ? Uint8Array : T[]>;\n  cancelRead(): void | { progress: number; result: number };\n  drop(): void;\n}\n";
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
