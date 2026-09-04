use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Error, Module};

use crate::CtxExt;
use crate::init::local::init::module_loader;
use crate::module_loader_available;

/// Only static `import`s reachable during Wizer's build-time module
/// evaluation ever resolve - see `JsState::module_loader_available`'s doc
/// comment. A dynamic `import()` a bundled dependency reaches lazily at
/// real runtime (e.g. only the first time a particular code path actually
/// runs) hits this after the flag has flipped.
const RUNTIME_IMPORT_MESSAGE: &str = "dynamic import() (or any import reached after dwarf's \
    build-time init has finished) is not supported - dwarf's module resolution needs real \
    filesystem access, which only exists during Wizer's build-time pre-init. Make sure every \
    module your code can reach is imported statically, or configure your bundler to inline \
    dynamic imports (e.g. Rollup/Vite's build.rollupOptions.output.inlineDynamicImports: true)";

pub(super) struct HostModuleResolver;

/// Whether a specifier is asking for a WIT interface rather than a file.
///
/// WIT specifiers are `package:name/interface`, optionally `@version`. A
/// relative or absolute path is a file; anything else carrying a `:` before
/// any `/` is someone reaching for WIT.
fn looks_like_wit(name: &str) -> bool {
    if name.starts_with('.') || name.starts_with('/') {
        return false;
    }
    match (name.find(':'), name.find('/')) {
        (Some(colon), Some(slash)) => colon < slash,
        (Some(_), None) => true,
        _ => false,
    }
}

/// The error for a WIT-shaped specifier that no interface answers to.
///
/// This used to fall through to the filesystem resolver, which reported
/// "filesystem module not found" for something that was never going to be
/// a file - true, and useless, because the reader is not looking for a
/// file. It also could not mention the most common cause: a world-level
/// function import is NOT a module. dwarf puts those on `globalThis`, so
/// they are called with no import at all, and nothing said so.
///
/// The detail goes to the BUILD'S STDERR rather than into the exception.
/// QuickJS truncates a thrown error's message at 256 bytes and the
/// "Error resolving module 'x' from 'y': " prefix already spends a third
/// of that, so a full explanation arrives cut mid-word. The exception
/// carries the one-line cause; `build-log` carries the rest, unbounded,
/// and is printed as it happens.
fn wit_import_error<'js>(ctx: &Ctx<'js>, base: &str, name: &str) -> Error {
    let registry = ctx.wit_import_registry();
    let specifiers = registry.importable_specifiers();
    let globals = registry.root_global_names();

    if module_loader_available() {
        let mut detail = format!(
            "\n  `{name}` is not a WIT interface this world imports.\n"
        );

        if specifiers.is_empty() {
            detail.push_str("\n  This world imports no interfaces at all.\n");
        } else {
            detail.push_str("\n  Interfaces this world imports, and their import specifiers:\n");
            for spec in &specifiers {
                detail.push_str(&format!("    import ... from \"{spec}\";\n"));
            }
        }

        if globals.is_empty() {
            detail.push_str(
                "\n  This world also has no world-level function imports.\n",
            );
        } else {
            detail.push_str(
                "\n  A WORLD-LEVEL function import (`import foo: func();` written\n\
                 \x20 directly in the world, not inside an interface) is NOT a module.\n\
                 \x20 dwarf puts each one on globalThis, so you call it with no import:\n\n",
            );
            for g in &globals {
                detail.push_str(&format!("    {g}(...)   // no import line\n"));
            }
        }

        detail.push_str(
            "\n  A WIT import specifier is `package:name/interface`. A world's own\n\
             \x20 name is not an interface, so `pkg:ns/<world>` never resolves.\n",
        );

        module_loader::build_log("stderr", detail.as_bytes());
    }

    // Short enough to survive QuickJS's 256-byte truncation once the
    // "Error resolving module ... from ..." prefix is added.
    let hint = if globals.is_empty() {
        "not a WIT interface in this world".to_string()
    } else {
        format!(
            "not an interface; world-level imports are globals - call {}() with no import",
            globals[0]
        )
    };
    Error::new_resolving_message(base, name, hint)
}

impl Resolver for HostModuleResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        // Reached only after the WIT resolver declined, so a WIT-shaped
        // specifier here names no interface. Say that, rather than letting
        // it fail later as a missing file.
        if looks_like_wit(name) {
            return Err(wit_import_error(_ctx, base, name));
        }
        if !module_loader_available() {
            return Err(Error::new_resolving_message(
                base,
                name,
                RUNTIME_IMPORT_MESSAGE,
            ));
        }
        module_loader::resolve(base, name)
            .map_err(|err| Error::new_resolving_message(base, name, err))
    }
}

pub(super) struct HostModuleLoader;

impl Loader for HostModuleLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        if !module_loader_available() {
            return Err(Error::new_loading_message(name, RUNTIME_IMPORT_MESSAGE));
        }
        let source =
            module_loader::load(name).map_err(|err| Error::new_loading_message(name, err))?;
        Module::declare(ctx.clone(), name, source)
    }
}
