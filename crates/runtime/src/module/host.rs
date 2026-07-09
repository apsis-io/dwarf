use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Error, Module};

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

impl Resolver for HostModuleResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
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
