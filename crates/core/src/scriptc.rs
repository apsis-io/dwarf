//! Statically compiled TypeScript, plugged into the component being built.
//!
//! dwarf's answer for hot code is to not run it in QuickJS at all: scriptc
//! compiles a TypeScript module ahead of time and this plugs the result in
//! as a component. The division of labour is the point — QuickJS keeps
//! everything dynamic, and the leaf modules doing real work over numbers,
//! strings, and bytes become native Wasm.
//!
//! What happens here, per profile:
//! 1. `scriptc build --lib --component` produces a component plus the WIT
//!    describing it (both generated from the profile, so they agree).
//! 2. That WIT package joins the user's `Resolve` and its interface is
//!    added to the world as an import, which is what makes it reachable
//!    from JavaScript as a module specifier.
//! 3. After the JavaScript component is built, the scriptc component is
//!    plugged into that import (see `plug_scriptc`), leaving nothing of
//!    the seam in the final component's imports.
//!
//! scriptc's wasm target is wasm32-wasip3, whose link runs
//! wasm-component-ld: the component arrives already componentized and
//! importing WASI directly, so no preview1 adapter is involved on this
//! side at all. dwarf's own embedded adapter serves dwarf's own module.
//! Callers configure nothing beyond naming the profile.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use wac_graph::types::Package;
use wac_graph::{CompositionGraph, EncodeOptions, plug};
use wit_parser::{Resolve, Stability, WorldId, WorldItem, WorldKey};

/// One built scriptc component and the WIT identifying it.
pub struct ScriptcComponent {
    /// The component bytes, ready to plug.
    pub wasm: Vec<u8>,
    /// The generated `.wit`. It names the package and interface, so dwarf
    /// never has to parse the profile to learn either.
    pub wit_path: PathBuf,
    /// The specifier JavaScript imports this under, filled in once the WIT
    /// joins the resolve (`scriptc:<profile name>/ops` by default).
    pub specifier: String,
}

/// Build one module into a component by invoking scriptc.
///
/// `source` is either a profile, which declares the boundary, or a
/// TypeScript module, whose exported signatures scriptc derives one from.
/// `work_dir` receives the archive, the generated shim and WIT, and the
/// component itself. scriptc reports the paths it wrote on stdout, one per
/// line, and names any export that could not cross on stderr — passed
/// through here, since a missing export is something the author needs to
/// see rather than discover at run time.
pub fn build(source: &Path, scriptc_bin: &Path, work_dir: &Path) -> Result<ScriptcComponent> {
    std::fs::create_dir_all(work_dir)
        .with_context(|| format!("failed to create {}", work_dir.display()))?;
    let stem = source
        .file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "scriptc".to_string());
    // A profile is passed by flag; a module is the positional input.
    let is_profile = source.extension().is_some_and(|e| e == "json");

    // No --wit-package: scriptc defaults it to scriptc:<profile name>, and
    // the generated WIT below is where dwarf learns what that was.
    let archive = work_dir.join(format!("{stem}.lib.a"));
    let mut command = Command::new(scriptc_bin);
    command.args(["build", "--lib", "--component"]);
    if is_profile {
        command.arg("--profile");
    }
    let output = command
        .arg(source)
        .arg("-o")
        .arg(&archive)
        // scriptc's one wasm target. Its link runs wasm-component-ld, so
        // the component comes out componentized and needs no preview1
        // adapter from us — dwarf's own embedded adapter is for dwarf's
        // own module, and never crosses over here.
        .env("SCRIPTC_TARGET", "wasm32-wasip3")
        .output()
        .with_context(|| {
            format!(
                "failed to run {} — install scriptc or name it with --scriptc-bin",
                scriptc_bin.display()
            )
        })?;

    if !output.status.success() {
        bail!(
            "scriptc failed to build {}:\n{}{}",
            source.display(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
    }

    // scriptc's notes about exports that stayed out of the boundary, and
    // the derived profile's location, ride stderr.
    let notes = String::from_utf8_lossy(&output.stderr);
    for line in notes.lines().filter(|l| !l.trim().is_empty()) {
        eprintln!("{line}");
    }

    // stdout is the component then the WIT, one path per line.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines().filter(|l| !l.trim().is_empty()).rev();
    let (Some(wit_line), Some(component_line)) = (lines.next(), lines.next()) else {
        bail!(
            "scriptc did not report the component and WIT it wrote for {}:\n{stdout}",
            source.display()
        );
    };
    let component_path = PathBuf::from(component_line.trim());
    let wit_path = PathBuf::from(wit_line.trim());
    let wasm = std::fs::read(&component_path).with_context(|| {
        format!(
            "scriptc reported success but {} is missing",
            component_path.display()
        )
    })?;

    Ok(ScriptcComponent {
        wasm,
        wit_path,
        specifier: String::new(),
    })
}

/// Add a built component's interface to `world` as an import, so the
/// generated JavaScript shim exposes it and the seam type-checks.
pub fn import_into_world(
    resolve: &mut Resolve,
    world_id: WorldId,
    component: &mut ScriptcComponent,
) -> Result<()> {
    let wit = std::fs::read_to_string(&component.wit_path).with_context(|| {
        format!(
            "failed to read generated WIT {}",
            component.wit_path.display()
        )
    })?;
    let pkg_id = resolve
        .push_str(&component.wit_path, &wit)
        .with_context(|| {
            format!(
                "failed to add generated WIT {}",
                component.wit_path.display()
            )
        })?;

    let pkg_name = resolve.packages[pkg_id].name.clone();
    let Some((iface_name, iface_id)) = resolve.packages[pkg_id]
        .interfaces
        .iter()
        .next()
        .map(|(n, id)| (n.clone(), *id))
    else {
        bail!(
            "generated WIT {} declares no interface",
            component.wit_path.display()
        );
    };
    component.specifier = format!("{pkg_name}/{iface_name}");

    let key = WorldKey::Interface(iface_id);
    if resolve.worlds[world_id].imports.contains_key(&key) {
        // The world already declared it by hand; that spelling wins and
        // the plug below still satisfies it.
        return Ok(());
    }
    resolve.worlds[world_id].imports.insert(
        key,
        WorldItem::Interface {
            id: iface_id,
            stability: Stability::Unknown,
            docs: Default::default(),
            span: Default::default(),
            // wit-parser 0.254 carries an `@external-id` attribute here;
            // dwarf synthesises this import rather than parsing one, so
            // there is nothing to preserve.
            external_id: None,
        },
    );
    Ok(())
}

/// The interface names a component still imports.
///
/// A plugged seam disappears from here, which is the only honest check: the
/// interface name stays behind in the composition's own metadata, so
/// searching the bytes says nothing.
pub fn import_names(component: &[u8]) -> Result<Vec<String>> {
    use wit_parser::decoding::{DecodedWasm, decode};

    match decode(component).context("failed to decode component WIT")? {
        DecodedWasm::Component(resolve, world_id) => Ok(resolve.worlds[world_id]
            .imports
            .keys()
            .map(|key| resolve.name_world_key(key))
            .collect()),
        _ => bail!("expected a component, got a WIT package"),
    }
}

/// Plug built scriptc components into `component`, satisfying the imports
/// `import_into_world` added. The same composition `stub_wasi_imports`
/// performs, with real implementations instead of traps.
pub fn plug_scriptc(component: &[u8], parts: &[ScriptcComponent]) -> Result<Vec<u8>> {
    if parts.is_empty() {
        return Ok(component.to_vec());
    }

    let mut graph = CompositionGraph::new();
    let socket = Package::from_bytes("app", None, component.to_vec(), graph.types_mut())
        .context("failed to register the JavaScript component")?;
    let socket_id = graph.register_package(socket)?;

    let mut plug_ids = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let label = format!("scriptc{i}");
        let pkg = Package::from_bytes(&label, None, part.wasm.clone(), graph.types_mut())
            .with_context(|| format!("failed to register scriptc component {}", part.specifier))?;
        plug_ids.push(graph.register_package(pkg)?);
    }

    plug(&mut graph, plug_ids, socket_id).context("failed to plug scriptc components")?;

    graph
        .encode(EncodeOptions::default())
        .context("failed to encode the composed component")
}
