//! Minifies the entry module's JavaScript with oxc, before it is embedded.
//!
//! This lives next to the TypeScript transform, and after it, because the
//! order is load-bearing: minifying first meant parsing a `.ts` entry as
//! plain JavaScript, and oxc's parse errors were discarded rather than
//! raised, so the minifier emitted a program with the exports missing. That
//! built a component successfully and trapped on the first call - the worst
//! shape a bug can take. Stripping types first means the minifier only ever
//! sees JavaScript.

use anyhow::{Result, anyhow};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_minifier::{
    CompressOptions, CompressOptionsKeepNames, CompressOptionsUnused, MangleOptions, Minifier,
    MinifierOptions,
};
use oxc_parser::Parser;
use oxc_span::SourceType;

/// Minifies `source`, which must already be JavaScript.
pub(crate) fn minify_js(source: &str) -> Result<String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::mjs()).parse();

    // Refusing to minify a program oxc could not fully parse, rather than
    // minifying whatever partial AST came back: that silence is what turned
    // a TypeScript entry into a component that built and then trapped.
    if !parsed.diagnostics.is_empty() {
        let rendered = parsed
            .diagnostics
            .iter()
            .map(|err| err.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(anyhow!(
            "failed to parse JavaScript for --minify: {rendered}"
        ));
    }

    let mut program = parsed.program;
    let options = MinifierOptions {
        mangle: Some(MangleOptions {
            // Top-level names are the WIT exports; mangling them would
            // rename the very functions the bindings look up.
            top_level: Some(false),
            ..Default::default()
        }),
        compress: Some(CompressOptions {
            unused: CompressOptionsUnused::Keep,
            keep_names: CompressOptionsKeepNames::all_false(),
            ..CompressOptions::default()
        }),
    };
    let ret = Minifier::new(options).minify(&allocator, &mut program);

    Ok(Codegen::new()
        .with_scoping(ret.scoping)
        .build(&program)
        .code)
}

#[cfg(test)]
mod tests {
    use super::minify_js;

    #[test]
    fn exports_survive_minification() {
        let out = minify_js("export function greet(name) {\n  const x = 1;\n  return name + x;\n}\n")
            .unwrap();
        assert!(out.contains("greet"), "top-level export must not be mangled: {out}");
        assert!(out.len() < 60, "should actually be smaller: {out}");
    }

    #[test]
    fn unparseable_input_is_refused_rather_than_half_minified() {
        // TypeScript reaching this function means the strip did not run.
        // Returning a partial program instead of an error is what produced a
        // component that built and trapped.
        let err = minify_js("export function greet(name: string): string { return name; }")
            .unwrap_err();
        assert!(
            err.to_string().contains("--minify"),
            "should name the flag: {err}"
        );
    }
}
