//! TypeScript input: type stripping, not type checking.
//!
//! QuickJS runs JavaScript, so a `.ts` entry (or a `.ts` module reached by
//! an `import`) is transformed to JavaScript before it is evaluated. The
//! transform is oxc's, in-process - no `tsc`, no `esbuild`, nothing on
//! `PATH` - which is what makes `dwarf --file app.ts` a single command
//! rather than a bundler step somebody has to remember.
//!
//! What this deliberately does NOT do is check types. Annotations are
//! erased, exactly as Node's own type stripping and esbuild do it; a type
//! error stays a type error for `tsc --noEmit` to find in the editor or in
//! CI. dwarf is a component builder, and silently becoming the project's
//! type checker would make every build slower to serve a job the toolchain
//! it sits in already does better.

use std::path::Path;

use anyhow::{Result, anyhow};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};

/// Whether `path` names a TypeScript module dwarf should transform.
///
/// `.d.ts` is excluded: it declares types and emits no code, so feeding one
/// in is a mistake worth reporting rather than compiling to an empty
/// module.
pub(crate) fn is_typescript(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if name.ends_with(".d.ts") || name.ends_with(".d.mts") || name.ends_with(".d.cts") {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts" | "mts" | "cts")
    )
}

/// Strips TypeScript types from `source`, returning JavaScript.
///
/// `path` is passed through to oxc so its diagnostics name the real file,
/// and so JSX/TSX is recognised by extension rather than guessed at.
pub(crate) fn strip_types(source: &str, path: &Path) -> Result<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path)
        .map_err(|err| anyhow!("unsupported TypeScript file {}: {err}", path.display()))?;

    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        // A parse error here is a syntax error in the user's TypeScript, and
        // it is worth being precise that it is not a type error: dwarf never
        // reports those, so "dwarf rejected my types" is the wrong
        // conclusion to leave available.
        let rendered = parsed
            .diagnostics
            .iter()
            .map(|err| err.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(anyhow!(
            "failed to parse TypeScript {}: {rendered}\n\
             (this is a syntax error, not a type error - dwarf strips types \
             without checking them, so run `tsc --noEmit` for type errors)",
            path.display()
        ));
    }

    let mut program = parsed.program;
    // `with_enum_eval` is REQUIRED, not an optimization: the transformer
    // needs const-evaluated enum members to lower `enum` at all, and
    // panics outright without them. A user enum would take the whole build
    // down with a message about oxc's internals.
    let scoping = SemanticBuilder::new()
        .with_enum_eval(true)
        .build(&program)
        .semantic
        .into_scoping();

    // Default options: erase types and leave everything else alone. No
    // downleveling to an older JavaScript - QuickJS is current enough, and
    // transforming syntax dwarf's engine already supports would only make
    // the embedded source bigger and harder to read in a stack trace.
    let options = TransformOptions::default();
    let result = Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);

    if result.diagnostics.has_errors() {
        let rendered = result
            .diagnostics
            .iter()
            .map(|err| err.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(anyhow!(
            "failed to transform TypeScript {}: {rendered}",
            path.display()
        ));
    }

    Ok(Codegen::new().build(&program).code)
}

/// Transforms `source` when `path` is TypeScript, and passes JavaScript
/// through untouched.
pub(crate) fn to_javascript(source: &str, path: &Path) -> Result<String> {
    if is_typescript(path) {
        strip_types(source, path)
    } else {
        Ok(source.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_erased_and_code_survives() {
        let ts = r#"
            interface Greeting { name: string }
            type Loud = string;
            export function greet(g: Greeting): Loud {
                const n: string = g.name;
                return `Hello, ${n}!` as Loud;
            }
        "#;
        let js = strip_types(ts, Path::new("app.ts")).unwrap();

        assert!(!js.contains("interface"), "types should be gone: {js}");
        assert!(!js.contains(": string"), "annotations should be gone: {js}");
        assert!(!js.contains(" as Loud"), "assertions should be gone: {js}");
        assert!(js.contains("export function greet"), "code should stay: {js}");
        assert!(js.contains("Hello, "), "code should stay: {js}");
    }

    #[test]
    fn enums_and_parameter_properties_become_real_javascript() {
        // The parts of TypeScript that are not merely annotations: these
        // EMIT code, so stripping alone would silently drop behaviour.
        let ts = r#"
            export enum Color { Red, Green }
            export class P { constructor(public x: number) {} }
        "#;
        let js = strip_types(ts, Path::new("app.ts")).unwrap();

        assert!(js.contains("Color"), "enum should emit code: {js}");
        assert!(
            js.contains("this.x") || js.contains("x = x"),
            "parameter property should be assigned: {js}"
        );
    }

    #[test]
    fn javascript_passes_through_untouched() {
        let js = "export const x = 1;\n";
        assert_eq!(to_javascript(js, Path::new("app.js")).unwrap(), js);
    }

    #[test]
    fn a_syntax_error_says_it_is_not_a_type_error() {
        let err = strip_types("export function ( {", Path::new("bad.ts")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("bad.ts"), "should name the file: {msg}");
        assert!(
            msg.contains("not a type error"),
            "should not read as a type error: {msg}"
        );
    }

    #[test]
    fn declaration_files_are_not_treated_as_input() {
        assert!(!is_typescript(Path::new("types.d.ts")));
        assert!(is_typescript(Path::new("app.ts")));
        assert!(is_typescript(Path::new("app.mts")));
        assert!(!is_typescript(Path::new("app.js")));
    }
}
