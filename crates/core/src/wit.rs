//! Resolves a WIT path, automatically fetching missing dependencies with
//! `wkg fetch` (<https://github.com/bytecodealliance/wasm-pkg-tools>) when
//! resolution fails because a referenced package isn't vendored under
//! `deps/` yet.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use wit_parser::{PackageId, PackageName, Resolve, ResolveError, ResolveErrorKind};

const INSTALL_HINT: &str = "install it with `cargo install wkg` (see https://github.com/bytecodealliance/wasm-pkg-tools), \
     or vendor the missing package(s) manually under a `deps/` directory next to your WIT";

/// Parses the WIT package at `path`, retrying once via `wkg fetch` if the
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
        "WIT package `{missing}` not found under {}/deps - fetching it with `wkg fetch`...",
        path.display()
    );
    match fetch_deps(path) {
        Ok(()) => parse_wit(path).with_context(|| {
            format!(
                "still failed to resolve WIT after running `wkg fetch` under {}",
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

/// Fetches `wit_dir`'s missing dependencies into `wit_dir/deps`.
///
/// wkg's CLI has moved twice. `wkg fetch <DIR>` is the current spelling;
/// `wkg wit fetch <DIR>` still works in 0.16 but is deprecated ("`wkg wit
/// <command>` is deprecated, use `wkg <command>` instead"); and the
/// `--wit-dir <DIR>` flag older releases took has been removed outright.
/// dwarf passed that removed flag unconditionally until today, so
/// vendoring failed against every current wkg with "unexpected argument
/// '--wit-dir' found" - a complaint about dwarf's own command line, shown
/// to someone who had merely asked for a WIT dependency.
///
/// All three are tried, newest first, so neither a fresh install nor a
/// pinned old one breaks, and the deprecation becoming a removal costs
/// nothing here.
fn fetch_deps(wit_dir: &Path) -> Result<()> {
    let dir = wit_dir.as_os_str();

    // Newest spelling first, oldest last. `wkg wit fetch` still works in
    // 0.16 but prints a deprecation warning, and `--wit-dir` was already
    // removed - dwarf passed it unconditionally until today, which is how
    // vendoring came to be broken against every current wkg. Trying the
    // current form first means the deprecation becoming a removal is a
    // no-op here rather than the same outage again.
    let spellings: [&[&OsStr]; 3] = [
        &[OsStr::new("fetch"), dir],
        &[OsStr::new("wit"), OsStr::new("fetch"), dir],
        &[
            OsStr::new("wit"),
            OsStr::new("fetch"),
            OsStr::new("--wit-dir"),
            dir,
        ],
    ];

    let mut first_failure = None;
    for args in spellings {
        let output = run_wkg(args)?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        // Only an argument-parsing rejection means "try the other
        // spelling". A fetch that reached a registry and failed must stop
        // here, or every spelling fails in turn and the last one's message
        // blames the flag for a network problem.
        if !is_cli_usage_error(&stderr) {
            return Err(anyhow!(
                "`wkg` exited with {}: {}",
                output.status,
                stderr.trim()
            ));
        }
        first_failure.get_or_insert((output.status, stderr));
    }

    let (status, stderr) = first_failure.expect("at least one spelling is always tried");
    Err(anyhow!(
        "`wkg` exited with {}: {}\n\
         (dwarf tried `wkg fetch <dir>`, `wkg wit fetch <dir>` and the older \
         `--wit-dir <dir>`; none was accepted, which usually means an \
         incompatible `wkg` - `cargo install wkg` to update, or pass \
         --no-vendor and vendor deps by hand)",
        status,
        stderr.trim()
    ))
}

fn run_wkg(args: &[&OsStr]) -> Result<std::process::Output> {
    match Command::new("wkg").args(args).output() {
        Ok(output) => Ok(output),
        Err(io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
            Err(anyhow!("`wkg` is not installed"))
        }
        Err(io_err) => Err(anyhow!("failed to run `wkg`: {io_err}")),
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
    fn the_deprecated_subcommand_is_also_a_usage_error() {
        // If a future wkg removes `wkg wit <command>` outright, the message
        // must still read as "try the next spelling" rather than as a real
        // fetch failure.
        assert!(is_cli_usage_error(
            "error: unrecognized subcommand 'wit'\n\nUsage: wkg <COMMAND>"
        ));
    }

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
