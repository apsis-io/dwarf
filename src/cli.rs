use dwarf_core::{ComponentizeOpts, Runtime, ScriptcConfig, componentize_with};

use anyhow::{Context, Result};
use clap::Parser;

use std::fs;
use std::path::Path;

/// Renders `path`'s display form with a trailing separator when it's a directory,
/// so e.g. `--wit wit` is echoed back as `wit/` rather than looking like a file.
fn display_path(path: &Path) -> String {
    let rendered = path.display().to_string();
    if path.is_dir() && !rendered.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{rendered}{}", std::path::MAIN_SEPARATOR)
    } else {
        rendered
    }
}

#[derive(Parser)]
#[command(name = "dwarf")]
#[command(about = "Convert TypeScript/JavaScript to WebAssembly components using QuickJS")]
pub struct CliArgs {
    /// Path to the WIT file or directory
    #[arg(short, long)]
    pub wit: std::path::PathBuf,

    /// Path to the entry module: TypeScript (`.ts`/`.mts`/`.cts`, types
    /// stripped, never checked) or JavaScript
    ///
    /// `--js`/`-j` remain accepted: the flag was renamed because the file it
    /// points at is the build's input, and saying "js" in it made a
    /// TypeScript project name the wrong language on every invocation.
    #[arg(
        short = 'f',
        long = "file",
        visible_alias = "js",
        short_alias = 'j',
        value_name = "PATH"
    )]
    pub file: std::path::PathBuf,

    /// Root directory exposed during Wizer for resolving JavaScript imports
    #[arg(long, value_name = "PATH")]
    pub module_root: Option<std::path::PathBuf>,

    /// Output path for the component
    #[arg(short, long, default_value = "output.wasm")]
    pub output: std::path::PathBuf,

    /// World name to use from the WIT
    #[arg(short = 'n', long)]
    pub world: Option<String>,

    /// Stub all WASI imports with traps
    #[arg(long)]
    pub stub_wasi: bool,

    /// Disable automatically fetching missing WIT dependencies via `wkg fetch`
    #[arg(long)]
    pub no_vendor: bool,

    /// Also generate TypeScript type declarations for the WIT world via `jco types`
    #[arg(long, value_name = "DIR")]
    pub emit_types: Option<std::path::PathBuf>,

    /// Include a static polyfill (repeatable), e.g. `--polyfill buffer`
    #[arg(long = "polyfill", value_name = "NAME")]
    pub polyfills: Vec<String>,

    /// Minify the JS source via oxc before componentizing
    #[arg(short = 'm', long)]
    pub minify: bool,

    /// Disable automatic garbage collection in the QuickJS runtime
    #[arg(long)]
    pub disable_gc: bool,

    /// Use the built-in runtime optimized for smaller generated components
    #[arg(long, conflicts_with = "runtime")]
    pub opt_size: bool,

    /// Use the built-in non-async runtime, producing components that do not use
    /// the component-model async ABI
    #[arg(long, conflicts_with = "runtime")]
    pub sync: bool,

    /// Path to a custom QuickJS runtime Wasm module
    #[arg(long, value_name = "PATH")]
    pub runtime: Option<std::path::PathBuf>,

    /// Compile a TypeScript module statically with scriptc and plug it in
    /// (repeatable). The boundary is derived from the module's exported
    /// signatures; JavaScript imports it as `scriptc:<name>/ops`.
    #[arg(long = "optimize", value_name = "MODULE")]
    pub optimize: Vec<std::path::PathBuf>,

    /// Like --optimize, but from a scriptc profile that declares the
    /// boundary explicitly instead of deriving it (repeatable).
    #[arg(long = "scriptc", value_name = "PROFILE")]
    pub scriptc: Vec<std::path::PathBuf>,

    /// The scriptc executable to use for --scriptc (default: `scriptc` on PATH)
    #[arg(long, value_name = "PATH")]
    pub scriptc_bin: Option<std::path::PathBuf>,
}

/// Run the dwarf CLI with the given arguments.
pub async fn run(args: Vec<String>) -> Result<()> {
    let args = CliArgs::try_parse_from(std::iter::once("dwarf".to_string()).chain(args))?;

    if !args.wit.exists() {
        anyhow::bail!("WIT file/directory not found: {}", args.wit.display());
    }
    if !args.file.exists() {
        anyhow::bail!("Source file not found: {}", args.file.display());
    }
    if let Some(runtime_file) = &args.runtime
        && !runtime_file.exists()
    {
        anyhow::bail!("Runtime file not found: {}", runtime_file.display());
    }
    if let Some(module_root) = &args.module_root
        && !module_root.exists()
    {
        anyhow::bail!("Module root not found: {}", module_root.display());
    }

    let js_source = fs::read_to_string(&args.file)
        .with_context(|| format!("failed to read source file: {}", args.file.display()))?;

    println!("dwarf");
    println!("  WIT:    {}", display_path(&args.wit));
    println!("  Source: {}", args.file.display());
    println!("  Output: {}", args.output.display());

    let custom_runtime = args
        .runtime
        .as_ref()
        .map(|file| fs::read(file).context("failed to read runtime file"))
        .transpose()?;

    let runtime = match custom_runtime.as_deref() {
        Some(wasm) => Runtime::Custom(wasm),
        None => Runtime::builtin(args.sync, args.opt_size),
    };

    if args.stub_wasi {
        println!("Stubbing WASI imports...");
    }

    // --optimize and --scriptc are the same pipeline; scriptc tells a
    // module from a profile by its extension.
    let statically_compiled: Vec<std::path::PathBuf> = args
        .optimize
        .iter()
        .chain(args.scriptc.iter())
        .cloned()
        .collect();
    for module in &statically_compiled {
        if !module.exists() {
            anyhow::bail!("not found: {}", module.display());
        }
        println!("Compiling {} statically with scriptc...", display_path(module));
    }

    let polyfills: Vec<&str> = args.polyfills.iter().map(String::as_str).collect();
    let component = componentize_with(
        &ComponentizeOpts {
            wit_path: &args.wit,
            js_source: &js_source,
            js_path: Some(&args.file),
            minify: args.minify,
            module_root: args.module_root.as_deref(),
            world_name: args.world.as_deref(),
            stub_wasi: args.stub_wasi,
            auto_vendor: !args.no_vendor,
            polyfills: &polyfills,
            disable_gc: args.disable_gc,
            runtime,
        },
        &ScriptcConfig {
            profiles: &statically_compiled,
            bin: args.scriptc_bin.as_deref(),
        },
    )
    .await?;

    // Create the output's directory rather than failing at the very end of a
    // build that has already done all its work. `-o dist/app.wasm` into a
    // fresh checkout is an ordinary thing to ask for, and the failure it
    // produced named an I/O error rather than the directory.
    if let Some(parent) = args.output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create output directory {}", parent.display())
        })?;
    }
    fs::write(&args.output, &component)
        .with_context(|| format!("failed to write output to {}", args.output.display()))?;

    println!("Component written to {}", args.output.display());
    println!("  Size: {} bytes", component.len());

    if let Some(types_dir) = &args.emit_types {
        println!("Generating TypeScript types via `jco types`...");
        dwarf_core::types::emit_ts_types(&args.wit, args.world.as_deref(), types_dir, &polyfills)
            .context("failed to generate TypeScript types")?;
        println!("Types written to {}", display_path(types_dir));
    }

    Ok(())
}
