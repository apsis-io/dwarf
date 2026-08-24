//! Resolves a WIT path, automatically fetching missing dependencies with
//! `wkg wit fetch` (<https://github.com/bytecodealliance/wasm-pkg-tools>) when
//! resolution fails because a referenced package isn't vendored under
//! `deps/` yet.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use wit_parser::{PackageId, PackageName, Resolve, ResolveError, ResolveErrorKind};

const INSTALL_HINT: &str = "install it with `cargo install wkg` (see https://github.com/bytecodealliance/wasm-pkg-tools), \
     or vendor the missing package(s) manually under a `deps/` directory next to your WIT";

/// Parses the WIT package at `path`, retrying once via `wkg wit fetch` if the
/// first parse fails because of an unresolved package and `auto_vendor` is
/// enabled. `path` must be a directory for vendoring to apply, since a single
/// standalone WIT file has no `deps/` directory for `wkg` to populate.
pub(crate) fn resolve_wit(path: &Path, auto_vendor: bool) -> Result<(Resolve, PackageId)> {
    let err = match parse_wit(path) {
        Ok(resolved) => return Ok(resolved),
        Err(err) => err,
    };

    let Some(missing) = missing_package(&err) else {
        return Err(annotate_syntax_hint(err));
    };

    if !auto_vendor {
        return Err(err.context(format!(
            "missing WIT package `{missing}`; auto-vendoring is disabled (--no-vendor), {INSTALL_HINT}"
        )));
    }
    if !path.is_dir() {
        return Err(err.context(format!(
            "missing WIT package `{missing}`; auto-vendoring only applies when --wit points at a \
             directory (so `wkg` has a `deps/` directory to populate) - point --wit at the \
             directory containing this WIT file instead of the file itself"
        )));
    }

    eprintln!(
        "WIT package `{missing}` not found under {}/deps - fetching it with `wkg wit fetch`...",
        path.display()
    );
    match fetch_deps(path) {
        Ok(()) => parse_wit(path).with_context(|| {
            format!(
                "still failed to resolve WIT after running `wkg wit fetch` under {}",
                path.display()
            )
        }),
        Err(fetch_err) => Err(err.context(format!(
            "auto-vendoring failed: {fetch_err}; {INSTALL_HINT}"
        ))),
    }
}

fn parse_wit(path: &Path) -> Result<(Resolve, PackageId)> {
    let mut resolve = Resolve::default();
    let (pkg_id, _) = resolve.push_path(path)?;
    Ok((resolve, pkg_id))
}

/// If `err` failed to resolve because of an unresolved package reference,
/// returns the name of the missing package.
fn missing_package(err: &anyhow::Error) -> Option<PackageName> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<ResolveError>())
        .and_then(|e| match e.kind() {
            ResolveErrorKind::PackageNotFound { requested, .. } => Some(requested.clone()),
            _ => None,
        })
}

/// wit-parser reports a missing top-level terminator (most commonly a `;`
/// after a `package` declaration, or a `}` closing a preceding
/// `interface`/`record`/`variant`/etc. block) as an error at the position of
/// the *next* item it manages to parse, not at the missing token itself -
/// technically correct (that's genuinely where the token stream diverges
/// from what's valid) but easy to misread as "the following item is wrong"
/// instead of "something before this wasn't closed/terminated". Confirmed by
/// reproducing several variants (missing `;` after `package`, missing `}`
/// closing an `interface` cascading into the next `world`, missing `}` at
/// EOF) - genuine typos elsewhere (a misspelled keyword, an unknown type
/// name) already produce a directly-actionable, correctly-located error with
/// no cascading, so this only targets the specific "next top-level
/// construct, or EOF, showed up somewhere it structurally can't" shape.
/// Appends a hint pointing at the real, more common cause instead of trying
/// to fix wit-parser's own error location (out of dwarf's control - see
/// below).
///
/// wit-parser's own parse error (`wit_parser::ParseError`) is flattened into a
/// plain rendered string (`UnresolvedPackageGroup::parse`/`parse_dir` both do
/// `anyhow!("{}", e.highlight(&map))`) before it ever reaches this crate, so
/// there's no structured error left to downcast to here - matching on the
/// rendered message is the only option wit-parser's public API leaves us.
fn annotate_syntax_hint(err: anyhow::Error) -> anyhow::Error {
    let msg = err.to_string();
    // `world`/`interface` can only legally start a fresh top-level item, and
    // EOF can only legally occur once every block is closed - if the parser
    // reports any of these as the *unexpected* token, something before it
    // was left unterminated/unclosed; there's no legitimate WIT program
    // where seeing one of these next is itself the actual mistake.
    let looks_like_missing_terminator = msg.contains("found keyword `world`")
        || msg.contains("found keyword `interface`")
        || msg.contains("found eof");

    if looks_like_missing_terminator {
        err.context(
            "hint: this is often caused by a missing ';' terminating the previous top-level \
             item (e.g. `package foo:bar;`) or a missing '}' closing a preceding \
             world/interface/record/variant/etc. block - the parser reports the error at the \
             next item it saw (or end of file), not at the actual missing ';'/'}'",
        )
    } else {
        err
    }
}

/// Runs `wkg wit fetch` for `wit_dir`.
///
/// The directory is POSITIONAL (`wkg wit fetch <DIR>`) as of wkg 0.16;
/// earlier releases spelled it `--wit-dir`, which is what dwarf used to
/// pass unconditionally. Against a current wkg that failed with
/// "unexpected argument '--wit-dir' found" - a message about dwarf's own
/// command line, surfaced to someone who had merely asked for a WIT
/// dependency, with nothing to act on. Both spellings are tried now, the
/// current one first, so neither a fresh install nor a pinned older one
/// breaks.
fn fetch_deps(wit_dir: &Path) -> Result<()> {
    let current = run_wkg_fetch(&[OsStr::new(wit_dir)])?;
    if current.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&current.stderr).into_owned();
    if !is_cli_usage_error(&stderr) {
        return Err(anyhow!(
            "`wkg wit fetch` exited with {}: {}",
            current.status,
            stderr.trim()
        ));
    }

    let legacy = run_wkg_fetch(&[OsStr::new("--wit-dir"), OsStr::new(wit_dir)])?;
    if legacy.status.success() {
        return Ok(());
    }

    // Neither spelling was accepted: report the CURRENT one's error, since
    // that is the CLI dwarf targets, and say the version is the suspect
    // rather than leaving a raw argument-parsing message to be puzzled over.
    Err(anyhow!(
        "`wkg wit fetch` exited with {}: {}\n\
         (dwarf tried both `wkg wit fetch <dir>` and the older `--wit-dir <dir>`; \
         neither was accepted, which usually means an incompatible `wkg` - \
         `cargo install wkg` to update, or pass --no-vendor and vendor deps by hand)",
        current.status,
        stderr.trim()
    ))
}

fn run_wkg_fetch(dir_args: &[&OsStr]) -> Result<std::process::Output> {
    match Command::new("wkg")
        .arg("wit")
        .arg("fetch")
        .args(dir_args)
        .output()
    {
        Ok(output) => Ok(output),
        Err(io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
            Err(anyhow!("`wkg` is not installed"))
        }
        Err(io_err) => Err(anyhow!("failed to run `wkg wit fetch`: {io_err}")),
    }
}

/// Whether `wkg` rejected dwarf's COMMAND LINE rather than failing at the
/// job itself - the signal that the other spelling is worth trying. A
/// network or registry failure must not be retried this way: it would just
/// fail twice and report the wrong cause.
fn is_cli_usage_error(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("unexpected argument")
        || stderr.contains("unrecognized")
        || stderr.contains("unknown option")
        || (stderr.contains("usage:") && stderr.contains("error:"))
}

#[cfg(test)]
mod tests {
    use super::is_cli_usage_error;

    #[test]
    fn a_rejected_flag_is_a_usage_error() {
        // Verbatim from wkg 0.16.1, which is how this was found.
        assert!(is_cli_usage_error(
            "error: unexpected argument '--wit-dir' found\n\nUsage: wkg wit fetch [OPTIONS] [DIR]"
        ));
        assert!(is_cli_usage_error("error: unrecognized subcommand 'fetch'"));
    }

    #[test]
    fn a_failed_fetch_is_not_retried_as_one() {
        // The retry exists to try the other spelling of the directory
        // argument. Anything that got as far as talking to a registry must
        // not go round again: it would fail identically and then report the
        // argument spelling as the cause of a network problem.
        for real_failure in [
            "error: failed to fetch package wasi:cli: connection refused",
            "error: no such package `wasi:http@0.3.0` in registry",
            "error: failed to parse lock file",
        ] {
            assert!(
                !is_cli_usage_error(real_failure),
                "should not look like a usage error: {real_failure}"
            );
        }
    }
}
