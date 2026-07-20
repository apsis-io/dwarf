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
//!
//! Every WASI-backed polyfill's implementation is built under an explicit
//! `...P3` name (`consoleP3`, `processP3`, `crypto.getRandomValuesP3`,
//! `setTimeoutP3`/`setIntervalP3`/`clearTimeoutP3`/`clearIntervalP3`,
//! `fetchP3`, `WebSocketServerP3`) - the plain name (`console`, `fetch`,
//! etc.) is then aliased FROM the `P3` one (`globalThis.console =
//! globalThis.consoleP3;`), never the other way around. This direction
//! matters: the plain name is the "current best available" convenience
//! binding, meant to be free to repoint at a newer version's implementation
//! later (a hypothetical WASI 0.4/"p4") without notice; `xP3` is meant to
//! always mean "the 0.3 implementation, forever," for code that wants that
//! guarantee. Building `xP3 = <impl>; x = xP3;` (in that order) keeps that
//! true even after a future version is added; the reverse (`x = <impl>;
//! xP3 = x;`) would silently break it the moment `x` gets repointed at
//! something newer, since `xP3` would just be whatever `x` happened to be
//! at generation time, not necessarily the 0.3 implementation specifically.
//! Vendored library code that itself needs one of these internally (e.g.
//! `webcrypto.js`'s bundled `@noble/*` code, which upstream calls
//! `crypto.getRandomValues`) is patched at vendor time to call the `P3`
//! name directly, for the same reason - see
//! `crates/core/polyfills/webcrypto.js`.

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
    Polyfill {
        name: "ufo",
        source: include_str!("../polyfills/ufo.js"),
        install: "globalThis.ufo = { $URL, cleanDoubleSlashes, createURL, decode, decodePath, decodeQueryKey, decodeQueryValue, encode, encodeHash, encodeHost, encodeParam, encodePath, encodeQueryItem, encodeQueryKey, encodeQueryValue, filterQuery, getQuery, hasLeadingSlash, hasProtocol, hasTrailingSlash, isEmptyURL, isEqual, isNonEmptyURL, isRelative, isSamePath, isScriptProtocol, joinRelativeURL, joinURL, normalizeURL, parseAuth, parseFilename, parseHost, parsePath, parseQuery, parseURL, resolveURL, stringifyParsedURL, stringifyQuery, withBase, withFragment, withHttp, withHttps, withLeadingSlash, withProtocol, withQuery, withTrailingSlash, withoutBase, withoutFragment, withoutHost, withoutLeadingSlash, withoutProtocol, withoutTrailingSlash };",
        dts: include_str!("../polyfills/ufo.d.ts"),
    },
    Polyfill {
        name: "scule",
        source: include_str!("../polyfills/scule.js"),
        install: "globalThis.scule = { camelCase, flatCase, isUppercase, kebabCase, lowerFirst, pascalCase, snakeCase, splitByCase, titleCase, trainCase, upperFirst };",
        dts: include_str!("../polyfills/scule.d.ts"),
    },
    Polyfill {
        name: "klona",
        source: include_str!("../polyfills/klona.js"),
        install: "globalThis.klona = klona;",
        dts: include_str!("../polyfills/klona.d.ts"),
    },
    Polyfill {
        name: "ohash",
        source: include_str!("../polyfills/ohash.js"),
        install: "globalThis.ohash = { hash, serialize, isEqual };",
        dts: include_str!("../polyfills/ohash.d.ts"),
    },
    Polyfill {
        name: "knitwork",
        source: include_str!("../polyfills/knitwork.js"),
        install: "globalThis.knitwork = { escapeString, genArrayFromRaw, genAugmentation, genDynamicImport, genDynamicTypeImport, genExport, genImport, genInlineTypeImport, genInterface, genObjectFromRaw, genObjectFromRawEntries, genObjectFromValues, genObjectKey, genSafeVariableName, genString, genTypeExport, genTypeImport, genTypeObject, wrapInDelimiters };",
        dts: include_str!("../polyfills/knitwork.d.ts"),
    },
    Polyfill {
        name: "unstorage",
        source: include_str!("../polyfills/unstorage.js"),
        install: "globalThis.unstorage = { builtinDrivers, createStorage, defineDriver, filterKeyByBase, filterKeyByDepth, joinKeys, normalizeBaseKey, normalizeKey, prefixStorage, restoreSnapshot, snapshot };",
        dts: include_str!("../polyfills/unstorage.d.ts"),
    },
];

/// `.d.ts` for globals dwarf always provides, regardless of `--polyfill` -
/// `console`/`process`/`WebSocketServer` (WASI-backed, wired automatically)
/// and `TextEncoder`/`TextDecoder` (no WIT dependency, foundational). Paired
/// with their on-disk filename so `load_override` can find a dev override.
/// `fetch`/`fetchP3`'s types live in `fetch-classes.d.ts` instead (gated
/// behind `--polyfill fetch-classes`), not here - its signature references
/// `Request`/`Response`, which only exist when that polyfill is requested;
/// `WebSocketServer`'s signature has no such cross-dependency (needing
/// `--polyfill webcrypto` is a runtime requirement of `.listen()`, not
/// something its type signature references), so it's always-on like
/// `console`/`process`.
const ALWAYS_ON_DTS: &[(&str, &str)] = &[
    ("builtins.d.ts", include_str!("../polyfills/builtins.d.ts")),
    ("console.d.ts", include_str!("../polyfills/console.d.ts")),
    ("process.d.ts", include_str!("../polyfills/process.d.ts")),
    (
        "websocket-server.d.ts",
        include_str!("../polyfills/websocket-server.d.ts"),
    ),
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
    generate_timers(resolve, world_id, &mut lines);
    generate_fetch(resolve, world_id, &mut lines);
    generate_websocket(resolve, world_id, &mut lines);
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

    generate_abort_controller(lines);
}

/// `AbortController`/`AbortSignal`, no WIT/host dependency (foundational,
/// like `TextEncoder`/`TextDecoder` above) - real semantics (`abort()` flips
/// `aborted`, sets `reason`, fires `onabort`/`'abort'` listeners), not the
/// no-op stand-in `fetch-classes.d.ts` used to document (`{ readonly aborted:
/// boolean }`, "nothing observes signal"). `fetch-classes.js` already
/// optionally uses a global `AbortController` if present
/// (`"AbortController" in g`); this is what makes that branch real. Uses a
/// plain `Error` (named `"AbortError"`) as the default abort reason when
/// `DOMException` isn't also available (the `fetch-classes` polyfill's, not
/// a hard dependency of this) - matches the real default reason's `.name`
/// even without the real class. No `AbortSignal.timeout()` static: that
/// needs `setTimeout`, and this stays dependency-free on purpose.
fn generate_abort_controller(lines: &mut Vec<String>) {
    lines.push("function __dwarfAbortDefaultReason() {".into());
    lines.push("  if (typeof DOMException !== 'undefined') return new DOMException('signal is aborted without reason', 'AbortError');".into());
    lines.push("  const e = new Error('signal is aborted without reason');".into());
    lines.push("  e.name = 'AbortError';".into());
    lines.push("  return e;".into());
    lines.push("}".into());

    lines.push("globalThis.AbortSignal = class AbortSignal {".into());
    lines.push("  constructor() {".into());
    lines.push("    this._aborted = false;".into());
    lines.push("    this._reason = undefined;".into());
    lines.push("    this._listeners = new Set();".into());
    lines.push("    this.onabort = null;".into());
    lines.push("  }".into());
    lines.push("  get aborted() { return this._aborted; }".into());
    lines.push("  get reason() { return this._reason; }".into());
    lines.push("  throwIfAborted() { if (this._aborted) throw this._reason; }".into());
    lines.push("  addEventListener(type, listener) { if (type === 'abort') this._listeners.add(listener); }".into());
    lines.push("  removeEventListener(type, listener) { if (type === 'abort') this._listeners.delete(listener); }".into());
    lines.push("  _signalAbort(reason) {".into());
    lines.push("    if (this._aborted) return;".into());
    lines.push("    this._aborted = true;".into());
    lines.push(
        "    this._reason = reason === undefined ? __dwarfAbortDefaultReason() : reason;".into(),
    );
    lines.push("    const event = { type: 'abort', target: this };".into());
    lines.push("    if (typeof this.onabort === 'function') this.onabort(event);".into());
    lines.push("    for (const listener of this._listeners) listener(event);".into());
    lines.push("  }".into());
    lines.push("  static abort(reason) {".into());
    lines.push("    const signal = new AbortSignal();".into());
    lines.push(
        "    signal._signalAbort(reason === undefined ? __dwarfAbortDefaultReason() : reason);"
            .into(),
    );
    lines.push("    return signal;".into());
    lines.push("  }".into());
    lines.push("};".into());

    lines.push("globalThis.AbortController = class AbortController {".into());
    lines.push("  constructor() { this.signal = new AbortSignal(); }".into());
    lines.push("  abort(reason) { this.signal._signalAbort(reason); }".into());
    lines.push("};".into());
}

fn has_cli(resolve: &Resolve, world_id: WorldId, interface_name: &str, probe_fn: &str) -> bool {
    has_wasi_function(resolve, world_id, "wasi", "cli", interface_name, probe_fn)
}

/// Wires `console.log`/`info`/`debug`/`warn`/`error` to `wasi:cli/stdout`/
/// `stderr`'s WASI 0.3 `write-via-stream` interface (matched by that
/// function name) when the world imports it. `console` always exists; the
/// half backed by neither import throws a clear error naming it instead of
/// silently no-op-ing or leaving `console` undefined entirely.
///
/// This makes `log`/`info`/`debug`/`warn`/`error` Promise-returning -
/// unavoidable, since WASI 0.3 has no synchronous write primitive at all.
/// Callers that need the write to have completed before continuing must
/// await it; fire-and-forget calls carry the same completion-ordering
/// caveat already documented for `print`/`println` below. This reuses that
/// same async writer rather than a separate code path.
///
/// Also wires `console.print`/`println`/`eprint`/`eprintln` - always async
/// (Promise-returning), using WASI 0.3's `wasi:cli/stdout`/`stderr`
/// (matched by `write-via-stream`) when imported. Calling the write
/// machinery from a plain sync export has no task state at all and crashes
/// outright (verified empirically - "no active task state"), so only an
/// explicitly-async, explicitly-awaited surface is ever offered for it -
/// true of `print`/`println` unconditionally, and true of
/// `log`/`info`/`debug`/`warn`/`error` as well.
fn generate_console(resolve: &Resolve, world_id: WorldId, lines: &mut Vec<String>) {
    let stdout_async = has_cli(resolve, world_id, "stdout", "write-via-stream");
    let stderr_async = has_cli(resolve, world_id, "stderr", "write-via-stream");

    if stdout_async {
        lines.push(r#"import __consoleStdoutAsync from "wasi:cli/stdout";"#.into());
    }
    if stderr_async {
        lines.push(r#"import __consoleStderrAsync from "wasi:cli/stderr";"#.into());
    }

    lines.push("globalThis.consoleP3 = (function() {".into());
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

    if stdout_async {
        lines.push("  const writeStdout = (...args) => __dwarfTrackWrite(writeStdoutAsync(encodeUtf8(format(args))));".into());
    } else {
        lines.push("  const writeStdout = () => { throw new Error(\"console.log/info/debug requires importing wasi:cli/stdout@0.3.x in your WIT world\"); };".into());
    }

    if stderr_async {
        lines.push("  const writeStderr = (...args) => __dwarfTrackWrite(writeStderrAsync(encodeUtf8(format(args))));".into());
    } else {
        lines.push("  const writeStderr = () => { throw new Error(\"console.warn/error requires importing wasi:cli/stderr@0.3.x in your WIT world\"); };".into());
    }

    emit_async_writer(
        lines,
        "Stdout",
        stdout_async,
        "__consoleStdoutAsync",
        "print/println",
        "wasi:cli/stdout",
    );
    emit_async_writer(
        lines,
        "Stderr",
        stderr_async,
        "__consoleStderrAsync",
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
    lines.push("globalThis.console = globalThis.consoleP3;".into());
}

/// Emits `write{var_suffix}Async(bytes) -> Promise<void>`: uses WASI 0.3's
/// `write-via-stream` when available (genuinely async, a real `stream<u8>`
/// handed off and awaited via its `future<result<_, error-code>>`), and
/// otherwise throws inside an `async` function (i.e. a rejected promise, the
/// correct "not available" signal for an async API) naming the import.
fn emit_async_writer(
    lines: &mut Vec<String>,
    var_suffix: &str,
    has_async: bool,
    async_import: &str,
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
            "    if (!__cqjs.hasActiveTask()) {{ throw new Error(\"console.{methods} (via {interface}@0.3.x) requires an active async task - it can't be called from a plain sync export, or from top-level module code running during dwarf's build-time init (e.g. a library's own import-time side effect). Only call this from within an `async func` export.\"); }}"
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
    } else {
        lines.push(format!(
            "  const {var} = async () => {{ throw new Error(\"console.{methods} requires importing {interface}@0.3.x in your WIT world\"); }};"
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

    lines.push("globalThis.processP3 = Object.freeze({".into());

    if has_env {
        lines.push(
            "  get env() { return Object.fromEntries(__processEnv.getEnvironment()); },".into(),
        );
        lines.push("  get argv() { return __processEnv.getArguments(); },".into());
        lines.push("  cwd() { return __processEnv.initialCwd(); },".into());
    } else {
        let msg = "process.env/argv/cwd requires importing wasi:cli/environment (e.g. add `import wasi:cli/environment@0.3.x;` to your WIT world)";
        lines.push(format!("  get env() {{ throw new Error(\"{msg}\"); }},"));
        lines.push(format!("  get argv() {{ throw new Error(\"{msg}\"); }},"));
        lines.push(format!("  cwd() {{ throw new Error(\"{msg}\"); }},"));
    }

    if has_exit {
        lines.push(
            "  exit(code) { __processExit.exitWithCode(((code ?? 0) & 0xff) >>> 0); },".into(),
        );
    } else {
        lines.push("  exit() { throw new Error(\"process.exit requires importing wasi:cli/exit (e.g. add `import wasi:cli/exit@0.3.x;` to your WIT world)\"); },".into());
    }

    lines.push("});".into());
    lines.push("globalThis.process = globalThis.processP3;".into());
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
        lines.push("globalThis.crypto.getRandomValuesP3 = function(typedArray) {".into());
        lines.push("  const bytes = __wasiRandom.getRandomBytes(typedArray.byteLength);".into());
        lines.push(
            "  new Uint8Array(typedArray.buffer, typedArray.byteOffset, typedArray.byteLength).set(bytes);"
                .into(),
        );
        lines.push("  return typedArray;".into());
        lines.push("};".into());
    } else {
        lines.push("globalThis.crypto.getRandomValuesP3 = function() { throw new Error(\"crypto.getRandomValuesP3 requires importing wasi:random/random (e.g. add `import wasi:random/random;` to your WIT world)\"); };".into());
    }
    lines.push("globalThis.crypto.getRandomValues = globalThis.crypto.getRandomValuesP3;".into());
}

/// Wires `setTimeout`/`setInterval` to `wasi:clocks/monotonic-clock@0.3.x`'s
/// `wait-for` (a genuine `async func`, directly awaitable with no
/// stream/future ceremony - unlike `write-via-stream`, `wait-for` carries no
/// stream/future-typed payload of its own). WASI 0.2's `monotonic-clock` has
/// no non-blocking wait primitive at all (only `subscribe-duration` +
/// `wasi:io/poll`'s `pollable`, which *blocks* the whole component - the
/// opposite of what `setTimeout` is for), so only 0.3 is supported; matched
/// via the `wait-for` probe (absent from 0.2, which only has
/// `subscribe-duration`/`subscribe-instant`).
///
/// `clearTimeout`/`clearInterval` are always defined and never throw
/// (clearing is inherently idempotent/harmless, even in a world where
/// `setTimeout` itself would throw) - only creating a new timer needs the
/// import.
///
/// IMPORTANT CAVEAT, unavoidable under component-model-async (the same class
/// of limitation already documented for `console`'s fire-and-forget writes,
/// see `__dwarfDrainPendingWrites` above): once the async export that (transitively)
/// called `setTimeout`/`setInterval` settles, any still-pending timer callback
/// that wasn't part of the explicitly-awaited chain is cancelled along with
/// the rest of that task - there is no way for dwarf to keep a timer alive
/// past its originating export call the way a real JS host's event loop
/// would. Reliable only when awaited (e.g. a `sleep(ms)` helper built on
/// this) or when called from a still-running, long-lived export.
fn generate_timers(resolve: &Resolve, world_id: WorldId, lines: &mut Vec<String>) {
    let has_wait_for = has_wasi_function(
        resolve,
        world_id,
        "wasi",
        "clocks",
        "monotonic-clock",
        "wait-for",
    );

    if has_wait_for {
        lines.push(r#"import __wasiMonotonicClock from "wasi:clocks/monotonic-clock";"#.into());
    }

    lines.push("(function() {".into());
    lines.push("  let nextHandle = 1;".into());
    lines.push("  const cancelled = new Set();".into());
    lines.push("  globalThis.clearTimeoutP3 = function(handle) { cancelled.add(handle); };".into());
    lines.push("  globalThis.clearIntervalP3 = globalThis.clearTimeoutP3;".into());

    if has_wait_for {
        lines.push("  function checkActiveTask(name) {".into());
        lines.push(
            "    if (!__cqjs.hasActiveTask()) { throw new Error(name + \" requires an active async task - it can't be called from a plain sync export, or from top-level module code running during dwarf's build-time init (e.g. a library's own import-time side effect). Only call this from within an `async func` export.\"); }"
                .into(),
        );
        lines.push("  }".into());

        lines.push("  globalThis.setTimeoutP3 = function(fn, ms, ...args) {".into());
        lines.push("    checkActiveTask('setTimeoutP3');".into());
        lines.push("    const handle = nextHandle++;".into());
        lines.push("    const nanos = BigInt(Math.max(0, Math.trunc(ms || 0))) * 1000000n;".into());
        lines.push("    __wasiMonotonicClock.waitFor(nanos).then(function() {".into());
        lines.push("      if (cancelled.has(handle)) { cancelled.delete(handle); return; }".into());
        lines.push("      fn(...args);".into());
        lines.push("    });".into());
        lines.push("    return handle;".into());
        lines.push("  };".into());

        lines.push("  globalThis.setIntervalP3 = function(fn, ms, ...args) {".into());
        lines.push("    checkActiveTask('setIntervalP3');".into());
        lines.push("    const handle = nextHandle++;".into());
        lines.push("    const nanos = BigInt(Math.max(0, Math.trunc(ms || 0))) * 1000000n;".into());
        lines.push("    (function tick() {".into());
        lines.push("      if (cancelled.has(handle)) { cancelled.delete(handle); return; }".into());
        lines.push("      __wasiMonotonicClock.waitFor(nanos).then(function() {".into());
        lines.push(
            "        if (cancelled.has(handle)) { cancelled.delete(handle); return; }".into(),
        );
        lines.push("        fn(...args);".into());
        lines.push("        tick();".into());
        lines.push("      });".into());
        lines.push("    })();".into());
        lines.push("    return handle;".into());
        lines.push("  };".into());
    } else {
        let msg = "requires importing wasi:clocks/monotonic-clock@0.3.x (e.g. add `import wasi:clocks/monotonic-clock@0.3.x;` to your WIT world) - wasi:clocks 0.2 has no non-blocking wait primitive, so this can't be backed by it";
        lines.push(format!(
            "  globalThis.setTimeoutP3 = function() {{ throw new Error(\"setTimeoutP3 {msg}\"); }};"
        ));
        lines.push(format!(
            "  globalThis.setIntervalP3 = function() {{ throw new Error(\"setIntervalP3 {msg}\"); }};"
        ));
    }

    lines.push("})();".into());
    lines.push("globalThis.setTimeout = globalThis.setTimeoutP3;".into());
    lines.push("globalThis.setInterval = globalThis.setIntervalP3;".into());
    lines.push("globalThis.clearTimeout = globalThis.clearTimeoutP3;".into());
    lines.push("globalThis.clearInterval = globalThis.clearIntervalP3;".into());
}

/// Wires a global `fetch(input, init)` directly to `wasi:http/client@0.3.x`,
/// generated only when the world imports it (matched via the `send`
/// function - `wasi:http` 0.2 has no `client` interface at all, so no
/// version ambiguity to worry about, unlike `console`).
///
/// This is the one case among dwarf's WASI-backed polyfills that looks like
/// it should be impossible: `examples/fetch-provider`'s own doc comment
/// explains that `wit.Stream`/`wit.Future` type indices are "auto-assigned
/// per-component based on every other stream/future type that component's
/// own WIT world happens to use - not something glue code injected into an
/// arbitrary caller's world could reliably reference," which is why
/// `fetch-provider` is a separate, fixed-world component composed in via
/// `wac plug` rather than a polyfill. That constraint is about a *prebuilt,
/// reusable* component meant to be composed into many different consumers
/// without rebuilding - it doesn't apply to dwarf's own codegen, which
/// generates fresh JS for each consuming component's own specific WIT world
/// at build time. And critically, `wit.Future`/`wit.Stream` CONSTANT NAMES
/// (`U8`, `RESULT_OPTION_OTHER_ERROR_CODE`, `RESULT_VOID_ERROR_CODE`) are
/// derived purely structurally from the payload type's shape (see
/// `codegen.rs`'s `type_const_name`) - not from the type's position or
/// anything else in that world - so any world that imports
/// `wasi:http/client` (which necessarily pulls in `wasi:http/types`' fixed
/// future/stream shapes for trailers/bodies) computes the exact same
/// constant names on its own generated `wit.Future`/`wit.Stream` object,
/// even though the numeric indices those names resolve to differ per
/// component. Hardcoding the JS source text below is therefore safe; only
/// hardcoding a raw numeric index would not be (see `unique_const_names` for
/// the one narrow caveat: an unrelated anonymous future/stream elsewhere in
/// the same world that happens to share an identical nested option/result
/// shape would collide on the bare name - vanishingly unlikely for
/// wasi:http's specific shapes, and no worse than the same risk any other
/// WASI-backed polyfill already accepts).
///
/// The actual request/response construction below is a direct port of
/// `examples/fetch-provider/main.js` (verified there against real HTTP
/// servers) merged with its `fetch.js` DX layer, so `fetch()` here accepts a
/// URL string or a `Request` and returns a real `Response` directly, with no
/// intermediate flattened wire format (unlike `fetch-provider`, which needs
/// one only because it's a separate, plain-types-only-boundary component).
///
/// Requires the `fetch-classes` polyfill (`--polyfill fetch-classes`) for
/// `Request`/`Response`/`Headers` - referenced only inside the function
/// body (resolved at call time, not definition time), so generation order
/// relative to that static polyfill doesn't matter, but calling `fetch()`
/// without also requesting `fetch-classes` throws a plain `ReferenceError`.
/// Same single-read response body limitation as `fetch-provider` (a real
/// drain loop needs a proven end-of-stream signal for `wasi:io`'s
/// `stream.read`, not yet verified) - large bodies are truncated.
fn generate_fetch(resolve: &Resolve, world_id: WorldId, lines: &mut Vec<String>) {
    let has_client = has_wasi_function(resolve, world_id, "wasi", "http", "client", "send");

    if !has_client {
        lines.push("globalThis.fetchP3 = function() { throw new Error(\"fetchP3() requires importing wasi:http/client@0.3.x (e.g. add `import wasi:http/client@0.3.x;` to your WIT world) and the fetch-classes polyfill (--polyfill fetch-classes) for Request/Response/Headers\"); };".into());
        lines.push("globalThis.fetch = globalThis.fetchP3;".into());
        return;
    }

    lines.push(
        r#"import { Fields as __wasiHttpFields, Request as __WasiHttpRequest, Response as __WasiHttpResponse } from "wasi:http/types";"#
            .into(),
    );
    lines.push(r#"import { send as __wasiHttpSend } from "wasi:http/client";"#.into());

    lines.push("globalThis.fetchP3 = async function fetchP3(input, init = {}) {".into());
    lines.push(
        "  if (!__cqjs.hasActiveTask()) { throw new Error(\"fetchP3() (via wasi:http/client) requires an active async task - it can't be called from a plain sync export, or from top-level module code running during dwarf's build-time init. Only call this from within an `async func` export.\"); }"
            .into(),
    );
    lines.push(
        "  const request = input instanceof Request ? input : new Request(input, init);".into(),
    );
    lines.push(
        "  const m = /^(https?):\\/\\/([^/:?#]+)(?::(\\d+))?([^?#]*)(\\?[^#]*)?/.exec(request.url);"
            .into(),
    );
    lines.push("  if (!m) throw new TypeError(\"fetch: invalid URL: \" + request.url);".into());
    lines.push("  const [, scheme, host, port, path, query] = m;".into());
    lines.push("  const authority = port ? `${host}:${port}` : host;".into());
    lines.push("  const pathWithQuery = (path || '/') + (query || '');".into());

    lines.push("  const headerEntries = [];".into());
    lines.push("  for (const [name, value] of request.headers.entries()) {".into());
    lines.push(
        "    headerEntries.push([name, Array.from(new TextEncoder().encode(value))]);".into(),
    );
    lines.push("  }".into());
    lines.push("  const headers = __wasiHttpFields.fromList(headerEntries);".into());

    lines.push("  const buf = await request.arrayBuffer();".into());
    lines.push("  const hasBody = buf.byteLength > 0;".into());
    lines.push("  let bodyStream = null;".into());
    lines.push("  let bodyWritable = null;".into());
    lines.push("  if (hasBody) {".into());
    lines.push("    const pair = wit.Stream(wit.Stream.U8);".into());
    lines.push("    bodyStream = pair.readable;".into());
    lines.push("    bodyWritable = pair.writable;".into());
    lines.push("  }".into());

    lines.push(
        "  const trailersFuture = wit.Future(wit.Future.RESULT_OPTION_OTHER_ERROR_CODE);".into(),
    );
    lines.push("  trailersFuture.writable.write({ tag: 'ok', val: null });".into());

    lines.push(
        "  const __fetchMethods = new Set(['get', 'head', 'post', 'put', 'delete', 'connect', 'options', 'trace', 'patch']);"
            .into(),
    );
    lines.push("  const methodLower = (request.method || 'GET').toLowerCase();".into());
    lines.push(
        "  const methodTag = __fetchMethods.has(methodLower) ? { tag: methodLower } : { tag: 'other', val: request.method };"
            .into(),
    );

    lines.push(
        "  const [wireRequest, transmitFuture] = __WasiHttpRequest.new(headers, bodyStream, trailersFuture.readable, null);"
            .into(),
    );
    lines.push("  wireRequest.setMethod(methodTag);".into());
    lines.push("  wireRequest.setScheme({ tag: scheme === 'https' ? 'HTTPS' : 'HTTP' });".into());
    lines.push("  wireRequest.setAuthority(authority);".into());
    lines.push("  wireRequest.setPathWithQuery(pathWithQuery);".into());

    lines.push("  let response;".into());
    lines.push("  try {".into());
    lines.push("    // send() must start before the body is written - the writable end".into());
    lines.push("    // has no reader until the request is actually in flight, so awaiting".into());
    lines
        .push("    // the write first would hang waiting for a reader that never shows up.".into());
    lines.push("    const sendPromise = __wasiHttpSend(wireRequest);".into());
    lines.push("    if (hasBody) {".into());
    lines.push("      await bodyWritable.writeAll(Array.from(new Uint8Array(buf)));".into());
    lines.push("      bodyWritable.drop();".into());
    lines.push("    }".into());
    lines.push("    response = await sendPromise;".into());
    lines.push("  } catch (e) {".into());
    lines.push(
        "    const wrapped = new TypeError('fetch failed: ' + (e && e.message ? e.message : JSON.stringify(e)));"
            .into(),
    );
    lines.push(
        "    if (e && typeof e === 'object' && 'payload' in e) wrapped.payload = e.payload;".into(),
    );
    lines.push("    throw wrapped;".into());
    lines.push("  }".into());

    lines.push("  const status = response.getStatusCode();".into());
    lines.push("  const responseHeaders = new Headers();".into());
    lines.push("  for (const [name, value] of response.getHeaders().copyAll()) {".into());
    lines.push(
        "    responseHeaders.append(name, new TextDecoder().decode(Uint8Array.from(value)));"
            .into(),
    );
    lines.push("  }".into());

    lines.push("  const voidFuture = wit.Future(wit.Future.RESULT_VOID_ERROR_CODE);".into());
    lines.push("  voidFuture.writable.write({ tag: 'ok', val: undefined });".into());
    lines.push(
        "  // A single read, not a drain loop: known limitation (see examples/fetch-provider),"
            .into(),
    );
    lines.push(
        "  // bodies larger than this are truncated - a real drain loop needs a proven".into(),
    );
    lines.push("  // end-of-stream signal for wasi:io's stream.read, not yet verified.".into());
    lines.push(
        "  const [respBodyStream] = __WasiHttpResponse.consumeBody(response, voidFuture.readable);"
            .into(),
    );
    lines.push("  const bytes = await respBodyStream.read(65536);".into());
    lines.push("  respBodyStream.drop();".into());
    lines.push("  await transmitFuture.read();".into());
    lines.push("  transmitFuture.drop();".into());

    lines.push(
        "  return new Response(Uint8Array.from(bytes), { status, headers: responseHeaders });"
            .into(),
    );
    lines.push("};".into());
    lines.push("globalThis.fetch = globalThis.fetchP3;".into());
}

/// Wires a global `WebSocketServer` class to `wasi:sockets/types@0.3.0`'s
/// `tcp-socket` resource, generated only when the world imports it (matched
/// via the resource's static `create` function, which `wit_parser` names
/// `[static]tcp-socket.create` - confirmed against the real calling
/// convention in `tests/wit/sockprobe/probe.js`: `TcpSocket.create("ipv4")`
/// throws/returns directly per dwarf's usual `result<T, error-code>`
/// convention, `await sock.connect(...)`/`sock.listen()` etc. follow the
/// same shape).
///
/// The bulk of the implementation - the RFC 6455 frame parser/builder, the
/// HTTP upgrade handshake, and the `WebSocketServer`/connection wrapper
/// classes - lives in `websocket-server.js` (a hand-adapted port of
/// websockets/ws's receiver.js, see NOTICES) since none of it needs to
/// change based on the WIT world's shape; only the decision of whether to
/// wire it up at all, and the `wasi:sockets/types` import line itself, are
/// generated here, matching `generate_fetch`'s split between Rust-generated
/// wiring and hand-written protocol logic.
///
/// Requires the `webcrypto` polyfill (`--polyfill webcrypto`) for computing
/// the `Sec-WebSocket-Accept` handshake header via `crypto.subtle.digest
/// ("SHA-1", ...)` - referenced only inside an async function body, so
/// generation order doesn't matter, but `new WebSocketServer().listen(...)`
/// without also requesting `webcrypto` throws a clear error naming it.
///
/// `WebSocketServer` also doubles as a general single-port HTTP+WS router:
/// registering an `on("request", (request) => response)` handler
/// (additionally requires `--polyfill fetch-classes`, for `Request`/
/// `Response` - same lazy, throws-a-plain-`ReferenceError`-without-it
/// convention as `fetch()`) routes any non-upgrade HTTP request there,
/// HTTP/1.1 keep-alive included, while WS upgrade requests still go through
/// the handshake/frame path unchanged. This exists because some hosts can
/// only reach a component on a single port (e.g. a TLS-passthrough-by-SNI
/// funnel with one backend) - splitting a normal HTTP response path (SSR)
/// and WebSocket upgrades across two ports isn't reachable through that
/// kind of edge. Without a `"request"` handler registered, behavior is
/// unchanged from before this existed: a non-upgrade request is dropped.
///
/// Scope cuts, matching this codebase's existing "documented limitation"
/// style rather than a half-finished implementation: IPv4 only, no
/// permessage-deflate (never negotiated, so real clients simply don't use
/// it), text/binary messages delivered as a plain `string`/`Uint8Array` (no
/// Blob/ArrayBuffer `binaryType` switch - there's no `Blob` in dwarf), no
/// backpressure signaling on `send()` beyond a per-connection write queue.
/// The HTTP router side has its own cuts: no chunked transfer-encoding on
/// the request side (`Content-Length` only, rejected outright with `501`
/// rather than silently mishandled), no `Expect: 100-continue`.
///
/// The HTTP router parses untrusted network input, so it's hardened
/// against hostile/malformed requests: bounded header block size/count
/// (16 KiB / 100 headers, `431` if exceeded), strict request-line/
/// `Content-Length` validation (`400` on anything malformed), an oversized
/// `Content-Length` rejected (`413`) before any of the body is read (not
/// after buffering an attacker-declared amount), and an idle-read timeout
/// (`idleTimeoutMs`, default 30s) so a connection that opens and then sends
/// nothing/trickles bytes forever (slowloris) doesn't hold a connection or
/// task open indefinitely - only enforced when the world imports
/// `wasi:clocks/monotonic-clock@0.3.x` (same graceful-degradation shape as
/// `setTimeout` itself), since the HTTP router shouldn't otherwise depend
/// on a WIT import unrelated to sockets. Notably, the timeout path does
/// NOT call the stream's `cancelRead()` on the losing side of its
/// `Promise.race` - confirmed empirically that doing so traps the guest
/// ("waitable cannot be used synchronously while added to a waitable
/// set"), a hard wasm trap rather than a catchable error; dropping the
/// connection's sockets/streams at the end of `handleConnection` (already
/// happening regardless) is sufficient to actually close the underlying
/// connection, without needing to proactively cancel the abandoned read.
fn generate_websocket(resolve: &Resolve, world_id: WorldId, lines: &mut Vec<String>) {
    let has_tcp = has_wasi_function(
        resolve,
        world_id,
        "wasi",
        "sockets",
        "types",
        "[static]tcp-socket.create",
    );

    if !has_tcp {
        lines.push("globalThis.WebSocketServerP3 = class WebSocketServerP3 { constructor() { throw new Error(\"WebSocketServerP3 requires importing wasi:sockets/types@0.3.0 (e.g. add `import wasi:sockets/types@0.3.0;` to your WIT world)\"); } };".into());
        lines.push("globalThis.WebSocketServer = globalThis.WebSocketServerP3;".into());
        return;
    }

    lines.push(r#"import { TcpSocket } from "wasi:sockets/types@0.3.0";"#.into());
    lines.push(include_str!("../polyfills/websocket-server.js").into());
    lines.push("globalThis.WebSocketServer = globalThis.WebSocketServerP3;".into());
}
