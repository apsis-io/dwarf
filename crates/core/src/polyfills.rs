//! JS polyfills bundled with dwarf.
//!
//! Two kinds, matching two different needs:
//!
//! - **Static** polyfills (e.g. `buffer`) are pure JS with no WIT/host
//!   dependency at all - vendored library code (see NOTICES for
//!   provenance), opt-in via `ComponentizeOpts::polyfills`/`--polyfill
//!   <name>` since there's nothing in a WIT world to auto-detect "this is
//!   wanted" from.
//! - **WASI-backed** polyfills (`console`, `process`) wrap a real WASI
//!   interface and need the resolved WIT world to know whether that
//!   interface is actually imported - generated unconditionally into every
//!   shim (matching `console`'s already-shipped behavior), each throwing a
//!   clear "import this to enable it" error at the point of use rather than
//!   failing the build when the backing import is missing.

use std::sync::LazyLock;

use anyhow::{Result, anyhow};
use regex::Regex;
use wit_parser::{Resolve, WorldId, WorldItem};

// ---------------------------------------------------------------------
// Static polyfills
// ---------------------------------------------------------------------

/// A pure-JS, no-WIT-dependency polyfill: `source` is the vendored library
/// code, `install` is a one-line trailer run in the same module scope
/// immediately after it to expose the library's top-level bindings on
/// `globalThis` (module-local `const`/`class`/`function` declarations in
/// `source` aren't visible to user code otherwise - user JS never imports
/// the shim module itself).
pub struct Polyfill {
    pub name: &'static str,
    pub source: &'static str,
    pub install: &'static str,
    pub dts: &'static str,
}

pub const POLYFILLS: &[Polyfill] = &[
    Polyfill {
        name: "buffer",
        source: include_str!("../polyfills/buffer.js"),
        install: "globalThis.Buffer = Buffer;",
        dts: include_str!("../polyfills/buffer.d.ts"),
    },
    Polyfill {
        name: "url",
        source: include_str!("../polyfills/url.js"),
        install: "globalThis.URL = $URL; globalThis.URLSearchParams = $URLSearchParams;",
        dts: include_str!("../polyfills/url.d.ts"),
    },
    Polyfill {
        name: "readable-stream",
        source: include_str!("../polyfills/readable-stream.js"),
        install: "globalThis.ReadableStream = DwarfReadableStream; wit.readableStreamFromStream = dwarfReadableStreamFromWitStream;",
        dts: include_str!("../polyfills/readable-stream.d.ts"),
    },
    Polyfill {
        name: "fetch-classes",
        source: include_str!("../polyfills/fetch-classes.js"),
        install: "globalThis.Headers = Headers; globalThis.Request = Request; globalThis.Response = Response; globalThis.DOMException = DOMException;",
        dts: include_str!("../polyfills/fetch-classes.d.ts"),
    },
    Polyfill {
        name: "path",
        source: include_str!("../polyfills/path.js"),
        install: "globalThis.path = { join, dirname, basename, extname, resolve, relative, normalize, isAbsolute, parse, format, delimiter, sep, posix, win32, matchesGlob, toNamespacedPath, normalizeString };",
        dts: include_str!("../polyfills/path.d.ts"),
    },
    Polyfill {
        name: "webcrypto",
        source: include_str!("../polyfills/webcrypto.js"),
        install: "globalThis.crypto = globalThis.crypto || {}; globalThis.crypto.subtle = subtle;",
        dts: include_str!("../polyfills/webcrypto.d.ts"),
    },
];

/// `.d.ts` for globals dwarf always provides, regardless of `--polyfill` -
/// `console`/`process` (WASI-backed, wired automatically) and
/// `TextEncoder`/`TextDecoder` (no WIT dependency, foundational). Paired with
/// their on-disk filename so `load_override` can find a dev override.
const ALWAYS_ON_DTS: &[(&str, &str)] = &[
    ("builtins.d.ts", include_str!("../polyfills/builtins.d.ts")),
    ("console.d.ts", include_str!("../polyfills/console.d.ts")),
    ("process.d.ts", include_str!("../polyfills/process.d.ts")),
];

fn find(name: &str) -> Result<&'static Polyfill> {
    POLYFILLS.iter().find(|p| p.name == name).ok_or_else(|| {
        let available: Vec<_> = POLYFILLS.iter().map(|p| p.name).collect();
        anyhow!(
            "unknown polyfill `{name}`; available polyfills: {}",
            available.join(", ")
        )
    })
}

/// When set, polyfill `.js`/`.d.ts` content is read fresh from this
/// directory instead of using what's compiled into the binary - so editing
/// a polyfill's source doesn't require rebuilding dwarf itself, only set
/// during development (e.g. `DWARF_POLYFILLS_DIR=crates/core/polyfills`
/// pointed at a local checkout). Falls back to the embedded copy whenever
/// the override file doesn't exist, so this is purely additive - normal
/// (env var unset) usage is unaffected and stays a single self-contained
/// binary with no runtime directory dependency.
fn load_override(filename: &str) -> Option<String> {
    let dir = std::env::var_os("DWARF_POLYFILLS_DIR")?;
    std::fs::read_to_string(std::path::Path::new(&dir).join(filename)).ok()
}

/// Bundled sources end in a trailing ES module `export { ... };` (from how
/// they were bundled) - dead weight already, since `install` grabs the local
/// bindings directly, but invalid syntax once wrapped in a plain function
/// (see `resolve_shim_suffix`), so it's stripped before wrapping.
static TRAILING_EXPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"export\s*\{[\s\S]*?\}\s*;?\s*$").unwrap());

/// Resolves each requested static polyfill name and concatenates its
/// `source` + `install`, each wrapped in its own IIFE so two polyfills that
/// happen to share an internal top-level identifier (confirmed to happen in
/// practice - `buffer.js` and `url.js` each declare their own unrelated
/// top-level `Buffer`/`atob`, `url.js` and `fetch-classes.js` each declare
/// their own `decode`) don't collide with "invalid redefinition of global
/// identifier" when multiple polyfills are requested together. Only the
/// `install` line's explicit `globalThis.X = ...` assignments are meant to
/// cross into the shared scope. Errors naming the invalid entry and listing
/// available polyfills on an unknown name, rather than silently ignoring a
/// typo.
pub fn resolve_shim_suffix(names: &[impl AsRef<str>]) -> Result<String> {
    let mut out = String::new();
    for name in names {
        let polyfill = find(name.as_ref())?;
        let raw = load_override(&format!("{}.js", polyfill.name))
            .unwrap_or_else(|| polyfill.source.to_string());
        let source = TRAILING_EXPORT_RE.replace(&raw, "");
        out.push_str("(function() {\n");
        out.push_str(&source);
        out.push('\n');
        out.push_str(polyfill.install);
        out.push_str("\n})();\n");
    }
    Ok(out)
}

/// Concatenates `.d.ts` declarations for every always-on global plus each
/// requested static polyfill, for `--emit-types` to write out alongside the
/// WIT-derived types - so `--polyfill` and `--emit-types` give full type
/// coverage together rather than `--emit-types` only ever covering WIT
/// interfaces and silently saying nothing about polyfill globals.
pub fn dts_for(names: &[impl AsRef<str>]) -> Result<String> {
    let mut out = String::new();
    for (filename, embedded) in ALWAYS_ON_DTS {
        out.push_str(&load_override(filename).unwrap_or_else(|| embedded.to_string()));
        out.push('\n');
    }
    for name in names {
        let polyfill = find(name.as_ref())?;
        out.push_str(
            &load_override(&format!("{}.d.ts", polyfill.name))
                .unwrap_or_else(|| polyfill.dts.to_string()),
        );
        out.push('\n');
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// WASI-backed polyfills
// ---------------------------------------------------------------------

/// Whether `world_id` imports an interface named `interface_name` from
/// `namespace:package` that has a function named `probe_fn` - version
/// agnostic (matches regardless of `@0.2.x`/`@0.3.x`), and precise enough to
/// tell apart interfaces that share a name but differ in shape across WASI
/// versions (e.g. WASI 0.3's `wasi:cli/stdout` is a structurally different,
/// stream-based interface from 0.2's - probing for a version-specific
/// function name tells them apart correctly).
fn has_wasi_function(
    resolve: &Resolve,
    world_id: WorldId,
    namespace: &str,
    package: &str,
    interface_name: &str,
    probe_fn: &str,
) -> bool {
    resolve.worlds[world_id].imports.values().any(|item| {
        let WorldItem::Interface { id, .. } = item else {
            return false;
        };
        let iface = &resolve.interfaces[*id];
        if iface.name.as_deref() != Some(interface_name) {
            return false;
        }
        let Some(pkg) = iface.package else {
            return false;
        };
        let pkg = &resolve.packages[pkg];
        pkg.name.namespace == namespace
            && pkg.name.name == package
            && iface.functions.contains_key(probe_fn)
    })
}

/// Generates the JS for every WASI-backed polyfill, ready to append to the
/// shim module.
pub(crate) fn generate_wasi_polyfills(resolve: &Resolve, world_id: WorldId) -> String {
    let mut lines = Vec::new();
    generate_builtins(&mut lines);
    generate_console(resolve, world_id, &mut lines);
    generate_process(resolve, world_id, &mut lines);
    generate_crypto_get_random_values(resolve, world_id, &mut lines);
    lines.join("\n") + "\n"
}

/// Generates JS for the small set of standard (WHATWG Encoding spec)
/// globals dwarf's QuickJS runtime doesn't provide out of the box, but
/// which are foundational enough (no WIT/host dependency, and needed by
/// other polyfills like `url`) to always include rather than gate behind
/// `--polyfill`. `encodeUtf8`/`decodeUtf8` are verified byte-for-byte
/// against real `TextEncoder`/`TextDecoder` (ASCII, Latin-1, 2/3/4-byte
/// UTF-8, astral code points, and - for encode - lone/unpaired surrogates
/// replaced with U+FFFD, matching the WHATWG spec).
fn generate_builtins(lines: &mut Vec<String>) {
    // EXPERIMENTAL: registry of fire-and-forget write promises (currently
    // only console.log/info/debug/warn/error's WASI-0.3 fallback), drained
    // by the export dispatcher (bindings.rs's build_async_exports) before
    // it calls task_return - so a library that calls console.log() without
    // awaiting it still gets a flushed write by the time the whole export
    // call completes, instead of being silently cancelled with the task.
    lines.push("globalThis.__dwarfPendingWrites = new Set();".into());
    lines.push("globalThis.__dwarfTrackWrite = function(p) {".into());
    lines.push("  globalThis.__dwarfPendingWrites.add(p);".into());
    lines.push("  const clear = () => { globalThis.__dwarfPendingWrites.delete(p); };".into());
    lines.push("  p.then(clear, clear);".into());
    lines.push("  return p;".into());
    lines.push("};".into());
    lines.push("globalThis.__dwarfDrainPendingWrites = function() {".into());
    lines
        .push("  if (globalThis.__dwarfPendingWrites.size === 0) return Promise.resolve();".into());
    lines.push(
        "  return Promise.allSettled(Array.from(globalThis.__dwarfPendingWrites)).then(function() {"
            .into(),
    );
    lines.push("    return globalThis.__dwarfDrainPendingWrites();".into());
    lines.push("  });".into());
    lines.push("};".into());

    lines.push("globalThis.TextEncoder = class TextEncoder {".into());
    lines.push("  get encoding() { return 'utf-8'; }".into());
    lines.push("  encode(str = '') {".into());
    lines.push("    const bytes = [];".into());
    lines.push("    for (let i = 0; i < str.length; i++) {".into());
    lines.push("      let code = str.charCodeAt(i);".into());
    lines.push("      if (code >= 0xd800 && code <= 0xdbff) {".into());
    lines.push("        const next = str.charCodeAt(i + 1);".into());
    lines.push("        if (next >= 0xdc00 && next <= 0xdfff) {".into());
    lines.push("          code = (code - 0xd800) * 0x400 + (next - 0xdc00) + 0x10000;".into());
    lines.push("          i++;".into());
    lines.push("        } else {".into());
    lines.push("          code = 0xfffd;".into());
    lines.push("        }".into());
    lines.push("      } else if (code >= 0xdc00 && code <= 0xdfff) {".into());
    lines.push("        code = 0xfffd;".into());
    lines.push("      }".into());
    lines.push("      if (code < 0x80) { bytes.push(code); }".into());
    lines.push(
        "      else if (code < 0x800) { bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f)); }"
            .into(),
    );
    lines.push("      else if (code < 0x10000) { bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)); }".into());
    lines.push("      else { bytes.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)); }".into());
    lines.push("    }".into());
    lines.push("    return Uint8Array.from(bytes);".into());
    lines.push("  }".into());
    lines.push("};".into());

    lines.push("globalThis.TextDecoder = class TextDecoder {".into());
    lines.push("  constructor(label = 'utf-8') { this.encoding = label; }".into());
    lines.push("  decode(input) {".into());
    // A raw ArrayBuffer (as returned by e.g. the webcrypto polyfill's
    // subtle.decrypt, matching the real spec) has no `.length`/numeric
    // indexing of its own - only ArrayBufferView (TypedArray/DataView) does.
    // Passing one through unwrapped silently decoded as "" (len === undefined
    // makes the loop below never run) instead of throwing or working -
    // wrap it in a Uint8Array view first, matching TextDecoder's real
    // `BufferSource = ArrayBuffer | ArrayBufferView` input type.
    lines.push(
        "    const bytes = input == null ? [] : (input instanceof ArrayBuffer ? new Uint8Array(input) : input);"
            .into(),
    );
    lines.push("    let out = '';".into());
    lines.push("    let i = 0;".into());
    lines.push("    const len = bytes.length;".into());
    lines.push("    while (i < len) {".into());
    lines.push("      const b0 = bytes[i];".into());
    lines.push("      if (b0 < 0x80) { out += String.fromCharCode(b0); i += 1; }".into());
    lines.push("      else if ((b0 & 0xe0) === 0xc0 && i + 1 < len) {".into());
    lines.push(
        "        out += String.fromCharCode(((b0 & 0x1f) << 6) | (bytes[i + 1] & 0x3f));".into(),
    );
    lines.push("        i += 2;".into());
    lines.push("      } else if ((b0 & 0xf0) === 0xe0 && i + 2 < len) {".into());
    lines.push("        out += String.fromCharCode(((b0 & 0x0f) << 12) | ((bytes[i + 1] & 0x3f) << 6) | (bytes[i + 2] & 0x3f));".into());
    lines.push("        i += 3;".into());
    lines.push("      } else if ((b0 & 0xf8) === 0xf0 && i + 3 < len) {".into());
    lines.push("        let code = ((b0 & 0x07) << 18) | ((bytes[i + 1] & 0x3f) << 12) | ((bytes[i + 2] & 0x3f) << 6) | (bytes[i + 3] & 0x3f);".into());
    lines.push("        code -= 0x10000;".into());
    lines.push(
        "        out += String.fromCharCode(0xd800 + (code >> 10), 0xdc00 + (code & 0x3ff));"
            .into(),
    );
    lines.push("        i += 4;".into());
    lines.push("      } else { out += '\\ufffd'; i += 1; }".into());
    lines.push("    }".into());
    lines.push("    return out;".into());
    lines.push("  }".into());
    lines.push("};".into());
}

fn has_cli(resolve: &Resolve, world_id: WorldId, interface_name: &str, probe_fn: &str) -> bool {
    has_wasi_function(resolve, world_id, "wasi", "cli", interface_name, probe_fn)
}

/// Wires `console.log`/`info`/`debug`/`warn`/`error` to `wasi:cli/stdout`/
/// `stderr`, preferring the WASI 0.2 sync interface (matched by
/// `get-stdout`/`get-stderr`) when the world imports it - genuinely
/// synchronous, matches real `console.log`'s non-Promise contract exactly -
/// and falling back to the WASI 0.3 `write-via-stream` path (matched by
/// that function name, so it's told apart from 0.2's structurally different
/// interface of the same name) when only that's available. `console` always
/// exists; the half backed by neither import throws a clear error naming
/// both options instead of silently no-op-ing or leaving `console`
/// undefined entirely.
///
/// The 0.3 fallback makes `log`/`info`/`debug`/`warn`/`error` Promise-
/// returning in a 0.3-only world (unlike the always-synchronous 0.2 path) -
/// unavoidable, since WASI 0.3 has no synchronous write primitive at all.
/// Callers that need the write to have completed before continuing must
/// await it; fire-and-forget calls carry the same completion-ordering
/// caveat already documented for `print`/`println` below. This reuses that
/// same async writer rather than a separate code path.
///
/// Also wires `console.print`/`println`/`eprint`/`eprintln` - always async
/// (Promise-returning) regardless of which WASI version backs them, using
/// WASI 0.3's `wasi:cli/stdout`/`stderr` (matched by `write-via-stream`)
/// when imported, falling back to the WASI 0.2 sync write wrapped in an
/// async fn otherwise. Calling the WASI 0.3 write machinery from a plain
/// sync export has no task state at all and crashes outright (verified
/// empirically - "no active task state"), so only an explicitly-async,
/// explicitly-awaited surface is ever offered for it - true of `print`/
/// `println` unconditionally, and true of `log`/`info`/`debug`/`warn`/
/// `error` specifically in the 0.3-fallback case above.
fn generate_console(resolve: &Resolve, world_id: WorldId, lines: &mut Vec<String>) {
    let stdout = has_cli(resolve, world_id, "stdout", "get-stdout");
    let stderr = has_cli(resolve, world_id, "stderr", "get-stderr");
    let stdout_async = has_cli(resolve, world_id, "stdout", "write-via-stream");
    let stderr_async = has_cli(resolve, world_id, "stderr", "write-via-stream");

    if stdout {
        lines.push(r#"import __consoleStdout from "wasi:cli/stdout";"#.into());
    }
    if stderr {
        lines.push(r#"import __consoleStderr from "wasi:cli/stderr";"#.into());
    }
    if stdout_async {
        lines.push(r#"import __consoleStdoutAsync from "wasi:cli/stdout";"#.into());
    }
    if stderr_async {
        lines.push(r#"import __consoleStderrAsync from "wasi:cli/stderr";"#.into());
    }

    lines.push("globalThis.console = (function() {".into());
    lines.push("  function encodeUtf8(str) {".into());
    lines.push("    const bytes = [];".into());
    lines.push("    for (let i = 0; i < str.length; i++) {".into());
    lines.push("      let code = str.charCodeAt(i);".into());
    lines.push("      if (code >= 0xd800 && code <= 0xdbff) {".into());
    lines.push("        const next = str.charCodeAt(i + 1);".into());
    lines.push("        if (next >= 0xdc00 && next <= 0xdfff) {".into());
    lines.push("          code = (code - 0xd800) * 0x400 + (next - 0xdc00) + 0x10000;".into());
    lines.push("          i++;".into());
    lines.push("        } else {".into());
    lines.push("          code = 0xfffd;".into()); // unpaired high surrogate
    lines.push("        }".into());
    lines.push("      } else if (code >= 0xdc00 && code <= 0xdfff) {".into());
    lines.push("        code = 0xfffd;".into()); // unpaired low surrogate
    lines.push("      }".into());
    lines.push("      if (code < 0x80) { bytes.push(code); }".into());
    lines.push(
        "      else if (code < 0x800) { bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f)); }"
            .into(),
    );
    lines.push("      else if (code < 0x10000) { bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)); }".into());
    lines.push("      else { bytes.push(0xf0 | (code >> 18), 0x80 | ((code >> 12) & 0x3f), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f)); }".into());
    lines.push("    }".into());
    lines.push("    return bytes;".into());
    lines.push("  }".into());
    lines.push("  function formatPlain(args) {".into());
    lines.push("    return args.map((a) => typeof a === 'string' ? a : (a instanceof Error ? (a.stack || a.message) : JSON.stringify(a))).join(' ');".into());
    lines.push("  }".into());
    lines.push("  function format(args) { return formatPlain(args) + '\\n'; }".into());

    if stdout {
        // getStdout() returns a live host resource handle, so it must be
        // re-acquired on every call rather than cached at module scope - a
        // handle captured at Wizer-init time is baked into the snapshotted
        // heap but refers to nothing in a real run's fresh resource table
        // (the same class of bug as capturing raw resources across a
        // checkpoint boundary).
        lines.push("  const writeStdout = (...args) => { __consoleStdout.getStdout().blockingWriteAndFlush(encodeUtf8(format(args))); };".into());
    } else if stdout_async {
        // No WASI 0.2 stdout import (and per the WIT-ecosystem constraint
        // documented in the module docs, a p3 world including
        // wasi:cli/command@0.3.0 cannot add one) - fall back to the 0.3
        // write-via-stream path, same as print/println. This makes
        // log/info/debug Promise-returning in a 0.3-only world (unlike the
        // always-synchronous 0.2 path above) - callers that need the write
        // to have actually completed before continuing must await it, same
        // caveat that already applies to print/println.
        lines.push("  const writeStdout = (...args) => __dwarfTrackWrite(writeStdoutAsync(encodeUtf8(format(args))));".into());
    } else {
        lines.push("  const writeStdout = () => { throw new Error(\"console.log/info/debug requires importing wasi:cli/stdout@0.2.x or wasi:cli/stdout@0.3.x in your WIT world\"); };".into());
    }

    if stderr {
        lines.push("  const writeStderr = (...args) => { __consoleStderr.getStderr().blockingWriteAndFlush(encodeUtf8(format(args))); };".into());
    } else if stderr_async {
        lines.push("  const writeStderr = (...args) => __dwarfTrackWrite(writeStderrAsync(encodeUtf8(format(args))));".into());
    } else {
        lines.push("  const writeStderr = () => { throw new Error(\"console.warn/error requires importing wasi:cli/stderr@0.2.x or wasi:cli/stderr@0.3.x in your WIT world\"); };".into());
    }

    emit_async_writer(
        lines,
        "Stdout",
        stdout_async,
        "__consoleStdoutAsync",
        stdout,
        "__consoleStdout.getStdout()",
        "print/println",
        "wasi:cli/stdout",
    );
    emit_async_writer(
        lines,
        "Stderr",
        stderr_async,
        "__consoleStderrAsync",
        stderr,
        "__consoleStderr.getStderr()",
        "eprint/eprintln",
        "wasi:cli/stderr",
    );

    lines.push("  return Object.freeze({".into());
    lines.push("    log: writeStdout, info: writeStdout, debug: writeStdout, warn: writeStderr, error: writeStderr,".into());
    lines.push("    print: (...args) => writeStdoutAsync(encodeUtf8(formatPlain(args))),".into());
    lines.push("    println: (...args) => writeStdoutAsync(encodeUtf8(format(args))),".into());
    lines.push("    eprint: (...args) => writeStderrAsync(encodeUtf8(formatPlain(args))),".into());
    lines.push("    eprintln: (...args) => writeStderrAsync(encodeUtf8(format(args))),".into());
    lines.push("  });".into());
    lines.push("})();".into());
}

/// Emits `write{var_suffix}Async(bytes) -> Promise<void>`: uses WASI 0.3's
/// `write-via-stream` when available (genuinely async, a real `stream<u8>`
/// handed off and awaited via its `future<result<_, error-code>>`), falls
/// back to wrapping the WASI 0.2 sync write in an `async` function
/// otherwise (still Promise-returning, for a uniform API), and otherwise
/// throws inside an `async` function (i.e. a rejected promise, the correct
/// "not available" signal for an async API) naming both import options.
#[allow(clippy::too_many_arguments)]
fn emit_async_writer(
    lines: &mut Vec<String>,
    var_suffix: &str,
    has_async: bool,
    async_import: &str,
    has_sync: bool,
    sync_getter_expr: &str,
    methods: &str,
    interface: &str,
) {
    let var = format!("write{var_suffix}Async");
    if has_async {
        lines.push(format!("  const {var} = async (bytes) => {{"));
        // Component-model stream/future operations require an active async
        // task (a genuine `async func` export call in progress) - without
        // one, `wit.Stream()`/its writeViaStream/read chain aborts the whole
        // guest outright (a raw wasm trap, not a catchable JS exception).
        // This matters beyond the already-documented "called from a plain
        // sync export" case: dwarf's own Wizer build-time module-init call
        // is ALSO a plain (non-async) export, so any top-level module code
        // (a library's own import-time side effect, not just user code)
        // that logs something in a WASI-0.3-only world would hit this too -
        // confirmed empirically. Check first and throw a normal, catchable
        // Error instead.
        lines.push(format!(
            "    if (!__cqjs.hasActiveTask()) {{ throw new Error(\"console.{methods} (via {interface}@0.3.x) requires an active async task - it can't be called from a plain sync export, or from top-level module code running during dwarf's build-time init (e.g. a library's own import-time side effect). Import {interface}@0.2.x for a version that works everywhere, or only call this from within an `async func` export.\"); }}"
        ));
        lines.push("    const { readable, writable } = wit.Stream(wit.Stream.U8);".into());
        lines.push("    const writeDone = writable.writeAll(bytes);".into());
        lines.push(format!(
            "    const futureResult = {async_import}.writeViaStream(readable);"
        ));
        lines.push("    await writeDone;".into());
        lines.push("    writable.drop();".into());
        lines.push("    const result = await futureResult.read();".into());
        lines.push(format!(
            "    if (result && result.tag === 'err') {{ throw new Error(\"console.{methods} write failed: \" + JSON.stringify(result.val)); }}"
        ));
        lines.push("  };".into());
    } else if has_sync {
        lines.push(format!(
            "  const {var} = async (bytes) => {{ {sync_getter_expr}.blockingWriteAndFlush(bytes); }};"
        ));
    } else {
        lines.push(format!(
            "  const {var} = async () => {{ throw new Error(\"console.{methods} requires importing {interface}@0.3.x (async) or {interface}@0.2.x (sync fallback) in your WIT world\"); }};"
        ));
    }
}

/// Wires `process.env`/`process.argv`/`process.cwd()` to
/// `wasi:cli/environment` and `process.exit(code)` to `wasi:cli/exit`'s
/// `exit-with-code`, when the world imports them. `environment`/`exit` are
/// unchanged in shape between WASI 0.2 and 0.3, so unlike `console` there's
/// no version-specific branching needed. All four are getters/methods
/// (never captured at module scope) for the same reason `console`'s stdout
/// handle is re-acquired per call: `wasi:cli/environment`'s functions return
/// plain data, not a resource handle, but that data would still be
/// captured from dwarf's own *build-time* Wizer environment if fetched
/// eagerly at module top level - the real *runtime* environment (whatever
/// it is for a given instantiation) is only correct if fetched lazily,
/// on each access.
///
/// Divergences from Node's `process` worth knowing: `argv` is exactly
/// `wasi:cli/environment`'s `get-arguments()` with no synthetic
/// `node`/script-path entries prepended (WASI has no such convention);
/// `cwd()` returns `null` (not a fabricated path) when
/// `initial-cwd()` is `option::none`; `exit(code)` maps onto
/// `wasi:cli/exit`'s `exit-with-code(status-code: u8)`, so `code` is
/// coerced into a single byte the same way Node itself truncates exit
/// codes outside 0-255.
fn generate_process(resolve: &Resolve, world_id: WorldId, lines: &mut Vec<String>) {
    let has_env = has_cli(resolve, world_id, "environment", "get-environment");
    let has_exit = has_cli(resolve, world_id, "exit", "exit-with-code");

    if has_env {
        lines.push(r#"import __processEnv from "wasi:cli/environment";"#.into());
    }
    if has_exit {
        lines.push(r#"import __processExit from "wasi:cli/exit";"#.into());
    }

    lines.push("globalThis.process = Object.freeze({".into());

    if has_env {
        lines.push(
            "  get env() { return Object.fromEntries(__processEnv.getEnvironment()); },".into(),
        );
        lines.push("  get argv() { return __processEnv.getArguments(); },".into());
        lines.push("  cwd() { return __processEnv.initialCwd(); },".into());
    } else {
        let msg = "process.env/argv/cwd requires importing wasi:cli/environment (e.g. add `import wasi:cli/environment@0.2.x;` to your WIT world)";
        lines.push(format!("  get env() {{ throw new Error(\"{msg}\"); }},"));
        lines.push(format!("  get argv() {{ throw new Error(\"{msg}\"); }},"));
        lines.push(format!("  cwd() {{ throw new Error(\"{msg}\"); }},"));
    }

    if has_exit {
        lines.push(
            "  exit(code) { __processExit.exitWithCode(((code ?? 0) & 0xff) >>> 0); },".into(),
        );
    } else {
        lines.push("  exit() { throw new Error(\"process.exit requires importing wasi:cli/exit (e.g. add `import wasi:cli/exit@0.2.x;` to your WIT world)\"); },".into());
    }

    lines.push("});".into());
}

/// Wires `crypto.getRandomValues` to `wasi:random/random#get-random-bytes`
/// when the world imports it - always generated (matching `console`/
/// `process`'s "wired automatically, clear error if the backing import is
/// missing" convention), independent of the `webcrypto` static polyfill
/// (`--polyfill webcrypto`, `crates/core/polyfills/webcrypto.js`) which
/// provides `crypto.subtle` and has no WIT/host dependency of its own.
/// Runs before that static polyfill's install line (`generate_wasi_polyfills`
/// is emitted into the shim ahead of `resolve_shim_suffix`, see
/// `lib.rs::componentize`), which matters because `@noble/*`'s own
/// `randomBytes()` helper calls `globalThis.crypto.getRandomValues`
/// internally (confirmed by reading `@noble/hashes/utils.js`) - so
/// `crypto.subtle.generateKey` works transparently once this is wired,
/// with no extra glue needed in the static polyfill itself.
fn generate_crypto_get_random_values(
    resolve: &Resolve,
    world_id: WorldId,
    lines: &mut Vec<String>,
) {
    let has_random = has_wasi_function(
        resolve,
        world_id,
        "wasi",
        "random",
        "random",
        "get-random-bytes",
    );

    lines.push("globalThis.crypto = globalThis.crypto || {};".into());

    if has_random {
        lines.push(r#"import __wasiRandom from "wasi:random/random";"#.into());
        lines.push("globalThis.crypto.getRandomValues = function(typedArray) {".into());
        lines.push("  const bytes = __wasiRandom.getRandomBytes(typedArray.byteLength);".into());
        lines.push(
            "  new Uint8Array(typedArray.buffer, typedArray.byteOffset, typedArray.byteLength).set(bytes);"
                .into(),
        );
        lines.push("  return typedArray;".into());
        lines.push("};".into());
    } else {
        lines.push("globalThis.crypto.getRandomValues = function() { throw new Error(\"crypto.getRandomValues requires importing wasi:random/random (e.g. add `import wasi:random/random;` to your WIT world)\"); };".into());
    }
}
